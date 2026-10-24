//! Treemap layout and hit testing: the focused folder fills a rectangle, split
//! into one rectangle per child with an area proportional to its size, and so on
//! inside every folder ("squarified", so rectangles stay close to squares).
//!
//! Geometry is in pixels from the top left of the chart, `y` pointing down. The
//! top `params.top` pixels are a bar for the focused folder itself (clicking it
//! goes up), like the central disc of the sunburst.

use std::collections::HashSet;
use std::f64::consts::TAU;

use crate::layout::Hit;
use crate::tree::{NodeId, NodeKind, Tree};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreemapParams {
    pub width: f64,
    pub height: f64,
    /// Height of the bar for the focused folder at the top.
    pub top: f64,
    /// Height of a folder's name strip, when the folder is big enough for one.
    pub header: f64,
    /// Gap between a folder's edge and its contents.
    pub padding: f64,
    /// Rectangles narrower or lower than this are not drawn (their space is kept).
    pub min_side: f64,
    /// At most this many rectangles, the largest levels first.
    pub max_rects: usize,
}

impl Default for TreemapParams {
    fn default() -> Self {
        TreemapParams { width: 800.0, height: 600.0, top: 28.0, header: 16.0, padding: 2.0, min_side: 3.0, max_rects: 20_000 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    fn area(&self) -> f64 {
        self.w * self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    /// Index of the tree in `parts` (0 for [`TreemapLayout::compute`]).
    pub part: u32,
    pub node: NodeId,
    /// 1 for the focused folder's children (or a summary's drives).
    pub depth: u32,
    pub rect: Rect,
    /// Index of the parent tile; `None` at depth 1.
    pub parent: Option<usize>,
    /// The share of the whole this tile has, as the angle a sunburst would give
    /// it: lets both charts colour a branch the same way.
    pub start: f64,
    pub sweep: f64,
    /// The tile has a name strip at its top; its contents start below.
    pub has_header: bool,
}

#[derive(Clone, Debug, Default)]
pub struct TreemapLayout {
    params: TreemapParams,
    /// Breadth-first: a parent always comes before its children, depths ascend.
    tiles: Vec<Tile>,
}

impl TreemapLayout {
    /// Lays out the subtree of `root`; folders in `collapsed` are not divided.
    pub fn compute(tree: &Tree, root: NodeId, params: TreemapParams, collapsed: &HashSet<NodeId>) -> Self {
        let mut layout = TreemapLayout { params, tiles: Vec::new() };
        layout.build(&[(tree, root)], false, |_, n| collapsed.contains(&n));
        layout
    }

    /// Several trees side by side (a summary of drives): their roots are the
    /// depth-1 tiles. `collapsed` holds `(part index, node)` pairs.
    pub fn compute_summary(parts: &[(&Tree, NodeId)], params: TreemapParams, collapsed: &HashSet<(u32, NodeId)>) -> Self {
        let mut layout = TreemapLayout { params, tiles: Vec::new() };
        layout.build(parts, true, |p, n| collapsed.contains(&(p, n)));
        layout
    }

    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    pub fn params(&self) -> &TreemapParams {
        &self.params
    }

    /// The area below the top bar.
    pub fn area(&self) -> Rect {
        let p = &self.params;
        Rect { x: 0.0, y: p.top, w: p.width.max(0.0), h: (p.height - p.top).max(0.0) }
    }

    /// The deepest tile under `(x, y)`; the top bar is [`Hit::Center`].
    pub fn hit(&self, x: f64, y: f64) -> Hit {
        if x >= 0.0 && x < self.params.width && y >= 0.0 && y < self.params.top {
            return Hit::Center;
        }
        // Children come after their parents, so the last tile containing the
        // point is the deepest.
        self.tiles.iter().rposition(|t| t.rect.contains(x, y)).map_or(Hit::None, Hit::Segment)
    }

    fn build(&mut self, parts: &[(&Tree, NodeId)], roots_first: bool, is_collapsed: impl Fn(u32, NodeId) -> bool) {
        let area = self.area();
        let p = self.params;
        let mut tiles: Vec<Tile> = Vec::new();
        let first: Vec<(u32, NodeId, u64)> = if roots_first {
            parts.iter().enumerate().map(|(i, &(t, n))| (i as u32, n, t.allocated(n))).collect()
        } else {
            let (tree, root) = parts[0];
            tree.children_by_size(root).into_iter().map(|c| (0, c, tree.allocated(c))).collect()
        };
        let total: u64 = first.iter().map(|i| i.2).sum();
        self.place(&mut tiles, &first, total, area, 1, None, 0.0, TAU);

        let mut i = 0;
        while i < tiles.len() && tiles.len() < p.max_rects {
            let t = tiles[i];
            let tree = parts[t.part as usize].0;
            if tree.kind(t.node) == NodeKind::Dir && !is_collapsed(t.part, t.node) {
                let inner = inner_rect(&t, &p);
                if inner.w >= p.min_side && inner.h >= p.min_side {
                    let kids: Vec<(u32, NodeId, u64)> =
                        tree.children_by_size(t.node).into_iter().map(|c| (t.part, c, tree.allocated(c))).collect();
                    self.place(&mut tiles, &kids, tree.allocated(t.node), inner, t.depth + 1, Some(i), t.start, t.sweep);
                }
            }
            i += 1;
        }
        tiles.truncate(p.max_rects);
        self.tiles = tiles;
    }

    /// Squarifies `items` (largest first) into `rect`, out of `total`; the part of
    /// `total` the items do not cover stays empty.
    #[allow(clippy::too_many_arguments)]
    fn place(
        &self,
        tiles: &mut Vec<Tile>,
        items: &[(u32, NodeId, u64)],
        total: u64,
        rect: Rect,
        depth: u32,
        parent: Option<usize>,
        start: f64,
        sweep: f64,
    ) {
        if total == 0 || rect.area() <= 0.0 {
            return;
        }
        let p = &self.params;
        let scale = rect.area() / total as f64;
        let min_area = p.min_side * p.min_side;
        // Items too small to see go into one block of space at the end.
        let shown = items.iter().take_while(|i| i.2 as f64 * scale >= min_area).count();
        let rest = total - items[..shown].iter().map(|i| i.2).sum::<u64>();
        let mut areas: Vec<f64> = items[..shown].iter().map(|i| i.2 as f64 * scale).collect();
        if rest > 0 {
            areas.push(rest as f64 * scale);
        }
        let mut offset = 0u64;
        for (k, r) in squarify(&areas, rect).into_iter().enumerate().take(shown) {
            let (part, node, size) = items[k];
            let tile = Tile {
                part,
                node,
                depth,
                rect: r,
                parent,
                start: start + sweep * offset as f64 / total as f64,
                sweep: sweep * size as f64 / total as f64,
                has_header: false,
            };
            offset += size;
            if r.w < p.min_side || r.h < p.min_side {
                continue;
            }
            tiles.push(Tile { has_header: r.h >= p.header * 2.5 && r.w >= 40.0, ..tile });
        }
    }
}

/// Where a folder tile's contents go: inside its padding, below its name strip.
fn inner_rect(t: &Tile, p: &TreemapParams) -> Rect {
    let top = if t.has_header { p.header } else { p.padding };
    Rect {
        x: t.rect.x + p.padding,
        y: t.rect.y + top,
        w: (t.rect.w - 2.0 * p.padding).max(0.0),
        h: (t.rect.h - top - p.padding).max(0.0),
    }
}

/// Squarified treemap (Bruls, Huizing, van Wijk): `areas`, largest first and
/// summing to `rect`'s area, become rectangles in the same order. Items are laid
/// in rows along the shorter side, a row growing while that keeps its worst
/// aspect ratio from getting worse.
fn squarify(areas: &[f64], rect: Rect) -> Vec<Rect> {
    let mut out = Vec::with_capacity(areas.len());
    let mut free = rect;
    let mut row: Vec<f64> = Vec::new();
    let worst = |row: &[f64], extra: f64, side: f64| {
        let sum: f64 = row.iter().sum::<f64>() + extra;
        let max = row.iter().copied().fold(extra, f64::max);
        let min = row.iter().copied().fold(if extra > 0.0 { extra } else { f64::INFINITY }, f64::min);
        let s2 = side * side;
        (s2 * max / (sum * sum)).max(sum * sum / (s2 * min))
    };
    for &a in areas {
        let side = free.w.min(free.h);
        if row.is_empty() || worst(&row, a, side) <= worst(&row, 0.0, side) {
            row.push(a);
        } else {
            lay_row(&row, &mut free, &mut out);
            row.clear();
            row.push(a);
        }
    }
    if !row.is_empty() {
        lay_row(&row, &mut free, &mut out);
    }
    out
}

/// Puts `row` along the shorter side of `free` and takes its strip off `free`.
fn lay_row(row: &[f64], free: &mut Rect, out: &mut Vec<Rect>) {
    let sum: f64 = row.iter().sum();
    if free.w >= free.h {
        // A column at the left.
        let w = if free.h > 0.0 { (sum / free.h).min(free.w) } else { 0.0 };
        let mut y = free.y;
        for &a in row {
            let h = if w > 0.0 { a / w } else { 0.0 };
            out.push(Rect { x: free.x, y, w, h });
            y += h;
        }
        free.x += w;
        free.w -= w;
    } else {
        // A row at the top.
        let h = if free.w > 0.0 { (sum / free.w).min(free.h) } else { 0.0 };
        let mut x = free.x;
        for &a in row {
            let w = if h > 0.0 { a / h } else { 0.0 };
            out.push(Rect { x, y: free.y, w, h });
            x += w;
        }
        free.y += h;
        free.h -= h;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-6;

    fn params() -> TreemapParams {
        TreemapParams { width: 400.0, height: 328.0, top: 28.0, ..TreemapParams::default() }
    }

    fn sample() -> Tree {
        let mut t = Tree::new("D:\\");
        let a = t.add_dir(NodeId::ROOT, "a");
        t.add_file(a, "a1", 0, 3000);
        t.add_file(a, "a2", 0, 1000);
        let b = t.add_dir(NodeId::ROOT, "b");
        t.add_file(b, "b1", 0, 2000);
        t.add_file(NodeId::ROOT, "c", 0, 1500);
        t.add_file(NodeId::ROOT, "d", 0, 500);
        t
    }

    fn inside(inner: &Rect, outer: &Rect) -> bool {
        inner.x >= outer.x - EPS
            && inner.y >= outer.y - EPS
            && inner.x + inner.w <= outer.x + outer.w + EPS
            && inner.y + inner.h <= outer.y + outer.h + EPS
    }

    fn overlap(a: &Rect, b: &Rect) -> bool {
        a.x + EPS < b.x + b.w && b.x + EPS < a.x + a.w && a.y + EPS < b.y + b.h && b.y + EPS < a.y + a.h
    }

    #[test]
    fn squarify_fills_the_rectangle_in_proportion() {
        let rect = Rect { x: 10.0, y: 20.0, w: 600.0, h: 400.0 };
        let areas = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0].map(|a| a * 240_000.0 / 24.0);
        let rects = squarify(&areas, rect);
        assert_eq!(rects.len(), areas.len());
        for (r, a) in rects.iter().zip(areas) {
            assert!((r.area() - a).abs() < 1e-3, "{r:?} should have area {a}");
            assert!(inside(r, &rect), "{r:?}");
            assert!(r.w.max(r.h) / r.w.min(r.h) < 3.0, "too thin: {r:?}");
        }
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                assert!(!overlap(&rects[i], &rects[j]), "{i} and {j} overlap");
            }
        }
    }

    #[test]
    fn children_nest_inside_parents_without_overlap() {
        let t = sample();
        let l = TreemapLayout::compute(&t, NodeId::ROOT, params(), &HashSet::new());
        let tiles = l.tiles();
        let area = l.area();
        // Depth 1: a (4000), b (2000), c (1500), d (500), total 8000.
        let top: Vec<&Tile> = tiles.iter().filter(|x| x.depth == 1).collect();
        assert_eq!(top.len(), 4);
        for x in &top {
            let share = x.rect.area() / area.area();
            assert!((share - t.allocated(x.node) as f64 / 8000.0).abs() < 1e-9);
            assert!(inside(&x.rect, &area));
        }
        for x in tiles.iter().filter(|x| x.depth > 1) {
            let parent = &tiles[x.parent.unwrap()];
            assert!(inside(&x.rect, &inner_rect(parent, l.params())));
            assert_eq!(t.parent(x.node), Some(parent.node));
        }
        for i in 0..tiles.len() {
            for j in i + 1..tiles.len() {
                if tiles[i].parent == tiles[j].parent {
                    assert!(!overlap(&tiles[i].rect, &tiles[j].rect));
                }
            }
        }
        // Breadth-first: parents first.
        for (i, x) in tiles.iter().enumerate() {
            assert!(x.parent.is_none_or(|p| p < i));
        }
    }

    #[test]
    fn shares_match_the_sunburst_angles() {
        let t = sample();
        let l = TreemapLayout::compute(&t, NodeId::ROOT, params(), &HashSet::new());
        let a = l.tiles().iter().find(|x| t.name(x.node) == "a").unwrap();
        assert!((a.start - 0.0).abs() < EPS && (a.sweep - TAU * 0.5).abs() < EPS);
        let a2 = l.tiles().iter().find(|x| t.name(x.node) == "a2").unwrap();
        assert!((a2.start - TAU * 3000.0 / 8000.0).abs() < EPS, "inside a, after a1");
    }

    #[test]
    fn hit_finds_the_deepest_tile_and_the_top_bar() {
        let t = sample();
        let l = TreemapLayout::compute(&t, NodeId::ROOT, params(), &HashSet::new());
        assert_eq!(l.hit(5.0, 5.0), Hit::Center);
        assert_eq!(l.hit(-1.0, 100.0), Hit::None);
        for (i, x) in l.tiles().iter().enumerate() {
            let (cx, cy) = (x.rect.x + x.rect.w / 2.0, x.rect.y + x.rect.h - 1.0);
            let Hit::Segment(found) = l.hit(cx, cy) else { panic!("nothing at {cx},{cy}") };
            // The hit is this tile or one inside it.
            let mut j = found;
            while j != i {
                j = l.tiles()[j].parent.unwrap_or_else(|| panic!("{found} is not inside {i}"));
            }
        }
    }

    #[test]
    fn collapsed_folders_and_tiny_items_are_not_divided() {
        let mut t = sample();
        let many = t.add_dir(NodeId::ROOT, "many");
        t.add_file(many, "big", 0, 100_000);
        for i in 0..1000 {
            t.add_file(many, &format!("tiny{i}"), 0, 1);
        }
        let l = TreemapLayout::compute(&t, NodeId::ROOT, params(), &HashSet::new());
        assert!(l.tiles().iter().all(|x| x.rect.w >= 3.0 && x.rect.h >= 3.0));
        assert!(l.tiles().iter().all(|x| !t.name(x.node).starts_with("tiny")));
        let a = t.find_path(&["a"]).0;
        let l = TreemapLayout::compute(&t, NodeId::ROOT, params(), &HashSet::from([a]));
        assert!(l.tiles().iter().all(|x| t.parent(x.node) != Some(a)));
    }

    #[test]
    fn summary_puts_each_tree_in_its_own_tile() {
        let (t1, t2) = (sample(), sample());
        let l = TreemapLayout::compute_summary(&[(&t1, NodeId::ROOT), (&t2, NodeId::ROOT)], params(), &HashSet::new());
        let roots: Vec<&Tile> = l.tiles().iter().filter(|x| x.depth == 1).collect();
        assert_eq!(roots.len(), 2);
        assert!(roots.iter().all(|x| x.node == NodeId::ROOT));
        assert!((roots[0].rect.area() - roots[1].rect.area()).abs() < 1e-6);
        assert!(l.tiles().iter().any(|x| x.part == 1 && x.depth == 3));
    }

    #[test]
    fn empty_and_tiny_canvases_do_not_panic() {
        let empty = Tree::new("E:\\");
        assert!(TreemapLayout::compute(&empty, NodeId::ROOT, params(), &HashSet::new()).tiles().is_empty());
        let t = sample();
        let tiny = TreemapParams { width: 2.0, height: 2.0, ..params() };
        let l = TreemapLayout::compute(&t, NodeId::ROOT, tiny, &HashSet::new());
        assert!(l.tiles().is_empty());
        let capped = TreemapParams { max_rects: 3, ..params() };
        assert!(TreemapLayout::compute(&t, NodeId::ROOT, capped, &HashSet::new()).tiles().len() <= 3);
    }
}
