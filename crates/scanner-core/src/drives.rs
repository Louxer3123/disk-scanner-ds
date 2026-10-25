//! Lists the drives of the system with their capacity and physical disk.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveKind {
    Fixed,
    Removable,
    Network,
    CdRom,
    RamDisk,
    Unknown,
}

impl DriveKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DriveKind::Fixed => "fixed",
            DriveKind::Removable => "removable",
            DriveKind::Network => "network",
            DriveKind::CdRom => "cdrom",
            DriveKind::RamDisk => "ram",
            DriveKind::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriveInfo {
    /// Root path, e.g. `C:\`.
    pub root: String,
    pub label: String,
    pub file_system: String,
    pub kind: DriveKind,
    pub capacity: u64,
    pub free: u64,
    /// Number of the physical disk holding the volume. Volumes with the same number
    /// should be scanned one after another; different disks can be scanned in parallel.
    pub physical_disk: Option<u32>,
    /// `Some(true)` for a spinning disk (seeks are costly), `Some(false)` for an
    /// SSD, `None` if the device does not say.
    pub rotational: Option<bool>,
    /// `false` for an empty card reader or optical drive.
    pub ready: bool,
}

impl DriveInfo {
    pub fn used(&self) -> u64 {
        self.capacity.saturating_sub(self.free)
    }

    /// Threads worth using to scan this drive: one on a spinning disk, where
    /// parallel reads only add seeks; several otherwise.
    pub fn scan_threads(&self) -> usize {
        if self.rotational == Some(true) { 1 } else { crate::default_threads() }
    }
}

/// All drive letters present right now, in letter order.
pub fn list_drives() -> Vec<DriveInfo> {
    imp::list_drives()
}

/// Roots of the drive letters present right now. Much cheaper than
/// [`list_drives`]: it does not touch the volumes, so it is fine to poll.
pub fn present_roots() -> Vec<String> {
    imp::present_roots()
}

/// Fresh information about one drive, given its root such as `C:\`.
pub fn drive_info(root: &str) -> Option<DriveInfo> {
    imp::drive_info_for(root)
}

#[cfg(windows)]
mod imp {
    use super::{DriveInfo, DriveKind};
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives,
        GetVolumeInformationW, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{
        DEVICE_SEEK_PENALTY_DESCRIPTOR, IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_QUERY_PROPERTY,
        PropertyStandardQuery, STORAGE_DEVICE_NUMBER, STORAGE_PROPERTY_QUERY, StorageDeviceSeekPenaltyProperty,
    };
    use windows_sys::Win32::System::WindowsProgramming::{
        DRIVE_CDROM, DRIVE_FIXED, DRIVE_RAMDISK, DRIVE_REMOTE, DRIVE_REMOVABLE,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn from_wide(buf: &[u16]) -> String {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len])
    }

    fn letters() -> impl Iterator<Item = char> {
        // SAFETY: no arguments.
        let mask = unsafe { GetLogicalDrives() };
        (0..26u8).filter(move |i| mask & (1 << i) != 0).map(|i| char::from(b'A' + i))
    }

    pub fn list_drives() -> Vec<DriveInfo> {
        letters().map(drive_info).collect()
    }

    pub fn present_roots() -> Vec<String> {
        letters().map(|l| format!("{l}:\\")).collect()
    }

    pub fn drive_info_for(root: &str) -> Option<DriveInfo> {
        let mut chars = root.chars();
        let letter = chars.next()?.to_ascii_uppercase();
        (letter.is_ascii_uppercase() && chars.next() == Some(':')).then(|| drive_info(letter))
    }

    fn drive_info(letter: char) -> DriveInfo {
        let root = format!("{letter}:\\");
        let root_w = wide(&root);
        // SAFETY: `root_w` is NUL-terminated.
        let kind = match unsafe { GetDriveTypeW(root_w.as_ptr()) } {
            DRIVE_FIXED => DriveKind::Fixed,
            DRIVE_REMOVABLE => DriveKind::Removable,
            DRIVE_REMOTE => DriveKind::Network,
            DRIVE_CDROM => DriveKind::CdRom,
            DRIVE_RAMDISK => DriveKind::RamDisk,
            _ => DriveKind::Unknown,
        };

        let mut label = [0u16; 261];
        let mut fs = [0u16; 261];
        // SAFETY: buffer lengths match the arrays; unused outputs are null.
        let ready = unsafe {
            GetVolumeInformationW(
                root_w.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                null_mut(),
                null_mut(),
                null_mut(),
                fs.as_mut_ptr(),
                fs.len() as u32,
            ) != 0
        };

        let (mut capacity, mut free) = (0u64, 0u64);
        if ready {
            // SAFETY: valid out-pointers; the caller-quota output is not needed.
            unsafe { GetDiskFreeSpaceExW(root_w.as_ptr(), null_mut(), &mut capacity, &mut free) };
        }

        let (physical_disk, rotational) =
            if ready && kind != DriveKind::Network { device_facts(letter) } else { (None, None) };
        DriveInfo {
            root,
            label: from_wide(&label),
            file_system: from_wide(&fs),
            kind,
            capacity,
            free,
            physical_disk,
            rotational,
            ready,
        }
    }

    /// Physical disk number and whether it is a spinning disk.
    fn device_facts(letter: char) -> (Option<u32>, Option<bool>) {
        let device = wide(&format!(r"\\.\{letter}:"));
        // SAFETY: zero access rights are enough for both queries and need no admin
        // rights; the out-buffers match the sizes passed; the handle is closed below.
        unsafe {
            let handle = CreateFileW(
                device.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            );
            if handle == INVALID_HANDLE_VALUE {
                return (None, None);
            }
            let mut returned = 0u32;

            let mut number: STORAGE_DEVICE_NUMBER = std::mem::zeroed();
            let number_ok = DeviceIoControl(
                handle,
                IOCTL_STORAGE_GET_DEVICE_NUMBER,
                std::ptr::null(),
                0,
                (&raw mut number).cast(),
                size_of::<STORAGE_DEVICE_NUMBER>() as u32,
                &mut returned,
                null_mut(),
            ) != 0;

            let query = STORAGE_PROPERTY_QUERY {
                PropertyId: StorageDeviceSeekPenaltyProperty,
                QueryType: PropertyStandardQuery,
                AdditionalParameters: [0],
            };
            let mut seek: DEVICE_SEEK_PENALTY_DESCRIPTOR = std::mem::zeroed();
            let seek_ok = DeviceIoControl(
                handle,
                IOCTL_STORAGE_QUERY_PROPERTY,
                (&raw const query).cast(),
                size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                (&raw mut seek).cast(),
                size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>() as u32,
                &mut returned,
                null_mut(),
            ) != 0;

            CloseHandle(handle);
            (number_ok.then_some(number.DeviceNumber), seek_ok.then_some(seek.IncursSeekPenalty != 0))
        }
    }
}

/// Linux and macOS: every mounted file system worth showing (block devices
/// and network shares, not `/proc`, `tmpfs`, snaps and the like) is a "drive",
/// its mount point the root.
#[cfg(unix)]
mod imp {
    use super::{DriveInfo, DriveKind};

    pub(super) struct Mount {
        pub root: String,
        pub device: String,
        pub fs: String,
        pub kind: DriveKind,
    }

    /// The physical disk under a device and whether it spins.
    pub(super) struct Disk {
        pub number: u32,
        pub rotational: Option<bool>,
    }

    pub fn list_drives() -> Vec<DriveInfo> {
        os::mounts().into_iter().map(info).collect()
    }

    pub fn present_roots() -> Vec<String> {
        os::mounts().into_iter().map(|m| m.root).collect()
    }

    pub fn drive_info_for(root: &str) -> Option<DriveInfo> {
        os::mounts().into_iter().find(|m| m.root == root).map(info)
    }

    fn info(m: Mount) -> DriveInfo {
        let space = os::space(&m.root);
        let disk = os::disk_of(&m.device);
        let kind = match m.kind {
            DriveKind::Fixed if disk.as_ref().is_some_and(|d| os::removable(d.number)) => DriveKind::Removable,
            kind => kind,
        };
        DriveInfo {
            label: os::label(&m),
            capacity: space.map_or(0, |s| s.0),
            free: space.map_or(0, |s| s.1),
            ready: space.is_some(),
            physical_disk: disk.as_ref().map(|d| d.number),
            rotational: disk.and_then(|d| d.rotational),
            root: m.root,
            file_system: m.fs,
            kind,
        }
    }

    #[cfg(target_os = "linux")]
    mod os {
        use super::{Disk, Mount};
        use crate::drives::DriveKind;
        use std::ffi::CString;
        use std::fs;
        use std::path::{Path, PathBuf};

        const NETWORK: [&str; 7] = ["nfs", "nfs4", "cifs", "smb3", "smbfs", "fuse.sshfs", "9p"];
        /// Block-device file systems that are not user storage: snap and
        /// AppImage loop images.
        const SKIP: [&str; 1] = ["squashfs"];

        pub fn mounts() -> Vec<Mount> {
            let Ok(text) = fs::read_to_string("/proc/self/mounts") else { return Vec::new() };
            let mut out: Vec<Mount> = Vec::new();
            for line in text.lines() {
                let mut fields = line.split(' ');
                let (Some(device), Some(root), Some(fs)) = (fields.next(), fields.next(), fields.next()) else {
                    continue;
                };
                let (device, root) = (unescape(device), unescape(root));
                let network = NETWORK.contains(&fs);
                let block = device.starts_with("/dev/") && !SKIP.contains(&fs);
                // `/` always counts, even as an overlay inside a container.
                if !(network || block || root == "/") || root.starts_with("/snap/") {
                    continue;
                }
                let kind = if network {
                    DriveKind::Network
                } else if matches!(fs, "iso9660" | "udf") {
                    DriveKind::CdRom
                } else {
                    DriveKind::Fixed
                };
                // A later mount on the same place hides the earlier one.
                out.retain(|m| m.root != root);
                out.push(Mount { root, device, fs: fs.to_owned(), kind });
            }
            out.sort_by(|a, b| a.root.cmp(&b.root));
            out
        }

        /// Mount table fields escape space, tab, newline and `\` as octal.
        fn unescape(field: &str) -> String {
            let bytes = field.as_bytes();
            let mut out = Vec::with_capacity(bytes.len());
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'\\'
                    && let Some(code) = field.get(i + 1..i + 4).and_then(|o| u8::from_str_radix(o, 8).ok())
                {
                    out.push(code);
                    i += 4;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            String::from_utf8_lossy(&out).into_owned()
        }

        pub fn space(root: &str) -> Option<(u64, u64)> {
            let path = CString::new(root).ok()?;
            // SAFETY: `path` is NUL-terminated; `st` is a plain struct the call fills.
            unsafe {
                let mut st: libc::statvfs = std::mem::zeroed();
                if libc::statvfs(path.as_ptr(), &mut st) != 0 {
                    return None;
                }
                let unit = st.f_frsize as u64;
                Some((st.f_blocks as u64 * unit, st.f_bavail as u64 * unit))
            }
        }

        /// The disk under `/dev/sda2`, `/dev/nvme0n1p1` or a device-mapper
        /// volume (LVM, LUKS), numbered by its place among `/sys/block`.
        pub fn disk_of(device: &str) -> Option<Disk> {
            let name = disk_name(fs::canonicalize(device).ok()?.file_name()?.to_str()?, 0)?;
            let mut disks: Vec<String> =
                fs::read_dir("/sys/block").ok()?.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect();
            disks.sort();
            let number = disks.iter().position(|d| *d == name)? as u32;
            let rotational = match read_sys(&name, "queue/rotational").as_deref() {
                Some("1") => Some(true),
                Some("0") => Some(false),
                _ => None,
            };
            Some(Disk { number, rotational })
        }

        fn disk_name(name: &str, depth: u32) -> Option<String> {
            let sys = Path::new("/sys/class/block").join(name);
            if sys.join("partition").exists() {
                let real: PathBuf = fs::canonicalize(&sys).ok()?;
                return Some(real.parent()?.file_name()?.to_str()?.to_owned());
            }
            // A device-mapper volume: follow it down to the disk it sits on.
            if depth < 4
                && let Some(slave) = fs::read_dir(sys.join("slaves")).ok().and_then(|mut d| d.next()?.ok())
            {
                return disk_name(slave.file_name().to_str()?, depth + 1);
            }
            Some(name.to_owned())
        }

        fn read_sys(disk: &str, file: &str) -> Option<String> {
            Some(fs::read_to_string(Path::new("/sys/block").join(disk).join(file)).ok()?.trim().to_owned())
        }

        pub fn removable(number: u32) -> bool {
            let Ok(entries) = fs::read_dir("/sys/block") else { return false };
            let mut disks: Vec<String> = entries.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect();
            disks.sort();
            disks.get(number as usize).is_some_and(|d| read_sys(d, "removable").as_deref() == Some("1"))
        }

        /// The file system label from `/dev/disk/by-label`, if it has one.
        pub fn label(m: &Mount) -> String {
            let Ok(device) = fs::canonicalize(&m.device) else { return String::new() };
            let Ok(entries) = fs::read_dir("/dev/disk/by-label") else { return String::new() };
            entries
                .filter_map(Result::ok)
                .find(|e| fs::canonicalize(e.path()).is_ok_and(|p| p == device))
                .map(|e| unhex(&e.file_name().to_string_lossy()))
                .unwrap_or_default()
        }

        /// udev writes unsafe characters of a label as `\x20`.
        fn unhex(name: &str) -> String {
            let mut out = Vec::new();
            let bytes = name.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if name.get(i..i + 2) == Some("\\x")
                    && let Some(code) = name.get(i + 2..i + 4).and_then(|h| u8::from_str_radix(h, 16).ok())
                {
                    out.push(code);
                    i += 4;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            String::from_utf8_lossy(&out).into_owned()
        }

        #[cfg(test)]
        mod tests {
            #[test]
            fn mount_table_escapes() {
                assert_eq!(super::unescape(r"/media/my\040disk"), "/media/my disk");
                assert_eq!(super::unescape(r"/a\134b"), r"/a\b");
                assert_eq!(super::unescape("/plain"), "/plain");
                assert_eq!(super::unhex(r"My\x20Disk"), "My Disk");
            }
        }
    }

    #[cfg(target_os = "macos")]
    mod os {
        use super::{Disk, Mount};
        use crate::drives::DriveKind;
        use std::ffi::{CStr, CString};

        const NETWORK: [&str; 4] = ["smbfs", "nfs", "afpfs", "webdav"];

        fn text(field: &[libc::c_char]) -> String {
            // SAFETY: the kernel NUL-terminates these fixed-size fields.
            unsafe { CStr::from_ptr(field.as_ptr()) }.to_string_lossy().into_owned()
        }

        /// Mounted volumes. The system volume is `/`; user data lives on the
        /// `/System/Volumes/Data` volume (firmlinked into `/`), shown on its own.
        /// The other `/System/Volumes/*` (VM, Preboot, Update...) are left out.
        pub fn mounts() -> Vec<Mount> {
            // SAFETY: a null buffer asks for the count; the second call fills at
            // most `cap` entries of a buffer of exactly that many.
            let all = unsafe {
                let count = libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT);
                if count <= 0 {
                    return Vec::new();
                }
                let cap = count as usize + 4;
                let mut buf: Vec<libc::statfs> = vec![std::mem::zeroed(); cap];
                let bytes = (cap * std::mem::size_of::<libc::statfs>()) as libc::c_int;
                let n = libc::getfsstat(buf.as_mut_ptr(), bytes, libc::MNT_NOWAIT);
                buf.truncate(n.max(0) as usize);
                buf
            };
            let mut out = Vec::new();
            for st in &all {
                let (root, device, fs) = (text(&st.f_mntonname), text(&st.f_mntfromname), text(&st.f_fstypename));
                let network = NETWORK.contains(&fs.as_str());
                let local = u64::from(st.f_flags) & libc::MNT_LOCAL as u64 != 0;
                if !(network || local) || matches!(fs.as_str(), "devfs" | "autofs" | "nullfs") {
                    continue;
                }
                if root.starts_with("/System/Volumes/") && root != "/System/Volumes/Data" {
                    continue;
                }
                let kind = if network {
                    DriveKind::Network
                } else if matches!(fs.as_str(), "cd9660" | "udf") {
                    DriveKind::CdRom
                } else if root.starts_with("/Volumes/") {
                    DriveKind::Removable
                } else {
                    DriveKind::Fixed
                };
                out.push(Mount { root, device, fs, kind });
            }
            out.sort_by(|a, b| a.root.cmp(&b.root));
            out
        }

        pub fn space(root: &str) -> Option<(u64, u64)> {
            let path = CString::new(root).ok()?;
            // SAFETY: `path` is NUL-terminated; `st` is a plain struct the call fills.
            unsafe {
                let mut st: libc::statfs = std::mem::zeroed();
                if libc::statfs(path.as_ptr(), &mut st) != 0 {
                    return None;
                }
                let unit = u64::from(st.f_bsize);
                Some((st.f_blocks * unit, st.f_bavail * unit))
            }
        }

        /// `/dev/disk3s1s1` is on disk 3 (for APFS, the container).
        pub fn disk_of(device: &str) -> Option<Disk> {
            let rest = device.strip_prefix("/dev/disk")?;
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            Some(Disk { number: digits.parse().ok()?, rotational: None })
        }

        pub fn removable(_number: u32) -> bool {
            false
        }

        pub fn label(m: &Mount) -> String {
            match m.root.as_str() {
                "/" => "Macintosh HD".into(),
                "/System/Volumes/Data" => "Data".into(),
                root => root.rsplit('/').next().unwrap_or_default().to_owned(),
            }
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    mod os {
        use super::{Disk, Mount};
        use crate::drives::DriveKind;

        pub fn mounts() -> Vec<Mount> {
            vec![Mount { root: "/".into(), device: String::new(), fs: String::new(), kind: DriveKind::Fixed }]
        }
        pub fn space(_root: &str) -> Option<(u64, u64)> {
            Some((0, 0))
        }
        pub fn disk_of(_device: &str) -> Option<Disk> {
            None
        }
        pub fn removable(_number: u32) -> bool {
            false
        }
        pub fn label(_m: &Mount) -> String {
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_system_drive() {
        let drives = list_drives();
        assert!(!drives.is_empty());
        if cfg!(windows) {
            let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
            let d = drives
                .iter()
                .find(|d| d.root.eq_ignore_ascii_case(&format!("{system}\\")))
                .expect("system drive missing");
            assert_eq!(d.kind, DriveKind::Fixed);
            assert!(d.ready && d.capacity > 0 && d.free <= d.capacity);
            assert!(!d.file_system.is_empty());
            assert!(d.physical_disk.is_some(), "physical disk number not found");
            eprintln!("system drive rotational: {:?}", d.rotational);
        }
        if cfg!(any(target_os = "linux", target_os = "macos")) {
            let d = drives.iter().find(|d| d.root == "/").expect("/ missing");
            assert!(d.ready && d.capacity > 0 && d.free <= d.capacity, "{d:?}");
            for d in &drives {
                eprintln!("{d:?}");
            }
        }
    }
}
