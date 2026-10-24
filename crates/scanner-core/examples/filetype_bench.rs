//! Scans a folder, times the file type passes and prints the largest types.
//!
//! cargo run --release -p scanner-core --example filetype_bench -- "C:\Users"

use std::time::Instant;

use scanner_core::filetype::{TypeStats, dominant_categories};
use scanner_core::{CancelToken, NodeId, format_size, scan};

fn main() {
    let path = std::env::args().nth(1).expect("usage: filetype_bench <folder>");
    let result = scan(path.as_ref(), &CancelToken::new(), |_| {}).expect("scan failed");
    let tree = &result.tree;
    println!("scanned {} nodes in {:.2} s", tree.node_count(), result.stats.elapsed.as_secs_f64());

    let t = Instant::now();
    let cats = dominant_categories(tree);
    println!("dominant categories {:>6.1} ms, root: {:?}", t.elapsed().as_secs_f64() * 1000.0, cats[0]);

    let t = Instant::now();
    let mut stats = TypeStats::default();
    stats.add(tree, NodeId::ROOT);
    let exts = stats.extensions();
    println!("extension stats     {:>6.1} ms, {} extensions", t.elapsed().as_secs_f64() * 1000.0, exts.len());
    for e in exts.iter().take(12) {
        println!("  {:>10}  {:>8} files  .{:<10} {:?}", format_size(e.allocated), e.files, e.extension, e.category);
    }
}
