//! The motion controller's own telemetry, as the stock firmware subscribed it at boot, decoded from
//! frames read off the UART (`out/2026-10-30-poc2/capture/hdvt_uav.strace`, fd 16, stock alive) and
//! from the 2026-10-29 idle capture. Every expected value below was computed independently of this
//! crate (Python over the same bytes, 2026-10-30 (night)) and is a physical reading: a battery between 11
//! and 12 V, gravity of one g on the z axis, a tilt bit set exactly when the pitch is −6.6°.

use s1_duml::Frame;
use s1_duml::dds::{BlockOrder, PushPeriod, StockPush};
use s1_duml::events::GimbalPush;

fn frame(bytes: &[u8]) -> Frame {
    Frame::decode(bytes).expect("a whole frame with both CRCs")
}

fn stock(bytes: &[u8]) -> StockPush {
    let f = frame(bytes);
    let push = PushPeriod::parse(&f).expect("a 0x48/0x08 push");
    StockPush::parse(push.data).expect("the stock's 161-byte layout")
}

#[test]
fn the_idle_push_of_2026_10_30_reads_the_battery_gravity_and_a_still_robot() {
    let p = stock(include_bytes!(
        "fixtures/dds_push_stock_idle_2026-10-30.bin"
    ));
    assert_eq!(p.time_ms, 1_206_430);
    assert_eq!(p.battery.adc_value, 11_940, "millivolts");
    assert_eq!(p.battery.temperature, 316, "tenths of a degree");
    assert_eq!(p.battery.current, -1_060, "milliamps, discharging");
    assert_eq!(p.battery.percent, 80);
    assert_eq!(p.esc.speed_rpm, [-5, 1, 2, 0], "idle jitter");
    assert_eq!(p.esc.angle, [27_692, 8_155, 6_572, 27_743]);
    assert!(
        (p.imu.acc[2] + 1.0039).abs() < 1e-3,
        "one g down: {}",
        p.imu.acc[2]
    );
    assert!(p.sa_status.static_flag && !p.sa_status.on_slope);
    assert!((p.attitude.yaw + 0.695).abs() < 1e-3);
    assert!((p.position.x + 0.1121).abs() < 1e-4);
    assert_eq!(p.gimbal_yaw_deg, 0.0);
}

#[test]
fn the_moving_push_carries_body_speed_turn_rate_and_no_static_bit() {
    let p = stock(include_bytes!(
        "fixtures/dds_push_stock_moving_2026-10-30.bin"
    ));
    assert!((p.velocity.body[0] - 0.437).abs() < 1e-3, "m/s forward");
    assert!((p.imu.gyro[2] + 0.5839).abs() < 1e-3, "rad/s, turning");
    assert_eq!(p.esc.speed_rpm, [-34, -50, -27, -55]);
    assert!(!p.sa_status.static_flag);
    assert_eq!(p.battery.percent, 78);
    assert_eq!(p.gimbal_yaw_deg, -1.0);
}

#[test]
fn the_turned_push_shows_the_chassis_yaw_the_owner_drove() {
    let p = stock(include_bytes!(
        "fixtures/dds_push_stock_turned_2026-10-30.bin"
    ));
    assert!((p.attitude.yaw - 19.329).abs() < 1e-3, "degrees");
    assert_eq!(p.gimbal_yaw_deg, -178.0);
    assert_eq!(p.battery.adc_value, 11_819);
}

/// The 2026-10-29 capture, robot parked on a slope: pitch −6.6° and the controller's own
/// "on slope" bit set in the same push — two fields agreeing is what validates the offsets.
#[test]
fn the_2026_10_29_push_agrees_with_itself_about_the_slope() {
    let p = stock(include_bytes!("fixtures/dds_push_176_real.bin"));
    assert_eq!(p.battery.adc_value, 11_862);
    assert_eq!(p.battery.temperature, 332);
    assert_eq!(p.battery.current, -1_129);
    assert_eq!(p.battery.percent, 77);
    assert!(
        (p.attitude.pitch + 6.6).abs() < 0.05,
        "{}",
        p.attitude.pitch
    );
    assert!(p.sa_status.on_slope && p.sa_status.static_flag);
    assert!((p.imu.acc[2] + 0.998).abs() < 1e-3);
}

/// Anything but 161 bytes of subjects is not this push: the 1 Hz one (54 bytes) and a truncation.
/// The 46..92 block holds two subjects, the ESC's 36 bytes and the battery's 10, and their
/// ORDER differs between boots: every push of the 17:05 capture and the 29/10 one read
/// `esc, battery` (msg ids 0 and 1), every push after 20:50 — the hand relaunch, two clean boots,
/// with and without the app, 3080 pushes — read `battery, esc`, with the pack at 10.7–12.2 V and
/// the wheels at idle, and the other order implausible in all 5442 pushes (2026-10-30, 23:xx,
/// Python over every capture). The msg id does not tell the orders apart; the content does. This
/// fixture is one such push, cut from `telemetry-free-20261030-221150/s1d_tilt_reads.strace` (22:19): the robot on
/// since 20:50, 29 % and 10.675 V — the "10 675 rpm" the other order read; at 82 it would be 49 mV at 0 %.
#[test]
fn the_other_boots_put_the_battery_before_the_esc_and_the_layout_reads_both_orders() {
    let p = stock(include_bytes!(
        "fixtures/dds_push_stock_relaunched_2026-10-30.bin"
    ));
    assert_eq!(p.block_order, Some(BlockOrder::BatteryThenEsc));
    assert!(
        (p.imu.acc[2] + 1.0).abs() < 0.02,
        "one g down: {}",
        p.imu.acc[2]
    );
    assert!((p.attitude.yaw - 19.9).abs() < 0.1, "{}", p.attitude.yaw);
    assert!(p.sa_status.static_flag);
    assert_eq!(p.battery.adc_value, 10_675, "millivolts, the pack at 29 %");
    assert_eq!(p.battery.percent, 29);
    assert_eq!(p.battery.temperature, 304, "30.4 °C");
    assert_eq!(p.battery.current, -1_107, "milliamps, discharging");
    assert!(p.battery.plausible() && p.esc.plausible());
    assert!(
        p.esc.speed_rpm.iter().all(|r| r.abs() <= 5),
        "idle: {:?}",
        p.esc.speed_rpm
    );
    assert!(
        p.esc.timestamp.iter().max().unwrap() - p.esc.timestamp.iter().min().unwrap() < 100,
        "four ESC clocks within a tick: {:?}",
        p.esc.timestamp
    );
    // the captures the layout was first read from are the other order, unchanged
    let idle = stock(include_bytes!(
        "fixtures/dds_push_stock_idle_2026-10-30.bin"
    ));
    assert_eq!(idle.block_order, Some(BlockOrder::EscThenBattery));
    assert_eq!(idle.battery.adc_value, 11_940);
    let real = stock(include_bytes!("fixtures/dds_push_176_real.bin"));
    assert_eq!(
        real.block_order,
        Some(BlockOrder::EscThenBattery),
        "29/10, msg id 1"
    );
    assert_eq!(real.battery.adc_value, 11_862);
}

/// 161 zero bytes are a push of the right length whose block reads as nothing in either order:
/// no order is claimed, and what the daemon publishes from it is decided by the plausibility
/// guards, not by the layout.
#[test]
fn a_block_plausible_in_neither_order_claims_no_order() {
    let p = StockPush::parse(&[0u8; StockPush::LEN]).expect("the length is the stock's");
    assert_eq!(p.block_order, None);
    assert!(!p.battery.plausible());
}

#[test]
fn other_lengths_are_refused() {
    let f = frame(include_bytes!("fixtures/dds_push_stock_1hz_2026-10-30.bin"));
    let push = PushPeriod::parse(&f).unwrap();
    assert_eq!(push.data.len(), 54);
    assert!(StockPush::parse(push.data).is_none());
    let f = frame(include_bytes!(
        "fixtures/dds_push_stock_idle_2026-10-30.bin"
    ));
    let push = PushPeriod::parse(&f).unwrap();
    assert!(StockPush::parse(&push.data[..160]).is_none());
    assert_eq!(StockPush::LEN, 161);
}

/// `0x04/0x05`, the gimbal's own attitude push (gimbal → 0xf1, ~21 Hz with the stock alive):
/// pitch and yaw in tenths of a degree, and a quaternion that agrees with the yaw. The limits the
/// manual gives (pitch −20°..+35°, yaw ±250°) bound every value, and the two extreme fixtures sit
/// on the pitch limit.
#[test]
fn gimbal_pushes_read_pitch_yaw_and_a_quaternion_that_agrees() {
    let first = GimbalPush::parse(&frame(include_bytes!(
        "fixtures/gimbal_push_first_2026-10-30.bin"
    )))
    .expect("a 0x04/0x05 of 40 bytes");
    assert_eq!(
        (first.pitch_ddeg, first.yaw_ddeg, first.yaw_vs_chassis_ddeg),
        (0, 2, -1)
    );
    assert_eq!(first.time_ms, 1_205_640);
    assert!((first.quat[0] - 0.99998).abs() < 1e-4, "w ≈ 1 at rest");

    let yaw_max = GimbalPush::parse(&frame(include_bytes!(
        "fixtures/gimbal_push_yaw_max_2026-10-30.bin"
    )))
    .unwrap();
    assert_eq!(
        (
            yaw_max.pitch_ddeg,
            yaw_max.yaw_ddeg,
            yaw_max.yaw_vs_chassis_ddeg
        ),
        (356, -1796, 1583)
    );
    // A rotation of −179.6° about z: w ≈ cos(−89.8°) ≈ 0.006, z ≈ sin(−89.8°) ≈ −1.
    assert!((yaw_max.quat[0] - 0.0062).abs() < 1e-3 && (yaw_max.quat[3] + 0.95178).abs() < 1e-3);
    assert!((yaw_max.yaw_deg() + 179.6).abs() < 1e-6);

    let pitch_max = GimbalPush::parse(&frame(include_bytes!(
        "fixtures/gimbal_push_pitch_max_2026-10-30.bin"
    )))
    .unwrap();
    assert_eq!((pitch_max.pitch_ddeg, pitch_max.yaw_ddeg), (358, 1471));
    assert!(
        (pitch_max.pitch_deg() - 35.8).abs() < 1e-6,
        "at the manual's +35° limit"
    );

    // Not a gimbal push: the telemetry frame.
    assert!(
        GimbalPush::parse(&frame(include_bytes!(
            "fixtures/dds_push_stock_idle_2026-10-30.bin"
        )))
        .is_none()
    );
}
