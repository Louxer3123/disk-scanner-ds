// Types and calls mirroring app/src-tauri/src/commands.rs.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type DriveKind = "fixed" | "removable" | "network" | "cdrom" | "ram" | "unknown";

export interface Drive {
  index: number;
  root: string;
  label: string;
  fileSystem: string;
  kind: DriveKind;
  capacity: number;
  free: number;
  physicalDisk: number | null;
  /** False once the drive letter is gone; indices stay stable. */
  present: boolean;
  ready: boolean;
  scanned: boolean;
  scanning: boolean;
  treeGen: number;
  /** How the last full scan read the drive. */
  scanMethod: "mft" | "walk" | null;
}

export type NodeKind = "dir" | "file" | "link" | "free" | "unscanned" | "summary";

export interface NodeInfo {
  drive: number | null;
  node: number | null;
  treeGen: number | null;
  name: string;
  path: string;
  kind: NodeKind;
  allocated: number;
  used: number;
  free: number;
  logical: number;
  files: number;
  dirs: number;
  flags: string[];
  parent: number | null;
}

export interface Segment {
  drive: number;
  node: number;
  depth: number;
  /** Radians, clockwise from 12 o'clock. */
  start: number;
  sweep: number;
  /** Index of the parent segment, -1 in ring 1. */
  parent: number;
  kind: NodeKind;
  name: string;
  /** Treemap tiles: `[x, y, width, height]` in px from the top left. */
  rect: [number, number, number, number] | null;
  /** Treemap tiles: a folder with its name strip at the top. */
  header: boolean;
}

export type ChartKind = "sunburst" | "treemap";

export interface Layout {
  generation: number;
  chart: ChartKind;
  centerRadius: number;
  ringWidth: number;
  rings: number;
  /** Treemap: height of the bar for the shown folder at the top. */
  topBar: number;
  /** Treemap: height of a folder's name strip. */
  header: number;
  center: NodeInfo;
  segments: Segment[];
  /** With a search query: per segment 0 no match, 1 contains a match, 2 matches. */
  marks: number[] | null;
  /** When colouring by file type: per segment a category (see CATEGORY_HUES). */
  categories: number[] | null;
  /** When colouring by change: per segment the growth in bytes since the snapshot... */
  changes: number[] | null;
  /** ...and 0 if it was there before, 1 if new, 2 if its drive is not compared. */
  changeKinds: number[] | null;
}

export type ColorMode = "folder" | "type" | "change";

export type ExportFormat = "csv" | "json";

export interface Snapshot {
  id: string;
  root: string;
  /** Unix seconds. */
  created: number;
  allocated: number;
  files: number;
  dirs: number;
  bytes: number;
}

export interface Change {
  delta: number;
  added: boolean;
  since: number;
}

export interface ChangedNode {
  info: NodeInfo;
  delta: number;
  added: boolean;
}

export interface RemovedNode {
  name: string;
  path: string;
  kind: NodeKind;
  allocated: number;
}

export interface Changes {
  /** Id of the compared snapshot; null when the drive is not compared. */
  snapshot: string | null;
  since: number | null;
  delta: number;
  children: ChangedNode[];
  files: ChangedNode[];
  removed: RemovedNode[];
  removedTotal: number;
  removedAllocated: number;
}

export interface Children {
  items: NodeInfo[];
  total: number;
  restAllocated: number;
}

export interface CategoryStat {
  category: number;
  files: number;
  allocated: number;
}

export interface ExtensionStat {
  /** Lower-case, without the dot; empty for files without one. */
  extension: string;
  category: number;
  files: number;
  allocated: number;
}

export interface FileTypes {
  categories: CategoryStat[];
  extensions: ExtensionStat[];
  total: number;
  restAllocated: number;
}

export interface DupGroup {
  /** Length of each copy. */
  size: number;
  /** Space freed by keeping one copy. */
  wasted: number;
  files: NodeInfo[];
}

export interface Duplicates {
  groups: DupGroup[];
  totalGroups: number;
  wasted: number;
  errors: number;
  skipped: number;
  cancelled: boolean;
}

export interface DupProgress {
  /** 1: first blocks, 2: whole files. */
  phase: number;
  done: number;
  total: number;
  bytesDone: number;
  bytesTotal: number;
}

export interface Settings {
  values: Record<string, string>;
  /** Where settings and snapshots are kept. */
  dataDir: string;
  /** Data is kept beside the exe (a `portable.txt` file is there). */
  portable: boolean;
  /** Windows redirects the data folder elsewhere; `dataDir` is the real place. */
  redirected: boolean;
  version: string;
  /** `windows`, `linux` or `macos`. */
  os: string;
}

export type StorageKind = "program" | "settings" | "snapshots" | "webview" | "explorer_menu";

/** One place where the program keeps files. */
export interface StorageItem {
  kind: StorageKind;
  path: string | null;
  /** Exists; for the Explorer menu: is registered. */
  present: boolean;
  bytes: number;
  files: number;
}

export interface SmartAttribute {
  id: number;
  name: string;
  current: number;
  worst: number;
  threshold: number;
  raw: number;
  prefailure: boolean;
}

export type Verdict = "good" | "caution" | "bad" | "unknown";

/** One physical disk on the Disk health screen. */
export interface DiskHealth {
  number: number;
  model: string;
  serial: string;
  firmware: string;
  bus: string;
  size: number;
  rotational: boolean | null;
  verdict: Verdict;
  /** Why the verdict is what it is, in short English phrases. */
  reasons: string[];
  letters: string[];
  temperature: number | null;
  powerOnHours: number | null;
  powerCycles: number | null;
  unsafeShutdowns: number | null;
  bytesWritten: number | null;
  bytesRead: number | null;
  lifeLeft: number | null;
  availableSpare: number | null;
  spareThreshold: number | null;
  mediaErrors: number | null;
  reallocatedSectors: number | null;
  pendingSectors: number | null;
  uncorrectableSectors: number | null;
  attributes: SmartAttribute[];
  smartError: "needs_admin" | "unsupported" | null;
}

export interface HealthReport {
  disks: DiskHealth[];
  elevated: boolean;
  /** How S.M.A.R.T. that needs rights is read: by restarting as administrator
   * (Windows), once through pkexec (Linux), or not at all. */
  elevation: "restart" | "pkexec" | "none";
}

export interface SearchResult {
  total: number;
  allocated: number;
  items: NodeInfo[];
}

/**
 * A node as the backend knows it. When `treeGen` is outdated (the drive's tree
 * changed), the backend finds the node again by `path`.
 */
export interface NodeRef {
  drive: number;
  node: number;
  treeGen: number;
  path: string;
}

export type View = { kind: "summary"; drives: number[] } | ({ kind: "node" } & NodeRef);

export interface HoverResult {
  hit: "none" | "center" | "segment";
  segment: number | null;
  info: NodeInfo | null;
  /** Growth since the snapshot the drive is compared with. */
  change: Change | null;
}

/** Hover positions: px from the centre for a sunburst, from the top left for a treemap. */
export type ChartPoint = { x: number; y: number };

export interface ScanProgress {
  drive: number;
  files: number;
  allocated: number;
}

export interface ScanFinished {
  drive: number;
  treeGen: number;
  files: number;
  dirs: number;
  errors: number;
  elapsedMs: number;
  cancelled: boolean;
  error: string | null;
}

export interface TreeChanged {
  drive: number;
  treeGen: number;
}

export interface FolderScan {
  drive: number;
  path: string;
  running: boolean;
  error: string | null;
}

export interface RecycleBin {
  size: number;
  items: number;
}

export const api = {
  listDrives: () => invoke<Drive[]>("list_drives"),
  scanDrives: (drives: number[]) => invoke<void>("scan_drives", { drives }),
  layout: (
    view: View,
    width: number,
    height: number,
    collapsed: NodeRef[],
    query: string | null,
    color: ColorMode,
    chart: ChartKind,
  ) => invoke<Layout>("layout", { view, width, height, collapsed, query, color, chart }),
  children: (view: View, limit: number) => invoke<Children>("children", { view, limit }),
  search: (view: View, query: string, limit: number) => invoke<SearchResult>("search", { view, query, limit }),
  fileTypes: (view: View, limit: number) => invoke<FileTypes>("file_types", { view, limit }),
  changes: (view: View, limit: number) => invoke<Changes>("changes", { view, limit }),
  diskHealth: () => invoke<HealthReport>("disk_health"),
  /** Linux: reads the health as root through pkexec, which asks for the password. */
  diskHealthElevated: () => invoke<HealthReport>("disk_health_elevated"),
  findDupes: (view: View, minSize: number) => invoke<Duplicates>("find_dupes", { view, minSize }),
  cancelDupes: () => invoke<void>("cancel_dupes"),
  /** Asks where to save and writes the file; resolves to its path, or `null` if cancelled. */
  exportView: (view: View, format: ExportFormat, maxDepth: number | null, minSize: number, fileName: string) =>
    invoke<string | null>("export_view", { view, format, maxDepth, minSize, fileName }),
  listSnapshots: (root: string | null) => invoke<Snapshot[]>("list_snapshots", { root }),
  saveSnapshot: (drive: number) => invoke<Snapshot>("save_snapshot", { drive }),
  deleteSnapshot: (id: string) => invoke<void>("delete_snapshot", { id }),
  /** Compares the drive with snapshot `id` from now on; `null` stops comparing. */
  compare: (drive: number, id: string | null) => invoke<void>("compare", { drive, id }),
  hover: (generation: number, x: number, y: number) => invoke<HoverResult>("hover", { generation, x, y }),

  openNode: (target: NodeRef) => invoke<void>("open_node", { target }),
  revealNode: (target: NodeRef) => invoke<void>("reveal_node", { target }),
  removeNode: (target: NodeRef, permanent: boolean) => invoke<void>("remove_node", { target, permanent }),
  rescanFolder: (target: NodeRef) => invoke<void>("rescan_folder", { target }),
  recycleBinInfo: () => invoke<RecycleBin>("recycle_bin_info"),
  emptyRecycleBin: () => invoke<void>("empty_recycle_bin"),
  openAppsSettings: () => invoke<void>("open_apps_settings"),
  explorerMenuInstalled: () => invoke<boolean>("explorer_menu_installed"),
  setExplorerMenu: (enabled: boolean) => invoke<void>("set_explorer_menu", { enabled }),
  startupTarget: () => invoke<NodeRef | null>("startup_target"),
  fastMode: () => invoke<boolean>("fast_mode"),
  getSettings: () => invoke<Settings>("get_settings"),
  /** Sets a setting, or removes it with `null`. */
  setSetting: (key: string, value: string | null) => invoke<void>("set_setting", { key, value }),
  openDataDir: () => invoke<void>("open_data_dir"),
  /** The language of messages from the backend (errors, the Explorer menu entry). */
  setLanguage: (lang: string) => invoke<void>("set_language", { lang }),
  storageInfo: () => invoke<StorageItem[]>("storage_info"),
  /** Shows one of the places from `storageInfo` in Explorer. */
  openStorage: (kind: StorageKind) => invoke<void>("open_storage", { kind }),
  /** Resolves `true` if this window is about to close, `false` if it stays (dev build). */
  enableFastMode: () => invoke<boolean>("enable_fast_mode"),

  onScanProgress: (cb: (p: ScanProgress) => void): Promise<UnlistenFn> =>
    listen<ScanProgress>("scan-progress", (e) => cb(e.payload)),
  onScanFinished: (cb: (p: ScanFinished) => void): Promise<UnlistenFn> =>
    listen<ScanFinished>("scan-finished", (e) => cb(e.payload)),
  /** A newer partial tree of a drive's first scan can be shown. */
  onScanPartial: (cb: (p: TreeChanged) => void): Promise<UnlistenFn> =>
    listen<TreeChanged>("scan-partial", (e) => cb(e.payload)),
  onTreeChanged: (cb: (p: TreeChanged) => void): Promise<UnlistenFn> =>
    listen<TreeChanged>("tree-changed", (e) => cb(e.payload)),
  onFolderScan: (cb: (p: FolderScan) => void): Promise<UnlistenFn> =>
    listen<FolderScan>("folder-scan", (e) => cb(e.payload)),
  onDupProgress: (cb: (p: DupProgress) => void): Promise<UnlistenFn> =>
    listen<DupProgress>("dup-progress", (e) => cb(e.payload)),
  onDrivesChanged: (cb: (changed: number[]) => void): Promise<UnlistenFn> =>
    listen<{ changed: number[] }>("drives-changed", (e) => cb(e.payload.changed)),
};

/** The reference to `info`, if it is a real node of a drive. */
export function refOf(info: NodeInfo): NodeRef | null {
  if (info.drive === null || info.node === null || info.treeGen === null) return null;
  return { drive: info.drive, node: info.node, treeGen: info.treeGen, path: info.path };
}
