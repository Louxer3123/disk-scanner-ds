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

#[cfg(not(windows))]
mod imp {
    use super::{DriveInfo, DriveKind};

    pub fn list_drives() -> Vec<DriveInfo> {
        vec![DriveInfo {
            root: "/".into(),
            label: String::new(),
            file_system: String::new(),
            kind: DriveKind::Fixed,
            capacity: 0,
            free: 0,
            physical_disk: None,
            rotational: None,
            ready: true,
        }]
    }

    pub fn drive_info_for(root: &str) -> Option<DriveInfo> {
        list_drives().into_iter().find(|d| d.root == root)
    }

    pub fn present_roots() -> Vec<String> {
        vec!["/".into()]
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
    }
}
