//! DJI's own on-board library as the oracle.
//!
//! The robot carries the Scratch/Python runtime (`/system/firm/dji_scratch/src/robomaster/`,
//! pulled 2026-10-30). Its `rm_module.py` builds every command the body accepts and
//! `duss_event_msg.py` packs it; both run unchanged on the Mac. Each hex below is that packer's
//! output for the arguments in the test's name (`scripts/s1/dji-oracle.py`, over the lab's copy),
//! sender `scratch_script_id` (`0xC9`), seq 0. Our builders must reproduce them byte for byte:
//! same receiver, same ack request, same payload.

use s1_duml::cmd::{self, host};
use s1_duml::dds;

fn hex(frame: &s1_duml::Frame) -> String {
    frame
        .encode()
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn chassis_follow_matches_the_library() {
    assert_eq!(
        hex(&cmd::chassis_follow(0, 0.0, 0.0, 45.0)),
        "551904e4c9c30000003f220000000000000000000034421a1f"
    );
}

#[test]
fn chassis_stick_overlay_matches_the_library() {
    assert_eq!(
        hex(&cmd::chassis_stick_overlay(0, 0)),
        "550e0466c9c30000403f2800cc8d"
    );
    assert_eq!(
        hex(&cmd::chassis_stick_overlay(0, 1)),
        "550e0466c9c30000403f2801459c"
    );
    assert_eq!(
        hex(&cmd::chassis_stick_overlay(0, 2)),
        "550e0466c9c30000403f2802deae"
    );
}

#[test]
fn chassis_position_and_its_stop_match_the_library() {
    assert_eq!(
        hex(&cmd::chassis_position(0, 7, 0, 100, -50, 900, 50, 300)),
        "551a04b1c9c30000403f25070800006400ceff8403322c016440"
    );
    assert_eq!(
        hex(&cmd::chassis_position_stop(0, 7)),
        "55170438c9c30000403f2507010000000000000000406c"
    );
}

#[test]
fn chassis_pwm_matches_the_library() {
    assert_eq!(
        hex(&cmd::chassis_pwm_value(0, 0x3f, [500; 6])),
        "551a04b1c9c30000403f3c3ff401f401f401f401f401f401e052"
    );
    assert_eq!(
        hex(&cmd::chassis_pwm_freq(0, 0x3f, [1000; 6])),
        "551a04b1c9c30000403f2b3fe803e803e803e803e803e803ae19"
    );
}

#[test]
fn gimbal_workmode_matches_the_library() {
    assert_eq!(
        hex(&cmd::gimbal_workmode(0, 0)),
        "550f04a2c904000040044c00007f12"
    );
    assert_eq!(
        hex(&cmd::gimbal_workmode(0, 1)),
        "550f04a2c904000040044c0100a70b"
    );
    assert_eq!(
        hex(&cmd::gimbal_workmode(0, 2)),
        "550f04a2c904000040044c0200cf21"
    );
}

#[test]
fn gimbal_recenter_matches_the_library() {
    assert_eq!(
        hex(&cmd::gimbal_recenter(0, 7, 100, 100)),
        "551604fcc9040000403fb2070805640000006400bfa0"
    );
}

#[test]
fn gimbal_degree_matches_the_library() {
    // coordinate 1 (relative to where it is), both axes, yaw +30.0°, pitch −10.0°, 120 °/s.
    assert_eq!(
        hex(&cmd::gimbal_degree(0, 7, 1, 0x05, 300, -100, 120, 120)),
        "551e048ac9040000403fb007080d2c0100009cff00007800000078001cb4"
    );
}

#[test]
fn gimbal_speed_matches_the_library() {
    assert_eq!(
        hex(&cmd::gimbal_speed(0, 300, -150, 0xdc)),
        "5514046dc904000000040c2c0100006affdc8c4f"
    );
}

#[test]
fn gimbal_suspend_and_resume_match_the_library() {
    assert_eq!(
        hex(&cmd::gimbal_suspend_resume(0, true)),
        "550f04a2c904000020040db52ae825"
    );
    assert_eq!(
        hex(&cmd::gimbal_suspend_resume(0, false)),
        "550f04a2c904000020040df27e273a"
    );
}

#[test]
fn gimbal_subscribe_position_matches_the_library() {
    assert_eq!(
        hex(&cmd::gimbal_subscribe_position(0, true)),
        "550f04a2c904000040041e05ffe455"
    );
}

#[test]
fn gun_led_matches_the_library() {
    assert_eq!(
        hex(&cmd::gun_led(0, 1, 0, 255, 255, 255, 1, 100, 100)),
        "551604fcc9170000403f5510ffffff0164006400767c"
    );
    // `GunCtrl.set_led(mode)` is control 7 with rm_define's effect code: on, off, breath, flash,
    // marquee — what `s1.lights --blaster` sends.
    for (mode, want) in [
        (0, "551604fcc9170000403f5570ffffff01640064002c6d"),
        (1, "551604fcc9170000403f5571ffffff0164006400d120"),
        (2, "551604fcc9170000403f5572ffffff0164006400d6f6"),
        (3, "551604fcc9170000403f5573ffffff01640064002bbb"),
        (4, "551604fcc9170000403f5574ffffff0164006400c952"),
    ] {
        assert_eq!(
            hex(&cmd::gun_led(0, 7, mode, 255, 255, 255, 1, 100, 100)),
            want,
            "mode {mode}"
        );
    }
}

#[test]
fn armor_sensitivity_matches_the_library() {
    // The library addresses it to the stock controller, host 0x09.
    assert_eq!(
        hex(&cmd::armor_sensitivity(0, host::BASE, 1.0)),
        "5520047bc9090000403f073ff4012c0132000d0006000100a000b400c8000964"
    );
}

#[test]
fn node_reset_matches_the_sdk_layout() {
    let f = dds::reset_node(0, 9);
    assert_eq!((f.cmd_set, f.cmd_id), cmd::id::DDS_RESET_NODE);
    assert_eq!(f.data, vec![9]);
    assert_eq!(f.ack, s1_duml::Ack::WhenDone);
}

/// The status word is four bytes with bit 9 the roll-over, bit 4 picked up — as DJI's own library
/// reads the push; a two-byte reading would put roll-over in the wrong byte.
#[test]
fn sa_status_is_a_four_byte_word() {
    let word = (1u32 << 4) | (1 << 9);
    let bytes = [word.to_le_bytes(), [0xaa; 4]].concat();
    let (s, rest) = dds::SaStatus::parse(&bytes).unwrap();
    assert!(s.is_pick_up && s.roll_over && !s.static_flag && !s.impact_z);
    assert_eq!(rest, &[0xaa; 4]);
    assert!(
        dds::SaStatus::parse(&[0; 3]).is_none(),
        "three bytes are not a status"
    );
}

#[test]
fn gimbal_pos_reads_pitch_and_yaw_where_the_library_does() {
    // bytes 2..4 pitch, 4..6 yaw, tenths of a degree.
    let mut b = [0u8; 9];
    b[2..4].copy_from_slice(&(-150i16).to_le_bytes());
    b[4..6].copy_from_slice(&(900i16).to_le_bytes());
    let (g, _) = dds::GimbalPos::parse(&b).unwrap();
    assert_eq!((g.pitch_ddeg, g.yaw_ddeg), (-150, 900));
    let (s, _) = dds::Stick::parse(&[0b10]).unwrap();
    assert!(s.gimbal && !s.chassis);
}

/// The LED frame the stock app writes, rebuilt with DJI's packer from the captured layout, equals
/// what `system_led` encodes — the two sources of this crate agree on the one command both cover.
#[test]
fn system_led_solid_matches_the_oracles_rebuild_of_the_captured_layout() {
    assert_eq!(
        hex(&cmd::system_led(0, &cmd::SystemLed::solid(255, 0, 0))),
        "551a04b109180000003f3201ff00ff000000000000003f0084d3"
    );
}

#[test]
fn gun_fire_matches_the_library() {
    // The library's `fire_once` in mode 1 ("single"), one shot, to the stock controller.
    assert_eq!(
        hex(&cmd::gun_fire(0, host::BASE, true, 1)),
        "550e0466c9090000403f51115403"
    );
}
