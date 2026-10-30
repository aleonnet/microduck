//! Telemetry the motion controller pushes once asked: DJI's "DDS" subscriptions.
//!
//! The stock firmware subscribes at boot and the controller then receives `0x48/0x08` pushes every
//! few milliseconds — 109 frames of 176 bytes in the idle capture, one of them in
//! `tests/fixtures/dds_push_176_real.bin`. The subscription request (`0x48/0x03`), its subject UIDs
//! and each subject's layout are quoted from DJI's SDK (`protocol.py` `ProtoAddSubMsg`, `dds.py`
//! `SUB_UID_MAP`, the `*Subject.decode` formats in `chassis.py` and `battery.py`); proroklab sends
//! the same request over CAN (`dds.cpp`). **What the bench still has to show:** that the motion
//! controller honours a subscription arriving on the UART, and the order the subjects come back in
//! — which is the order they were asked for, by both sources.

use crate::cmd::{host, id};
use crate::frame::Frame;

/// Subject ids, from `dds.py`.
pub mod uid {
    pub const BATTERY: u64 = 0x0002_0009_6862_229f;
    pub const GIMBAL_BASE: u64 = 0x0002_0009_f588_2874;
    pub const VELOCITY: u64 = 0x0002_0009_49a4_009c;
    pub const ESC: u64 = 0x0002_0009_c14c_b7c5;
    pub const ATTITUDE: u64 = 0x0002_0009_6b98_6306;
    pub const IMU: u64 = 0x0002_0009_a798_5b8d;
    pub const POSITION: u64 = 0x0002_0009_eeb7_cece;
    pub const SA_STATUS: u64 = 0x0002_0009_4a2c_6d55;
    pub const CHASSIS_MODE: u64 = 0x0002_0009_4fcb_1146;
    pub const GIMBAL_POS: u64 = 0x0002_0009_f79b_3c97;
}

/// Rates the SDK accepts for a subscription, Hz.
pub const FREQUENCIES: [u16; 5] = [1, 5, 10, 20, 50];

/// `0x48/0x03`: subscribe `uids` at `freq` Hz under `msg_id`.
///
/// Byte layout per `ProtoAddSubMsg.pack_req`: node id, msg id, flags (`timestamp & 1 |
/// stop_when_disconnect & 2`), sub mode, count, then each UID as 8 bytes little-endian, then the
/// frequency as 2 bytes little-endian. `timestamp = false` keeps the pushes as the SDK expects
/// them — `sub_mode, msg_id, subjects…` — with no time fields in front.
pub fn add_sub(
    seq: u16,
    node_id: u8,
    msg_id: u8,
    uids: &[u64],
    freq: u16,
    timestamp: bool,
) -> Frame {
    let mut data = Vec::with_capacity(7 + 8 * uids.len());
    data.push(node_id);
    data.push(msg_id);
    data.push(u8::from(timestamp));
    data.push(0); // sub_mode
    data.push(uids.len() as u8);
    for uid in uids {
        data.extend_from_slice(&uid.to_le_bytes());
    }
    data.extend_from_slice(&freq.to_le_bytes());
    Frame::request(
        host::BASE,
        host::MOTION_CONTROLLER,
        seq,
        id::DDS_ADD_SUB.0,
        id::DDS_ADD_SUB.1,
        data,
    )
}

/// `0x48/0x04`: drop the subscription `msg_id` (`ProtoDelMsg`: sub mode, node id, msg id).
pub fn del_sub(seq: u16, node_id: u8, msg_id: u8) -> Frame {
    Frame::request(
        host::BASE,
        host::MOTION_CONTROLLER,
        seq,
        id::DDS_DEL_SUB.0,
        id::DDS_DEL_SUB.1,
        vec![0, node_id, msg_id],
    )
}

/// One `0x48/0x08` push, split as `ProtoPushPeriodMsg.unpack_req` splits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushPeriod<'a> {
    pub sub_mode: u8,
    pub msg_id: u8,
    /// The subjects, concatenated in subscription order.
    pub data: &'a [u8],
}

impl<'a> PushPeriod<'a> {
    pub fn parse(frame: &'a Frame) -> Option<Self> {
        if (frame.cmd_set, frame.cmd_id) != id::DDS_PUSH || frame.data.len() < 2 {
            return None;
        }
        Some(Self {
            sub_mode: frame.data[0],
            msg_id: frame.data[1],
            data: &frame.data[2..],
        })
    }
}

fn f32_at(b: &[u8], i: usize) -> f32 {
    f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// `BatterySubject`: `<HhiBB`, ten bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub adc_value: u16,
    pub temperature: i16,
    pub current: i32,
    pub percent: u8,
    pub recv: u8,
}

impl Battery {
    pub const SIZE: usize = 10;

    pub fn parse(b: &[u8]) -> Option<(Self, &[u8])> {
        let (me, rest) = b.split_at_checked(Self::SIZE)?;
        Some((
            Self {
                adc_value: u16::from_le_bytes([me[0], me[1]]),
                temperature: i16::from_le_bytes([me[2], me[3]]),
                current: i32::from_le_bytes([me[4], me[5], me[6], me[7]]),
                percent: me[8],
                recv: me[9],
            },
            rest,
        ))
    }
}

/// `AttiInfoSubject`: yaw, pitch, roll as `<fff`. The SDK states no unit; degrees by its own
/// examples, confirmed on the bench by tilting the robot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Attitude {
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
}

impl Attitude {
    pub const SIZE: usize = 12;

    pub fn parse(b: &[u8]) -> Option<(Self, &[u8])> {
        let (me, rest) = b.split_at_checked(Self::SIZE)?;
        Some((
            Self {
                yaw: f32_at(me, 0),
                pitch: f32_at(me, 4),
                roll: f32_at(me, 8),
            },
            rest,
        ))
    }
}

/// `PositionSubject`: x, y, z as `<fff`, metres, in the frame the chassis had at power-on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Position {
    pub const SIZE: usize = 12;

    pub fn parse(b: &[u8]) -> Option<(Self, &[u8])> {
        let (me, rest) = b.split_at_checked(Self::SIZE)?;
        Some((
            Self {
                x: f32_at(me, 0),
                y: f32_at(me, 4),
                z: f32_at(me, 8),
            },
            rest,
        ))
    }
}

/// `VelocitySubject`: world-frame then body-frame velocities, `<ffffff`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Velocity {
    pub world: [f32; 3],
    pub body: [f32; 3],
}

impl Velocity {
    pub const SIZE: usize = 24;

    pub fn parse(b: &[u8]) -> Option<(Self, &[u8])> {
        let (me, rest) = b.split_at_checked(Self::SIZE)?;
        Some((
            Self {
                world: [f32_at(me, 0), f32_at(me, 4), f32_at(me, 8)],
                body: [f32_at(me, 12), f32_at(me, 16), f32_at(me, 20)],
            },
            rest,
        ))
    }
}

/// `ImuSubject`: accelerometer then gyroscope, `<ffffff`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Imu {
    pub acc: [f32; 3],
    pub gyro: [f32; 3],
}

impl Imu {
    pub const SIZE: usize = 24;

    pub fn parse(b: &[u8]) -> Option<(Self, &[u8])> {
        let (me, rest) = b.split_at_checked(Self::SIZE)?;
        Some((
            Self {
                acc: [f32_at(me, 0), f32_at(me, 4), f32_at(me, 8)],
                gyro: [f32_at(me, 12), f32_at(me, 16), f32_at(me, 20)],
            },
            rest,
        ))
    }
}

/// `EscSubject`: per wheel, speed in rpm (`-8192..8191`), angle (`0..32767` for a turn), a
/// timestamp and a state — `<hhhhhhhhIIIIBBBB`, 36 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Esc {
    pub speed_rpm: [i16; 4],
    pub angle: [i16; 4],
    pub timestamp: [u32; 4],
    pub state: [u8; 4],
}

impl Esc {
    pub const SIZE: usize = 36;

    pub fn parse(b: &[u8]) -> Option<(Self, &[u8])> {
        let (me, rest) = b.split_at_checked(Self::SIZE)?;
        let h = |i: usize| i16::from_le_bytes([me[i], me[i + 1]]);
        let w = |i: usize| u32::from_le_bytes([me[i], me[i + 1], me[i + 2], me[i + 3]]);
        Some((
            Self {
                speed_rpm: [h(0), h(2), h(4), h(6)],
                angle: [h(8), h(10), h(12), h(14)],
                timestamp: [w(16), w(20), w(24), w(28)],
                state: [me[32], me[33], me[34], me[35]],
            },
            rest,
        ))
    }
}

/// `SaStatusSubject`: the chassis's own judgement of its situation, as bits in two bytes.
///
/// Bit order as the SDK unpacks it (`static_flag, up_hill, down_hill, on_slope, is_pick_up,
/// slip_flag, impact_x, impact_y` in the first byte; `roll_over, hill_static` in the second) —
/// **to be confirmed on the bench by lifting the robot and watching `is_pick_up`.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaStatus {
    pub static_flag: bool,
    pub up_hill: bool,
    pub down_hill: bool,
    pub on_slope: bool,
    pub is_pick_up: bool,
    pub slip: bool,
    pub impact_x: bool,
    pub impact_y: bool,
    pub roll_over: bool,
    pub hill_static: bool,
}

impl SaStatus {
    pub const SIZE: usize = 2;

    pub fn parse(b: &[u8]) -> Option<(Self, &[u8])> {
        let (me, rest) = b.split_at_checked(Self::SIZE)?;
        let bit = |byte: u8, n: u8| (byte >> n) & 1 == 1;
        Some((
            Self {
                static_flag: bit(me[0], 0),
                up_hill: bit(me[0], 1),
                down_hill: bit(me[0], 2),
                on_slope: bit(me[0], 3),
                is_pick_up: bit(me[0], 4),
                slip: bit(me[0], 5),
                impact_x: bit(me[0], 6),
                impact_y: bit(me[0], 7),
                roll_over: bit(me[1], 0),
                hill_static: bit(me[1], 1),
            },
            rest,
        ))
    }
}
