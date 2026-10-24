//! Scans a folder and finds duplicate files in it.
//!
//! cargo run --release -p scanner-core --example dupes_bench -- "C:\Users" 1048576

use std::time::Instant;

use scanner_core::dupes::{DupOptions, find_duplicates};
use scanner_core::{CancelToken, NodeId, default_threads, format_size, scan};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: dupes_bench <folder> [min size]");
    let min_size = args.next().map_or(1 << 20, |s| s.parse().expect("min size in bytes"));
    let tree = scan(path.as_ref(), &CancelToken::new(), |_| {}).expect("scan failed").tree;
    println!("scanned {} nodes", tree.node_count());

    let t = Instant::now();
    let options = DupOptions { min_size, threads: default_threads() };
    let mut phases = [(0usize, 0u64); 3];
    let r = find_duplicates(&[(&tree, NodeId::ROOT)], options, &CancelToken::new(), |p| {
        phases[p.phase as usize] = (p.total, p.bytes_total);
    });
    let wasted: u64 = r.groups.iter().map(|g| g.wasted()).sum();
    println!(
        "{:.1} s: {} groups, {} wasted, {} errors, {} placeholders skipped",
        t.elapsed().as_secs_f64(),
        r.groups.len(),
        format_size(wasted),
        r.errors,
        r.skipped
    );
    println!(
        "read heads of {} files, then {} files in full ({})",
        phases[1].0,
        phases[2].0,
        format_size(phases[2].1)
    );
    for g in r.groups.iter().take(5) {
        println!("  {} x {}  {}", g.files.len(), format_size(g.size), tree.path(g.files[0].node).display());
    }
}
