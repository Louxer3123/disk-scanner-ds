//! Known system places (see [`scanner_core::known`]): the "Free up space" list
//! and the system tools its buttons open.

use scanner_core::known::{self, Known};
use scanner_core::{NodeId, Tree};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::commands::{View, scope};
use crate::i18n::tr;
use crate::shell;
use crate::state::AppState;

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct KnownDto {
    /// The rule's id; the UI has the texts under it.
    id: &'static str,
    /// `safe`, `tool` or `keep`.
    safety: &'static str,
    /// The node lies inside the known place rather than being it.
    inside: bool,
}

impl From<Known> for KnownDto {
    fn from(k: Known) -> Self {
        KnownDto { id: k.id, safety: k.safety.as_str(), inside: k.inside }
    }
}

/// What the catalogue knows about `node`, if anything.
pub fn known_of(tree: &Tree, node: NodeId) -> Option<KnownDto> {
    let path = tree.path(node);
    let path = path.to_string_lossy();
    known::catalogue().identify(&known::components(&path)).map(KnownDto::from)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupItem {
    id: &'static str,
    safety: &'static str,
    drive: usize,
    node: u32,
    tree_gen: u64,
    path: String,
    /// `dir`, `file` or `link`.
    kind: &'static str,
    /// The containing folder, to show a file in its place.
    parent: Option<u32>,
    allocated: u64,
}

/// Known places on the drives of `view`, largest first. A folder view lists
/// those of its whole drive: the page file sits at the root, not in the folder.
#[tauri::command]
pub async fn cleanup_report(app: AppHandle, view: View) -> Result<Vec<CleanupItem>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let drives = state.drives();
        let parts = scope(&drives, &view)?;
        let mut items = Vec::new();
        for p in &parts {
            let d = &drives[p.drive];
            let root = known::components(&d.info.root);
            for (rule, node) in known::catalogue().find_all(&d.tree, &root) {
                let allocated = d.tree.allocated(node);
                if allocated == 0 {
                    continue;
                }
                items.push(CleanupItem {
                    id: rule.id,
                    safety: rule.safety.as_str(),
                    drive: p.drive,
                    node: node.index() as u32,
                    tree_gen: d.tree_gen,
                    path: d.tree.path(node).to_string_lossy().into_owned(),
                    kind: d.tree.kind(node).as_str(),
                    parent: d.tree.parent(node).map(|p| p.index() as u32),
                    allocated,
                });
            }
        }
        items.sort_by_key(|i| std::cmp::Reverse(i.allocated));
        Ok(items)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Opens one of the system tools the hints point to. Only these names are
/// accepted; the program itself runs no cleanup command.
#[tauri::command]
pub fn open_tool(tool: String) -> Result<(), String> {
    let target = if cfg!(windows) {
        match tool.as_str() {
            "cleanmgr" => "cleanmgr.exe",
            "storage" => "ms-settings:storagesense",
            "restore" => "SystemPropertiesProtection.exe",
            "performance" => "SystemPropertiesPerformance.exe",
            "apps" => return shell::open_apps_settings(),
            _ => "",
        }
    } else {
        ""
    };
    if target.is_empty() {
        return Err(tr("Неизвестное средство", "Unknown tool").into());
    }
    shell::open(std::path::Path::new(target))
}
