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

export type View =
  | { kind: "summary"; drives: number[] }
  | { kind: "node"; drive: number; node: number; treeGen: number };

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

export const api = {
  listDrives: () => invoke<Drive[]>("list_drives"),
  scanDrives: (drives: number[]) => invoke<void>("scan_drives", { drives }),
  layout: (view: View, width: number, height: number) => invoke<Layout>("layout", { view, width, height }),
  hover: (generation: number, x: number, y: number) => invoke<HoverResult>("hover", { generation, x, y }),
  onScanProgress: (cb: (p: ScanProgress) => void): Promise<UnlistenFn> =>
    listen<ScanProgress>("scan-progress", (e) => cb(e.payload)),
  onScanFinished: (cb: (p: ScanFinished) => void): Promise<UnlistenFn> =>
    listen<ScanFinished>("scan-finished", (e) => cb(e.payload)),
};

/** Whether a rejected call failed because the view refers to a replaced tree or layout. */
export function isStale(err: unknown): boolean {
  return String(err) === "stale";
}
