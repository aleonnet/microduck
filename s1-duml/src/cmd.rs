//! The commands the S1's body obeys, as frames.
//!
//! Three provenances, kept apart on purpose:
//! - **Moved the real robot** (2026-10-29): [`chassis_workmode`], [`chassis_wheel_speed`],
//!   [`gimbal_angles`] and [`heartbeat_proroklab`] — byte for byte the lab's `s1_duml.py`, which came
//!   from proroklab's `chassis.cpp`/`gimbal.cpp`. `tests/fixtures/poc_*.bin` are those exact bytes.
//! - **Seen on the wire** from the stock firmware: [`heartbeat_stock`] (`tests/fixtures/heartbeat_real.bin`).
//! - **Never sent to this robot:** [`chassis_speed`], quoted from DJI's SDK (`protocol.py`), only
//!   under `--twist speed`. [`system_led`] is the stock app's captured frame and lit the LEDs.
//!
//! Every builder takes the sequence number rather than keeping one, because the robot keeps a
//! sequence *per command* — the lab generator counted per `(cmd_set, cmd_id)` and the captured
//! stock stream does the same — and whose counter that is belongs to the caller. [`SeqCounter`] is
//! that counter for a daemon.

use std::collections::HashMap;

use crate::frame::{Ack, Frame};

/// Who is who on the bus (`index * 32 + host`, `protocol.host2byte`).
pub mod host {
    /// The intelligent controller — the processor this code runs on. Sender of the stock heartbeat.
    pub const BASE: u8 = 0x09;
    /// The motion controller as a whole (`SUBCONTROLLER_ID` in proroklab).
    pub const MOTION_CONTROLLER: u8 = 0x03;
    /// The chassis, host 3 at index 6 — where wheel and speed commands go.
    pub const CHASSIS: u8 = 0xC3;
    /// The sender proroklab uses for chassis and gimbal commands (host 9 at index 6).
    pub const CHASSIS_SENDER: u8 = 0xC9;
    /// The gimbal.
    pub const GIMBAL: u8 = 0x04;
    /// Where the stock app sends the LED command (`0x18`): captured, not named by the SDK. DJI's
    /// on-board library names it: `armor_id = 2400` → `hostid2packid` → `0x18`.
    pub const LED: u8 = 0x18;
    /// The blaster (`gun_id = 2300` → `0x17`).
    pub const GUN: u8 = 0x17;
}

/// `(cmd_set, cmd_id)` of each command this crate builds or reads.
pub mod id {
    pub const CHASSIS_WORKMODE: (u8, u8) = (0x3f, 0x19);
    pub const CHASSIS_WHEEL_SPEED: (u8, u8) = (0x3f, 0x20);
    pub const CHASSIS_SPEED: (u8, u8) = (0x3f, 0x21);
    pub const HEARTBEAT: (u8, u8) = (0x3f, 0x60);
    pub const GIMBAL_ROTATE: (u8, u8) = (0x3f, 0xb0);
    /// The system LEDs as the stock app drives them on this bus: `0x3f/0x32`, captured on
    /// 2026-10-30 (stage 1, `hdvt_uav.strace`, three frames while the app changed colours).
    /// The SDK's `ProtoSetSystemLed` is `0x3f/0x33` with another layout; it never appeared.
    pub const SYSTEM_LED: (u8, u8) = (0x3f, 0x32);
    pub const ARMOR_HIT: (u8, u8) = (0x3f, 0x02);
    pub const IR_HIT: (u8, u8) = (0x3f, 0x10);
    /// The rest of what DJI's on-board library (`rm_module.py`, `duml_cmdset.py`, pulled from the
    /// robot 2026-10-30) sends to the chassis, the gimbal and the blaster over this bus.
    pub const CHASSIS_FOLLOW: (u8, u8) = (0x3f, 0x22);
    pub const CHASSIS_STICK_OVERLAY: (u8, u8) = (0x3f, 0x28);
    pub const CHASSIS_POSITION: (u8, u8) = (0x3f, 0x25);
    pub const CHASSIS_POSITION_PUSH: (u8, u8) = (0x3f, 0x2a);
    pub const CHASSIS_PWM_VALUE: (u8, u8) = (0x3f, 0x3c);
    pub const CHASSIS_PWM_FREQ: (u8, u8) = (0x3f, 0x2b);
    pub const GIMBAL_WORKMODE: (u8, u8) = (0x04, 0x4c);
    pub const GIMBAL_RECENTER: (u8, u8) = (0x3f, 0xb2);
    pub const GIMBAL_ACTION_PUSH: (u8, u8) = (0x3f, 0xb1);
    pub const GIMBAL_SPEED: (u8, u8) = (0x04, 0x0c);
    pub const GIMBAL_SUSPEND_RESUME: (u8, u8) = (0x04, 0x0d);
    pub const GIMBAL_SUBSCRIBE_POSITION: (u8, u8) = (0x04, 0x1e);
    pub const GIMBAL_POSITION_PUSH: (u8, u8) = (0x04, 0x05);
    pub const GUN_LED: (u8, u8) = (0x3f, 0x55);
    pub const GUN_FIRE: (u8, u8) = (0x3f, 0x51);
    pub const ARMOR_SENSITIVITY: (u8, u8) = (0x3f, 0x07);
    pub const DDS_ADD_NODE: (u8, u8) = (0x48, 0x01);
    pub const DDS_RESET_NODE: (u8, u8) = (0x48, 0x02);
    pub const DDS_ADD_SUB: (u8, u8) = (0x48, 0x03);
    pub const DDS_DEL_SUB: (u8, u8) = (0x48, 0x04);
    pub const DDS_PUSH: (u8, u8) = (0x48, 0x08);
}

/// One sequence number per command, as the robot expects.
#[derive(Debug, Default)]
pub struct SeqCounter {
    next: HashMap<(u8, u8), u16>,
}

impl SeqCounter {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next sequence for this command, starting at 0 — which is what the lab generator did and
    /// what `poc_*.bin` encode.
    pub fn next(&mut self, cmd: (u8, u8)) -> u16 {
        let slot = self.next.entry(cmd).or_insert(0);
        let seq = *slot;
        *slot = slot.wrapping_add(1);
        seq
    }
}

/// The heartbeat that armed the chassis on the bench — proroklab's bytes, sender 9, receiver 3.
///
/// **Without this streamed continuously the wheels do not move**: the first PoC attempt sent only
/// workmode and wheel speed and nothing happened. ~100 Hz is what the stock firmware does (91.5 Hz
/// measured over 35 s); 20 Hz was enough on the bench.
pub fn heartbeat_proroklab(seq: u16) -> Frame {
    Frame::request(
        host::BASE,
        host::MOTION_CONTROLLER,
        seq,
        id::HEARTBEAT.0,
        id::HEARTBEAT.1,
        vec![
            0x00, 0x04, 0x20, 0x00, 0x01, 0x00, 0x40, 0x00, 0x02, 0x10, 0x04, 0x03, 0x00, 0x04,
        ],
    )
}

/// The heartbeat the stock firmware actually sends — receiver `0xC3` and two bytes that differ from
/// proroklab's (`0x08` at offset 5, `0x00` at offset 11). Captured on the UART; not yet sent by us.
pub fn heartbeat_stock(seq: u16) -> Frame {
    Frame::request(
        host::BASE,
        host::CHASSIS,
        seq,
        id::HEARTBEAT.0,
        id::HEARTBEAT.1,
        vec![
            0x00, 0x04, 0x20, 0x00, 0x01, 0x08, 0x40, 0x00, 0x02, 0x10, 0x04, 0x00, 0x00, 0x04,
        ],
    )
}

/// `0x3f/0x19`: chassis work mode. The PoC sent mode 1 five times before driving.
pub fn chassis_workmode(seq: u16, mode: u8) -> Frame {
    Frame::request(
        host::BASE,
        host::CHASSIS,
        seq,
        id::CHASSIS_WORKMODE.0,
        id::CHASSIS_WORKMODE.1,
        vec![mode],
    )
}

/// `0x3f/0x20`: wheel speeds, front-right, front-left, back-left, back-right.
///
/// The left wheels are negated on the wire — proroklab does it, the lab encoder did it, and the robot
/// drove straight. Units: DJI's SDK documents `drive_wheels` as rpm in `[-1000, 1000]`; the bench has
/// yet to confirm that the serial path uses the same scale (the PoC used 30–40 and the wheels turned).
pub fn chassis_wheel_speed(seq: u16, fr: i16, fl: i16, bl: i16, br: i16) -> Frame {
    let mut data = Vec::with_capacity(8);
    for w in [fr, fl.wrapping_neg(), bl.wrapping_neg(), br] {
        data.extend_from_slice(&w.to_le_bytes());
    }
    Frame::request(
        host::CHASSIS_SENDER,
        host::CHASSIS,
        seq,
        id::CHASSIS_WHEEL_SPEED.0,
        id::CHASSIS_WHEEL_SPEED.1,
        data,
    )
}

/// `0x3f/0x21`: a twist for the chassis — `vx`, `vy` in m/s (`[-3.5, 3.5]`), `vyaw` in °/s
/// (`[-600, 600]`), three little-endian floats (`ProtoChassisSpeedMode`, `drive_speed` in the SDK).
///
/// **Not yet sent to this robot over the serial.** proroklab sends it over CAN; the bench decides
/// whether the motion controller takes it on the UART, which would spare `s1d` the mecanum
/// kinematics. Note the SDK's `vyaw` is in degrees per second, not radians: the caller converts.
pub fn chassis_speed(seq: u16, vx: f32, vy: f32, vyaw_deg_s: f32) -> Frame {
    let mut data = Vec::with_capacity(12);
    for v in [vx, vy, vyaw_deg_s] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    Frame::request(
        host::CHASSIS_SENDER,
        host::CHASSIS,
        seq,
        id::CHASSIS_SPEED.0,
        id::CHASSIS_SPEED.1,
        data,
    )
}

/// `0x3f/0xb0`: gimbal to an absolute yaw and pitch, in tenths of a degree, at 30 units of speed.
///
/// The bytes are `ProtoGimbalRotate`'s: action id 0, push frequency 2, yaw and pitch valid in the
/// coordinate frame 3, roll zero. This moved the gimbal on the bench (`poc_gim.bin`, yaw +15°, pitch
/// +10°). Manual limits: yaw ±250°, pitch −20°…+35° — the caller clamps, this does not.
pub fn gimbal_angles(seq: u16, yaw_ddeg: i16, pitch_ddeg: i16) -> Frame {
    gimbal_angles_axes(seq, yaw_ddeg, pitch_ddeg, GIMBAL_AXES_BOTH)
}

/// Axis bits of `0x3f/0xb0`'s ctrl byte (`rm_define.gimbal_axis_*_maskbit`): yaw `0x01`, pitch `0x04`.
pub const GIMBAL_AXES_BOTH: u8 = 0x05;
pub const GIMBAL_AXIS_PITCH: u8 = 0x04;

/// [`gimbal_angles`] for the given axes only. With `GIMBAL_AXES_BOTH` it is the PoC's frame byte
/// for byte; with `GIMBAL_AXIS_PITCH` the yaw is left to the controller — what DJI's own library
/// does while the gimbal follows the chassis (`rm_ctrl.py` `angle_ctrl`: "gimbal_yaw_follow_mode,
/// the yaw ctrl invalid", the yaw bit cleared).
pub fn gimbal_angles_axes(seq: u16, yaw_ddeg: i16, pitch_ddeg: i16, axes: u8) -> Frame {
    const ACTION_ID: u8 = 0x00;
    const CTRL_AND_FREQ: u8 = 0x08; // action_ctrl 0 | push_freq 2 << 2
    const COORDINATE: u8 = 3; // yaw relative to the chassis, pitch NED (`rm_define.gimbal_coodrdinate_3`)
    let valid_and_frame: u8 = (COORDINATE << 3) | (axes & 0x07);
    const SPEED: u16 = 30;
    let mut data = vec![ACTION_ID, CTRL_AND_FREQ, valid_and_frame];
    for v in [yaw_ddeg, 0, pitch_ddeg] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    for v in [0u16, SPEED, 0, SPEED] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    Frame::request(
        host::CHASSIS_SENDER,
        host::GIMBAL,
        seq,
        id::GIMBAL_ROTATE.0,
        id::GIMBAL_ROTATE.1,
        data,
    )
}

/// One LED effect, in the thirteen bytes the stock app writes (captured on the robot's UART,
/// 2026-10-30, stage 1 of `docs/s1/project/s1-bringup.md`; `tests/fixtures/led_stock_*_real.bin`):
///
/// ```text
/// effect  led_mask(u16 LE)  r g b     repeat  t1(u16 LE)  t2(u16 LE)  comp_mask(u16 LE)
/// 01      ff 00             00 7f 46  00      00 00       00 00       3f 00   solid teal, every group
/// 02      ff 00             ff ff ff  00      f4 01       f4 01       3f 00   white breath, 500/500 ms
/// ```
///
/// The field names follow the SDK's `led.py` (`set_led`: effect `0` off, `1` on, `2` breath,
/// `3` flash, `4` scrolling; `comp_mask` `0x01` bottom back … `0x20` top right, `0x3f` all), the
/// order and widths follow the wire. Which bit is which group is still the bench's (stage 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemLed {
    pub effect: u8,
    /// Which LEDs within a group; the app sends `0xff`.
    pub led_mask: u16,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// The SDK's `loop`; the app sends `0`.
    pub repeat: u8,
    /// Effect timings, ms; the app sends 500/500 for breath and 0/0 for solid.
    pub t1: u16,
    pub t2: u16,
    pub comp_mask: u16,
}

impl SystemLed {
    /// Every LED a solid colour: what `robot.flashlight` becomes on this body.
    pub fn solid(r: u8, g: u8, b: u8) -> Self {
        Self {
            effect: 1,
            led_mask: 0xff,
            r,
            g,
            b,
            repeat: 0,
            t1: 0,
            t2: 0,
            comp_mask: 0x3f,
        }
    }

    /// Every LED breathing a colour, as the app's own breath frame has it.
    pub fn breath(r: u8, g: u8, b: u8, t1: u16, t2: u16) -> Self {
        Self {
            effect: 2,
            t1,
            t2,
            ..Self::solid(r, g, b)
        }
    }

    pub fn off() -> Self {
        Self {
            effect: 0,
            ..Self::solid(0, 0, 0)
        }
    }
}

/// `0x3f/0x32` to host `0x18`: the system LEDs, byte for byte what the app writes.
pub fn system_led(seq: u16, led: &SystemLed) -> Frame {
    let mut data = Vec::with_capacity(13);
    data.push(led.effect);
    data.extend_from_slice(&led.led_mask.to_le_bytes());
    data.extend_from_slice(&[led.r, led.g, led.b, led.repeat]);
    data.extend_from_slice(&led.t1.to_le_bytes());
    data.extend_from_slice(&led.t2.to_le_bytes());
    data.extend_from_slice(&led.comp_mask.to_le_bytes());
    Frame::request(
        host::BASE,
        host::LED,
        seq,
        id::SYSTEM_LED.0,
        id::SYSTEM_LED.1,
        data,
    )
}

// ---------------------------------------------------------------------------------------------
// What DJI's own on-board library sends. Every builder below reproduces `rm_module.py` byte for
// byte (`tests/dji_oracle.rs`: the library's packer, run on the Mac with the same arguments, is
// the expected value), with its sender (`scratch_script_id` → `0xC9`), its receiver and its ack
// request (`NEED_ACK_TYPE` 0x40 unless the library says `NO_ACK_TYPE`).
// ---------------------------------------------------------------------------------------------

/// Task header the library puts in front of every "task" command: task id, then
/// `TASK_FREQ_10Hz << 2 | TASK_CTRL_START`.
const TASK_START_10HZ: u8 = 0x08;
const TASK_STOP: u8 = 0x01;

/// `0x3f/0x22`, no ack: the twist in **follow mode** — `vx`, `vy` in m/s and the yaw *offset*
/// between chassis and gimbal in degrees. In this mode the motion controller turns the chassis to
/// keep the gimbal's heading plus `angle_deg` (`Chassis.set_follow_speed`; the library's
/// `chassis_sdk_follow_mode` is "gimbal lead", the app's "chassis follows the gimbal").
pub fn chassis_follow(seq: u16, vx: f32, vy: f32, angle_deg: f32) -> Frame {
    let mut data = Vec::with_capacity(12);
    for v in [vx, vy, angle_deg] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    Frame::request(
        host::CHASSIS_SENDER,
        host::CHASSIS,
        seq,
        id::CHASSIS_FOLLOW.0,
        id::CHASSIS_FOLLOW.1,
        data,
    )
}

/// `0x3f/0x28`: the frame a velocity is read in. `0` off, `1` body axes, `2` the gimbal's heading
/// (`stick_overlay_and_axes_enable`).
pub fn chassis_stick_overlay(seq: u16, mode: u8) -> Frame {
    Frame::request(
        host::CHASSIS_SENDER,
        host::CHASSIS,
        seq,
        id::CHASSIS_STICK_OVERLAY.0,
        id::CHASSIS_STICK_OVERLAY.1,
        vec![mode],
    )
    .with_ack(Ack::WhenDone)
}

/// `0x3f/0x25`: move a distance and stop, the loop closed by the motion controller. `x`, `y` in
/// cm (±500), `yaw` in tenths of a degree (±18000), `vel_xy` in cm/s (10..250), `omega` in tenths
/// of a degree per second (100..5400). `axis_mode` 0 = from where the chassis is, 1 = from the
/// power-on origin. Progress comes back as `0x3f/0x2a` pushes with this `task_id`.
#[allow(clippy::too_many_arguments)]
pub fn chassis_position(
    seq: u16,
    task_id: u8,
    axis_mode: u8,
    x_cm: i16,
    y_cm: i16,
    yaw_ddeg: i16,
    vel_xy_cm_s: u8,
    omega_ddeg_s: u16,
) -> Frame {
    let mut data = vec![task_id, TASK_START_10HZ, 0, axis_mode];
    for v in [x_cm, y_cm, yaw_ddeg] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    data.push(vel_xy_cm_s);
    data.extend_from_slice(&omega_ddeg_s.to_le_bytes());
    Frame::request(
        host::CHASSIS_SENDER,
        host::CHASSIS,
        seq,
        id::CHASSIS_POSITION.0,
        id::CHASSIS_POSITION.1,
        data,
    )
    .with_ack(Ack::WhenDone)
}

/// `0x3f/0x25` with `TASK_CTRL_STOP`: abandon the position task `task_id`.
pub fn chassis_position_stop(seq: u16, task_id: u8) -> Frame {
    let mut data = vec![task_id, TASK_STOP, 0, 0];
    data.extend_from_slice(&[0u8; 6]);
    Frame::request(
        host::CHASSIS_SENDER,
        host::CHASSIS,
        seq,
        id::CHASSIS_POSITION.0,
        id::CHASSIS_POSITION.1,
        data,
    )
    .with_ack(Ack::WhenDone)
}

fn chassis_pwm(seq: u16, cmd: (u8, u8), mask: u8, values: [u16; 6]) -> Frame {
    let mut data = vec![mask];
    for v in values {
        data.extend_from_slice(&v.to_le_bytes());
    }
    Frame::request(host::CHASSIS_SENDER, host::CHASSIS, seq, cmd.0, cmd.1, data)
        .with_ack(Ack::WhenDone)
}

/// `0x3f/0x3c`: duty of the six PWM ports, 0..1000 per mille, for the ports in `mask`.
pub fn chassis_pwm_value(seq: u16, mask: u8, per_mille: [u16; 6]) -> Frame {
    chassis_pwm(seq, id::CHASSIS_PWM_VALUE, mask, per_mille)
}

/// `0x3f/0x2b`: frequency of the six PWM ports, Hz (0..50000), for the ports in `mask`.
pub fn chassis_pwm_freq(seq: u16, mask: u8, hz: [u16; 6]) -> Frame {
    chassis_pwm(seq, id::CHASSIS_PWM_FREQ, mask, hz)
}

/// `0x04/0x4c`: gimbal work mode — `0` free, `1` FPV, `2` yaw follows the chassis
/// (`gimbal_yaw_follow_mode`, the app's "gimbal follows the chassis").
pub fn gimbal_workmode(seq: u16, mode: u8) -> Frame {
    Frame::request(
        host::CHASSIS_SENDER,
        host::GIMBAL,
        seq,
        id::GIMBAL_WORKMODE.0,
        id::GIMBAL_WORKMODE.1,
        vec![mode, 0],
    )
    .with_ack(Ack::WhenDone)
}

/// `0x3f/0xb2`: recentre yaw and pitch at the given speeds (°/s, 5..540), as a task.
pub fn gimbal_recenter(seq: u16, task_id: u8, yaw_speed: u16, pitch_speed: u16) -> Frame {
    const YAW_AND_PITCH: u8 = 0x05;
    let mut data = vec![task_id, TASK_START_10HZ, YAW_AND_PITCH];
    for v in [yaw_speed, 0, pitch_speed] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    Frame::request(
        host::CHASSIS_SENDER,
        host::GIMBAL,
        seq,
        id::GIMBAL_RECENTER.0,
        id::GIMBAL_RECENTER.1,
        data,
    )
    .with_ack(Ack::WhenDone)
}

/// `0x3f/0xb0` as the library sends it, a task: `coordinate` (0 NED, 1 relative to where the
/// gimbal is, 2 the chassis, 3 pitch NED, 4 yaw chassis + pitch NED, 5 yaw chassis + pitch offset),
/// which axes (`0x01` yaw, `0x04` pitch, `0x05` both), targets in tenths of a degree, speeds in
/// °/s (0..540). [`gimbal_angles`] is the same command with the PoC's fixed header.
#[allow(clippy::too_many_arguments)]
pub fn gimbal_degree(
    seq: u16,
    task_id: u8,
    coordinate: u8,
    axes: u8,
    yaw_ddeg: i16,
    pitch_ddeg: i16,
    yaw_speed: u16,
    pitch_speed: u16,
) -> Frame {
    let ctrl = ((coordinate & 0x07) << 3) | axes;
    let mut data = vec![task_id, TASK_START_10HZ, ctrl];
    for v in [yaw_ddeg, 0, pitch_ddeg, 0] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    for v in [yaw_speed, 0, pitch_speed] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    Frame::request(
        host::CHASSIS_SENDER,
        host::GIMBAL,
        seq,
        id::GIMBAL_ROTATE.0,
        id::GIMBAL_ROTATE.1,
        data,
    )
    .with_ack(Ack::WhenDone)
}

/// `0x04/0x0c`, no ack: gimbal by rate, yaw and pitch in tenths of a degree per second (±5400),
/// `ctrl` the library's `accel_contral_flag` (`0xdc`; bit 6 cleared when the sticks overlay).
pub fn gimbal_speed(seq: u16, yaw_ddeg_s: i16, pitch_ddeg_s: i16, ctrl: u8) -> Frame {
    let mut data = Vec::with_capacity(7);
    for v in [yaw_ddeg_s, 0, pitch_ddeg_s] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    data.push(ctrl);
    Frame::request(
        host::CHASSIS_SENDER,
        host::GIMBAL,
        seq,
        id::GIMBAL_SPEED.0,
        id::GIMBAL_SPEED.1,
        data,
    )
}

/// `0x04/0x0d`, ack now: the gimbal goes limp and folds (`0x2ab5`) or wakes (`0x7ef2`).
pub fn gimbal_suspend_resume(seq: u16, suspend: bool) -> Frame {
    let code: u16 = if suspend { 0x2ab5 } else { 0x7ef2 };
    Frame::request(
        host::CHASSIS_SENDER,
        host::GIMBAL,
        seq,
        id::GIMBAL_SUSPEND_RESUME.0,
        id::GIMBAL_SUSPEND_RESUME.1,
        code.to_le_bytes().to_vec(),
    )
    .with_ack(Ack::Now)
}

/// `0x04/0x1e`: ask the gimbal to push its position (`0x04/0x05`) — `0xff` on, `0` off.
pub fn gimbal_subscribe_position(seq: u16, on: bool) -> Frame {
    Frame::request(
        host::CHASSIS_SENDER,
        host::GIMBAL,
        seq,
        id::GIMBAL_SUBSCRIBE_POSITION.0,
        id::GIMBAL_SUBSCRIBE_POSITION.1,
        vec![id::GIMBAL_POSITION_PUSH.1, if on { 0xff } else { 0 }],
    )
    .with_ack(Ack::WhenDone)
}

/// `0x3f/0x55` to the blaster: its LED. `ctrl` in the high nibble, `mode` (effect) in the low
/// one, colour, repeat count and the two effect times in ms.
#[allow(clippy::too_many_arguments)]
pub fn gun_led(
    seq: u16,
    ctrl: u8,
    mode: u8,
    r: u8,
    g: u8,
    b: u8,
    times: u8,
    t1: u16,
    t2: u16,
) -> Frame {
    let mut data = vec![((ctrl << 4) & 0xf0) | (mode & 0x0f), r, g, b, times];
    data.extend_from_slice(&t1.to_le_bytes());
    data.extend_from_slice(&t2.to_le_bytes());
    Frame::request(
        host::CHASSIS_SENDER,
        host::GUN,
        seq,
        id::GUN_LED.0,
        id::GUN_LED.1,
        data,
    )
    .with_ack(Ack::WhenDone)
}

/// The launcher's muzzle flash, as the stock controller sends it once per shot: `0x3f/0x55` with
/// control 7, effect 3 (flash), green, once, 40 ms on, 0 off. Captured on the UART with the stock
/// alive and the app firing (`capture-20261030-165821/hdvt_fire.strace`, 2026-10-30 15:45:56.117, 30 of them);
/// `tests/fixtures/gun_led_fire_stock_real.bin` is one, and `real_frames.rs` holds this builder
/// to its payload. Nothing comes back from the launcher on the UART (its only frame in that
/// capture is its version string), so the stock paces the flashes itself, and so does `s1d`.
pub fn gun_fire_flash(seq: u16) -> Frame {
    gun_led(seq, 7, 3, 0, 255, 0, 1, 40, 0)
}

/// `0x3f/0x07`: the armour's hit detector thresholds, scaled by `k` as `Armor.set_hit_sensitivity`
/// scales them (`k = 1.0` is the library's default; the peak thresholds are `160 k`, `180 k`,
/// `200 k`). The library addresses it to the stock controller (`0x09`), stopped under `s1d`, and
/// `s1.armor` sends it to `0x18`, without any observed effect — hence `receiver` is the caller's.
pub fn armor_sensitivity(seq: u16, receiver: u8, k: f32) -> Frame {
    let scaled = |v: f32| ((v * k) as u16).to_le_bytes();
    let mut data = vec![0x3f];
    for v in [500u16, 300, 50, 13, 6, 1] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    for v in [160.0, 180.0, 200.0] {
        data.extend_from_slice(&scaled(v));
    }
    Frame::request(
        host::CHASSIS_SENDER,
        receiver,
        seq,
        id::ARMOR_SENSITIVITY.0,
        id::ARMOR_SENSITIVITY.1,
        data,
    )
    .with_ack(Ack::WhenDone)
}

/// `0x3f/0x51`: fire the blaster `times` (1..8) shots, `ir` for the infrared beam, else gel beads
/// (`Gun.set_cmd_fire`: `mode << 4 | count`, mode 1 single… the library addresses it to the stock
/// controller, `receiver` here; the bench finds whether the blaster host takes it directly).
pub fn gun_fire(seq: u16, receiver: u8, ir: bool, times: u8) -> Frame {
    let kind: u8 = if ir { 1 } else { 0 };
    Frame::request(
        host::CHASSIS_SENDER,
        receiver,
        seq,
        id::GUN_FIRE.0,
        id::GUN_FIRE.1,
        vec![((kind << 4) & 0xf0) | (times & 0x0f)],
    )
    .with_ack(Ack::WhenDone)
}
