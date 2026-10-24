//! Snapshots: a scan result saved to a file, to compare with a later scan.
//!
//! Layout: a small uncompressed header (so a list of snapshots can be shown
//! without reading whole files), then the tree in pre-order, deflate-compressed.
//! Each node is its kind, flags and name, then by kind: a directory's child
//! count; a file's length and allocated size; a space block's size. Numbers are
//! LEB128 varints. Reading checks every count and length, so a damaged or
//! hostile file gives an error, not a panic or a huge allocation.

use std::io::{self, BufReader, BufWriter, Read, Write};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;

use crate::tree::{NodeFlags, NodeId, NodeKind, Tree};

const MAGIC: &[u8; 8] = b"DSKSNAP\0";
const VERSION: u32 = 1;
/// Longest name stored: NTFS allows 255 UTF-16 units, up to 765 UTF-8 bytes.
const MAX_NAME: u64 = 4096;
const MAX_ROOT_NAME: u64 = 64 * 1024;
/// Limits what a damaged header can make the reader reserve up front.
const MAX_PREALLOC: u64 = 1 << 20;

/// What a snapshot holds, readable without loading the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotHeader {
    /// Seconds since the Unix epoch when the scan was saved.
    pub created: u64,
    /// The scanned folder, as in [`Tree::name`] of the root, e.g. `C:\`.
    pub root: String,
    pub allocated: u64,
    pub logical: u64,
    pub files: u64,
    pub dirs: u64,
    pub nodes: u64,
}

/// Writes `tree` as a snapshot taken at `created` (Unix seconds).
pub fn write_snapshot(tree: &Tree, created: u64, out: impl Write) -> io::Result<()> {
    let mut out = BufWriter::new(out);
    out.write_all(MAGIC)?;
    out.write_all(&VERSION.to_le_bytes())?;
    let root = NodeId::ROOT;
    put_varint(&mut out, created)?;
    put_str(&mut out, tree.name(root))?;
    for v in [tree.allocated(root), tree.logical(root), tree.files(root), tree.dirs(root), live_nodes(tree)] {
        put_varint(&mut out, v)?;
    }

    // Buffered in front of the encoder too: it is slow with many tiny writes.
    let mut z = BufWriter::with_capacity(1 << 16, DeflateEncoder::new(out, Compression::fast()));
    // Explicit stack: folders can nest deeper than the thread stack allows.
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let kind = tree.kind(id);
        z.write_all(&[kind_code(kind), tree.flags(id).bits()])?;
        put_str(&mut z, if id == root { "" } else { tree.name(id) })?;
        match kind {
            NodeKind::Dir => {
                let children: Vec<NodeId> = tree.children(id).collect();
                put_varint(&mut z, children.len() as u64)?;
                // Written last to first: reading adds each child in front of
                // its siblings, which restores the order.
                stack.extend(children);
            }
            NodeKind::File => {
                put_varint(&mut z, tree.logical(id))?;
                put_varint(&mut z, tree.allocated(id))?;
            }
            NodeKind::FreeSpace | NodeKind::Unscanned => put_varint(&mut z, tree.allocated(id))?,
            NodeKind::Link => {}
        }
    }
    z.into_inner().map_err(|e| e.into_error())?.finish()?.flush()
}

/// Nodes reachable from the root; the arena also holds detached ones.
fn live_nodes(tree: &Tree) -> u64 {
    let mut count = 0;
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        count += 1;
        stack.extend(tree.children(id));
    }
    count
}

/// Reads only the header of a snapshot.
pub fn read_header(input: impl Read) -> io::Result<SnapshotHeader> {
    header(&mut BufReader::new(input))
}

/// Reads a whole snapshot back into a tree.
pub fn read_snapshot(input: impl Read) -> io::Result<(SnapshotHeader, Tree)> {
    let mut input = BufReader::new(input);
    let head = header(&mut input)?;
    let mut z = BufReader::new(DeflateDecoder::new(input));

    let mut tree = Tree::new(&head.root);
    let (kind, flags) = kind_flags(&mut z)?;
    if kind != NodeKind::Dir || !get_str(&mut z, MAX_NAME)?.is_empty() {
        return Err(bad("root is not a folder"));
    }
    tree.insert_flags(NodeId::ROOT, flags);
    let mut read = 1u64;
    // Open folders and how many of their children are still to come.
    let mut stack = vec![(NodeId::ROOT, get_varint(&mut z)?)];
    while let Some((parent, left)) = stack.last_mut() {
        if *left == 0 {
            stack.pop();
            continue;
        }
        *left -= 1;
        let parent = *parent;
        read += 1;
        if read > head.nodes {
            return Err(bad("more nodes than the header says"));
        }
        let (kind, flags) = kind_flags(&mut z)?;
        let name = get_str(&mut z, MAX_NAME)?;
        let id = match kind {
            NodeKind::Dir => {
                let id = tree.add_dir(parent, &name);
                stack.push((id, get_varint(&mut z)?));
                id
            }
            NodeKind::File => {
                let logical = get_varint(&mut z)?;
                tree.add_file(parent, &name, logical, get_varint(&mut z)?)
            }
            NodeKind::FreeSpace | NodeKind::Unscanned => tree.add_space(parent, &name, kind, get_varint(&mut z)?),
            NodeKind::Link => tree.add_link(parent, &name),
        };
        tree.insert_flags(id, flags);
    }
    if read != head.nodes {
        return Err(bad("fewer nodes than the header says"));
    }
    Ok((head, tree))
}

fn header(input: &mut impl Read) -> io::Result<SnapshotHeader> {
    let mut magic = [0u8; 8];
    input.read_exact(&mut magic).map_err(|_| bad("not a snapshot"))?;
    if &magic != MAGIC {
        return Err(bad("not a snapshot"));
    }
    let mut version = [0u8; 4];
    input.read_exact(&mut version)?;
    if u32::from_le_bytes(version) != VERSION {
        return Err(bad("unsupported snapshot version"));
    }
    let created = get_varint(input)?;
    let root = get_str(input, MAX_ROOT_NAME)?;
    let mut v = [0u64; 5];
    for x in &mut v {
        *x = get_varint(input)?;
    }
    let [allocated, logical, files, dirs, nodes] = v;
    if nodes == 0 || nodes > u64::from(u32::MAX) {
        return Err(bad("bad node count"));
    }
    Ok(SnapshotHeader { created, root, allocated, logical, files, dirs, nodes })
}

fn kind_code(kind: NodeKind) -> u8 {
    match kind {
        NodeKind::Dir => 0,
        NodeKind::File => 1,
        NodeKind::Link => 2,
        NodeKind::FreeSpace => 3,
        NodeKind::Unscanned => 4,
    }
}

fn kind_flags(input: &mut impl Read) -> io::Result<(NodeKind, NodeFlags)> {
    let mut b = [0u8; 2];
    input.read_exact(&mut b)?;
    let kind = match b[0] {
        0 => NodeKind::Dir,
        1 => NodeKind::File,
        2 => NodeKind::Link,
        3 => NodeKind::FreeSpace,
        4 => NodeKind::Unscanned,
        _ => return Err(bad("unknown node kind")),
    };
    Ok((kind, NodeFlags::from_bits_truncate(b[1])))
}

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("damaged snapshot: {what}"))
}

fn put_varint(out: &mut impl Write, mut v: u64) -> io::Result<()> {
    let mut buf = [0u8; 10];
    let mut n = 0;
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            buf[n] = byte;
            n += 1;
            break;
        }
        buf[n] = byte | 0x80;
        n += 1;
    }
    out.write_all(&buf[..n])
}

fn get_varint(input: &mut impl Read) -> io::Result<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let mut b = [0u8];
        input.read_exact(&mut b)?;
        let bits = u64::from(b[0] & 0x7F);
        if shift == 63 && bits > 1 {
            return Err(bad("number too large"));
        }
        v |= bits << shift;
        if b[0] & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(bad("number too long"))
}

fn put_str(out: &mut impl Write, s: &str) -> io::Result<()> {
    put_varint(out, s.len() as u64)?;
    out.write_all(s.as_bytes())
}

fn get_str(input: &mut impl Read, max: u64) -> io::Result<String> {
    let len = get_varint(input)?;
    if len > max {
        return Err(bad("name too long"));
    }
    let mut buf = Vec::with_capacity(len.min(MAX_PREALLOC) as usize);
    input.take(len).read_to_end(&mut buf)?;
    if buf.len() as u64 != len {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    String::from_utf8(buf).map_err(|_| bad("name is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Tree {
        let mut t = Tree::new("D:\\");
        let users = t.add_dir(NodeId::ROOT, "Users");
        let me = t.add_dir(users, "Сергей");
        t.add_file(me, "фильм.mkv", 7_000_000_000, 7_000_001_536);
        t.add_file(me, "empty.txt", 0, 0);
        let locked = t.add_dir(users, "Locked");
        t.insert_flags(locked, NodeFlags::ACCESS_DENIED);
        t.add_link(users, "link");
        t.add_dir(NodeId::ROOT, "Empty");
        t.add_space(NodeId::ROOT, "Free space", NodeKind::FreeSpace, 1 << 40);
        t.add_space(NodeId::ROOT, "Unscanned", NodeKind::Unscanned, 12345);
        t
    }

    fn roundtrip(t: &Tree) -> (SnapshotHeader, Tree) {
        let mut bytes = Vec::new();
        write_snapshot(t, 1_760_000_000, &mut bytes).unwrap();
        read_snapshot(bytes.as_slice()).unwrap()
    }

    /// Every node's name, kind, flags and sizes, in pre-order.
    fn dump(t: &Tree) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![(NodeId::ROOT, 0)];
        while let Some((id, depth)) = stack.pop() {
            out.push(format!(
                "{depth} {} {:?} {:?} {} {} {} {}",
                t.name(id),
                t.kind(id),
                t.flags(id),
                t.allocated(id),
                t.logical(id),
                t.files(id),
                t.dirs(id)
            ));
            let kids: Vec<_> = t.children(id).collect();
            stack.extend(kids.into_iter().rev().map(|c| (c, depth + 1)));
        }
        out
    }

    #[test]
    fn a_tree_survives_the_round_trip() {
        let t = sample();
        let (head, back) = roundtrip(&t);
        assert_eq!(dump(&back), dump(&t));
        assert_eq!(head.created, 1_760_000_000);
        assert_eq!(head.root, "D:\\");
        assert_eq!((head.allocated, head.files, head.dirs), (t.allocated(NodeId::ROOT), 2, 4));
        assert_eq!(head.nodes, t.node_count() as u64, "no detached nodes here");

        let mut bytes = Vec::new();
        write_snapshot(&t, 5, &mut bytes).unwrap();
        assert_eq!(read_header(bytes.as_slice()).unwrap(), head.clone().with_created(5));
    }

    impl SnapshotHeader {
        fn with_created(mut self, created: u64) -> Self {
            self.created = created;
            self
        }
    }

    #[test]
    fn detached_nodes_are_not_saved() {
        let mut t = sample();
        let users = t.find_path(&["Users"]).0;
        t.detach(users);
        let mut bytes = Vec::new();
        write_snapshot(&t, 0, &mut bytes).unwrap();
        let (head, back) = read_snapshot(bytes.as_slice()).unwrap();
        assert_eq!(dump(&back), dump(&t));
        assert_eq!(head.nodes, back.node_count() as u64);
    }

    #[test]
    fn varints_cover_the_whole_range() {
        for v in [0, 1, 127, 128, 300, u64::from(u32::MAX), u64::MAX - 1, u64::MAX] {
            let mut b = Vec::new();
            put_varint(&mut b, v).unwrap();
            assert_eq!(get_varint(&mut b.as_slice()).unwrap(), v);
        }
        // Eleven continuation bytes, and a 10th byte with too many bits.
        assert!(get_varint(&mut [0xFFu8; 11].as_slice()).is_err());
        let mut over = vec![0xFFu8; 9];
        over.push(0x02);
        assert!(get_varint(&mut over.as_slice()).is_err());
    }

    #[test]
    fn damaged_files_give_errors() {
        let mut bytes = Vec::new();
        write_snapshot(&sample(), 0, &mut bytes).unwrap();
        assert!(read_snapshot(&b"hello"[..]).is_err());
        for cut in [4, 12, 20, bytes.len() / 2, bytes.len() - 1] {
            assert!(read_snapshot(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        // Flipped bits in the compressed part: an error or some tree, never a panic.
        for pos in (20..bytes.len()).step_by(7) {
            let mut flipped = bytes.clone();
            flipped[pos] ^= 0x5A;
            let _ = read_snapshot(flipped.as_slice());
        }
    }
}
