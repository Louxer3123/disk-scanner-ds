//! Drives, their scan results and the layout currently shown.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use scanner_core::diff::Diff;
use scanner_core::filetype::{Category, dominant_categories};
use scanner_core::search::{Query, mark_matches};
use scanner_core::{CancelToken, DriveInfo, NodeId, NodeKind, SunburstLayout, Tree};
use serde::{Deserialize, Serialize};

pub struct Drive {
    pub info: DriveInfo,
    /// Scan result, or a placeholder with the OS-reported used and free space.
    pub tree: Arc<Tree>,
    pub scanned: bool,
    /// `false` once the drive letter disappeared. The slot is kept so that drive
    /// indices stay stable; it is reused if the same letter comes back.
    pub present: bool,
    /// How the last full scan read the drive: `mft` or `walk`.
    pub scan_method: Option<&'static str>,
    /// Set while a scan of this drive runs.
    pub cancel: Option<CancelToken>,
    /// Bumped whenever `tree` changes, so node ids from an older tree are
    /// re-resolved by path instead of trusted.
    pub tree_gen: u64,
    /// The snapshot this drive is compared with, if any.
    pub comparison: Option<Arc<Comparison>>,
}

/// A loaded snapshot and how the drive's current tree differs from it.
pub struct Comparison {
    pub id: String,
    /// When the snapshot was saved, Unix seconds.
    pub created: u64,
    pub old: Arc<Tree>,
    /// The diff against one tree generation; recomputed when the tree changes.
    diff: Mutex<Option<(u64, Arc<Diff>)>>,
}

impl Comparison {
    pub fn new(id: String, created: u64, old: Tree) -> Self {
        Comparison { id, created, old: Arc::new(old), diff: Mutex::new(None) }
    }

    pub fn diff(&self, tree: &Tree, tree_gen: u64) -> Arc<Diff> {
        let mut cached = lock(&self.diff);
        match cached.as_ref() {
            Some((g, d)) if *g == tree_gen => d.clone(),
            _ => {
                let d = Arc::new(Diff::compute(&self.old, tree));
                *cached = Some((tree_gen, d.clone()));
                d
            }
        }
    }
}

impl Drive {
    pub fn new(info: DriveInfo) -> Self {
        let mut tree = Tree::new(&info.root);
        refresh_space_blocks(&mut tree, &info);
        Drive {
            info,
            tree: Arc::new(tree),
            scanned: false,
            present: true,
            scan_method: None,
            cancel: None,
            tree_gen: 0,
            comparison: None,
        }
    }

    /// Forgets the scan and starts over with `info` (new media, or the letter came
    /// back), keeping the generation counter increasing.
    pub fn reset(&mut self, info: DriveInfo, present: bool) {
        if let Some(token) = self.cancel.take() {
            token.cancel();
        }
        let tree_gen = self.tree_gen + 1;
        *self = Drive { present, tree_gen, ..Drive::new(info) };
    }
}

/// Makes the root's free-space and unscanned blocks match the drive's current
/// usage, creating them if needed, so the root covers the whole capacity.
pub fn refresh_space_blocks(tree: &mut Tree, info: &DriveInfo) {
    let find = |tree: &Tree, kind| tree.children(NodeId::ROOT).find(|&c| tree.kind(c) == kind);
    let free = find(tree, NodeKind::FreeSpace)
        .unwrap_or_else(|| tree.add_space(NodeId::ROOT, "Free space", NodeKind::FreeSpace, 0));
    let unscanned = find(tree, NodeKind::Unscanned)
        .unwrap_or_else(|| tree.add_space(NodeId::ROOT, "Unscanned", NodeKind::Unscanned, 0));
    let scanned = tree.allocated(NodeId::ROOT) - tree.allocated(free) - tree.allocated(unscanned);
    tree.set_space_size(free, info.free);
    // Hard links are counted once per name, so a scan can exceed the OS figure;
    // then there is simply no gap to show.
    tree.set_space_size(unscanned, info.used().saturating_sub(scanned));
}

/// A node as the UI refers to it. When `tree_gen` is outdated the node is found
/// again by `path`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeRef {
    pub drive: usize,
    pub node: u32,
    pub tree_gen: u64,
    #[serde(default)]
    pub path: String,
}

/// Names below the drive root, e.g. `C:\Users\me` → `["Users", "me"]`.
/// `None` if `path` is not on the drive.
pub fn rel_components(root: &str, path: &str) -> Option<Vec<String>> {
    let root = root.trim_end_matches('\\');
    let head = path.get(..root.len())?;
    if !head.eq_ignore_ascii_case(root) {
        return None;
    }
    let rest = &path[root.len()..];
    if !(rest.is_empty() || rest.starts_with('\\')) {
        return None;
    }
    Some(rest.split('\\').filter(|c| !c.is_empty()).map(str::to_owned).collect())
}

impl Drive {
    /// Finds the node `r` refers to: by id if the tree has not changed since, else
    /// by path. With `exact`, a path that no longer fully exists yields `None`;
    /// otherwise its deepest existing ancestor is returned.
    pub fn resolve(&self, r: &NodeRef, exact: bool) -> Option<NodeId> {
        if r.tree_gen == self.tree_gen {
            return self.tree.node_id(r.node as usize);
        }
        let comps = rel_components(&self.info.root, &r.path)?;
        let (id, matched) = self.tree.find_path(&comps);
        (!exact || matched == comps.len()).then_some(id)
    }
}

/// The layout last sent to the UI, kept for hit testing.
pub struct Current {
    pub generation: u64,
    pub layout: SunburstLayout,
    /// `(drive index, tree, tree generation)` for each layout part.
    pub parts: Vec<(usize, Arc<Tree>, u64)>,
}

/// Search marks of one drive tree for one query.
struct CachedMarks {
    drive: usize,
    tree_gen: u64,
    query: Query,
    marks: Arc<Vec<u8>>,
}

/// File type of every node of one drive tree.
struct CachedTypes {
    drive: usize,
    tree_gen: u64,
    categories: Arc<Vec<Category>>,
}

/// Enough for a few drives times the last couple of queries.
const SEARCH_CACHE_SIZE: usize = 8;

#[derive(Default)]
pub struct AppState {
    drives: Mutex<Vec<Drive>>,
    current: Mutex<Option<Current>>,
    next_generation: AtomicU64,
    search_cache: Mutex<Vec<CachedMarks>>,
    type_cache: Mutex<Vec<CachedTypes>>,
    /// Folder passed on the command line (e.g. from the Explorer context menu).
    pub startup_path: Option<String>,
}

impl AppState {
    pub fn new(startup_path: Option<String>) -> Self {
        AppState { startup_path, ..Default::default() }
    }

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

    /// Search marks (see [`mark_matches`]) of a drive's tree, computed once per tree
    /// generation and query, so redrawing or zooming does not search again.
    pub fn search_marks(&self, drive: usize, tree: &Tree, tree_gen: u64, query: &Query) -> Arc<Vec<u8>> {
        let hit = |e: &CachedMarks| e.drive == drive && e.tree_gen == tree_gen && e.query == *query;
        if let Some(e) = lock(&self.search_cache).iter().find(|e| hit(e)) {
            return e.marks.clone();
        }
        // Searched without the lock held: it can take a while on a big drive.
        let marks = Arc::new(mark_matches(tree, query));
        let mut cache = lock(&self.search_cache);
        cache.retain(|e| !(e.drive == drive && e.tree_gen != tree_gen));
        cache.push(CachedMarks { drive, tree_gen, query: query.clone(), marks: marks.clone() });
        if cache.len() > SEARCH_CACHE_SIZE {
            cache.remove(0);
        }
        marks
    }

    /// File type of every node of a drive's tree (see [`dominant_categories`]),
    /// computed once per tree generation.
    pub fn categories(&self, drive: usize, tree: &Tree, tree_gen: u64) -> Arc<Vec<Category>> {
        if let Some(e) = lock(&self.type_cache).iter().find(|e| e.drive == drive && e.tree_gen == tree_gen) {
            return e.categories.clone();
        }
        let categories = Arc::new(dominant_categories(tree));
        let mut cache = lock(&self.type_cache);
        cache.retain(|e| e.drive != drive);
        cache.push(CachedTypes { drive, tree_gen, categories: categories.clone() });
        categories
    }

    /// Changes a drive's tree in place: re-reads the drive's usage, runs `f`,
    /// refreshes the space blocks and bumps the tree generation.
    pub fn mutate_tree<R>(&self, drive: usize, f: impl FnOnce(&mut Tree, &DriveInfo) -> R) -> Result<R, String> {
        // The shown layout holds the tree too; letting it go avoids copying the
        // whole tree in `make_mut`. The UI re-lays out after any change anyway.
        *self.current() = None;
        let mut drives = self.drives();
        let d = drives.get_mut(drive).ok_or("no such drive")?;
        if let Some(info) = scanner_core::drive_info(&d.info.root) {
            d.info = info;
        }
        let tree = Arc::make_mut(&mut d.tree);
        let result = f(tree, &d.info);
        refresh_space_blocks(tree, &d.info);
        d.tree_gen += 1;
        Ok(result)
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
            rotational: Some(false),
            ready: true,
        }
    }

    fn space(t: &Tree, kind: NodeKind) -> u64 {
        t.children(NodeId::ROOT).filter(|&c| t.kind(c) == kind).map(|c| t.allocated(c)).sum()
    }

    #[test]
    fn placeholder_covers_capacity() {
        let d = Drive::new(info());
        assert_eq!(d.tree.allocated(NodeId::ROOT), 1000);
        assert_eq!(space(&d.tree, NodeKind::Unscanned), 700);
        assert!(!d.scanned);
    }

    #[test]
    fn space_blocks_fill_the_gap_and_follow_changes() {
        let mut t = Tree::new("C:\\");
        let a = t.add_file(NodeId::ROOT, "a", 600, 600);
        refresh_space_blocks(&mut t, &info());
        assert_eq!(t.allocated(NodeId::ROOT), 1000);
        assert_eq!((space(&t, NodeKind::FreeSpace), space(&t, NodeKind::Unscanned)), (300, 100));

        // Deleting `a` frees its space on disk; the blocks are updated, not duplicated.
        t.detach(a);
        refresh_space_blocks(&mut t, &DriveInfo { free: 900, ..info() });
        assert_eq!((space(&t, NodeKind::FreeSpace), space(&t, NodeKind::Unscanned)), (900, 100));
        assert_eq!(t.children(NodeId::ROOT).count(), 2);
    }

    #[test]
    fn scan_larger_than_os_usage_leaves_no_gap() {
        let mut t = Tree::new("C:\\");
        t.add_file(NodeId::ROOT, "a", 800, 800);
        refresh_space_blocks(&mut t, &info());
        assert_eq!(space(&t, NodeKind::Unscanned), 0);
    }

    #[test]
    fn rel_components_strips_the_root() {
        assert_eq!(rel_components("C:\\", "C:\\Users\\me"), Some(vec!["Users".into(), "me".into()]));
        assert_eq!(rel_components("C:\\", "c:\\users\\"), Some(vec!["users".into()]));
        assert_eq!(rel_components("C:\\", "C:\\"), Some(vec![]));
        assert_eq!(rel_components("C:\\", "C:"), Some(vec![]));
        assert_eq!(rel_components("C:\\", "D:\\x"), None);
        assert_eq!(rel_components("C:\\", "C:x"), None);
        assert_eq!(rel_components("C:\\", ""), None);
    }

    #[test]
    fn resolve_uses_id_then_path() {
        let mut d = Drive::new(info());
        let tree = Arc::make_mut(&mut d.tree);
        let users = tree.add_dir(NodeId::ROOT, "Users");
        let me = tree.add_dir(users, "me");
        let r = |node: NodeId, gen_: u64, path: &str| NodeRef { drive: 0, node: node.index() as u32, tree_gen: gen_, path: path.into() };

        assert_eq!(d.resolve(&r(me, 0, ""), true), Some(me));
        d.tree_gen = 1;
        assert_eq!(d.resolve(&r(me, 0, "C:\\Users\\me"), true), Some(me));
        assert_eq!(d.resolve(&r(me, 0, "C:\\Users\\gone\\x"), true), None);
        assert_eq!(d.resolve(&r(me, 0, "C:\\Users\\gone\\x"), false), Some(users));
        assert_eq!(d.resolve(&r(me, 0, ""), false), None);
    }
}
