//! DUML/V1 for the RoboMaster S1: the wire the intelligent controller shares with the motion
//! controller on `/dev/ttyS3`.
//!
//! What is known for certain, and how: the frame shape and both CRCs were checked against frames
//! captured on the robot (`tests/fixtures/*_real.bin`); the chassis and gimbal commands moved the real
//! robot on 2026-10-29 (`tests/fixtures/poc_*.bin` are the exact bytes that did); the remaining layouts
//! — telemetry subscriptions, armour and infrared events, the system LEDs — are quoted from DJI's SDK
//! and are marked in their own docs with what the bench still has to confirm.
//!
//! Hosts on this bus are addressed as `index * 32 + host` (`protocol.host2byte` in the SDK). The
//! controller this code runs on is host 9; the motion controller is 3; the chassis is 3 at index 6
//! (`0xC3`); the gimbal is 4. [`cmd::host`] names them.

pub mod cmd;
pub mod crc;
pub mod dds;
pub mod events;
pub mod frame;

pub use frame::{DecodeError, Decoder, Frame};
