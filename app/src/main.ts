import "./style.css";
import {
  api,
  refOf,
  type Change,
  type ColorMode,
  type Drive,
  type Duplicates,
  type DupProgress,
  type ExportFormat,
  type HoverResult,
  type Layout,
  type NodeInfo,
  type NodeRef,
  type View,
} from "./api";
import { CHANGE_GREW, CHANGE_SHRANK, categoryColor, changeColor } from "./colors";
import { formatDelta, formatPercent, formatShortDate, formatSize } from "./format";
import { Panel, type PanelTab } from "./panel";
import { S, nodeTitle } from "./strings";
import { SunburstRenderer } from "./sunburst";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const els = {
  drives: $<HTMLDivElement>("drives"),
  summary: $<HTMLButtonElement>("summary-btn"),
  back: $<HTMLButtonElement>("back"),
  up: $<HTMLButtonElement>("up"),
  rescan: $<HTMLButtonElement>("rescan"),
  bin: $<HTMLButtonElement>("bin"),
  apps: $<HTMLButtonElement>("apps"),
  fast: $<HTMLButtonElement>("fast"),
  more: $<HTMLButtonElement>("more"),
  panelToggle: $<HTMLButtonElement>("panel-toggle"),
  colorMode: $<HTMLButtonElement>("color-mode"),
  legend: $<HTMLDivElement>("legend"),
  panel: $<HTMLElement>("panel"),
  search: $<HTMLInputElement>("search"),
  searchClear: $<HTMLButtonElement>("search-clear"),
  crumb: $<HTMLDivElement>("crumb"),
  chart: $<HTMLDivElement>("chart"),
  canvas: $<HTMLCanvasElement>("canvas"),
  tooltip: $<HTMLDivElement>("tooltip"),
  empty: $<HTMLDivElement>("empty"),
  status: $<HTMLDivElement>("status"),
  menu: $<HTMLDivElement>("menu"),
  modal: $<HTMLDivElement>("modal"),
  modalTitle: $<HTMLHeadingElement>("modal-title"),
  modalBody: $<HTMLDivElement>("modal-body"),
  modalOk: $<HTMLButtonElement>("modal-ok"),
  modalCancel: $<HTMLButtonElement>("modal-cancel"),
  toast: $<HTMLDivElement>("toast"),
};

const ICONS = {
  back: '<svg viewBox="0 0 24 24"><path d="M15 5l-7 7 7 7"/></svg>',
  up: '<svg viewBox="0 0 24 24"><path d="M5 15l7-7 7 7"/></svg>',
  rescan: '<svg viewBox="0 0 24 24"><path d="M20 12a8 8 0 1 1-2.3-5.6M20 4v5h-5"/></svg>',
  bin: '<svg viewBox="0 0 24 24"><path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 11v6M14 11v6"/></svg>',
  apps: '<svg viewBox="0 0 24 24"><rect x="4" y="4" width="7" height="7" rx="1.5"/><rect x="13" y="4" width="7" height="7" rx="1.5"/><rect x="4" y="13" width="7" height="7" rx="1.5"/><path d="M16.5 13.5v6M13.5 16.5h6"/></svg>',
  more: '<svg viewBox="0 0 24 24"><circle cx="5" cy="12" r="1.3"/><circle cx="12" cy="12" r="1.3"/><circle cx="19" cy="12" r="1.3"/></svg>',
  panel: '<svg viewBox="0 0 24 24"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M15 4v16"/></svg>',
  palette: '<svg viewBox="0 0 24 24"><path d="M12 3a9 9 0 1 0 0 18c1.1 0 1.6-.8 1.6-1.6 0-.9-.7-1.3-.7-2.2 0-.9.7-1.6 1.6-1.6H17a4 4 0 0 0 4-4c0-4.7-4-8.6-9-8.6z"/><circle cx="7.5" cy="11" r="1.2"/><circle cx="10.5" cy="7" r="1.2"/><circle cx="15" cy="7.5" r="1.2"/></svg>',
  fast: '<svg viewBox="0 0 24 24"><path d="M13 2L4 14h7l-1 8 9-12h-7z"/></svg>',
  summary: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="8"/><path d="M12 4v8l6 5"/></svg>',
  fixed: '<svg viewBox="0 0 24 24"><rect x="3" y="6" width="18" height="12" rx="2"/><circle cx="17" cy="12" r="1.2"/></svg>',
  removable: '<svg viewBox="0 0 24 24"><path d="M8 3h8v6l2 2v10H6V11l2-2z"/><path d="M10 6h1M13 6h1"/></svg>',
  network: '<svg viewBox="0 0 24 24"><rect x="4" y="4" width="16" height="9" rx="2"/><path d="M12 13v4M5 20h14M12 17v3"/></svg>',
  cdrom: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="8"/><circle cx="12" cy="12" r="2"/></svg>',
};

const SUMMARY_KEY = "summaryDrives";
const PANEL_KEY = "panelHidden";
const COLOR_KEY = "colorMode";
const TAB_KEY = "panelTab";
const EXPORT_KEY = "exportOptions";
const DUP_SIZE_KEY = "dupMinSize";
const LIST_LIMIT = 300;
const SEARCH_DELAY_MS = 250;
const renderer = new SunburstRenderer(els.canvas);
const panel = new Panel(els.panel, {
  activate: (info, fromSearch) => {
    if (info.kind === "dir") zoomTo(info);
    else if (fromSearch) showContaining(info);
  },
  open: (info) => {
    const ref = refOf(info);
    if (ref) run(api.openNode(ref), S.open);
  },
  menu: (info, x, y) => openMenu(x, y, nodeMenu(info, false)),
  hover: (info) => {
    const seg = info && info.drive !== null && info.node !== null ? renderer.segmentOf(info.drive, info.node) : null;
    renderer.setHighlight(seg === null ? null : { kind: "segment", index: seg });
    setStatus(info ?? layout?.center ?? null);
  },
  colorOf: (info) => (info.drive !== null && info.node !== null ? renderer.colorOf(info.drive, info.node) : null),
  title: (info) => {
    const drive = info.drive !== null ? drives[info.drive] : undefined;
    const title = displayName(info);
    return info.parent === null && drive?.label ? `${title} ${drive.label}` : title;
  },
  categoryColor: (category) => categoryColor(category, isDark()),
  tab: (tab) => {
    panelTab = tab;
    store(TAB_KEY, tab);
    void loadPanel();
  },
  findExtension: (ext) => {
    els.search.value = `*.${ext}`;
    els.searchClear.hidden = false;
    applySearch();
  },
  changeColor: (delta, added, scale) => changeColor(delta, added ? 1 : 0, scale, isDark()),
  compare: (id) => void compareWith(id),
  saveSnapshot: () => void saveSnapshot(),
  deleteSnapshot: (id) => void deleteSnapshot(id),
  findDupes: (minSize) => void findDupes(minSize),
  cancelDupes: () => void api.cancelDupes(),
});

/** The duplicate search: its last result, and progress while it runs. */
const dupes = {
  running: false,
  progress: null as DupProgress | null,
  result: null as Duplicates | null,
  resultScope: "",
  minSize: 1024 * 1024,
};

let colorMode: ColorMode = "folder";
let panelTab: PanelTab = "contents";

/** Current search text, trimmed; empty when not searching. */
let query = "";
let panelSeq = 0;

let drives: Drive[] = [];
/** Drive roots shown in the summary. */
let summaryRoots = new Set<string>();
const progress = new Map<number, { files: number; allocated: number }>();
const scanErrors = new Map<number, string>();
/** Folders scanned right now, by path, for the status line. */
const folderScans = new Set<string>();
/** Folders whose contents are hidden (Show/Hide), keyed by drive and lower-case path. */
const collapsed = new Map<string, NodeRef>();

let view: View = { kind: "summary", drives: [] };
let layout: Layout | null = null;
const history: View[] = [];
/** Only the latest layout request may update the screen. */
let layoutSeq = 0;
let hoverSeq = 0;
let lastHover: HoverResult | null = null;

// ---------- Navigation ----------

function summaryView(): View {
  return { kind: "summary", drives: drives.filter((d) => summaryRoots.has(d.root)).map((d) => d.index) };
}

function driveRootView(d: Drive): View {
  return { kind: "node", drive: d.index, node: 0, treeGen: d.treeGen, path: d.root };
}

function sameView(a: View, b: View): boolean {
  if (a.kind === "summary" || b.kind === "summary") return a.kind === b.kind;
  return a.drive === b.drive && a.path.toLowerCase() === b.path.toLowerCase();
}

/** Re-reads a summary's drive list; node views are resolved by the backend. */
function refreshed(v: View): View {
  return v.kind === "summary" ? summaryView() : v;
}

/** Shows `next`; with `push`, the current view goes onto the Back history. */
async function show(next: View, push = true): Promise<void> {
  const seq = ++layoutSeq;
  const { width, height } = chartSize();
  let result: Layout;
  try {
    result = await api.layout(next, width, height, [...collapsed.values()], query || null, colorMode);
  } catch (err) {
    console.error("layout failed", err);
    return;
  }
  if (seq !== layoutSeq) return;
  // The backend may have resolved the folder by path, or fallen back to an
  // ancestor if it was deleted: keep the view in step with what is shown.
  const c = result.center;
  const shown: View = next.kind === "node" ? { kind: "node", ...(refOf(c) ?? next) } : next;
  if (push && !sameView(view, shown)) history.push(view);
  view = shown;
  layout = result;
  renderer.setLayout(result, shown.kind === "summary", drives);
  renderLegend();
  lastHover = null;
  hideTooltip();
  updateChrome();
  void loadPanel();
}

/** Fills the side panel: search results while searching, else the folder's contents. */
async function loadPanel() {
  if (document.body.classList.contains("panel-hidden") || !layout) return;
  const seq = ++panelSeq;
  const whole = layout.center.allocated;
  try {
    panel.setTab(query ? null : panelTab);
    if (query) {
      panel.showMessage(S.searching);
      const r = await api.search(view, query, LIST_LIMIT);
      if (seq !== panelSeq) return;
      const scope = view.kind === "summary" ? S.summary : layout.center.path;
      panel.showSearch(r.total, r.allocated, r.items, scope, whole);
    } else if (panelTab === "dupes") {
      showDupes();
    } else if (panelTab === "changes") {
      const drive = currentDrive();
      const d = drive !== null ? drives[drive] : undefined;
      if (!d) {
        panel.showMessage(S.changesNeedDrive);
        return;
      }
      const [snapshots, changes] = await Promise.all([api.listSnapshots(d.root), api.changes(view, LIST_LIMIT)]);
      if (seq !== panelSeq) return;
      panel.showChanges({ snapshots, changes, canSave: d.scanned && !d.scanning });
    } else if (panelTab === "types") {
      const t = await api.fileTypes(view, LIST_LIMIT);
      if (seq !== panelSeq) return;
      panel.showTypes(t, whole);
    } else {
      const c = await api.children(view, LIST_LIMIT);
      if (seq !== panelSeq) return;
      panel.showChildren(c.items, c.total, c.restAllocated, whole, view.kind === "summary");
    }
  } catch (e) {
    if (seq === panelSeq) panel.showMessage(S.failed(S.searching, e));
  }
}

function isDark(): boolean {
  return matchMedia("(prefers-color-scheme: dark)").matches;
}

/** Saves a UI preference; storage may be unavailable, which is not critical. */
function store(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Not critical.
  }
}

function load(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function legendItem(color: string, label: string): HTMLElement {
  const item = document.createElement("span");
  const swatch = document.createElement("i");
  swatch.style.background = color;
  item.append(swatch, label);
  return item;
}

/** Colour key: grew / shrank when comparing, else the file types on the chart. */
function renderLegend() {
  const kinds = colorMode === "change" ? layout?.changeKinds : null;
  if (kinds && layout) {
    const compared = kinds.some((k) => k !== 2);
    const dark = isDark();
    els.legend.replaceChildren(
      ...(compared
        ? [
            legendItem(`hsl(${CHANGE_GREW} 70% ${dark ? 50 : 56}%)`, S.grew),
            legendItem(`hsl(${CHANGE_SHRANK} 70% ${dark ? 50 : 56}%)`, S.shrank),
            legendItem(changeColor(0, 0, 1, dark), S.unchanged),
          ]
        : [legendItem(changeColor(0, 2, 1, dark), S.notCompared)]),
    );
    els.legend.hidden = false;
    return;
  }
  const cats = colorMode === "type" ? layout?.categories : null;
  if (!cats || !layout) {
    els.legend.hidden = true;
    return;
  }
  const branchDepth = view.kind === "summary" ? 2 : 1;
  const present = new Set<number>();
  layout.segments.forEach((s, i) => {
    if ((s.kind === "dir" || s.kind === "file") && s.depth >= branchDepth) present.add(cats[i]);
  });
  els.legend.replaceChildren(
    ...[...present]
      .sort((a, b) => (a === 0 ? 1 : b === 0 ? -1 : a - b))
      .map((c) => legendItem(categoryColor(c, isDark()), S.categories[c] ?? S.categories[0])),
  );
  els.legend.hidden = present.size === 0;
}

function setColorMode(mode: ColorMode) {
  colorMode = mode;
  els.colorMode.classList.toggle("on", mode !== "folder");
  els.colorMode.title = S.colorMode;
  store(COLOR_KEY, mode);
}

function openColorMenu() {
  const r = els.colorMode.getBoundingClientRect();
  const item = (mode: ColorMode, label: string) => ({
    label,
    checked: colorMode === mode,
    action: () => {
      setColorMode(mode);
      void show(refreshed(view), false);
    },
  });
  openMenu(r.right, r.bottom + 4, [
    item("folder", S.colorFolder),
    item("type", S.colorType),
    item("change", S.colorChange),
  ], true);
}

interface ExportOptions {
  format: ExportFormat;
  /** `0` for all levels. */
  depth: number;
  minSize: number;
}

function selectRow(label: string, options: [string, string][], value: string): [HTMLElement, HTMLSelectElement] {
  const row = document.createElement("label");
  row.className = "form-row";
  row.textContent = label;
  const select = document.createElement("select");
  for (const [v, text] of options) select.append(new Option(text, v));
  select.value = value;
  if (select.selectedIndex < 0) select.selectedIndex = 0;
  row.append(select);
  return [row, select];
}

/** A file name for an export of `info`: `Диск C_2026-10-24.csv`, `Users_2026-10-24.json`. */
function exportFileName(info: NodeInfo | null, format: ExportFormat): string {
  const d = new Date();
  const date = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  let name = !info || info.kind === "summary" ? S.summary : info.name;
  if (info && info.parent === null && info.kind !== "summary") name = `Диск ${info.name.replace(/[:\\]/g, "")}`;
  return `${name.replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_")}_${date}.${format}`;
}

/** Asks for format, depth and size threshold, then for a file, and exports `target`. */
async function exportView(target: View, info: NodeInfo | null) {
  let saved: ExportOptions = { format: "csv", depth: 0, minSize: 0 };
  try {
    saved = { ...saved, ...JSON.parse(load(EXPORT_KEY) ?? "{}") };
  } catch {
    // Defaults.
  }
  const MB = 1024 * 1024;
  const [formatRow, format] = selectRow(S.exportFormat, [["csv", S.exportCsv], ["json", S.exportJson]], saved.format);
  const [depthRow, depth] = selectRow(
    S.exportDepth,
    [["0", S.exportDepthAll], ...[1, 2, 3, 5].map((n): [string, string] => [String(n), S.exportDepthN(n)])],
    String(saved.depth),
  );
  const [sizeRow, size] = selectRow(
    S.exportMinSize,
    [["0", S.exportMinNone], ...[1, 10, 100, 1024].map((mb): [string, string] => [String(mb * MB), formatSize(mb * MB)])],
    String(saved.minSize),
  );
  const what = info ? displayName(info) : S.summary;
  const ok = await confirmDialog({
    title: S.exportTitle(what),
    body: [S.exportNote],
    ok: S.exportSave,
    content: [formatRow, depthRow, sizeRow],
  });
  if (!ok) return;
  const options: ExportOptions = {
    format: format.value === "json" ? "json" : "csv",
    depth: Number(depth.value) || 0,
    minSize: Number(size.value) || 0,
  };
  store(EXPORT_KEY, JSON.stringify(options));
  try {
    const path = await api.exportView(
      target,
      options.format,
      options.depth || null,
      options.minSize,
      exportFileName(info, options.format),
    );
    if (path) toast(S.exported(path));
  } catch (e) {
    toast(S.failed(S.exportFolder, e), true);
  }
}

/** Where the shown view is, for labels: a folder path, a drive, or the summary. */
function viewLabel(): string {
  return view.kind === "summary" || !layout ? S.summary : layout.center.path;
}

function showDupes() {
  if (panelTab !== "dupes" || query || document.body.classList.contains("panel-hidden")) return;
  panel.showDupes({ ...dupes, here: viewLabel() });
}

async function findDupes(minSize: number) {
  dupes.minSize = minSize;
  store(DUP_SIZE_KEY, String(minSize));
  dupes.running = true;
  dupes.progress = null;
  const scope = viewLabel();
  showDupes();
  try {
    dupes.result = await api.findDupes(view, minSize);
    dupes.resultScope = scope;
  } catch (e) {
    toast(S.failed(S.dupesFind, e), true);
  }
  dupes.running = false;
  showDupes();
}

/** Takes a deleted file out of the duplicate groups; a group of one is no longer one. */
function forgetDuplicate(path: string) {
  const r = dupes.result;
  if (!r) return;
  const before = r.groups.length;
  for (const g of r.groups) {
    const kept = g.files.filter((f) => f.path !== path);
    if (kept.length !== g.files.length) {
      // `wasted` is one copy's space times the copies beyond the first.
      const each = g.wasted / Math.max(g.files.length - 1, 1);
      g.files = kept;
      g.wasted = each * Math.max(kept.length - 1, 0);
    }
  }
  r.groups = r.groups.filter((g) => g.files.length > 1);
  r.totalGroups -= before - r.groups.length;
  r.wasted = r.groups.reduce((sum, g) => sum + g.wasted, 0);
  showDupes();
}

/** Compares the shown drive with snapshot `id` (or stops), then colours by change. */
async function compareWith(id: string | null) {
  const drive = currentDrive();
  if (drive === null) return;
  if (id) panel.showMessage(S.loadingSnapshot);
  try {
    await api.compare(drive, id);
  } catch (e) {
    toast(S.failed(S.compareWith, e), true);
  }
  if (id && colorMode !== "change") setColorMode("change");
  await show(refreshed(view), false);
}

async function saveSnapshot() {
  const drive = currentDrive();
  if (drive === null) return;
  toast(S.savingSnapshot);
  try {
    await api.saveSnapshot(drive);
    toast(S.snapshotSaved(drives[drive].root.replace(/\\$/, "")));
  } catch (e) {
    toast(S.failed(S.saveSnapshot, e), true);
  }
  void loadPanel();
}

async function deleteSnapshot(id: string) {
  const ok = await confirmDialog({ title: S.deleteSnapshotTitle, body: [S.cannotUndo], ok: S.deleteSnapshot, danger: true });
  if (!ok) return;
  try {
    await api.deleteSnapshot(id);
  } catch (e) {
    toast(S.failed(S.deleteSnapshot, e), true);
  }
  await show(refreshed(view), false);
}

/** Shows the folder that holds `info` (a search result that is not a folder). */
function showContaining(info: NodeInfo) {
  const ref = refOf(info);
  if (!ref || info.parent === null) return;
  const path = info.path.replace(/\\[^\\]+$/, "");
  void show({ kind: "node", ...ref, node: info.parent, path: path.endsWith(":") ? `${path}\\` : path });
}

let searchTimer = 0;

function onSearchInput() {
  els.searchClear.hidden = !els.search.value;
  clearTimeout(searchTimer);
  searchTimer = window.setTimeout(applySearch, SEARCH_DELAY_MS);
}

function applySearch() {
  clearTimeout(searchTimer);
  const next = els.search.value.trim();
  if (next === query) return;
  query = next;
  void show(refreshed(view), false);
}

function clearSearch() {
  els.search.value = "";
  els.searchClear.hidden = true;
  applySearch();
}

function setPanelHidden(hidden: boolean) {
  document.body.classList.toggle("panel-hidden", hidden);
  els.panelToggle.classList.toggle("on", !hidden);
  try {
    localStorage.setItem(PANEL_KEY, hidden ? "1" : "");
  } catch {
    // Not critical.
  }
  if (!hidden) void loadPanel();
}

function goBack() {
  const prev = history.pop();
  if (prev) void show(refreshed(prev), false);
}

function goUp() {
  if (!layout || view.kind === "summary") return;
  const c = layout.center;
  if (c.parent === null) {
    void show(summaryView());
    return;
  }
  const path = c.path.replace(/\\[^\\]+\\?$/, "");
  void show({ ...view, node: c.parent, path: path.endsWith(":") ? `${path}\\` : path });
}

function zoomTo(info: NodeInfo) {
  const ref = refOf(info);
  if (ref && info.kind === "dir") void show({ kind: "node", ...ref });
}

function rescanCurrent() {
  if (view.kind === "summary") void startScan(view.drives);
  else if (layout) rescan(layout.center);
}

/** Rescans a folder, or the whole drive for a drive root. */
function rescan(info: NodeInfo) {
  const ref = refOf(info);
  if (!ref) return;
  if (info.parent === null) void startScan([ref.drive]);
  else api.rescanFolder(ref).catch((e) => toast(S.failed(S.rescanFolder, e), true));
}

async function startScan(indices: number[]) {
  const ready = indices.filter((i) => drives[i]?.ready);
  if (!ready.length) return;
  for (const i of ready) {
    drives[i].scanning = true;
    progress.set(i, { files: 0, allocated: 0 });
    scanErrors.delete(i);
  }
  renderDrives();
  await api.scanDrives(ready);
}

// ---------- File actions ----------

function displayName(info: NodeInfo): string {
  return nodeTitle(info, info.drive !== null ? drives[info.drive]?.scanned ?? true : true);
}

function sizeLine(info: NodeInfo): string {
  const parts = [formatSize(info.used)];
  if (info.kind === "dir") parts.push(S.files(info.files));
  return parts.join(", ");
}

async function removeNode(info: NodeInfo, permanent: boolean) {
  const ref = refOf(info);
  if (!ref) return;
  const name = displayName(info);
  const isLink = info.kind === "link";
  const ok = await confirmDialog({
    title: permanent ? (isLink ? S.deleteLinkTitle : S.deleteTitle) : S.recycleTitle,
    body: [`«${name}»${isLink ? "" : ` — ${sizeLine(info)}`}`, isLink ? S.deleteLinkNote : permanent ? S.cannotUndo : ""],
    path: info.path,
    ok: permanent ? S.deleteButton : S.recycle,
    danger: permanent,
  });
  if (!ok) return;
  setBusy(true);
  try {
    await api.removeNode(ref, permanent);
    toast(permanent ? S.deleted(name) : S.recycled(name));
    forgetDuplicate(info.path);
  } catch (e) {
    toast(S.failed(permanent ? S.deleteButton : S.recycle, e), true);
  } finally {
    setBusy(false);
    void refreshBin();
  }
}

async function emptyBin() {
  const bin = await api.recycleBinInfo().catch(() => null);
  if (bin && bin.items === 0) {
    toast(S.recycleBinEmpty);
    return;
  }
  const ok = await confirmDialog({
    title: S.emptyBinTitle,
    body: [bin ? `${S.items(bin.items)}, ${formatSize(bin.size)}` : "", S.cannotUndo],
    ok: S.emptyBinButton,
    danger: true,
  });
  if (!ok) return;
  setBusy(true);
  try {
    await api.emptyRecycleBin();
    toast(S.binEmptied);
  } catch (e) {
    toast(S.failed(S.emptyBinButton, e), true);
  } finally {
    setBusy(false);
    void refreshBin();
  }
}

async function copyPath(info: NodeInfo) {
  try {
    await navigator.clipboard.writeText(info.path);
    toast(S.pathCopied);
  } catch (e) {
    toast(S.failed(S.copyPath, e), true);
  }
}

function collapseKey(ref: NodeRef): string {
  return `${ref.drive}|${ref.path.toLowerCase()}`;
}

function toggleCollapsed(info: NodeInfo) {
  const ref = refOf(info);
  if (!ref) return;
  const key = collapseKey(ref);
  if (collapsed.has(key)) collapsed.delete(key);
  else collapsed.set(key, ref);
  void show(refreshed(view), false);
}

function run(action: Promise<unknown>, what: string) {
  action.catch((e) => toast(S.failed(what, e), true));
}

/** Context menu items for a node in the chart, depending on what it is. */
function nodeMenu(info: NodeInfo, isCenter: boolean): MenuItem[] {
  const ref = refOf(info);
  if (!ref || info.kind === "free") return [];
  if (info.kind === "unscanned") return [{ label: S.rescanDrive, action: () => void startScan([ref.drive]) }];

  const isDriveRoot = info.parent === null;
  const items: MenuItem[] = [];
  if (isDriveRoot) {
    if (!isCenter) items.push({ label: S.openDrive, action: () => zoomTo(info) });
    items.push(
      { label: S.rescanDrive, action: () => rescan(info) },
      { label: S.openInExplorer, action: () => run(api.openNode(ref), S.open) },
      { label: S.exportFolder, action: () => void exportView({ kind: "node", ...ref }, info) },
    );
    return items;
  }

  if (info.kind === "dir") {
    if (!isCenter) {
      items.push({ label: S.zoom, action: () => zoomTo(info) });
      const hidden = collapsed.has(collapseKey(ref));
      items.push({ label: hidden ? S.showContents : S.hideContents, action: () => toggleCollapsed(info) });
    }
    items.push(
      { label: S.rescanFolder, action: () => rescan(info) },
      { label: S.exportFolder, action: () => void exportView({ kind: "node", ...ref }, info) },
      "-",
    );
  }
  if (info.kind !== "link") items.push({ label: S.open, action: () => run(api.openNode(ref), S.open) });
  items.push(
    { label: S.reveal, action: () => run(api.revealNode(ref), S.reveal) },
    { label: S.copyPath, action: () => void copyPath(info) },
    "-",
  );
  if (info.flags.includes("recycle_bin")) items.push({ label: S.emptyBin, action: () => void emptyBin() });
  if (info.kind === "link") {
    items.push({ label: S.deleteLink, danger: true, action: () => void removeNode(info, true) });
  } else {
    items.push(
      { label: S.recycle, action: () => void removeNode(info, false) },
      { label: S.deleteForever, danger: true, action: () => void removeNode(info, true) },
    );
  }
  return items;
}

async function onChartContextMenu(e: MouseEvent) {
  e.preventDefault();
  if (!layout) return;
  const { x: cx, y: cy } = renderer.center;
  let hit: HoverResult;
  try {
    hit = await api.hover(layout.generation, e.offsetX - cx, e.offsetY - cy);
  } catch {
    return;
  }
  hideTooltip();
  if (hit.hit === "segment" && hit.info) openMenu(e.clientX, e.clientY, nodeMenu(hit.info, false));
  else if (hit.hit === "center" && view.kind === "node") openMenu(e.clientX, e.clientY, nodeMenu(layout.center, true));
}

// ---------- Chrome: sidebar, toolbar, status ----------

function chartSize() {
  const r = els.chart.getBoundingClientRect();
  return { width: Math.max(r.width, 100), height: Math.max(r.height, 100) };
}

function currentDrive(): number | null {
  return view.kind === "node" ? view.drive : null;
}

function updateChrome() {
  els.back.disabled = history.length === 0;
  els.up.disabled = view.kind === "summary";
  els.summary.classList.toggle("active", view.kind === "summary");
  renderDrives();

  const c = layout?.center;
  els.crumb.textContent = view.kind === "summary" ? S.summary : c?.path ?? "";
  els.empty.hidden = !(layout && layout.segments.length === 0);
  els.empty.textContent = view.kind === "summary" ? S.emptySummary : S.emptyFolder;
  setStatus(c ?? null);
}

function setStatus(info: NodeInfo | null) {
  if (folderScans.size) {
    els.status.textContent = S.scanningFolder([...folderScans][0]);
    return;
  }
  if (!info) {
    els.status.textContent = "";
    return;
  }
  const parts = [info.kind === "summary" ? S.summary : info.path || nodeTitle(info), formatSize(info.used)];
  if (info.kind === "dir" || info.kind === "summary") parts.push(`${S.files(info.files)}, ${S.folders(info.dirs)}`);
  els.status.textContent = parts.join(" · ");
}

function setBusy(busy: boolean) {
  document.body.classList.toggle("busy", busy);
}

function renderDrives() {
  const active = currentDrive();
  const shown = drives.filter((d) => d.present).sort((a, b) => a.root.localeCompare(b.root));
  els.drives.replaceChildren(
    ...shown.map((d) => {
      const btn = document.createElement("button");
      btn.className = "drive";
      btn.classList.toggle("active", d.index === active);
      btn.classList.toggle("in-summary", summaryRoots.has(d.root));
      btn.disabled = !d.ready;
      const icon = ICONS[d.kind as keyof typeof ICONS] ?? ICONS.fixed;
      const usedShare = d.capacity > 0 ? (d.capacity - d.free) / d.capacity : 0;
      const p = progress.get(d.index);
      const used = d.capacity - d.free;
      let sub: string;
      if (!d.ready) sub = S.notReady;
      else if (d.scanning) sub = p ? `${S.scanning} ${formatPercent(p.allocated, used)}` : S.scanning;
      else if (scanErrors.has(d.index)) sub = S.scanError;
      else sub = `${formatSize(d.free)} ${S.free} ${S.of} ${formatSize(d.capacity)}`;
      const scanShare = d.scanning && p && used > 0 ? Math.min(p.allocated / used, 1) : null;

      btn.innerHTML = `
        <span class="drive-icon">${icon}</span>
        <span class="drive-text">
          <span class="drive-name"><b>${escapeHtml(d.root.slice(0, 2))}</b> ${escapeHtml(d.label)}</span>
          <span class="drive-sub">${escapeHtml(sub)}</span>
          <span class="bar"><span class="bar-used" style="width:${(usedShare * 100).toFixed(1)}%"></span>${
            scanShare !== null ? `<span class="bar-scan" style="width:${(scanShare * 100).toFixed(1)}%"></span>` : ""
          }</span>
        </span>`;
      const method = d.scanMethod === "mft" ? S.methodMft : d.scanMethod === "walk" ? S.methodWalk : "";
      btn.title = [d.root, d.label, d.fileSystem, method, scanErrors.get(d.index)].filter(Boolean).join(" · ");
      btn.addEventListener("click", () => onDriveClick(d));
      btn.addEventListener("contextmenu", (e) => {
        e.preventDefault();
        openDriveMenu(d, e.clientX, e.clientY);
      });
      return btn;
    }),
  );
}

function onDriveClick(d: Drive) {
  // As in the original: clicking the drive already shown at its root rescans it.
  if (view.kind === "node" && view.drive === d.index && layout?.center.parent === null && !d.scanning) {
    void startScan([d.index]);
    return;
  }
  if (!d.scanned && !d.scanning) void startScan([d.index]);
  void show(driveRootView(d));
}

function openDriveMenu(d: Drive, x: number, y: number) {
  const root: NodeRef = { drive: d.index, node: 0, treeGen: d.treeGen, path: d.root };
  openMenu(x, y, [
    {
      label: S.inSummary,
      checked: summaryRoots.has(d.root),
      action: () => {
        if (summaryRoots.has(d.root)) summaryRoots.delete(d.root);
        else summaryRoots.add(d.root);
        saveSummaryRoots();
        if (view.kind === "summary") void show(summaryView(), false);
        else renderDrives();
      },
    },
    { label: S.rescan, action: () => void startScan([d.index]) },
    { label: S.openInExplorer, action: () => run(api.openNode(root), S.open) },
    ...(d.scanned && !d.scanning
      ? [
          {
            label: S.saveSnapshot,
            action: () =>
              void api
                .saveSnapshot(d.index)
                .then(() => {
                  toast(S.snapshotSaved(d.root.replace(/\\$/, "")));
                  if (panelTab === "changes") void loadPanel();
                })
                .catch((e) => toast(S.failed(S.saveSnapshot, e), true)),
          },
        ]
      : []),
  ]);
}

async function openMoreMenu() {
  const installed = await api.explorerMenuInstalled().catch(() => false);
  const r = els.more.getBoundingClientRect();
  openMenu(r.right, r.bottom + 4, [
    { label: S.exportView, action: () => void exportView(view, layout?.center ?? null) },
    "-",
    {
      label: S.explorerMenu,
      checked: installed,
      action: () =>
        api
          .setExplorerMenu(!installed)
          .then(() => toast(installed ? S.explorerMenuOff : S.explorerMenuOn))
          .catch((e) => toast(S.failed(S.explorerMenu, e), true)),
    },
  ], true);
}

async function setupFastMode() {
  const on = await api.fastMode().catch(() => false);
  els.fast.innerHTML = ICONS.fast;
  els.fast.classList.toggle("on", on);
  els.fast.title = on ? S.fastModeOn : S.fastModeOff;
  els.fast.addEventListener("click", async () => {
    if (on) {
      toast(S.fastModeOn);
      return;
    }
    const ok = await confirmDialog({ title: S.fastModeTitle, body: S.fastModeBody, ok: S.fastModeButton });
    if (!ok) return;
    try {
      const closing = await api.enableFastMode();
      if (!closing) toast(S.fastModeDev);
    } catch (e) {
      toast(S.failed(S.fastMode, e), true);
    }
  });
}

async function refreshBin() {
  try {
    const bin = await api.recycleBinInfo();
    els.bin.innerHTML = `${ICONS.bin}<span>${bin.items ? formatSize(bin.size) : ""}</span>`;
    els.bin.title = bin.items ? `${S.recycleBin}: ${S.items(bin.items)}, ${formatSize(bin.size)} — ${S.emptyBin}` : S.recycleBinEmpty;
  } catch {
    els.bin.innerHTML = ICONS.bin;
    els.bin.title = S.recycleBin;
  }
}

// ---------- Menu, dialog, toast ----------

type MenuItem = { label: string; checked?: boolean; danger?: boolean; action: () => void } | "-";

/** Opens a menu at `x, y`; with `alignRight`, its right edge is at `x`. */
function openMenu(x: number, y: number, items: MenuItem[], alignRight = false) {
  if (!items.length) return;
  els.menu.replaceChildren(
    ...items.map((item) => {
      if (item === "-") {
        const sep = document.createElement("div");
        sep.className = "sep";
        return sep;
      }
      const b = document.createElement("button");
      b.textContent = item.label;
      if (item.checked !== undefined) b.classList.toggle("checked", item.checked);
      b.classList.toggle("danger", !!item.danger);
      b.addEventListener("click", () => {
        closeMenu();
        item.action();
      });
      return b;
    }),
  );
  els.menu.hidden = false;
  const r = els.menu.getBoundingClientRect();
  const left = alignRight ? x - r.width : x;
  els.menu.style.left = `${Math.max(4, Math.min(left, innerWidth - r.width - 4))}px`;
  els.menu.style.top = `${Math.max(4, Math.min(y, innerHeight - r.height - 4))}px`;
}

function closeMenu() {
  els.menu.hidden = true;
}

let resolveDialog: ((ok: boolean) => void) | null = null;

/** Asks the user to confirm; resolves `true` only on the OK button. A `path`
 * is shown in small print after the first line. */
function confirmDialog(opts: {
  title: string;
  body: string[];
  path?: string;
  ok: string;
  danger?: boolean;
  /** Form controls shown above the text. */
  content?: HTMLElement[];
}): Promise<boolean> {
  closeDialog(false);
  els.modalTitle.textContent = opts.title;
  const line = (text: string, className = "") => {
    const p = document.createElement("p");
    p.textContent = text;
    p.className = className;
    return p;
  };
  const [first, ...rest] = opts.body.filter(Boolean);
  els.modalBody.replaceChildren(
    ...(opts.content ?? []),
    ...(first ? [line(first, opts.content ? "note" : "")] : []),
    ...(opts.path ? [line(opts.path, "path")] : []),
    ...rest.map((text) => line(text)),
  );
  els.modalOk.textContent = opts.ok;
  els.modalOk.classList.toggle("danger", !!opts.danger);
  els.modalCancel.textContent = S.cancel;
  els.modal.hidden = false;
  // Destructive actions start on Cancel so Enter does not delete by accident.
  (opts.danger ? els.modalCancel : els.modalOk).focus();
  return new Promise((resolve) => (resolveDialog = resolve));
}

function closeDialog(ok: boolean) {
  els.modal.hidden = true;
  const resolve = resolveDialog;
  resolveDialog = null;
  resolve?.(ok);
}

let toastTimer = 0;

function toast(text: string, error = false) {
  els.toast.textContent = text;
  els.toast.classList.toggle("error", error);
  els.toast.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => (els.toast.hidden = true), error ? 7000 : 3500);
}

function loadSummaryRoots() {
  try {
    const saved = localStorage.getItem(SUMMARY_KEY);
    if (saved) {
      summaryRoots = new Set(JSON.parse(saved) as string[]);
      return;
    }
  } catch {
    // Storage unavailable or corrupt: fall back to the default below.
  }
  summaryRoots = new Set(drives.filter((d) => d.kind === "fixed" && d.ready).map((d) => d.root));
}

function saveSummaryRoots() {
  try {
    localStorage.setItem(SUMMARY_KEY, JSON.stringify([...summaryRoots]));
  } catch {
    // Not critical: the choice just will not survive a restart.
  }
}

// ---------- Hover and tooltip ----------

let pendingMove: { x: number; y: number; clientX: number; clientY: number } | null = null;
/** The pointer over the chart, to hit-test again after the picture changes under it. */
let pointer: { x: number; y: number; clientX: number; clientY: number } | null = null;

/** Hit-tests the pointer again, e.g. after a scan update redrew the chart. */
function rehover() {
  if (!pointer || pendingMove) return;
  pendingMove = pointer;
  requestAnimationFrame(processMove);
}

function onMouseMove(e: MouseEvent) {
  if (!els.menu.hidden) return;
  const scheduled = pendingMove !== null;
  pendingMove = { x: e.offsetX, y: e.offsetY, clientX: e.clientX, clientY: e.clientY };
  pointer = pendingMove;
  if (!scheduled) requestAnimationFrame(processMove);
}

async function processMove() {
  const move = pendingMove;
  pendingMove = null;
  if (!move || !layout) return;
  const seq = ++hoverSeq;
  const { x: cx, y: cy } = renderer.center;
  let result: HoverResult;
  try {
    result = await api.hover(layout.generation, move.x - cx, move.y - cy);
  } catch {
    return;
  }
  if (seq !== hoverSeq) return;
  lastHover = result;
  if (result.hit === "segment" && result.segment !== null && result.info) {
    renderer.setHighlight({ kind: "segment", index: result.segment });
    els.canvas.style.cursor = result.info.kind === "dir" ? "pointer" : "default";
    showTooltip(result.info, move.clientX, move.clientY, result.change);
    setStatus(result.info);
  } else if (result.hit === "center" && view.kind === "node") {
    renderer.setHighlight({ kind: "center" });
    els.canvas.style.cursor = "pointer";
    hideTooltip();
    setStatus(layout.center);
  } else {
    renderer.setHighlight(null);
    els.canvas.style.cursor = "default";
    hideTooltip();
    setStatus(layout.center);
  }
}

function showTooltip(info: NodeInfo, clientX: number, clientY: number, change: Change | null = null) {
  const drive = info.drive !== null ? drives[info.drive] : undefined;
  const isDriveRoot = view.kind === "summary" && info.parent === null;
  let title = displayName(info);
  if (isDriveRoot && drive?.label) title += ` ${drive.label}`;
  const whole = layout?.center.allocated ?? 0;
  const ref = refOf(info);

  const lines = [`<div class="tt-title">${escapeHtml(title)}</div>`];
  if (info.kind !== "free" && info.kind !== "unscanned") lines.push(`<div class="tt-path">${escapeHtml(info.path)}</div>`);
  lines.push(`<div class="tt-size"><b>${formatSize(info.used)}</b> · ${formatPercent(info.allocated, whole)}</div>`);
  if (info.free > 0) lines.push(`<div>${S.free} ${formatSize(info.free)}</div>`);
  if (change && info.kind !== "free" && info.kind !== "unscanned") {
    const date = formatShortDate(change.since);
    const text = change.added ? S.newSince(date) : S.changeLine(formatDelta(change.delta), date);
    const cls = change.added || change.delta > 0 ? "tt-grew" : change.delta < 0 ? "tt-shrank" : "tt-note";
    lines.push(`<div class="${cls}">${escapeHtml(text)}</div>`);
  }
  if (info.kind === "dir") lines.push(`<div>${S.files(info.files)}, ${S.folders(info.dirs)}</div>`);
  if (info.kind === "link") lines.push(`<div class="tt-note">${S.link}</div>`);
  if (ref && collapsed.has(collapseKey(ref))) lines.push(`<div class="tt-note">${S.contentsHidden}</div>`);
  if (info.flags.includes("access_denied")) lines.push(`<div class="tt-warn">${S.accessDenied}</div>`);
  els.tooltip.innerHTML = lines.join("");
  els.tooltip.hidden = false;

  const box = els.chart.getBoundingClientRect();
  const tt = els.tooltip.getBoundingClientRect();
  let left = clientX - box.left + 16;
  let top = clientY - box.top + 16;
  if (left + tt.width > box.width - 8) left = clientX - box.left - tt.width - 16;
  if (top + tt.height > box.height - 8) top = clientY - box.top - tt.height - 16;
  els.tooltip.style.left = `${Math.max(8, left)}px`;
  els.tooltip.style.top = `${Math.max(8, top)}px`;
}

function hideTooltip() {
  els.tooltip.hidden = true;
}

function onChartClick() {
  const h = lastHover;
  if (!h || !layout) return;
  if (h.hit === "center") goUp();
  else if (h.hit === "segment" && h.info) zoomTo(h.info);
}

// ---------- Backend events ----------

async function refreshDrives() {
  drives = await api.listDrives();
}

function viewUsesDrive(v: View, drive: number): boolean {
  return v.kind === "summary" ? v.drives.includes(drive) : v.drive === drive;
}

/** A drive's tree was replaced or changed: redraw if it is on screen. */
async function onTreeChanged(drive: number) {
  await refreshDrives();
  if (viewUsesDrive(view, drive)) await show(refreshed(view), false);
  else updateChrome();
}

/** Drives were plugged in, pulled out, or had their media changed. */
async function onDrivesChanged(changed: number[]) {
  const before = new Map(drives.map((d) => [d.index, d.present && d.ready]));
  await refreshDrives();
  for (const i of changed) {
    progress.delete(i);
    scanErrors.delete(i);
    const d = drives[i];
    if (!d) continue;
    const name = d.label ? `${d.root.slice(0, 2)} ${d.label}` : d.root.slice(0, 2);
    const nowReady = d.present && d.ready;
    if (nowReady && !before.get(i)) toast(S.driveAdded(name));
    else if (!nowReady && before.get(i)) toast(S.driveRemoved(name));
    // Views into the old contents of this drive are meaningless now.
    for (let h = history.length - 1; h >= 0; h--) {
      const v = history[h];
      if (v.kind === "node" && v.drive === i) history.splice(h, 1);
    }
  }
  if (view.kind === "node" && changed.includes(view.drive)) {
    const d = drives[view.drive];
    await show(d?.present && d.ready ? driveRootView(d) : summaryView(), false);
  } else if (view.kind === "summary") {
    await show(summaryView(), false);
  } else {
    updateChrome();
  }
}

async function onScanFinished(drive: number, error: string | null) {
  progress.delete(drive);
  if (error) scanErrors.set(drive, error);
  await onTreeChanged(drive);
}

// ---------- Setup ----------

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}

function setupButtons() {
  els.back.innerHTML = ICONS.back;
  els.up.innerHTML = ICONS.up;
  els.rescan.innerHTML = ICONS.rescan;
  els.bin.innerHTML = ICONS.bin;
  els.apps.innerHTML = ICONS.apps;
  els.more.innerHTML = ICONS.more;
  els.panelToggle.innerHTML = ICONS.panel;
  els.panelToggle.title = S.togglePanel;
  els.search.placeholder = S.searchPlaceholder;
  els.searchClear.title = S.searchClear;
  els.colorMode.innerHTML = ICONS.palette;
  const savedMode = load(COLOR_KEY);
  setColorMode(savedMode === "type" || savedMode === "change" ? savedMode : "folder");
  const savedTab = load(TAB_KEY);
  panelTab = savedTab === "types" || savedTab === "changes" || savedTab === "dupes" ? savedTab : "contents";
  dupes.minSize = Number(load(DUP_SIZE_KEY)) || dupes.minSize;
  els.colorMode.addEventListener("click", openColorMenu);
  els.panelToggle.addEventListener("click", () =>
    setPanelHidden(!document.body.classList.contains("panel-hidden")),
  );
  els.search.addEventListener("input", onSearchInput);
  els.search.addEventListener("keydown", (e) => {
    if (e.key === "Enter") applySearch();
    else if (e.key === "Escape") {
      if (els.search.value) clearSearch();
      else els.search.blur();
    } else return;
    e.preventDefault();
    e.stopPropagation();
  });
  els.searchClear.addEventListener("click", () => {
    clearSearch();
    els.search.focus();
  });
  let panelHidden = false;
  try {
    panelHidden = localStorage.getItem(PANEL_KEY) === "1";
  } catch {
    // Storage unavailable: show the panel.
  }
  setPanelHidden(panelHidden);
  els.back.title = `${S.back} (Backspace)`;
  els.up.title = `${S.up} (Alt+↑)`;
  els.rescan.title = `${S.rescan} (F5)`;
  els.apps.title = S.apps;
  els.more.title = S.more;
  els.summary.innerHTML = `<span class="drive-icon">${ICONS.summary}</span><span class="drive-name"><b>${S.summary}</b></span>`;

  els.back.addEventListener("click", goBack);
  els.up.addEventListener("click", goUp);
  els.rescan.addEventListener("click", rescanCurrent);
  els.bin.addEventListener("click", () => void emptyBin());
  els.apps.addEventListener("click", () => {
    run(api.openAppsSettings(), S.apps);
    toast(S.appsHint);
  });
  els.more.addEventListener("click", () => void openMoreMenu());
  els.summary.addEventListener("click", () => void show(summaryView()));

  els.canvas.addEventListener("mousemove", onMouseMove);
  els.canvas.addEventListener("mouseleave", () => {
    pendingMove = null;
    pointer = null;
    hoverSeq++;
    lastHover = null;
    renderer.setHighlight(null);
    hideTooltip();
    setStatus(layout?.center ?? null);
  });
  els.canvas.addEventListener("click", onChartClick);
  els.canvas.addEventListener("contextmenu", (e) => void onChartContextMenu(e));

  els.modalOk.addEventListener("click", () => closeDialog(true));
  els.modalCancel.addEventListener("click", () => closeDialog(false));
  els.modal.addEventListener("mousedown", (e) => {
    if (e.target === els.modal) closeDialog(false);
  });

  document.addEventListener("contextmenu", (e) => e.preventDefault());
  document.addEventListener("mousedown", (e) => {
    if (!els.menu.contains(e.target as Node)) closeMenu();
  });
  document.addEventListener("keydown", (e) => {
    if (!els.modal.hidden) {
      if (e.key === "Escape") closeDialog(false);
      return;
    }
    if ((e.ctrlKey && e.key.toLowerCase() === "f") || (e.ctrlKey && e.code === "KeyF")) {
      els.search.focus();
      els.search.select();
      e.preventDefault();
      return;
    }
    // Typing in the search box must not trigger Back on Backspace.
    if (e.target instanceof HTMLInputElement) return;
    if (e.key === "Escape") {
      if (!els.menu.hidden) closeMenu();
      else if (query) clearSearch();
    }
    else if (e.key === "Backspace" || (e.altKey && e.key === "ArrowLeft")) goBack();
    else if (e.altKey && e.key === "ArrowUp") goUp();
    else if (e.key === "F5") rescanCurrent();
    else return;
    e.preventDefault();
  });
  window.addEventListener("focus", () => void refreshBin());

  let resizeTimer = 0;
  new ResizeObserver(() => {
    const { width, height } = chartSize();
    renderer.resize(width, height);
    clearTimeout(resizeTimer);
    // Re-layout once the size settles; the old picture stays until then.
    resizeTimer = window.setTimeout(() => void show(refreshed(view), false), 60);
  }).observe(els.chart);

  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    renderer.refreshTheme();
    renderLegend();
    void loadPanel();
  });
}

async function main() {
  setupButtons();
  await api.onScanProgress((p) => {
    progress.set(p.drive, { files: p.files, allocated: p.allocated });
    renderDrives();
  });
  await api.onScanFinished((f) => void onScanFinished(f.drive, f.error));
  // The chart of a drive's first scan grows while it runs.
  await api.onScanPartial(async (p) => {
    await onTreeChanged(p.drive);
    rehover();
  });
  await api.onTreeChanged((t) => {
    void onTreeChanged(t.drive);
    void refreshBin();
  });
  await api.onDrivesChanged((changed) => void onDrivesChanged(changed));
  await api.onDupProgress((p) => {
    if (!dupes.running) return;
    dupes.progress = p;
    showDupes();
  });
  await api.onFolderScan((f) => {
    if (f.running) folderScans.add(f.path);
    else folderScans.delete(f.path);
    if (f.error) toast(S.failed(S.rescanFolder, f.error), true);
    setStatus(layout?.center ?? null);
  });

  void setupFastMode();
  await refreshDrives();
  loadSummaryRoots();
  const { width, height } = chartSize();
  renderer.resize(width, height);
  void refreshBin();

  // Started with a folder (e.g. from Explorer): scan it first and show it right away.
  const start = await api.startupTarget().catch(() => null);
  if (start) {
    const isRoot = start.path.length <= 3;
    if (isRoot) await startScan([start.drive]);
    else await api.rescanFolder(start).catch((e) => toast(S.failed(S.rescanFolder, e), true));
    await show({ kind: "node", ...start }, false);
  } else {
    await show(summaryView(), false);
  }
  // As in the original, every summary drive is scanned right away.
  await startScan(drives.filter((d) => summaryRoots.has(d.root) && !d.scanned && !d.scanning).map((d) => d.index));
}

void main();
