//! The control loop: one tick every period, never waiting on a client.
//!
//! ```text
//! snapshot intents → arm/disarm? → heartbeat → wheels (deadman) → gimbal → LED → read body →
//! decode → telemetry/events → publish state → stamp
//! ```
//!
//! **The heartbeat is the chassis's permission to move.** Without it streamed continuously the
//! wheels ignore every command (bench, 2026-10-29). The stock firmware sends it at 91.5 Hz; this,
//! at the loop's rate. Stopping it is how the chassis is disarmed, which is also what happens if this loop dies.
//!
//! **The deadman is on the twist, not the head.** A stale velocity drives the robot into a wall; a
//! stale gimbal angle holds a pose. `duck_control::safety::Config::deadman` is 500 ms and this
//! copies the number.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use s1_duml::cmd::{self, SeqCounter};
use s1_duml::dds;
use s1_duml::events::{ArmorHit, GimbalPush, IrHit};
use s1_duml::{Decoder, Frame};

use crate::body::S1Body;
use crate::intents::{FollowMode, Intents, PowerRequest};
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
    /// The DDS node this daemon registers and subscribes under. `DDS_NODE_SCRIPT` by default.
    pub dds_node: u8,
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
            dds_node: DDS_NODE_SCRIPT,
        }
    }
}

/// The node id DJI's on-board Python runtime registers for its own subscriptions
/// (`rm_module.py`: `(9 & 0x1f) | ((6 << 5) & 0xe0)`), beside the stock controller's node 9.
pub const DDS_NODE_SCRIPT: u8 = 0xC9;

/// The SDK's documented wheel range, rpm.
pub const WHEEL_RPM_MAX: f64 = 1000.0;
/// Gimbal limits from the manual, tenths of a degree.
pub const YAW_DDEG_MAX: i16 = 2500;
pub const PITCH_DDEG_MIN: i16 = -200;
pub const PITCH_DDEG_MAX: i16 = 350;
/// Our own subscription, which the controller refuses, re-asked only while nothing flows.
/// Our subscription's msg id. **Not 1**: that is the stock firmware's, and its pushes outlive
/// `dji_hdvt_uav` — measured 2026-10-30, the controller kept pushing msg id 1 after the stock was
/// stopped, and `s1d` decoded its subjects with our layout (gyro in the 10³² range). Any other
/// value avoids that collision; `DDS_PUSH_LEN` is what makes a collision harmless.
pub const DDS_MSG_ID: u8 = 0x5d;
/// The subscription's flags, proroklab's `0x3`: a time stamp in front of every push and stop
/// pushing when the subscriber goes away. Without them (flags 0, 2026-10-30) the motion controller
/// neither answered the subscription nor pushed under our id.
pub const DDS_TIMESTAMP: bool = true;
pub const DDS_STOP_WHEN_DISCONNECT: bool = true;
/// Bytes in one push of ours: the time stamp, then the nine of `SUBJECTS` concatenated, in the
/// order DJI's own library asks for them and slices the push (`RobotCtrl.init` and
/// `gimbal_and_chassis_push_info_process`, pulled from the robot). A push under our id with any
/// other length is not ours.
pub const DDS_PUSH_LEN: usize = dds::PUSH_TIMESTAMP_LEN
    + dds::Esc::SIZE
    + dds::Position::SIZE
    + dds::Velocity::SIZE
    + dds::Imu::SIZE
    + dds::Attitude::SIZE
    + dds::SaStatus::SIZE
    + dds::GimbalPos::SIZE
    + dds::Stick::SIZE
    + dds::Battery::SIZE;
const DDS_RETRY: Duration = Duration::from_secs(5);
/// The gimbal command is an action; it is re-sent at this rate while the target is unchanged, and
/// at once when it changes. The PoC sent it at 20 Hz.
const GIMBAL_HOLD_PERIOD: Duration = Duration::from_millis(100);
/// The ctrl byte of the gimbal rate frame: DJI's `accel_contral_flag` (`rm_module.py` `Gimbal`,
/// `0xdc`; bit 6 is cleared only when the sticks overlay).
const GIMBAL_RATE_CTRL: u8 = 0xdc;
/// The launcher's own cadence for a fire command of `n` shots: measured 2026-10-30 on the robot,
/// `blaster --times 3` against a white wall gave three pairs of infrared-hit events 330 ms apart
/// (`out/2026-10-30-poc2/items/F17_infrared_fire3.log`). The muzzle light and the shot sound
/// follow that cadence, one per shot, as the stock app shows them.
pub const SHOT_PERIOD: Duration = Duration::from_millis(330);
/// The sound bank's name for the shot: the stock's own `ir_gun_shoot.wav` (0.45 s), which the
/// stock plays through the speaker on every shot (`s1-push.sh` links it under this name).
pub const SHOT_SOUND: &str = "shoot";
/// Subjects we subscribe to, in the order they come back concatenated — DJI's own order, minus
/// the servo subject an S1 has no servos for.
pub const SUBJECTS: [u64; 9] = [
    dds::uid::ESC,
    dds::uid::POSITION,
    dds::uid::VELOCITY,
    dds::uid::IMU,
    dds::uid::ATTITUDE,
    dds::uid::SA_STATUS,
    dds::uid::GIMBAL_POS,
    dds::uid::STICK,
    dds::uid::BATTERY,
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
///
/// **Both signs flip.** The duck's frame is `y` left, `z` up: positive yaw turns left
/// (`MoveParams::vyaw`'s own words) and positive pitch noses down. DJI's gimbal is the other way
/// — its library sends `gimbal_right` as a positive yaw and `gimbal_up` as a positive pitch
/// (`rm_ctrl.py`, `rotate_with_degree`), and the bench saw it: the pad's "look right and up"
/// arrived as yaw −143°, pitch −20° and the gimbal looked **left and down** (2026-10-30).
pub fn gimbal_ddeg(head: [f64; 4]) -> (i16, i16) {
    let yaw = (-head[2].to_degrees() * 10.0)
        .round()
        .clamp(-f64::from(YAW_DDEG_MAX), f64::from(YAW_DDEG_MAX)) as i16;
    let pitch = (-head[1].to_degrees() * 10.0)
        .round()
        .clamp(f64::from(PITCH_DDEG_MIN), f64::from(PITCH_DDEG_MAX)) as i16;
    (yaw, pitch)
}

/// `robot.look`: a trunk-frame point to the head joints the gimbal can reach. The duck's frame is
/// `x` forward, `y` left, `z` up, and `HeadParams` turns left with a positive yaw and noses down
/// with a positive pitch (`MoveParams::vyaw`'s own words; `gimbal_ddeg` flips both to DJI's). So a
/// point to the left is `atan2(y, x)`, and a point below is a positive pitch, `−atan2(z, ‖xy‖)`.
/// A two-axis gimbal has no neck and no roll: `neck_pitch` is accepted and ignored. The manual's
/// travel clamps the gaze, and `clamped` says so — the contract's "closest gaze, not a lock". The
/// origin itself has no direction and looks straight ahead.
pub fn gaze_head(look: &duck_ipc_proto::LookParams) -> (duck_ipc_proto::HeadParams, bool) {
    let flat = look.x.hypot(look.y);
    let (yaw, pitch) = if flat == 0.0 && look.z == 0.0 {
        (0.0, 0.0)
    } else {
        (look.y.atan2(look.x), -look.z.atan2(flat))
    };
    let yaw_max = f64::from(YAW_DDEG_MAX) / 10.0;
    // DJI's pitch is up-positive: the duck's down-positive pitch runs from −max up to −min down.
    let (pitch_min, pitch_max) = (
        -f64::from(PITCH_DDEG_MAX) / 10.0,
        -f64::from(PITCH_DDEG_MIN) / 10.0,
    );
    let yaw_deg = yaw.to_degrees().clamp(-yaw_max, yaw_max);
    let pitch_deg = pitch.to_degrees().clamp(pitch_min, pitch_max);
    let clamped = yaw_deg != yaw.to_degrees() || pitch_deg != pitch.to_degrees();
    (
        duck_ipc_proto::HeadParams {
            neck_pitch: 0.0,
            head_pitch: pitch_deg.to_radians(),
            head_yaw: yaw_deg.to_radians(),
            head_roll: 0.0,
        },
        clamped,
    )
}

/// The inverse of [`gimbal_ddeg`]: the gimbal's own angle (DJI's signs, tenths) back to the
/// duck's `head` radians, for `robot.state`.
pub fn head_from_gimbal(yaw_ddeg: i16, pitch_ddeg: i16) -> [f64; 4] {
    [
        0.0,
        (-f64::from(pitch_ddeg) / 10.0).to_radians(),
        (-f64::from(yaw_ddeg) / 10.0).to_radians(),
        0.0,
    ]
}

/// The wheels-stop rule's thresholds. **They have no valid origin:** the two levels come from the
/// floor session of 2026-10-31, which the owner discarded (`out/2026-10-31-poc2/floor/`), and the
/// release level and both times are design choices, not measurements. The tests prove the
/// mechanism, not these numbers; measure again before relying on them. What does stand: lifted,
/// the wheels stop (owner's rule, 2026-10-30); DJI's `pick_up` bit never set in the captures of
/// 2026-10-30, and the fused velocity climbed with the wheels stopped while the robot was carried
/// (`telemetry-free/pickup_wheels.ndjson`), which is why the signal is its disagreement with the
/// velocity the four wheel speeds imply. `docs/s1/robot/s1-commands.md` lists this as a defect.
pub const LIFT_TRIP_M_S: f64 = 0.4;
pub const LIFT_TRIP_FOR: Duration = Duration::from_millis(300);
pub const LIFT_RELEASE_M_S: f64 = 0.15;
pub const LIFT_RELEASE_FOR: Duration = Duration::from_millis(500);

/// The body speed the four ESC speeds imply, metres per second — the forward mecanum kinematics,
/// the inverse of [`mecanum_rpm`] with the wire's sign convention (the left wheels read negative
/// when the robot drives forward: `[27, −26, −33, 31]` rpm, a sample of the discarded floor session,
/// reads as 0.153 m/s). Order as the ESC subject and the wire have it: front-right,
/// front-left, back-left, back-right.
pub fn wheel_speed_m_s(esc_rpm: [i16; 4], radius_m: f64) -> f64 {
    let [fr, fl_w, bl_w, br] = esc_rpm.map(f64::from);
    let (fl, bl) = (-fl_w, -bl_w);
    let to_m_s = |rpm: f64| rpm / 60.0 * std::f64::consts::TAU * radius_m;
    let vx = to_m_s((fl + fr + bl + br) / 4.0);
    let vy = to_m_s((fr + bl - fl - br) / 4.0);
    vx.hypot(vy)
}

/// Lifted or not, from the disagreement between the fused velocity and the wheels, with the
/// persistence the thresholds above ask for: over the trip level for `LIFT_TRIP_FOR` trips, under
/// the release level for `LIFT_RELEASE_FOR` releases; anything in between keeps the state.
#[derive(Debug, Default)]
pub struct LiftDetector {
    over_since: Option<Instant>,
    under_since: Option<Instant>,
    lifted: bool,
}

impl LiftDetector {
    pub fn lifted(&self) -> bool {
        self.lifted
    }

    /// Feed one disagreement, metres per second, at `now`; returns the state after it.
    pub fn update(&mut self, disagreement_m_s: f64, now: Instant) -> bool {
        if disagreement_m_s > LIFT_TRIP_M_S {
            self.under_since = None;
            let since = *self.over_since.get_or_insert(now);
            if now.duration_since(since) >= LIFT_TRIP_FOR {
                self.lifted = true;
            }
        } else if disagreement_m_s < LIFT_RELEASE_M_S {
            self.over_since = None;
            let since = *self.under_since.get_or_insert(now);
            if now.duration_since(since) >= LIFT_RELEASE_FOR {
                self.lifted = false;
            }
        } else {
            self.over_since = None;
            self.under_since = None;
        }
        self.lifted
    }
}

/// Whose push a `0x48/0x08` was, once decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    /// `DDS_MSG_ID` and exactly `DDS_PUSH_LEN` bytes, in `SUBJECTS` order.
    Ours,
    /// The stock firmware's boot-time subscription, `dds::StockPush::LEN` bytes under whatever
    /// msg id that boot gave it (0 or 1, measured both ways on 2026-10-30).
    Stock,
    /// Nobody's here: the stock's 1 Hz subject, a truncation. Telemetry keeps its last values.
    Foreign,
}

/// Decode a push into telemetry. Ours by id and length; the stock's by length alone — it is the
/// telemetry that actually flows on this robot, our own subscription being refused by the
/// controller (`02 03` / `0f …`, 2026-10-30) — and it carries every subject but the gimbal's and
/// the sticks', which `0x04/0x05` supplies.
fn decode_push(msg_id: u8, data: &[u8], t: &mut Telemetry) -> Push {
    if let Some(p) = dds::StockPush::parse(data) {
        // The ESC and battery block (46..92) comes in either of two subject orders, read off
        // the bytes by which order makes both subjects physics (`StockPush::block_order`,
        // 2026-10-30: 2363 pushes one way, 3080 the other). When neither does — a push of zeros,
        // a layout this firmware has not shown yet — neither subject is published: a health that
        // says the pack is empty is worse than none.
        let block_read = p.block_order.is_some();
        t.esc = block_read.then_some(p.esc);
        t.position = Some(p.position);
        t.velocity = Some(p.velocity);
        t.imu = Some(p.imu);
        t.attitude = Some(p.attitude);
        t.sa_status = Some(p.sa_status);
        t.battery = block_read.then_some(p.battery);
        return Push::Stock;
    }
    if msg_id != DDS_MSG_ID || data.len() != DDS_PUSH_LEN {
        return Push::Foreign;
    }
    // The length was checked, so every parse below has its bytes.
    let subjects = &data[dds::PUSH_TIMESTAMP_LEN..];
    let (esc, rest) = dds::Esc::parse(subjects).expect("length checked");
    let (position, rest) = dds::Position::parse(rest).expect("length checked");
    let (velocity, rest) = dds::Velocity::parse(rest).expect("length checked");
    let (imu, rest) = dds::Imu::parse(rest).expect("length checked");
    let (attitude, rest) = dds::Attitude::parse(rest).expect("length checked");
    let (sa_status, rest) = dds::SaStatus::parse(rest).expect("length checked");
    let (gimbal_pos, rest) = dds::GimbalPos::parse(rest).expect("length checked");
    let (stick, rest) = dds::Stick::parse(rest).expect("length checked");
    let (battery, _) = dds::Battery::parse(rest).expect("length checked");
    t.esc = Some(esc);
    t.position = Some(position);
    t.velocity = Some(velocity);
    t.imu = Some(imu);
    t.attitude = Some(attitude);
    t.sa_status = Some(sa_status);
    t.gimbal_pos = Some(gimbal_pos);
    t.stick = Some(stick);
    t.battery = Some(battery);
    Push::Ours
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
    let mut rate_was_on = false;
    let mut lift = LiftDetector::default();
    let mut last_sub: Option<Instant> = None;
    let mut last_push: Option<Instant> = None;
    // The launcher's shots still to be lit and sounded.
    let mut shots: std::collections::VecDeque<Instant> = std::collections::VecDeque::new();
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
                // The gimbal freed from the chassis, explicitly (`0x04/0x4c 00 00`, DJI's
                // `gimbal_free_mode`): the duck's head is independent of its body, and with the
                // stock stopped the controller keeps whatever mode the app left — the body
                // followed the gimbal through the whole session of 2026-10-30 (owner's finding).
                // Following is a behaviour asked for through `s1.follow`, never inherited.
                let s = w.seq(cmd::id::GIMBAL_WORKMODE);
                w.send(cmd::gimbal_workmode(s, 0));
                state.armed.store(true, Ordering::Relaxed);
                tracing::info!("armed: heartbeat on, chassis in work mode 1, gimbal free");
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

            let mut twist = if snap.twist_age > cfg.deadman {
                if snap.twist != [0.0; 3] {
                    limited_by.push("deadman".to_owned());
                }
                [0.0; 3]
            } else {
                snap.twist
            };
            // Lifted: the wheels stop — a zero wheel frame every tick, whatever was asked, the
            // heartbeat on; the gimbal, the sounds, the lights and the pushes go on (owner's rule).
            let lifted = lift.lifted();
            if lifted {
                if twist != [0.0; 3] || snap.wheels.is_some_and(|w| w != [0; 4]) {
                    limited_by.push("lifted".to_owned());
                }
                twist = [0.0; 3];
            }
            // `s1.wheels` asked for rpm directly and more recently than any twist: send those,
            // under the same deadman.
            let direct = match snap.wheels {
                Some(rpm)
                    if !lifted
                        && snap.wheels_age <= snap.twist_age
                        && snap.wheels_age <= cfg.deadman =>
                {
                    Some(rpm)
                }
                _ => None,
            };
            // A position task of the controller (`s1.goto`) owns the wheels: no wheel or follow
            // frame while it runs, or the task would be overwritten every tick (read here, no valid proof).
            let twist = if snap.position_task && !lifted {
                [0.0; 3]
            } else {
                twist
            };
            match (direct, snap.follow, cfg.twist_mode) {
                _ if snap.position_task && !lifted => {}
                // Lifted: zero wheel speeds, the controller's own task included.
                _ if lifted => {
                    let s = w.seq(cmd::id::CHASSIS_WHEEL_SPEED);
                    w.send(cmd::chassis_wheel_speed(s, 0, 0, 0, 0));
                }
                (Some([fr, fl, bl, br]), _, _) => {
                    let s = w.seq(cmd::id::CHASSIS_WHEEL_SPEED);
                    w.send(cmd::chassis_wheel_speed(s, fr, fl, bl, br));
                }
                // Gimbal lead: the controller steers the chassis after the gimbal; the twist's
                // yaw rate has no say and goes as a zero offset (`Chassis.set_follow_speed`).
                (None, FollowMode::GimbalLead, _) => {
                    let s = w.seq(cmd::id::CHASSIS_FOLLOW);
                    w.send(cmd::chassis_follow(
                        s,
                        twist[0] as f32,
                        twist[1] as f32,
                        0.0,
                    ));
                }
                (None, _, TwistMode::Wheels) => {
                    let [fr, fl, bl, br] =
                        mecanum_rpm(twist, cfg.wheel_radius_m, cfg.half_sum_lw_m);
                    let s = w.seq(cmd::id::CHASSIS_WHEEL_SPEED);
                    w.send(cmd::chassis_wheel_speed(s, fr, fl, bl, br));
                }
                (None, _, TwistMode::Speed) => {
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

            // The gimbal: a rate while one is fresh, else the angle target.
            //
            // The rate (`s1.gimbal speed`) goes out every tick while fresher than the deadman — the
            // one measurement of a single frame is from the discarded floor session — and the
            // angle hold is silent meanwhile, or it would pull the gimbal back every 100 ms.
            // When the rate ends (`stop`, or the deadman) one zero-rate frame goes out and the
            // head target becomes where the gimbal is, read from its own push, so the hold does
            // not snap it back to where it was before the rate.
            let rate_fresh = snap.gimbal_rate_age <= cfg.deadman;
            let rate_on = rate_fresh && snap.gimbal_rate != [0.0; 2];
            if rate_on {
                let ddeg = |v: f64| (v * 10.0).round() as i16;
                let s = w.seq(cmd::id::GIMBAL_SPEED);
                w.send(cmd::gimbal_speed(
                    s,
                    ddeg(snap.gimbal_rate[0]),
                    ddeg(snap.gimbal_rate[1]),
                    GIMBAL_RATE_CTRL,
                ));
                rate_was_on = true;
            } else {
                if rate_was_on {
                    // DJI's own stop: a zero rate with ctrl `0x00` (`rm_ctrl.py` `_stop_rotate`:
                    // `set_accel_ctrl(0, 0, 0x00)`), not the moving frame's flag.
                    let s = w.seq(cmd::id::GIMBAL_SPEED);
                    w.send(cmd::gimbal_speed(s, 0, 0, 0));
                    if let Some(g) = state.telemetry.load().gimbal {
                        intents.set_head(head_from_gimbal(g.yaw_vs_chassis_ddeg, g.pitch_ddeg));
                    }
                    rate_was_on = false;
                    last_gimbal = None;
                }
                let target = gimbal_ddeg(snap.head);
                // Where the gimbal leads, the chassis chases it and the target's yaw is relative
                // to the chassis (coordinate 3, inferred): a re-send would put the target ahead
                // of the new heading again and the body would not stop (no valid proof). There
                // the target goes out once per change, as DJI's library sends one task. Where the
                // chassis leads, the controller owns the yaw and only the pitch is ours
                // (`rm_ctrl.py` `angle_ctrl` in `gimbal_yaw_follow_mode`).
                let hold = snap.follow != FollowMode::GimbalLead;
                let due = match last_gimbal {
                    Some((prev, at)) => {
                        prev != target || (hold && at.elapsed() >= GIMBAL_HOLD_PERIOD)
                    }
                    None => true,
                };
                if due {
                    let axes = match snap.follow {
                        FollowMode::ChassisLead => cmd::GIMBAL_AXIS_PITCH,
                        _ => cmd::GIMBAL_AXES_BOTH,
                    };
                    let s = w.seq(cmd::id::GIMBAL_ROTATE);
                    w.send(cmd::gimbal_angles_axes(s, target.0, target.1, axes));
                    state.gimbal_ddeg.store(Arc::new(target));
                    last_gimbal = Some((target, Instant::now()));
                }
            }
        }

        // Telemetry is asked for armed or not: battery and pick-up matter before anything moves,
        // and a subscription moves nothing. **Never a node reset**: measured 2026-10-30 (night),
        // after one `0x48/0x02` on node 9 the controller answered every request of ours `0x0f`
        // and the stock's own re-registration (`dji_hdvt_uav` restarted under strace) got no
        // answer and no push at all — the reset leaves the controller's bus dead until a power
        // cycle. The node is registered as DJI's on-board library registers its own (`node_id`
        // `(9 & 0x1f) | (6 << 5)` = `0xC9`, `rm_module.py`), a second subscriber beside the stock's
        // node 9, then subscribed. `--dds-node` picks another id for the bench.
        if dds_due(last_sub, last_push, DDS_RETRY, Instant::now()) {
            let s = w.seq(cmd::id::DDS_ADD_NODE);
            w.send(dds::add_node(s, cfg.dds_node));
            let s = w.seq(cmd::id::DDS_ADD_SUB);
            w.send(dds::add_sub(
                s,
                cfg.dds_node,
                DDS_MSG_ID,
                &SUBJECTS,
                cfg.dds_hz,
                DDS_TIMESTAMP,
                DDS_STOP_WHEN_DISCONNECT,
            ));
            last_sub = Some(Instant::now());
        }

        if let Some(led) = intents.take_led() {
            let s = w.seq(cmd::id::SYSTEM_LED);
            w.send(cmd::system_led(s, &led));
        }
        // One-shots from the socket, each with a fresh sequence for its command.
        for mut frame in intents.take_frames() {
            frame.seq = w.seq((frame.cmd_set, frame.cmd_id));
            w.send(frame);
        }

        // A volley just asked of the launcher: one muzzle flash and one shot sound per shot, on
        // the launcher's cadence, the first in the same tick as the fire frame above. The stock
        // paces its flashes itself too: the launcher reports nothing per shot on the UART
        // (`capture/hdvt_fire.strace`).
        if let Some(n) = intents.take_volley() {
            let now = Instant::now();
            shots.clear();
            shots.extend((0..u32::from(n)).map(|k| now + SHOT_PERIOD * k));
        }
        while shots.front().is_some_and(|at| *at <= Instant::now()) {
            shots.pop_front();
            // The stock's own muzzle flash, byte for byte (`cmd::gun_fire_flash`): green, 40 ms,
            // once — it ends by itself, no "off" is sent.
            let s = w.seq(cmd::id::GUN_LED);
            w.send(cmd::gun_fire_flash(s));
            if let Ok(mut slot) = state.sound.lock()
                && let Some(sound) = slot.as_mut()
                && let Err(why) = sound.play_named(SHOT_SOUND)
            {
                tracing::debug!(why, "shot sound not played");
            }
        }

        // Whatever the body pushed: telemetry, hits, acks.
        match w.body.read_available(&mut inbox) {
            Ok(0) => {}
            Ok(n) => {
                let mut changed = false;
                for frame in decoder.push(&inbox[..n]) {
                    if let Some(push) = dds::PushPeriod::parse(&frame) {
                        let whose = decode_push(push.msg_id, push.data, &mut telemetry);
                        tracing::trace!(
                            msg_id = push.msg_id,
                            len = push.data.len(),
                            ?whose,
                            "push"
                        );
                        match whose {
                            Push::Ours => telemetry.pushes += 1,
                            Push::Stock => {
                                telemetry.pushes += 1;
                                telemetry.stock_pushes += 1;
                            }
                            Push::Foreign => telemetry.foreign_pushes += 1,
                        }
                        // Telemetry flowing, ours or the stock's, is what stops the re-ask: a
                        // controller already pushing has nothing to be asked again.
                        if whose != Push::Foreign {
                            last_push = Some(Instant::now());
                        }
                        changed = true;
                        // Answer it as the stock does, ours or not: the controller has only ever
                        // been seen pushing to a subscriber that acks every push.
                        w.send(dds::push_ack(frame.seq));
                    } else if let Some(gimbal) = GimbalPush::parse(&frame) {
                        telemetry.gimbal = Some(gimbal);
                        changed = true;
                    } else if let Some(hit) = ArmorHit::parse(&frame) {
                        state.armor_hits.fetch_add(1, Ordering::Relaxed);
                        let _ = state.event_tx.send(crate::api::Event::ArmorHit {
                            plate: hit.index,
                            hit_kind: hit.kind,
                            mic_value: hit.mic_value,
                        });
                        tracing::warn!(
                            plate = hit.index,
                            kind = hit.kind,
                            mic = hit.mic_value,
                            "armour hit"
                        );
                    } else if let Some(ir) = IrHit::parse(&frame) {
                        state.ir_hits.fetch_add(1, Ordering::Relaxed);
                        let _ = state.event_tx.send(crate::api::Event::IrHit {
                            role: ir.role_id,
                            skill: ir.skill_id,
                            device: ir.recv_dev,
                            pin: ir.recv_ir_pin,
                        });
                        tracing::warn!(
                            role = ir.role_id,
                            skill = ir.skill_id,
                            dev = ir.recv_dev,
                            pin = ir.recv_ir_pin,
                            "infrared hit"
                        );
                    } else if frame.cmd_set == cmd::id::DDS_ADD_SUB.0 {
                        // The controller's answers to our DDS requests: retcode first. Rare, and
                        // the bench reads them (2026-10-30: a silent controller was the symptom).
                        tracing::info!(
                            cmd_id = frame.cmd_id,
                            data = %hex(&frame.data),
                            meaning = dds_retcode_meaning(frame.cmd_id, frame.data.first().copied()),
                            "dds answer"
                        );
                    } else if frame.is_ack {
                        // A module answering one of our one-shots (`gimbal sleep`, a work mode, a
                        // position task…): kept in telemetry and said at `info`, retcode first, so
                        // that whether the gimbal takes `sleep` can be read; it has no valid
                        // measurement yet.
                        let retcode = frame.data.first().copied();
                        telemetry.acks += 1;
                        telemetry.last_ack = Some((frame.cmd_set, frame.cmd_id, retcode));
                        changed = true;
                        tracing::info!(
                            cmd_set = format!("{:#04x}", frame.cmd_set),
                            cmd_id = format!("{:#04x}", frame.cmd_id),
                            seq = frame.seq,
                            ?retcode,
                            data = %hex(&frame.data),
                            "ack"
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

        // The wheels-stop rule, judged every tick on the latest push: the fused body velocity
        // against what the four wheel speeds imply.
        if let (Some(esc), Some(vel)) = (telemetry.esc, telemetry.velocity) {
            let wheels = wheel_speed_m_s(esc.speed_rpm, cfg.wheel_radius_m);
            let fused = f64::from(vel.body[0]).hypot(f64::from(vel.body[1]));
            let was = lift.lifted();
            let now_lifted = lift.update((fused - wheels).abs(), Instant::now());
            state.lifted.store(now_lifted, Ordering::Relaxed);
            if now_lifted != was {
                tracing::warn!(
                    fused_m_s = format!("{fused:.2}"),
                    wheels_m_s = format!("{wheels:.2}"),
                    lifted = now_lifted,
                    "wheels-stop rule"
                );
            }
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

/// What the first byte of a DDS answer means, as far as DJI's SDK documents it: `0` is accepted
/// on every request; `0x50` only on add-node (`ProtoSubscribeAddNode.unpack_resp` takes it as
/// "already registered"; the other `unpack_resp`s accept `0` alone). Every other value is a
/// refusal the SDK only logs by number — the bench collects them (2026-10-30: `0x02` to add-node,
/// `0x0f` and `0x21` to subscribe).
pub fn dds_retcode_meaning(cmd_id: u8, retcode: Option<u8>) -> &'static str {
    match (cmd_id, retcode) {
        (_, Some(0x00)) => "accepted",
        (0x01, Some(0x50)) => "already registered (accepted by the SDK for add-node)",
        (_, Some(_)) => "refused (undocumented code)",
        (_, None) => "empty answer",
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
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

    /// The stock's push is known by its 161 bytes under any msg id and fills every subject but
    /// the gimbal's; ours by id and exact length; anything else leaves the telemetry as it was.
    #[test]
    fn a_push_is_decoded_by_whose_layout_it_has() {
        let mut t = Telemetry::default();
        let mut stock = vec![0u8; dds::StockPush::LEN];
        stock[82..84].copy_from_slice(&11_900u16.to_le_bytes()); // a pack the guard believes
        stock[82 + 8] = 80; // the battery percent where the stock's layout puts it
        assert_eq!(
            decode_push(1, &stock, &mut t),
            Push::Stock,
            "msg id 1 on one boot"
        );
        assert_eq!(
            decode_push(0, &stock, &mut t),
            Push::Stock,
            "msg id 0 on another"
        );
        assert_eq!(t.battery.map(|b| b.percent), Some(80));
        assert!(
            t.gimbal_pos.is_none() && t.stick.is_none(),
            "not in the stock's push"
        );
        assert_eq!(
            decode_push(DDS_MSG_ID, &stock[..160], &mut t),
            Push::Foreign,
            "our id, a length that is nobody's"
        );
        assert_eq!(
            decode_push(0, &[0u8; 54], &mut t),
            Push::Foreign,
            "the 1 Hz subject"
        );
        assert_eq!(t.battery.map(|b| b.percent), Some(80), "left as it was");
        let mut ours = vec![0u8; DDS_PUSH_LEN];
        ours[DDS_PUSH_LEN - dds::Battery::SIZE + 8] = 77;
        assert_eq!(decode_push(DDS_MSG_ID, &ours, &mut t), Push::Ours);
        assert_eq!(t.battery.map(|b| b.percent), Some(77));
        assert!(
            t.esc.is_some() && t.sa_status.is_some() && t.gimbal_pos.is_some(),
            "every subject lands in one push"
        );
    }

    /// The boot of 2026-10-30 22:00 put the battery before the ESC in the push; read in the
    /// other order it was 49 mV at 0 % and 10 675 rpm. The layout reads both orders, so the
    /// telemetry carries the pack (10.675 V, 29 %) and idle wheels from that push.
    #[test]
    fn the_battery_first_order_is_published_as_a_pack() {
        let frame = s1_duml::Frame::decode(include_bytes!(
            "../../s1-duml/tests/fixtures/dds_push_stock_relaunched_2026-10-30.bin"
        ))
        .expect("a whole frame");
        let push = dds::PushPeriod::parse(&frame).expect("a push");
        let mut t = Telemetry::default();
        assert_eq!(decode_push(push.msg_id, push.data, &mut t), Push::Stock);
        assert_eq!(
            t.battery.map(|b| (b.adc_value, b.percent)),
            Some((10_675, 29))
        );
        assert!(t.esc.unwrap().speed_rpm.iter().all(|r| r.abs() <= 5));
    }

    /// A block plausible in neither order — 161 zero bytes, 0 mV at 0 % — is not published as a
    /// battery or as wheel speeds: a health that says the pack is empty is worse than none.
    #[test]
    fn an_implausible_esc_or_battery_block_is_not_published() {
        let mut t = Telemetry {
            battery: Some(dds::Battery {
                adc_value: 11_900,
                temperature: 300,
                current: -1000,
                percent: 80,
                recv: 0,
            }),
            ..Telemetry::default()
        };
        assert_eq!(
            decode_push(1, &[0u8; dds::StockPush::LEN], &mut t),
            Push::Stock
        );
        assert!(t.attitude.is_some() && t.imu.is_some() && t.sa_status.is_some());
        assert_eq!(t.battery, None, "0 mV at 0 % is not a reading");
        assert_eq!(t.esc, None);
    }

    /// `0x50` is "already there" only where the SDK reads it so, on add-node.
    #[test]
    fn dds_answer_codes_are_named_per_request() {
        assert_eq!(dds_retcode_meaning(0x03, Some(0)), "accepted");
        assert!(dds_retcode_meaning(0x01, Some(0x50)).starts_with("already registered"));
        assert!(dds_retcode_meaning(0x03, Some(0x50)).starts_with("refused"));
        assert!(dds_retcode_meaning(0x01, Some(0x02)).starts_with("refused"));
        assert_eq!(dds_retcode_meaning(0x02, None), "empty answer");
    }

    /// A sample of the discarded floor session (`[27, −26, −33, 31]` rpm) reads as 0.153 m/s;
    /// a turn in place (sides opposed) has wheel speed and no body speed; a strafe is lateral.
    #[test]
    fn wheel_speed_is_the_forward_mecanum_kinematics_in_the_wires_signs() {
        let v = wheel_speed_m_s([27, -26, -33, 31], 0.05);
        assert!((v - 0.153).abs() < 0.002, "{v}");
        let turn = wheel_speed_m_s([50, 50, 50, 50], 0.05);
        assert!(
            turn.abs() < 1e-9,
            "a turn in place is no body speed: {turn}"
        );
        // The inverse of mecanum_rpm for a pure strafe, through the wire's negations.
        let [fr, fl, bl, br] = mecanum_rpm([0.0, 0.3, 0.0], 0.05, 0.2);
        let v = wheel_speed_m_s([fr, -fl, -bl, br], 0.05);
        assert!((v - 0.3).abs() < 0.01, "{v}");
    }

    /// Over 0.4 m/s for 300 ms trips; under 0.15 m/s for 500 ms releases; 0.13 m/s (a figure of the
    /// discarded floor session) never trips; a brief spike alone does not either.
    #[test]
    fn the_lift_detector_needs_persistence_both_ways() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut d = LiftDetector::default();
        for ms in (0..=1000).step_by(20) {
            assert!(
                !d.update(0.13, at(ms)),
                "the floor's worst case must not trip"
            );
        }
        assert!(
            !d.update(0.9, at(1020)),
            "a first sample over the level is not a lift"
        );
        assert!(!d.update(0.9, at(1200)));
        assert!(d.update(0.9, at(1320)), "300 ms over the level: lifted");
        assert!(
            d.update(0.3, at(1400)),
            "between the levels the state holds"
        );
        assert!(
            d.update(0.1, at(1500)),
            "a first sample under the level is not a release"
        );
        assert!(d.update(0.1, at(1900)));
        assert!(!d.update(0.1, at(2000)), "500 ms under the level: released");
        // A spike shorter than the trip time, then the floor again: never lifted.
        let mut d = LiftDetector::default();
        assert!(!d.update(1.0, at(0)));
        assert!(!d.update(1.0, at(200)));
        assert!(!d.update(0.1, at(220)));
        assert!(!d.update(1.0, at(400)));
    }

    /// The duck looks left with a positive yaw and down with a positive pitch; DJI's gimbal takes
    /// right and up as positive — so the duck's "left 15°, down 10°" is DJI's (−150, −100), and the
    /// clamps hold at the manual's travel. `head_from_gimbal` is the exact inverse.
    #[test]
    fn gimbal_is_clamped_to_the_manual_with_djis_signs() {
        assert_eq!(gimbal_ddeg([0.0, 0.0, 0.0, 0.0]), (0, 0));
        assert_eq!(
            gimbal_ddeg([0.0, 10f64.to_radians(), 15f64.to_radians(), 0.0]),
            (-150, -100)
        );
        assert_eq!(
            gimbal_ddeg([0.0, 1.0, -7.0, 0.0]),
            (YAW_DDEG_MAX, PITCH_DDEG_MIN)
        );
        assert_eq!(
            gimbal_ddeg([0.0, -1.0, 7.0, 0.0]),
            (-YAW_DDEG_MAX, PITCH_DDEG_MAX)
        );
        let back = head_from_gimbal(-150, -100);
        assert!((back[2] - 15f64.to_radians()).abs() < 1e-9);
        assert!((back[1] - 10f64.to_radians()).abs() < 1e-9);
    }
}
