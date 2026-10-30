//! The commands the S1's body obeys, as frames.
//!
//! Three provenances, kept apart on purpose:
//! - **Moved the real robot** (2026-10-29): [`chassis_workmode`], [`chassis_wheel_speed`],
//!   [`gimbal_angles`] and [`heartbeat_proroklab`] — byte for byte the lab's `s1_duml.py`, which came
//!   from proroklab's `chassis.cpp`/`gimbal.cpp`. `tests/fixtures/poc_*.bin` are those exact bytes.
//! - **Seen on the wire** from the stock firmware: [`heartbeat_stock`] (`tests/fixtures/heartbeat_real.bin`).
//! - **Quoted from DJI's SDK** (`protocol.py`) and not yet sent to this robot: [`chassis_speed`]
//!   and [`system_led`]. The bench confirms them; until then their docs say so.
//!
//! Every builder takes the sequence number rather than keeping one, because the robot keeps a
//! sequence *per command* — the lab generator counted per `(cmd_set, cmd_id)` and the captured
//! stock stream does the same — and whose counter that is belongs to the caller. [`SeqCounter`] is
//! that counter for a daemon.

use std::collections::HashMap;

use crate::frame::Frame;

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
    /// Where the stock app sends the LED command (`0x18`): captured, not named by the SDK.
    pub const LED: u8 = 0x18;
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
    const ACTION_ID: u8 = 0x00;
    const CTRL_AND_FREQ: u8 = 0x08; // action_ctrl 0 | push_freq 2 << 2
    const VALID_AND_FRAME: u8 = 0x1d; // yaw_valid | pitch_valid << 2 | coordinate 3 << 3
    const SPEED: u16 = 30;
    let mut data = vec![ACTION_ID, CTRL_AND_FREQ, VALID_AND_FRAME];
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
/// 2026-10-30, stage 1 of `docs/project/s1-bringup.md`; `tests/fixtures/led_stock_*_real.bin`):
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
