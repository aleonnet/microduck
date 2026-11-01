//! DUML/V1 for the RoboMaster S1: the wire the intelligent controller shares with the motion
//! controller on `/dev/ttyS3`.
//!
//! What is known for certain, and how: the frame shape and both CRCs were checked against frames
//! captured on the robot (`tests/fixtures/*_real.bin`); the chassis and gimbal commands moved the real
//! robot on 2026-10-29 (`tests/fixtures/poc_*.bin` are the exact bytes that did); the stock's telemetry
//! push, armour and infrared events and the system LEDs were read off the robot; other builders are
//! held to DJI's own packer (`tests/dji_oracle.rs`), which proves the bytes, not the effect.
//!
//! Hosts on this bus are addressed as `index * 32 + host` (`protocol.host2byte` in the SDK). The
//! controller this code runs on is host 9; the motion controller is 3; the chassis is 3 at index 6
//! (`0xC3`); the gimbal is 4. [`cmd::host`] names them.

pub mod cmd;
pub mod crc;
pub mod dds;
pub mod events;
pub mod frame;

pub use frame::{Ack, DecodeError, Decoder, Frame};
