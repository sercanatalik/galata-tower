//! `--version` exits before the tower starts anything: the installer launches
//! a fresh build with it while the old tower still serves.

use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn version_prints_and_exits_before_serving() {
    let started = Instant::now();
    // An address nothing else holds: if --version fell through to serving,
    // the tower would bind it and this would hang rather than return.
    let out = Command::new(env!("CARGO_BIN_EXE_galata-tower"))
        .arg("--version")
        .env("GALATA_TOWER_LISTEN", "127.0.0.1:0")
        .env("GALATA_BROKER", "127.0.0.1:1")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("galata-tower "), "{text}");
    // A caller asks --check-config only of a tower that lists it here.
    assert!(
        text.contains("understands:") && text.contains("--check-config"),
        "{text}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "it served instead of exiting"
    );
}

#[test]
fn check_config_judges_a_document_and_exits() {
    let dir = std::env::temp_dir().join(format!("tower-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bad.toml");
    std::fs::write(&path, "[capture]\nfrom_the_future = 1\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_galata-tower"))
        .args(["--check-config", path.to_str().unwrap()])
        .env("GALATA_TOWER_LISTEN", "127.0.0.1:0")
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(!out.status.success(), "{text}");
    assert!(text.starts_with("refused: "), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
