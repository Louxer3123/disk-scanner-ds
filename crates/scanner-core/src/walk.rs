//! Directory walker that fills a [`Tree`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::platform;
use crate::tree::{NodeFlags, NodeId, Tree};

/// Cluster size assumed when the OS does not report one.
const DEFAULT_CLUSTER: u64 = 4096;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// Recycle bin folder names: Vista and later, XP NTFS, FAT.
const RECYCLE_BIN_NAMES: [&str; 3] = ["$Recycle.Bin", "RECYCLER", "RECYCLED"];

/// Stops a running [`scan`] from another thread. Clones share the same flag.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// Whether `other` is a clone of this token rather than an independent one.
    pub fn same_as(&self, other: &CancelToken) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Snapshot passed to the progress callback, at most every 100 ms.
#[derive(Debug)]
pub struct Progress<'a> {
    pub files: u64,
    pub dirs: u64,
    pub allocated: u64,
    /// Directory being listed. On Windows it has the `\\?\` prefix; see [`crate::display_path`].
    pub current_dir: &'a Path,
}

#[derive(Clone, Debug)]
pub struct ScanStats {
    pub files: u64,
    pub dirs: u64,
    /// Symlinks, junctions and mount points that were recorded but not followed.
    pub links: u64,
    /// Directories or entries that could not be read.
    pub errors: u64,
    pub cluster_size: u64,
    pub elapsed: Duration,
    /// The scan was stopped early; the tree holds what was read so far.
    pub cancelled: bool,
}

#[derive(Debug)]
pub struct Scan {
    pub tree: Tree,
    pub stats: ScanStats,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanOptions {
    /// Threads listing directories. Several help a lot on SSDs and network
    /// shares; on a spinning disk use 1, as parallel reads make it seek.
    pub threads: usize,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions { threads: default_threads() }
    }
}

/// One thread per core, up to 8: beyond that the file system is the bottleneck.
pub fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get()).min(8)
}

/// [`scan_with`] with default options.
pub fn scan(root: &Path, cancel: &CancelToken, on_progress: impl FnMut(&Progress)) -> io::Result<Scan> {
    scan_with(root, &ScanOptions::default(), cancel, on_progress)
}

/// One directory's entries, read by a worker thread.
struct Listing {
    dir: NodeId,
    path: PathBuf,
    entries: Vec<Entry>,
    /// The directory could not be listed at all.
    denied: bool,
    /// Entries that could not be read.
    errors: u64,
}

struct Entry {
    name: String,
    lossy: bool,
    kind: EntryKind,
}

enum EntryKind {
    Dir(PathBuf),
    File { logical: u64, allocated: u64 },
    Link,
}

/// Lists one directory. Runs on worker threads, so it must not touch the tree.
fn list_dir(dir: NodeId, path: PathBuf, cluster: u64) -> Listing {
    let mut listing = Listing { dir, path, entries: Vec::new(), denied: false, errors: 0 };
    let entries = match fs::read_dir(&listing.path) {
        Ok(entries) => entries,
        Err(_) => {
            listing.denied = true;
            return listing;
        }
    };
    for entry in entries {
        // On Windows, both the entry and its metadata come from the directory
        // listing itself, without opening each file.
        let Ok((entry, meta)) = entry.and_then(|e| e.metadata().map(|m| (e, m))) else {
            listing.errors += 1;
            continue;
        };
        let os_name = entry.file_name();
        let file_type = meta.file_type();
        let kind = if file_type.is_symlink() {
            EntryKind::Link
        } else if file_type.is_dir() {
            EntryKind::Dir(entry.path())
        } else {
            let allocated = platform::allocated_size(&entry.path(), &meta, cluster);
            EntryKind::File { logical: meta.len(), allocated }
        };
        listing.entries.push(Entry {
            name: os_name.to_string_lossy().into_owned(),
            lossy: os_name.to_str().is_none(),
            kind,
        });
    }
    listing
}

/// Walks `root` and builds a tree of the space taken by everything below it.
///
/// Worker threads list directories in parallel; the calling thread puts their
/// results into the tree and runs `on_progress`, so the callback needs no `Send`.
/// Links (symlinks, junctions, mount points) are recorded but not followed, so
/// nothing is counted twice. Unreadable directories are flagged
/// [`NodeFlags::ACCESS_DENIED`] and skipped.
///
/// Fails only if `root` itself cannot be resolved or is not a directory.
pub fn scan_with(
    root: &Path,
    options: &ScanOptions,
    cancel: &CancelToken,
    mut on_progress: impl FnMut(&Progress),
) -> io::Result<Scan> {
    let started = Instant::now();
    // On Windows this yields a `\\?\` path, which lifts the 260-character limit
    // for every path joined onto it.
    let fs_root = fs::canonicalize(root)?;
    if !fs::metadata(&fs_root)?.is_dir() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("not a directory: {}", root.display())));
    }
    let cluster = platform::cluster_size(&fs_root).unwrap_or(DEFAULT_CLUSTER);
    let is_volume_root = fs_root.parent().is_none();

    let mut tree = Tree::new(&platform::display_path(&fs_root).to_string_lossy());
    let mut links = 0;
    let mut errors = 0;
    let mut cancelled = false;
    let mut last_report = Instant::now();

    let (task_tx, task_rx) = crossbeam_channel::unbounded::<(NodeId, PathBuf)>();
    let (done_tx, done_rx) = crossbeam_channel::unbounded::<Listing>();
    std::thread::scope(|s| {
        for _ in 0..options.threads.max(1) {
            let (tasks, done) = (task_rx.clone(), done_tx.clone());
            s.spawn(move || {
                // Ends once the main thread drops `task_tx` and the queue is empty.
                for (dir, path) in tasks {
                    let listing = if cancel.is_cancelled() {
                        Listing { dir, path, entries: Vec::new(), denied: false, errors: 0 }
                    } else {
                        list_dir(dir, path, cluster)
                    };
                    if done.send(listing).is_err() {
                        break;
                    }
                }
            });
        }
        drop(done_tx);

        task_tx.send((NodeId::ROOT, fs_root)).expect("workers are running");
        // Directories queued or being listed; the walk ends when none are left.
        let mut pending = 1usize;
        while pending > 0 {
            let Ok(listing) = done_rx.recv() else { break };
            pending -= 1;
            if cancel.is_cancelled() {
                cancelled = true;
                break;
            }
            errors += listing.errors;
            if listing.denied {
                tree.insert_flags(listing.dir, NodeFlags::ACCESS_DENIED);
                errors += 1;
            }
            for entry in listing.entries {
                let id = match entry.kind {
                    EntryKind::Link => {
                        links += 1;
                        tree.add_link(listing.dir, &entry.name)
                    }
                    EntryKind::Dir(path) => {
                        let id = tree.add_dir(listing.dir, &entry.name);
                        if is_volume_root
                            && listing.dir == NodeId::ROOT
                            && RECYCLE_BIN_NAMES.iter().any(|n| n.eq_ignore_ascii_case(&entry.name))
                        {
                            tree.insert_flags(id, NodeFlags::RECYCLE_BIN);
                        }
                        task_tx.send((id, path)).expect("workers are running");
                        pending += 1;
                        id
                    }
                    EntryKind::File { logical, allocated } => tree.add_file(listing.dir, &entry.name, logical, allocated),
                };
                if entry.lossy {
                    tree.insert_flags(id, NodeFlags::LOSSY_NAME);
                }
            }
            if last_report.elapsed() >= PROGRESS_INTERVAL {
                last_report = Instant::now();
                on_progress(&Progress {
                    files: tree.files(NodeId::ROOT),
                    dirs: tree.dirs(NodeId::ROOT),
                    allocated: tree.allocated(NodeId::ROOT),
                    current_dir: &listing.path,
                });
            }
        }
        // Lets the workers finish: after a cancel they skip what is still queued.
        drop(task_tx);
    });

    let stats = ScanStats {
        files: tree.files(NodeId::ROOT),
        dirs: tree.dirs(NodeId::ROOT),
        links,
        errors,
        cluster_size: cluster,
        elapsed: started.elapsed(),
        cancelled,
    };
    Ok(Scan { tree, stats })
}
