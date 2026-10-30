//! What clients are asking the body to do — the slots `robotd/src/intents.rs` keeps, cut down to
//! what this body has: a twist, a head pose, an arm/disarm request, an LED request.
//!
//! Each continuous slot is an `ArcSwap` of a stamped value: the loop does one atomic load per tick
//! and the IPC task one store, and neither waits for the other. Twist and head are separate slots
//! so a client steering the gimbal never clobbers a client driving the chassis. Every slot is
//! stamped because the loop's question is "how old is it", which is what the deadman reads; the
//! twist starts maximally stale so a daemon that just came up holds the chassis still.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use s1_duml::Frame;
use s1_duml::cmd::SystemLed;

#[derive(Debug, Clone, Copy)]
struct Stamped<T> {
    value: T,
    at_us: u64,
}

/// A pending arm or disarm, taken once by the loop. Alternatives, not two flags: the later one wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerRequest {
    /// Start the heartbeat and the chassis work mode: the wheels obey from here on.
    Arm,
    /// Zero the wheels and stop the heartbeat: the chassis disarms itself.
    Disarm,
}

const POWER_NONE: u8 = 0;
const POWER_ARM: u8 = 1;
const POWER_DISARM: u8 = 2;

/// Who leads whom — the app's follow button, as DJI's on-board library models it
/// (`RobotCtrl.set_mode`: `free`, `gimbal_lead` = chassis follows the gimbal, `chassis_lead` =
/// gimbal follows the chassis).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowMode {
    /// Chassis and gimbal independent: the twist goes out as wheel speeds, the gimbal by angle.
    Free = 0,
    /// The chassis turns to keep the gimbal's heading (`0x3f/0x22` carries the twist); the
    /// twist's yaw rate is ignored, the gimbal leads.
    GimbalLead = 1,
    /// The gimbal's yaw follows the chassis (gimbal work mode 2); the twist drives as in `Free`.
    ChassisLead = 2,
}

impl FollowMode {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "free" => Some(Self::Free),
            "gimbal_lead" => Some(Self::GimbalLead),
            "chassis_lead" => Some(Self::ChassisLead),
            _ => None,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::GimbalLead,
            2 => Self::ChassisLead,
            _ => Self::Free,
        }
    }
}

/// Frames queued for the loop per tick, so a burst from the socket cannot starve the heartbeat.
pub const MAX_FRAMES_PER_TICK: usize = 4;

pub struct Intents {
    epoch: Instant,
    twist: ArcSwap<Stamped<[f64; 3]>>,
    head: ArcSwap<Stamped<[f64; 4]>>,
    enabled: AtomicBool,
    power: AtomicU8,
    /// The next LED effect to send, taken once. A mutex because it carries a struct; the loop uses
    /// `try_lock`, so it never waits.
    led: Mutex<Option<SystemLed>>,
    /// One-shot frames the bench (or a skill) asks the loop to put on the wire, in order. The loop
    /// gives each its own sequence number; the caller only says what.
    frames: Mutex<VecDeque<Frame>>,
    follow: AtomicU8,
}

/// What the loop reads at the top of a tick.
#[derive(Debug, Clone, Copy)]
pub struct Snapshot {
    pub twist: [f64; 3],
    pub twist_age: Duration,
    pub head: [f64; 4],
    pub head_age: Duration,
    pub enabled: bool,
    pub follow: FollowMode,
}

impl Default for Intents {
    fn default() -> Self {
        Self::new()
    }
}

impl Intents {
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
            // Stamped at zero: maximally stale before any client speaks, so the deadman holds.
            twist: ArcSwap::from_pointee(Stamped {
                value: [0.0; 3],
                at_us: 0,
            }),
            head: ArcSwap::from_pointee(Stamped {
                value: [0.0; 4],
                at_us: 0,
            }),
            enabled: AtomicBool::new(false),
            power: AtomicU8::new(POWER_NONE),
            led: Mutex::new(None),
            frames: Mutex::new(VecDeque::new()),
            follow: AtomicU8::new(FollowMode::Free as u8),
        }
    }

    fn now_us(&self) -> u64 {
        self.epoch.elapsed().as_micros() as u64
    }

    pub fn set_twist(&self, twist: [f64; 3]) {
        self.twist.store(Arc::new(Stamped {
            value: twist,
            at_us: self.now_us(),
        }));
    }

    pub fn set_head(&self, head: [f64; 4]) {
        self.head.store(Arc::new(Stamped {
            value: head,
            at_us: self.now_us(),
        }));
    }

    /// Zero the velocity now. Deliberate, unlike the deadman.
    pub fn stop(&self) {
        self.set_twist([0.0; 3]);
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
        self.power
            .store(if on { POWER_ARM } else { POWER_DISARM }, Ordering::Relaxed);
    }

    pub fn take_power_request(&self) -> Option<PowerRequest> {
        match self.power.swap(POWER_NONE, Ordering::Relaxed) {
            POWER_ARM => Some(PowerRequest::Arm),
            POWER_DISARM => Some(PowerRequest::Disarm),
            _ => None,
        }
    }

    pub fn request_led(&self, led: SystemLed) {
        if let Ok(mut slot) = self.led.lock() {
            *slot = Some(led);
        }
    }

    pub fn take_led(&self) -> Option<SystemLed> {
        self.led
            .try_lock()
            .ok()
            .and_then(|mut slot: MutexGuard<'_, Option<SystemLed>>| slot.take())
    }

    pub fn request_frame(&self, frame: Frame) {
        if let Ok(mut queue) = self.frames.lock() {
            queue.push_back(frame);
        }
    }

    /// Up to `MAX_FRAMES_PER_TICK` queued frames, oldest first; `try_lock`, so the loop never waits.
    pub fn take_frames(&self) -> Vec<Frame> {
        match self.frames.try_lock() {
            Ok(mut queue) => {
                let n = queue.len().min(MAX_FRAMES_PER_TICK);
                queue.drain(..n).collect()
            }
            Err(_) => Vec::new(),
        }
    }

    pub fn follow(&self) -> FollowMode {
        FollowMode::from_u8(self.follow.load(Ordering::Relaxed))
    }

    pub fn set_follow(&self, mode: FollowMode) {
        self.follow.store(mode as u8, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> Snapshot {
        let now = self.now_us();
        let twist = self.twist.load();
        let head = self.head.load();
        Snapshot {
            twist: twist.value,
            twist_age: Duration::from_micros(now.saturating_sub(twist.at_us)),
            head: head.value,
            head_age: Duration::from_micros(now.saturating_sub(head.at_us)),
            enabled: self.enabled.load(Ordering::Relaxed),
            follow: self.follow(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A daemon that comes up believing it has a live driver would run its deadman from "now".
    #[test]
    fn the_twist_starts_stale() {
        let i = Intents::new();
        assert!(i.snapshot().twist_age >= Duration::ZERO);
        assert_eq!(i.snapshot().twist, [0.0; 3]);
        assert!(!i.snapshot().enabled);
    }

    #[test]
    fn head_and_twist_are_independent() {
        let i = Intents::new();
        i.set_twist([0.5, 0.0, 0.1]);
        std::thread::sleep(Duration::from_millis(5));
        i.set_head([0.0, 0.2, 0.3, 0.0]);
        let s = i.snapshot();
        assert_eq!(s.twist, [0.5, 0.0, 0.1]);
        assert!(s.twist_age >= Duration::from_millis(5));
        assert!(s.head_age < Duration::from_millis(5));
    }

    /// Queued frames come out in order and at most a tick's worth at a time; what is left waits.
    #[test]
    fn frames_are_taken_in_order_and_rate_limited() {
        let i = Intents::new();
        for n in 0..6u8 {
            i.request_frame(Frame::request(9, 3, u16::from(n), 0x3f, 0x19, vec![n]));
        }
        let first: Vec<u16> = i.take_frames().iter().map(|f| f.seq).collect();
        assert_eq!(first, vec![0, 1, 2, 3]);
        assert_eq!(i.take_frames().len(), 2);
        assert!(i.take_frames().is_empty());
        assert_eq!(
            FollowMode::parse("gimbal_lead"),
            Some(FollowMode::GimbalLead)
        );
        assert_eq!(FollowMode::parse("tank"), None);
        i.set_follow(FollowMode::ChassisLead);
        assert_eq!(i.snapshot().follow, FollowMode::ChassisLead);
    }

    /// The later request replaces the earlier one unread: asked to arm then disarm in one tick, the
    /// second is what was meant.
    #[test]
    fn power_requests_are_taken_once_and_last_writer_wins() {
        let i = Intents::new();
        i.set_enabled(true);
        i.set_enabled(false);
        assert_eq!(i.take_power_request(), Some(PowerRequest::Disarm));
        assert_eq!(i.take_power_request(), None);
    }
}
