//! Telemetry the motion controller pushes once asked: DJI's "DDS" subscriptions.
//!
//! The stock firmware subscribes at boot and the controller then receives `0x48/0x08` pushes every
//! few milliseconds — 109 frames of 176 bytes in the idle capture, one of them in
//! `tests/fixtures/dds_push_176_real.bin`. The subscription request (`0x48/0x03`), its subject UIDs
//! and each subject's layout are quoted from DJI's SDK (`protocol.py` `ProtoAddSubMsg`, `dds.py`
//! `SUB_UID_MAP`, the `*Subject.decode` formats in `chassis.py` and `battery.py`); proroklab sends
//! the same request over CAN (`dds.cpp`). **What the robot showed (2026-10-30):** the motion
//! controller refuses our subscription on the UART, cause undetermined; the telemetry `s1d` reads
//! is the stock's own boot-time subscription, decoded by [`StockPush`].

use crate::cmd::{host, id};
use crate::frame::{Ack, Frame};

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
    /// Whether the app's sticks are live (DJI's `stick_flag`, one byte).
    pub const STICK: u64 = 0x0002_0009_55e9_a0fa;
}

/// Rates the SDK accepts for a subscription, Hz.
pub const FREQUENCIES: [u16; 5] = [1, 5, 10, 20, 50];

/// `0x48/0x01`: register `node_id` with the controller's DDS before subscribing — what the stock
/// sends first when it starts (`ProtoSubscribeAddNode`: node id, then `sub_vision = 0x03000000`
/// little-endian; a reply of `0x50` means "already there" and the SDK accepts it).
pub fn add_node(seq: u16, node_id: u8) -> Frame {
    let mut data = vec![node_id];
    data.extend_from_slice(&0x0300_0000u32.to_le_bytes());
    Frame::request(
        host::BASE,
        host::MOTION_CONTROLLER,
        seq,
        id::DDS_ADD_NODE.0,
        id::DDS_ADD_NODE.1,
        data,
    )
    .with_ack(Ack::WhenDone)
}

/// `0x48/0x02`: forget everything `node_id` subscribed (`ProtoSubNodeReset`, one byte).
pub fn reset_node(seq: u16, node_id: u8) -> Frame {
    Frame::request(
        host::BASE,
        host::MOTION_CONTROLLER,
        seq,
        id::DDS_RESET_NODE.0,
        id::DDS_RESET_NODE.1,
        vec![node_id],
    )
    .with_ack(Ack::WhenDone)
}

/// `0x48/0x03`: subscribe `uids` at `freq` Hz under `msg_id`.
///
/// Byte layout per `ProtoAddSubMsg.pack_req`: node id, msg id, flags (`timestamp & 1 |
/// stop_when_disconnect & 2`), sub mode, count, then each UID as 8 bytes little-endian, then the
/// frequency as 2 bytes little-endian. With `timestamp` the pushes carry `time_ms, time_ns` (two
/// `u32`, [`PUSH_TIMESTAMP_LEN`] bytes) in front of the subjects — proroklab's `metadata`, and
/// proroklab asks for both flags (`0x3`) on the motion controller's own bus.
pub fn add_sub(
    seq: u16,
    node_id: u8,
    msg_id: u8,
    uids: &[u64],
    freq: u16,
    timestamp: bool,
    stop_when_disconnect: bool,
) -> Frame {
    let mut data = Vec::with_capacity(7 + 8 * uids.len());
    data.push(node_id);
    data.push(msg_id);
    data.push(u8::from(timestamp) | (u8::from(stop_when_disconnect) << 1));
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
    .with_ack(Ack::WhenDone)
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
    .with_ack(Ack::WhenDone)
}

/// The acknowledgement the stock firmware writes back for **every** `0x48/0x08` push it receives
/// (1849 of them in 70 s of `hdvt_uav.strace`, 2026-10-30; `tests/fixtures/dds_push_ack_stock_real.bin`
/// is one): an ack frame with the push's own sequence number, attribute `0xa0` (`is_ack`, "ack
/// now") and a single data byte `0x01`. Sent for ours and the stock's alike, because the controller
/// has never been seen pushing to a subscriber that does not answer.
pub fn push_ack(seq: u16) -> Frame {
    let mut f = Frame::request(
        host::BASE,
        host::MOTION_CONTROLLER,
        seq,
        id::DDS_PUSH.0,
        id::DDS_PUSH.1,
        vec![0x01],
    )
    .with_ack(Ack::Now);
    f.is_ack = true;
    f
}

/// Bytes of time stamp in front of the subjects when the subscription asked for one.
pub const PUSH_TIMESTAMP_LEN: usize = 8;

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
    /// A reading the S1's pack can give: three cells, 10.8 V nominal and 4.2 V a cell when full
    /// (user manual, "Intelligent Battery": 2400 mAh, 10.8 V; "cell voltage is at 4.2 V" when
    /// charged), so 9.0 to 13.0 V with margin, and a percentage. Captures with the stock alive
    /// read 11.75 to 11.94 V; the boot of 2026-10-30 22:00, with the stock controller process
    /// relaunched by hand, read 49 mV at 0 % in the same ten bytes — the two subjects come in
    /// the other order on such a boot ([`BlockOrder`]; `s1-bringup.md`, "Item 1 closed").
    pub const PLAUSIBLE_MV: std::ops::RangeInclusive<u16> = 9_000..=13_000;

    /// Whether these ten bytes read as the S1's pack at all — see [`Self::PLAUSIBLE_MV`].
    pub fn plausible(&self) -> bool {
        Self::PLAUSIBLE_MV.contains(&self.adc_value) && self.percent <= 100
    }

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
    /// The wheel motor's range in DJI's own SDK (`chassis.drive_wheels`: `[-1000, 1000]` rpm).
    pub const MAX_RPM: i16 = 1000;

    /// Whether these 36 bytes read as the four ESCs: speeds within the motor's range and the
    /// four ESC clocks within a second of each other (they sit a few counts apart in every real
    /// push; read at the wrong offset they are tens of millions apart). The battery-first order
    /// read as ESC put 10 675 rpm here (2026-10-30 22:00).
    pub fn plausible(&self) -> bool {
        let (lo, hi) = (
            self.timestamp.iter().min().copied().unwrap_or(0),
            self.timestamp.iter().max().copied().unwrap_or(0),
        );
        self.speed_rpm.iter().all(|r| r.abs() <= Self::MAX_RPM) && hi - lo < 1000
    }

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

/// The chassis's own judgement of its situation: **four bytes** (DJI's on-board library,
/// `gimbal_and_chassis_push_info_process`: `ns_sa_status = data[108:112]`, read as a `uint32`),
/// bits `static, uphill, downhill, on_slope, pick_up, slip, impact_x, impact_y, impact_z,
/// roll_over, slp_output`. The desktop SDK's two-byte reading was wrong for this push.
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
    pub impact_z: bool,
    pub roll_over: bool,
    pub slp_output: bool,
}

impl SaStatus {
    pub const SIZE: usize = 4;

    pub fn parse(b: &[u8]) -> Option<(Self, &[u8])> {
        let (me, rest) = b.split_at_checked(Self::SIZE)?;
        let word = u32::from_le_bytes([me[0], me[1], me[2], me[3]]);
        let bit = |n: u32| (word >> n) & 1 == 1;
        Some((
            Self {
                static_flag: bit(0),
                up_hill: bit(1),
                down_hill: bit(2),
                on_slope: bit(3),
                is_pick_up: bit(4),
                slip: bit(5),
                impact_x: bit(6),
                impact_y: bit(7),
                impact_z: bit(8),
                roll_over: bit(9),
                slp_output: bit(10),
            },
            rest,
        ))
    }
}

/// The gimbal's angles as the library reads the `gimbal_pos` subject: nine bytes, pitch at 2..4
/// and yaw at 4..6, tenths of a degree, little-endian signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GimbalPos {
    pub pitch_ddeg: i16,
    pub yaw_ddeg: i16,
}

impl GimbalPos {
    pub const SIZE: usize = 9;

    pub fn parse(b: &[u8]) -> Option<(Self, &[u8])> {
        let (me, rest) = b.split_at_checked(Self::SIZE)?;
        Some((
            Self {
                pitch_ddeg: i16::from_le_bytes([me[2], me[3]]),
                yaw_ddeg: i16::from_le_bytes([me[4], me[5]]),
            },
            rest,
        ))
    }
}

/// `stick_flag`: one byte, bit 0 the chassis stick live, bit 1 the gimbal stick live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stick {
    pub chassis: bool,
    pub gimbal: bool,
}

impl Stick {
    pub const SIZE: usize = 1;

    pub fn parse(b: &[u8]) -> Option<(Self, &[u8])> {
        let (me, rest) = b.split_at_checked(Self::SIZE)?;
        Some((
            Self {
                chassis: me[0] & 1 == 1,
                gimbal: (me[0] >> 1) & 1 == 1,
            },
            rest,
        ))
    }
}

/// The telemetry the stock firmware subscribes at boot and the motion controller keeps pushing —
/// to `dji_hdvt_uav` while it lives, and to whoever reads the UART after it is stopped (measured
/// 2026-10-30: 399 of these reached `s1d` with the stock dead). 161 bytes after `sub_mode` and
/// `msg_id`; the `msg_id` is not stable across boots (0 on 2026-10-30 17:05, 1 the same afternoon),
/// so this push is recognised by its length alone.
///
/// Offsets were fitted to 2362 pushes of `capture/hdvt_uav.strace` and hold on the 2026-10-29 idle
/// capture too (`tests/stock_push.rs`): the subject formats are the SDK's (`chassis.py`,
/// `battery.py`), the order is the stock's own request, which was never captured — it is
/// subscribed in the first milliseconds of `dji_hdvt_uav` — and the 14 bytes at 8..22 are one
/// subject the SDK does not name: a float that tracks the gimbal's yaw in whole degrees
/// (0, −1, −178 against the gimbal's own `0x04/0x05` push of −179.6°) and ten zero bytes.
/// Which way round the two subjects of bytes 46..92 came — the ESC's 36 and the battery's 10.
/// Both orders have been read off the robot (2026-10-30: `esc, battery` in the 17:05 capture and
/// the 29/10 push, msg ids 0 and 1; `battery, esc` in every push after 20:50 across two clean
/// boots, with and without the app, 3080 pushes), and the msg id does not tell them apart: the
/// order is read from the bytes themselves, by which way both subjects read as physics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockOrder {
    /// ESC at 46, battery at 82.
    EscThenBattery,
    /// Battery at 46, ESC at 56.
    BatteryThenEsc,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StockPush {
    /// The controller's clock, milliseconds, then the same instant in microseconds.
    pub time_ms: u32,
    pub time_us: u32,
    /// The gimbal's yaw in whole degrees, ground frame, as the unnamed subject carries it.
    pub gimbal_yaw_deg: f32,
    pub velocity: Velocity,
    pub esc: Esc,
    pub battery: Battery,
    /// The order the ESC and the battery were read in; `None` when neither order read as
    /// physics — `esc` and `battery` then hold the `EscThenBattery` reading and fail their
    /// plausibility, which is what a consumer must check before publishing either.
    pub block_order: Option<BlockOrder>,
    pub position: Position,
    pub imu: Imu,
    pub sa_status: SaStatus,
    pub attitude: Attitude,
}

impl StockPush {
    /// A push shaped like the stock's, for a test or the bench: the four wheel speeds (rpm, the
    /// wire's order and signs), the fused body velocity, a plausible pack (11.9 V, 80 %), ESC
    /// clocks a few counts apart, one g down, everything else zero — ESC before battery, the
    /// 2026-10-30 17:05 order. `time_ms` is the controller's clock.
    pub fn synthesize(time_ms: u32, esc_rpm: [i16; 4], body_velocity: [f32; 3]) -> Vec<u8> {
        let mut d = vec![0u8; Self::LEN];
        d[0..4].copy_from_slice(&time_ms.to_le_bytes());
        d[4..8].copy_from_slice(&(time_ms * 1000).to_le_bytes());
        for (i, v) in body_velocity.iter().enumerate() {
            let at = Self::AT_VELOCITY + 12 + 4 * i;
            d[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        for (i, r) in esc_rpm.iter().enumerate() {
            let at = Self::AT_ESC + 2 * i;
            d[at..at + 2].copy_from_slice(&r.to_le_bytes());
        }
        for i in 0..4 {
            let at = Self::AT_ESC + 16 + 4 * i;
            d[at..at + 4].copy_from_slice(&(1000 + i as u32).to_le_bytes());
        }
        let b = Self::AT_BATTERY;
        d[b..b + 2].copy_from_slice(&11_900u16.to_le_bytes());
        d[b + 2..b + 4].copy_from_slice(&300i16.to_le_bytes());
        d[b + 4..b + 8].copy_from_slice(&(-1000i32).to_le_bytes());
        d[b + 8] = 80;
        let z = Self::AT_IMU + 8;
        d[z..z + 4].copy_from_slice(&(-1.0f32).to_le_bytes());
        d
    }

    /// Bytes of subjects after `sub_mode` and `msg_id`.
    pub const LEN: usize = 161;
    const AT_GIMBAL_YAW: usize = 8;
    const AT_VELOCITY: usize = 22;
    const AT_ESC: usize = 46;
    const AT_BATTERY: usize = 82;
    /// The other order: battery at 46, ESC right after its ten bytes.
    const AT_BATTERY_FIRST: usize = 46;
    const AT_ESC_AFTER_BATTERY: usize = 56;
    const AT_POSITION: usize = 92;
    const AT_IMU: usize = 104;
    const AT_SA_STATUS: usize = 128;
    const AT_ATTITUDE: usize = 132;
    // 17 zero bytes follow the attitude: the servo subject an S1 has no servos for.

    /// `data` is [`PushPeriod::data`]; `None` for any other length.
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() != Self::LEN {
            return None;
        }
        let u32_at =
            |i: usize| u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
        let (velocity, _) = Velocity::parse(&data[Self::AT_VELOCITY..])?;
        let (esc_a, _) = Esc::parse(&data[Self::AT_ESC..])?;
        let (battery_a, _) = Battery::parse(&data[Self::AT_BATTERY..])?;
        let (battery_b, _) = Battery::parse(&data[Self::AT_BATTERY_FIRST..])?;
        let (esc_b, _) = Esc::parse(&data[Self::AT_ESC_AFTER_BATTERY..])?;
        let (esc, battery, block_order) = if battery_a.plausible() && esc_a.plausible() {
            (esc_a, battery_a, Some(BlockOrder::EscThenBattery))
        } else if battery_b.plausible() && esc_b.plausible() {
            (esc_b, battery_b, Some(BlockOrder::BatteryThenEsc))
        } else {
            (esc_a, battery_a, None)
        };
        let (position, _) = Position::parse(&data[Self::AT_POSITION..])?;
        let (imu, _) = Imu::parse(&data[Self::AT_IMU..])?;
        let (sa_status, _) = SaStatus::parse(&data[Self::AT_SA_STATUS..])?;
        let (attitude, _) = Attitude::parse(&data[Self::AT_ATTITUDE..])?;
        Some(Self {
            time_ms: u32_at(0),
            time_us: u32_at(4),
            gimbal_yaw_deg: f32_at(data, Self::AT_GIMBAL_YAW),
            velocity,
            esc,
            battery,
            block_order,
            position,
            imu,
            sa_status,
            attitude,
        })
    }
}
