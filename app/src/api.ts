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
  ready: boolean;
  scanned: boolean;
  scanning: boolean;
  treeGen: number;
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
}

export interface Layout {
  generation: number;
  centerRadius: number;
  ringWidth: number;
  rings: number;
  center: NodeInfo;
  segments: Segment[];
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
}

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
  layout: (view: View, width: number, height: number, collapsed: NodeRef[]) =>
    invoke<Layout>("layout", { view, width, height, collapsed }),
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

  onScanProgress: (cb: (p: ScanProgress) => void): Promise<UnlistenFn> =>
    listen<ScanProgress>("scan-progress", (e) => cb(e.payload)),
  onScanFinished: (cb: (p: ScanFinished) => void): Promise<UnlistenFn> =>
    listen<ScanFinished>("scan-finished", (e) => cb(e.payload)),
  onTreeChanged: (cb: (p: TreeChanged) => void): Promise<UnlistenFn> =>
    listen<TreeChanged>("tree-changed", (e) => cb(e.payload)),
  onFolderScan: (cb: (p: FolderScan) => void): Promise<UnlistenFn> =>
    listen<FolderScan>("folder-scan", (e) => cb(e.payload)),
};

/** The reference to `info`, if it is a real node of a drive. */
export function refOf(info: NodeInfo): NodeRef | null {
  if (info.drive === null || info.node === null || info.treeGen === null) return null;
  return { drive: info.drive, node: info.node, treeGen: info.treeGen, path: info.path };
}
