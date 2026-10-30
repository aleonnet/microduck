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
