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
    /// `false` for an empty card reader or optical drive.
    pub ready: bool,
}

impl DriveInfo {
    pub fn used(&self) -> u64 {
        self.capacity.saturating_sub(self.free)
    }
}

/// All drive letters present right now, in letter order.
pub fn list_drives() -> Vec<DriveInfo> {
    imp::list_drives()
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
    use windows_sys::Win32::System::Ioctl::{IOCTL_STORAGE_GET_DEVICE_NUMBER, STORAGE_DEVICE_NUMBER};
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

    pub fn list_drives() -> Vec<DriveInfo> {
        // SAFETY: no arguments.
        let mask = unsafe { GetLogicalDrives() };
        (0..26u8)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| drive_info(char::from(b'A' + i)))
            .collect()
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

        DriveInfo {
            root,
            label: from_wide(&label),
            file_system: from_wide(&fs),
            kind,
            capacity,
            free,
            physical_disk: if ready && kind != DriveKind::Network { physical_disk(letter) } else { None },
            ready,
        }
    }

    fn physical_disk(letter: char) -> Option<u32> {
        let device = wide(&format!(r"\\.\{letter}:"));
        // SAFETY: zero access rights are enough for this query and need no admin
        // rights; the handle is closed before returning.
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
                return None;
            }
            let mut number: STORAGE_DEVICE_NUMBER = std::mem::zeroed();
            let mut returned = 0u32;
            let ok = DeviceIoControl(
                handle,
                IOCTL_STORAGE_GET_DEVICE_NUMBER,
                std::ptr::null(),
                0,
                (&raw mut number).cast(),
                size_of::<STORAGE_DEVICE_NUMBER>() as u32,
                &mut returned,
                null_mut(),
            ) != 0;
            CloseHandle(handle);
            ok.then_some(number.DeviceNumber)
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
            ready: true,
        }]
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
        }
    }
}
