//! Writing scan results as CSV (one row per entry) or JSON (a nested tree).
//! Both stream: a tree of millions of entries is never built in memory twice.

use std::io::{self, Write};

use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

use crate::platform::display_path;
use crate::tree::{NodeId, NodeKind, Tree};

/// Which entries to write.
#[derive(Clone, Copy, Debug)]
pub struct Filter {
    /// Deepest level to write; the root's children are level 1.
    pub max_depth: usize,
    /// Entries taking less space are left out.
    pub min_size: u64,
}

impl Filter {
    pub const ALL: Filter = Filter { max_depth: usize::MAX, min_size: 0 };

    fn children(self, tree: &Tree, id: NodeId, depth: usize) -> Vec<NodeId> {
        if depth >= self.max_depth {
            return Vec::new();
        }
        let mut kids = tree.children_by_size(id);
        kids.retain(|&c| tree.allocated(c) >= self.min_size);
        kids
    }
}

/// CSV dialect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CsvStyle {
    /// RFC 4180: comma-separated UTF-8, for scripts.
    Plain,
    /// For Excel in a European locale: `;`-separated, with a UTF-8 byte order mark
    /// (without it Excel reads Cyrillic names as ANSI), and text that Excel would
    /// take for a formula (`=`, `+`, `-`, `@` first) prefixed with `'`.
    Excel,
}

impl CsvStyle {
    fn separator(self) -> char {
        match self {
            CsvStyle::Plain => ',',
            CsvStyle::Excel => ';',
        }
    }
}

/// Writes the entries below each `(tree, root)` (roots included), largest first,
/// in pre-order, one row each.
pub fn write_csv(out: &mut impl Write, parts: &[(&Tree, NodeId)], filter: Filter, style: CsvStyle) -> io::Result<()> {
    let sep = style.separator();
    if style == CsvStyle::Excel {
        out.write_all("\u{FEFF}".as_bytes())?;
    }
    let header = ["path", "kind", "allocated", "logical", "files", "dirs", "flags"];
    writeln!(out, "{}", header.join(&sep.to_string()))?;
    for &(tree, root) in parts {
        // Explicit stack: folders can nest deeper than recursion allows.
        let mut stack = vec![(root, 0usize)];
        while let Some((id, depth)) = stack.pop() {
            let path = display_path(&tree.path(id)).to_string_lossy().into_owned();
            let flags = tree.flags(id).names().collect::<Vec<_>>().join("|");
            writeln!(
                out,
                "{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}",
                csv_field(&path, style),
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
    }
    Ok(())
}

fn csv_field(s: &str, style: CsvStyle) -> String {
    let formula = style == CsvStyle::Excel && s.starts_with(['=', '+', '-', '@', '\t', '\r']);
    let s = if formula { format!("'{s}") } else { s.to_owned() };
    if s.contains([style.separator(), '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s }
}

/// The subtree at `id` as a JSON value that serializes lazily: name, kind,
/// sizes, counts for folders, flags if any, and children largest first.
pub fn json_tree(tree: &Tree, id: NodeId, filter: Filter) -> impl Serialize + '_ {
    JsonNode { tree, id, depth: 0, filter }
}

/// Writes `{"exported": <Unix seconds>, "roots": [<tree>, ...]}`.
pub fn write_json(out: &mut impl Write, parts: &[(&Tree, NodeId)], filter: Filter, exported: u64) -> io::Result<()> {
    let mut ser = serde_json::Serializer::new(&mut *out);
    let mut doc = ser.serialize_map(Some(2))?;
    doc.serialize_entry("exported", &exported)?;
    doc.serialize_entry("roots", &JsonRoots { parts, filter })?;
    SerializeMap::end(doc)?;
    writeln!(out)
}

struct JsonRoots<'a> {
    parts: &'a [(&'a Tree, NodeId)],
    filter: Filter,
}

impl Serialize for JsonRoots<'_> {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let mut seq = ser.serialize_seq(Some(self.parts.len()))?;
        for &(tree, id) in self.parts {
            seq.serialize_element(&JsonNode { tree, id, depth: 0, filter: self.filter })?;
        }
        seq.end()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Tree {
        let mut t = Tree::new("D:\\");
        let docs = t.add_dir(NodeId::ROOT, "Документы");
        t.add_file(docs, "отчёт, итог.xlsx", 900, 4096);
        t.add_file(docs, "=cmd.txt", 10, 4096);
        let deep = t.add_dir(docs, "deep");
        t.add_file(deep, "small", 1, 100);
        t.add_file(NodeId::ROOT, "big.iso", 20_000, 20_480);
        t
    }

    fn csv(t: &Tree, root: NodeId, filter: Filter, style: CsvStyle) -> String {
        let mut out = Vec::new();
        write_csv(&mut out, &[(t, root)], filter, style).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn plain_csv_lists_every_entry_largest_first() {
        let t = sample();
        let text = csv(&t, NodeId::ROOT, Filter::ALL, CsvStyle::Plain);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "path,kind,allocated,logical,files,dirs,flags");
        assert!(lines[1].starts_with("D:\\,dir,28772,"));
        assert!(lines[2].starts_with("D:\\big.iso,file,20480,20000,1,0,"));
        assert!(lines.iter().any(|l| l.starts_with("\"D:\\Документы\\отчёт, итог.xlsx\",file,")));
        assert!(lines.iter().any(|l| l.starts_with("D:\\Документы\\=cmd.txt,")), "plain CSV keeps names as they are");
        assert_eq!(lines.len(), 1 + 7);
    }

    #[test]
    fn excel_csv_has_bom_semicolons_and_no_formulas() {
        let t = sample();
        let text = csv(&t, NodeId::ROOT, Filter::ALL, CsvStyle::Excel);
        assert!(text.starts_with("\u{FEFF}path;kind;"));
        assert!(text.contains("D:\\Документы\\отчёт, итог.xlsx;file;4096;"), "commas need no quotes here");
        // A path starts with the drive letter, but a lone name could start with `=`.
        assert_eq!(csv_field("=SUM(A1)", CsvStyle::Excel), "'=SUM(A1)");
        assert_eq!(csv_field("a;b", CsvStyle::Excel), "\"a;b\"");
        assert_eq!(csv_field("-x", CsvStyle::Plain), "-x");
    }

    #[test]
    fn filters_limit_depth_and_size_from_any_root() {
        let t = sample();
        let docs = t.find_path(&["Документы"]).0;
        let text = csv(&t, docs, Filter { max_depth: 1, min_size: 200 }, CsvStyle::Plain);
        let paths: Vec<&str> = text.lines().skip(1).map(|l| l.split(',').next().unwrap()).collect();
        // The folder and its two files; `deep` (100 B) is below the minimum.
        assert_eq!(paths.len(), 3, "{paths:?}");
        assert!(paths.iter().all(|p| !p.contains("deep")));
    }

    #[test]
    fn json_nests_children_and_lists_roots() {
        let t = sample();
        let mut out = Vec::new();
        write_json(&mut out, &[(&t, NodeId::ROOT)], Filter { max_depth: 1, min_size: 0 }, 42).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["exported"], 42);
        let root = &v["roots"][0];
        assert_eq!(root["name"], "D:\\");
        assert_eq!(root["children"][0]["name"], "big.iso");
        assert_eq!(root["children"][1]["files"], 3);
        assert!(root["children"][1].get("children").is_none(), "depth 1 stops here");
    }
}
