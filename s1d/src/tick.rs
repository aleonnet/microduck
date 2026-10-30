//! The control loop: one tick every period, never waiting on a client.
//!
//! ```text
//! snapshot intents → arm/disarm? → heartbeat → wheels (deadman) → gimbal → LED → read body →
//! decode → telemetry/events → publish state → stamp
//! ```
//!
//! **The heartbeat is the chassis's permission to move.** Without it streamed continuously the
//! wheels ignore every command (bench, 2026-10-29). The stock firmware sends it at ~100 Hz; so does
//! this. Stopping it is how the chassis is disarmed, which is also what happens if this loop dies.
//!
//! **The deadman is on the twist, not the head.** A stale velocity drives the robot into a wall; a
//! stale gimbal angle holds a pose. `duck_control::safety::Config::deadman` is 500 ms and this
//! copies the number.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use s1_duml::cmd::{self, SeqCounter};
use s1_duml::dds;
use s1_duml::events::{ArmorHit, IrHit};
use s1_duml::{Decoder, Frame};

use crate::body::S1Body;
use crate::intents::{Intents, PowerRequest};
use crate::state::{State, Telemetry};

/// Which heartbeat bytes to send. The proroklab one armed the chassis on the bench; the stock one
/// is what the firmware sends and has not been tried by us yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heartbeat {
    Proroklab,
    Stock,
}

/// How the twist reaches the chassis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TwistMode {
    /// Mecanum inverse kinematics here, `0x3f/0x20` per wheel — the command the bench proved.
    Wheels,
    /// `0x3f/0x21` with the twist itself, the motion controller doing the kinematics — the SDK's
    /// `drive_speed`, not yet tried on the UART.
    Speed,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub hz: u32,
    pub deadman: Duration,
    pub heartbeat: Heartbeat,
    pub twist_mode: TwistMode,
    /// Wheel radius, metres. 0.05 is the URDF's collision cylinder (jeguzzi/robomaster_ros); the
    /// bench measures the real one by rolling a known distance.
    pub wheel_radius_m: f64,
    /// Half the wheelbase plus half the track, metres: the wheel joints sit at (±0.1, ±0.1).
    pub half_sum_lw_m: f64,
    /// Telemetry rate asked of the motion controller, one of `dds::FREQUENCIES`.
    pub dds_hz: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hz: 100,
            deadman: Duration::from_millis(500),
            heartbeat: Heartbeat::Proroklab,
            twist_mode: TwistMode::Wheels,
            wheel_radius_m: 0.05,
            half_sum_lw_m: 0.2,
            dds_hz: 20,
        }
    }
}

/// The SDK's documented wheel range, rpm.
pub const WHEEL_RPM_MAX: f64 = 1000.0;
/// Gimbal limits from the manual, tenths of a degree.
pub const YAW_DDEG_MAX: i16 = 2500;
pub const PITCH_DDEG_MIN: i16 = -200;
pub const PITCH_DDEG_MAX: i16 = 350;
/// Our subscription's id in the motion controller, and how often it is re-asked until it answers.
const DDS_MSG_ID: u8 = 1;
const DDS_RETRY: Duration = Duration::from_secs(5);
/// The gimbal command is an action; it is re-sent at this rate while the target is unchanged, and
/// at once when it changes. The PoC sent it at 20 Hz.
const GIMBAL_HOLD_PERIOD: Duration = Duration::from_millis(100);
/// Subjects we subscribe to, in the order they come back concatenated.
pub const SUBJECTS: [u64; 7] = [
    dds::uid::BATTERY,
    dds::uid::ATTITUDE,
    dds::uid::POSITION,
    dds::uid::VELOCITY,
    dds::uid::IMU,
    dds::uid::ESC,
    dds::uid::SA_STATUS,
];

/// Whether to (re)ask the motion controller for telemetry now.
///
/// At once when never asked; then only when the last ask is `retry` old **and** no push has
/// arrived for `retry` — a controller power-cycled under us has forgotten the subscription, and
/// this is how it gets it back without a re-arm, while a controller that is pushing is left alone.
pub fn dds_due(
    last_sub: Option<Instant>,
    last_push: Option<Instant>,
    retry: Duration,
    now: Instant,
) -> bool {
    match last_sub {
        None => true,
        Some(asked) => {
            now.duration_since(asked) >= retry
                && last_push.is_none_or(|pushed| now.duration_since(pushed) >= retry)
        }
    }
}

/// Twist in the robot's frame (x forward, y left, yaw counter-clockwise) to wheel rpm, in the
/// order the wire wants: front-right, front-left, back-left, back-right.
///
/// Textbook mecanum with the rollers at 45°: `ω_fl = (vx − vy − (L+W)·ωz)/r`, `ω_fr = (vx + vy +
/// (L+W)·ωz)/r`, `ω_bl = (vx + vy − (L+W)·ωz)/r`, `ω_br = (vx − vy + (L+W)·ωz)/r`. Which wheel
/// turns which way on *this* chassis is the bench's to confirm with the wheels in the air; the
/// signs here are the textbook's and the PoC's "all four positive drives forward".
pub fn mecanum_rpm(twist: [f64; 3], radius_m: f64, half_sum_lw_m: f64) -> [i16; 4] {
    let [vx, vy, wz] = twist;
    let k = half_sum_lw_m * wz;
    let to_rpm = |v: f64| {
        ((v / radius_m) * 60.0 / std::f64::consts::TAU)
            .clamp(-WHEEL_RPM_MAX, WHEEL_RPM_MAX)
            .round() as i16
    };
    let fl = to_rpm(vx - vy - k);
    let fr = to_rpm(vx + vy + k);
    let bl = to_rpm(vx + vy - k);
    let br = to_rpm(vx - vy + k);
    [fr, fl, bl, br]
}

/// Head joints (`[neck_pitch, head_pitch, head_yaw, head_roll]`, radians) to gimbal tenths of a
/// degree, clamped to the manual's travel. Neck and roll are ignored: the gimbal has two axes.
pub fn gimbal_ddeg(head: [f64; 4]) -> (i16, i16) {
    let yaw = (head[2].to_degrees() * 10.0)
        .round()
        .clamp(-f64::from(YAW_DDEG_MAX), f64::from(YAW_DDEG_MAX)) as i16;
    let pitch = (head[1].to_degrees() * 10.0)
        .round()
        .clamp(f64::from(PITCH_DDEG_MIN), f64::from(PITCH_DDEG_MAX)) as i16;
    (yaw, pitch)
}

/// Decode one of our pushes into telemetry, in `SUBJECTS` order. Subjects after a short push are
/// left as they were — the bench has yet to confirm the controller sends every subject every time.
fn decode_push(data: &[u8], t: &mut Telemetry) {
    let mut rest = data;
    if let Some((v, r)) = dds::Battery::parse(rest) {
        t.battery = Some(v);
        rest = r;
    } else {
        return;
    }
    if let Some((v, r)) = dds::Attitude::parse(rest) {
        t.attitude = Some(v);
        rest = r;
    } else {
        return;
    }
    if let Some((v, r)) = dds::Position::parse(rest) {
        t.position = Some(v);
        rest = r;
    } else {
        return;
    }
    if let Some((v, r)) = dds::Velocity::parse(rest) {
        t.velocity = Some(v);
        rest = r;
    } else {
        return;
    }
    if let Some((v, r)) = dds::Imu::parse(rest) {
        t.imu = Some(v);
        rest = r;
    } else {
        return;
    }
    if let Some((v, r)) = dds::Esc::parse(rest) {
        t.esc = Some(v);
        rest = r;
    } else {
        return;
    }
    if let Some((v, _)) = dds::SaStatus::parse(rest) {
        t.sa_status = Some(v);
    }
}

struct Writer<'a> {
    body: &'a mut dyn S1Body,
    state: &'a State,
    seq: SeqCounter,
}

impl Writer<'_> {
    fn send(&mut self, frame: Frame) {
        let bytes = match frame.encode() {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(error = %e, "frame too long; not sent");
                return;
            }
        };
        match self.body.write_frame(&bytes) {
            Ok(()) => {
                self.state
                    .consecutive_write_errors
                    .store(0, Ordering::Relaxed);
            }
            Err(e) => {
                let n = self
                    .state
                    .consecutive_write_errors
                    .fetch_add(1, Ordering::Relaxed)
                    + 1;
                self.state
                    .write_errors_total
                    .fetch_add(1, Ordering::Relaxed);
                if n == 1 || n.is_multiple_of(100) {
                    tracing::warn!(error = %e, consecutive = n, "serial write failed");
                }
            }
        }
    }

    fn seq(&mut self, cmd: (u8, u8)) -> u16 {
        self.seq.next(cmd)
    }
}

/// Run the loop until `state.shutdown` is set. Returns after zeroing the wheels.
pub fn run(body: &mut dyn S1Body, state: Arc<State>, intents: Arc<Intents>, cfg: Config) {
    let period = Duration::from_micros(state.period_us);
    let mut w = Writer {
        body,
        state: &state,
        seq: SeqCounter::new(),
    };
    let mut decoder = Decoder::new();
    let mut inbox = vec![0u8; 4096];
    let mut telemetry = Telemetry::default();
    let mut last_gimbal: Option<((i16, i16), Instant)> = None;
    let mut last_sub: Option<Instant> = None;
    let mut last_push: Option<Instant> = None;
    let mut window_start = Instant::now();
    let mut window_ticks: u64 = 0;
    let mut next = Instant::now();

    tracing::info!(hz = cfg.hz, deadman_ms = cfg.deadman.as_millis() as u64, ?cfg.heartbeat, ?cfg.twist_mode, "control loop running");

    while !state.shutdown.load(Ordering::Relaxed) {
        let tick_start = Instant::now();
        let snap = intents.snapshot();

        match intents.take_power_request() {
            Some(PowerRequest::Arm) => {
                // The PoC sent work mode 1 five times before driving; the heartbeat starts below.
                for _ in 0..5 {
                    let s = w.seq(cmd::id::CHASSIS_WORKMODE);
                    w.send(cmd::chassis_workmode(s, 1));
                }
                state.armed.store(true, Ordering::Relaxed);
                last_sub = None;
                tracing::info!("armed: heartbeat on, chassis in work mode 1");
            }
            Some(PowerRequest::Disarm) => {
                zero_wheels(&mut w, 10);
                state.armed.store(false, Ordering::Relaxed);
                state.applied_twist.store(Arc::new([0.0; 3]));
                tracing::info!("disarmed: wheels zeroed, heartbeat off");
            }
            None => {}
        }

        let armed = state.armed.load(Ordering::Relaxed);
        let mut limited_by = Vec::new();
        if armed {
            let s = w.seq(cmd::id::HEARTBEAT);
            w.send(match cfg.heartbeat {
                Heartbeat::Proroklab => cmd::heartbeat_proroklab(s),
                Heartbeat::Stock => cmd::heartbeat_stock(s),
            });

            let twist = if snap.twist_age > cfg.deadman {
                if snap.twist != [0.0; 3] {
                    limited_by.push("deadman".to_owned());
                }
                [0.0; 3]
            } else {
                snap.twist
            };
            match cfg.twist_mode {
                TwistMode::Wheels => {
                    let [fr, fl, bl, br] =
                        mecanum_rpm(twist, cfg.wheel_radius_m, cfg.half_sum_lw_m);
                    let s = w.seq(cmd::id::CHASSIS_WHEEL_SPEED);
                    w.send(cmd::chassis_wheel_speed(s, fr, fl, bl, br));
                }
                TwistMode::Speed => {
                    let s = w.seq(cmd::id::CHASSIS_SPEED);
                    w.send(cmd::chassis_speed(
                        s,
                        twist[0] as f32,
                        twist[1] as f32,
                        twist[2].to_degrees() as f32,
                    ));
                }
            }
            state.applied_twist.store(Arc::new(twist));

            let target = gimbal_ddeg(snap.head);
            let due = match last_gimbal {
                Some((prev, at)) => prev != target || at.elapsed() >= GIMBAL_HOLD_PERIOD,
                None => true,
            };
            if due {
                let s = w.seq(cmd::id::GIMBAL_ROTATE);
                w.send(cmd::gimbal_angles(s, target.0, target.1));
                state.gimbal_ddeg.store(Arc::new(target));
                last_gimbal = Some((target, Instant::now()));
            }

            if dds_due(last_sub, last_push, DDS_RETRY, Instant::now()) {
                let s = w.seq(cmd::id::DDS_ADD_SUB);
                w.send(dds::add_sub(
                    s,
                    cmd::host::BASE,
                    DDS_MSG_ID,
                    &SUBJECTS,
                    cfg.dds_hz,
                    false,
                ));
                last_sub = Some(Instant::now());
            }
        }

        if let Some(led) = intents.take_led() {
            let s = w.seq(cmd::id::SYSTEM_LED);
            w.send(cmd::system_led(s, &led));
        }

        // Whatever the body pushed: telemetry, hits, acks.
        match w.body.read_available(&mut inbox) {
            Ok(0) => {}
            Ok(n) => {
                let mut changed = false;
                for frame in decoder.push(&inbox[..n]) {
                    if let Some(push) = dds::PushPeriod::parse(&frame) {
                        if push.msg_id == DDS_MSG_ID {
                            decode_push(push.data, &mut telemetry);
                            telemetry.pushes += 1;
                            last_push = Some(Instant::now());
                            changed = true;
                        }
                    } else if let Some(hit) = ArmorHit::parse(&frame) {
                        state.armor_hits.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(
                            plate = hit.index,
                            kind = hit.kind,
                            mic = hit.mic_value,
                            "armour hit"
                        );
                    } else if let Some(ir) = IrHit::parse(&frame) {
                        state.ir_hits.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(
                            role = ir.role_id,
                            skill = ir.skill_id,
                            dev = ir.recv_dev,
                            pin = ir.recv_ir_pin,
                            "infrared hit"
                        );
                    } else {
                        tracing::trace!(
                            cmd_set = frame.cmd_set,
                            cmd_id = frame.cmd_id,
                            len = frame.data.len(),
                            "frame"
                        );
                    }
                }
                if changed {
                    state.telemetry.store(Arc::new(telemetry.clone()));
                }
                state
                    .decoder_dropped
                    .store(decoder.dropped(), Ordering::Relaxed);
            }
            Err(e) => tracing::debug!(error = %e, "serial read failed"),
        }

        // Publish only when somebody listens: assembling a frame allocates.
        if state.state_tx.receiver_count() > 0 {
            let _ = state.state_tx.send(state.frame(snap.twist, limited_by));
        }

        // Stamp, count, and keep the rate honest.
        state.ticks.fetch_add(1, Ordering::Relaxed);
        state.last_tick_us.store(state.now_us(), Ordering::Relaxed);
        window_ticks += 1;
        if window_start.elapsed() >= Duration::from_secs(1) {
            let hz = window_ticks as f64 / window_start.elapsed().as_secs_f64();
            state.achieved_hz.store(hz.to_bits(), Ordering::Relaxed);
            window_start = Instant::now();
            window_ticks = 0;
        }
        if tick_start.elapsed() > period {
            state.missed.fetch_add(1, Ordering::Relaxed);
        }
        next += period;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            // Fell behind: resynchronise rather than burst-catch-up on the UART.
            next = now;
        }
    }

    // Whatever stopped us, the wheels stop too — the PoC's `finally`.
    zero_wheels(&mut w, 10);
    state.armed.store(false, Ordering::Relaxed);
    tracing::info!("control loop stopped; wheels zeroed");
}

fn zero_wheels(w: &mut Writer<'_>, times: usize) {
    for _ in 0..times {
        let s = w.seq(cmd::id::CHASSIS_WHEEL_SPEED);
        w.send(cmd::chassis_wheel_speed(s, 0, 0, 0, 0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Forward drives every wheel the same way; a pure turn drives the two sides against each
    /// other; the clamp holds at the SDK's range.
    #[test]
    fn mecanum_signs_and_clamp() {
        let fwd = mecanum_rpm([0.5, 0.0, 0.0], 0.05, 0.2);
        assert!(
            fwd.iter().all(|&w| w > 0) && fwd.iter().all(|&w| w == fwd[0]),
            "{fwd:?}"
        );
        let [fr, fl, bl, br] = mecanum_rpm([0.0, 0.0, 1.0], 0.05, 0.2);
        assert!(
            fr > 0 && br > 0 && fl < 0 && bl < 0,
            "turn left: right side forward, left back"
        );
        let fast = mecanum_rpm([100.0, 0.0, 0.0], 0.05, 0.2);
        assert!(fast.iter().all(|&w| w == 1000));
        // 0.5 m/s on a 0.05 m wheel is 10 rad/s = 95.5 rpm.
        assert_eq!(fwd[0], 95);
    }

    /// Asked at once; not again while pushes arrive; again once the controller has been silent
    /// for the retry, which is what a controller power-cycled under us looks like.
    #[test]
    fn telemetry_is_reasked_only_after_silence() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let retry = Duration::from_secs(5);
        assert!(dds_due(None, None, retry, t0));
        assert!(!dds_due(Some(t0), None, retry, at(4)), "too soon");
        assert!(dds_due(Some(t0), None, retry, at(5)), "never answered");
        assert!(!dds_due(Some(t0), Some(at(3)), retry, at(6)), "pushing");
        assert!(
            dds_due(Some(t0), Some(at(3)), retry, at(8)),
            "silent for the retry"
        );
    }

    #[test]
    fn gimbal_is_clamped_to_the_manual() {
        assert_eq!(gimbal_ddeg([0.0, 0.0, 0.0, 0.0]), (0, 0));
        assert_eq!(
            gimbal_ddeg([0.0, 10f64.to_radians(), 15f64.to_radians(), 0.0]),
            (150, 100)
        );
        assert_eq!(
            gimbal_ddeg([0.0, -1.0, 7.0, 0.0]),
            (YAW_DDEG_MAX, PITCH_DDEG_MIN)
        );
        assert_eq!(
            gimbal_ddeg([0.0, 1.0, -7.0, 0.0]),
            (-YAW_DDEG_MAX, PITCH_DDEG_MAX)
        );
    }
}
