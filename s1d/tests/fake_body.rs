//! The control loop against a body made of a file: what reaches the wire, decoded back.
//!
//! This is the deadman's proof (acceptance 3): one `robot.move`, then silence, and the wheel
//! frames must go to zero after the deadman while the heartbeat keeps coming.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use s1_duml::cmd::{host, id};
use s1_duml::{Decoder, Frame};
use s1d::body::FakeFile;
use s1d::intents::Intents;
use s1d::state::State;
use s1d::tick::{Config, DDS_MSG_ID, DDS_PUSH_LEN, run};

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
    assert_eq!(
        subs[0].data[4], 9,
        "nine subjects, DJI's own list minus the servos"
    );
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
    // Telemetry is asked for at rest — battery and pick-up matter before anything moves — and a
    // subscription moves nothing. Everything else on the wire is the exit's zeroes.
    let (dds, rest): (Vec<&Frame>, Vec<&Frame>) =
        frames.iter().partition(|f| f.cmd_set == id::DDS_ADD_SUB.0);
    assert_eq!(
        dds.iter()
            .map(|f| (f.cmd_set, f.cmd_id))
            .collect::<Vec<_>>(),
        vec![id::DDS_RESET_NODE, id::DDS_ADD_NODE, id::DDS_ADD_SUB],
        "node reset, registered, then one subscription, while disarmed"
    );
    assert!(
        rest.iter()
            .all(|f| (f.cmd_set, f.cmd_id) == id::CHASSIS_WHEEL_SPEED && wheel_rpm(f) == [0; 4]),
        "only the exit zeroes: {}",
        rest.len()
    );
    assert_eq!(rest.len(), 10);
}

/// The stock firmware's subscription outlives `dji_hdvt_uav` (measured 2026-10-30: the motion
/// controller kept pushing msg id 1 after the stock was stopped), so a push with any other msg id,
/// or ours with the wrong length, is somebody else's and must not become our telemetry.
#[test]
fn only_our_pushes_become_telemetry() {
    const STOCK_PUSH: &[u8] = include_bytes!("../../s1-duml/tests/fixtures/dds_push_176_real.bin");
    let path = std::env::temp_dir().join(format!("s1d-fake-push-{}.bin", std::process::id()));
    let mut body = FakeFile::create(&path).unwrap();
    // The stock's real push (msg id 1, 161 bytes of subjects), then ours: the time stamp, then the
    // subjects in SUBJECTS order, battery first with 77 % at its ninth byte.
    body.inbound.extend_from_slice(STOCK_PUSH);
    let mut ours = vec![0u8, DDS_MSG_ID];
    let mut subjects = vec![0u8; DDS_PUSH_LEN];
    subjects[DDS_PUSH_LEN - 10 + 8] = 77; // battery is the last subject, ten bytes, percent ninth
    ours.extend_from_slice(&subjects);
    let frame = Frame::request(
        host::MOTION_CONTROLLER,
        host::BASE,
        7,
        id::DDS_PUSH.0,
        id::DDS_PUSH.1,
        ours,
    );
    body.inbound.extend_from_slice(&frame.encode().unwrap());

    let state = Arc::new(State::new(100));
    let intents = Arc::new(Intents::new());
    let (s, i) = (Arc::clone(&state), Arc::clone(&intents));
    let handle = std::thread::spawn(move || run(&mut body, s, i, Config::default()));
    std::thread::sleep(Duration::from_millis(200));
    state.shutdown.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_file(&path);

    let t = state.telemetry.load();
    assert_eq!(t.pushes, 1, "the stock's push must not count");
    assert_eq!(t.foreign_pushes, 1, "but it is seen and counted");
    assert_eq!(t.battery.map(|b| b.percent), Some(77));
    assert_eq!(t.sa_status.map(|s| s.is_pick_up), Some(false));
}

/// A frame queued over the socket reaches the wire with its own sequence number, and in gimbal
/// lead the twist leaves as the controller's follow command instead of wheel speeds.
#[test]
fn queued_frames_and_gimbal_lead_reach_the_wire() {
    use s1d::intents::FollowMode;
    let path = std::env::temp_dir().join(format!("s1d-fake-queue-{}.bin", std::process::id()));
    let mut body = FakeFile::create(&path).unwrap();
    let state = Arc::new(State::new(100));
    let intents = Arc::new(Intents::new());
    intents.request_frame(s1_duml::cmd::gimbal_workmode(0, 2));
    intents.request_frame(s1_duml::cmd::gimbal_workmode(0, 0));
    intents.set_follow(FollowMode::GimbalLead);
    intents.set_enabled(true);
    intents.set_twist([0.3, 0.0, 1.0]);
    let (s, i) = (Arc::clone(&state), Arc::clone(&intents));
    let handle = std::thread::spawn(move || run(&mut body, s, i, Config::default()));
    std::thread::sleep(Duration::from_millis(200));
    state.shutdown.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let frames = Decoder::new().push(&bytes);
    let modes: Vec<u16> = frames
        .iter()
        .filter(|f| (f.cmd_set, f.cmd_id) == id::GIMBAL_WORKMODE)
        .map(|f| f.seq)
        .collect();
    assert_eq!(modes, vec![0, 1], "each queued frame got its own sequence");
    let follows = frames
        .iter()
        .filter(|f| (f.cmd_set, f.cmd_id) == id::CHASSIS_FOLLOW)
        .count();
    assert!(
        follows >= 10,
        "the twist went out as follow frames: {follows}"
    );
    let moving_wheels = frames
        .iter()
        .filter(|f| (f.cmd_set, f.cmd_id) == id::CHASSIS_WHEEL_SPEED && wheel_rpm(f) != [0; 4])
        .count();
    assert_eq!(moving_wheels, 0, "no wheel speeds while the gimbal leads");
}
