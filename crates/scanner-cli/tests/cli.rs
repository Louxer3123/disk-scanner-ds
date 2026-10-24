use std::fs;
use std::process::{Command, Output};

fn scanner(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scanner")).args(args).output().unwrap()
}

fn sample() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("big/inner")).unwrap();
    fs::write(dir.path().join("big/inner/data.bin"), vec![1u8; 50_000]).unwrap();
    fs::write(dir.path().join("small, quoted.txt"), b"hi").unwrap();
    dir
}

fn stdout(out: &Output) -> String {
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout.clone()).unwrap()
}

#[test]
fn json_has_stats_and_sorted_tree() {
    let dir = sample();
    let out = stdout(&scanner(&[dir.path().to_str().unwrap(), "-f", "json", "-q"]));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();

    assert_eq!(v["stats"]["files"], 2);
    assert_eq!(v["stats"]["dirs"], 2);
    assert_eq!(v["stats"]["cancelled"], false);
    assert_eq!(v["tree"]["logical"], 50_002);
    let kids = v["tree"]["children"].as_array().unwrap();
    assert_eq!(kids[0]["name"], "big");
    assert_eq!(kids[0]["children"][0]["children"][0]["name"], "data.bin");
    assert_eq!(kids[1]["kind"], "file");
}

#[test]
fn json_depth_and_min_size_filters() {
    let dir = sample();
    let out = stdout(&scanner(&[dir.path().to_str().unwrap(), "-f", "json", "-q", "-d", "1", "--min-size", "4097"]));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let kids = v["tree"]["children"].as_array().unwrap();
    assert_eq!(kids.len(), 1, "small file should be filtered out");
    assert!(kids[0].get("children").is_none(), "depth 1 must not list grandchildren");
}

#[test]
fn csv_lists_every_entry_with_quoting() {
    let dir = sample();
    let out = stdout(&scanner(&[dir.path().to_str().unwrap(), "-f", "csv", "-q"]));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "path,kind,allocated,logical,files,dirs,flags");
    assert_eq!(lines.len(), 1 + 5, "root, big, inner, data.bin, small file");
    assert!(lines[1].contains(",dir,") && lines[1].ends_with(",2,2,"));
    assert!(lines.iter().any(|l| l.starts_with('"') && l.contains("small, quoted.txt\"")));
}

#[test]
fn summary_lists_largest_first() {
    let dir = sample();
    let out = stdout(&scanner(&[dir.path().to_str().unwrap(), "-q", "-n", "1"]));
    assert!(out.contains("2 files, 2 folders"), "{out}");
    let big = out.find("big").unwrap();
    assert!(!out[big..].contains("small, quoted.txt"), "top 1 must hide the small file");
    assert!(out.contains("… and 1 more"));
}

#[test]
fn writes_to_output_file() {
    let dir = sample();
    let target = tempfile::tempdir().unwrap();
    let file = target.path().join("report.json");
    let out = scanner(&[dir.path().to_str().unwrap(), "-f", "json", "-q", "-o", file.to_str().unwrap()]);
    assert!(stdout(&out).is_empty());
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap();
    assert_eq!(v["stats"]["files"], 2);
}

#[test]
fn missing_path_fails_with_message() {
    let out = scanner(&["Z:\\definitely\\not\\here", "-q"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("scanner: "));
}
