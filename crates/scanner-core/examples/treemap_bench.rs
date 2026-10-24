//! Scans a folder and times the treemap layout and hit testing for it.
//!
//! cargo run --release -p scanner-core --example treemap_bench -- "C:\Users"

use std::collections::HashSet;
use std::time::Instant;

use scanner_core::treemap::{TreemapLayout, TreemapParams};
use scanner_core::{CancelToken, NodeId, scan};

fn main() {
    let path = std::env::args().nth(1).expect("usage: treemap_bench <folder>");
    let tree = scan(path.as_ref(), &CancelToken::new(), |_| {}).expect("scan failed").tree;
    println!("scanned {} nodes", tree.node_count());
    for (w, h) in [(800.0, 600.0), (1600.0, 1000.0), (3200.0, 2000.0)] {
        let params = TreemapParams { width: w, height: h, ..TreemapParams::default() };
        let t = Instant::now();
        let layout = TreemapLayout::compute(&tree, NodeId::ROOT, params, &HashSet::new());
        let layout_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let n = 1000;
        for i in 0..n {
            let _ = layout.hit((i as f64 * 7.3) % w, 28.0 + (i as f64 * 3.1) % (h - 28.0));
        }
        println!(
            "{w}x{h}: {} tiles, layout {layout_ms:.1} ms, hit {:.1} us",
            layout.tiles().len(),
            t.elapsed().as_secs_f64() * 1e6 / n as f64
        );
    }
}
