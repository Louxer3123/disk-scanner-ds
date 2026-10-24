//! Sunburst layout and hit testing.
//!
//! The focused node is the central disc; its children form ring 1, their children
//! ring 2, and so on. Each segment's angle is proportional to its allocated size.
//!
//! Geometry conventions, shared with the UI:
//! - coordinates are in pixels relative to the centre, with `y` pointing **down**
//!   (screen coordinates);
//! - angles are in radians in `[0, TAU)`, measured **clockwise from 12 o'clock**;
//! - ring `d` (1-based) spans radii `center_radius + (d - 1) * ring_width` to
//!   `center_radius + d * ring_width`.

use std::collections::HashSet;
use std::f64::consts::TAU;

use crate::tree::{NodeId, NodeKind, Tree};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutParams {
    /// Radius of the central disc that stands for the focused folder.
    pub center_radius: f64,
    pub ring_width: f64,
    /// Outer edge of the outermost ring; only whole rings below it are laid out.
    pub max_radius: f64,
    /// Segments whose outer arc is shorter than this are dropped, together with
    /// their subtrees.
    pub min_arc: f64,
    /// Size of the whole circle. Defaults to the focused node's size; set it to the
    /// drive capacity to leave the rest of ring 1 as free space.
    pub total: Option<u64>,
}

impl Default for LayoutParams {
    fn default() -> Self {
        LayoutParams {
            center_radius: 60.0,
            ring_width: 40.0,
            max_radius: 300.0,
            min_arc: 2.0,
            total: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub node: NodeId,
    /// Ring number, 1 for children of the focused node.
    pub depth: u32,
    pub start: f64,
    pub sweep: f64,
}

impl Segment {
    pub fn end(&self) -> f64 {
        self.start + self.sweep
    }

    pub fn mid_angle(&self) -> f64 {
        self.start + self.sweep / 2.0
    }
}

/// What lies under a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The central disc, i.e. the focused node itself.
    Center,
    /// Index into [`SunburstLayout::segments`].
    Segment(usize),
    /// The free-space part of ring 1, when [`LayoutParams::total`] exceeds the used size.
    Free,
    /// A gap (dropped small segments, empty space) or outside the chart.
    None,
}

#[derive(Clone, Debug)]
pub struct SunburstLayout {
    root: NodeId,
    params: LayoutParams,
    rings: u32,
    /// Ordered by depth, then by start angle.
    segments: Vec<Segment>,
    /// `ring_starts[d - 1]..ring_starts[d]` are the segments of ring `d`; `rings + 1` entries.
    ring_starts: Vec<usize>,
    used_sweep: f64,
}

impl SunburstLayout {
    /// Lays out the subtree of `root`. Children of nodes in `collapsed` are not shown.
    /// Within each parent, children go clockwise from largest to smallest.
    pub fn compute(tree: &Tree, root: NodeId, params: LayoutParams, collapsed: &HashSet<NodeId>) -> Self {
        let rings = ((params.max_radius - params.center_radius) / params.ring_width)
            .floor()
            .max(0.0) as u32;
        let used = tree.allocated(root);
        let total = params.total.unwrap_or(used).max(used);
        // An empty folder with no capacity given has no free space either.
        let used_sweep = if total == 0 { TAU } else { TAU * used as f64 / total as f64 };

        let mut layout = SunburstLayout {
            root,
            params,
            rings,
            segments: Vec::new(),
            ring_starts: Vec::new(),
            used_sweep,
        };
        if rings == 0 || used == 0 {
            layout.ring_starts = vec![0; rings as usize + 1];
            return layout;
        }

        // Breadth-first, using `segments` itself as the queue: parents of a ring are
        // visited in angle order and children are packed inside their parent's arc,
        // so every ring comes out sorted by start angle.
        let outer = |depth: u32| params.center_radius + f64::from(depth) * params.ring_width;
        let place_children =|segments: &mut Vec<Segment>, parent: NodeId, start: f64, sweep: f64, depth: u32| {
            let parent_size = tree.allocated(parent) as f64;
            let min_sweep = params.min_arc / outer(depth);
            let mut offset = 0u64;
            for child in tree.children_by_size(parent) {
                let size = tree.allocated(child);
                let child_sweep = sweep * size as f64 / parent_size;
                // Children are sorted by size, so all the rest are smaller still.
                if size == 0 || child_sweep < min_sweep {
                    break;
                }
                segments.push(Segment {
                    node: child,
                    depth,
                    start: start + sweep * offset as f64 / parent_size,
                    sweep: child_sweep,
                });
                offset += size;
            }
        };

        let mut segments = Vec::new();
        place_children(&mut segments, root, 0.0, used_sweep, 1);
        let mut i = 0;
        while i < segments.len() {
            let s = segments[i];
            if s.depth < rings && tree.kind(s.node) == NodeKind::Dir && !collapsed.contains(&s.node) {
                place_children(&mut segments, s.node, s.start, s.sweep, s.depth + 1);
            }
            i += 1;
        }

        layout.ring_starts = (1..=rings + 1).map(|d| segments.partition_point(|s| s.depth < d)).collect();
        layout.segments = segments;
        layout
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    pub fn params(&self) -> &LayoutParams {
        &self.params
    }

    /// Number of rings that fit between the central disc and `max_radius`.
    pub fn ring_count(&self) -> u32 {
        self.rings
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Segments of ring `depth` (1-based), sorted by start angle. Empty if out of range.
    pub fn ring(&self, depth: u32) -> &[Segment] {
        match self.ring_range(depth) {
            Some((a, b)) => &self.segments[a..b],
            None => &[],
        }
    }

    /// Inner and outer radius of ring `depth` (1-based).
    pub fn ring_radii(&self, depth: u32) -> (f64, f64) {
        let p = &self.params;
        let inner = p.center_radius + f64::from(depth.saturating_sub(1)) * p.ring_width;
        (inner, inner + p.ring_width)
    }

    /// Angle taken by the focused node; the rest of the circle is free space.
    pub fn used_sweep(&self) -> f64 {
        self.used_sweep
    }

    pub fn free_sweep(&self) -> f64 {
        TAU - self.used_sweep
    }

    /// Finds what is drawn at `(x, y)`, relative to the centre, in screen coordinates.
    pub fn hit(&self, x: f64, y: f64) -> Hit {
        let p = &self.params;
        let r = x.hypot(y);
        if r < p.center_radius {
            return Hit::Center;
        }
        let depth = ((r - p.center_radius) / p.ring_width).floor() as u32 + 1;
        let Some((first, last)) = self.ring_range(depth) else {
            return Hit::None;
        };
        let angle = angle_at(x, y);
        let ring = &self.segments[first..last];
        let idx = ring.partition_point(|s| s.start <= angle);
        if idx > 0 && angle < ring[idx - 1].end() {
            return Hit::Segment(first + idx - 1);
        }
        if depth == 1 && angle >= self.used_sweep && self.used_sweep < TAU {
            return Hit::Free;
        }
        Hit::None
    }

    fn ring_range(&self, depth: u32) -> Option<(usize, usize)> {
        if depth == 0 || depth > self.rings {
            return None;
        }
        let d = depth as usize;
        Some((self.ring_starts[d - 1], self.ring_starts[d]))
    }
}

/// Angle of the point `(x, y)` (screen coordinates, `y` down), clockwise from
/// 12 o'clock, in `[0, TAU)`.
pub fn angle_at(x: f64, y: f64) -> f64 {
    let a = x.atan2(-y);
    if a < 0.0 { a + TAU } else { a }
}

/// Point at `angle` (as in [`angle_at`]) and `radius` from the centre.
pub fn point_at(angle: f64, radius: f64) -> (f64, f64) {
    (radius * angle.sin(), -radius * angle.cos())
}
