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
/// How many entries to process between clock reads and cancellation checks.
const CHECK_EVERY: u32 = 256;
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

/// Walks `root` and builds a tree of the space taken by everything below it.
///
/// Links (symlinks, junctions, mount points) are recorded but not followed, so nothing
/// is counted twice. Unreadable directories are flagged [`NodeFlags::ACCESS_DENIED`] and
/// skipped. The walk is iterative, so deep trees cannot overflow the stack.
///
/// Fails only if `root` itself cannot be resolved or is not a directory.
pub fn scan(root: &Path, cancel: &CancelToken, mut on_progress: impl FnMut(&Progress)) -> io::Result<Scan> {
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
    let mut since_check = 0u32;
    let mut last_report = Instant::now();

    let mut stack: Vec<(NodeId, PathBuf)> = vec![(NodeId::ROOT, fs_root)];
    'walk: while let Some((dir_id, dir_path)) = stack.pop() {
        let entries = match fs::read_dir(&dir_path) {
            Ok(entries) => entries,
            Err(_) => {
                tree.insert_flags(dir_id, NodeFlags::ACCESS_DENIED);
                errors += 1;
                continue;
            }
        };

        for entry in entries {
            since_check += 1;
            if since_check >= CHECK_EVERY {
                since_check = 0;
                if cancel.is_cancelled() {
                    cancelled = true;
                    break 'walk;
                }
                if last_report.elapsed() >= PROGRESS_INTERVAL {
                    last_report = Instant::now();
                    on_progress(&Progress {
                        files: tree.files(NodeId::ROOT),
                        dirs: tree.dirs(NodeId::ROOT),
                        allocated: tree.allocated(NodeId::ROOT),
                        current_dir: &dir_path,
                    });
                }
            }

            // On Windows, both the entry and its metadata come from the directory
            // listing itself, without opening each file.
            let Ok((entry, meta)) = entry.and_then(|e| e.metadata().map(|m| (e, m))) else {
                errors += 1;
                continue;
            };
            let os_name = entry.file_name();
            let name = os_name.to_string_lossy();
            let file_type = meta.file_type();

            let id = if file_type.is_symlink() {
                links += 1;
                tree.add_link(dir_id, &name)
            } else if file_type.is_dir() {
                let id = tree.add_dir(dir_id, &name);
                if is_volume_root
                    && dir_id == NodeId::ROOT
                    && RECYCLE_BIN_NAMES.iter().any(|n| n.eq_ignore_ascii_case(&name))
                {
                    tree.insert_flags(id, NodeFlags::RECYCLE_BIN);
                }
                stack.push((id, entry.path()));
                id
            } else {
                let path = entry.path();
                let allocated = platform::allocated_size(&path, &meta, cluster);
                tree.add_file(dir_id, &name, meta.len(), allocated)
            };
            if os_name.to_str().is_none() {
                tree.insert_flags(id, NodeFlags::LOSSY_NAME);
            }
        }

        if cancel.is_cancelled() {
            cancelled = true;
            break;
        }
    }

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
