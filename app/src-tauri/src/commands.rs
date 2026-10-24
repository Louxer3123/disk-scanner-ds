//! Commands and events exchanged with the UI.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use scanner_core::{CancelToken, Hit, LayoutParams, NodeId, NodeKind, SunburstLayout, Tree, display_path, scan};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::{AppState, Current, complete_drive_tree};

/// Target width of a ring; the number of rings follows from the window size.
const RING_WIDTH: f64 = 56.0;
const MIN_RINGS: u32 = 3;
const MAX_RINGS: u32 = 8;
/// Segments thinner than this at their outer edge are not drawn.
const MIN_ARC_PX: f64 = 1.5;
const PROGRESS_EVERY: Duration = Duration::from_millis(150);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveDto {
    index: usize,
    root: String,
    label: String,
    file_system: String,
    kind: &'static str,
    capacity: u64,
    free: u64,
    physical_disk: Option<u32>,
    ready: bool,
    scanned: bool,
    scanning: bool,
    tree_gen: u64,
}

#[tauri::command]
pub fn list_drives(state: State<'_, AppState>) -> Vec<DriveDto> {
    state
        .drives()
        .iter()
        .enumerate()
        .map(|(index, d)| DriveDto {
            index,
            root: d.info.root.clone(),
            label: d.info.label.clone(),
            file_system: d.info.file_system.clone(),
            kind: d.info.kind.as_str(),
            capacity: d.info.capacity,
            free: d.info.free,
            physical_disk: d.info.physical_disk,
            ready: d.info.ready,
            scanned: d.scanned,
            scanning: d.cancel.is_some(),
            tree_gen: d.tree_gen,
        })
        .collect()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanProgress {
    drive: usize,
    files: u64,
    allocated: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanFinished {
    drive: usize,
    tree_gen: u64,
    files: u64,
    dirs: u64,
    errors: u64,
    elapsed_ms: u64,
    cancelled: bool,
    error: Option<String>,
}

/// Starts scanning `drives` in the background. Drives on the same physical disk are
/// scanned one after another, different disks in parallel. A drive that is already
/// being scanned is restarted. Emits `scan-progress` and `scan-finished`.
#[tauri::command]
pub fn scan_drives(app: AppHandle, state: State<'_, AppState>, drives: Vec<usize>) -> Result<(), String> {
    let mut groups: HashMap<String, Vec<(usize, String, CancelToken)>> = HashMap::new();
    {
        let mut list = state.drives();
        for idx in drives {
            let d = list.get_mut(idx).ok_or_else(|| format!("no drive {idx}"))?;
            if !d.info.ready {
                continue;
            }
            if let Some(old) = d.cancel.take() {
                old.cancel();
            }
            let token = CancelToken::new();
            d.cancel = Some(token.clone());
            let key = match d.info.physical_disk {
                Some(n) => format!("disk{n}"),
                None => format!("drive{idx}"),
            };
            groups.entry(key).or_default().push((idx, d.info.root.clone(), token));
        }
    }
    for (_, jobs) in groups {
        let app = app.clone();
        std::thread::spawn(move || {
            for (idx, root, token) in jobs {
                scan_one(&app, idx, &root, &token);
            }
        });
    }
    Ok(())
}

fn scan_one(app: &AppHandle, idx: usize, root: &str, token: &CancelToken) {
    let mut last = Instant::now();
    let result = scan(root.as_ref(), token, |p| {
        if last.elapsed() >= PROGRESS_EVERY {
            last = Instant::now();
            let _ = app.emit("scan-progress", ScanProgress { drive: idx, files: p.files, allocated: p.allocated });
        }
    });

    let state = app.state::<AppState>();
    let mut drives = state.drives();
    let Some(d) = drives.get_mut(idx) else { return };
    // A newer scan of the same drive replaced our token: leave everything to it.
    let is_current = d.cancel.as_ref().is_some_and(|t| t.same_as(token));
    if is_current {
        d.cancel = None;
    }
    let mut event = ScanFinished {
        drive: idx,
        tree_gen: d.tree_gen,
        files: 0,
        dirs: 0,
        errors: 0,
        elapsed_ms: 0,
        cancelled: true,
        error: None,
    };
    match result {
        Ok(mut s) => {
            event.files = s.stats.files;
            event.dirs = s.stats.dirs;
            event.errors = s.stats.errors;
            event.elapsed_ms = s.stats.elapsed.as_millis() as u64;
            event.cancelled = s.stats.cancelled;
            if is_current && !s.stats.cancelled {
                complete_drive_tree(&mut s.tree, &d.info);
                d.tree = Arc::new(s.tree);
                d.scanned = true;
                d.tree_gen += 1;
                event.tree_gen = d.tree_gen;
            }
        }
        Err(e) => event.error = Some(e.to_string()),
    }
    drop(drives);
    if is_current {
        let _ = app.emit("scan-finished", event);
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum View {
    /// All `drives` side by side.
    Summary { drives: Vec<usize> },
    /// One folder of one drive.
    #[serde(rename_all = "camelCase")]
    Node { drive: usize, node: u32, tree_gen: u64 },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    drive: Option<usize>,
    node: Option<u32>,
    tree_gen: Option<u64>,
    name: String,
    path: String,
    kind: &'static str,
    /// Space taken, including free-space blocks for a drive root.
    allocated: u64,
    /// `allocated` minus free space directly below the node.
    used: u64,
    free: u64,
    logical: u64,
    files: u64,
    dirs: u64,
    flags: Vec<&'static str>,
    /// Index of the parent node, `None` at a drive root.
    parent: Option<u32>,
}

fn node_info(drive: usize, tree: &Tree, tree_gen: u64, node: NodeId) -> NodeInfo {
    let free: u64 = tree
        .children(node)
        .filter(|&c| tree.kind(c) == NodeKind::FreeSpace)
        .map(|c| tree.allocated(c))
        .sum();
    NodeInfo {
        drive: Some(drive),
        node: Some(node.index() as u32),
        tree_gen: Some(tree_gen),
        name: tree.name(node).to_owned(),
        path: display_path(&tree.path(node)).to_string_lossy().into_owned(),
        kind: tree.kind(node).as_str(),
        allocated: tree.allocated(node),
        used: tree.allocated(node) - free,
        free,
        logical: tree.logical(node),
        files: tree.files(node),
        dirs: tree.dirs(node),
        flags: tree.flags(node).names().collect(),
        parent: tree.parent(node).map(|p| p.index() as u32),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentDto {
    drive: usize,
    node: u32,
    depth: u32,
    start: f64,
    sweep: f64,
    /// Index of the parent segment, -1 in ring 1.
    parent: i64,
    kind: &'static str,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutDto {
    generation: u64,
    center_radius: f64,
    ring_width: f64,
    rings: u32,
    center: NodeInfo,
    segments: Vec<SegmentDto>,
}

fn params_for(width: f64, height: f64) -> LayoutParams {
    let radius = (width.min(height) / 2.0 - 12.0).max(80.0);
    let center_radius = (radius * 0.17).max(48.0);
    let rings = (((radius - center_radius) / RING_WIDTH).round() as u32).clamp(MIN_RINGS, MAX_RINGS);
    let ring_width = (radius - center_radius) / f64::from(rings);
    LayoutParams {
        center_radius,
        ring_width,
        // A hair over the last ring, so rounding never drops it.
        max_radius: center_radius + ring_width * f64::from(rings) + 0.5,
        min_arc: MIN_ARC_PX,
        total: None,
    }
}

/// Computes the sunburst for `view` in a `width` × `height` px canvas and remembers
/// it for [`hover`].
#[tauri::command]
pub fn layout(state: State<'_, AppState>, view: View, width: f64, height: f64) -> Result<LayoutDto, String> {
    let params = params_for(width, height);
    let none = HashSet::new();
    let (layout, parts, center) = {
        let drives = state.drives();
        match view {
            View::Node { drive, node, tree_gen } => {
                let d = drives.get(drive).ok_or("no such drive")?;
                if d.tree_gen != tree_gen {
                    return Err("stale".into());
                }
                let node = d.tree.node_id(node as usize).ok_or("no such node")?;
                let layout = SunburstLayout::compute(&d.tree, node, params, &none);
                let center = node_info(drive, &d.tree, d.tree_gen, node);
                (layout, vec![(drive, d.tree.clone(), d.tree_gen)], center)
            }
            View::Summary { drives: wanted } => {
                let parts: Vec<_> = wanted
                    .into_iter()
                    .filter_map(|i| drives.get(i).map(|d| (i, d)))
                    .filter(|(_, d)| d.info.ready && d.info.capacity > 0)
                    .map(|(i, d)| (i, d.tree.clone(), d.tree_gen))
                    .collect();
                let refs: Vec<(&Tree, NodeId)> = parts.iter().map(|(_, t, _)| (t.as_ref(), NodeId::ROOT)).collect();
                let layout = SunburstLayout::compute_summary(&refs, params, &HashSet::new());
                let center = summary_info(&parts);
                (layout, parts, center)
            }
        }
    };

    let segments = layout
        .segments()
        .iter()
        .map(|s| {
            let (drive, tree, _) = &parts[s.part as usize];
            SegmentDto {
                drive: *drive,
                node: s.node.index() as u32,
                depth: s.depth,
                start: s.start,
                sweep: s.sweep,
                parent: s.parent.map_or(-1, |p| p as i64),
                kind: tree.kind(s.node).as_str(),
                name: tree.name(s.node).to_owned(),
            }
        })
        .collect();
    let generation = state.next_generation();
    let dto = LayoutDto {
        generation,
        center_radius: params.center_radius,
        ring_width: params.ring_width,
        rings: layout.ring_count(),
        center,
        segments,
    };
    *state.current() = Some(Current { generation, layout, parts });
    Ok(dto)
}

fn summary_info(parts: &[(usize, Arc<Tree>, u64)]) -> NodeInfo {
    let mut info = NodeInfo {
        drive: None,
        node: None,
        tree_gen: None,
        name: String::new(),
        path: String::new(),
        kind: "summary",
        allocated: 0,
        used: 0,
        free: 0,
        logical: 0,
        files: 0,
        dirs: 0,
        flags: Vec::new(),
        parent: None,
    };
    for (drive, tree, tree_gen) in parts {
        let d = node_info(*drive, tree, *tree_gen, NodeId::ROOT);
        info.allocated += d.allocated;
        info.used += d.used;
        info.free += d.free;
        info.logical += d.logical;
        info.files += d.files;
        info.dirs += d.dirs;
    }
    info
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HoverDto {
    /// `none`, `center`, `segment`.
    hit: &'static str,
    segment: Option<usize>,
    info: Option<NodeInfo>,
}

/// What lies under `(x, y)` (px from the chart centre) in layout `generation`.
#[tauri::command]
pub fn hover(state: State<'_, AppState>, generation: u64, x: f64, y: f64) -> Result<HoverDto, String> {
    let current = state.current();
    let cur = current.as_ref().filter(|c| c.generation == generation).ok_or("stale")?;
    Ok(match cur.layout.hit(x, y) {
        Hit::Segment(i) => {
            let s = cur.layout.segments()[i];
            let (drive, tree, tree_gen) = &cur.parts[s.part as usize];
            HoverDto { hit: "segment", segment: Some(i), info: Some(node_info(*drive, tree, *tree_gen, s.node)) }
        }
        Hit::Center => HoverDto { hit: "center", segment: None, info: None },
        Hit::Free | Hit::None => HoverDto { hit: "none", segment: None, info: None },
    })
}

#[cfg(test)]
mod tests {
    use super::params_for;

    #[test]
    fn rings_fill_the_canvas() {
        for (w, h) in [(400.0, 300.0), (1000.0, 800.0), (3000.0, 2000.0), (50.0, 50.0)] {
            let p = params_for(w, h);
            let rings = ((p.max_radius - p.center_radius) / p.ring_width).floor() as u32;
            assert!((3..=8).contains(&rings), "{w}x{h}: {rings} rings");
            assert!(p.max_radius <= (w.min(h) / 2.0).max(80.0 + 12.0) + 1.0, "{w}x{h} overflows");
        }
    }
}
