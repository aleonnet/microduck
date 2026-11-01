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

    // The subscription went out once, to our msg id, with nine subjects.
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
        vec![id::DDS_ADD_NODE, id::DDS_ADD_SUB],
        "registered, then one subscription, while disarmed — never a node reset (measured 2026-10-30: one reset leaves the controller's bus dead until a power cycle)"
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
/// controller kept pushing it after the stock was stopped), and it is decoded by its layout: the
/// 2026-10-29 push lands as battery 77 %, then a push of ours (66 %) overwrites it; a push with a
/// length nobody owns is counted and left alone. Both are acked as the stock acks.
#[test]
fn the_stocks_push_and_ours_both_become_telemetry() {
    const STOCK_PUSH: &[u8] = include_bytes!("../../s1-duml/tests/fixtures/dds_push_176_real.bin");
    const ONE_HZ_PUSH: &[u8] =
        include_bytes!("../../s1-duml/tests/fixtures/dds_push_stock_1hz_2026-10-30.bin");
    let path = std::env::temp_dir().join(format!("s1d-fake-push-{}.bin", std::process::id()));
    let mut body = FakeFile::create(&path).unwrap();
    body.inbound.extend_from_slice(STOCK_PUSH);
    body.inbound.extend_from_slice(ONE_HZ_PUSH);
    // Ours: the time stamp, then the subjects in SUBJECTS order, battery last with 66 % at its
    // ninth byte.
    let mut ours = vec![0u8, DDS_MSG_ID];
    let mut subjects = vec![0u8; DDS_PUSH_LEN];
    subjects[DDS_PUSH_LEN - 10 + 8] = 66;
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

    let stock_seq = Frame::decode(STOCK_PUSH).unwrap().seq;
    let one_hz_seq = Frame::decode(ONE_HZ_PUSH).unwrap().seq;
    let state = Arc::new(State::new(100));
    let intents = Arc::new(Intents::new());
    let (s, i) = (Arc::clone(&state), Arc::clone(&intents));
    let handle = std::thread::spawn(move || run(&mut body, s, i, Config::default()));
    std::thread::sleep(Duration::from_millis(200));
    state.shutdown.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);

    let t = state.telemetry.load();
    assert_eq!(t.pushes, 2, "the stock's push and ours");
    assert_eq!(t.stock_pushes, 1);
    assert_eq!(
        t.foreign_pushes, 1,
        "the 1 Hz subject: seen, counted, not decoded"
    );
    assert_eq!(t.battery.map(|b| b.percent), Some(66), "ours came last");
    assert_eq!(
        t.sa_status.map(|s| s.on_slope),
        Some(false),
        "ours, all zero, came last"
    );
    // Every push was acknowledged with the stock's frame, each echoing the push's own sequence.
    let acks: Vec<Vec<u8>> = Decoder::new()
        .push(&bytes)
        .iter()
        .filter(|f| f.is_ack && (f.cmd_set, f.cmd_id) == id::DDS_PUSH)
        .map(|f| f.encode().unwrap())
        .collect();
    assert_eq!(
        acks,
        vec![
            s1_duml::dds::push_ack(stock_seq).encode().unwrap(),
            s1_duml::dds::push_ack(one_hz_seq).encode().unwrap(),
            s1_duml::dds::push_ack(7).encode().unwrap()
        ],
        "one ack per push, in order, with their sequences"
    );
}

/// The stock's push alone (what the robot gives with `dji_hdvt_uav` stopped) fills `robot.health`
/// with the controller's battery and `robot.state` with odometry, attitude and the status bits;
/// the gimbal's own push puts the real angle in `head`, converted to the duck's signs.
#[test]
fn stock_pushes_fill_health_state_and_the_head() {
    let path = std::env::temp_dir().join(format!("s1d-fake-stock-{}.bin", std::process::id()));
    let mut body = FakeFile::create(&path).unwrap();
    body.inbound.extend_from_slice(include_bytes!(
        "../../s1-duml/tests/fixtures/dds_push_stock_turned_2026-10-30.bin"
    ));
    body.inbound.extend_from_slice(include_bytes!(
        "../../s1-duml/tests/fixtures/gimbal_push_yaw_max_2026-10-30.bin"
    ));
    let state = Arc::new(State::new(100));
    let intents = Arc::new(Intents::new());
    let (s, i) = (Arc::clone(&state), Arc::clone(&intents));
    let handle = std::thread::spawn(move || run(&mut body, s, i, Config::default()));
    std::thread::sleep(Duration::from_millis(200));
    state.shutdown.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_file(&path);

    let health = state.health();
    let battery = health.battery.expect("the controller's battery reading");
    assert!((battery.volts - 11.819).abs() < 1e-9, "{}", battery.volts);
    assert_eq!(battery.percent, 78.0);

    let frame = state.frame([0.0; 3], Vec::new());
    assert!((frame.odom.position[0] - 0.0938).abs() < 1e-4);
    // DJI's yaw is clockwise positive; the duck's heading is counter-clockwise, as its `vyaw`.
    assert!(
        (frame.odom.yaw + 19.329f64.to_radians()).abs() < 1e-4,
        "attitude yaw, radians, the duck's sign"
    );
    assert!(!frame.safety.fallen && !frame.safety.picked_up);
    let imu = frame.imu.expect("gyro present once a push arrived");
    assert!(
        (imu.gyro[2] - 0.0034).abs() < 1e-4,
        "z up: DJI's −0.0034 about z-down"
    );
    // The chassis attitude travels as a quaternion, and gravity as the accelerometer saw it:
    // a flat robot, one g down the trunk's z.
    assert!((imu.quat[0] - (19.329f64.to_radians() / 2.0).cos()).abs() < 1e-3);
    assert!(frame.safety.gravity[2] < -0.99 && frame.safety.gravity[0].abs() < 0.05);
    // The head is relative to the body: the gimbal's yaw against the chassis, 158.3° to DJI's
    // right, is the duck's −158.3° (left positive); the ground-frame −179.6° stays in
    // `s1.telemetry`. Pitch +35.6° up is the duck's −35.6°.
    assert!(
        (frame.head[2] + 158.3f64.to_radians()).abs() < 1e-6,
        "{}",
        frame.head[2]
    );
    assert!(
        (frame.head[1] + 35.6f64.to_radians()).abs() < 1e-6,
        "{}",
        frame.head[1]
    );

    let t = state.telemetry_result();
    assert_eq!((t.pushes, t.stock_pushes, t.foreign_pushes), (1, 1, 0));
    assert_eq!(t.gimbal_deg, Some([-179.6, 35.6]));
    assert_eq!(t.gimbal_yaw_vs_chassis_deg, Some(158.3));
    assert_eq!(t.battery.map(|b| b.adc_value), Some(11_819));
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
    // Arming sends the gimbal's free mode first (seq 0, below); the two queued frames follow with
    // their own sequences.
    let modes: Vec<(u16, u8)> = frames
        .iter()
        .filter(|f| (f.cmd_set, f.cmd_id) == id::GIMBAL_WORKMODE)
        .map(|f| (f.seq, f.data[0]))
        .collect();
    assert_eq!(
        modes,
        vec![(0, 0), (1, 2), (2, 0)],
        "the arm's free mode, then each queued frame with its own sequence"
    );
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

/// `s1.wheels` fresher than the twist drives the wheels directly, and the deadman zeroes it too.
#[test]
fn direct_wheel_speeds_reach_the_wire_and_die_with_the_deadman() {
    let path = std::env::temp_dir().join(format!("s1d-fake-wheels-{}.bin", std::process::id()));
    let mut body = FakeFile::create(&path).unwrap();
    let state = Arc::new(State::new(100));
    let intents = Arc::new(Intents::new());
    let cfg = Config {
        deadman: Duration::from_millis(300),
        ..Config::default()
    };
    intents.set_enabled(true);
    intents.set_wheels([100, 200, 300, 400]);
    let (s, i) = (Arc::clone(&state), Arc::clone(&intents));
    let handle = std::thread::spawn(move || run(&mut body, s, i, cfg));
    std::thread::sleep(Duration::from_millis(800));
    state.shutdown.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let wheels: Vec<[i16; 4]> = Decoder::new()
        .push(&bytes)
        .iter()
        .filter(|f| (f.cmd_set, f.cmd_id) == id::CHASSIS_WHEEL_SPEED)
        .map(wheel_rpm)
        .collect();
    // On the wire the left wheels are negated: the PoC convention the robot drove straight with.
    assert_eq!(
        wheels[0],
        [100, -200, -300, 400],
        "the first ticks carry the direct rpm"
    );
    assert!(
        wheels[wheels.len() - 40..].iter().all(|w| *w == [0; 4]),
        "zero after the deadman"
    );
}

/// An armour hit on the wire reaches `s1.event` subscribers as a notification with the plate.
#[test]
fn an_armour_hit_on_the_wire_becomes_an_event() {
    let path = std::env::temp_dir().join(format!("s1d-fake-hit-{}.bin", std::process::id()));
    let mut body = FakeFile::create(&path).unwrap();
    // `ProtoArmorHitEvent`'s bytes: index 3 in the high nibble... as `events_parse_as_the_sdk_unpacks_them`.
    let hit = Frame::request(3, 9, 0, 0x3f, 0x02, vec![0x31, 0x34, 0x12, 0x02, 0x00]);
    body.inbound.extend_from_slice(&hit.encode().unwrap());
    let state = Arc::new(State::new(100));
    let mut events = state.event_tx.subscribe();
    let intents = Arc::new(Intents::new());
    let (s, i) = (Arc::clone(&state), Arc::clone(&intents));
    let handle = std::thread::spawn(move || run(&mut body, s, i, Config::default()));
    std::thread::sleep(Duration::from_millis(100));
    state.shutdown.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(state.armor_hits.load(Ordering::Relaxed), 1);
    match events.try_recv() {
        Ok(s1d::api::Event::ArmorHit {
            plate, hit_kind, ..
        }) => assert_eq!((plate, hit_kind), (3, 1)),
        other => panic!("expected an armour hit event, got {other:?}"),
    }
}

/// `s1.blaster` with three shots: the fire frame goes out first, then every shot gets the
/// stock's own muzzle flash (`gun_led_fire_stock_real.bin`) and the shot sound, on the
/// launcher's cadence (330 ms, measured on the robot). The cadence is read off this clock, from
/// when each start of the player's stand-in appears.
#[test]
fn a_volley_lights_the_muzzle_and_sounds_each_shot_on_the_launchers_cadence() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;
    let dir = std::env::temp_dir().join(format!("s1d-volley-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("shoot.wav"), b"RIFF").unwrap();
    let log = dir.join("starts.log");
    let player = dir.join("aplay");
    std::fs::write(
        &player,
        format!("#!/bin/sh\necho \"$4\" >> \"{}\"\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&player, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = dir.join("wire.bin");
    let mut body = FakeFile::create(&path).unwrap();
    let state = Arc::new(State::new(100));
    *state.sound.lock().unwrap() =
        Some(s1d::sound::Sound::new(dir.clone(), "hw:0,0".to_owned()).with_player(player));
    let intents = Arc::new(Intents::new());
    intents.request_frame(s1_duml::cmd::gun_fire(0, host::GUN, true, 3));
    intents.request_volley(3);
    let (s, i) = (Arc::clone(&state), Arc::clone(&intents));
    let t0 = Instant::now();
    let handle = std::thread::spawn(move || run(&mut body, s, i, Config::default()));
    let mut seen: Vec<Duration> = Vec::new();
    let deadline = t0 + Duration::from_secs(3);
    while seen.len() < 3 && Instant::now() < deadline {
        let n = std::fs::read_to_string(&log)
            .map(|s| s.lines().count())
            .unwrap_or(0);
        while seen.len() < n {
            seen.push(t0.elapsed());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // The third shot's flash goes out 660 ms after the first; wait past it.
    std::thread::sleep(Duration::from_millis(300));
    state.shutdown.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let frames = Decoder::new().push(&bytes);

    assert_eq!(seen.len(), 3, "three shot sounds: {seen:?}");
    // The cadence, from below: the third shot is scheduled 660 ms after the first, and the
    // first no earlier than the loop's start, which is after `t0`. The stand-in's own start-up
    // jitter forbids an upper bound per gap (measured under the suite: 185 ms for one 330 ms
    // gap, the first start late, the second on time).
    assert!(
        seen[2] >= Duration::from_millis(660),
        "the third shot comes no earlier than two cadences after the first: {seen:?}"
    );
    let launcher: Vec<String> = frames
        .iter()
        .filter(|f| f.receiver == host::GUN)
        .map(|f| match (f.cmd_set, f.cmd_id) {
            x if x == id::GUN_FIRE => format!("fire {:02x}", f.data[0]),
            x if x == id::GUN_LED && f.data == s1_duml::cmd::gun_fire_flash(0).data => {
                "flash".to_owned()
            }
            _ => format!("{:02x}/{:02x} {:02x?}", f.cmd_set, f.cmd_id, f.data),
        })
        .collect();
    assert_eq!(
        launcher,
        ["fire 13", "flash", "flash", "flash"],
        "the fire frame, then the stock's muzzle flash once per shot"
    );
}

/// Arming puts the gimbal in its free mode explicitly (`0x04/0x4c 00 00`, DJI's `gimbal_free_mode`),
/// once, beside the chassis work mode: the duck's head is independent of its body, and with the
/// stock stopped the controller would otherwise stay in whatever mode the app left — on 2026-10-30
/// the body followed the gimbal through our whole session (owner's finding). Following is asked
/// for explicitly through `s1.follow`, never inherited.
#[test]
fn arming_frees_the_gimbal_from_the_chassis_once() {
    let path = std::env::temp_dir().join(format!("s1d-fake-arm-{}.bin", std::process::id()));
    let mut body = FakeFile::create(&path).unwrap();
    let state = Arc::new(State::new(100));
    let intents = Arc::new(Intents::new());
    intents.set_enabled(true);
    let (s, i) = (Arc::clone(&state), Arc::clone(&intents));
    let handle = std::thread::spawn(move || run(&mut body, s, i, Config::default()));
    std::thread::sleep(Duration::from_millis(200));
    state.shutdown.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let frames = Decoder::new().push(&bytes);
    let free: Vec<Vec<u8>> = frames
        .iter()
        .filter(|f| (f.cmd_set, f.cmd_id) == id::GIMBAL_WORKMODE)
        .map(|f| f.encode().unwrap())
        .collect();
    assert_eq!(
        free,
        vec![s1_duml::cmd::gimbal_workmode(0, 0).encode().unwrap()],
        "exactly one gimbal work-mode frame on arm, DJI's free mode, byte for byte"
    );
    let chassis_modes = frames
        .iter()
        .filter(|f| (f.cmd_set, f.cmd_id) == id::CHASSIS_WORKMODE)
        .count();
    assert_eq!(chassis_modes, 5, "the chassis work mode as before");
    // And the order: the gimbal is freed after the chassis is put in work mode 1, before any heartbeat.
    let first_heartbeat = frames
        .iter()
        .position(|f| (f.cmd_set, f.cmd_id) == id::HEARTBEAT)
        .expect("armed: the heartbeat runs");
    let free_at = frames
        .iter()
        .position(|f| (f.cmd_set, f.cmd_id) == id::GIMBAL_WORKMODE)
        .unwrap();
    assert!(
        free_at < first_heartbeat,
        "freed before the first heartbeat"
    );
}

/// `s1.goto`: while the controller's position task runs, the loop sends the heartbeat and **no**
/// wheel or follow frame, which would overwrite the task every tick. Tested here, without valid
/// proof on the robot. A twist that asks for motion takes the wheels back.
#[test]
fn a_position_task_owns_the_wheels_until_a_twist_takes_them_back() {
    let path = std::env::temp_dir().join(format!("s1d-fake-goto-{}.bin", std::process::id()));
    let mut body = FakeFile::create(&path).unwrap();
    let state = Arc::new(State::new(100));
    let intents = Arc::new(Intents::new());
    intents.set_enabled(true);
    intents.set_twist([0.0; 3]);
    intents.start_position_task(Duration::from_secs(5));
    let (s, i) = (Arc::clone(&state), Arc::clone(&intents));
    let handle = std::thread::spawn(move || run(&mut body, s, i, Config::default()));
    std::thread::sleep(Duration::from_millis(500));
    let written_during_task = std::fs::metadata(&path).unwrap().len() as usize;
    // A pad at rest streams zero twists: they must not cancel the task.
    intents.set_twist([0.0; 3]);
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        intents.position_task_active(),
        "a zero twist leaves the task alone"
    );
    intents.set_twist([0.3, 0.0, 0.0]);
    assert!(
        !intents.position_task_active(),
        "motion asked: the wheels are the driver's"
    );
    std::thread::sleep(Duration::from_millis(300));
    state.shutdown.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let during = Decoder::new().push(&bytes[..written_during_task]);
    let heartbeats = during
        .iter()
        .filter(|f| (f.cmd_set, f.cmd_id) == id::HEARTBEAT)
        .count();
    assert!(
        heartbeats >= 40,
        "the heartbeat keeps the chassis armed: {heartbeats}"
    );
    let wheel_or_follow = during
        .iter()
        .filter(|f| {
            (f.cmd_set, f.cmd_id) == id::CHASSIS_WHEEL_SPEED
                || (f.cmd_set, f.cmd_id) == id::CHASSIS_FOLLOW
        })
        .count();
    assert_eq!(
        wheel_or_follow, 0,
        "no wheel or follow frame while the task runs"
    );
    let after = Decoder::new().push(&bytes);
    let moving = after
        .iter()
        .filter(|f| (f.cmd_set, f.cmd_id) == id::CHASSIS_WHEEL_SPEED && wheel_rpm(f) != [0; 4])
        .count();
    assert!(
        moving >= 20,
        "the twist drives again once it takes the wheels back: {moving}"
    );
}

/// Where the gimbal leads the chassis, the head target goes out once per change: re-sent every
/// 100 ms with its yaw relative to the chassis (inferred), it would keep putting the target ahead
/// of the body and the body would not stop. No valid proof on the robot. In free mode the hold
/// re-send stays; where the chassis leads, only the pitch axis is ours.
#[test]
fn the_head_target_is_sent_once_where_the_gimbal_leads_and_held_elsewhere() {
    use s1d::intents::FollowMode;
    let rotates = |follow: FollowMode, change: bool| -> Vec<Frame> {
        let path = std::env::temp_dir().join(format!(
            "s1d-fake-lead-{}-{:?}-{change}.bin",
            std::process::id(),
            follow
        ));
        let mut body = FakeFile::create(&path).unwrap();
        let state = Arc::new(State::new(100));
        let intents = Arc::new(Intents::new());
        intents.set_follow(follow);
        intents.set_enabled(true);
        intents.set_head([0.0, 0.0, 45f64.to_radians(), 0.0]);
        let (s, i) = (Arc::clone(&state), Arc::clone(&intents));
        let handle = std::thread::spawn(move || run(&mut body, s, i, Config::default()));
        std::thread::sleep(Duration::from_millis(500));
        if change {
            intents.set_head([0.0, 0.0, -30f64.to_radians(), 0.0]);
            std::thread::sleep(Duration::from_millis(100));
        }
        state.shutdown.store(true, Ordering::Relaxed);
        handle.join().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        Decoder::new()
            .push(&bytes)
            .into_iter()
            .filter(|f| (f.cmd_set, f.cmd_id) == id::GIMBAL_ROTATE)
            .collect()
    };
    let lead = rotates(FollowMode::GimbalLead, false);
    assert_eq!(lead.len(), 1, "gimbal leads: the target once, not held");
    let lead = rotates(FollowMode::GimbalLead, true);
    assert_eq!(
        lead.len(),
        2,
        "gimbal leads: once more when the target changes"
    );
    let free = rotates(FollowMode::Free, false);
    assert!(
        free.len() >= 4,
        "free: the target is held every 100 ms: {}",
        free.len()
    );
    assert!(
        free.iter().all(|f| f.data[2] == 0x1d),
        "free: both axes, coordinate 3"
    );
    let chassis = rotates(FollowMode::ChassisLead, false);
    assert!(
        chassis.iter().all(|f| f.data[2] == 0x1c),
        "chassis leads: the yaw bit is cleared, pitch only"
    );
}

/// `s1.gimbal speed` is a rate: re-sent every tick while fresh (no valid proof on the robot), the
/// angle hold silent meanwhile; `stop` sends one zero rate
/// and the hold resumes; a rate older than the deadman ends the same way.
#[test]
fn a_gimbal_rate_is_streamed_until_stopped_and_the_angle_hold_waits() {
    let path = std::env::temp_dir().join(format!("s1d-fake-rate-{}.bin", std::process::id()));
    let mut body = FakeFile::create(&path).unwrap();
    let state = Arc::new(State::new(100));
    let intents = Arc::new(Intents::new());
    let cfg = Config {
        deadman: Duration::from_millis(300),
        ..Config::default()
    };
    intents.set_enabled(true);
    let (s, i) = (Arc::clone(&state), Arc::clone(&intents));
    let handle = std::thread::spawn(move || run(&mut body, s, i, cfg));
    std::thread::sleep(Duration::from_millis(150));
    let before_rate = std::fs::metadata(&path).unwrap().len() as usize;
    intents.set_gimbal_rate([30.0, -15.0]);
    std::thread::sleep(Duration::from_millis(250));
    let during_rate = std::fs::metadata(&path).unwrap().len() as usize;
    intents.set_gimbal_rate([0.0; 2]);
    std::thread::sleep(Duration::from_millis(250));
    state.shutdown.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    // `(yaw, pitch, ctrl)`: DJI's moving frame carries its `accel_contral_flag` 0xdc, its stop
    // carries 0x00 (`rm_ctrl.py` `_stop_rotate`).
    let rate_frames = |frames: &[Frame]| -> Vec<(i16, i16, u8)> {
        frames
            .iter()
            .filter(|f| (f.cmd_set, f.cmd_id) == id::GIMBAL_SPEED)
            .map(|f| {
                (
                    i16::from_le_bytes([f.data[0], f.data[1]]),
                    i16::from_le_bytes([f.data[4], f.data[5]]),
                    f.data[6],
                )
            })
            .collect()
    };
    let during = Decoder::new().push(&bytes[before_rate..during_rate]);
    let rates = rate_frames(&during);
    assert!(
        rates.len() >= 15,
        "the rate is streamed every tick: {}",
        rates.len()
    );
    assert!(rates.iter().all(|r| *r == (300, -150, 0xdc)), "{rates:?}");
    let holds = during
        .iter()
        .filter(|f| (f.cmd_set, f.cmd_id) == id::GIMBAL_ROTATE)
        .count();
    assert_eq!(holds, 0, "the angle hold is silent while the rate runs");
    let after = Decoder::new().push(&bytes[during_rate..]);
    // The file length was read between two ticks: a last moving frame may sit before the stop.
    let zeros: Vec<_> = rate_frames(&after)
        .into_iter()
        .skip_while(|r| *r == (300, -150, 0xdc))
        .collect();
    assert_eq!(
        zeros,
        vec![(0, 0, 0x00)],
        "stop: one zero rate with DJI's stop flag, then no more"
    );
    let holds_after = after
        .iter()
        .filter(|f| (f.cmd_set, f.cmd_id) == id::GIMBAL_ROTATE)
        .count();
    assert!(holds_after >= 2, "the angle hold resumes: {holds_after}");
}

/// A module's acknowledgement to one of our one-shots lands in `s1.telemetry` with its retcode —
/// the gimbal's answer to `sleep`, for one, which has no valid measurement yet.
#[test]
fn an_ack_to_a_one_shot_is_kept_with_its_retcode() {
    let path = std::env::temp_dir().join(format!("s1d-fake-ack-{}.bin", std::process::id()));
    let mut body = FakeFile::create(&path).unwrap();
    let mut ack = Frame::request(
        host::GIMBAL,
        host::CHASSIS_SENDER,
        5,
        0x04,
        0x0d,
        vec![0x00],
    );
    ack.is_ack = true;
    body.inbound.extend_from_slice(&ack.encode().unwrap());
    let state = Arc::new(State::new(100));
    let intents = Arc::new(Intents::new());
    let (s, i) = (Arc::clone(&state), Arc::clone(&intents));
    let handle = std::thread::spawn(move || run(&mut body, s, i, Config::default()));
    std::thread::sleep(Duration::from_millis(100));
    state.shutdown.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let _ = std::fs::remove_file(&path);
    let t = state.telemetry_result();
    assert_eq!(t.acks, 1);
    assert_eq!(
        t.last_ack,
        Some(s1d::api::AckResult {
            cmd_set: 0x04,
            cmd_id: 0x0d,
            retcode: Some(0)
        })
    );
}

/// A stock push with the given wheel speeds and fused body velocity, as a frame on the wire.
fn stock_push(seq: u16, rpm: [i16; 4], body: [f32; 3]) -> Vec<u8> {
    let mut data = vec![0u8, 1u8];
    data.extend_from_slice(&s1_duml::dds::StockPush::synthesize(
        u32::from(seq) * 20,
        rpm,
        body,
    ));
    Frame::request(
        host::MOTION_CONTROLLER,
        host::BASE,
        seq,
        id::DDS_PUSH.0,
        id::DDS_PUSH.1,
        data,
    )
    .encode()
    .unwrap()
}

/// The wheels-stop rule on the loop, with synthetic pushes (the shape, a discarded session's):
/// driving at 0.15 m/s with the fused velocity agreeing, the wheels turn; the fused velocity
/// running away to 0.9 m/s while the wheels still read 0.15 (the robot in a hand) zeroes the
/// wheels within the trip time and marks `picked_up`; agreement again for the release time gives
/// the wheels back. A disagreement of 0.13 m/s never trips it. Proves the mechanism, not the levels.
#[test]
fn lifted_stops_the_wheels_and_the_floor_does_not() {
    let dir = std::env::temp_dir().join(format!("s1d-fake-lift-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wire = dir.join("wire.bin");
    let inbound = dir.join("in.bin");
    let mut body = FakeFile::create(&wire)
        .unwrap()
        .with_inbound_file(&inbound)
        .unwrap();
    let state = Arc::new(State::new(100));
    let intents = Arc::new(Intents::new());
    intents.set_enabled(true);
    let (s, i) = (Arc::clone(&state), Arc::clone(&intents));
    let handle = std::thread::spawn(move || run(&mut body, s, i, Config::default()));
    // Driving at 0.15 m/s: the rpm, the fused velocity a little above (0.26, a figure of the
    // discarded floor session).
    let mut seq = 0u16;
    let mut push = |rpm: [i16; 4], body: [f32; 3]| {
        seq = seq.wrapping_add(1);
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&inbound)
            .unwrap();
        f.write_all(&stock_push(seq, rpm, body)).unwrap();
    };
    use std::io::Write;
    let drive = |phase_ms: u64, body: [f32; 3], push: &mut dyn FnMut([i16; 4], [f32; 3])| {
        let end = std::time::Instant::now() + Duration::from_millis(phase_ms);
        while std::time::Instant::now() < end {
            intents.set_twist([0.15, 0.0, 0.0]);
            push([27, -26, -33, 31], body);
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    drive(600, [0.26, 0.0, 0.0], &mut push);
    let floor_end = std::fs::metadata(&wire).unwrap().len() as usize;
    assert!(
        !state.lifted.load(Ordering::Relaxed),
        "the floor's worst case is not a lift"
    );
    drive(700, [0.9, 0.0, 0.0], &mut push);
    let lifted_end = std::fs::metadata(&wire).unwrap().len() as usize;
    assert!(
        state.lifted.load(Ordering::Relaxed),
        "0.9 vs 0.15 m/s for 700 ms: lifted"
    );
    assert!(state.frame([0.0; 3], Vec::new()).safety.picked_up);
    drive(900, [0.16, 0.0, 0.0], &mut push);
    assert!(
        !state.lifted.load(Ordering::Relaxed),
        "agreement for 900 ms: released"
    );
    intents.set_twist([0.15, 0.0, 0.0]);
    std::thread::sleep(Duration::from_millis(200));
    state.shutdown.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    let bytes = std::fs::read(&wire).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let wheels = |slice: &[u8]| -> Vec<[i16; 4]> {
        Decoder::new()
            .push(slice)
            .iter()
            .filter(|f| (f.cmd_set, f.cmd_id) == id::CHASSIS_WHEEL_SPEED)
            .map(wheel_rpm)
            .collect()
    };
    let floor = wheels(&bytes[..floor_end]);
    assert!(
        floor.iter().filter(|w| **w != [0; 4]).count() >= 40,
        "on the floor the wheels drive: {}",
        floor.len()
    );
    let lifted = wheels(&bytes[floor_end..lifted_end]);
    // The trip takes 300 ms of pushes (~30 ticks of moving frames), then every frame is zero.
    let tail = &lifted[lifted.len() - 20..];
    assert!(
        tail.iter().all(|w| *w == [0; 4]),
        "lifted: the wheels are held at zero: {:?}",
        &tail[..3]
    );
    let first_zero = lifted
        .iter()
        .position(|w| *w == [0; 4])
        .expect("the wheels stopped");
    assert!(
        first_zero <= 45,
        "the stop comes within the trip time (300 ms + a push period): {first_zero} ticks"
    );
    let after = wheels(&bytes[lifted_end..]);
    assert!(
        after.iter().filter(|w| **w != [0; 4]).count() >= 10,
        "released: the twist drives again: {}",
        after.len()
    );
}
