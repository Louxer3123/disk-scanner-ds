//! Scans a folder and times searching it.
//!
//! cargo run --release -p scanner-core --example search_bench -- "C:\Users" "*.dll" "cache" "фото"

use std::time::Instant;

use scanner_core::search::{Query, find_marked, mark_matches};
use scanner_core::{CancelToken, NodeId, format_size, scan};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: search_bench <folder> <query>...");
    let result = scan(path.as_ref(), &CancelToken::new(), |_| {}).expect("scan failed");
    let tree = &result.tree;
    println!("scanned {} nodes in {:.2} s", tree.node_count(), result.stats.elapsed.as_secs_f64());

    for text in args {
        let query = Query::parse(&text).expect("empty query");
        let t = Instant::now();
        let marks = mark_matches(tree, &query);
        let mark_time = t.elapsed();
        let t = Instant::now();
        let found = find_marked(tree, NodeId::ROOT, &marks);
        let find_time = t.elapsed();
        println!(
            "{text:<12} {:>8} matches, {:>9}  mark {:>6.1} ms, collect {:>6.1} ms",
            found.nodes.len(),
            format_size(found.allocated),
            mark_time.as_secs_f64() * 1000.0,
            find_time.as_secs_f64() * 1000.0
        );
    }
}
