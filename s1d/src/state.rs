//! What the loop publishes and the IPC side reads: counters, the latest telemetry, and the health
//! verdict computed from them — never by asking the loop, so a wedged loop reports itself.
//!
//! The verdict follows `robotd`'s `fn health`: consecutive write failures past a ceiling, a loop
//! that stopped stamping, a rate below the floor. Everything else on the answer is a description.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use arc_swap::ArcSwap;
use duck_ipc_proto as proto;

/// The controller speaks DJI's frames and the duck speaks its own; they differ by a half turn
/// about x. DJI's body is x forward, y right, z down (NED), its yaw/pitch/roll ZYX in degrees with
/// yaw positive clockwise and pitch positive nose up; the duck's trunk is x forward, y left, z up,
/// its world the IMU's gravity-aligned frame with z up (`duck_ipc_proto::ImuState`). Measured on
/// the robot, 2026-10-30: at rest the accelerometer reads `[0, 0, −1.0]` g; with the chassis held
/// nose-up at +21.7° it read `acc_x = +0.36 = sin 21.7°` (`telemetry-free/tilt.ndjson`), which is
/// NED specific force; the gimbal's yaw went to DJI's +60 when asked right (`drive`, morning).
/// jeguzzi's `robomaster_sim` reaches the same mapping from its x-forward, y-left, z-up body
/// (`command_subjects.hpp`: `from_robot` negates yaw and pitch, keeps roll; `ImuSubject` negates
/// acc and gyro y and z). So a vector goes `[x, −y, −z]`, and a quaternion `[w, x, −y, −z]`.
fn ned_to_trunk(v: [f32; 3]) -> [f64; 3] {
    [f64::from(v[0]), -f64::from(v[1]), -f64::from(v[2])]
}

/// Gravity as `SafetyState::gravity` states it — where gravity pulls, in the trunk frame, upright
/// about `[0, 0, −1]`. The accelerometer reads specific force, which at rest is minus gravity:
/// `[0, 0, −1]` in NED is "up"; gravity in NED is `−acc`, and in the trunk frame `[−ax, ay, az]`.
pub fn gravity_in_trunk(acc_ned_g: [f32; 3]) -> [f64; 3] {
    let g = ned_to_trunk(acc_ned_g);
    [-g[0], -g[1], -g[2]]
}

/// `ImuState::quat`, trunk → world, scalar-first, from DJI's yaw/pitch/roll (ZYX, degrees, NED):
/// the body→NED quaternion, then both frames turned by the half turn about x, which negates the
/// y and z parts.
pub fn trunk_quat(att: &s1_duml::dds::Attitude) -> [f64; 4] {
    let (y, p, r) = (
        f64::from(att.yaw).to_radians() / 2.0,
        f64::from(att.pitch).to_radians() / 2.0,
        f64::from(att.roll).to_radians() / 2.0,
    );
    let (cy, sy, cp, sp, cr, sr) = (y.cos(), y.sin(), p.cos(), p.sin(), r.cos(), r.sin());
    let w = cr * cp * cy + sr * sp * sy;
    let x = sr * cp * cy - cr * sp * sy;
    let yq = cr * sp * cy + sr * cp * sy;
    let z = cr * cp * sy - sr * sp * cy;
    [w, x, -yq, -z]
}

/// What the motion controller last told us, decoded from its DDS pushes. `None` fields are
/// subjects that have not arrived yet — which is how a subscription the controller refused
/// shows up: as nothing, not as zeros.
#[derive(Debug, Clone, Default)]
pub struct Telemetry {
    pub battery: Option<s1_duml::dds::Battery>,
    pub attitude: Option<s1_duml::dds::Attitude>,
    pub position: Option<s1_duml::dds::Position>,
    pub velocity: Option<s1_duml::dds::Velocity>,
    pub imu: Option<s1_duml::dds::Imu>,
    pub esc: Option<s1_duml::dds::Esc>,
    pub sa_status: Option<s1_duml::dds::SaStatus>,
    pub gimbal_pos: Option<s1_duml::dds::GimbalPos>,
    pub stick: Option<s1_duml::dds::Stick>,
    /// The gimbal's own attitude push (`0x04/0x05`), the real angle whenever the gimbal runs.
    pub gimbal: Option<s1_duml::events::GimbalPush>,
    /// Pushes decoded so far — ours and the stock's alike — so health can say telemetry flows.
    pub pushes: u64,
    /// Of those, the stock firmware's own subscription, recognised by its 161-byte layout
    /// (`dds::StockPush`): the controller keeps pushing it after `dji_hdvt_uav` is stopped
    /// (measured 2026-10-30), and it is what this daemon reads while its own subscription is
    /// refused.
    pub stock_pushes: u64,
    /// Pushes nobody here decodes: the stock's 1 Hz subject, a truncation.
    pub foreign_pushes: u64,
    /// The last acknowledgement a module sent to one of our one-shots: `(cmd_set, cmd_id, retcode)`,
    /// the retcode being the first data byte DJI's `unpack_resp` reads. Kept so a one-shot's answer
    /// can be read; whether the gimbal takes `sleep` has no valid measurement yet.
    pub last_ack: Option<(u8, u8, Option<u8>)>,
    pub acks: u64,
}

pub struct State {
    pub started: Instant,
    pub period_us: u64,
    pub ticks: AtomicU64,
    pub missed: AtomicU64,
    pub last_tick_us: AtomicU64,
    /// Achieved rate over the last window, as `f64::to_bits`; 0 until the first window closes.
    pub achieved_hz: AtomicU64,
    pub consecutive_write_errors: AtomicUsize,
    pub write_errors_total: AtomicU64,
    pub decoder_dropped: AtomicU64,
    pub armed: AtomicBool,
    /// The wheels-stop rule's verdict (`tick::LiftDetector`): lifted, so the wheels are held at
    /// zero and `robot.state.safety.picked_up` says so beside DJI's own bit.
    pub lifted: AtomicBool,
    /// The last twist the loop applied, `[vx, vy, vyaw]`, for `robot.state`.
    pub applied_twist: ArcSwap<[f64; 3]>,
    pub gimbal_ddeg: ArcSwap<(i16, i16)>,
    pub telemetry: ArcSwap<Telemetry>,
    pub armor_hits: AtomicU64,
    pub ir_hits: AtomicU64,
    pub button_presses: AtomicU64,
    pub state_tx: tokio::sync::broadcast::Sender<proto::RobotState>,
    /// Armour hits, infrared hits and button presses, as `s1.event` notifications to subscribers.
    pub event_tx: tokio::sync::broadcast::Sender<crate::api::Event>,
    pub shutdown: AtomicBool,
    /// Whether the last `robot.flashlight` left the LEDs on, for `toggle`.
    pub flashlight_on: AtomicBool,
    /// The speaker, when `main` gave this daemon a sound bank; `None` on a fake body.
    pub sound: std::sync::Mutex<Option<crate::sound::Sound>>,
    /// `--blaster-gel`: the owner's key for gel beads; without it every shot is infrared.
    pub blaster_gel_allowed: AtomicBool,
    /// `--volume-control`: the mixer control `s1.volume` sets, once the bench names it.
    pub volume_control: std::sync::Mutex<Option<String>>,
}

/// Consecutive write failures that make the body unhealthy. One is a hiccup; this many in a row
/// at 100 Hz is half a second of a port that does not answer.
pub const MAX_CONSECUTIVE_WRITE_ERRORS: usize = 50;
/// Periods without a stamp before the loop counts as wedged.
pub const STALL_PERIODS: u64 = 25;
/// Fraction of the target rate below which the loop is unhealthy. `robotd`'s gate is 45 of 50.
pub const MIN_RATE_FRACTION: f64 = 0.9;

impl State {
    pub fn new(hz: u32) -> Self {
        let (state_tx, _) = tokio::sync::broadcast::channel(16);
        let (event_tx, _) = tokio::sync::broadcast::channel(64);
        Self {
            started: Instant::now(),
            period_us: 1_000_000 / u64::from(hz.max(1)),
            ticks: AtomicU64::new(0),
            missed: AtomicU64::new(0),
            last_tick_us: AtomicU64::new(0),
            achieved_hz: AtomicU64::new(0),
            consecutive_write_errors: AtomicUsize::new(0),
            write_errors_total: AtomicU64::new(0),
            decoder_dropped: AtomicU64::new(0),
            armed: AtomicBool::new(false),
            lifted: AtomicBool::new(false),
            applied_twist: ArcSwap::from_pointee([0.0; 3]),
            gimbal_ddeg: ArcSwap::from_pointee((0, 0)),
            telemetry: ArcSwap::from_pointee(Telemetry::default()),
            armor_hits: AtomicU64::new(0),
            ir_hits: AtomicU64::new(0),
            button_presses: AtomicU64::new(0),
            state_tx,
            event_tx,
            shutdown: AtomicBool::new(false),
            flashlight_on: AtomicBool::new(false),
            sound: std::sync::Mutex::new(None),
            blaster_gel_allowed: AtomicBool::new(false),
            volume_control: std::sync::Mutex::new(None),
        }
    }

    /// Everything the controller pushed, in the S1 list's shape.
    pub fn telemetry_result(&self) -> crate::api::TelemetryResult {
        let t = self.telemetry.load();
        crate::api::TelemetryResult {
            pushes: t.pushes,
            stock_pushes: t.stock_pushes,
            foreign_pushes: t.foreign_pushes,
            wheels_rpm: t.esc.map(|e| e.speed_rpm),
            position_m: t.position.map(|p| [p.x, p.y, p.z]),
            velocity_world: t.velocity.map(|v| v.world),
            velocity_body: t.velocity.map(|v| v.body),
            acc: t.imu.map(|i| i.acc),
            gyro: t.imu.map(|i| i.gyro),
            attitude_deg: t.attitude.map(|a| [a.yaw, a.pitch, a.roll]),
            status: t.sa_status.map(|s| crate::api::StatusWord {
                static_flag: s.static_flag,
                up_hill: s.up_hill,
                down_hill: s.down_hill,
                on_slope: s.on_slope,
                picked_up: s.is_pick_up,
                slip: s.slip,
                impact_x: s.impact_x,
                impact_y: s.impact_y,
                impact_z: s.impact_z,
                roll_over: s.roll_over,
            }),
            // The gimbal's own push is the angle that is actually on the wire; the DDS subject
            // only arrives under a subscription of ours, which the controller has refused so far.
            gimbal_deg: t
                .gimbal
                .map(|g| [g.yaw_deg() as f32, g.pitch_deg() as f32])
                .or_else(|| {
                    t.gimbal_pos
                        .map(|g| [f32::from(g.yaw_ddeg) / 10.0, f32::from(g.pitch_ddeg) / 10.0])
                }),
            gimbal_yaw_vs_chassis_deg: t.gimbal.map(|g| f32::from(g.yaw_vs_chassis_ddeg) / 10.0),
            gimbal_quat: t.gimbal.map(|g| g.quat),
            sticks: t.stick.map(|s| [s.chassis, s.gimbal]),
            battery: t.battery.map(|b| crate::api::BatteryResult {
                percent: b.percent,
                adc_value: b.adc_value,
                temperature: b.temperature,
                current_ma: b.current,
            }),
            acks: t.acks,
            last_ack: t
                .last_ack
                .map(|(cmd_set, cmd_id, retcode)| crate::api::AckResult {
                    cmd_set,
                    cmd_id,
                    retcode,
                }),
        }
    }

    pub fn now_us(&self) -> u64 {
        self.started.elapsed().as_micros() as u64
    }

    pub fn target_hz(&self) -> f64 {
        1_000_000.0 / self.period_us as f64
    }

    pub fn loop_health(&self) -> proto::LoopHealth {
        let hz = f64::from_bits(self.achieved_hz.load(Ordering::Relaxed));
        proto::LoopHealth {
            target_hz: self.target_hz(),
            achieved_hz: (hz > 0.0).then_some(hz),
            ticks: self.ticks.load(Ordering::Relaxed),
            missed: self.missed.load(Ordering::Relaxed),
            last_tick_age_ms: self
                .now_us()
                .saturating_sub(self.last_tick_us.load(Ordering::Relaxed))
                / 1000,
        }
    }

    /// The verdict `updaterd`'s gate reads, plus the description a person reads. The battery is
    /// the controller's own reading (millivolts and percent, `dds::Battery`), when a push has
    /// carried one — reported, never judged, as `robotd` reports its pack.
    pub fn health(&self) -> proto::HealthResult {
        let battery = self.telemetry.load().battery.map(|b| proto::Battery {
            volts: f64::from(b.adc_value) / 1000.0,
            percent: f64::from(b.percent),
        });
        let describe =
            |healthy: bool, degraded: bool, reason: Option<String>| proto::HealthResult {
                healthy,
                degraded,
                reason,
                battery,
                control_loop: Some(self.loop_health()),
                bus: proto::BusHealth {
                    consecutive_errors: self.consecutive_write_errors.load(Ordering::Relaxed)
                        as u32,
                    startup_failures: 0,
                    missing: Vec::new(),
                },
                ..Default::default()
            };
        if self.ticks.load(Ordering::Relaxed) == 0 {
            return describe(
                false,
                false,
                Some("control loop has not completed a cycle yet".to_owned()),
            );
        }
        let errors = self.consecutive_write_errors.load(Ordering::Relaxed);
        if errors >= MAX_CONSECUTIVE_WRITE_ERRORS {
            return describe(
                false,
                false,
                Some(format!("{errors} consecutive serial write failures")),
            );
        }
        let stale_us = self
            .now_us()
            .saturating_sub(self.last_tick_us.load(Ordering::Relaxed));
        if stale_us > self.period_us.saturating_mul(STALL_PERIODS) {
            return describe(
                false,
                false,
                Some(format!("control loop stalled for {} ms", stale_us / 1000)),
            );
        }
        let hz = f64::from_bits(self.achieved_hz.load(Ordering::Relaxed));
        let floor = self.target_hz() * MIN_RATE_FRACTION;
        if hz > 0.0 && hz < floor {
            return describe(
                false,
                false,
                Some(format!(
                    "control loop at {hz:.1} Hz, below the {floor:.1} Hz floor"
                )),
            );
        }
        describe(true, false, None)
    }

    /// The `robot.state` frame for this tick. Joints and targets are empty: this body has no
    /// joint vector in the duck's sense, and every client reads them with `get`. The gimbal travels
    /// in `head` as `[0, pitch, yaw, 0]`, the order `HeadParams` uses and the duck's signs (left
    /// and down positive; the gimbal's own are the opposite, `tick::gimbal_ddeg`) — the angle the
    /// gimbal itself pushes (`0x04/0x05`) when it flows, else the DDS subject, else the last
    /// commanded one. The duck's head is relative to its body, so the gimbal's yaw **relative to
    /// the chassis** is what travels here (owner, 2026-10-30: the body following the gimbal in the
    /// app's mode must not read as the head turning); the ground-frame yaw stays in `s1.telemetry`.
    pub fn frame(&self, requested: [f64; 3], limited_by: Vec<String>) -> proto::RobotState {
        let t = self.telemetry.load();
        let (yaw_ddeg, pitch_ddeg) = match (t.gimbal, t.gimbal_pos) {
            (Some(g), _) => (g.yaw_vs_chassis_ddeg, g.pitch_ddeg),
            (None, Some(g)) => (g.yaw_ddeg, g.pitch_ddeg),
            (None, None) => **self.gimbal_ddeg.load(),
        };
        let hz = f64::from_bits(self.achieved_hz.load(Ordering::Relaxed));
        // DJI's world and body are NED; the duck's are z-up — `ned_to_trunk` on every vector, the
        // heading negated (DJI clockwise positive, the duck's `vyaw` counter-clockwise).
        let odom = t
            .position
            .map(|p| proto::OdomState {
                position: ned_to_trunk([p.x, p.y, p.z]),
                yaw: t
                    .attitude
                    .map(|a| -f64::from(a.yaw).to_radians())
                    .unwrap_or(0.0),
            })
            .unwrap_or_default();
        let imu = t.imu.map(|i| proto::ImuState {
            gyro: ned_to_trunk(i.gyro),
            // The chassis attitude as the controller states it, as a trunk→world quaternion;
            // identity only until the first attitude arrives (the same push carries both).
            quat: t
                .attitude
                .map(|a| trunk_quat(&a))
                .unwrap_or([1.0, 0.0, 0.0, 0.0]),
        });
        let gravity = t
            .imu
            .map(|i| gravity_in_trunk(i.acc))
            .unwrap_or([0.0, 0.0, -1.0]);
        let armed = self.armed.load(Ordering::Relaxed);
        proto::RobotState {
            t: self.started.elapsed().as_secs_f64(),
            movement: proto::MoveState {
                requested,
                applied: **self.applied_twist.load(),
                limited_by,
            },
            head: crate::tick::head_from_gimbal(yaw_ddeg, pitch_ddeg),
            policy: if armed {
                "roller".to_owned()
            } else {
                "held".to_owned()
            },
            safety: proto::SafetyState {
                fallen: t.sa_status.map(|s| s.roll_over).unwrap_or(false),
                limp: !armed,
                gravity,
                gain: None,
                // DJI's own bit (never seen set, 2026-10-30) or the wheels-stop rule's verdict.
                picked_up: t.sa_status.map(|s| s.is_pick_up).unwrap_or(false)
                    || self.lifted.load(Ordering::Relaxed),
            },
            control_loop: proto::LoopState {
                hz,
                missed: self.missed.load(Ordering::Relaxed),
            },
            joints: Vec::new(),
            targets: Vec::new(),
            velocities: Vec::new(),
            currents_ma: Vec::new(),
            odom,
            theremin: None,
            chorale: None,
            t_ns: proto::clock::monotonic_ns(),
            imu,
            frames: None,
            skeleton: Vec::new(),
        }
    }
}

#[cfg(test)]
mod frame_tests {
    use super::*;
    use s1_duml::dds::Attitude;

    /// `q v q*` for a unit quaternion, scalar-first.
    fn rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
        let (w, x, y, z) = (q[0], q[1], q[2], q[3]);
        let (vx, vy, vz) = (v[0], v[1], v[2]);
        // t = 2 (q_v × v); v' = v + w t + q_v × t
        let tx = 2.0 * (y * vz - z * vy);
        let ty = 2.0 * (z * vx - x * vz);
        let tz = 2.0 * (x * vy - y * vx);
        [
            vx + w * tx + (y * tz - z * ty),
            vy + w * ty + (z * tx - x * tz),
            vz + w * tz + (x * ty - y * tx),
        ]
    }

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b).all(|(p, q)| (p - q).abs() < 0.02)
    }

    /// At rest the controller reads `[0, 0, −1]` g (every push of 2026-10-30): gravity pulls down
    /// the trunk's z, which is what `SafetyState::gravity` calls upright.
    #[test]
    fn at_rest_gravity_is_minus_z_and_the_quaternion_is_the_heading() {
        assert!(close(gravity_in_trunk([0.0, 0.0, -1.0]), [0.0, 0.0, -1.0]));
        // DJI yaw +90 is clockwise: the robot faces right, which in the duck's y-left world is −y.
        let q = trunk_quat(&Attitude {
            yaw: 90.0,
            pitch: 0.0,
            roll: 0.0,
        });
        assert!(close(rotate(q, [1.0, 0.0, 0.0]), [0.0, -1.0, 0.0]));
        let q = trunk_quat(&Attitude {
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
        });
        assert_eq!(q, [1.0, 0.0, 0.0, 0.0]);
    }

    /// The tilt window of 2026-10-30 (`telemetry-free/tilt.ndjson`, t = 15.2 s): chassis pitch
    /// +21.7°, roll −1.5°, yaw 24.8°, accelerometer `[0.372, 0.015, −0.935]`. Nose up: gravity in
    /// the trunk points back and down, and the trunk's forward axis rises in the world.
    #[test]
    fn nose_up_reads_as_gravity_behind_and_a_forward_axis_that_climbs() {
        let g = gravity_in_trunk([0.372, 0.015, -0.935]);
        // y: 0.015 g to DJI's right is gravity pulling slightly left, the duck's +y
        assert!(close(g, [-0.372, 0.015, -0.935]), "{g:?}");
        let q = trunk_quat(&Attitude {
            yaw: 24.8,
            pitch: 21.7,
            roll: -1.5,
        });
        let fwd = rotate(q, [1.0, 0.0, 0.0]);
        assert!(
            (fwd[2] - 21.7f64.to_radians().sin()).abs() < 0.02,
            "climbs: {fwd:?}"
        );
        // and the heading is the duck's: counter-clockwise positive, so DJI's 24.8 is −24.8
        assert!(
            (fwd[1].atan2(fwd[0]) + 24.8f64.to_radians()).abs() < 0.02,
            "{fwd:?}"
        );
        let norm: f64 = q.iter().map(|c| c * c).sum::<f64>().sqrt();
        assert!((norm - 1.0).abs() < 1e-9);
    }

    /// Nose down the other way, as the owner tilted first (pitch −17.4°, acc_x −0.318).
    #[test]
    fn nose_down_reads_the_opposite() {
        let g = gravity_in_trunk([-0.32, 0.005, -0.95]);
        assert!(g[0] > 0.3 && g[2] < -0.9, "{g:?}");
        let q = trunk_quat(&Attitude {
            yaw: 0.0,
            pitch: -17.4,
            roll: 0.0,
        });
        assert!(rotate(q, [1.0, 0.0, 0.0])[2] < -0.29);
    }
}
