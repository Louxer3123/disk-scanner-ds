//! Summary, JSON and CSV writers.

use std::borrow::Cow;
use std::io::{self, Write};

use scanner_core::{NodeId, NodeKind, Scan, Tree, format_size};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

/// Which entries the JSON and CSV writers include.
#[derive(Clone, Copy)]
pub struct Filter {
    /// Deepest level to output; the root's children are level 1.
    pub max_depth: usize,
    pub min_size: u64,
}

impl Filter {
    fn children(self, tree: &Tree, id: NodeId, depth: usize) -> Vec<NodeId> {
        if depth >= self.max_depth {
            return Vec::new();
        }
        let mut kids = tree.children_by_size(id);
        kids.retain(|&c| tree.allocated(c) >= self.min_size);
        kids
    }
}

/// Replaces control characters (C0, DEL, C1), bidi controls and invisible spaces
/// so that a file name can neither inject terminal escape sequences nor disguise
/// itself (e.g. `U+202E` making `txt.exe` read as `exe.txt`). ZWJ and ZWNJ are kept:
/// emoji sequences and several scripts need them.
pub fn printable(s: &str) -> Cow<'_, str> {
    fn unsafe_char(c: char) -> bool {
        c.is_control()
            || matches!(
                c,
                '\u{061C}' | '\u{200B}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
            )
    }
    if s.chars().any(unsafe_char) {
        Cow::Owned(s.chars().map(|c| if unsafe_char(c) { '\u{FFFD}' } else { c }).collect())
    } else {
        Cow::Borrowed(s)
    }
}

pub fn write_summary(out: &mut impl Write, scan: &Scan, top: usize) -> io::Result<()> {
    let tree = &scan.tree;
    let s = &scan.stats;
    let root = NodeId::ROOT;
    let total = tree.allocated(root);

    writeln!(out, "{}", printable(tree.name(root)))?;
    writeln!(
        out,
        "{} used ({} in file lengths), {} files, {} folders",
        format_size(total),
        format_size(tree.logical(root)),
        s.files,
        s.dirs
    )?;
    writeln!(
        out,
        "cluster {} B, {} links skipped, {} unreadable, {:.2} s",
        s.cluster_size,
        s.links,
        s.errors,
        s.elapsed.as_secs_f64()
    )?;
    writeln!(out)?;

    let kids = tree.children_by_size(root);
    for &c in kids.iter().take(top) {
        let size = tree.allocated(c);
        let share = if total == 0 { 0.0 } else { size as f64 * 100.0 / total as f64 };
        let marker = match tree.kind(c) {
            NodeKind::Dir => std::path::MAIN_SEPARATOR_STR,
            NodeKind::File | NodeKind::FreeSpace | NodeKind::Unscanned => "",
            NodeKind::Link => " ->",
        };
        let mut flags = tree.flags(c).names().collect::<Vec<_>>().join(", ");
        if !flags.is_empty() {
            flags = format!("  [{flags}]");
        }
        writeln!(out, "{:>10} {share:>5.1}%  {}{marker}{flags}", format_size(size), printable(tree.name(c)))?;
    }
    if kids.len() > top {
        writeln!(out, "… and {} more", kids.len() - top)?;
    }
    Ok(())
}

pub fn write_json(out: &mut impl Write, scan: &Scan, filter: Filter) -> io::Result<()> {
    let s = &scan.stats;
    let stats = serde_json::json!({
        "files": s.files,
        "dirs": s.dirs,
        "links": s.links,
        "errors": s.errors,
        "cluster_size": s.cluster_size,
        "elapsed_ms": s.elapsed.as_millis() as u64,
        "cancelled": s.cancelled,
    });
    // The tree can be huge, so it is streamed rather than built as a `Value`.
    let mut ser = serde_json::Serializer::new(&mut *out);
    let mut doc = ser.serialize_map(Some(3))?;
    doc.serialize_entry("root", scan.tree.name(NodeId::ROOT))?;
    doc.serialize_entry("stats", &stats)?;
    doc.serialize_entry("tree", &JsonNode { tree: &scan.tree, id: NodeId::ROOT, depth: 0, filter })?;
    SerializeMap::end(doc)?;
    writeln!(out)
}

struct JsonNode<'a> {
    tree: &'a Tree,
    id: NodeId,
    depth: usize,
    filter: Filter,
}

impl Serialize for JsonNode<'_> {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let (tree, id) = (self.tree, self.id);
        let kind = tree.kind(id);
        let mut map = ser.serialize_map(None)?;
        map.serialize_entry("name", tree.name(id))?;
        map.serialize_entry("kind", kind.as_str())?;
        map.serialize_entry("allocated", &tree.allocated(id))?;
        map.serialize_entry("logical", &tree.logical(id))?;
        if kind == NodeKind::Dir {
            map.serialize_entry("files", &tree.files(id))?;
            map.serialize_entry("dirs", &tree.dirs(id))?;
        }
        let flags: Vec<&str> = tree.flags(id).names().collect();
        if !flags.is_empty() {
            map.serialize_entry("flags", &flags)?;
        }
        if kind == NodeKind::Dir && self.depth < self.filter.max_depth {
            map.serialize_entry("children", &JsonChildren { parent: self })?;
        }
        map.end()
    }
}

struct JsonChildren<'a, 'b> {
    parent: &'b JsonNode<'a>,
}

impl Serialize for JsonChildren<'_, '_> {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let p = self.parent;
        let kids = p.filter.children(p.tree, p.id, p.depth);
        let mut seq = ser.serialize_seq(Some(kids.len()))?;
        for id in kids {
            seq.serialize_element(&JsonNode { tree: p.tree, id, depth: p.depth + 1, filter: p.filter })?;
        }
        seq.end()
    }
}

pub fn write_csv(out: &mut impl Write, tree: &Tree, filter: Filter) -> io::Result<()> {
    writeln!(out, "path,kind,allocated,logical,files,dirs,flags")?;
    // Explicit stack: pre-order, largest first, without recursion.
    let mut stack = vec![(NodeId::ROOT, 0usize)];
    while let Some((id, depth)) = stack.pop() {
        let flags = tree.flags(id).names().collect::<Vec<_>>().join(";");
        writeln!(
            out,
            "{},{},{},{},{},{},{}",
            csv_field(&tree.path(id).to_string_lossy()),
            tree.kind(id).as_str(),
            tree.allocated(id),
            tree.logical(id),
            tree.files(id),
            tree.dirs(id),
            flags
        )?;
        let kids = filter.children(tree, id, depth);
        stack.extend(kids.into_iter().rev().map(|c| (c, depth + 1)));
    }
    Ok(())
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{csv_field, printable};

    #[test]
    fn printable_replaces_control_characters() {
        assert_eq!(printable("plain имя.txt"), "plain имя.txt");
        assert_eq!(printable("a\x1b[2Jb\u{9b}c\x7f"), "a\u{FFFD}[2Jb\u{FFFD}c\u{FFFD}");
        assert_eq!(printable("invoice\u{202E}fdp.exe"), "invoice\u{FFFD}fdp.exe");
        let family = "👨\u{200D}👩\u{200D}👧.jpg";
        assert_eq!(printable(family), family);
    }

    #[test]
    fn csv_quotes_only_when_needed() {
        assert_eq!(csv_field(r"C:\plain"), r"C:\plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}
