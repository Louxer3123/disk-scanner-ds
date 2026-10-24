//! Fast scan of an NTFS volume by reading its Master File Table (MFT) directly
//! instead of listing every folder. Needs administrator rights to open the volume.
//!
//! Compared with the directory walker it is several times faster, sees files the
//! user cannot list, counts hard-linked files once, and shows NTFS's own metadata
//! (`$MFT`, `$LogFile`, ...) as files in the root.

pub(crate) mod record;

use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::tree::{NodeFlags, NodeId, Tree};
use crate::walk::{CancelToken, Progress, Scan, ScanStats};
use record::{NAMESPACE_DOS, Record};

/// Record number of the volume's root folder.
const ROOT_RECORD: u64 = 5;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const RECYCLE_BIN_NAMES: [&str; 3] = ["$Recycle.Bin", "RECYCLER", "RECYCLED"];

pub use imp::{is_elevated, scan_mft};

/// Turns parsed records (indexed by record number) into a tree rooted at record 5.
///
/// Extension records are merged into their base record. Each file name becomes a
/// node in its parent folder; short DOS aliases are skipped. A file with several
/// hard links appears under each name, but only the first carries its size, so
/// the space is counted once. Names whose parent is gone are dropped.
pub(crate) fn build_tree(root_name: &str, mut records: Vec<Option<Record>>) -> (Tree, u64) {
    let n = records.len();
    for i in 0..n {
        let Some(base) = records[i].as_ref().and_then(|r| r.base) else { continue };
        let ext = records[i].take().expect("checked above");
        if let Some(Some(b)) = records.get_mut(base as usize) {
            b.names.extend(ext.names);
            b.allocated += ext.allocated;
            b.logical = b.logical.or(ext.logical);
            b.reparse_tag = b.reparse_tag.or(ext.reparse_tag);
        }
    }

    // Children by parent record: (child record, index of the name used).
    let mut children: Vec<Vec<(u32, u16)>> = vec![Vec::new(); n];
    for (i, rec) in records.iter().enumerate() {
        let Some(rec) = rec else { continue };
        let has_long = rec.names.iter().any(|f| f.namespace != NAMESPACE_DOS);
        for (k, f) in rec.names.iter().enumerate() {
            if (has_long && f.namespace == NAMESPACE_DOS) || f.parent == i as u64 {
                continue;
            }
            let parent_ok = records
                .get(f.parent as usize)
                .and_then(Option::as_ref)
                .is_some_and(|p| p.is_dir && (f.parent_seq == 0 || f.parent_seq == p.sequence));
            if parent_ok {
                children[f.parent as usize].push((i as u32, k as u16));
            }
        }
    }

    let mut tree = Tree::new(root_name);
    let mut links = 0u64;
    let mut counted = vec![false; n];
    let mut entered = vec![false; n];
    let root = ROOT_RECORD as usize;
    if root >= n || records[root].is_none() {
        return (tree, links);
    }
    entered[root] = true;
    let mut stack = vec![(NodeId::ROOT, root)];
    while let Some((dir_id, dir_rec)) = stack.pop() {
        for &(child, k) in &children[dir_rec] {
            let rec = records[child as usize].as_ref().expect("only existing records are children");
            let f = &rec.names[k as usize];
            let id = if rec.is_link() {
                links += 1;
                tree.add_link(dir_id, &f.name)
            } else if rec.is_dir {
                let id = tree.add_dir(dir_id, &f.name);
                if dir_id == NodeId::ROOT && RECYCLE_BIN_NAMES.iter().any(|r| r.eq_ignore_ascii_case(&f.name)) {
                    tree.insert_flags(id, NodeFlags::RECYCLE_BIN);
                }
                // A folder reached twice (a damaged MFT) is shown but not entered again.
                if !std::mem::replace(&mut entered[child as usize], true) {
                    stack.push((id, child as usize));
                }
                id
            } else if std::mem::replace(&mut counted[child as usize], true) {
                // Another hard link to a file already counted.
                tree.add_file(dir_id, &f.name, 0, 0)
            } else {
                tree.add_file(dir_id, &f.name, rec.logical.unwrap_or(0), rec.allocated)
            };
            if f.lossy {
                tree.insert_flags(id, NodeFlags::LOSSY_NAME);
            }
        }
    }
    (tree, links)
}

/// Assembles the [`Scan`] result once all records are read.
fn finish(root: &Path, records: Vec<Option<Record>>, cluster: u64, started: Instant) -> Scan {
    let root_name = crate::platform::display_path(root).to_string_lossy().into_owned();
    let (tree, links) = build_tree(&root_name, records);
    let stats = ScanStats {
        files: tree.files(NodeId::ROOT),
        dirs: tree.dirs(NodeId::ROOT),
        links,
        errors: 0,
        cluster_size: cluster,
        elapsed: started.elapsed(),
        cancelled: false,
    };
    Scan { tree, stats }
}

#[cfg(windows)]
mod imp {
    use super::record::{self, Record};
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile,
    };
    use windows_sys::Win32::System::IO::{DeviceIoControl, OVERLAPPED};
    use windows_sys::Win32::System::Ioctl::{FSCTL_GET_NTFS_VOLUME_DATA, NTFS_VOLUME_DATA_BUFFER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// Read size: big enough to keep the disk busy, a multiple of any cluster size.
    const CHUNK: u64 = 4 << 20;

    /// Whether this process runs with administrator rights (elevated by UAC).
    pub fn is_elevated() -> bool {
        // SAFETY: the token handle is closed; the out-struct matches the size passed.
        unsafe {
            let mut token: HANDLE = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let mut len = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                (&raw mut elevation).cast(),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            ) != 0;
            CloseHandle(token);
            ok && elevation.TokenIsElevated != 0
        }
    }

    struct Volume(HANDLE);

    impl Drop for Volume {
        fn drop(&mut self) {
            // SAFETY: the handle was opened by `open_volume` and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    impl Volume {
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
            let mut done = 0usize;
            while done < buf.len() {
                let pos = offset + done as u64;
                // SAFETY: a zeroed OVERLAPPED with the offset set makes a positioned
                // read on this synchronous handle; the buffer outlives the call.
                let read = unsafe {
                    let mut ov: OVERLAPPED = std::mem::zeroed();
                    ov.Anonymous.Anonymous.Offset = pos as u32;
                    ov.Anonymous.Anonymous.OffsetHigh = (pos >> 32) as u32;
                    let mut read = 0u32;
                    let want = (buf.len() - done).min(u32::MAX as usize) as u32;
                    if ReadFile(self.0, buf[done..].as_mut_ptr(), want, &mut read, &mut ov) == 0 {
                        return Err(io::Error::last_os_error());
                    }
                    read as usize
                };
                if read == 0 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "end of volume"));
                }
                done += read;
            }
            Ok(())
        }
    }

    fn open_volume(letter: char) -> io::Result<Volume> {
        let path: Vec<u16> = std::ffi::OsStr::new(&format!(r"\\.\{letter}:")).encode_wide().chain([0]).collect();
        // SAFETY: NUL-terminated path; the handle is owned by `Volume`.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(Volume(handle))
    }

    /// Scans a whole NTFS volume (`root` must be a drive root such as `C:\`) by
    /// reading its MFT. Fails without administrator rights, on other file
    /// systems, or on an MFT layout this reader does not handle; callers then
    /// fall back to the directory walker.
    pub fn scan_mft(root: &Path, cancel: &CancelToken, mut on_progress: impl FnMut(&Progress)) -> io::Result<Scan> {
        let started = Instant::now();
        let text = root.to_string_lossy();
        let text = text.trim_start_matches(r"\\?\");
        let letter = text.chars().next().filter(|c| c.is_ascii_alphabetic() && text[1..].trim_end_matches('\\') == ":");
        let letter = letter.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "MFT scan needs a drive root"))?;
        let volume = open_volume(letter)?;

        let mut data: NTFS_VOLUME_DATA_BUFFER = unsafe { std::mem::zeroed() };
        let mut returned = 0u32;
        // SAFETY: the out-buffer matches the size passed. Fails on non-NTFS volumes.
        let ok = unsafe {
            DeviceIoControl(
                volume.0,
                FSCTL_GET_NTFS_VOLUME_DATA,
                null(),
                0,
                (&raw mut data).cast(),
                size_of::<NTFS_VOLUME_DATA_BUFFER>() as u32,
                &mut returned,
                null_mut(),
            ) != 0
        };
        if !ok {
            return Err(io::Error::last_os_error());
        }
        let cluster = u64::from(data.BytesPerCluster);
        let sector = data.BytesPerSector as usize;
        let rec_size = data.BytesPerFileRecordSegment as u64;
        let mft_start = data.MftStartLcn as u64 * cluster;
        let total = data.MftValidDataLength as u64 / rec_size;
        let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, format!("unexpected MFT layout: {what}"));
        if rec_size == 0 || cluster == 0 || !CHUNK.is_multiple_of(cluster) || !CHUNK.is_multiple_of(rec_size) {
            return Err(bad("record or cluster size"));
        }

        // Record 0 describes the MFT itself: where its pieces lie on disk.
        let mut first = vec![0u8; rec_size.max(sector as u64) as usize];
        volume.read_at(mft_start, &mut first)?;
        let rec_bytes = &mut first[..rec_size as usize];
        if !record::apply_fixups(rec_bytes, sector) {
            return Err(bad("record 0"));
        }
        let runs = record::parse(rec_bytes, true).ok_or_else(|| bad("record 0"))?.data_runs;
        let mapped: u64 = runs.iter().map(|r| r.clusters * cluster).sum();
        if runs.is_empty() || runs.iter().any(|r| r.lcn.is_none()) || mapped < total * rec_size {
            // Extremely fragmented MFTs keep further runs in extension records.
            return Err(bad("data runs"));
        }

        let mut records: Vec<Option<Record>> = Vec::with_capacity(total as usize);
        let mut buf = vec![0u8; CHUNK as usize];
        let (mut files, mut allocated) = (0u64, 0u64);
        let mut last_report = Instant::now();
        'runs: for run in &runs {
            let run_start = run.lcn.expect("checked above") * cluster;
            let run_len = run.clusters * cluster;
            let mut done = 0;
            while done < run_len {
                if cancel.is_cancelled() {
                    let mut scan = finish(root, records, cluster, started);
                    scan.stats.cancelled = true;
                    return Ok(scan);
                }
                let len = (run_len - done).min(CHUNK) as usize;
                volume.read_at(run_start + done, &mut buf[..len])?;
                done += len as u64;
                for chunk in buf[..len].chunks_exact_mut(rec_size as usize) {
                    if records.len() as u64 >= total {
                        break 'runs;
                    }
                    let rec = record::apply_fixups(chunk, sector).then(|| record::parse(chunk, false)).flatten();
                    if let Some(r) = &rec
                        && r.base.is_none()
                    {
                        files += 1;
                        allocated += r.allocated;
                    }
                    records.push(rec);
                }
                if last_report.elapsed() >= PROGRESS_INTERVAL {
                    last_report = Instant::now();
                    on_progress(&Progress { files, dirs: 0, allocated, current_dir: root });
                }
            }
        }
        Ok(finish(root, records, cluster, started))
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn is_elevated() -> bool {
        false
    }

    pub fn scan_mft(_root: &Path, _cancel: &CancelToken, _on_progress: impl FnMut(&Progress)) -> io::Result<Scan> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "MFT scans are Windows-only"))
    }
}

#[cfg(test)]
mod tests {
    use super::record::testing::{RecordBuilder, parsed};
    use super::*;
    use crate::tree::NodeKind;

    /// Builds a record table from `(number, record)` pairs.
    fn table(entries: Vec<(usize, Vec<u8>)>) -> Vec<Option<Record>> {
        let n = entries.iter().map(|(i, _)| i + 1).max().unwrap_or(0);
        let mut t = vec![None; n];
        for (i, bytes) in entries {
            t[i] = parsed(bytes);
        }
        t
    }

    /// Sequence number of a parent in [`sample`]: 5 for the root, 1 for the rest.
    fn seq(parent: u64) -> u16 {
        if parent == ROOT_RECORD { 5 } else { 1 }
    }

    fn dir(parent: u64, name: &str) -> Vec<u8> {
        RecordBuilder::new(true).file_name(parent, seq(parent), name, 1).build()
    }

    fn file(parent: u64, name: &str, size: usize) -> Vec<u8> {
        RecordBuilder::new(false).file_name(parent, seq(parent), name, 1).resident_data("", size).build()
    }

    fn big_file(parent: u64, name: &str, allocated: u64) -> Vec<u8> {
        RecordBuilder::new(false)
            .file_name(parent, seq(parent), name, 1)
            .data("", 0, allocated, allocated - 100, None, &[0])
            .build()
    }

    /// root(5)/ { Users(16)/ { me(17)/ { a.bin(20) } }, $MFT(0), notes.txt(21), link(22) }
    fn sample() -> Vec<Option<Record>> {
        table(vec![
            (0, big_file(5, "$MFT", 4096 * 10)),
            (5, RecordBuilder::new(true).sequence(5).file_name(5, 5, ".", 1).build()),
            (16, dir(5, "Users")),
            (17, dir(16, "me")),
            (20, big_file(17, "a.bin", 8192)),
            (21, file(5, "notes.txt", 300)),
            (22, RecordBuilder::new(true).file_name(5, 5, "link", 1).reparse(0xA000_0003).build()),
        ])
    }

    #[test]
    fn builds_folders_files_and_links_from_parent_references() {
        let (t, links) = build_tree("C:\\", sample());
        assert_eq!(links, 1);
        let (a, n) = t.find_path(&["Users", "me", "a.bin"]);
        assert_eq!(n, 3);
        assert_eq!((t.allocated(a), t.logical(a)), (8192, 8092));
        assert_eq!(t.kind(t.find_path(&["link"]).0), NodeKind::Link);
        assert_eq!(t.allocated(t.find_path(&["$MFT"]).0), 40960, "NTFS metadata shows as files");
        assert_eq!(t.allocated(t.find_path(&["notes.txt"]).0), 0, "resident data takes no clusters");
        assert_eq!(t.files(NodeId::ROOT), 3);
        assert_eq!(t.dirs(NodeId::ROOT), 2);
        assert_eq!(t.allocated(NodeId::ROOT), 40960 + 8192);
        assert!(t.children(NodeId::ROOT).all(|c| t.name(c) != "."), "the root's self-reference is skipped");
    }

    #[test]
    fn hard_links_are_counted_once() {
        let mut t = sample();
        // a.bin also linked from the root under another name.
        t[20] = parsed(
            RecordBuilder::new(false)
                .file_name(17, 1, "a.bin", 1)
                .file_name(5, 5, "a-link.bin", 1)
                .data("", 0, 8192, 8092, None, &[0])
                .build(),
        );
        let (t, _) = build_tree("C:\\", t);
        let a = t.find_path(&["Users", "me", "a.bin"]).0;
        let other = t.find_path(&["a-link.bin"]).0;
        assert_eq!(t.allocated(a) + t.allocated(other), 8192);
        assert_eq!(t.allocated(NodeId::ROOT), 40960 + 8192);
    }

    #[test]
    fn dos_aliases_are_skipped_and_extensions_merged() {
        let mut t = sample();
        t.resize(40, None);
        t[30] = parsed(
            RecordBuilder::new(false)
                .file_name(5, 5, "Long File Name.txt", 1)
                .file_name(5, 5, "LONGFI~1.TXT", NAMESPACE_DOS)
                .build(),
        );
        // Its data lives in extension record 31.
        t[31] = parsed(RecordBuilder::new(false).extension_of(30).data("", 0, 12288, 12000, None, &[0]).build());
        let (t, _) = build_tree("C:\\", t);
        assert!(t.child_named(NodeId::ROOT, "LONGFI~1.TXT").is_none());
        let f = t.child_named(NodeId::ROOT, "Long File Name.txt").unwrap();
        assert_eq!((t.allocated(f), t.logical(f)), (12288, 12000));
    }

    #[test]
    fn orphans_and_stale_parents_are_dropped() {
        let mut t = sample();
        t.resize(40, None);
        t[32] = parsed(file(39, "parent never existed", 1));
        // Parent 16 has sequence 1; a reference to sequence 9 means a deleted folder
        // whose record number was reused.
        t[33] = parsed(RecordBuilder::new(false).file_name(16, 9, "stale", 1).build());
        let (t, _) = build_tree("C:\\", t);
        assert_eq!(t.files(NodeId::ROOT), 3);
    }

    #[test]
    fn cycles_in_a_damaged_mft_do_not_hang() {
        let mut t = sample();
        t.resize(40, None);
        // 34 and 35 claim to be each other's parent; 35 is also under the root.
        t[34] = parsed(RecordBuilder::new(true).file_name(35, 1, "x", 1).build());
        t[35] = parsed(RecordBuilder::new(true).file_name(5, 5, "y", 1).file_name(34, 1, "y2", 1).build());
        let (t, _) = build_tree("C:\\", t);
        assert!(t.child_named(NodeId::ROOT, "y").is_some());
    }

    #[test]
    fn recycle_bin_is_flagged_in_the_root() {
        let mut t = sample();
        t.resize(40, None);
        t[36] = parsed(dir(5, "$Recycle.Bin"));
        let (t, _) = build_tree("C:\\", t);
        let bin = t.child_named(NodeId::ROOT, "$Recycle.Bin").unwrap();
        assert!(t.flags(bin).contains(NodeFlags::RECYCLE_BIN));
    }

    #[test]
    fn missing_root_gives_an_empty_tree() {
        let (t, _) = build_tree("C:\\", table(vec![(16, dir(5, "Users"))]));
        assert_eq!(t.node_count(), 1);
    }
}
