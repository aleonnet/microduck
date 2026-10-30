//! Phase 1 end to end without a robot: `s1-e2e fake` as a test, so `cargo test -p s1d` runs the
//! whole chain — pad datagrams into `netpadd`, `robotctl` and `s1ctl` on the socket, `s1d` on a
//! fake body — and fails when any item's bytes on the wire are not the robot's.
//!
//! `netpadd` and `robotctl` belong to other packages, so this test builds them first with the
//! same `cargo` that runs it (cargo releases the build lock before the test binaries start).

use std::path::PathBuf;
use std::process::Command;

#[test]
fn every_item_of_phase_1_passes_through_the_ducks_mechanism() {
    let cargo = env!("CARGO");
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let e2e = PathBuf::from(env!("CARGO_BIN_EXE_s1-e2e"));
    // The sibling binaries are built into the profile directory this test's binary sits in, so
    // `cargo test --release` looks beside `target/release/s1-e2e` and not under `debug`.
    let bin = e2e.parent().expect("the binary has a directory");
    let mut build = vec!["build", "-p", "netpadd", "-p", "robotctl"];
    if bin.file_name().is_some_and(|p| p == "release") {
        build.push("--release");
    }
    let status = Command::new(cargo)
        .args(&build)
        .current_dir(manifest_dir.join(".."))
        .status()
        .expect("cargo runs");
    assert!(status.success(), "building netpadd and robotctl failed");
    for b in ["s1d", "s1ctl", "netpadd", "robotctl"] {
        assert!(
            bin.join(b).is_file(),
            "{} is missing",
            bin.join(b).display()
        );
    }
    let out = Command::new(&e2e)
        .arg("fake")
        .output()
        .expect("s1-e2e runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "s1-e2e fake failed:\n{stdout}\n{stderr}"
    );
    let line = stdout
        .lines()
        .find(|l| l.contains("items passed"))
        .unwrap_or("");
    assert!(
        line.starts_with("40 of 40"),
        "every item must run and pass: {line:?}"
    );
}
