//! Scans a folder and times the sunburst layout and hit testing on it.
//!
//! cargo run --release -p scanner-core --example layout_bench -- "C:\Program Files"

use std::collections::HashSet;
use std::time::Instant;

use scanner_core::{CancelToken, Hit, LayoutParams, NodeId, SunburstLayout, scan};

fn main() {
    let path = std::env::args().nth(1).expect("usage: layout_bench <folder>");
    let result = scan(path.as_ref(), &CancelToken::new(), |_| {}).expect("scan failed");
    let tree = &result.tree;
    println!(
        "scanned {} nodes in {:.2} s",
        tree.node_count(),
        result.stats.elapsed.as_secs_f64()
    );

    // Roughly a 1000 x 1000 px chart.
    let params = LayoutParams {
        center_radius: 80.0,
        ring_width: 50.0,
        max_radius: 480.0,
        min_arc: 1.5,
        total: None,
    };
    let collapsed = HashSet::new();
    let largest = tree.children_by_size(NodeId::ROOT).into_iter().take(3);
    for root in std::iter::once(NodeId::ROOT).chain(largest) {
        let t = Instant::now();
        let layout = SunburstLayout::compute(tree, root, params, &collapsed);
        let layout_time = t.elapsed();

        let t = Instant::now();
        let mut hits = 0;
        for y in (-480..480).step_by(4) {
            for x in (-480..480).step_by(4) {
                if matches!(layout.hit(f64::from(x), f64::from(y)), Hit::Segment(_)) {
                    hits += 1;
                }
            }
        }
        let per_hit = t.elapsed() / (240 * 240);

        println!(
            "{:<40} {:>6} segments in {:>6.2} ms, hit test {:>5} ns ({hits} of 57600 points on segments)",
            tree.path(root).display().to_string(),
            layout.segments().len(),
            layout_time.as_secs_f64() * 1000.0,
            per_hit.as_nanos()
        );
    }
}
