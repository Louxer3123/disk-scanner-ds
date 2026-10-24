import "./style.css";
import { api, isStale, type Drive, type HoverResult, type Layout, type NodeInfo, type View } from "./api";
import { formatPercent, formatSize } from "./format";
import { S, nodeTitle } from "./strings";
import { SunburstRenderer } from "./sunburst";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const els = {
  drives: $<HTMLDivElement>("drives"),
  summary: $<HTMLButtonElement>("summary-btn"),
  back: $<HTMLButtonElement>("back"),
  up: $<HTMLButtonElement>("up"),
  rescan: $<HTMLButtonElement>("rescan"),
  crumb: $<HTMLDivElement>("crumb"),
  chart: $<HTMLDivElement>("chart"),
  canvas: $<HTMLCanvasElement>("canvas"),
  tooltip: $<HTMLDivElement>("tooltip"),
  empty: $<HTMLDivElement>("empty"),
  status: $<HTMLDivElement>("status"),
  menu: $<HTMLDivElement>("menu"),
};

const ICONS = {
  back: '<svg viewBox="0 0 24 24"><path d="M15 5l-7 7 7 7"/></svg>',
  up: '<svg viewBox="0 0 24 24"><path d="M5 15l7-7 7 7"/></svg>',
  rescan: '<svg viewBox="0 0 24 24"><path d="M20 12a8 8 0 1 1-2.3-5.6M20 4v5h-5"/></svg>',
  summary: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="8"/><path d="M12 4v8l6 5"/></svg>',
  fixed: '<svg viewBox="0 0 24 24"><rect x="3" y="6" width="18" height="12" rx="2"/><circle cx="17" cy="12" r="1.2"/></svg>',
  removable: '<svg viewBox="0 0 24 24"><path d="M8 3h8v6l2 2v10H6V11l2-2z"/><path d="M10 6h1M13 6h1"/></svg>',
  network: '<svg viewBox="0 0 24 24"><rect x="4" y="4" width="16" height="9" rx="2"/><path d="M12 13v4M5 20h14M12 17v3"/></svg>',
  cdrom: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="8"/><circle cx="12" cy="12" r="2"/></svg>',
};

const SUMMARY_KEY = "summaryDrives";
const renderer = new SunburstRenderer(els.canvas);

let drives: Drive[] = [];
/** Drive roots shown in the summary. */
let summaryRoots = new Set<string>();
const progress = new Map<number, { files: number; allocated: number }>();
const scanErrors = new Map<number, string>();

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
  return { kind: "node", drive: d.index, node: 0, treeGen: d.treeGen };
}

function sameView(a: View, b: View): boolean {
  if (a.kind === "summary" || b.kind === "summary") return a.kind === b.kind;
  return a.drive === b.drive && a.node === b.node && a.treeGen === b.treeGen;
}

/** Shows `next`; with `push`, the current view goes onto the Back history. */
async function show(next: View, push = true): Promise<void> {
  const seq = ++layoutSeq;
  const { width, height } = chartSize();
  let result: Layout;
  try {
    result = await api.layout(next, width, height);
  } catch (err) {
    if (isStale(err) && next.kind === "node" && drives[next.drive]) {
      // The drive was rescanned: its old node ids are gone, start from its root.
      return show(driveRootView(drives[next.drive]), push);
    }
    console.error("layout failed", err);
    return;
  }
  if (seq !== layoutSeq) return;
  if (push && !sameView(view, next)) history.push(view);
  view = next;
  layout = result;
  renderer.setLayout(result, next.kind === "summary", drives);
  lastHover = null;
  hideTooltip();
  updateChrome();
}

function goBack() {
  const prev = history.pop();
  if (prev) void show(prev.kind === "summary" ? summaryView() : prev, false);
}

function goUp() {
  if (!layout || view.kind === "summary") return;
  const parent = layout.center.parent;
  void show(parent === null ? summaryView() : { ...view, node: parent });
}

function zoomTo(info: NodeInfo) {
  if (info.drive === null || info.node === null || info.treeGen === null) return;
  if (info.kind !== "dir") return;
  void show({ kind: "node", drive: info.drive, node: info.node, treeGen: info.treeGen });
}

function rescanCurrent() {
  const targets = view.kind === "summary" ? view.drives : [view.drive];
  void startScan(targets);
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
  if (view.kind === "summary") {
    els.crumb.textContent = S.summary;
  } else if (c) {
    els.crumb.textContent = c.path;
  }
  els.empty.hidden = !(layout && layout.segments.length === 0);
  els.empty.textContent = view.kind === "summary" ? S.emptySummary : S.emptyFolder;
  setStatus(c ?? null);
}

function setStatus(info: NodeInfo | null) {
  if (!info) {
    els.status.textContent = "";
    return;
  }
  const parts = [info.kind === "summary" ? S.summary : info.path || nodeTitle(info), formatSize(info.used)];
  if (info.kind === "dir" || info.kind === "summary") parts.push(`${S.files(info.files)}, ${S.folders(info.dirs)}`);
  els.status.textContent = parts.join(" · ");
}

function renderDrives() {
  const active = currentDrive();
  els.drives.replaceChildren(
    ...drives.map((d) => {
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
      btn.title = [d.root, d.label, d.fileSystem, scanErrors.get(d.index)].filter(Boolean).join(" · ");
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
  if (view.kind === "node" && view.drive === d.index && view.node === 0 && !d.scanning) {
    void startScan([d.index]);
    return;
  }
  if (!d.scanned && !d.scanning) void startScan([d.index]);
  void show(driveRootView(d));
}

function openDriveMenu(d: Drive, x: number, y: number) {
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
  ]);
}

interface MenuItem {
  label: string;
  checked?: boolean;
  action: () => void;
}

function openMenu(x: number, y: number, items: MenuItem[]) {
  els.menu.replaceChildren(
    ...items.map((item) => {
      const b = document.createElement("button");
      b.textContent = item.label;
      if (item.checked !== undefined) b.classList.toggle("checked", item.checked);
      b.addEventListener("click", () => {
        closeMenu();
        item.action();
      });
      return b;
    }),
  );
  els.menu.hidden = false;
  const r = els.menu.getBoundingClientRect();
  els.menu.style.left = `${Math.min(x, innerWidth - r.width - 4)}px`;
  els.menu.style.top = `${Math.min(y, innerHeight - r.height - 4)}px`;
}

function closeMenu() {
  els.menu.hidden = true;
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

function onMouseMove(e: MouseEvent) {
  const scheduled = pendingMove !== null;
  pendingMove = { x: e.offsetX, y: e.offsetY, clientX: e.clientX, clientY: e.clientY };
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
    showTooltip(result.info, move.clientX, move.clientY);
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

function showTooltip(info: NodeInfo, clientX: number, clientY: number) {
  const drive = info.drive !== null ? drives[info.drive] : undefined;
  const isDriveRoot = view.kind === "summary" && info.parent === null;
  let title = nodeTitle(info, drive?.scanned ?? true);
  if (isDriveRoot && drive?.label) title += ` ${drive.label}`;
  const whole = layout?.center.allocated ?? 0;

  const lines = [`<div class="tt-title">${escapeHtml(title)}</div>`];
  if (info.kind !== "free" && info.kind !== "unscanned") lines.push(`<div class="tt-path">${escapeHtml(info.path)}</div>`);
  lines.push(`<div class="tt-size"><b>${formatSize(info.used)}</b> · ${formatPercent(info.allocated, whole)}</div>`);
  if (info.free > 0) lines.push(`<div>${S.free} ${formatSize(info.free)}</div>`);
  if (info.kind === "dir") lines.push(`<div>${S.files(info.files)}, ${S.folders(info.dirs)}</div>`);
  if (info.kind === "link") lines.push(`<div class="tt-note">${S.link}</div>`);
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

// ---------- Scan events ----------

async function refreshDrives() {
  drives = await api.listDrives();
}

function viewUsesDrive(v: View, drive: number): boolean {
  return v.kind === "summary" ? v.drives.includes(drive) : v.drive === drive;
}

async function onScanFinished(drive: number, error: string | null) {
  progress.delete(drive);
  if (error) scanErrors.set(drive, error);
  await refreshDrives();
  // Views into the replaced tree no longer exist; drop them from Back.
  const d = drives[drive];
  for (let i = history.length - 1; i >= 0; i--) {
    const v = history[i];
    if (v.kind === "node" && v.drive === drive && d && v.treeGen !== d.treeGen) history.splice(i, 1);
  }
  if (viewUsesDrive(view, drive)) {
    const next = view.kind === "node" && d && view.treeGen !== d.treeGen ? driveRootView(d) : view.kind === "summary" ? summaryView() : view;
    await show(next, false);
  } else {
    updateChrome();
  }
}

// ---------- Setup ----------

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}

function setupButtons() {
  els.back.innerHTML = ICONS.back;
  els.up.innerHTML = ICONS.up;
  els.rescan.innerHTML = ICONS.rescan;
  els.back.title = `${S.back} (Backspace)`;
  els.up.title = `${S.up} (Alt+↑)`;
  els.rescan.title = `${S.rescan} (F5)`;
  els.summary.innerHTML = `<span class="drive-icon">${ICONS.summary}</span><span class="drive-name"><b>${S.summary}</b></span>`;

  els.back.addEventListener("click", goBack);
  els.up.addEventListener("click", goUp);
  els.rescan.addEventListener("click", rescanCurrent);
  els.summary.addEventListener("click", () => void show(summaryView()));

  els.canvas.addEventListener("mousemove", onMouseMove);
  els.canvas.addEventListener("mouseleave", () => {
    pendingMove = null;
    hoverSeq++;
    lastHover = null;
    renderer.setHighlight(null);
    hideTooltip();
    setStatus(layout?.center ?? null);
  });
  els.canvas.addEventListener("click", onChartClick);

  document.addEventListener("contextmenu", (e) => e.preventDefault());
  document.addEventListener("mousedown", (e) => {
    if (!els.menu.contains(e.target as Node)) closeMenu();
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") closeMenu();
    else if (e.key === "Backspace" || (e.altKey && e.key === "ArrowLeft")) goBack();
    else if (e.altKey && e.key === "ArrowUp") goUp();
    else if (e.key === "F5") rescanCurrent();
    else return;
    e.preventDefault();
  });

  let resizeTimer = 0;
  new ResizeObserver(() => {
    const { width, height } = chartSize();
    renderer.resize(width, height);
    clearTimeout(resizeTimer);
    // Re-layout once the size settles; the old picture stays until then.
    resizeTimer = window.setTimeout(() => void show(view.kind === "summary" ? summaryView() : view, false), 60);
  }).observe(els.chart);

  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => renderer.refreshTheme());
}

async function main() {
  setupButtons();
  await api.onScanProgress((p) => {
    progress.set(p.drive, { files: p.files, allocated: p.allocated });
    renderDrives();
  });
  await api.onScanFinished((f) => void onScanFinished(f.drive, f.error));

  await refreshDrives();
  loadSummaryRoots();
  const { width, height } = chartSize();
  renderer.resize(width, height);
  await show(summaryView(), false);
  // As in the original, every summary drive is scanned right away.
  await startScan(drives.filter((d) => summaryRoots.has(d.root) && !d.scanned).map((d) => d.index));
}

void main();
