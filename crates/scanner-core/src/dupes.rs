//! Finding duplicate files: same length, same first block, then same contents.
//!
//! Only files that could be duplicates are read: first files are grouped by
//! length, then by a hash of their first 64 KiB, and only what still matches is
//! hashed in full (BLAKE3). Several names of one file (hard links) count as
//! one file, and cloud placeholders are never read, as that would download them.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::platform;
use crate::tree::{NodeFlags, NodeId, NodeKind, Tree};
use crate::walk::CancelToken;

const HEAD: usize = 64 * 1024;
const BUFFER: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct DupOptions {
    /// Smaller files are ignored.
    pub min_size: u64,
    /// Files read at once; 1 on a spinning disk.
    pub threads: usize,
}

/// A file in the searched trees: the index of its tree in the `parts` slice and its node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileRef {
    pub part: usize,
    pub node: NodeId,
}

#[derive(Clone, Debug)]
pub struct DupGroup {
    /// Length of each copy.
    pub size: u64,
    /// Space each copy takes on disk.
    pub allocated: u64,
    pub files: Vec<FileRef>,
}

impl DupGroup {
    /// Space freed by keeping one copy.
    pub fn wasted(&self) -> u64 {
        self.allocated.saturating_mul(self.files.len() as u64 - 1)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DupProgress {
    /// 1: first blocks, 2: whole files.
    pub phase: u8,
    pub done: usize,
    pub total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

#[derive(Debug, Default)]
pub struct DupResult {
    /// Largest waste first.
    pub groups: Vec<DupGroup>,
    /// Files that could not be read (locked, gone, no access).
    pub errors: usize,
    /// Cloud placeholders left out.
    pub skipped: usize,
    pub cancelled: bool,
}

struct Candidate {
    file: FileRef,
    path: PathBuf,
    size: u64,
    allocated: u64,
}

/// Finds duplicate files below each `(tree, root)`.
pub fn find_duplicates(
    parts: &[(&Tree, NodeId)],
    options: DupOptions,
    cancel: &CancelToken,
    mut on_progress: impl FnMut(&DupProgress),
) -> DupResult {
    let mut result = DupResult::default();

    // 1. Group by length; a length seen once cannot have a duplicate.
    let mut by_size: HashMap<u64, Vec<Candidate>> = HashMap::new();
    for (part, &(tree, root)) in parts.iter().enumerate() {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            match tree.kind(id) {
                NodeKind::Dir => stack.extend(tree.children(id)),
                // A lossy name's path does not lead to the file.
                NodeKind::File if !tree.flags(id).contains(NodeFlags::LOSSY_NAME) => {
                    let size = tree.logical(id);
                    if size >= options.min_size.max(1) {
                        by_size.entry(size).or_default().push(Candidate {
                            file: FileRef { part, node: id },
                            path: tree.path(id),
                            size,
                            allocated: tree.allocated(id),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    let candidates: Vec<Candidate> = by_size.into_values().filter(|g| g.len() > 1).flatten().collect();

    // 2. Hash the first block (all of a small file), learning each file's identity.
    let heads = hash_all(&candidates, options.threads, cancel, 1, |c| hash_file(c, true), &mut on_progress);
    if cancel.is_cancelled() {
        result.cancelled = true;
        return result;
    }
    let mut by_head: HashMap<(u64, [u8; 32]), Vec<usize>> = HashMap::new();
    let mut seen_ids = std::collections::HashSet::new();
    for (i, h) in heads.into_iter().enumerate() {
        match h {
            Hashed::Ok { hash, identity } => {
                // A second name of a file already listed (hard link) is no copy.
                if identity.is_some_and(|id| !seen_ids.insert(id)) {
                    continue;
                }
                by_head.entry((candidates[i].size, hash)).or_default().push(i);
            }
            Hashed::Placeholder => result.skipped += 1,
            Hashed::Failed => result.errors += 1,
        }
    }

    // 3. Files whose first block matches: small ones are already fully hashed,
    // larger ones are read in full.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut to_read: Vec<usize> = Vec::new();
    for g in by_head.into_values().filter(|g| g.len() > 1) {
        if candidates[g[0]].size as usize <= HEAD {
            groups.push(g);
        } else {
            to_read.extend(g);
        }
    }
    let subset: Vec<&Candidate> = to_read.iter().map(|&i| &candidates[i]).collect();
    let full = hash_all(&subset, options.threads, cancel, 2, |c| hash_file(c, false), &mut on_progress);
    if cancel.is_cancelled() {
        result.cancelled = true;
        return result;
    }
    let mut by_full: HashMap<(u64, [u8; 32]), Vec<usize>> = HashMap::new();
    for (k, h) in full.into_iter().enumerate() {
        match h {
            Hashed::Ok { hash, .. } => by_full.entry((subset[k].size, hash)).or_default().push(to_read[k]),
            Hashed::Placeholder => result.skipped += 1,
            Hashed::Failed => result.errors += 1,
        }
    }
    groups.extend(by_full.into_values().filter(|g| g.len() > 1));

    result.groups = groups
        .into_iter()
        .map(|mut g| {
            g.sort_by(|&a, &b| candidates[a].path.cmp(&candidates[b].path));
            DupGroup {
                size: candidates[g[0]].size,
                allocated: g.iter().map(|&i| candidates[i].allocated).max().unwrap_or(0),
                files: g.iter().map(|&i| candidates[i].file).collect(),
            }
        })
        .collect();
    result.groups.sort_by_key(|g| std::cmp::Reverse((g.wasted(), g.size)));
    result
}

enum Hashed {
    Ok { hash: [u8; 32], identity: Option<(u64, u64)> },
    Placeholder,
    Failed,
}

fn hash_file(c: &Candidate, head_only: bool) -> Hashed {
    match try_hash(c, head_only) {
        Ok(Some(h)) => h,
        Ok(None) => Hashed::Placeholder,
        Err(_) => Hashed::Failed,
    }
}

fn try_hash(c: &Candidate, head_only: bool) -> io::Result<Option<Hashed>> {
    let file = File::open(&c.path)?;
    let meta = file.metadata()?;
    if platform::is_placeholder(&meta) {
        return Ok(None);
    }
    // Changed since the scan: comparing it with the others would mislead.
    if meta.len() != c.size {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let identity = platform::file_identity(&file);
    let mut hasher = blake3::Hasher::new();
    let mut reader = file.take(if head_only { HEAD as u64 } else { u64::MAX });
    let mut buf = vec![0u8; if head_only { HEAD } else { BUFFER }];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(Some(Hashed::Ok { hash: *hasher.finalize().as_bytes(), identity }))
}

/// Runs `hash` over `items` on `threads` threads, in order of the result.
fn hash_all<T: std::borrow::Borrow<Candidate> + Sync>(
    items: &[T],
    threads: usize,
    cancel: &CancelToken,
    phase: u8,
    hash: impl Fn(&Candidate) -> Hashed + Sync,
    on_progress: &mut impl FnMut(&DupProgress),
) -> Vec<Hashed> {
    let total = items.len();
    let bytes_total = items.iter().map(|c| bytes_read(c.borrow(), phase)).sum();
    let mut progress = DupProgress { phase, done: 0, total, bytes_done: 0, bytes_total };
    on_progress(&progress);
    let next = AtomicUsize::new(0);
    let mut out: Vec<Option<Hashed>> = (0..total).map(|_| None).collect();
    let (tx, rx) = crossbeam_channel::unbounded();
    std::thread::scope(|s| {
        for _ in 0..threads.clamp(1, 16).min(total.max(1)) {
            let tx = tx.clone();
            let (next, hash) = (&next, &hash);
            s.spawn(move || {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= total || cancel.is_cancelled() {
                        break;
                    }
                    if tx.send((i, hash(items[i].borrow()))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        let mut last = std::time::Instant::now();
        for (i, h) in rx {
            progress.done += 1;
            progress.bytes_done += bytes_read(items[i].borrow(), phase);
            out[i] = Some(h);
            if last.elapsed().as_millis() >= 100 {
                last = std::time::Instant::now();
                on_progress(&progress);
            }
        }
    });
    on_progress(&progress);
    out.into_iter().map(|h| h.unwrap_or(Hashed::Failed)).collect()
}

fn bytes_read(c: &Candidate, phase: u8) -> u64 {
    if phase == 1 { c.size.min(HEAD as u64) } else { c.size }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walk::scan;
    use std::fs;

    fn find(root: &std::path::Path, min_size: u64) -> (Tree, DupResult) {
        let tree = scan(root, &CancelToken::new(), |_| {}).unwrap().tree;
        let r = find_duplicates(&[(&tree, NodeId::ROOT)], DupOptions { min_size, threads: 3 }, &CancelToken::new(), |_| {});
        (tree, r)
    }

    fn names(tree: &Tree, g: &DupGroup) -> Vec<String> {
        g.files.iter().map(|f| tree.name(f.node).to_owned()).collect()
    }

    #[test]
    fn finds_copies_and_ignores_lookalikes() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fs::create_dir_all(d.join("a/b")).unwrap();
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(d.join("movie.bin"), &big).unwrap();
        fs::write(d.join("a/b/movie copy.bin"), &big).unwrap();
        // Same length and same first 64 KiB, different end.
        let mut other = big.clone();
        *other.last_mut().unwrap() ^= 1;
        fs::write(d.join("a/almost.bin"), &other).unwrap();
        // Small duplicates are hashed in one pass.
        fs::write(d.join("x.txt"), b"same text").unwrap();
        fs::write(d.join("a/y.txt"), b"same text").unwrap();
        fs::write(d.join("a/z.txt"), b"diff text").unwrap();
        fs::write(d.join("empty1"), b"").unwrap();
        fs::write(d.join("empty2"), b"").unwrap();

        let (tree, r) = find(d, 0);
        assert_eq!(r.groups.len(), 2, "{:?}", r.groups.iter().map(|g| names(&tree, g)).collect::<Vec<_>>());
        let movies = &r.groups[0];
        assert_eq!(movies.size, 300_000);
        let mut n = names(&tree, movies);
        n.sort();
        assert_eq!(n, ["movie copy.bin", "movie.bin"]);
        assert_eq!(movies.wasted(), movies.allocated);
        let mut n = names(&tree, &r.groups[1]);
        n.sort();
        assert_eq!(n, ["x.txt", "y.txt"]);
        assert_eq!((r.errors, r.skipped, r.cancelled), (0, 0, false));

        // The size threshold leaves the small ones out.
        let (_, r) = find(d, 1000);
        assert_eq!(r.groups.len(), 1);
    }

    #[test]
    fn hard_links_are_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fs::write(d.join("data.bin"), vec![7u8; 5000]).unwrap();
        if fs::hard_link(d.join("data.bin"), d.join("link.bin")).is_err() {
            eprintln!("skipping: no hard links here");
            return;
        }
        let (tree, r) = find(d, 0);
        assert!(r.groups.is_empty(), "{:?}", r.groups.iter().map(|g| names(&tree, g)).collect::<Vec<_>>());
        // A real copy next to them is found, once.
        fs::write(d.join("copy.bin"), vec![7u8; 5000]).unwrap();
        let (_, r) = find(d, 0);
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.groups[0].files.len(), 2);
    }

    #[test]
    fn files_changed_after_the_scan_are_errors_not_matches() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fs::write(d.join("a"), b"0123456789").unwrap();
        fs::write(d.join("b"), b"0123456789").unwrap();
        let tree = scan(d, &CancelToken::new(), |_| {}).unwrap().tree;
        fs::write(d.join("b"), b"012345678").unwrap();
        let r = find_duplicates(&[(&tree, NodeId::ROOT)], DupOptions { min_size: 0, threads: 1 }, &CancelToken::new(), |_| {});
        assert!(r.groups.is_empty());
        assert_eq!(r.errors, 1);
    }

    #[test]
    fn cancelling_stops_early() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..20 {
            fs::write(dir.path().join(format!("f{i}")), b"same").unwrap();
        }
        let tree = scan(dir.path(), &CancelToken::new(), |_| {}).unwrap().tree;
        let cancel = CancelToken::new();
        cancel.cancel();
        let r = find_duplicates(&[(&tree, NodeId::ROOT)], DupOptions { min_size: 0, threads: 2 }, &cancel, |_| {});
        assert!(r.cancelled && r.groups.is_empty());
    }
}
