// Draws a layout from the backend onto a canvas. The static picture is cached in an
// offscreen canvas; hover highlights are composed on top without redrawing it.

import type { Drive, Layout, Segment } from "./api";
import { segmentColors, readPalette } from "./colors";
import { formatSize } from "./format";
import { S, nodeTitle } from "./strings";

const HALF_PI = Math.PI / 2;
const LABEL_FONT_PX = 12;

export type Highlight = { kind: "segment"; index: number } | { kind: "center" } | null;

export class SunburstRenderer {
  private layout: Layout | null = null;
  private summary = false;
  private drives: Drive[] = [];
  private base = document.createElement("canvas");
  private highlight: Highlight = null;
  private cssW = 0;
  private cssH = 0;
  private dpr = 1;

  constructor(private canvas: HTMLCanvasElement) {}

  /** Chart centre in CSS pixels relative to the canvas. */
  get center(): { x: number; y: number } {
    return { x: this.cssW / 2, y: this.cssH / 2 };
  }

  resize(width: number, height: number) {
    this.dpr = window.devicePixelRatio || 1;
    this.cssW = width;
    this.cssH = height;
    for (const c of [this.canvas, this.base]) {
      c.width = Math.round(width * this.dpr);
      c.height = Math.round(height * this.dpr);
    }
    this.canvas.style.width = `${width}px`;
    this.canvas.style.height = `${height}px`;
  }

  setLayout(layout: Layout, summary: boolean, drives: Drive[]) {
    this.layout = layout;
    this.summary = summary;
    this.drives = drives;
    this.highlight = null;
    this.drawBase();
    this.compose();
  }

  /** Redraws with the current theme colours. */
  refreshTheme() {
    if (!this.layout) return;
    this.drawBase();
    this.compose();
  }

  setHighlight(h: Highlight) {
    const same =
      h === this.highlight ||
      (h && this.highlight && h.kind === this.highlight.kind &&
        (h.kind === "center" || (this.highlight.kind === "segment" && h.index === this.highlight.index)));
    if (same) return;
    this.highlight = h;
    this.compose();
  }

  private drawBase() {
    const ctx = this.base.getContext("2d")!;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, this.base.width, this.base.height);
    const layout = this.layout;
    if (!layout) return;
    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);

    const palette = readPalette();
    const css = getComputedStyle(document.documentElement);
    const bg = css.getPropertyValue("--chart-bg").trim();
    const colors = segmentColors(layout.segments, this.summary, palette);
    const { x: cx, y: cy } = this.center;

    layout.segments.forEach((s, i) => {
      const [inner, outer] = this.radii(s.depth);
      sectorPath(ctx, cx, cy, inner, outer, s.start, s.sweep);
      ctx.fillStyle = colors[i];
      ctx.fill();
    });
    // Thin separators in the background colour keep neighbours apart.
    ctx.strokeStyle = bg;
    ctx.lineWidth = 1;
    for (const s of layout.segments) {
      const [inner, outer] = this.radii(s.depth);
      sectorPath(ctx, cx, cy, inner, outer, s.start, s.sweep);
      ctx.stroke();
    }

    ctx.font = `${LABEL_FONT_PX}px ${css.getPropertyValue("--font").trim()}`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillStyle = css.getPropertyValue("--seg-text").trim();
    if (layout.ringWidth >= LABEL_FONT_PX + 4) {
      for (const s of layout.segments) this.drawLabel(ctx, s);
    }

    this.drawCenter(ctx, css);
  }

  private drawLabel(ctx: CanvasRenderingContext2D, s: Segment) {
    const [inner, outer] = this.radii(s.depth);
    const r = (inner + outer) / 2;
    const room = s.sweep * r - 8;
    if (room < 24) return;
    const text = fitText(ctx, this.segmentName(s), room);
    if (!text) return;
    const mid = s.start + s.sweep / 2;
    const { x: cx, y: cy } = this.center;
    ctx.save();
    ctx.translate(cx + r * Math.sin(mid), cy - r * Math.cos(mid));
    // Text follows the arc and is flipped on the lower half so it never reads upside down.
    ctx.rotate(mid > HALF_PI && mid < 3 * HALF_PI ? mid - Math.PI : mid);
    ctx.fillText(text, 0, 0);
    ctx.restore();
  }

  private segmentName(s: Segment): string {
    const drive = this.drives[s.drive];
    if (this.summary && s.depth === 1 && drive) {
      return drive.label ? `${drive.root.slice(0, 2)} ${drive.label}` : drive.root.slice(0, 2);
    }
    return nodeTitle({ kind: s.kind, name: s.name, parent: 0 }, drive?.scanned ?? true);
  }

  private drawCenter(ctx: CanvasRenderingContext2D, css: CSSStyleDeclaration) {
    const layout = this.layout!;
    const { x: cx, y: cy } = this.center;
    const r = layout.centerRadius;
    ctx.beginPath();
    ctx.arc(cx, cy, r - 1, 0, Math.PI * 2);
    ctx.fillStyle = css.getPropertyValue("--center-bg").trim();
    ctx.fill();

    const c = layout.center;
    const drive = c.drive !== null ? this.drives[c.drive] : undefined;
    let title = nodeTitle(c);
    if (c.parent === null && drive?.label) title = `${title} ${drive.label}`;
    const lines: [string, number, string][] = [
      [title, Math.max(12, Math.min(18, r / 5)), "600"],
      [formatSize(c.used), Math.max(11, Math.min(15, r / 6)), "400"],
    ];
    if (c.free > 0) lines.push([`${S.free} ${formatSize(c.free)}`, Math.max(10, Math.min(13, r / 7)), "400"]);

    const family = css.getPropertyValue("--font").trim();
    const height = lines.reduce((h, [, size]) => h + size * 1.3, 0);
    let y = cy - height / 2;
    lines.forEach(([text, size, weight], i) => {
      ctx.font = `${weight} ${size}px ${family}`;
      ctx.fillStyle = css.getPropertyValue(i === 0 ? "--text" : "--text-dim").trim();
      y += (size * 1.3) / 2;
      ctx.fillText(fitText(ctx, text, r * 1.7) ?? "", cx, y);
      y += (size * 1.3) / 2;
    });
  }

  private compose() {
    const ctx = this.canvas.getContext("2d")!;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
    ctx.drawImage(this.base, 0, 0);
    const layout = this.layout;
    const h = this.highlight;
    if (!layout || !h) return;

    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    const css = getComputedStyle(document.documentElement);
    const { x: cx, y: cy } = this.center;
    ctx.lineWidth = 2;
    ctx.strokeStyle = css.getPropertyValue("--accent").trim();
    if (h.kind === "center") {
      ctx.beginPath();
      ctx.arc(cx, cy, layout.centerRadius - 2, 0, Math.PI * 2);
      ctx.stroke();
      return;
    }
    const s = layout.segments[h.index];
    if (!s) return;
    const [inner, outer] = this.radii(s.depth);
    sectorPath(ctx, cx, cy, inner, outer, s.start, s.sweep);
    ctx.fillStyle = css.getPropertyValue("--hover-overlay").trim();
    ctx.fill();
    ctx.stroke();
  }

  private radii(depth: number): [number, number] {
    const l = this.layout!;
    const inner = l.centerRadius + (depth - 1) * l.ringWidth;
    return [inner, inner + l.ringWidth];
  }
}

/** Annular sector; angles are clockwise from 12 o'clock as in the backend. */
function sectorPath(
  ctx: CanvasRenderingContext2D,
  cx: number,
  cy: number,
  inner: number,
  outer: number,
  start: number,
  sweep: number,
) {
  // Canvas angles start at 3 o'clock, also clockwise on screen.
  const a0 = start - HALF_PI;
  const a1 = a0 + sweep;
  ctx.beginPath();
  if (sweep >= Math.PI * 2 - 1e-9) {
    ctx.arc(cx, cy, outer, 0, Math.PI * 2);
    ctx.moveTo(cx + inner, cy);
    ctx.arc(cx, cy, inner, 0, Math.PI * 2, true);
  } else {
    ctx.arc(cx, cy, outer, a0, a1);
    ctx.arc(cx, cy, inner, a1, a0, true);
  }
  ctx.closePath();
}

/** The text, shortened with an ellipsis to fit `width`, or null if too little fits. */
function fitText(ctx: CanvasRenderingContext2D, text: string, width: number): string | null {
  if (ctx.measureText(text).width <= width) return text;
  let lo = 0;
  let hi = text.length;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (ctx.measureText(text.slice(0, mid) + "…").width <= width) lo = mid;
    else hi = mid - 1;
  }
  return lo >= 3 ? text.slice(0, lo) + "…" : null;
}
