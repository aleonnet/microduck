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
