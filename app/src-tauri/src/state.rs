//! Drives, their scan results and the layout currently shown.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use scanner_core::{CancelToken, DriveInfo, NodeId, NodeKind, SunburstLayout, Tree};

pub struct Drive {
    pub info: DriveInfo,
    /// Scan result, or a placeholder with the OS-reported used and free space.
    pub tree: Arc<Tree>,
    pub scanned: bool,
    /// Set while a scan of this drive runs.
    pub cancel: Option<CancelToken>,
    /// Bumped whenever `tree` is replaced, so node ids from an older tree are rejected.
    pub tree_gen: u64,
}

impl Drive {
    pub fn new(info: DriveInfo) -> Self {
        let tree = Arc::new(placeholder_tree(&info));
        Drive { info, tree, scanned: false, cancel: None, tree_gen: 0 }
    }
}

/// Tree for a drive that has not been scanned yet: one block of used space, one of free.
fn placeholder_tree(info: &DriveInfo) -> Tree {
    let mut tree = Tree::new(&info.root);
    tree.add_space(NodeId::ROOT, "Used", NodeKind::Unscanned, info.used());
    tree.add_space(NodeId::ROOT, "Free space", NodeKind::FreeSpace, info.free);
    tree
}

/// Adds the drive's free space and the used space the scan did not see, so the
/// drive's root covers its whole capacity.
pub fn complete_drive_tree(tree: &mut Tree, info: &DriveInfo) {
    let unscanned = info.used().saturating_sub(tree.allocated(NodeId::ROOT));
    if unscanned > 0 {
        tree.add_space(NodeId::ROOT, "System and unreadable", NodeKind::Unscanned, unscanned);
    }
    tree.add_space(NodeId::ROOT, "Free space", NodeKind::FreeSpace, info.free);
}

/// The layout last sent to the UI, kept for hit testing.
pub struct Current {
    pub generation: u64,
    pub layout: SunburstLayout,
    /// `(drive index, tree, tree generation)` for each layout part.
    pub parts: Vec<(usize, Arc<Tree>, u64)>,
}

#[derive(Default)]
pub struct AppState {
    drives: Mutex<Vec<Drive>>,
    current: Mutex<Option<Current>>,
    next_generation: AtomicU64,
}

impl AppState {
    /// Locks the drive list, reading the system drives on first use.
    pub fn drives(&self) -> MutexGuard<'_, Vec<Drive>> {
        let mut drives = lock(&self.drives);
        if drives.is_empty() {
            *drives = scanner_core::list_drives().into_iter().map(Drive::new).collect();
        }
        drives
    }

    pub fn current(&self) -> MutexGuard<'_, Option<Current>> {
        lock(&self.current)
    }

    pub fn next_generation(&self) -> u64 {
        self.next_generation.fetch_add(1, Ordering::Relaxed) + 1
    }
}

/// A panic in one command must not take the whole UI down with poisoned locks.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scanner_core::DriveKind;

    fn info() -> DriveInfo {
        DriveInfo {
            root: "C:\\".into(),
            label: "System".into(),
            file_system: "NTFS".into(),
            kind: DriveKind::Fixed,
            capacity: 1000,
            free: 300,
            physical_disk: Some(0),
            ready: true,
        }
    }

    #[test]
    fn placeholder_covers_capacity() {
        let d = Drive::new(info());
        assert_eq!(d.tree.allocated(NodeId::ROOT), 1000);
        assert!(!d.scanned);
    }

    #[test]
    fn completed_tree_covers_capacity_and_shows_the_gap() {
        let mut t = Tree::new("C:\\");
        t.add_file(NodeId::ROOT, "a", 600, 600);
        complete_drive_tree(&mut t, &info());
        assert_eq!(t.allocated(NodeId::ROOT), 1000);
        let kinds: Vec<_> = t.children_by_size(NodeId::ROOT).into_iter().map(|c| (t.kind(c), t.allocated(c))).collect();
        assert_eq!(kinds, [(NodeKind::File, 600), (NodeKind::FreeSpace, 300), (NodeKind::Unscanned, 100)]);
    }

    #[test]
    fn scan_larger_than_os_usage_adds_no_gap_block() {
        // Hard links are counted once per name, so a scan can exceed the OS figure.
        let mut t = Tree::new("C:\\");
        t.add_file(NodeId::ROOT, "a", 800, 800);
        complete_drive_tree(&mut t, &info());
        assert!(t.children(NodeId::ROOT).all(|c| t.kind(c) != NodeKind::Unscanned));
    }
}
