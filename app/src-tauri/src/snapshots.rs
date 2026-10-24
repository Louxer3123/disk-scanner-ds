//! Saved scans of a drive ("snapshots") and comparing a drive with one of them.
//! Files live in the app's data folder (%APPDATA%\io.github.diskscanner\snapshots)
//! and are named `<drive letter>_<Unix seconds>.dsnap`.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use scanner_core::snapshot::{SnapshotHeader, read_header, read_snapshot, write_snapshot};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::state::{AppState, Comparison};

const EXTENSION: &str = "dsnap";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotDto {
    id: String,
    /// The scanned folder, e.g. `C:\`.
    root: String,
    /// Unix seconds.
    created: u64,
    allocated: u64,
    files: u64,
    dirs: u64,
    /// Size of the snapshot file.
    bytes: u64,
}

impl SnapshotDto {
    fn new(id: String, head: SnapshotHeader, bytes: u64) -> Self {
        SnapshotDto {
            id,
            root: head.root,
            created: head.created,
            allocated: head.allocated,
            files: head.files,
            dirs: head.dirs,
            bytes,
        }
    }
}

fn snapshot_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("snapshots");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// An id as this module makes them: a drive letter, `_`, digits. Anything else
/// (separators, dots) is refused, so an id can never name a file elsewhere.
fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() > 2 && b.len() < 32 && b[0].is_ascii_uppercase() && b[1] == b'_' && b[2..].iter().all(u8::is_ascii_digit)
}

fn path_of(dir: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("Неверный снимок".into());
    }
    Ok(dir.join(format!("{id}.{EXTENSION}")))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Snapshots of `root` (all when `None`), newest first. Unreadable files are skipped.
#[tauri::command]
pub async fn list_snapshots(app: AppHandle, root: Option<String>) -> Result<Vec<SnapshotDto>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = snapshot_dir(&app)?;
        let mut out = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
            let path = entry.path();
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).filter(|s| valid_id(s)) else { continue };
            if path.extension().and_then(|e| e.to_str()) != Some(EXTENSION) {
                continue;
            }
            let Ok(head) = File::open(&path).and_then(read_header) else { continue };
            if root.as_ref().is_some_and(|r| !r.eq_ignore_ascii_case(&head.root)) {
                continue;
            }
            let bytes = entry.metadata().map_or(0, |m| m.len());
            out.push(SnapshotDto::new(id.to_owned(), head, bytes));
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.created));
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Saves the drive's current scan as a snapshot.
#[tauri::command]
pub async fn save_snapshot(app: AppHandle, drive: usize) -> Result<SnapshotDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let (tree, root) = {
            let drives = state.drives();
            let d = drives.get(drive).ok_or("no such drive")?;
            if !d.scanned || d.cancel.is_some() {
                return Err("Сначала дождитесь окончания сканирования диска".into());
            }
            (d.tree.clone(), d.info.root.clone())
        };
        let letter = root.chars().next().filter(char::is_ascii_alphabetic).ok_or("Снимки есть только у дисков")?;
        let dir = snapshot_dir(&app)?;
        let mut created = now();
        let (id, path) = loop {
            let id = format!("{}_{created}", letter.to_ascii_uppercase());
            let path = path_of(&dir, &id)?;
            if !path.exists() {
                break (id, path);
            }
            created += 1;
        };
        // Written under another name and renamed, so a half-written file never
        // shows up as a snapshot.
        let tmp = path.with_extension("tmp");
        let written = File::create(&tmp).and_then(|f| write_snapshot(&tree, created, f)).and_then(|()| fs::rename(&tmp, &path));
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(format!("Не удалось сохранить снимок: {e}"));
        }
        let head = File::open(&path).and_then(read_header).map_err(|e| e.to_string())?;
        let bytes = fs::metadata(&path).map_or(0, |m| m.len());
        Ok(SnapshotDto::new(id, head, bytes))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Deletes a snapshot file and stops any comparison with it.
#[tauri::command]
pub fn delete_snapshot(app: AppHandle, state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    let path = path_of(&snapshot_dir(&app)?, &id)?;
    fs::remove_file(&path).map_err(|e| format!("Не удалось удалить снимок: {e}"))?;
    for d in state.drives().iter_mut() {
        if d.comparison.as_ref().is_some_and(|c| c.id == id) {
            d.comparison = None;
        }
    }
    Ok(())
}

/// Compares the drive with snapshot `id` from now on, or stops comparing when
/// `id` is `None`.
#[tauri::command]
pub async fn compare(app: AppHandle, drive: usize, id: Option<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let Some(id) = id else {
            if let Some(d) = state.drives().get_mut(drive) {
                d.comparison = None;
            }
            return Ok(());
        };
        let (root, tree, tree_gen) = {
            let drives = state.drives();
            let d = drives.get(drive).ok_or("no such drive")?;
            if !d.scanned {
                return Err("Сначала просканируйте диск".into());
            }
            (d.info.root.clone(), d.tree.clone(), d.tree_gen)
        };
        let path = path_of(&snapshot_dir(&app)?, &id)?;
        let (head, old) = File::open(&path)
            .and_then(read_snapshot)
            .map_err(|e| format!("Не удалось прочитать снимок: {e}"))?;
        if !head.root.eq_ignore_ascii_case(&root) {
            return Err(format!("Снимок сделан для {}, а не для {root}", head.root));
        }
        let comparison = Arc::new(Comparison::new(id, head.created, old));
        // Computed here, off the UI thread and without the lock, rather than at
        // the first layout.
        comparison.diff(&tree, tree_gen);
        state.drives().get_mut(drive).ok_or("no such drive")?.comparison = Some(comparison);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::valid_id;

    #[test]
    fn only_own_ids_are_accepted() {
        assert!(valid_id("C_1760000000"));
        for bad in ["", "C_", "c_123", "C-123", "C_12a", "..\\x", "C_1/..", "C_123.dsnap", "CC_1"] {
            assert!(!valid_id(bad), "{bad:?}");
        }
    }
}
