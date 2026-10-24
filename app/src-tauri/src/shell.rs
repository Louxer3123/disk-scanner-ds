//! Windows shell integration: open and reveal files, the recycle bin, the
//! "Apps" settings page and the Explorer context menu entry.

use std::path::Path;

pub struct RecycleBin {
    pub size: u64,
    pub items: u64,
}

pub use imp::*;

#[cfg(windows)]
mod imp {
    use super::RecycleBin;
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, MAX_PATH};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW,
    };
    use windows_sys::Win32::UI::Shell::{
        SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW, SHQUERYRBINFO,
        SHQueryRecycleBinW, ShellExecuteW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    /// Per-user verb keys: on folders, on drives, and on the empty area of a folder window.
    const MENU_KEYS: [(&str, &str); 3] = [
        (r"Software\Classes\Directory\shell\DiskScanner", "%1"),
        (r"Software\Classes\Drive\shell\DiskScanner", "%1"),
        (r"Software\Classes\Directory\Background\shell\DiskScanner", "%V"),
    ];
    const MENU_TITLE: &str = "Показать занятое место";

    fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        s.as_ref().encode_wide().chain(once(0)).collect()
    }

    fn shell_execute(target: &str) -> Result<(), String> {
        let (verb, file) = (wide("open"), wide(target));
        // SAFETY: NUL-terminated strings; no parent window.
        let result = unsafe { ShellExecuteW(null_mut(), verb.as_ptr(), file.as_ptr(), null(), null(), SW_SHOWNORMAL) };
        // Values above 32 mean success.
        if result as isize > 32 { Ok(()) } else { Err(format!("Не удалось открыть (код {})", result as isize)) }
    }

    /// Opens a file or folder with its default application.
    ///
    /// In fast mode this process runs as administrator, and ShellExecute would
    /// start the file (say, a downloaded `.exe`) with full rights too. explorer.exe
    /// instead hands the request to the user's already running, unelevated shell,
    /// so the file opens with normal rights.
    pub fn open(path: &Path) -> Result<(), String> {
        if scanner_core::is_elevated() {
            return std::process::Command::new("explorer.exe")
                .raw_arg(format!("\"{}\"", path.display()))
                .spawn()
                .map(drop)
                .map_err(|e| e.to_string());
        }
        shell_execute(&path.to_string_lossy())
    }

    /// Opens Explorer with `path` selected.
    pub fn reveal(path: &Path) -> Result<(), String> {
        // explorer.exe parses its own command line: the path must be quoted after the comma.
        std::process::Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()
            .map(drop)
            .map_err(|e| e.to_string())
    }

    pub fn open_apps_settings() -> Result<(), String> {
        shell_execute("ms-settings:appsfeatures")
    }

    /// Starts another copy of this program with administrator rights; Windows
    /// shows its UAC prompt first. Fails if the user declines.
    pub fn relaunch_elevated() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let (verb, file) = (wide("runas"), wide(&exe));
        // SAFETY: NUL-terminated strings; no parent window.
        let result = unsafe { ShellExecuteW(null_mut(), verb.as_ptr(), file.as_ptr(), null(), null(), SW_SHOWNORMAL) };
        if result as isize > 32 { Ok(()) } else { Err("Запуск с правами администратора отменён".into()) }
    }

    /// Size and item count of the recycle bins of all drives.
    pub fn recycle_bin() -> Result<RecycleBin, String> {
        let mut info = SHQUERYRBINFO { cbSize: size_of::<SHQUERYRBINFO>() as u32, i64Size: 0, i64NumItems: 0 };
        // SAFETY: `info` is a valid, sized struct; a null root means all drives.
        let hr = unsafe { SHQueryRecycleBinW(null(), &mut info) };
        if hr < 0 {
            return Err(format!("Не удалось прочитать корзину (0x{hr:08X})"));
        }
        Ok(RecycleBin { size: info.i64Size.max(0) as u64, items: info.i64NumItems.max(0) as u64 })
    }

    /// Empties the recycle bins of all drives without Windows' own confirmation;
    /// the app asks first.
    pub fn empty_recycle_bin() -> Result<(), String> {
        // SAFETY: null window and root (all drives) are allowed.
        let hr = unsafe {
            SHEmptyRecycleBinW(null_mut(), null(), SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND)
        };
        // An already empty bin reports a failure code; that is fine.
        if hr < 0 && recycle_bin().is_ok_and(|b| b.items > 0) {
            return Err(format!("Не удалось очистить корзину (0x{hr:08X})"));
        }
        Ok(())
    }

    fn exe() -> Result<String, String> {
        std::env::current_exe().map(|p| p.display().to_string()).map_err(|e| e.to_string())
    }

    fn command_for(exe: &str, arg: &str) -> String {
        format!("\"{exe}\" \"{arg}\"")
    }

    /// Whether the Explorer context menu entry exists and points at this executable.
    pub fn explorer_menu_installed() -> bool {
        let Ok(exe) = exe() else { return false };
        MENU_KEYS.iter().all(|(key, arg)| read_default(&format!(r"{key}\command")) == Some(command_for(&exe, arg)))
    }

    /// Adds or removes "Показать занятое место" in Explorer's menu for folders and
    /// drives, for the current user only (HKCU, no admin rights needed).
    pub fn set_explorer_menu(enabled: bool) -> Result<(), String> {
        if !enabled {
            for (key, _) in MENU_KEYS {
                let k = wide(key);
                // SAFETY: NUL-terminated key; a missing key is not an error here.
                unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
            }
            return Ok(());
        }
        let exe = exe()?;
        for (key, arg) in MENU_KEYS {
            write_values(key, &[(None, MENU_TITLE), (Some("Icon"), &format!("\"{exe}\",0"))])?;
            write_values(&format!(r"{key}\command"), &[(None, &command_for(&exe, arg))])?;
        }
        Ok(())
    }

    fn write_values(key: &str, values: &[(Option<&str>, &str)]) -> Result<(), String> {
        let k = wide(key);
        let mut hkey: HKEY = null_mut();
        // SAFETY: NUL-terminated key; `hkey` receives an open handle that is closed below.
        let err = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                k.as_ptr(),
                0,
                null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                null(),
                &mut hkey,
                null_mut(),
            )
        };
        if err != ERROR_SUCCESS {
            return Err(format!("Не удалось записать в реестр (код {err})"));
        }
        let mut result = Ok(());
        for (name, value) in values {
            let name_w = name.map(wide);
            let data = wide(value);
            // SAFETY: `data` is a NUL-terminated UTF-16 string; its byte length includes the NUL.
            let err = unsafe {
                RegSetValueExW(
                    hkey,
                    name_w.as_ref().map_or(null(), |n| n.as_ptr()),
                    0,
                    REG_SZ,
                    data.as_ptr().cast(),
                    (data.len() * 2) as u32,
                )
            };
            if err != ERROR_SUCCESS {
                result = Err(format!("Не удалось записать в реестр (код {err})"));
                break;
            }
        }
        // SAFETY: `hkey` was opened above.
        unsafe { RegCloseKey(hkey) };
        result
    }

    fn read_default(key: &str) -> Option<String> {
        let k = wide(key);
        let mut buf = vec![0u16; 2 * MAX_PATH as usize];
        let mut bytes = (buf.len() * 2) as u32;
        // SAFETY: `bytes` holds the buffer size in bytes; RRF_RT_REG_SZ ensures termination.
        let err = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), null(), RRF_RT_REG_SZ, null_mut(), buf.as_mut_ptr().cast(), &mut bytes)
        };
        if err != ERROR_SUCCESS {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

#[cfg(not(windows))]
mod imp {
    use super::RecycleBin;
    use std::path::Path;

    const UNSUPPORTED: &str = "Доступно только в Windows";

    pub fn open(_path: &Path) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }
    pub fn reveal(_path: &Path) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }
    pub fn open_apps_settings() -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }
    pub fn relaunch_elevated() -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }
    pub fn recycle_bin() -> Result<RecycleBin, String> {
        Err(UNSUPPORTED.into())
    }
    pub fn empty_recycle_bin() -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }
    pub fn explorer_menu_installed() -> bool {
        false
    }
    pub fn set_explorer_menu(_enabled: bool) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }
}

/// Deletes `path` for good. Links (junctions, symlinks) are removed themselves,
/// never the folder they point to.
pub fn delete_permanently(path: &Path, is_dir: bool) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    let ft = meta.file_type();
    if ft.is_symlink() {
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileTypeExt;
            if ft.is_symlink_dir() {
                return std::fs::remove_dir(path);
            }
        }
        return std::fs::remove_file(path);
    }
    if is_dir { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) }
}

#[cfg(test)]
mod tests {
    use super::delete_permanently;
    use std::fs;

    #[test]
    fn permanent_delete_removes_files_and_folders() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f.txt");
        fs::write(&f, b"x").unwrap();
        delete_permanently(&f, false).unwrap();
        assert!(!f.exists());

        let d = dir.path().join("d");
        fs::create_dir_all(d.join("inner")).unwrap();
        fs::write(d.join("inner/x"), b"x").unwrap();
        delete_permanently(&d, true).unwrap();
        assert!(!d.exists());
    }

    #[cfg(windows)]
    #[test]
    fn deleting_a_junction_keeps_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep.txt"), b"x").unwrap();
        let link = dir.path().join("link");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .is_ok_and(|o| o.status.success());
        if !made {
            eprintln!("skipping: could not create a junction");
            return;
        }
        delete_permanently(&link, true).unwrap();
        assert!(!link.exists());
        assert!(target.join("keep.txt").exists(), "junction target must survive");
    }
}
