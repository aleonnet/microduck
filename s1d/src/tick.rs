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
/// Our subscription's id in the motion controller, and how often it is re-asked until it answers.
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

/// Decode a push into telemetry when it is ours — `DDS_MSG_ID` and exactly `DDS_PUSH_LEN` bytes,
/// in `SUBJECTS` order — and say so. Anything else (the stock's subscription, a truncated push) is
/// left alone: the telemetry keeps its last good values.
fn decode_push(msg_id: u8, data: &[u8], t: &mut Telemetry) -> bool {
    if msg_id != DDS_MSG_ID || data.len() != DDS_PUSH_LEN {
        return false;
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
    true
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
                state.armed.store(true, Ordering::Relaxed);
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
            // `s1.wheels` asked for rpm directly and more recently than any twist: send those,
            // under the same deadman.
            let direct = match snap.wheels {
                Some(rpm)
                    if snap.wheels_age <= snap.twist_age && snap.wheels_age <= cfg.deadman =>
                {
                    Some(rpm)
                }
                _ => None,
            };
            match (direct, snap.follow, cfg.twist_mode) {
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
                        let ours = decode_push(push.msg_id, push.data, &mut telemetry);
                        tracing::trace!(msg_id = push.msg_id, len = push.data.len(), ours, "push");
                        if ours {
                            telemetry.pushes += 1;
                            last_push = Some(Instant::now());
                        } else {
                            telemetry.foreign_pushes += 1;
                        }
                        changed = true;
                        // Answer it as the stock does, ours or not: the controller has only ever
                        // been seen pushing to a subscriber that acks every push.
                        w.send(dds::push_ack(frame.seq));
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

    /// Msg id 1 is the stock's; a push of ours must be exactly the subjects we asked for. Either
    /// mismatch leaves the telemetry as it was and says so.
    #[test]
    fn a_push_is_decoded_only_under_our_id_and_length() {
        let mut t = Telemetry::default();
        let stock = vec![0u8; 161];
        assert!(!decode_push(1, &stock, &mut t), "the stock's id");
        assert!(
            !decode_push(DDS_MSG_ID, &stock, &mut t),
            "our id, their length"
        );
        assert!(t.battery.is_none());
        let mut ours = vec![0u8; DDS_PUSH_LEN];
        ours[DDS_PUSH_LEN - dds::Battery::SIZE + 8] = 77;
        assert!(decode_push(DDS_MSG_ID, &ours, &mut t));
        assert_eq!(t.battery.map(|b| b.percent), Some(77));
        assert!(
            t.esc.is_some() && t.sa_status.is_some() && t.gimbal_pos.is_some(),
            "every subject lands in one push"
        );
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
