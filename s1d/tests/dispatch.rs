//! What the socket answers, without a socket: the dispatch table against the contract's types.

use duck_ipc_proto as proto;
use s1d::intents::Intents;
use s1d::ipc::dispatch;
use s1d::state::State;

fn call(state: &State, intents: &Intents, c: proto::Call) -> proto::Response {
    dispatch(state, intents, proto::Id::Number(1), &c)
}

fn result_json(r: &proto::Response) -> serde_json::Value {
    serde_json::to_value(r).unwrap()["result"].clone()
}

/// `netpadd` picks the wheel limits only on this answer; a biped's gait limits on a mecanum base
/// is the bug the first plan review caught.
#[test]
fn mode_is_roller() {
    let (s, i) = (State::new(100), Intents::new());
    assert_eq!(
        result_json(&call(&s, &i, proto::Call::RobotMode))["mode"],
        "roller"
    );
}

/// The pad-loss handler stops asking for a sit only when the refusal says "not driving".
#[test]
fn do_is_refused_with_not_driving() {
    let (s, i) = (State::new(100), Intents::new());
    let r = result_json(&call(
        &s,
        &i,
        proto::Call::RobotDo(proto::DoParams {
            skill: "sit_toggle".to_owned(),
        }),
    ));
    assert_eq!(r["accepted"], false);
    assert!(r["reason"].as_str().unwrap().contains("not driving"));
}

#[test]
fn enable_toggles_and_arms() {
    let (s, i) = (State::new(100), Intents::new());
    let r = result_json(&call(
        &s,
        &i,
        proto::Call::RobotEnable(proto::EnableParams {
            on: false,
            toggle: true,
        }),
    ));
    assert_eq!(r["accepted"], true);
    assert!(i.enabled());
    assert_eq!(
        i.take_power_request(),
        Some(s1d::intents::PowerRequest::Arm)
    );
}

#[test]
fn a_route_that_is_not_ours_is_method_not_found() {
    let (s, i) = (State::new(100), Intents::new());
    let r = serde_json::to_value(call(&s, &i, proto::Call::NetStatus)).unwrap();
    assert_eq!(r["error"]["code"], proto::code::METHOD_NOT_FOUND);
}

#[test]
fn health_before_the_first_tick_is_not_healthy_but_answers() {
    let (s, i) = (State::new(100), Intents::new());
    let r = result_json(&call(&s, &i, proto::Call::RobotHealth));
    assert_eq!(r["healthy"], false);
    assert!(r["reason"].as_str().unwrap().contains("not completed"));
}

/// Every call `robotd` would answer is answered here too — with the same shape, or a refusal that
/// names the reason. "Method not found" is for routes that are not the robot's at all; eleven
/// robot routes fell through to it in the first cut, which the cold reader caught.
#[test]
fn every_robot_route_is_served_or_refused_with_a_reason() {
    let (s, i) = (State::new(100), Intents::new());
    for c in proto::test_support::every_call() {
        if !matches!(c.destination(), Some((proto::Service::Robot, _))) {
            continue;
        }
        let method = c.method();
        let r = serde_json::to_value(call(&s, &i, c)).unwrap();
        assert_ne!(
            r["error"]["code"],
            proto::code::METHOD_NOT_FOUND,
            "{method} is the robot's and fell through to METHOD_NOT_FOUND"
        );
    }
}

/// `toggle` flips what the last call left, whichever way that call spelled it. The pad's HOME
/// button only ever toggles, and `robotctl flashlight off` only ever sets — so a toggle that
/// inverted its own private count would switch the LEDs "off" when they were already off.
#[test]
fn flashlight_toggle_inverts_what_on_and_off_left() {
    let (s, i) = (State::new(100), Intents::new());
    let fl = |on, toggle| {
        proto::Call::RobotFlashlight(proto::FlashlightParams {
            on,
            toggle,
            color: proto::FlashlightColor::White,
        })
    };
    call(&s, &i, fl(true, false));
    i.take_led();
    call(&s, &i, fl(false, true));
    let led = i.take_led().expect("a LED request");
    assert_eq!(led.effect, 0, "on, then toggle, must switch the LEDs off");
    call(&s, &i, fl(false, false));
    i.take_led();
    call(&s, &i, fl(false, true));
    let led = i.take_led().expect("a LED request");
    assert_ne!(led.effect, 0, "off, then toggle, must switch the LEDs on");
}

/// The bench's own routes: a frame given as hex is queued for the loop, bad hex and a bad mode are
/// refused with a reason, and an unknown `s1.` route is "method not found".
#[test]
fn bench_routes_queue_frames_and_refuse_junk() {
    use s1d::intents::FollowMode;
    use s1d::ipc::bench;
    let i = Intents::new();
    let id = proto::Id::Number(1);
    let frame = s1_duml::cmd::gimbal_workmode(0, 2).encode().unwrap();
    let hex: String = frame.iter().map(|b| format!("{b:02x}")).collect();
    let r = result_json(&bench(
        &i,
        id.clone(),
        "s1.frame",
        Some(&serde_json::json!({"hex": hex})),
    ));
    assert_eq!(r["accepted"], true);
    assert_eq!(
        i.take_frames().len(),
        1,
        "the frame is waiting for the loop"
    );

    let r = result_json(&bench(
        &i,
        id.clone(),
        "s1.frame",
        Some(&serde_json::json!({"hex": "55zz"})),
    ));
    assert_eq!(r["accepted"], false);
    let r = result_json(&bench(
        &i,
        id.clone(),
        "s1.frame",
        Some(&serde_json::json!({"hex": "5500"})),
    ));
    assert_eq!(r["accepted"], false, "two bytes are not a frame");
    assert!(i.take_frames().is_empty());

    let r = result_json(&bench(
        &i,
        id.clone(),
        "s1.mode",
        Some(&serde_json::json!({"mode": "chassis_lead"})),
    ));
    assert_eq!(r["accepted"], true);
    assert_eq!(i.follow(), FollowMode::ChassisLead);
    let queued = i.take_frames();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        (queued[0].cmd_set, queued[0].cmd_id),
        s1_duml::cmd::id::GIMBAL_WORKMODE
    );
    assert_eq!(queued[0].data, vec![2, 0]);

    let r = result_json(&bench(
        &i,
        id.clone(),
        "s1.mode",
        Some(&serde_json::json!({"mode": "tank"})),
    ));
    assert_eq!(r["accepted"], false);
    let r = serde_json::to_value(bench(&i, id, "s1.nothing", None)).unwrap();
    assert_eq!(r["error"]["code"], proto::code::METHOD_NOT_FOUND);
}
