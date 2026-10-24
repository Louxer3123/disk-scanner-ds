//! File actions: open, reveal, rescan a folder, recycle, delete; the recycle bin,
//! the "Apps" settings page, the Explorer menu entry and the startup folder.

use std::path::{Path, PathBuf};

use scanner_core::{CancelToken, NodeFlags, NodeId, NodeKind, ScanOptions, Tree, scan_with};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::shell;
use crate::state::{AppState, NodeRef, rel_components};

/// Tree generation that never matches, so a [`NodeRef`] is resolved by path.
const NO_GEN: u64 = u32::MAX as u64;
const NOT_FOUND: &str = "Объект больше не найден — возможно, он уже удалён";

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
    let d = drives.get(r.drive).ok_or("Нет такого диска")?;
    let id = d.resolve(r, true).ok_or(NOT_FOUND)?;
    Ok((d.tree.path(id), d.tree.kind(id), id, d.info.root.clone()))
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

fn remove_blocking(app: &AppHandle, r: &NodeRef, permanent: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (path, kind, id, root) = target(&state, r)?;
    if id == NodeId::ROOT {
        return Err("Нельзя удалить диск целиком".into());
    }
    if matches!(kind, NodeKind::FreeSpace | NodeKind::Unscanned) {
        return Err("Это не файл и не папка".into());
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
    let (path, root) = {
        let drives = state.drives();
        let d = drives.get(target.drive).ok_or("Нет такого диска")?;
        // A folder outside the tree (the startup folder on an unscanned drive) is
        // given by path alone.
        let path = match d.resolve(&target, true) {
            Some(id) => d.tree.path(id).to_string_lossy().into_owned(),
            None => target.path.clone(),
        };
        (path, d.info.root.clone())
    };
    if rel_components(&root, &path).is_none_or(|c| c.is_empty()) {
        return Err("Папка не на этом диске".into());
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
    let comps = rel_components(root, path).filter(|c| !c.is_empty()).ok_or("Папка не на этом диске")?;
    let (last, parents) = comps.split_last().expect("not empty");
    let parent = tree.ensure_dir_path(parents).ok_or("На месте папки находится файл")?;
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
    use super::replace_subtree;
    use scanner_core::{NodeFlags, NodeId, Tree};

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
