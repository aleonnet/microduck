//! `deploy/s1/robotd.toml` must load with the validation `netpadd` applies to it, and the two
//! refusals below must still be refusals — otherwise this file could rot into one that
//! `netpadd` rejects at startup, which on the robot is a pad that does nothing.

use std::path::PathBuf;

fn shipped() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../deploy/s1/robotd.toml")
}

#[test]
fn the_shipped_file_loads_with_netpad_enabled() {
    let params =
        robotd_params::Params::load(&shipped(), true).expect("deploy/s1/robotd.toml loads");
    assert!(params.netpad.enabled);
    assert_eq!(params.netpad.port, 4210);
    assert!(
        params.netpad.stale_ms < params.safety.deadman_ms,
        "stale_ms must stay below the deadman"
    );
    // A mecanum base strafes: the prototype roller's `vy = 0` default would make the left
    // stick's sideways travel do nothing on the S1 (caught by `s1-e2e fake`, 2026-10-30).
    assert!(
        params.pad_roller.vy_max > 0.0 && params.pad_roller.vy_min < 0.0,
        "[pad_roller] must open the strafe: {:?}",
        params.pad_roller
    );
}

fn load_variant(from: &str, to: &str) -> Result<robotd_params::Params, robotd_params::ParamsError> {
    let text = std::fs::read_to_string(shipped()).unwrap();
    assert!(
        text.contains(from),
        "the shipped file no longer has `{from}`; the refutation must change with it"
    );
    let dir = std::env::temp_dir().join(format!("s1d-params-{}-{}", std::process::id(), to.len()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("robotd.toml");
    std::fs::write(&path, text.replace(from, to)).unwrap();
    let r = robotd_params::Params::load(&path, true);
    let _ = std::fs::remove_dir_all(&dir);
    r
}

/// A `stale_ms` above the deadman, or a `max_hz` of zero,
/// is refused by name.
#[test]
fn a_stale_ms_above_the_deadman_is_refused() {
    let err = load_variant("stale_ms = 300", "stale_ms = 900")
        .expect_err("900 ms is above the 500 ms deadman");
    let msg = err.to_string();
    assert!(msg.contains("stale_ms"), "the refusal names the key: {msg}");
}

#[test]
fn a_zero_max_hz_is_refused() {
    let err = load_variant("max_hz = 30", "max_hz = 0").expect_err("0 Hz is outside 10..=100");
    let msg = err.to_string();
    assert!(msg.contains("max_hz"), "the refusal names the key: {msg}");
}
