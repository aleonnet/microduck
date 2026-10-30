//! The control loop against a body made of a file: what reaches the wire, decoded back.
//!
//! This is the deadman's proof (acceptance 3): one `robot.move`, then silence, and the wheel
//! frames must go to zero after the deadman while the heartbeat keeps coming.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use s1_duml::cmd::id;
use s1_duml::{Decoder, Frame};
use s1d::body::FakeFile;
use s1d::intents::Intents;
use s1d::state::State;
use s1d::tick::{Config, run};

fn wheel_rpm(f: &Frame) -> [i16; 4] {
    let h = |i: usize| i16::from_le_bytes([f.data[i], f.data[i + 1]]);
    [h(0), h(2), h(4), h(6)]
}

#[test]
fn the_deadman_zeroes_the_wheels_and_keeps_the_heartbeat() {
    let path = std::env::temp_dir().join(format!("s1d-fake-{}.bin", std::process::id()));
    let mut body = FakeFile::create(&path).unwrap();
    let state = Arc::new(State::new(100));
    let intents = Arc::new(Intents::new());
    let cfg = Config {
        deadman: Duration::from_millis(300),
        ..Config::default()
    };

    intents.set_enabled(true);
    intents.set_twist([0.5, 0.0, 0.0]);
    let (s, i) = (Arc::clone(&state), Arc::clone(&intents));
    let handle = std::thread::spawn(move || run(&mut body, s, i, cfg));
    std::thread::sleep(Duration::from_millis(800));
    state.shutdown.store(true, Ordering::Relaxed);
    handle.join().unwrap();

    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let mut dec = Decoder::new();
    let frames = dec.push(&bytes);
    assert_eq!(
        dec.dropped(),
        0,
        "everything on the wire must be whole frames"
    );

    let heartbeats = frames
        .iter()
        .filter(|f| (f.cmd_set, f.cmd_id) == id::HEARTBEAT)
        .count();
    assert!(
        heartbeats >= 60,
        "heartbeat must keep coming: {heartbeats} in 0.8 s"
    );

    let wheels: Vec<[i16; 4]> = frames
        .iter()
        .filter(|f| (f.cmd_set, f.cmd_id) == id::CHASSIS_WHEEL_SPEED)
        .map(wheel_rpm)
        .collect();
    assert!(wheels.len() >= 60, "{}", wheels.len());
    let first_nonzero = wheels
        .iter()
        .position(|w| w.iter().any(|&r| r != 0))
        .expect("the move was driven");
    assert!(first_nonzero < 5, "the first ticks carry the move");
    // After the deadman (300 ms = ~30 ticks) every wheel frame is zero, including the shutdown burst.
    let tail = &wheels[wheels.len() - 40..];
    assert!(
        tail.iter().all(|w| *w == [0; 4]),
        "wheels after the deadman: {:?}",
        &tail[..5]
    );
    // And the loop stamped itself as healthy work would.
    assert!(state.ticks.load(Ordering::Relaxed) >= 60);
    assert!(state.health().healthy, "{:?}", state.health().reason);

    // The subscription went out once, to our msg id, with seven subjects.
    let subs: Vec<&Frame> = frames
        .iter()
        .filter(|f| (f.cmd_set, f.cmd_id) == id::DDS_ADD_SUB)
        .collect();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].data[4], 7);
}

/// A disarmed loop sends nothing to the chassis: no heartbeat, no wheels. That is the state a
/// daemon starts in, and it is what keeps a restart from moving a robot.
#[test]
fn disarmed_sends_nothing_but_zeroes_on_exit() {
    let path = std::env::temp_dir().join(format!("s1d-fake-idle-{}.bin", std::process::id()));
    let mut body = FakeFile::create(&path).unwrap();
    let state = Arc::new(State::new(100));
    let intents = Arc::new(Intents::new());
    intents.set_twist([0.5, 0.0, 0.0]);
    let (s, i) = (Arc::clone(&state), Arc::clone(&intents));
    let handle = std::thread::spawn(move || run(&mut body, s, i, Config::default()));
    std::thread::sleep(Duration::from_millis(200));
    state.shutdown.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let frames = Decoder::new().push(&bytes);
    assert!(
        frames
            .iter()
            .all(|f| (f.cmd_set, f.cmd_id) == id::CHASSIS_WHEEL_SPEED && wheel_rpm(f) == [0; 4]),
        "only the exit zeroes: {}",
        frames.len()
    );
    assert_eq!(frames.len(), 10);
}
