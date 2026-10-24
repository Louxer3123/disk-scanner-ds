//! Comparing two scans of the same folder: what grew, what is new, what is gone.

use std::collections::HashMap;

use crate::tree::{NodeId, NodeKind, Tree};

const NONE: u32 = u32::MAX;

/// How the nodes of a newer tree match those of an older one. Nodes match by
/// exact name and kind under matching parents, starting at the roots.
pub struct Diff {
    /// Per node of the new tree: the matching node of the old tree, or `NONE`.
    old_of: Vec<u32>,
    /// Old nodes without a match whose parent has one: the roots of what is gone.
    removed: Vec<NodeId>,
}

impl Diff {
    pub fn compute(old: &Tree, new: &Tree) -> Diff {
        let mut old_of = vec![NONE; new.node_count()];
        let mut removed = Vec::new();
        old_of[NodeId::ROOT.index()] = NodeId::ROOT.index() as u32;
        let mut by_name: HashMap<&str, NodeId> = HashMap::new();
        let mut stack = vec![(NodeId::ROOT, NodeId::ROOT)];
        while let Some((n, o)) = stack.pop() {
            by_name.clear();
            by_name.extend(old.children(o).map(|c| (old.name(c), c)));
            for c in new.children(n) {
                let Some(&oc) = by_name.get(new.name(c)) else { continue };
                if old.kind(oc) != new.kind(c) {
                    continue;
                }
                by_name.remove(new.name(c));
                old_of[c.index()] = oc.index() as u32;
                if new.kind(c) == NodeKind::Dir {
                    stack.push((c, oc));
                }
            }
            removed.extend(by_name.values().copied());
        }
        removed.sort_by_key(|&id| std::cmp::Reverse(old.allocated(id)));
        Diff { old_of, removed }
    }

    /// The node of the old tree that `id` (of the new tree) matches.
    pub fn old_node(&self, id: NodeId) -> Option<NodeId> {
        self.old_of.get(id.index()).filter(|&&o| o != NONE).map(|&o| NodeId::from_index(o))
    }

    /// Whether `id` did not exist in the old tree.
    pub fn is_added(&self, id: NodeId) -> bool {
        self.old_node(id).is_none()
    }

    /// Change of the space taken by `id`, in bytes: positive when it grew.
    pub fn delta(&self, old: &Tree, new: &Tree, id: NodeId) -> i64 {
        let before = self.old_node(id).map_or(0, |o| old.allocated(o));
        let d = i128::from(new.allocated(id)) - i128::from(before);
        d.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
    }

    /// Roots of what is gone, largest first, as nodes of the old tree; only
    /// those below `old_root` when given.
    pub fn removed(&self, old: &Tree, old_root: Option<NodeId>) -> Vec<NodeId> {
        self.removed
            .iter()
            .copied()
            .filter(|&id| old_root.is_none_or(|root| is_below(old, id, root)))
            .collect()
    }

    /// Files below `root` (of the new tree) that are new or grew, by growth,
    /// largest first, at most `limit`.
    pub fn grown_files(&self, old: &Tree, new: &Tree, root: NodeId, limit: usize) -> Vec<(NodeId, i64)> {
        let mut found = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            match new.kind(id) {
                NodeKind::Dir => stack.extend(new.children(id)),
                NodeKind::File => {
                    let d = self.delta(old, new, id);
                    if d > 0 {
                        found.push((id, d));
                    }
                }
                _ => {}
            }
        }
        if found.len() > limit && limit > 0 {
            found.select_nth_unstable_by(limit - 1, |a, b| b.1.cmp(&a.1));
        }
        found.truncate(limit);
        found.sort_by_key(|&(_, g)| std::cmp::Reverse(g));
        found
    }
}

fn is_below(tree: &Tree, mut id: NodeId, root: NodeId) -> bool {
    loop {
        if id == root {
            return true;
        }
        match tree.parent(id) {
            Some(p) => id = p,
            None => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Last week: Users/me with a movie and notes, Temp with a dump, a free block.
    fn old_tree() -> Tree {
        let mut t = Tree::new("C:\\");
        let users = t.add_dir(NodeId::ROOT, "Users");
        let me = t.add_dir(users, "me");
        t.add_file(me, "movie.mkv", 0, 4000);
        t.add_file(me, "notes.txt", 0, 100);
        let temp = t.add_dir(NodeId::ROOT, "Temp");
        t.add_file(temp, "crash.dmp", 0, 2000);
        t.add_dir(NodeId::ROOT, "data");
        t.add_space(NodeId::ROOT, "Free space", NodeKind::FreeSpace, 10_000);
        t
    }

    /// Today: notes grew, a game appeared, Temp is gone, `data` became a file.
    fn new_tree() -> Tree {
        let mut t = Tree::new("C:\\");
        let users = t.add_dir(NodeId::ROOT, "Users");
        let me = t.add_dir(users, "me");
        t.add_file(me, "movie.mkv", 0, 4000);
        t.add_file(me, "notes.txt", 0, 600);
        let games = t.add_dir(NodeId::ROOT, "Games");
        t.add_file(games, "game.pak", 0, 3000);
        t.add_file(NodeId::ROOT, "data", 0, 50);
        t.add_space(NodeId::ROOT, "Free space", NodeKind::FreeSpace, 8_000);
        t
    }

    #[test]
    fn matches_by_path_and_measures_growth() {
        let (old, new) = (old_tree(), new_tree());
        let d = Diff::compute(&old, &new);
        let at = |p: &[&str]| new.find_path(p).0;

        assert_eq!(d.delta(&old, &new, at(&["Users", "me", "notes.txt"])), 500);
        assert_eq!(d.delta(&old, &new, at(&["Users", "me", "movie.mkv"])), 0);
        assert_eq!(d.delta(&old, &new, at(&["Users"])), 500);
        assert!(d.is_added(at(&["Games"])) && d.is_added(at(&["Games", "game.pak"])));
        assert_eq!(d.delta(&old, &new, at(&["Games"])), 3000);
        assert!(d.is_added(at(&["data"])), "a file where a folder was is new");
        assert_eq!(d.delta(&old, &new, at(&["Free space"])), -2000);
        // Root: +500 +3000 +50 -2000 (Temp) -2000 (free).
        assert_eq!(d.delta(&old, &new, NodeId::ROOT), -450);
    }

    #[test]
    fn lists_what_is_gone_and_what_grew() {
        let (old, new) = (old_tree(), new_tree());
        let d = Diff::compute(&old, &new);
        let gone: Vec<&str> = d.removed(&old, None).iter().map(|&id| old.name(id)).collect();
        assert_eq!(gone, ["Temp", "data"], "largest first, subtrees by their root only");
        let users = old.find_path(&["Users"]).0;
        assert!(d.removed(&old, Some(users)).is_empty());

        let grown: Vec<(&str, i64)> =
            d.grown_files(&old, &new, NodeId::ROOT, 10).iter().map(|&(id, g)| (new.name(id), g)).collect();
        assert_eq!(grown, [("game.pak", 3000), ("notes.txt", 500), ("data", 50)]);
        assert_eq!(d.grown_files(&old, &new, NodeId::ROOT, 1).len(), 1);
    }

    #[test]
    fn identical_trees_have_no_changes() {
        let t = old_tree();
        let d = Diff::compute(&t, &t);
        assert!(d.removed(&t, None).is_empty());
        let mut stack = vec![NodeId::ROOT];
        while let Some(id) = stack.pop() {
            assert_eq!(d.delta(&t, &t, id), 0);
            stack.extend(t.children(id));
        }
    }
}
