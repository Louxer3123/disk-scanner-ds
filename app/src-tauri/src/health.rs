//! The Disk health screen: S.M.A.R.T. of every physical disk, with the drive
//! letters on it.
//!
//! Reading S.M.A.R.T. needs administrator rights. On Windows the whole app
//! restarts elevated (fast mode). On Linux a WebView must not run as root, so
//! the app runs itself once through `pkexec` with `--health-json`: that copy
//! has no window, prints the disks as JSON and exits.

use scanner_core::health::{DiskHealth, SmartError, list_disks};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// The argument that makes the program print [`list_disks`] as JSON and exit.
pub const HEALTH_JSON_ARG: &str = "--health-json";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttributeDto {
    id: u8,
    name: String,
    current: u8,
    worst: u8,
    threshold: u8,
    raw: u64,
    prefailure: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskHealthDto {
    number: u32,
    model: String,
    serial: String,
    firmware: String,
    bus: String,
    size: u64,
    rotational: Option<bool>,
    /// `good`, `caution`, `bad` or `unknown`.
    verdict: String,
    reasons: Vec<String>,
    /// Drive letters on this disk, e.g. `["C:", "D:"]`; mount points on Linux.
    letters: Vec<String>,
    temperature: Option<i32>,
    power_on_hours: Option<u64>,
    power_cycles: Option<u64>,
    unsafe_shutdowns: Option<u64>,
    bytes_written: Option<u64>,
    bytes_read: Option<u64>,
    life_left: Option<u8>,
    available_spare: Option<u8>,
    spare_threshold: Option<u8>,
    media_errors: Option<u64>,
    reallocated_sectors: Option<u64>,
    pending_sectors: Option<u64>,
    uncorrectable_sectors: Option<u64>,
    attributes: Vec<AttributeDto>,
    /// `needs_admin` or `unsupported` when there is no S.M.A.R.T. data.
    smart_error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthDto {
    disks: Vec<DiskHealthDto>,
    elevated: bool,
    /// How the UI gets S.M.A.R.T. that needs rights: `restart` (Windows, the
    /// fast mode restart), `pkexec` (Linux, [`disk_health_elevated`]) or `none`.
    elevation: &'static str,
}

fn dto(d: DiskHealth) -> DiskHealthDto {
    let f = d.facts;
    DiskHealthDto {
        number: d.number,
        model: d.model,
        serial: d.serial,
        firmware: d.firmware,
        bus: d.bus.to_owned(),
        size: d.size,
        rotational: d.rotational,
        verdict: d.verdict.as_str().to_owned(),
        reasons: d.reasons,
        letters: Vec::new(),
        temperature: f.temperature,
        power_on_hours: f.power_on_hours,
        power_cycles: f.power_cycles,
        unsafe_shutdowns: f.unsafe_shutdowns,
        bytes_written: f.bytes_written,
        bytes_read: f.bytes_read,
        life_left: f.life_left,
        available_spare: f.available_spare,
        spare_threshold: f.spare_threshold,
        media_errors: f.media_errors,
        reallocated_sectors: f.reallocated_sectors,
        pending_sectors: f.pending_sectors,
        uncorrectable_sectors: f.uncorrectable_sectors,
        attributes: d
            .attributes
            .into_iter()
            .map(|a| AttributeDto {
                id: a.id,
                name: a.name.to_owned(),
                current: a.current,
                worst: a.worst,
                threshold: a.threshold,
                raw: a.raw,
                prefailure: a.prefailure,
            })
            .collect(),
        smart_error: d.smart_error.map(|e| {
            match e {
                SmartError::NeedsAdmin => "needs_admin",
                SmartError::Unsupported => "unsupported",
            }
            .to_owned()
        }),
    }
}

fn elevation() -> &'static str {
    if cfg!(windows) {
        "restart"
    } else if cfg!(target_os = "linux") {
        "pkexec"
    } else {
        "none"
    }
}

/// Puts the drive letters (mount points) of each disk next to it.
fn with_letters(app: &AppHandle, mut disks: Vec<DiskHealthDto>) -> Vec<DiskHealthDto> {
    let state = app.state::<AppState>();
    let drives = state.drives();
    for disk in &mut disks {
        disk.letters = drives
            .iter()
            .filter(|d| d.present && d.info.physical_disk == Some(disk.number))
            .map(|d| d.info.root.trim_end_matches('\\').to_owned())
            .collect();
    }
    disks
}

/// For [`HEALTH_JSON_ARG`]: prints the disks as JSON on stdout.
pub fn print_json() {
    let disks: Vec<DiskHealthDto> = list_disks().into_iter().map(dto).collect();
    println!("{}", serde_json::to_string(&disks).unwrap_or_else(|_| "[]".into()));
}

/// Reads the health of every physical disk. Takes up to a second or two: each
/// disk answers a few device queries.
#[tauri::command]
pub async fn disk_health(app: AppHandle) -> Result<HealthDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let disks = list_disks().into_iter().map(dto).collect();
        Ok(HealthDto { disks: with_letters(&app, disks), elevated: scanner_core::is_elevated(), elevation: elevation() })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Linux: reads the health as root, through `pkexec`, which asks for the
/// password. Errors if the user cancels or `pkexec` is missing.
#[tauri::command]
pub async fn disk_health_elevated(app: AppHandle) -> Result<HealthDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let disks = read_as_root()?;
        Ok(HealthDto { disks: with_letters(&app, disks), elevated: true, elevation: elevation() })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(target_os = "linux")]
fn read_as_root() -> Result<Vec<DiskHealthDto>, String> {
    use crate::i18n::tr;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let out = std::process::Command::new("pkexec").arg(exe).arg(HEALTH_JSON_ARG).output().map_err(|e| {
        format!("{}: {e}", tr("Не удалось запустить pkexec (нужен polkit)", "Could not run pkexec (polkit is needed)"))
    })?;
    match out.status.code() {
        Some(0) => serde_json::from_slice(&out.stdout).map_err(|e| e.to_string()),
        // pkexec: 126 when the dialog was dismissed or authorization failed, 127 when not authorized.
        Some(126 | 127) => Err(tr("Доступ не получен", "Access was not granted").into()),
        _ => Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()),
    }
}

#[cfg(not(target_os = "linux"))]
fn read_as_root() -> Result<Vec<DiskHealthDto>, String> {
    Err(crate::i18n::tr("Только в Linux", "Linux only").into())
}
