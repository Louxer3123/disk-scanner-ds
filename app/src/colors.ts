// Segment colours: each branch of the focused folder gets a hue from its angle,
// and its descendants keep that hue and get lighter with depth. Coloured by file
// type instead, the hue comes from the category of the file or of most of the
// space in the folder.

import type { Segment } from "./api";

const TAU = Math.PI * 2;
/** Rotates the colour wheel so the first (largest) branch starts at blue. */
const HUE_OFFSET = 205;

/**
 * Hue per file category, indexed like `Category` in scanner-core's filetype.rs:
 * other, video, audio, images, documents, archives, programs, code, disk images,
 * data. "Other" (-1) is grey.
 */
const CATEGORY_HUES = [-1, 350, 270, 25, 215, 48, 155, 100, 185, 315];
export const CATEGORY_COUNT = CATEGORY_HUES.length;

/** Colour of a file category; `level` steps lighter, as rings go outwards. */
export function categoryColor(category: number, dark: boolean, level = 0): string {
  const hue = CATEGORY_HUES[category] ?? -1;
  const light = Math.min((dark ? 46 : 54) + level * 6, dark ? 72 : 84);
  if (hue < 0) return `hsl(220 8% ${light}%)`;
  return `hsl(${hue} ${Math.max(68 - level * 3, 30)}% ${light}%)`;
}

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
export function segmentColors(
  segments: Segment[],
  summary: boolean,
  p: Palette,
  categories: number[] | null = null,
): string[] {
  const branchDepth = summary ? 2 : 1;
  const hues = new Array<number>(segments.length);
  return segments.map((s, i) => {
    if (s.kind === "free") return p.free;
    if (s.kind === "unscanned") return p.unscanned;
    if (s.depth < branchDepth) return p.drive;
    if (categories) {
      return s.kind === "link"
        ? categoryColor(0, p.dark, 2)
        : categoryColor(categories[i], p.dark, s.depth - branchDepth);
    }

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
