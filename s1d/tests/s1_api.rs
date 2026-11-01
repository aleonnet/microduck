//! The S1's own list, end to end without a socket: every call parses and refuses an unknown
//! member by name, and every one-shot that moves something queues the frame DJI's own library
//! would send — the oracle hex of `scripts/s1/dji-oracle.py`, byte for byte. The continuous ones
//! (`wheels`, the gimbal rate) set the slot the loop streams from, judged in `fake_body.rs`.

use duck_ipc_proto as proto;
use s1d::api::{self, S1Call};
use s1d::intents::{FollowMode, Intents};
use s1d::ipc::{dispatch_s1, event_notification};
use s1d::state::State;
use serde_json::{Value, json};

fn hex(frame: &s1_duml::Frame) -> String {
    frame
        .encode()
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn run(state: &State, intents: &Intents, call: &S1Call) -> Value {
    let r = dispatch_s1(state, intents, proto::Id::Number(1), call);
    serde_json::to_value(r).unwrap()
}

/// Queued frames, re-sequenced to 0 as the loop would re-sequence them, as hex.
fn queued(intents: &Intents) -> Vec<String> {
    intents
        .take_frames()
        .into_iter()
        .map(|mut f| {
            f.seq = 0;
            hex(&f)
        })
        .collect()
}

#[test]
fn an_unknown_params_member_is_refused_by_name() {
    for call in S1Call::every_call() {
        let Value::Object(mut params) = call.params() else {
            panic!("{} params are not an object", call.method())
        };
        if params.is_empty() {
            continue;
        }
        params.insert("from_a_later_release".to_owned(), Value::Bool(true));
        let err = S1Call::parse(call.method(), Some(&Value::Object(params)))
            .expect("an s1 method")
            .expect_err(&format!("{} accepted an unknown member", call.method()));
        assert_eq!(err.code, proto::code::INVALID_PARAMS, "{}", call.method());
        assert!(
            err.message.contains("from_a_later_release"),
            "{}: {}",
            call.method(),
            err.message
        );
    }
}

#[test]
fn every_call_round_trips_and_other_lists_are_left_alone() {
    for call in S1Call::every_call() {
        let back = S1Call::parse(call.method(), Some(&call.params()))
            .unwrap()
            .unwrap();
        assert_eq!(back, call, "{}", call.method());
    }
    assert!(
        S1Call::parse("robot.move", None).is_none(),
        "the duck's list is not ours"
    );
    let err = S1Call::parse("s1.teleport", None).unwrap().unwrap_err();
    assert_eq!(err.code, proto::code::METHOD_NOT_FOUND);
    assert!(err.message.contains("version 1"));
}

#[test]
fn gimbal_calls_queue_the_librarys_frames() {
    let (s, i) = (State::new(100), Intents::new());
    let r = run(
        &s,
        &i,
        &S1Call::Gimbal(api::GimbalAction::Recenter {
            yaw_speed: 100,
            pitch_speed: 100,
        }),
    );
    assert_eq!(r["result"]["accepted"], true);
    assert_eq!(
        queued(&i),
        vec!["551604fcc9040000403fb2070805640000006400bfa0"]
    );

    // A rate is a continuous intent the loop streams, so the call queues nothing and sets the
    // slot; `stop` is the zero rate.
    run(
        &s,
        &i,
        &S1Call::Gimbal(api::GimbalAction::Speed {
            yaw: 30.0,
            pitch: -15.0,
        }),
    );
    assert!(
        queued(&i).is_empty(),
        "a rate is streamed by the loop, not queued once"
    );
    let snap = i.snapshot();
    assert_eq!(snap.gimbal_rate, [30.0, -15.0]);
    assert!(snap.gimbal_rate_age < std::time::Duration::from_secs(1));
    run(&s, &i, &S1Call::Gimbal(api::GimbalAction::Stop));
    assert_eq!(i.snapshot().gimbal_rate, [0.0; 2]);
    assert!(queued(&i).is_empty());

    run(&s, &i, &S1Call::Gimbal(api::GimbalAction::Sleep));
    run(&s, &i, &S1Call::Gimbal(api::GimbalAction::Wake));
    assert_eq!(
        queued(&i),
        vec![
            "550f04a2c904000020040db52ae825",
            "550f04a2c904000020040df27e273a"
        ]
    );
}

/// `s1.goto` in metres and degrees becomes the library's centimetres and tenths, with the yaw
/// negated as the library negates it (counter-clockwise positive on our side).
#[test]
fn goto_queues_the_position_task_and_its_stop() {
    let (s, i) = (State::new(100), Intents::new());
    run(
        &s,
        &i,
        &S1Call::Goto(api::GotoParams {
            x: 1.0,
            y: -0.5,
            yaw: -90.0,
            speed: 0.5,
            yaw_speed: 30.0,
            stop: false,
        }),
    );
    assert_eq!(
        queued(&i),
        vec!["551a04b1c9c30000403f25070800006400ceff8403322c016440"]
    );
    // The controller's task owns the wheels until it could have finished: 1.12 m at 0.5 m/s plus
    // 90° at 30 °/s plus two seconds of slack, well within the hour the test allows.
    assert!(i.position_task_active(), "the wheels are the controller's");
    run(
        &s,
        &i,
        &S1Call::Goto(api::GotoParams {
            stop: true,
            ..api::GotoParams {
                x: 0.0,
                y: 0.0,
                yaw: 0.0,
                speed: 0.5,
                yaw_speed: 30.0,
                stop: true,
            }
        }),
    );
    assert_eq!(
        queued(&i),
        vec!["55170438c9c30000403f2507010000000000000000406c"]
    );
    assert!(!i.position_task_active(), "--stop gives the wheels back");
}

#[test]
fn follow_sets_the_mode_and_queues_gimbal_mode_and_velocity_frame() {
    let (s, i) = (State::new(100), Intents::new());
    run(
        &s,
        &i,
        &S1Call::Follow(api::FollowParams {
            mode: api::Follow::ChassisLead,
            velocity_frame: api::VelocityFrame::Gimbal,
            gimbal_fpv: false,
        }),
    );
    assert_eq!(i.follow(), FollowMode::ChassisLead);
    assert_eq!(
        queued(&i),
        vec![
            "550f04a2c904000040044c0200cf21",
            "550e0466c9c30000403f2802deae"
        ]
    );
}

#[test]
fn pwm_lights_armor_and_blaster_queue_what_the_library_sends() {
    let (s, i) = (State::new(100), Intents::new());
    run(
        &s,
        &i,
        &S1Call::Pwm(api::PwmParams {
            mask: 0x3f,
            duty: Some(500),
            hz: Some(1000),
        }),
    );
    assert_eq!(
        queued(&i),
        vec![
            "551a04b1c9c30000403f2b3fe803e803e803e803e803e803ae19",
            "551a04b1c9c30000403f3c3ff401f401f401f401f401f401e052"
        ]
    );
    run(
        &s,
        &i,
        &S1Call::Lights(api::LightsParams {
            effect: api::LightEffect::Solid,
            r: 255,
            g: 255,
            b: 255,
            t1: 100,
            t2: 100,
            groups: 0x3f,
            blaster: true,
        }),
    );
    assert_eq!(
        queued(&i),
        vec!["551604fcc9170000403f5570ffffff01640064002c6d"]
    );
    // Body lights go through the LED slot, as `robot.flashlight` does, in the captured layout.
    run(
        &s,
        &i,
        &S1Call::Lights(api::LightsParams {
            effect: api::LightEffect::Breath,
            r: 0,
            g: 0,
            b: 255,
            t1: 500,
            t2: 500,
            groups: 0x3f,
            blaster: false,
        }),
    );
    let led = i.take_led().expect("an LED effect waits for the loop");
    assert_eq!(
        hex(&s1_duml::cmd::system_led(0, &led)),
        "551a04b109180000003f3202ff000000ff00f401f4013f00bc8b"
    );
    // Armour thresholds: the library's bytes, addressed to the armour host.
    run(&s, &i, &S1Call::Armor(api::ArmorParams { k: 1.0 }));
    let f = i.take_frames().remove(0);
    assert_eq!((f.receiver, f.cmd_set, f.cmd_id), (0x18, 0x3f, 0x07));
    assert_eq!(
        hex(&s1_duml::cmd::armor_sensitivity(0, 9, 1.0)),
        "5520047bc9090000403f073ff4012c0132000d0006000100a000b400c8000964"
    );
    // The blaster: infrared by default, gel refused without the key.
    let r = run(
        &s,
        &i,
        &S1Call::Blaster(api::BlasterParams {
            times: 1,
            gel: true,
        }),
    );
    assert_eq!(r["result"]["accepted"], false);
    assert!(
        r["result"]["reason"]
            .as_str()
            .unwrap()
            .contains("--blaster-gel")
    );
    assert_eq!(
        i.take_volley(),
        None,
        "a refused fire lights and sounds nothing"
    );
    run(
        &s,
        &i,
        &S1Call::Blaster(api::BlasterParams {
            times: 3,
            gel: false,
        }),
    );
    let f = i.take_frames().remove(0);
    assert_eq!(
        (f.receiver, f.cmd_id, f.data.clone()),
        (0x17, 0x51, vec![0x13])
    );
    // The loop dresses each of the three shots with the muzzle light and the shot sound.
    assert_eq!(i.take_volley(), Some(3));
    assert_eq!(i.take_volley(), None, "taken once");
}

#[test]
fn wheels_volume_version_and_telemetry_answer() {
    let (s, i) = (State::new(100), Intents::new());
    let r = run(
        &s,
        &i,
        &S1Call::Wheels(api::WheelsParams {
            rpm: [1200, 0, 0, 0],
        }),
    );
    assert_eq!(r["result"]["accepted"], false);
    run(
        &s,
        &i,
        &S1Call::Wheels(api::WheelsParams {
            rpm: [100, -100, 50, -50],
        }),
    );
    assert_eq!(i.snapshot().wheels, Some([100, -100, 50, -50]));
    let r = run(&s, &i, &S1Call::Volume(api::VolumeParams { percent: 50 }));
    assert_eq!(r["result"]["accepted"], false, "no mixer control named yet");
    let r = run(&s, &i, &S1Call::Version);
    assert_eq!(r["result"]["s1_api_version"], api::S1_API_VERSION);
    let r = run(&s, &i, &S1Call::Telemetry);
    assert_eq!(r["result"]["pushes"], 0);
    assert!(
        r["result"].get("battery").is_none(),
        "nothing pushed yet: no battery"
    );
    let n = serde_json::to_value(event_notification(&api::Event::ArmorHit {
        plate: 3,
        hit_kind: 1,
        mic_value: 500,
    }))
    .unwrap();
    assert_eq!(n["method"], "s1.event");
    assert_eq!(n["params"]["kind"], "armor_hit");
    assert!(n.get("id").is_none());
    let _ = json!({});
}
