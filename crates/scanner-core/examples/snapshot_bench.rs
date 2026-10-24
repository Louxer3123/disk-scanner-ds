//! Scans a folder, saves a snapshot, reads it back and compares it with the scan.
//!
//! cargo run --release -p scanner-core --example snapshot_bench -- "C:\Users"

use std::time::Instant;

use scanner_core::diff::Diff;
use scanner_core::snapshot::{read_snapshot, write_snapshot};
use scanner_core::{CancelToken, NodeId, format_size, scan};

fn main() {
    let path = std::env::args().nth(1).expect("usage: snapshot_bench <folder>");
    let tree = scan(path.as_ref(), &CancelToken::new(), |_| {}).expect("scan failed").tree;
    println!("scanned {} nodes", tree.node_count());

    // The app copies the tree this way to show a scan in progress.
    let t = Instant::now();
    let copy = tree.clone();
    println!("clone {:>7.0} ms, {} nodes", t.elapsed().as_secs_f64() * 1000.0, copy.node_count());
    drop(copy);

    let file = std::env::temp_dir().join("snapshot_bench.dsnap");
    let t = Instant::now();
    write_snapshot(&tree, 0, std::fs::File::create(&file).unwrap()).unwrap();
    let size = std::fs::metadata(&file).unwrap().len();
    println!("write {:>7.0} ms, {}", t.elapsed().as_secs_f64() * 1000.0, format_size(size));

    let t = Instant::now();
    let (_, old) = read_snapshot(std::fs::File::open(&file).unwrap()).unwrap();
    println!("read  {:>7.0} ms, {} nodes", t.elapsed().as_secs_f64() * 1000.0, old.node_count());

    let t = Instant::now();
    let diff = Diff::compute(&old, &tree);
    println!(
        "diff  {:>7.0} ms, root delta {}, {} removed",
        t.elapsed().as_secs_f64() * 1000.0,
        diff.delta(&old, &tree, NodeId::ROOT),
        diff.removed(&old, None).len()
    );
    let _ = std::fs::remove_file(&file);
}
