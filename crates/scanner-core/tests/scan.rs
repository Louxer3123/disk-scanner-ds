use std::fs;
use std::path::Path;

use scanner_core::{CancelToken, NodeFlags, NodeId, NodeKind, Scan, Tree, scan};

fn run(root: &Path) -> Scan {
    scan(root, &CancelToken::new(), |_| {}).expect("scan failed")
}

fn child(tree: &Tree, parent: NodeId, name: &str) -> NodeId {
    tree.children(parent)
        .find(|&c| tree.name(c) == name)
        .unwrap_or_else(|| panic!("no child {name:?} under {:?}", tree.path(parent)))
}

fn write(path: &Path, len: usize) {
    fs::write(path, vec![b'x'; len]).unwrap();
}

/// root/
///   empty.txt     0
///   small.txt     1
///   docs/
///     a.bin       5000
///     nested/
///       b.bin     10000
///   empty_dir/
fn sample() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(&root.join("empty.txt"), 0);
    write(&root.join("small.txt"), 1);
    fs::create_dir_all(root.join("docs/nested")).unwrap();
    write(&root.join("docs/a.bin"), 5000);
    write(&root.join("docs/nested/b.bin"), 10_000);
    fs::create_dir(root.join("empty_dir")).unwrap();
    dir
}

#[test]
fn counts_files_dirs_and_lengths() {
    let dir = sample();
    let Scan { tree, stats } = run(dir.path());

    assert_eq!(stats.files, 4);
    assert_eq!(stats.dirs, 3);
    assert_eq!(stats.errors, 0);
    assert!(!stats.cancelled);
    assert_eq!(tree.logical(NodeId::ROOT), 15_001);

    let docs = child(&tree, NodeId::ROOT, "docs");
    assert_eq!(tree.kind(docs), NodeKind::Dir);
    assert_eq!(tree.logical(docs), 15_000);
    assert_eq!(tree.files(docs), 2);
    assert_eq!(tree.dirs(docs), 1);

    let empty_dir = child(&tree, NodeId::ROOT, "empty_dir");
    assert_eq!(tree.allocated(empty_dir), 0);
    assert_eq!(tree.children(empty_dir).count(), 0);
}

#[test]
fn allocated_size_is_whole_clusters() {
    let dir = sample();
    let Scan { tree, stats } = run(dir.path());
    let cluster = stats.cluster_size;

    let a = child(&tree, child(&tree, NodeId::ROOT, "docs"), "a.bin");
    assert_eq!(tree.logical(a), 5000);
    assert!(tree.allocated(a) >= 5000, "allocated {} < length", tree.allocated(a));
    assert_eq!(tree.allocated(child(&tree, NodeId::ROOT, "empty.txt")), 0);

    if cfg!(windows) {
        assert!(cluster.is_power_of_two() && cluster >= 512, "odd cluster size {cluster}");
        assert_eq!(tree.allocated(a), 5000u64.div_ceil(cluster) * cluster);
        assert_eq!(tree.allocated(NodeId::ROOT) % cluster, 0);
    }
}

#[test]
fn root_name_is_full_path_and_paths_point_to_real_files() {
    let dir = sample();
    let Scan { tree, .. } = run(dir.path());

    let shown = Path::new(tree.name(NodeId::ROOT));
    assert!(shown.is_absolute());
    assert!(!tree.name(NodeId::ROOT).starts_with(r"\\?\"), "verbatim prefix leaked: {shown:?}");

    let b = child(&tree, child(&tree, child(&tree, NodeId::ROOT, "docs"), "nested"), "b.bin");
    assert_eq!(fs::metadata(tree.path(b)).unwrap().len(), 10_000);
}

#[test]
fn unicode_names() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("Документы 📁");
    fs::create_dir(&sub).unwrap();
    write(&sub.join("отчёт «итог».txt"), 42);

    let Scan { tree, .. } = run(dir.path());
    let sub_id = child(&tree, NodeId::ROOT, "Документы 📁");
    let file = child(&tree, sub_id, "отчёт «итог».txt");
    assert_eq!(tree.logical(file), 42);
    assert!(tree.flags(file).is_empty());
}

#[test]
fn paths_longer_than_max_path() {
    let dir = tempfile::tempdir().unwrap();
    let segment = "a".repeat(50);
    let mut deep = dir.path().to_path_buf();
    for _ in 0..8 {
        deep.push(&segment);
    }
    assert!(deep.as_os_str().len() > 300);
    fs::create_dir_all(&deep).unwrap();
    write(&deep.join("leaf.txt"), 7);

    let Scan { tree, stats } = run(dir.path());
    assert_eq!(stats.errors, 0);
    assert_eq!(stats.files, 1);
    assert_eq!(stats.dirs, 8);
    assert_eq!(tree.logical(NodeId::ROOT), 7);
}

#[test]
fn links_are_recorded_but_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    fs::create_dir(&target).unwrap();
    write(&target.join("big.bin"), 100_000);
    let link = dir.path().join("link");
    if !make_dir_link(&target, &link) {
        eprintln!("skipping: could not create a directory link");
        return;
    }

    let Scan { tree, stats } = run(dir.path());
    let link_id = child(&tree, NodeId::ROOT, "link");
    assert_eq!(tree.kind(link_id), NodeKind::Link);
    assert_eq!(tree.allocated(link_id), 0);
    assert_eq!(stats.links, 1);
    // big.bin is counted once, through `target` only.
    assert_eq!(stats.files, 1);
    assert_eq!(tree.logical(NodeId::ROOT), 100_000);
}

/// A junction on Windows (needs no admin rights), a symlink elsewhere.
fn make_dir_link(target: &Path, link: &Path) -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|o| o.status.success())
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
}

#[test]
fn cancelled_scan_returns_partial_tree() {
    let dir = sample();
    let cancel = CancelToken::new();
    cancel.cancel();
    let Scan { tree, stats } = scan(dir.path(), &cancel, |_| {}).unwrap();
    assert!(stats.cancelled);
    // The root is listed before the token is checked, but nothing deeper.
    assert_eq!(tree.files(child(&tree, NodeId::ROOT, "docs")), 0);
}

#[test]
fn progress_is_reported_on_large_trees() {
    let dir = tempfile::tempdir().unwrap();
    for d in 0..20 {
        let sub = dir.path().join(format!("d{d}"));
        fs::create_dir(&sub).unwrap();
        for f in 0..300 {
            write(&sub.join(format!("f{f}")), 0);
        }
    }
    let mut last_files = 0;
    let mut calls = 0;
    let started = std::time::Instant::now();
    let result = scan(dir.path(), &CancelToken::new(), |p| {
        calls += 1;
        assert!(p.files >= last_files, "file count went backwards");
        last_files = p.files;
    })
    .unwrap();
    assert_eq!(result.stats.files, 6000);
    // Reports are throttled to one per 100 ms.
    let max_calls = started.elapsed().as_millis() / 100 + 1;
    assert!(calls as u128 <= max_calls, "{calls} progress calls in {:?}", started.elapsed());
}

#[test]
fn missing_root_or_file_root_is_an_error() {
    let dir = sample();
    assert!(scan(&dir.path().join("nope"), &CancelToken::new(), |_| {}).is_err());
    let err = scan(&dir.path().join("small.txt"), &CancelToken::new(), |_| {}).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[cfg(windows)]
#[test]
fn compressed_files_use_real_allocation() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("zeros.bin");
    write(&file, 0);
    fs::write(&file, vec![0u8; 1 << 20]).unwrap();
    let compressed = std::process::Command::new("compact")
        .args(["/C", "/Q"])
        .arg(&file)
        .output()
        .is_ok_and(|o| o.status.success());
    if !compressed {
        eprintln!("skipping: NTFS compression not available");
        return;
    }

    let Scan { tree, .. } = run(dir.path());
    let id = child(&tree, NodeId::ROOT, "zeros.bin");
    assert_eq!(tree.logical(id), 1 << 20);
    assert!(tree.allocated(id) < 1 << 20, "compressed file reported {} bytes", tree.allocated(id));
}

#[cfg(windows)]
#[test]
fn recycle_bin_is_flagged_only_at_volume_root() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("$Recycle.Bin")).unwrap();
    let Scan { tree, .. } = run(dir.path());
    let fake = child(&tree, NodeId::ROOT, "$Recycle.Bin");
    assert!(!tree.flags(fake).contains(NodeFlags::RECYCLE_BIN));
}
