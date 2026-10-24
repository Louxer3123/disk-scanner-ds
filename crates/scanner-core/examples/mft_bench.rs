//! Scans a drive through its MFT and with the directory walker, and compares
//! time and totals. Run from an administrator terminal:
//!
//! cargo run --release -p scanner-core --example mft_bench -- C:\

use std::time::Instant;

use scanner_core::{CancelToken, NodeId, Scan, format_size, is_elevated, scan, scan_mft};

fn report(label: &str, scan: &Scan, seconds: f64) {
    let t = &scan.tree;
    println!(
        "{label:<7} {seconds:>7.2} s  {:>10} allocated  {:>10} in file lengths  {:>9} files  {:>8} folders  {} links",
        format_size(t.allocated(NodeId::ROOT)),
        format_size(t.logical(NodeId::ROOT)),
        scan.stats.files,
        scan.stats.dirs,
        scan.stats.links,
    );
}

fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| "C:\\".into());
    println!("elevated: {}", is_elevated());

    let t = Instant::now();
    match scan_mft(root.as_ref(), &CancelToken::new(), |_| {}) {
        Ok(s) => report("mft", &s, t.elapsed().as_secs_f64()),
        Err(e) => {
            println!("mft     failed: {e}");
            return;
        }
    }
    let t = Instant::now();
    let walked = scan(root.as_ref(), &CancelToken::new(), |_| {}).expect("walk failed");
    report("walker", &walked, t.elapsed().as_secs_f64());
}
