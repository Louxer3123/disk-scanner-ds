//! Compact arena tree holding scan results.
//!
//! Nodes live in one `Vec` and point at each other by `u32` index; names live in
//! one shared string pool. Sizes and counts are propagated to every ancestor as
//! nodes are added, so the tree is consistent at any moment during a scan.

use std::ops::{BitOr, BitOrAssign};
use std::path::PathBuf;

const NONE: u32 = u32::MAX;

/// Index of a node inside a [`Tree`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

impl NodeId {
    pub const ROOT: NodeId = NodeId(0);

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeKind {
    Dir,
    File,
    /// Symlink, junction or mount point. It is not followed, so it takes no space.
    Link,
    /// Synthetic block: free space of the drive.
    FreeSpace,
    /// Synthetic block: space the OS reports as used but the scan did not see
    /// (file system metadata, unreadable folders).
    Unscanned,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Dir => "dir",
            NodeKind::File => "file",
            NodeKind::Link => "link",
            NodeKind::FreeSpace => "free",
            NodeKind::Unscanned => "unscanned",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NodeFlags(u8);

impl NodeFlags {
    /// The directory could not be listed; its size is incomplete.
    pub const ACCESS_DENIED: NodeFlags = NodeFlags(1);
    /// The system recycle bin folder in the root of a volume.
    pub const RECYCLE_BIN: NodeFlags = NodeFlags(2);
    /// The on-disk name is not valid Unicode and was stored lossily,
    /// so [`Tree::path`] does not point at the real file.
    pub const LOSSY_NAME: NodeFlags = NodeFlags(4);

    const NAMES: [(NodeFlags, &'static str); 3] = [
        (NodeFlags::ACCESS_DENIED, "access_denied"),
        (NodeFlags::RECYCLE_BIN, "recycle_bin"),
        (NodeFlags::LOSSY_NAME, "lossy_name"),
    ];

    pub const fn empty() -> Self {
        NodeFlags(0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn contains(self, other: NodeFlags) -> bool {
        self.0 & other.0 == other.0
    }

    /// Snake-case names of the set flags, for output formats.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Self::NAMES
            .into_iter()
            .filter(move |&(flag, _)| self.contains(flag))
            .map(|(_, name)| name)
    }
}

impl BitOr for NodeFlags {
    type Output = NodeFlags;

    fn bitor(self, rhs: NodeFlags) -> NodeFlags {
        NodeFlags(self.0 | rhs.0)
    }
}

impl BitOrAssign for NodeFlags {
    fn bitor_assign(&mut self, rhs: NodeFlags) {
        self.0 |= rhs.0;
    }
}

#[derive(Clone, Debug)]
struct Node {
    /// Space taken on disk, summed over the subtree.
    allocated: u64,
    /// File lengths, summed over the subtree.
    logical: u64,
    parent: u32,
    first_child: u32,
    next_sibling: u32,
    name_start: u32,
    /// Files in the subtree (1 for a file node).
    files: u32,
    /// Directories in the subtree, not counting the node itself.
    dirs: u32,
    name_len: u16,
    kind: NodeKind,
    flags: NodeFlags,
}

#[derive(Clone, Debug)]
pub struct Tree {
    nodes: Vec<Node>,
    names: String,
    /// Kept apart from the pool because a full path can exceed the `u16` name length.
    root_name: String,
}

impl Tree {
    /// Creates a tree with a single directory node named `root_name`,
    /// normally the full path of the scanned folder.
    pub fn new(root_name: &str) -> Self {
        let root = Node {
            allocated: 0,
            logical: 0,
            parent: NONE,
            first_child: NONE,
            next_sibling: NONE,
            name_start: 0,
            files: 0,
            dirs: 0,
            name_len: 0,
            kind: NodeKind::Dir,
            flags: NodeFlags::empty(),
        };
        Tree {
            nodes: vec![root],
            names: String::new(),
            root_name: root_name.to_owned(),
        }
    }

    pub fn add_dir(&mut self, parent: NodeId, name: &str) -> NodeId {
        let id = self.push(parent, name, NodeKind::Dir);
        self.propagate(parent, |n| n.dirs += 1);
        id
    }

    pub fn add_file(&mut self, parent: NodeId, name: &str, logical: u64, allocated: u64) -> NodeId {
        let id = self.push(parent, name, NodeKind::File);
        let node = &mut self.nodes[id.index()];
        node.logical = logical;
        node.allocated = allocated;
        node.files = 1;
        self.propagate(parent, |n| {
            n.logical += logical;
            n.allocated += allocated;
            n.files += 1;
        });
        id
    }

    pub fn add_link(&mut self, parent: NodeId, name: &str) -> NodeId {
        self.push(parent, name, NodeKind::Link)
    }

    /// Adds a synthetic block of `size` bytes ([`NodeKind::FreeSpace`] or
    /// [`NodeKind::Unscanned`]). It counts towards allocated size only, not towards
    /// file lengths or file counts.
    pub fn add_space(&mut self, parent: NodeId, name: &str, kind: NodeKind, size: u64) -> NodeId {
        assert!(
            matches!(kind, NodeKind::FreeSpace | NodeKind::Unscanned),
            "add_space takes a space kind, got {kind:?}"
        );
        let id = self.push(parent, name, kind);
        self.nodes[id.index()].allocated = size;
        self.propagate(parent, |n| n.allocated += size);
        id
    }

    /// Turns an index from [`NodeId::index`] back into an id, if it is in range.
    pub fn node_id(&self, index: usize) -> Option<NodeId> {
        (index < self.nodes.len()).then_some(NodeId(index as u32))
    }

    pub fn insert_flags(&mut self, id: NodeId, flags: NodeFlags) {
        self.nodes[id.index()].flags |= flags;
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn name(&self, id: NodeId) -> &str {
        if id == NodeId::ROOT {
            return &self.root_name;
        }
        let n = &self.nodes[id.index()];
        let start = n.name_start as usize;
        &self.names[start..start + n.name_len as usize]
    }

    pub fn kind(&self, id: NodeId) -> NodeKind {
        self.nodes[id.index()].kind
    }

    pub fn flags(&self, id: NodeId) -> NodeFlags {
        self.nodes[id.index()].flags
    }

    pub fn allocated(&self, id: NodeId) -> u64 {
        self.nodes[id.index()].allocated
    }

    pub fn logical(&self, id: NodeId) -> u64 {
        self.nodes[id.index()].logical
    }

    pub fn files(&self, id: NodeId) -> u64 {
        self.nodes[id.index()].files.into()
    }

    pub fn dirs(&self, id: NodeId) -> u64 {
        self.nodes[id.index()].dirs.into()
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        match self.nodes[id.index()].parent {
            NONE => None,
            p => Some(NodeId(p)),
        }
    }

    /// Children in no particular order.
    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            tree: self,
            next: self.nodes[id.index()].first_child,
        }
    }

    /// Children sorted by allocated size, largest first; ties by name.
    pub fn children_by_size(&self, id: NodeId) -> Vec<NodeId> {
        let mut kids: Vec<NodeId> = self.children(id).collect();
        kids.sort_by(|&a, &b| {
            self.allocated(b)
                .cmp(&self.allocated(a))
                .then_with(|| self.name(a).cmp(self.name(b)))
        });
        kids
    }

    /// Full path of the node, starting from the root name.
    pub fn path(&self, id: NodeId) -> PathBuf {
        let mut chain = Vec::new();
        let mut cur = id;
        while let Some(parent) = self.parent(cur) {
            chain.push(cur);
            cur = parent;
        }
        let mut path = PathBuf::from(&self.root_name);
        for &n in chain.iter().rev() {
            path.push(self.name(n));
        }
        path
    }

    fn push(&mut self, parent: NodeId, name: &str, kind: NodeKind) -> NodeId {
        let id = u32::try_from(self.nodes.len())
            .ok()
            .filter(|&i| i != NONE)
            .expect("tree is limited to u32::MAX - 1 nodes");
        let name_start = u32::try_from(self.names.len()).expect("name pool exceeds 4 GiB");
        let name_len = u16::try_from(name.len()).expect("file name longer than 64 KiB");

        let p = &mut self.nodes[parent.index()];
        assert_eq!(p.kind, NodeKind::Dir, "children can only be added to directories");
        let next_sibling = std::mem::replace(&mut p.first_child, id);

        self.names.push_str(name);
        self.nodes.push(Node {
            allocated: 0,
            logical: 0,
            parent: parent.0,
            first_child: NONE,
            next_sibling,
            name_start,
            files: 0,
            dirs: 0,
            name_len,
            kind,
            flags: NodeFlags::empty(),
        });
        NodeId(id)
    }

    /// Applies `f` to `id` and every ancestor of it.
    fn propagate(&mut self, id: NodeId, f: impl Fn(&mut Node)) {
        let mut cur = id.0;
        while cur != NONE {
            let n = &mut self.nodes[cur as usize];
            f(n);
            cur = n.parent;
        }
    }
}

pub struct Children<'a> {
    tree: &'a Tree,
    next: u32,
}

impl Iterator for Children<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        if self.next == NONE {
            return None;
        }
        let id = NodeId(self.next);
        self.next = self.tree.nodes[id.index()].next_sibling;
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// root/
    ///   a.txt (100 / 4096)
    ///   sub/
    ///     b.bin (5000 / 8192)
    ///     deep/
    ///       c (0 / 0)
    ///   link -> elsewhere
    fn sample() -> (Tree, NodeId, NodeId, NodeId) {
        let mut t = Tree::new("C:\\data");
        t.add_file(NodeId::ROOT, "a.txt", 100, 4096);
        let sub = t.add_dir(NodeId::ROOT, "sub");
        t.add_file(sub, "b.bin", 5000, 8192);
        let deep = t.add_dir(sub, "deep");
        let c = t.add_file(deep, "c", 0, 0);
        t.add_link(NodeId::ROOT, "link");
        (t, sub, deep, c)
    }

    #[test]
    fn sizes_and_counts_propagate_to_all_ancestors() {
        let (t, sub, deep, _) = sample();
        assert_eq!(t.allocated(NodeId::ROOT), 4096 + 8192);
        assert_eq!(t.logical(NodeId::ROOT), 5100);
        assert_eq!(t.files(NodeId::ROOT), 3);
        assert_eq!(t.dirs(NodeId::ROOT), 2);
        assert_eq!(t.allocated(sub), 8192);
        assert_eq!(t.files(sub), 2);
        assert_eq!(t.dirs(sub), 1);
        assert_eq!(t.files(deep), 1);
        assert_eq!(t.dirs(deep), 0);
    }

    #[test]
    fn children_are_sorted_by_size_then_name() {
        let (t, sub, _, _) = sample();
        let names: Vec<&str> = t.children_by_size(NodeId::ROOT).into_iter().map(|c| t.name(c)).collect();
        assert_eq!(names, ["sub", "a.txt", "link"]);
        assert_eq!(t.children(sub).count(), 2);
    }

    #[test]
    fn path_and_parent_walk_up_to_root() {
        let (t, sub, deep, c) = sample();
        assert_eq!(t.path(c), PathBuf::from("C:\\data").join("sub").join("deep").join("c"));
        assert_eq!(t.path(NodeId::ROOT), PathBuf::from("C:\\data"));
        assert_eq!(t.parent(c), Some(deep));
        assert_eq!(t.parent(deep), Some(sub));
        assert_eq!(t.parent(NodeId::ROOT), None);
        assert_eq!(t.name(NodeId::ROOT), "C:\\data");
    }

    #[test]
    fn kinds_and_flags() {
        let (mut t, sub, _, c) = sample();
        let link = t.children(NodeId::ROOT).find(|&n| t.name(n) == "link").unwrap();
        assert_eq!(t.kind(link), NodeKind::Link);
        assert_eq!(t.allocated(link), 0);
        assert_eq!(t.kind(c), NodeKind::File);

        assert!(t.flags(sub).is_empty());
        t.insert_flags(sub, NodeFlags::ACCESS_DENIED);
        t.insert_flags(sub, NodeFlags::RECYCLE_BIN);
        let f = t.flags(sub);
        assert!(f.contains(NodeFlags::ACCESS_DENIED) && f.contains(NodeFlags::RECYCLE_BIN));
        assert!(!f.contains(NodeFlags::LOSSY_NAME));
        assert_eq!(f.names().collect::<Vec<_>>(), ["access_denied", "recycle_bin"]);
    }

    #[test]
    fn space_blocks_count_only_towards_allocated() {
        let (mut t, _, _, _) = sample();
        let before = (t.allocated(NodeId::ROOT), t.logical(NodeId::ROOT), t.files(NodeId::ROOT));
        let free = t.add_space(NodeId::ROOT, "Free space", NodeKind::FreeSpace, 1_000_000);
        t.add_space(NodeId::ROOT, "Unscanned", NodeKind::Unscanned, 500);
        assert_eq!(t.allocated(NodeId::ROOT), before.0 + 1_000_500);
        assert_eq!((t.logical(NodeId::ROOT), t.files(NodeId::ROOT)), (before.1, before.2));
        assert_eq!(t.kind(free), NodeKind::FreeSpace);
        assert_eq!(t.children_by_size(NodeId::ROOT)[0], free);
    }

    #[test]
    #[should_panic(expected = "space kind")]
    fn add_space_rejects_other_kinds() {
        Tree::new("/").add_space(NodeId::ROOT, "x", NodeKind::File, 1);
    }

    #[test]
    fn node_id_round_trips_and_checks_range() {
        let (t, _, _, c) = sample();
        assert_eq!(t.node_id(c.index()), Some(c));
        assert_eq!(t.node_id(t.node_count()), None);
    }

    #[test]
    fn unicode_names_round_trip() {
        let mut t = Tree::new("/");
        let d = t.add_dir(NodeId::ROOT, "Документы");
        let f = t.add_file(d, "фото 🌄.jpg", 1, 1);
        assert_eq!(t.name(d), "Документы");
        assert_eq!(t.name(f), "фото 🌄.jpg");
    }

    #[test]
    #[should_panic(expected = "only be added to directories")]
    fn adding_child_to_file_panics() {
        let mut t = Tree::new("/");
        let f = t.add_file(NodeId::ROOT, "f", 1, 1);
        t.add_file(f, "g", 1, 1);
    }

    #[test]
    fn node_stays_compact() {
        assert!(std::mem::size_of::<Node>() <= 48, "Node is {} bytes", std::mem::size_of::<Node>());
    }
}
