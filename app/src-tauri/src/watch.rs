//! Notices drives being plugged in or removed, and media changes in card readers
//! and optical drives, by polling.

use std::time::Duration;

use scanner_core::{DriveInfo, DriveKind, drive_info, present_roots};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::{AppState, Drive};

const POLL_EVERY: Duration = Duration::from_secs(2);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DrivesChanged {
    /// Indices of drives that appeared, disappeared or changed media.
    changed: Vec<usize>,
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(POLL_EVERY);
            let changed = poll(&app.state::<AppState>());
            if !changed.is_empty() {
                let _ = app.emit("drives-changed", DrivesChanged { changed });
            }
        }
    });
}

/// What the current drive slots look like, to compare with the system.
struct Known {
    root: String,
    present: bool,
    kind: DriveKind,
    info: DriveInfo,
}

/// Brings the drive list in line with the system; returns the changed indices.
fn poll(state: &AppState) -> Vec<usize> {
    let roots = present_roots();
    let known: Vec<Known> = state
        .drives()
        .iter()
        .map(|d| Known { root: d.info.root.clone(), present: d.present, kind: d.info.kind, info: d.info.clone() })
        .collect();

    // Query volumes without holding the lock: an optical drive can take a moment.
    let mut updates: Vec<(Option<usize>, Option<DriveInfo>)> = Vec::new();
    for (i, k) in known.iter().enumerate() {
        let here = roots.contains(&k.root);
        if k.present && !here {
            updates.push((Some(i), None));
        } else if here
            && (!k.present || has_media(k.kind))
            && let Some(info) = drive_info(&k.root).filter(|info| !k.present || media_changed(&k.info, info))
        {
            updates.push((Some(i), Some(info)));
        }
    }
    for root in roots.iter().filter(|r| !known.iter().any(|k| &k.root == *r)) {
        if let Some(info) = drive_info(root) {
            updates.push((None, Some(info)));
        }
    }
    if updates.is_empty() {
        return Vec::new();
    }

    let mut drives = state.drives();
    let mut changed = Vec::new();
    for (slot, info) in updates {
        match (slot, info) {
            (Some(i), Some(info)) => drives[i].reset(info, true),
            (Some(i), None) => {
                let info = DriveInfo { ready: false, capacity: 0, free: 0, ..drives[i].info.clone() };
                drives[i].reset(info, false);
            }
            (None, Some(info)) => drives.push(Drive::new(info)),
            (None, None) => continue,
        }
        changed.push(slot.unwrap_or(drives.len() - 1));
    }
    changed
}

/// Drives whose media can be swapped while the letter stays.
fn has_media(kind: DriveKind) -> bool {
    matches!(kind, DriveKind::Removable | DriveKind::CdRom | DriveKind::Unknown)
}

/// A different volume is in the drive, or it became (un)ready. Free space alone
/// changing is not a reason to drop a scan.
fn media_changed(old: &DriveInfo, new: &DriveInfo) -> bool {
    old.ready != new.ready
        || old.capacity != new.capacity
        || old.label != new.label
        || old.file_system != new.file_system
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(ready: bool, capacity: u64, free: u64) -> DriveInfo {
        DriveInfo {
            root: "E:\\".into(),
            label: "USB".into(),
            file_system: "exFAT".into(),
            kind: DriveKind::Removable,
            capacity,
            free,
            physical_disk: Some(2),
            rotational: Some(false),
            ready,
        }
    }

    #[test]
    fn only_a_different_volume_counts_as_a_media_change() {
        assert!(!media_changed(&info(true, 100, 50), &info(true, 100, 10)), "free space changed");
        assert!(media_changed(&info(true, 100, 50), &info(false, 0, 0)), "card pulled out");
        assert!(media_changed(&info(false, 0, 0), &info(true, 100, 50)), "card inserted");
        assert!(media_changed(&info(true, 100, 50), &info(true, 200, 50)), "another card");
    }

    #[test]
    fn removal_and_return_keep_the_slot_and_bump_the_generation() {
        let mut d = Drive::new(info(true, 100, 50));
        d.scanned = true;
        let before = d.tree_gen;
        d.reset(DriveInfo { ready: false, capacity: 0, free: 0, ..d.info.clone() }, false);
        assert!(!d.present && !d.scanned);
        assert!(d.tree_gen > before);
        d.reset(info(true, 100, 50), true);
        assert!(d.present && d.tree_gen > before + 1);
    }
}
