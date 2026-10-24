//! The Disk health screen: S.M.A.R.T. of every physical disk, with the drive
//! letters on it.

use scanner_core::health::{DiskHealth, SmartError, list_disks};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttributeDto {
    id: u8,
    name: &'static str,
    current: u8,
    worst: u8,
    threshold: u8,
    raw: u64,
    prefailure: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskHealthDto {
    number: u32,
    model: String,
    serial: String,
    firmware: String,
    bus: &'static str,
    size: u64,
    rotational: Option<bool>,
    /// `good`, `caution`, `bad` or `unknown`.
    verdict: &'static str,
    reasons: Vec<String>,
    /// Drive letters on this disk, e.g. `["C:", "D:"]`.
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
    smart_error: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthDto {
    disks: Vec<DiskHealthDto>,
    elevated: bool,
}

fn dto(d: DiskHealth, letters: Vec<String>) -> DiskHealthDto {
    let f = d.facts;
    DiskHealthDto {
        number: d.number,
        model: d.model,
        serial: d.serial,
        firmware: d.firmware,
        bus: d.bus,
        size: d.size,
        rotational: d.rotational,
        verdict: d.verdict.as_str(),
        reasons: d.reasons,
        letters,
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
                name: a.name,
                current: a.current,
                worst: a.worst,
                threshold: a.threshold,
                raw: a.raw,
                prefailure: a.prefailure,
            })
            .collect(),
        smart_error: d.smart_error.map(|e| match e {
            SmartError::NeedsAdmin => "needs_admin",
            SmartError::Unsupported => "unsupported",
        }),
    }
}

/// Reads the health of every physical disk. Takes up to a second or two: each
/// disk answers a few device queries.
#[tauri::command]
pub async fn disk_health(app: AppHandle) -> Result<HealthDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let disks = list_disks();
        let letters_of = |number: u32| -> Vec<String> {
            app.state::<AppState>()
                .drives()
                .iter()
                .filter(|d| d.present && d.info.physical_disk == Some(number))
                .map(|d| d.info.root.trim_end_matches('\\').to_owned())
                .collect()
        };
        Ok(HealthDto {
            disks: disks
                .into_iter()
                .map(|d| {
                    let letters = letters_of(d.number);
                    dto(d, letters)
                })
                .collect(),
            elevated: scanner_core::is_elevated(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}
