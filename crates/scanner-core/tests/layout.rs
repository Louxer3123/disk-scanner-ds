use std::collections::HashSet;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

use scanner_core::layout::{angle_at, point_at};
use scanner_core::{Hit, LayoutParams, NodeId, NodeKind, Segment, SunburstLayout, Tree};

const EPS: f64 = 1e-9;

/// Centre disc 50 px, rings of 50 px, 4 rings up to 250 px, no pruning.
fn params() -> LayoutParams {
    LayoutParams {
        center_radius: 50.0,
        ring_width: 50.0,
        max_radius: 250.0,
        min_arc: 0.0,
        total: None,
    }
}

fn layout(tree: &Tree, p: LayoutParams) -> SunburstLayout {
    SunburstLayout::compute(tree, NodeId::ROOT, p, &HashSet::new())
}

fn find(l: &SunburstLayout, tree: &Tree, name: &str) -> Segment {
    *l.segments()
        .iter()
        .find(|s| tree.name(s.node) == name)
        .unwrap_or_else(|| panic!("no segment {name:?}"))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < EPS
}

/// root
///   big/        (300)
///     x  200
///     y  100
///   small  100
fn three_to_one() -> Tree {
    let mut t = Tree::new("root");
    let big = t.add_dir(NodeId::ROOT, "big");
    t.add_file(big, "x", 200, 200);
    t.add_file(big, "y", 100, 100);
    t.add_file(NodeId::ROOT, "small", 100, 100);
    t
}

/// Point in the middle of a segment, in screen coordinates.
fn centre_of(l: &SunburstLayout, s: &Segment) -> (f64, f64) {
    let (inner, outer) = l.ring_radii(s.depth);
    point_at(s.mid_angle(), (inner + outer) / 2.0)
}

#[test]
fn angles_are_clockwise_from_twelve_o_clock() {
    assert!(close(angle_at(0.0, -1.0), 0.0), "top");
    assert!(close(angle_at(1.0, 0.0), FRAC_PI_2), "right");
    assert!(close(angle_at(0.0, 1.0), PI), "bottom");
    assert!(close(angle_at(-1.0, 0.0), 3.0 * FRAC_PI_2), "left");
    let just_left_of_top = angle_at(-1e-6, -1.0);
    assert!(just_left_of_top < TAU && just_left_of_top > TAU - 1e-5);
}

#[test]
fn point_at_inverts_angle_at() {
    for i in 0..64 {
        let a = TAU * f64::from(i) / 64.0;
        let (x, y) = point_at(a, 123.0);
        assert!(close(x.hypot(y), 123.0));
        assert!(close(angle_at(x, y), a) || close(angle_at(x, y) + TAU, a + TAU), "angle {a}");
    }
}

#[test]
fn sweeps_are_proportional_and_largest_first() {
    let t = three_to_one();
    let l = layout(&t, params());

    let big = find(&l, &t, "big");
    let small = find(&l, &t, "small");
    assert_eq!((big.depth, small.depth), (1, 1));
    assert!(close(big.start, 0.0));
    assert!(close(big.sweep, 3.0 * FRAC_PI_2));
    assert!(close(small.start, big.end()), "siblings must be contiguous");
    assert!(close(small.end(), TAU), "ring 1 must close the circle");

    let x = find(&l, &t, "x");
    let y = find(&l, &t, "y");
    assert_eq!((x.depth, y.depth), (2, 2));
    assert!(close(x.start, big.start) && close(x.sweep, PI));
    assert!(close(y.start, x.end()) && close(y.end(), big.end()), "children fill the parent arc");
}

#[test]
fn rings_are_sorted_by_depth_then_angle() {
    let t = three_to_one();
    let l = layout(&t, params());
    assert_eq!(l.ring_count(), 4);
    assert_eq!(l.ring(1).len(), 2);
    assert_eq!(l.ring(2).len(), 2);
    assert!(l.ring(3).is_empty() && l.ring(0).is_empty() && l.ring(99).is_empty());
    for d in 1..=l.ring_count() {
        let ring = l.ring(d);
        assert!(ring.iter().all(|s| s.depth == d));
        assert!(ring.windows(2).all(|w| w[0].start <= w[1].start));
    }
    assert_eq!(l.ring_radii(1), (50.0, 100.0));
    assert_eq!(l.ring_radii(4), (200.0, 250.0));
}

#[test]
fn hit_test_finds_center_segments_and_outside() {
    let t = three_to_one();
    let l = layout(&t, params());
    let name_at = |x, y| match l.hit(x, y) {
        Hit::Segment(i) => t.name(l.segments()[i].node).to_owned(),
        other => format!("{other:?}"),
    };

    assert_eq!(l.hit(0.0, 0.0), Hit::Center);
    assert_eq!(l.hit(49.9, 0.0), Hit::Center);
    // Ring 1: `big` spans 12 → 9 o'clock clockwise, `small` 9 → 12.
    assert_eq!(name_at(75.0, 0.0), "big", "3 o'clock");
    assert_eq!(name_at(0.0, 75.0), "big", "6 o'clock");
    assert_eq!(name_at(-75.0, -10.0), "small", "just above 9 o'clock");
    // Ring 2: `x` is the right half, `y` the lower-left quarter.
    assert_eq!(name_at(125.0, 0.0), "x");
    assert_eq!(name_at(-125.0, 10.0), "y");
    // Ring 2 above `small`: files have no children, so this is a gap.
    assert_eq!(l.hit(-125.0, -10.0), Hit::None);
    // Beyond the last ring.
    assert_eq!(l.hit(0.0, -250.0), Hit::None);
    assert_eq!(l.hit(1000.0, 1000.0), Hit::None);
}

#[test]
fn hit_test_at_ring_boundaries() {
    let t = three_to_one();
    let l = layout(&t, params());
    // Exactly on a radius boundary belongs to the outer ring.
    assert_eq!(l.hit(50.0, 0.0), l.hit(51.0, 0.0));
    assert_eq!(l.hit(100.0, 0.0), l.hit(101.0, 0.0));
    // Exactly at 12 o'clock is the start of the first segment.
    match l.hit(0.0, -75.0) {
        Hit::Segment(i) => assert_eq!(t.name(l.segments()[i].node), "big"),
        other => panic!("expected big, got {other:?}"),
    }
}

#[test]
fn small_segments_are_pruned_with_their_subtrees() {
    let mut t = Tree::new("root");
    t.add_file(NodeId::ROOT, "huge", 1_000_000, 1_000_000);
    let tiny = t.add_dir(NodeId::ROOT, "tiny");
    t.add_file(tiny, "inside", 1, 1);
    t.add_file(NodeId::ROOT, "zero", 0, 0);

    // tiny's arc at 100 px: 100 * TAU / 1e6 ≈ 0.0006 px.
    let l = layout(&t, LayoutParams { min_arc: 1.0, ..params() });
    let names: Vec<&str> = l.segments().iter().map(|s| t.name(s.node)).collect();
    assert_eq!(names, ["huge"]);

    // Without pruning, zero-sized entries are still left out.
    let l = layout(&t, params());
    let names: Vec<&str> = l.segments().iter().map(|s| t.name(s.node)).collect();
    assert_eq!(names, ["huge", "tiny", "inside"]);
}

#[test]
fn pruning_uses_the_outer_radius_of_the_ring() {
    // Ten equal children: each gets TAU/10 ≈ 0.628 rad, so 62.8 px at the 100 px
    // outer edge of ring 1.
    let mut t = Tree::new("root");
    for i in 0..10 {
        t.add_file(NodeId::ROOT, &format!("f{i}"), 10, 10);
    }
    assert_eq!(layout(&t, LayoutParams { min_arc: 62.0, ..params() }).segments().len(), 10);
    assert_eq!(layout(&t, LayoutParams { min_arc: 63.0, ..params() }).segments().len(), 0);
}

#[test]
fn depth_is_limited_by_radius() {
    let mut t = Tree::new("root");
    let mut parent = NodeId::ROOT;
    for i in 0..10 {
        parent = t.add_dir(parent, &format!("d{i}"));
    }
    t.add_file(parent, "leaf", 10, 10);

    let l = layout(&t, params());
    assert_eq!(l.ring_count(), 4);
    assert_eq!(l.segments().len(), 4);
    assert_eq!(l.segments().last().unwrap().depth, 4);

    // A partial ring does not count.
    let l = layout(&t, LayoutParams { max_radius: 249.0, ..params() });
    assert_eq!(l.ring_count(), 3);

    let l = layout(&t, LayoutParams { max_radius: 40.0, ..params() });
    assert_eq!(l.ring_count(), 0);
    assert!(l.segments().is_empty());
    assert_eq!(l.hit(45.0, 0.0), Hit::Center);
    assert_eq!(l.hit(60.0, 0.0), Hit::None);
}

#[test]
fn collapsed_folders_hide_their_contents() {
    let t = three_to_one();
    let big = t.children(NodeId::ROOT).find(|&c| t.name(c) == "big").unwrap();
    let l = SunburstLayout::compute(&t, NodeId::ROOT, params(), &HashSet::from([big]));
    let names: Vec<&str> = l.segments().iter().map(|s| t.name(s.node)).collect();
    assert_eq!(names, ["big", "small"]);
}

#[test]
fn zooming_lays_out_a_subtree_as_full_circle() {
    let t = three_to_one();
    let big = t.children(NodeId::ROOT).find(|&c| t.name(c) == "big").unwrap();
    let l = SunburstLayout::compute(&t, big, params(), &HashSet::new());
    assert_eq!(l.root(), Some(big));
    let x = find(&l, &t, "x");
    assert_eq!(x.depth, 1);
    assert!(close(x.sweep, TAU * 2.0 / 3.0));
    assert!(close(l.used_sweep(), TAU));
}

#[test]
fn free_space_takes_the_rest_of_ring_one() {
    let t = three_to_one(); // 400 bytes used
    let l = layout(&t, LayoutParams { total: Some(800), ..params() });
    assert!(close(l.used_sweep(), PI));
    assert!(close(l.free_sweep(), PI));
    assert!(close(find(&l, &t, "small").end(), PI));

    assert_eq!(l.hit(-75.0, 0.0), Hit::Free, "9 o'clock is in the free half");
    assert_eq!(l.hit(-125.0, 0.0), Hit::None, "free space is only in ring 1");
    assert!(matches!(l.hit(75.0, 0.0), Hit::Segment(_)));

    // A total smaller than the used size is ignored.
    let l = layout(&t, LayoutParams { total: Some(1), ..params() });
    assert!(close(l.used_sweep(), TAU));
}

#[test]
fn parent_links_point_to_the_parent_segment() {
    let t = three_to_one();
    let l = layout(&t, params());
    for (i, s) in l.segments().iter().enumerate() {
        match s.parent {
            None => assert_eq!(s.depth, 1),
            Some(p) => {
                assert!(p < i, "parents come before children");
                let ps = l.segments()[p];
                assert_eq!(t.parent(s.node), Some(ps.node));
                assert_eq!(ps.depth + 1, s.depth);
            }
        }
        assert_eq!(s.part, 0);
    }
}

/// Two "drives": C (big: 300 used + 100 free) and D (100 used + 0 free).
fn two_drives() -> (Tree, Tree) {
    let mut c = Tree::new("C:\\");
    let win = c.add_dir(NodeId::ROOT, "Windows");
    c.add_file(win, "kernel", 300, 300);
    c.add_space(NodeId::ROOT, "Free space", NodeKind::FreeSpace, 100);
    let mut d = Tree::new("D:\\");
    d.add_file(NodeId::ROOT, "movie", 100, 100);
    (c, d)
}

#[test]
fn summary_puts_tree_roots_in_ring_one_in_given_order() {
    let (c, d) = two_drives();
    let l = SunburstLayout::compute_summary(&[(&c, NodeId::ROOT), (&d, NodeId::ROOT)], params(), &HashSet::new());
    assert_eq!(l.root(), None);

    let ring1 = l.ring(1);
    assert_eq!(ring1.len(), 2);
    assert_eq!((ring1[0].part, ring1[0].node), (0, NodeId::ROOT));
    assert_eq!((ring1[1].part, ring1[1].node), (1, NodeId::ROOT));
    assert!(close(ring1[0].sweep, TAU * 0.8), "C is 400 of 500 bytes");
    assert!(close(ring1[1].start, ring1[0].end()) && close(ring1[1].end(), TAU));

    // Ring 2 holds each drive's contents inside its arc, with the right part index.
    let ring2 = l.ring(2);
    let names: Vec<(u32, &str)> = ring2
        .iter()
        .map(|s| (s.part, if s.part == 0 { c.name(s.node) } else { d.name(s.node) }))
        .collect();
    assert_eq!(names, [(0, "Windows"), (0, "Free space"), (1, "movie")]);
    assert!(close(ring2[0].sweep, TAU * 0.6) && close(ring2[1].sweep, TAU * 0.2));
    assert_eq!(ring2[2].parent, Some(1));
    // Free-space blocks are leaves.
    assert!(l.ring(3).iter().all(|s| s.part == 0 && c.name(s.node) == "kernel"));
}

#[test]
fn summary_hit_test_and_collapse_per_part() {
    let (c, d) = two_drives();
    let parts = [(&c, NodeId::ROOT), (&d, NodeId::ROOT)];
    let l = SunburstLayout::compute_summary(&parts, params(), &HashSet::new());
    // D covers the last fifth of the circle (from 288° to 360°): 10 o'clock.
    let (x, y) = point_at(TAU * 0.9, 75.0);
    assert_eq!(l.hit(x, y), Hit::Segment(1));

    let l = SunburstLayout::compute_summary(&parts, params(), &HashSet::from([(0, NodeId::ROOT)]));
    assert!(l.segments().iter().all(|s| s.part == 1 || s.depth == 1), "C collapsed");
    assert_eq!(l.ring(2).len(), 1);
}

#[test]
fn empty_summary_has_no_segments() {
    let l = SunburstLayout::compute_summary(&[], params(), &HashSet::new());
    assert!(l.segments().is_empty());
    assert_eq!(l.hit(75.0, 0.0), Hit::None);
}

#[test]
fn empty_root_has_no_segments() {
    let mut t = Tree::new("root");
    t.add_dir(NodeId::ROOT, "empty");
    let l = layout(&t, params());
    assert!(l.segments().is_empty());
    assert_eq!(l.hit(75.0, 0.0), Hit::None);
    assert_eq!(l.hit(0.0, 0.0), Hit::Center);
}

/// Small deterministic generator, so the test needs no extra crates.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

fn random_tree(seed: u64) -> Tree {
    let mut rng = Lcg(seed);
    let mut t = Tree::new("root");
    let mut dirs = vec![NodeId::ROOT];
    for i in 0..2000 {
        let parent = dirs[rng.next(dirs.len() as u64) as usize];
        if rng.next(4) == 0 {
            dirs.push(t.add_dir(parent, &format!("d{i}")));
        } else {
            // Sizes spread over several orders of magnitude, some zero.
            let size = rng.next(10).pow(rng.next(7) as u32);
            t.add_file(parent, &format!("f{i}"), size, size);
        }
    }
    t
}

#[test]
fn random_trees_keep_children_inside_parents() {
    for seed in 1..=20 {
        let t = random_tree(seed);
        let p = LayoutParams { min_arc: 0.5, max_radius: 400.0, ..params() };
        let l = layout(&t, p);
        let by_node: std::collections::HashMap<NodeId, &Segment> = l.segments().iter().map(|s| (s.node, s)).collect();

        for s in l.segments() {
            assert!(s.sweep > 0.0 && s.start >= -EPS && s.end() <= TAU + 1e-6, "seed {seed}: {s:?}");
            let parent = t.parent(s.node).unwrap();
            if parent == NodeId::ROOT {
                assert_eq!(s.depth, 1);
            } else {
                let ps = by_node.get(&parent).unwrap_or_else(|| panic!("seed {seed}: orphan segment {s:?}"));
                assert_eq!(s.depth, ps.depth + 1);
                assert!(s.start >= ps.start - 1e-9 && s.end() <= ps.end() + 1e-9, "seed {seed}: {s:?} outside {ps:?}");
            }
            let (_, outer) = l.ring_radii(s.depth);
            assert!(s.sweep * outer >= p.min_arc - 1e-9, "seed {seed}: pruning missed {s:?}");
        }

        // Within each ring, segments never overlap.
        for d in 1..=l.ring_count() {
            assert!(l.ring(d).windows(2).all(|w| w[0].end() <= w[1].start + 1e-9), "seed {seed}: overlap in ring {d}");
        }
    }
}

#[test]
fn random_trees_hit_every_segment_at_its_centre() {
    for seed in 1..=20 {
        let t = random_tree(seed);
        let l = layout(&t, LayoutParams { min_arc: 0.5, max_radius: 400.0, ..params() });
        assert!(!l.segments().is_empty());
        for (i, s) in l.segments().iter().enumerate() {
            let (x, y) = centre_of(&l, s);
            assert_eq!(l.hit(x, y), Hit::Segment(i), "seed {seed}: {s:?}");
        }
    }
}
