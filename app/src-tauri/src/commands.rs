//! Commands and events exchanged with the UI.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use scanner_core::filetype::{Category, TypeStats};
use scanner_core::search::{Query, find_marked};
use scanner_core::{
    CancelToken, Hit, LayoutParams, NodeId, NodeKind, ScanOptions, SunburstLayout, Tree, display_path, is_elevated,
    scan_mft, scan_with,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::{AppState, Comparison, Current, Drive, NodeRef, refresh_space_blocks};

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
    /// `false` once the drive letter is gone (e.g. a USB stick was pulled out).
    /// Indices stay stable, so the slot is kept.
    present: bool,
    ready: bool,
    scanned: bool,
    scanning: bool,
    tree_gen: u64,
    /// How the last full scan read the drive: `mft` or `walk`.
    scan_method: Option<&'static str>,
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
            present: d.present,
            ready: d.present && d.info.ready,
            scanned: d.scanned,
            scanning: d.cancel.is_some(),
            tree_gen: d.tree_gen,
            scan_method: d.scan_method,
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
    /// `mft` (fast mode, read from the NTFS master file table) or `walk`.
    method: &'static str,
}

/// Starts scanning `drives` in the background. Drives on the same physical disk are
/// scanned one after another, different disks in parallel. A drive that is already
/// being scanned is restarted. Emits `scan-progress` and `scan-finished`.
#[tauri::command]
pub fn scan_drives(app: AppHandle, state: State<'_, AppState>, drives: Vec<usize>) -> Result<(), String> {
    let elevated = is_elevated();
    let mut groups: HashMap<String, Vec<ScanJob>> = HashMap::new();
    {
        let mut list = state.drives();
        for idx in drives {
            let d = list.get_mut(idx).ok_or_else(|| format!("no drive {idx}"))?;
            if !d.present || !d.info.ready {
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
            let options = ScanOptions { threads: d.info.scan_threads() };
            let use_mft = elevated && d.info.file_system.eq_ignore_ascii_case("NTFS");
            groups.entry(key).or_default().push(ScanJob { idx, root: d.info.root.clone(), use_mft, options, token });
        }
    }
    for (_, jobs) in groups {
        let app = app.clone();
        std::thread::spawn(move || {
            for job in jobs {
                scan_one(&app, job);
            }
        });
    }
    Ok(())
}

struct ScanJob {
    idx: usize,
    root: String,
    /// Read the NTFS master file table instead of listing folders (fast mode).
    use_mft: bool,
    options: ScanOptions,
    token: CancelToken,
}

fn scan_one(app: &AppHandle, job: ScanJob) {
    let ScanJob { idx, root, use_mft, options, token } = job;
    let (root, token) = (root.as_str(), &token);
    let mut last = Instant::now();
    let mut report = |p: &scanner_core::Progress| {
        if last.elapsed() >= PROGRESS_EVERY {
            last = Instant::now();
            let _ = app.emit("scan-progress", ScanProgress { drive: idx, files: p.files, allocated: p.allocated });
        }
    };
    // In fast mode NTFS drives are read through the MFT; anything unexpected
    // there falls back to listing folders.
    let mut method = "walk";
    let mut result = Err(std::io::Error::other("not tried"));
    if use_mft {
        result = scan_mft(root.as_ref(), token, &mut report);
        if result.is_ok() {
            method = "mft";
        }
    }
    if result.is_err() {
        result = scan_with(root.as_ref(), &options, token, &mut report);
    }

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
        method,
    };
    match result {
        Ok(mut s) => {
            event.files = s.stats.files;
            event.dirs = s.stats.dirs;
            event.errors = s.stats.errors;
            event.elapsed_ms = s.stats.elapsed.as_millis() as u64;
            event.cancelled = s.stats.cancelled;
            if is_current && !s.stats.cancelled {
                if let Some(info) = scanner_core::drive_info(&d.info.root) {
                    d.info = info;
                }
                refresh_space_blocks(&mut s.tree, &d.info);
                d.tree = Arc::new(s.tree);
                d.scanned = true;
                d.scan_method = Some(method);
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
    /// One folder of one drive. If the tree changed since, the folder is found by
    /// path, falling back to its deepest ancestor that still exists.
    Node(NodeRef),
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
    /// With a search query, one mark per segment: 0 no match, 1 contains a match,
    /// 2 matches itself.
    marks: Option<Vec<u8>>,
    /// When colouring by file type, one [`Category`] number per segment.
    categories: Option<Vec<u8>>,
    /// When colouring by change: per segment the growth in bytes since the
    /// compared snapshot...
    changes: Option<Vec<i64>>,
    /// ...and whether it was there before (`0`), is new (`1`), or lies on a
    /// drive that is not compared with a snapshot (`2`).
    change_kinds: Option<Vec<u8>>,
}

/// How the chart is coloured.
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    /// By branch of the shown folder.
    Folder,
    /// By file type.
    Type,
    /// By growth since a snapshot.
    Change,
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

/// One drive's share of a view.
pub(crate) struct Part {
    drive: usize,
    pub(crate) tree: Arc<Tree>,
    tree_gen: u64,
    /// The node the view is centred on in this tree (the root in a summary).
    pub(crate) root: NodeId,
    comparison: Option<Arc<Comparison>>,
}

/// The drive trees a view shows. A node view has one part; a summary has one
/// per drive that is present and has a capacity.
pub(crate) fn scope(drives: &[Drive], view: &View) -> Result<Vec<Part>, String> {
    match view {
        View::Node(r) => {
            let d = drives.get(r.drive).ok_or("no such drive")?;
            let root = d.resolve(r, false).unwrap_or(NodeId::ROOT);
            Ok(vec![Part {
                drive: r.drive,
                tree: d.tree.clone(),
                tree_gen: d.tree_gen,
                root,
                comparison: d.comparison.clone(),
            }])
        }
        View::Summary { drives: wanted } => Ok(wanted
            .iter()
            .filter_map(|&i| drives.get(i).map(|d| (i, d)))
            .filter(|(_, d)| d.present && d.info.ready && d.info.capacity > 0)
            .map(|(i, d)| Part {
                drive: i,
                tree: d.tree.clone(),
                tree_gen: d.tree_gen,
                root: NodeId::ROOT,
                comparison: d.comparison.clone(),
            })
            .collect()),
    }
}

/// Computes the sunburst for `view` in a `width` × `height` px canvas and remembers
/// it for [`hover`]. With a `query`, each segment gets a search mark.
#[tauri::command]
pub fn layout(
    state: State<'_, AppState>,
    view: View,
    width: f64,
    height: f64,
    // Folders whose contents are hidden (Show/Hide).
    collapsed: Vec<NodeRef>,
    query: Option<String>,
    color: ColorMode,
) -> Result<LayoutDto, String> {
    let params = params_for(width, height);
    let summary = matches!(view, View::Summary { .. });
    let (parts, hidden) = {
        let drives = state.drives();
        let parts = scope(&drives, &view)?;
        let hidden: HashSet<(u32, NodeId)> = collapsed
            .iter()
            .filter_map(|c| {
                let part = parts.iter().position(|p| p.drive == c.drive)?;
                Some((part as u32, drives[c.drive].resolve(c, true)?))
            })
            .collect();
        (parts, hidden)
    };

    let layout = if summary {
        let refs: Vec<(&Tree, NodeId)> = parts.iter().map(|p| (p.tree.as_ref(), p.root)).collect();
        SunburstLayout::compute_summary(&refs, params, &hidden)
    } else {
        let p = &parts[0];
        let hidden: HashSet<NodeId> = hidden.into_iter().map(|(_, n)| n).collect();
        SunburstLayout::compute(&p.tree, p.root, params, &hidden)
    };
    let center = if summary {
        summary_info(&parts)
    } else {
        let p = &parts[0];
        node_info(p.drive, &p.tree, p.tree_gen, p.root)
    };

    let marks = query.as_deref().and_then(Query::parse).map(|q| {
        let per_part: Vec<_> = parts.iter().map(|p| state.search_marks(p.drive, &p.tree, p.tree_gen, &q)).collect();
        layout.segments().iter().map(|s| per_part[s.part as usize][s.node.index()]).collect()
    });

    let categories = (color == ColorMode::Type).then(|| {
        let per_part: Vec<_> = parts.iter().map(|p| state.categories(p.drive, &p.tree, p.tree_gen)).collect();
        layout.segments().iter().map(|s| per_part[s.part as usize][s.node.index()] as u8).collect()
    });

    let (changes, change_kinds) = if color == ColorMode::Change {
        let diffs: Vec<_> = parts.iter().map(|p| p.comparison.as_ref().map(|c| c.diff(&p.tree, p.tree_gen))).collect();
        let (deltas, kinds): (Vec<i64>, Vec<u8>) = layout
            .segments()
            .iter()
            .map(|s| {
                let p = &parts[s.part as usize];
                match (&diffs[s.part as usize], &p.comparison) {
                    (Some(d), Some(c)) => (d.delta(&c.old, &p.tree, s.node), u8::from(d.is_added(s.node))),
                    _ => (0, 2),
                }
            })
            .unzip();
        (Some(deltas), Some(kinds))
    } else {
        (None, None)
    };

    let segments = layout
        .segments()
        .iter()
        .map(|s| {
            let p = &parts[s.part as usize];
            SegmentDto {
                drive: p.drive,
                node: s.node.index() as u32,
                depth: s.depth,
                start: s.start,
                sweep: s.sweep,
                parent: s.parent.map_or(-1, |p| p as i64),
                kind: p.tree.kind(s.node).as_str(),
                name: p.tree.name(s.node).to_owned(),
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
        marks,
        categories,
        changes,
        change_kinds,
    };
    let parts = parts.into_iter().map(|p| (p.drive, p.tree, p.tree_gen)).collect();
    *state.current() = Some(Current { generation, layout, parts });
    Ok(dto)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildrenDto {
    items: Vec<NodeInfo>,
    /// Number of children, including those beyond `limit`.
    total: usize,
    /// Space taken by the children beyond `limit`.
    rest_allocated: u64,
}

/// Contents of the view's centre, largest first: the folder's children, or the
/// drives of a summary. Unlike the chart, this lists entries too small to draw.
#[tauri::command]
pub fn children(state: State<'_, AppState>, view: View, limit: usize) -> Result<ChildrenDto, String> {
    let parts = scope(&state.drives(), &view)?;
    if matches!(view, View::Summary { .. }) {
        let items: Vec<NodeInfo> = parts.iter().map(|p| node_info(p.drive, &p.tree, p.tree_gen, p.root)).collect();
        return Ok(ChildrenDto { total: items.len(), items, rest_allocated: 0 });
    }
    let p = &parts[0];
    let kids = p.tree.children_by_size(p.root);
    Ok(ChildrenDto {
        total: kids.len(),
        rest_allocated: kids.iter().skip(limit).map(|&c| p.tree.allocated(c)).sum(),
        items: kids.iter().take(limit).map(|&c| node_info(p.drive, &p.tree, p.tree_gen, c)).collect(),
    })
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchDto {
    /// Number of matches in the view.
    total: usize,
    /// Space they take, counting a match inside a matching folder once.
    allocated: u64,
    /// The largest `limit` matches, largest first.
    items: Vec<NodeInfo>,
}

/// Finds names containing `query`, or matching it as a mask with `*` and `?`,
/// everywhere below the view's centre.
#[tauri::command]
pub async fn search(app: AppHandle, view: View, query: String, limit: usize) -> Result<SearchDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(q) = Query::parse(&query) else { return Ok(SearchDto::default()) };
        let state = app.state::<AppState>();
        let parts = scope(&state.drives(), &view)?;
        let mut result = SearchDto::default();
        let mut hits: Vec<(usize, NodeId, u64)> = Vec::new();
        for (i, p) in parts.iter().enumerate() {
            let marks = state.search_marks(p.drive, &p.tree, p.tree_gen, &q);
            let found = find_marked(&p.tree, p.root, &marks);
            result.total += found.nodes.len();
            result.allocated += found.allocated;
            hits.extend(found.nodes.into_iter().map(|n| (i, n, p.tree.allocated(n))));
        }
        if hits.len() > limit && limit > 0 {
            hits.select_nth_unstable_by(limit - 1, |a, b| b.2.cmp(&a.2));
            hits.truncate(limit);
        }
        hits.sort_by_key(|h| std::cmp::Reverse(h.2));
        result.items = hits
            .into_iter()
            .take(limit)
            .map(|(i, n, _)| {
                let p = &parts[i];
                node_info(p.drive, &p.tree, p.tree_gen, n)
            })
            .collect();
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryDto {
    category: u8,
    files: u64,
    allocated: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionDto {
    /// Lower-case, without the dot; empty for files without one.
    extension: String,
    category: u8,
    files: u64,
    allocated: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTypesDto {
    /// Categories that have files, largest first.
    categories: Vec<CategoryDto>,
    /// The largest `limit` extensions.
    extensions: Vec<ExtensionDto>,
    /// Number of distinct extensions, including those beyond `limit`.
    total: usize,
    /// Space taken by the extensions beyond `limit`.
    rest_allocated: u64,
}

/// Space per file type and per extension below the view's centre.
#[tauri::command]
pub async fn file_types(app: AppHandle, view: View, limit: usize) -> Result<FileTypesDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let parts = scope(&app.state::<AppState>().drives(), &view)?;
        let mut stats = TypeStats::default();
        for p in &parts {
            stats.add(&p.tree, p.root);
        }
        let mut categories: Vec<CategoryDto> = Category::ALL
            .iter()
            .zip(stats.categories())
            .filter(|(_, (files, _))| *files > 0)
            .map(|(&c, (files, allocated))| CategoryDto { category: c as u8, files, allocated })
            .collect();
        categories.sort_by_key(|c| std::cmp::Reverse(c.allocated));
        let all = stats.extensions();
        Ok(FileTypesDto {
            categories,
            total: all.len(),
            rest_allocated: all.iter().skip(limit).map(|e| e.allocated).sum(),
            extensions: all
                .into_iter()
                .take(limit)
                .map(|e| ExtensionDto {
                    extension: e.extension,
                    category: e.category as u8,
                    files: e.files,
                    allocated: e.allocated,
                })
                .collect(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedNode {
    info: NodeInfo,
    delta: i64,
    added: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovedNode {
    name: String,
    path: String,
    kind: &'static str,
    allocated: u64,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChangesDto {
    /// Id of the snapshot the view's drive is compared with.
    snapshot: Option<String>,
    /// When it was saved, Unix seconds.
    since: Option<u64>,
    /// Growth of the view's centre.
    delta: i64,
    /// Entries of the centre that changed, by size of the change.
    children: Vec<ChangedNode>,
    /// Files below the centre that are new or grew, largest growth first.
    files: Vec<ChangedNode>,
    /// What is gone below the centre, largest first.
    removed: Vec<RemovedNode>,
    removed_total: usize,
    removed_allocated: u64,
}

/// What changed below the view's centre since the snapshot its drive is
/// compared with. Empty for a summary or a drive without a comparison.
#[tauri::command]
pub async fn changes(app: AppHandle, view: View, limit: usize) -> Result<ChangesDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if matches!(view, View::Summary { .. }) {
            return Ok(ChangesDto::default());
        }
        let parts = scope(&app.state::<AppState>().drives(), &view)?;
        let p = &parts[0];
        let Some(c) = &p.comparison else { return Ok(ChangesDto::default()) };
        let diff = c.diff(&p.tree, p.tree_gen);
        let (old, tree) = (c.old.as_ref(), p.tree.as_ref());
        let changed = |n: NodeId| ChangedNode {
            info: node_info(p.drive, tree, p.tree_gen, n),
            delta: diff.delta(old, tree, n),
            added: diff.is_added(n),
        };

        let mut kids: Vec<(NodeId, i64)> = tree
            .children(p.root)
            .map(|n| (n, diff.delta(old, tree, n)))
            .filter(|&(n, d)| d != 0 || diff.is_added(n))
            .collect();
        kids.sort_by_key(|&(_, d)| std::cmp::Reverse(d.unsigned_abs()));
        let removed = diff.old_node(p.root).map(|r| diff.removed(old, Some(r))).unwrap_or_default();
        Ok(ChangesDto {
            snapshot: Some(c.id.clone()),
            since: Some(c.created),
            delta: diff.delta(old, tree, p.root),
            children: kids.into_iter().take(limit).map(|(n, _)| changed(n)).collect(),
            files: diff.grown_files(old, tree, p.root, limit).into_iter().map(|(n, _)| changed(n)).collect(),
            removed_total: removed.len(),
            removed_allocated: removed.iter().map(|&n| old.allocated(n)).sum(),
            removed: removed
                .into_iter()
                .take(limit)
                .map(|n| RemovedNode {
                    name: old.name(n).to_owned(),
                    path: display_path(&old.path(n)).to_string_lossy().into_owned(),
                    kind: old.kind(n).as_str(),
                    allocated: old.allocated(n),
                })
                .collect(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

fn summary_info(parts: &[Part]) -> NodeInfo {
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
    for p in parts {
        let d = node_info(p.drive, &p.tree, p.tree_gen, p.root);
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
    /// Growth since the snapshot the drive is compared with.
    change: Option<ChangeDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeDto {
    delta: i64,
    /// Not there in the snapshot.
    added: bool,
    /// When the snapshot was saved, Unix seconds.
    since: u64,
}

/// What lies under `(x, y)` (px from the chart centre) in layout `generation`.
#[tauri::command]
pub fn hover(state: State<'_, AppState>, generation: u64, x: f64, y: f64) -> Result<HoverDto, String> {
    let (index, node, drive, tree, tree_gen) = {
        let current = state.current();
        let cur = current.as_ref().filter(|c| c.generation == generation).ok_or("stale")?;
        let hit = match cur.layout.hit(x, y) {
            Hit::Segment(i) => i,
            Hit::Center => return Ok(HoverDto { hit: "center", segment: None, info: None, change: None }),
            Hit::Free | Hit::None => return Ok(HoverDto { hit: "none", segment: None, info: None, change: None }),
        };
        let s = cur.layout.segments()[hit];
        let (drive, tree, tree_gen) = &cur.parts[s.part as usize];
        (hit, s.node, *drive, tree.clone(), *tree_gen)
    };
    let comparison = state.drives().get(drive).and_then(|d| d.comparison.clone());
    let change = comparison.map(|c| {
        let diff = c.diff(&tree, tree_gen);
        ChangeDto { delta: diff.delta(&c.old, &tree, node), added: diff.is_added(node), since: c.created }
    });
    Ok(HoverDto { hit: "segment", segment: Some(index), info: Some(node_info(drive, &tree, tree_gen, node)), change })
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
