//! The "Autonomous Program" button on the intelligent controller, read from its evdev node.
//!
//! The stock firmware holds `/dev/input/event0` (seen among `dji_hdvt_uav`'s file descriptors).
//! Today it only reads it, counts presses and reports them as `s1.event`. Making it the switch
//! between the DJI mode and ours is a plan, with no code.
//!
//! `struct input_event` is `timeval` + `u16 type` + `u16 code` + `i32 value`. On the robot's
//! 32-bit ARM that is 16 bytes; on a 64-bit host it is 24. Sized at compile time for the target.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::state::State;

#[cfg(target_pointer_width = "32")]
const EVENT_SIZE: usize = 16;
#[cfg(target_pointer_width = "64")]
const EVENT_SIZE: usize = 24;
const TIME_SIZE: usize = EVENT_SIZE - 8;

const EV_KEY: u16 = 1;
/// Where the intelligent controller's button is (among `dji_hdvt_uav`'s open files).
pub const DEFAULT_DEVICE: &str = "/dev/input/event0";
/// The size of one `struct input_event` on this build's target.
pub const EVENT_LEN: usize = EVENT_SIZE;

/// Parse one raw event into `(type, code, value)`.
pub fn parse_event(raw: &[u8]) -> Option<(u16, u16, i32)> {
    if raw.len() < EVENT_SIZE {
        return None;
    }
    let kind = u16::from_ne_bytes([raw[TIME_SIZE], raw[TIME_SIZE + 1]]);
    let code = u16::from_ne_bytes([raw[TIME_SIZE + 2], raw[TIME_SIZE + 3]]);
    let value = i32::from_ne_bytes([
        raw[TIME_SIZE + 4],
        raw[TIME_SIZE + 5],
        raw[TIME_SIZE + 6],
        raw[TIME_SIZE + 7],
    ]);
    Some((kind, code, value))
}

/// Read `path` until shutdown, logging key presses and counting them. Blocking reads on a thread
/// of its own, because the control loop must never wait on a button.
pub fn watch(path: std::path::PathBuf, state: Arc<State>) {
    let mut file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) => {
            tracing::info!(path = %path.display(), error = %e, "no button to read; carrying on without it");
            return;
        }
    };
    tracing::info!(path = %path.display(), "watching the button");
    let mut raw = [0u8; EVENT_SIZE];
    while !state.shutdown.load(Ordering::Relaxed) {
        match file.read_exact(&mut raw) {
            Ok(()) => {
                if let Some((EV_KEY, code, value)) = parse_event(&raw) {
                    if value == 1 {
                        state.button_presses.fetch_add(1, Ordering::Relaxed);
                        let _ = state
                            .event_tx
                            .send(crate::api::Event::Button { code, value });
                    }
                    tracing::info!(code, value, "button");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "button read failed; stopping the watcher");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_press_parses_at_the_targets_event_size() {
        let mut raw = vec![0u8; EVENT_SIZE];
        raw[TIME_SIZE..TIME_SIZE + 2].copy_from_slice(&EV_KEY.to_ne_bytes());
        raw[TIME_SIZE + 2..TIME_SIZE + 4].copy_from_slice(&0x74u16.to_ne_bytes());
        raw[TIME_SIZE + 4..].copy_from_slice(&1i32.to_ne_bytes());
        assert_eq!(parse_event(&raw), Some((EV_KEY, 0x74, 1)));
        assert_eq!(parse_event(&raw[..EVENT_SIZE - 1]), None);
    }
}
