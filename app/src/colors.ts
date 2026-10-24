// Segment colours: each branch of the focused folder gets a hue from its angle,
// and its descendants keep that hue and get lighter with depth.

import type { Segment } from "./api";

const TAU = Math.PI * 2;
/** Rotates the colour wheel so the first (largest) branch starts at blue. */
const HUE_OFFSET = 205;

export interface Palette {
  free: string;
  unscanned: string;
  drive: string;
  dark: boolean;
}

export function readPalette(): Palette {
  const css = getComputedStyle(document.documentElement);
  const v = (name: string) => css.getPropertyValue(name).trim();
  return {
    free: v("--seg-free"),
    unscanned: v("--seg-unscanned"),
    drive: v("--seg-drive"),
    dark: matchMedia("(prefers-color-scheme: dark)").matches,
  };
}

/**
 * One CSS colour per segment. In a summary, ring 1 holds the drives, which are
 * drawn neutral; branches start in ring 2.
 */
export function segmentColors(segments: Segment[], summary: boolean, p: Palette): string[] {
  const branchDepth = summary ? 2 : 1;
  const hues = new Array<number>(segments.length);
  return segments.map((s, i) => {
    if (s.kind === "free") return p.free;
    if (s.kind === "unscanned") return p.unscanned;
    if (s.depth < branchDepth) return p.drive;

    const hue =
      s.depth === branchDepth || s.parent < 0
        ? ((s.start + s.sweep / 2) / TAU) * 360 + HUE_OFFSET
        : hues[s.parent];
    hues[i] = hue;

    const level = s.depth - branchDepth;
    let light = (p.dark ? 44 : 52) + level * 6;
    let sat = 62 - level * 3;
    if (s.kind === "file") {
      light += 6;
      sat -= 12;
    } else if (s.kind === "link") {
      sat = 8;
    }
    return `hsl(${hue % 360} ${Math.max(sat, 20)}% ${Math.min(light, p.dark ? 72 : 84)}%)`;
  });
}
