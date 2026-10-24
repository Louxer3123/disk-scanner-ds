//! OS-specific pieces: cluster size, on-disk file size, path display.

use std::fs::Metadata;
use std::path::Path;

pub(crate) fn round_up(size: u64, cluster: u64) -> u64 {
    size.div_ceil(cluster) * cluster
}

#[cfg(windows)]
mod imp {
    use super::round_up;
    use std::fs::Metadata;
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Foundation::{GetLastError, NO_ERROR};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
        FILE_ATTRIBUTE_RECALL_ON_OPEN, FILE_ATTRIBUTE_SPARSE_FILE, GetCompressedFileSizeW, GetDiskFreeSpaceW,
        GetVolumePathNameW, INVALID_FILE_SIZE,
    };

    /// Attributes for which the file length says little about the space taken:
    /// compressed, sparse and cloud placeholder (OneDrive etc.) files.
    const ON_DISK_DIFFERS: u32 = FILE_ATTRIBUTE_COMPRESSED
        | FILE_ATTRIBUTE_SPARSE_FILE
        | FILE_ATTRIBUTE_OFFLINE
        | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS
        | FILE_ATTRIBUTE_RECALL_ON_OPEN;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(once(0)).collect()
    }

    pub fn cluster_size(path: &Path) -> Option<u64> {
        let path = wide(path);
        let mut volume = [0u16; 1024];
        let (mut sectors_per_cluster, mut bytes_per_sector, mut free, mut total) = (0, 0, 0, 0);
        // SAFETY: both buffers are NUL-terminated and the length passed matches `volume`.
        let ok = unsafe {
            GetVolumePathNameW(path.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) != 0
                && GetDiskFreeSpaceW(
                    volume.as_ptr(),
                    &mut sectors_per_cluster,
                    &mut bytes_per_sector,
                    &mut free,
                    &mut total,
                ) != 0
        };
        let cluster = u64::from(sectors_per_cluster) * u64::from(bytes_per_sector);
        (ok && cluster > 0).then_some(cluster)
    }

    /// Space the file takes on disk: the length rounded up to whole clusters,
    /// or the real allocation for compressed, sparse and placeholder files.
    pub fn allocated_size(path: &Path, meta: &Metadata, cluster: u64) -> u64 {
        if meta.file_attributes() & ON_DISK_DIFFERS != 0
            && let Some(size) = compressed_size(path)
        {
            return round_up(size, cluster);
        }
        round_up(meta.len(), cluster)
    }

    fn compressed_size(path: &Path) -> Option<u64> {
        let path = wide(path);
        let mut high = 0u32;
        // SAFETY: `path` is NUL-terminated; GetLastError is read right after the call,
        // as INVALID_FILE_SIZE is also a valid low half of a real size.
        unsafe {
            let low = GetCompressedFileSizeW(path.as_ptr(), &mut high);
            if low == INVALID_FILE_SIZE && GetLastError() != NO_ERROR {
                return None;
            }
            Some(u64::from(high) << 32 | u64::from(low))
        }
    }

    /// Removes the `\\?\` prefix that `fs::canonicalize` adds, for display.
    pub fn display_path(path: &Path) -> PathBuf {
        let s = path.as_os_str().to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            PathBuf::from(format!(r"\\{rest}"))
        } else if let Some(rest) = s.strip_prefix(r"\\?\") {
            PathBuf::from(rest)
        } else {
            path.to_path_buf()
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::fs::Metadata;
    use std::path::{Path, PathBuf};

    pub fn cluster_size(_path: &Path) -> Option<u64> {
        None
    }

    pub fn allocated_size(_path: &Path, meta: &Metadata, cluster: u64) -> u64 {
        #[cfg(unix)]
        {
            let _ = cluster;
            std::os::unix::fs::MetadataExt::blocks(meta) * 512
        }
        #[cfg(not(unix))]
        super::round_up(meta.len(), cluster)
    }

    pub fn display_path(path: &Path) -> PathBuf {
        path.to_path_buf()
    }
}

/// Allocation unit of the volume holding `path`, if the OS reports one.
pub(crate) fn cluster_size(path: &Path) -> Option<u64> {
    imp::cluster_size(path)
}

pub(crate) fn allocated_size(path: &Path, meta: &Metadata, cluster: u64) -> u64 {
    imp::allocated_size(path, meta, cluster)
}

/// Whether a plain (non-`\\?\`) Win32 path ending in `name` reaches exactly that
/// file. Win32 path parsing drops trailing dots and spaces, turns `CON`, `NUL`,
/// `COM1` and the like (even with an extension) into devices, and rejects
/// `< > : " / \ | ? *` and control characters. NTFS can still hold such names
/// (created via `\\?\` paths or by other systems); acting on them through a plain
/// path would hit a different file, or the parent folder.
pub fn is_plain_win32_name(name: &str) -> bool {
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    if name.is_empty() || name.ends_with(['.', ' ']) {
        return false;
    }
    if name.chars().any(|c| c < ' ' || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')) {
        return false;
    }
    let stem = name.split('.').next().unwrap_or("").trim_end_matches(' ').to_ascii_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix)
            .is_some_and(|rest| matches!(rest, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"))
    };
    !(RESERVED.contains(&stem.as_str()) || numbered("COM") || numbered("LPT"))
}

/// Turns an internal path (on Windows, `\\?\`-prefixed) into the form users expect.
pub fn display_path(path: &Path) -> std::path::PathBuf {
    imp::display_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_up_to_whole_clusters() {
        assert_eq!(round_up(0, 4096), 0);
        assert_eq!(round_up(1, 4096), 4096);
        assert_eq!(round_up(4096, 4096), 4096);
        assert_eq!(round_up(4097, 4096), 8192);
    }

    #[test]
    fn plain_win32_names() {
        for ok in ["file.txt", "Отчёт 2024.docx", ".gitignore", "a b", "CONFIG.SYS", "console", "COM10", "nul-ish"] {
            assert!(is_plain_win32_name(ok), "{ok:?} should be fine");
        }
        for bad in [
            "", ".", "..", "...", "name.", "name ", " . ", "a:b", "a\\b", "a/b", "a?b", "a*b", "a|b", "a\"b", "a<b",
            "tab\there", "CON", "con", "NUL.txt", "aux.tar.gz", "COM1", "lpt9.log", "COM¹", "PRN ",
        ] {
            assert!(!is_plain_win32_name(bad), "{bad:?} should be refused");
        }
    }

    #[cfg(windows)]
    #[test]
    fn strips_verbatim_prefix() {
        assert_eq!(display_path(Path::new(r"\\?\C:\Users")), Path::new(r"C:\Users"));
        assert_eq!(display_path(Path::new(r"\\?\UNC\srv\share\x")), Path::new(r"\\srv\share\x"));
        assert_eq!(display_path(Path::new(r"D:\a")), Path::new(r"D:\a"));
    }
}
