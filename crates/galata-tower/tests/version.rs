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
    assert!(started.elapsed() < Duration::from_secs(60), "it served instead of exiting");
}
