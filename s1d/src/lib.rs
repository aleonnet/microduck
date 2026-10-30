//! The RoboMaster S1 as a microduck robot: `robot.*` in, DUML out. See `Cargo.toml` for why this
//! is not a port of `robotd`, and `docs/project/s1-bringup.md` for what the bench has and has not
//! yet confirmed about each byte.
//!
//! A library and a thin binary, so the integration tests can drive the control loop against a body
//! that writes to a file instead of a UART — which is how the deadman is proved without a robot.

pub mod api;
pub mod body;
pub mod button;
pub mod intents;
pub mod ipc;
pub mod sound;
pub mod state;
pub mod tick;

/// Socket mode, as every daemon in this workspace: owner and group, nothing for others.
pub const SOCKET_MODE: u32 = 0o660;
/// Longest request line accepted, as `robotd`.
pub const MAX_LINE: usize = 64 * 1024;
/// The model API this body implements. `robotd`'s is 2; this one answers the same so an
/// `updaterd` asking `robot.modelApi` is not refused — the policies it gates do not run here anyway.
pub const MODEL_API: u32 = 2;
