//! File actions: open, reveal, rescan a folder, recycle, delete; the recycle bin,
//! the "Apps" settings page, the Explorer menu entry and the startup folder.

use std::path::{Path, PathBuf};

use scanner_core::{CancelToken, NodeFlags, NodeId, NodeKind, ScanOptions, Tree, scan_with};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::i18n::tr;
use crate::shell;
use crate::state::{AppState, NodeRef, rel_components};

/// Tree generation that never matches, so a [`NodeRef`] is resolved by path.
const NO_GEN: u64 = u32::MAX as u64;
fn not_found() -> &'static str {
    tr("Объект больше не найден — возможно, он уже удалён", "The item is gone — it may have been deleted already")
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TreeChanged {
    drive: usize,
    tree_gen: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FolderScan {
    drive: usize,
    path: String,
    running: bool,
    error: Option<String>,
}

fn emit_tree_changed(app: &AppHandle, drive: usize) {
    let tree_gen = app.state::<AppState>().drives().get(drive).map_or(0, |d| d.tree_gen);
    let _ = app.emit("tree-changed", TreeChanged { drive, tree_gen });
}

/// Path, kind and drive root of the node `r` refers to; it must still exist.
fn target(state: &AppState, r: &NodeRef) -> Result<(PathBuf, NodeKind, NodeId, String), String> {
    let drives = state.drives();
    let d = drives.get(r.drive).ok_or_else(|| tr("Нет такого диска", "No such drive"))?;
    let id = d.resolve(r, true).ok_or_else(not_found)?;
    let path = checked_path(&d.tree, id, &d.info.root)?;
    Ok((path, d.tree.kind(id), id, d.info.root.clone()))
}

/// The node's path, refused if it might not lead to the node itself: a name on
/// the way was stored lossily (bad Unicode on disk, or a name from the MFT with
/// separators or `..` that was neutralized), or the path does not split into
/// exactly one part per level. Open, reveal and delete all go through here.
fn checked_path(tree: &Tree, id: NodeId, root: &str) -> Result<PathBuf, String> {
    let mut depth = 0;
    let mut cur = Some(id);
    while let Some(n) = cur {
        cur = tree.parent(n);
        // Every name below the drive root must reach exactly that file through a
        // plain Win32 path (see `is_plain_win32_name`).
        let refused = tree.flags(n).contains(NodeFlags::LOSSY_NAME)
            || (cur.is_some() && !scanner_core::is_plain_win32_name(tree.name(n)));
        if refused {
            return Err(tr(
                "В пути есть имя, которое Windows не может открыть напрямую, — действие недоступно",
                "A name on the path cannot be opened by Windows directly, so this is not available",
            )
            .into());
        }
        depth += usize::from(cur.is_some());
    }
    let path = tree.path(id);
    let comps = rel_components(root, &path.to_string_lossy()).unwrap_or_default();
    if comps.len() != depth || comps.iter().any(|c| c == "." || c == "..") {
        return Err(tr("Путь к объекту выглядит неверным — действие недоступно", "The path looks wrong, so this is not available").into());
    }
    Ok(path)
}

#[tauri::command]
pub fn open_node(state: State<'_, AppState>, target: NodeRef) -> Result<(), String> {
    let (path, ..) = self::target(&state, &target)?;
    shell::open(&path)
}

#[tauri::command]
pub fn reveal_node(state: State<'_, AppState>, target: NodeRef) -> Result<(), String> {
    let (path, ..) = self::target(&state, &target)?;
    shell::reveal(&path)
}

/// Moves a file or folder to the recycle bin, or with `permanent` deletes it for
/// good, then updates the drive's tree. The UI asks for confirmation first.
#[tauri::command]
pub async fn remove_node(app: AppHandle, target: NodeRef, permanent: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || remove_blocking(&app, &target, permanent))
        .await
        .map_err(|e| e.to_string())?
}

/// Refusal for actions on a drive whose first scan is still running.
fn wait_for_scan() -> &'static str {
    tr("Дождитесь окончания сканирования диска", "Wait until the drive scan finishes")
}

fn remove_blocking(app: &AppHandle, r: &NodeRef, permanent: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.busy_scanning(r.drive) {
        return Err(wait_for_scan().into());
    }
    let (path, kind, id, root) = target(&state, r)?;
    if id == NodeId::ROOT {
        return Err(tr("Нельзя удалить диск целиком", "A whole drive cannot be deleted").into());
    }
    if matches!(kind, NodeKind::FreeSpace | NodeKind::Unscanned) {
        return Err(tr("Это не файл и не папка", "This is not a file or folder").into());
    }

    let result = if permanent {
        shell::delete_permanently(&path, kind == NodeKind::Dir).map_err(|e| e.to_string())
    } else {
        trash::delete(&path).map_err(|e| e.to_string())
    };
    let path_str = path.to_string_lossy().into_owned();

    if let Err(err) = result {
        // Part of a folder may already be gone: rescan around it so the chart
        // matches the disk again.
        if let Some(parent) = path.parent().filter(|p| p.parent().is_some()) {
            spawn_rescan(app.clone(), r.drive, parent.to_string_lossy().into_owned());
        }
        return Err(err);
    }

    state.mutate_tree(r.drive, |tree, _| {
        if let Some(comps) = rel_components(&root, &path_str) {
            let (id, matched) = tree.find_path(&comps);
            if matched == comps.len() && id != NodeId::ROOT {
                tree.detach(id);
            }
        }
    })?;
    emit_tree_changed(app, r.drive);

    // Recycled data now sits in the drive's recycle bin; count it there.
    if !permanent && let Some(bin) = recycle_bin_path(&state, r.drive) {
        spawn_rescan(app.clone(), r.drive, bin);
    }
    Ok(())
}

/// Path of the scanned recycle bin folder in the root of `drive`, if any.
fn recycle_bin_path(state: &AppState, drive: usize) -> Option<String> {
    let drives = state.drives();
    let tree = &drives.get(drive)?.tree;
    tree.children(NodeId::ROOT)
        .find(|&c| tree.flags(c).contains(NodeFlags::RECYCLE_BIN))
        .map(|c| tree.path(c).to_string_lossy().into_owned())
}

/// Rescans one folder and puts the result in place of the old one. Rescanning a
/// drive root is a full drive scan, which the UI starts with `scan_drives`.
#[tauri::command]
pub fn rescan_folder(app: AppHandle, state: State<'_, AppState>, target: NodeRef) -> Result<(), String> {
    if state.busy_scanning(target.drive) {
        return Err(wait_for_scan().into());
    }
    let (path, root) = {
        let drives = state.drives();
        let d = drives.get(target.drive).ok_or_else(|| tr("Нет такого диска", "No such drive"))?;
        // A folder outside the tree (the startup folder on an unscanned drive) is
        // given by path alone.
        let path = match d.resolve(&target, true) {
            Some(id) => checked_path(&d.tree, id, &d.info.root)?.to_string_lossy().into_owned(),
            None => target.path.clone(),
        };
        (path, d.info.root.clone())
    };
    if rel_components(&root, &path).is_none_or(|c| c.is_empty()) {
        return Err(tr("Папка не на этом диске", "The folder is not on this drive").into());
    }
    spawn_rescan(app, target.drive, path);
    Ok(())
}

fn spawn_rescan(app: AppHandle, drive: usize, path: String) {
    std::thread::spawn(move || rescan_blocking(&app, drive, &path));
}

fn rescan_blocking(app: &AppHandle, drive: usize, path: &str) {
    let event = |running, error| FolderScan { drive, path: path.to_owned(), running, error };
    let _ = app.emit("folder-scan", event(true, None));
    let state = app.state::<AppState>();
    let threads = state.drives().get(drive).map_or_else(scanner_core::default_threads, |d| d.info.scan_threads());
    let options = ScanOptions { threads };
    let result = scan_with(Path::new(path), &options, &CancelToken::new(), |_| {}).map_err(|e| e.to_string()).and_then(|s| {
        state.mutate_tree(drive, |tree, info| replace_subtree(tree, &info.root, path, &s.tree))?
    });
    let _ = app.emit("folder-scan", event(false, result.as_ref().err().cloned()));
    if result.is_ok() {
        emit_tree_changed(app, drive);
    }
}

/// Puts `scanned` (a scan of `path`) into `tree` in place of the old folder,
/// creating missing parent folders.
fn replace_subtree(tree: &mut Tree, root: &str, path: &str, scanned: &Tree) -> Result<(), String> {
    let comps = rel_components(root, path).filter(|c| !c.is_empty()).ok_or_else(|| tr("Папка не на этом диске", "The folder is not on this drive"))?;
    let (last, parents) = comps.split_last().expect("not empty");
    let parent = tree.ensure_dir_path(parents).ok_or_else(|| tr("На месте папки находится файл", "There is a file where the folder should be"))?;
    // The scan's root name is the canonical path; its last part has the on-disk case.
    let name = Path::new(scanned.name(NodeId::ROOT))
        .file_name()
        .map_or_else(|| last.clone(), |n| n.to_string_lossy().into_owned());
    let mut keep = NodeFlags::empty();
    if let Some(old) = tree.child_named(parent, last) {
        if tree.flags(old).contains(NodeFlags::RECYCLE_BIN) {
            keep = NodeFlags::RECYCLE_BIN;
        }
        tree.detach(old);
    }
    let id = tree.graft(parent, scanned, &name);
    tree.insert_flags(id, keep);
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecycleBinDto {
    size: u64,
    items: u64,
}

#[tauri::command]
pub fn recycle_bin_info() -> Result<RecycleBinDto, String> {
    shell::recycle_bin().map(|b| RecycleBinDto { size: b.size, items: b.items })
}

/// Empties the recycle bins of all drives, then updates every drive's tree.
#[tauri::command]
pub async fn empty_recycle_bin(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        shell::empty_recycle_bin()?;
        let state = app.state::<AppState>();
        let count = state.drives().len();
        for drive in 0..count {
            match recycle_bin_path(&state, drive) {
                Some(bin) => rescan_blocking(&app, drive, &bin),
                None => {
                    // Only the free space changes on a drive whose bin was not scanned.
                    state.mutate_tree(drive, |_, _| ())?;
                    emit_tree_changed(&app, drive);
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn open_apps_settings() -> Result<(), String> {
    shell::open_apps_settings()
}

/// Whether fast mode is on: the app runs as administrator and reads NTFS drives
/// through their MFT.
#[tauri::command]
pub fn fast_mode() -> bool {
    scanner_core::is_elevated()
}

/// Restarts the app as administrator. In a release build this window then
/// closes; returns `false` when it stays open (a dev build, where it hosts the
/// dev server the new window needs).
#[tauri::command]
pub fn enable_fast_mode(app: AppHandle) -> Result<bool, String> {
    shell::relaunch_elevated()?;
    if cfg!(debug_assertions) {
        return Ok(false);
    }
    app.exit(0);
    Ok(true)
}

#[tauri::command]
pub fn explorer_menu_installed() -> bool {
    shell::explorer_menu_installed()
}

#[tauri::command]
pub fn set_explorer_menu(enabled: bool) -> Result<(), String> {
    shell::set_explorer_menu(enabled)
}

/// The folder passed on the command line, as a reference to resolve by path.
#[tauri::command]
pub fn startup_target(state: State<'_, AppState>) -> Option<NodeRef> {
    let path = state.startup_path.clone()?;
    let drives = state.drives();
    let drive = drives.iter().position(|d| rel_components(&d.info.root, &path).is_some())?;
    Some(NodeRef { drive, node: u32::MAX, tree_gen: NO_GEN, path })
}

#[cfg(test)]
mod tests {
    use super::{checked_path, replace_subtree};
    use scanner_core::{NodeFlags, NodeId, Tree};

    #[test]
    fn actions_refuse_lossy_names_anywhere_on_the_path() {
        let mut t = Tree::new("C:\\");
        let users = t.add_dir(NodeId::ROOT, "Users");
        let ok = t.add_file(users, "ok.txt", 1, 1);
        assert_eq!(checked_path(&t, ok, "C:\\").unwrap(), std::path::Path::new("C:\\Users\\ok.txt"));

        let odd = t.add_dir(users, "bad\u{FFFD}name");
        t.insert_flags(odd, NodeFlags::LOSSY_NAME);
        let inside = t.add_file(odd, "file.txt", 1, 1);
        assert!(checked_path(&t, odd, "C:\\").is_err());
        assert!(checked_path(&t, inside, "C:\\").is_err(), "a lossy ancestor taints the path");
    }

    #[test]
    fn actions_refuse_paths_that_do_not_match_the_depth() {
        // A name with a separator (only possible from a damaged or crafted MFT,
        // which also flags it) would add a level to the path.
        let mut t = Tree::new("C:\\");
        let sneaky = t.add_file(NodeId::ROOT, "..\\Windows", 1, 1);
        assert!(checked_path(&t, sneaky, "C:\\").is_err());
    }

    #[test]
    fn actions_refuse_names_win32_would_rewrite() {
        // "data." and "... " would be trimmed by Win32 path parsing, making the path
        // point at "data" or at the parent folder itself.
        let mut t = Tree::new("C:\\");
        let users = t.add_dir(NodeId::ROOT, "Users");
        for name in ["data.", "... ", "NUL", "con.txt"] {
            let n = t.add_dir(users, name);
            let inside = t.add_file(n, "x", 1, 1);
            assert!(checked_path(&t, n, "C:\\").is_err(), "{name:?}");
            assert!(checked_path(&t, inside, "C:\\").is_err(), "inside {name:?}");
        }
        assert!(checked_path(&t, users, "C:\\").is_ok());
    }

    fn scanned(path: &str, file: &str, size: u64) -> Tree {
        let mut t = Tree::new(path);
        t.add_file(NodeId::ROOT, file, size, size);
        t
    }

    #[test]
    fn replaces_an_existing_folder_and_keeps_the_bin_flag() {
        let mut t = Tree::new("C:\\");
        let users = t.add_dir(NodeId::ROOT, "Users");
        let me = t.add_dir(users, "me");
        t.add_file(me, "old", 100, 100);
        t.insert_flags(me, NodeFlags::RECYCLE_BIN);

        replace_subtree(&mut t, "C:\\", "C:\\users\\ME", &scanned("C:\\Users\\me", "new", 40)).unwrap();
        let (id, matched) = t.find_path(&["Users", "me", "new"]);
        assert_eq!(matched, 3);
        assert_eq!(t.allocated(id), 40);
        assert_eq!(t.allocated(NodeId::ROOT), 40);
        assert_eq!(t.children(users).count(), 1, "old folder replaced, not duplicated");
        let (me, _) = t.find_path(&["Users", "me"]);
        assert!(t.flags(me).contains(NodeFlags::RECYCLE_BIN));
    }

    #[test]
    fn creates_missing_parents_for_a_new_folder() {
        let mut t = Tree::new("C:\\");
        replace_subtree(&mut t, "C:\\", "C:\\a\\b\\c", &scanned("C:\\a\\b\\c", "f", 7)).unwrap();
        assert_eq!(t.find_path(&["a", "b", "c", "f"]).1, 4);
        assert_eq!(t.allocated(NodeId::ROOT), 7);
    }

    #[test]
    fn refuses_roots_and_other_drives() {
        let mut t = Tree::new("C:\\");
        assert!(replace_subtree(&mut t, "C:\\", "C:\\", &scanned("C:\\", "f", 1)).is_err());
        assert!(replace_subtree(&mut t, "C:\\", "D:\\x", &scanned("D:\\x", "f", 1)).is_err());
        t.add_file(NodeId::ROOT, "file", 1, 1);
        assert!(replace_subtree(&mut t, "C:\\", "C:\\file\\x", &scanned("C:\\file\\x", "f", 1)).is_err());
    }
}
