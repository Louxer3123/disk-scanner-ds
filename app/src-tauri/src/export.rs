//! Exporting the shown folder, drive or summary to a CSV or JSON file.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use scanner_core::Tree;
use scanner_core::export::{CsvStyle, Filter, write_csv, write_json};
use serde::Deserialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::commands::{View, scope};
use crate::state::AppState;

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    /// `;`-separated with a byte order mark, opens in Excel as is.
    Csv,
    Json,
}

/// Asks where to save, then writes the view's entries down to `max_depth` levels
/// (all when `None`) that take at least `min_size` bytes. Returns the file's
/// path, or `None` if the user cancelled the dialog.
#[tauri::command]
pub async fn export_view(
    app: AppHandle,
    view: View,
    format: ExportFormat,
    max_depth: Option<usize>,
    min_size: u64,
    file_name: String,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (filter_name, ext) = match format {
            ExportFormat::Csv => ("CSV", "csv"),
            ExportFormat::Json => ("JSON", "json"),
        };
        let Some(chosen) = app
            .dialog()
            .file()
            .set_file_name(&file_name)
            .add_filter(filter_name, &[ext])
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = chosen.into_path().map_err(|e| e.to_string())?;

        let parts = scope(&app.state::<AppState>().drives(), &view)?;
        let roots: Vec<(&Tree, _)> = parts.iter().map(|p| (p.tree.as_ref(), p.root)).collect();
        let filter = Filter { max_depth: max_depth.unwrap_or(usize::MAX), min_size };
        let written = File::create(&path).and_then(|f| {
            let mut out = BufWriter::new(f);
            match format {
                ExportFormat::Csv => write_csv(&mut out, &roots, filter, CsvStyle::Excel)?,
                ExportFormat::Json => {
                    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
                    write_json(&mut out, &roots, filter, now)?;
                }
            }
            out.flush()
        });
        if let Err(e) = written {
            let _ = fs::remove_file(&path);
            return Err(format!("Не удалось записать файл: {e}"));
        }
        Ok(Some(path.display().to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}
