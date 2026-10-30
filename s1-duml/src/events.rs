//! Events the body pushes without being asked: a hit on an armour plate, an infrared hit.
//!
//! Layouts from DJI's `protocol.py` (`ProtoArmorHitEvent`, `ProtoIrHitEvent`). The SDK's own
//! `armor.py` unpacks the same bytes differently, so **which plate is which is a bench question**:
//! the `sub_hit_event` docstring numbers them 1 chassis back, 2 front, 3 left, 4 right, 5 gimbal
//! left, 6 gimbal right, and tapping each one under `s1d`'s log settles it.

use crate::cmd::id;
use crate::frame::Frame;

/// `0x3f/0x02`: a plate was struck.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmorHit {
    /// Plate number, high nibble of the first byte.
    pub index: u8,
    /// `0` a gel bead (the SDK's "water"), `1` an infrared beam — low nibble.
    pub kind: u8,
    pub mic_value: u16,
    pub mic_len: u16,
}

impl ArmorHit {
    pub fn parse(frame: &Frame) -> Option<Self> {
        if (frame.cmd_set, frame.cmd_id) != id::ARMOR_HIT || frame.data.len() < 5 {
            return None;
        }
        let d = &frame.data;
        Some(Self {
            index: d[0] >> 4,
            kind: d[0] & 0x0f,
            mic_value: u16::from_le_bytes([d[1], d[2]]),
            mic_len: u16::from_le_bytes([d[3], d[4]]),
        })
    }
}

/// `0x3f/0x10`: an infrared beam from another robot landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrHit {
    pub role_id: u8,
    pub skill_id: u8,
    pub recv_dev: u8,
    pub recv_ir_pin: u8,
}

impl IrHit {
    pub fn parse(frame: &Frame) -> Option<Self> {
        if (frame.cmd_set, frame.cmd_id) != id::IR_HIT || frame.data.len() < 3 {
            return None;
        }
        let d = &frame.data;
        Some(Self {
            role_id: d[0] >> 4,
            skill_id: d[0] & 0x0f,
            recv_dev: d[1],
            recv_ir_pin: d[2],
        })
    }
}

/// `0x04/0x05`: the gimbal's own attitude, pushed by the gimbal (sender `0x04`) to host `0xf1`
/// about 21 times a second while the stock runs (2969 in 142 s of `capture/hdvt_uav.strace`,
/// 2026-10-30), and the only place the gimbal's real angle travels on the UART: the stock's DDS
/// telemetry carries it only as a whole-degree yaw.
///
/// 40 bytes. Fitted on all 2969 pushes and pinned by the extremes (`tests/stock_push.rs`): pitch
/// reaches +35.8° where the manual puts the limit at +35°, yaw spans ±179.6°, and the quaternion at
/// 24..40 agrees with the yaw (a −179.6° yaw gives w ≈ 0.006, z ≈ −0.95). The yaw at 8 is 21° less
/// than the one at 4 when the chassis had turned 19°: the first is in the ground frame, the second
/// relative to the chassis — a reading consistent with the data, confirmed on the robot by driving
/// the gimbal to its stops (the bench's telemetry stage).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GimbalPush {
    /// Tenths of a degree, up positive (DJI's sign).
    pub pitch_ddeg: i16,
    /// Tenths of a degree, ground frame, right positive (DJI's sign).
    pub yaw_ddeg: i16,
    /// Tenths of a degree, relative to the chassis heading.
    pub yaw_vs_chassis_ddeg: i16,
    /// The gimbal's clock, milliseconds — the same clock as the telemetry push.
    pub time_ms: u32,
    /// Attitude quaternion, `[w, x, y, z]`, unit length at rest (`w` ≈ 1).
    pub quat: [f32; 4],
}

impl GimbalPush {
    pub const LEN: usize = 40;

    pub fn parse(frame: &Frame) -> Option<Self> {
        if (frame.cmd_set, frame.cmd_id) != id::GIMBAL_POSITION_PUSH || frame.data.len() < Self::LEN
        {
            return None;
        }
        let d = &frame.data;
        let i16_at = |i: usize| i16::from_le_bytes([d[i], d[i + 1]]);
        let f32_at = |i: usize| f32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
        Some(Self {
            pitch_ddeg: i16_at(0),
            yaw_ddeg: i16_at(4),
            yaw_vs_chassis_ddeg: i16_at(8),
            time_ms: u32::from_le_bytes([d[12], d[13], d[14], d[15]]),
            quat: [f32_at(24), f32_at(28), f32_at(32), f32_at(36)],
        })
    }

    pub fn pitch_deg(&self) -> f64 {
        f64::from(self.pitch_ddeg) / 10.0
    }

    pub fn yaw_deg(&self) -> f64 {
        f64::from(self.yaw_ddeg) / 10.0
    }
}
