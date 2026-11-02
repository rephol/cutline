mod common;
use common::*;
use cutline::replay::replay_dir;

#[test]
fn replay_over_synthetic_directory() {
    let dir = tempfile::tempdir().unwrap();
    // A growing session (consecutive turns), an unrelated tiny body, and two bad files.
    let full = session(24, 60);
    for (n, turns) in [(1, 4), (2, 12), (3, 16), (4, 20), (5, 24)] {
        let mut r = full.clone();
        r["messages"]
            .as_array_mut()
            .unwrap()
            .truncate(1 + 2 * turns);
        std::fs::write(dir.path().join(format!("{n:04}.json")), r.to_string()).unwrap();
    }
    std::fs::write(dir.path().join("0000.json"), "").unwrap();
    std::fs::write(dir.path().join("0009.json"), r#"{"nope":1}"#).unwrap();
    std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();

    let s = replay_dir(dir.path(), &cfg()).unwrap();
    assert_eq!(s.rows.len(), 5);
    assert_eq!(s.skipped, vec!["0000.json", "0009.json"]);
    assert!(!s.rows[0].report.over_budget, "4 turns fit the budget");
    assert!(s.rows[1..].iter().all(|r| r.report.cliff_fired));
    let out = s.render();
    assert!(out.contains("total: 5 bodies (2 skipped)"), "{out}");
    assert!(out.contains("saved"), "{out}");
    assert!(out.contains("cache breaks caused by compaction"), "{out}");
}

#[test]
fn cli_replay_prints_summary() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.json"), session(12, 60).to_string()).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cutline"))
        .args([
            "replay",
            dir.path().to_str().unwrap(),
            "--budget",
            "4000",
            "--keep",
            "2",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(
        s.contains("cliff=yes") && s.contains("total: 1 bodies"),
        "{s}"
    );
    let bad = std::process::Command::new(env!("CARGO_BIN_EXE_cutline"))
        .args(["replay", dir.path().to_str().unwrap(), "--bogus"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
}
