//! Where the app keeps its data, and the UI's settings.
//!
//! Normally that is `%APPDATA%\io.github.diskscanner`. With a file named
//! `portable.txt` (or `portable`) next to the exe, everything (settings,
//! snapshots, the WebView's own data) goes to a `data` folder beside it
//! instead, so the program can live on a USB stick and leave nothing behind.
//!
//! Settings are string values by key in `settings.json`, written whole on every
//! change (they are a few hundred bytes).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tauri::State;

use crate::state::lock;

const IDENTIFIER: &str = "io.github.diskscanner";
const PORTABLE_MARKERS: [&str; 2] = ["portable.txt", "portable"];
const FILE: &str = "settings.json";
const MAX_KEYS: usize = 64;
const MAX_KEY: usize = 64;
const MAX_VALUE: usize = 16 * 1024;

pub struct Settings {
    pub data_dir: PathBuf,
    pub portable: bool,
    values: Mutex<BTreeMap<String, String>>,
}

impl Settings {
    /// Finds the data folder and reads the settings; a missing or damaged file
    /// gives empty settings.
    pub fn load() -> Self {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
        let portable_dir = exe_dir.filter(|d| PORTABLE_MARKERS.iter().any(|m| d.join(m).is_file()));
        let (data_dir, portable) = match portable_dir {
            Some(d) => (d.join("data"), true),
            None => {
                let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
                (base.join(IDENTIFIER), false)
            }
        };
        let values = fs::read_to_string(data_dir.join(FILE))
            .ok()
            .and_then(|text| serde_json::from_str::<BTreeMap<String, String>>(&text).ok())
            .map(|mut v| {
                v.retain(|k, val| valid_key(k) && val.len() <= MAX_VALUE);
                v
            })
            .unwrap_or_default();
        Settings { data_dir, portable, values: Mutex::new(values) }
    }

    /// The data folder's subfolder `name`, created if needed.
    pub fn dir(&self, name: &str) -> Result<PathBuf, String> {
        let dir = self.data_dir.join(name);
        fs::create_dir_all(&dir).map_err(|e| format!("Не удалось создать папку {}: {e}", dir.display()))?;
        Ok(dir)
    }

    /// Where the WebView keeps its cache and storage: in portable mode inside
    /// the data folder, otherwise the system default.
    pub fn webview_dir(&self) -> Option<PathBuf> {
        self.portable.then(|| self.data_dir.join("webview"))
    }

    fn save(&self, values: &BTreeMap<String, String>) -> Result<(), String> {
        fs::create_dir_all(&self.data_dir).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(values).map_err(|e| e.to_string())?;
        // Written aside and renamed, so a crash never leaves half a file.
        let tmp = self.data_dir.join("settings.tmp");
        fs::write(&tmp, text).and_then(|()| fs::rename(&tmp, self.data_dir.join(FILE))).map_err(|e| e.to_string())
    }
}

fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= MAX_KEY && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    values: BTreeMap<String, String>,
    data_dir: String,
    portable: bool,
    version: &'static str,
}

#[tauri::command]
pub fn get_settings(settings: State<'_, Settings>) -> SettingsDto {
    SettingsDto {
        values: lock(&settings.values).clone(),
        data_dir: settings.data_dir.display().to_string(),
        portable: settings.portable,
        version: env!("CARGO_PKG_VERSION"),
    }
}

/// Sets `key` to `value`, or removes it when `value` is `None`, and saves.
#[tauri::command]
pub fn set_setting(settings: State<'_, Settings>, key: String, value: Option<String>) -> Result<(), String> {
    if !valid_key(&key) || value.as_ref().is_some_and(|v| v.len() > MAX_VALUE) {
        return Err("Недопустимая настройка".into());
    }
    let mut values = lock(&settings.values);
    match value {
        Some(v) => {
            if !values.contains_key(&key) && values.len() >= MAX_KEYS {
                return Err("Слишком много настроек".into());
            }
            values.insert(key, v);
        }
        None => {
            values.remove(&key);
        }
    }
    settings.save(&values)
}

#[tauri::command]
pub fn open_data_dir(settings: State<'_, Settings>) -> Result<(), String> {
    fs::create_dir_all(&settings.data_dir).map_err(|e| e.to_string())?;
    crate::shell::open(&settings.data_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_plain_identifiers() {
        for ok in ["colorMode", "panel_tab", "a1"] {
            assert!(valid_key(ok), "{ok}");
        }
        for bad in ["", "a.b", "../x", "ключ", &"k".repeat(65)] {
            assert!(!valid_key(bad), "{bad}");
        }
    }

    #[test]
    fn settings_survive_a_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings { data_dir: dir.path().join("data"), portable: true, values: Mutex::new(BTreeMap::new()) };
        let mut v = BTreeMap::new();
        v.insert("colorMode".to_owned(), "type".to_owned());
        s.save(&v).unwrap();
        let text = fs::read_to_string(dir.path().join("data").join(FILE)).unwrap();
        let back: BTreeMap<String, String> = serde_json::from_str(&text).unwrap();
        assert_eq!(back, v);
        assert!(!dir.path().join("data").join("settings.tmp").exists());
        assert_eq!(s.webview_dir(), Some(dir.path().join("data").join("webview")));
    }
}
