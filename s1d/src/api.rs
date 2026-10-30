//! The S1's own list — what this body does that a duck does not — served beside the duck
//! contract on the same socket, versioned on its own.
//!
//! One entry per physical capability, named `s1.<thing>`; the duck's `robot.*` routes keep
//! covering what both bodies share (move, head, flashlight, sound, state, health). Nothing of
//! `duck-ipc-proto` is touched by this list: a duck asked `s1.follow` answers "method not found"
//! as it always has, and an S1 asked `robot.pose` answers "refused" as it always has.
//!
//! The wire is the same JSON-RPC/NDJSON; params are `deny_unknown_fields` so a member from a later
//! release is refused by name (`tests/s1_api.rs` holds every call to that), and every call maps to
//! the bytes DJI's own library would send (`s1_duml::cmd`, oracle-tested).

use duck_ipc_proto::{Error, code};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Bumped when a call, a member or a result changes shape.
pub const S1_API_VERSION: u32 = 1;

pub mod method {
    pub const FOLLOW: &str = "s1.follow";
    pub const GIMBAL: &str = "s1.gimbal";
    pub const GOTO: &str = "s1.goto";
    pub const LIGHTS: &str = "s1.lights";
    pub const ARMOR: &str = "s1.armor";
    pub const PWM: &str = "s1.pwm";
    pub const TELEMETRY: &str = "s1.telemetry";
    pub const SUBSCRIBE: &str = "s1.subscribe";
    pub const EVENT: &str = "s1.event";
    pub const FRAME: &str = "s1.frame";
    pub const VERSION: &str = "s1.version";
    pub const WHEELS: &str = "s1.wheels";
    pub const VOLUME: &str = "s1.volume";
    pub const BLASTER: &str = "s1.blaster";
}

/// Who leads whom (DJI's `RobotCtrl.set_mode`: `free`, `gimbal_lead`, `chassis_lead`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Follow {
    Free,
    GimbalLead,
    ChassisLead,
}

/// The frame a velocity is read in (DJI's stick overlay): the body's axes or the gimbal's heading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VelocityFrame {
    #[default]
    Body,
    Gimbal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FollowParams {
    pub mode: Follow,
    #[serde(default)]
    pub velocity_frame: VelocityFrame,
    /// Gimbal work mode 1, the app's FPV: the gimbal holds its heading relative to the body.
    #[serde(default)]
    pub gimbal_fpv: bool,
}

/// Per-wheel speed, rpm (±1000): front-right, front-left, back-left, back-right. Continuous
/// like `robot.move`, with the same deadman; `robot.move` and this overwrite each other.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WheelsParams {
    pub rpm: [i16; 4],
}

/// Speaker volume, 0..100, through the mixer (`amixer`), as the stock's `0x3f/0x1b` sets it on
/// the same controller.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeParams {
    pub percent: u8,
}

/// Fire the blaster. Infrared unless the daemon was started with `--blaster-gel` and the bead tank
/// is out (owner's rule, 2026-10-29); `times` 1..8.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlasterParams {
    #[serde(default = "default_times")]
    pub times: u8,
    #[serde(default)]
    pub gel: bool,
}

fn default_times() -> u8 {
    1
}

/// One gimbal action. `recenter` and the speeds are degrees per second (5..540); `speed` is a
/// continuous rate in degrees per second on each axis (±540) that lasts until the next `speed`
/// or a `stop`; `sleep` folds the gimbal, `wake` levels it. Signs are DJI's own, as this list
/// is DJI's library on the duck's socket: yaw positive turns **right**, pitch positive looks
/// **up** — the opposite of `robot.head`, which keeps the duck's frame (`tick::gimbal_ddeg`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum GimbalAction {
    Recenter {
        #[serde(default = "default_gimbal_speed")]
        yaw_speed: u16,
        #[serde(default = "default_gimbal_speed")]
        pitch_speed: u16,
    },
    Speed {
        yaw: f64,
        pitch: f64,
    },
    Stop,
    Sleep,
    Wake,
}

fn default_gimbal_speed() -> u16 {
    100
}

/// Move a distance and stop, the loop closed by the motion controller: `x`, `y` metres (±5,
/// forward and left), `yaw` degrees (±1800, counter-clockwise), `speed` m/s (0.1..2.5),
/// `yaw_speed` °/s (10..540). `stop` abandons the move in flight.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GotoParams {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub yaw: f64,
    #[serde(default = "default_goto_speed")]
    pub speed: f64,
    #[serde(default = "default_goto_yaw_speed")]
    pub yaw_speed: f64,
    #[serde(default)]
    pub stop: bool,
}

fn default_goto_speed() -> f64 {
    0.5
}

fn default_goto_yaw_speed() -> f64 {
    30.0
}

/// LED effect, DJI's numbering on the wire the stock app uses (`0x3f/0x32`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightEffect {
    Off = 0,
    Solid = 1,
    Breath = 2,
    Flash = 3,
    Marquee = 4,
}

/// Which lights: the six groups of the body by bit (`0x01` bottom back, `0x02` bottom front,
/// `0x04` bottom left, `0x08` bottom right, `0x10` top left, `0x20` top right, `0x3f` all), or the
/// blaster's own LED (`blaster: true`, which ignores `groups`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightsParams {
    pub effect: LightEffect,
    #[serde(default)]
    pub r: u8,
    #[serde(default)]
    pub g: u8,
    #[serde(default)]
    pub b: u8,
    /// Effect times, ms (breath: rise/fall; flash: on/off; marquee: step).
    #[serde(default)]
    pub t1: u16,
    #[serde(default)]
    pub t2: u16,
    #[serde(default = "default_groups")]
    pub groups: u16,
    #[serde(default)]
    pub blaster: bool,
}

fn default_groups() -> u16 {
    0x3f
}

/// Hit sensitivity as DJI scales it: `k = 1` the factory thresholds, below 1 more sensitive.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmorParams {
    pub k: f32,
}

/// The six PWM ports: a bit per port in `mask`, duty in per mille or frequency in Hz.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PwmParams {
    #[serde(default = "default_pwm_mask")]
    pub mask: u8,
    #[serde(default)]
    pub duty: Option<u16>,
    #[serde(default)]
    pub hz: Option<u16>,
}

fn default_pwm_mask() -> u8 {
    0x3f
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameParams {
    pub hex: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscribeParams {
    /// `true` to receive `s1.event` notifications on this connection, `false` to stop.
    pub events: bool,
}

/// Everything the motion controller pushes, as last decoded.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TelemetryResult {
    pub pushes: u64,
    pub foreign_pushes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wheels_rpm: Option<[i16; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_m: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub velocity_world: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub velocity_body: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acc: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gyro: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attitude_deg: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusWord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gimbal_deg: Option<[f32; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sticks: Option<[bool; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery: Option<BatteryResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusWord {
    pub static_flag: bool,
    pub up_hill: bool,
    pub down_hill: bool,
    pub on_slope: bool,
    pub picked_up: bool,
    pub slip: bool,
    pub impact_x: bool,
    pub impact_y: bool,
    pub impact_z: bool,
    pub roll_over: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatteryResult {
    pub percent: u8,
    pub adc_value: u16,
    pub temperature: i16,
    pub current_ma: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionResult {
    pub s1_api_version: u32,
    pub daemon: String,
}

/// What the body reports on its own: an armour plate hit, an infrared hit, the button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    ArmorHit {
        plate: u8,
        hit_kind: u8,
        mic_value: u16,
    },
    IrHit {
        role: u8,
        skill: u8,
        device: u8,
        pin: u8,
    },
    Button {
        code: u16,
        value: i32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum S1Call {
    Follow(FollowParams),
    Gimbal(GimbalAction),
    Goto(GotoParams),
    Lights(LightsParams),
    Armor(ArmorParams),
    Pwm(PwmParams),
    Telemetry,
    Subscribe(SubscribeParams),
    Frame(FrameParams),
    Version,
    Wheels(WheelsParams),
    Volume(VolumeParams),
    Blaster(BlasterParams),
}

impl S1Call {
    pub fn method(&self) -> &'static str {
        match self {
            Self::Follow(_) => method::FOLLOW,
            Self::Gimbal(_) => method::GIMBAL,
            Self::Goto(_) => method::GOTO,
            Self::Lights(_) => method::LIGHTS,
            Self::Armor(_) => method::ARMOR,
            Self::Pwm(_) => method::PWM,
            Self::Telemetry => method::TELEMETRY,
            Self::Subscribe(_) => method::SUBSCRIBE,
            Self::Frame(_) => method::FRAME,
            Self::Version => method::VERSION,
            Self::Wheels(_) => method::WHEELS,
            Self::Volume(_) => method::VOLUME,
            Self::Blaster(_) => method::BLASTER,
        }
    }

    pub fn params(&self) -> Value {
        fn encode(v: &impl Serialize) -> Value {
            serde_json::to_value(v).expect("params serialise")
        }
        match self {
            Self::Follow(p) => encode(p),
            Self::Gimbal(p) => encode(p),
            Self::Goto(p) => encode(p),
            Self::Lights(p) => encode(p),
            Self::Armor(p) => encode(p),
            Self::Pwm(p) => encode(p),
            Self::Subscribe(p) => encode(p),
            Self::Frame(p) => encode(p),
            Self::Wheels(p) => encode(p),
            Self::Volume(p) => encode(p),
            Self::Blaster(p) => encode(p),
            Self::Telemetry | Self::Version => Value::Object(Default::default()),
        }
    }

    /// `Some` for every `s1.` method, `None` for a method that is not this list's.
    pub fn parse(method_name: &str, params: Option<&Value>) -> Option<Result<Self, Error>> {
        fn decode<T: for<'de> Deserialize<'de>>(params: Option<&Value>) -> Result<T, Error> {
            serde_json::from_value(params.cloned().unwrap_or(Value::Null))
                .map_err(|e| Error::new(code::INVALID_PARAMS, e.to_string()))
        }
        Some(Ok(match method_name {
            method::FOLLOW => Self::Follow(match decode(params) {
                Ok(p) => p,
                Err(e) => return Some(Err(e)),
            }),
            method::GIMBAL => Self::Gimbal(match decode(params) {
                Ok(p) => p,
                Err(e) => return Some(Err(e)),
            }),
            method::GOTO => Self::Goto(match decode(params) {
                Ok(p) => p,
                Err(e) => return Some(Err(e)),
            }),
            method::LIGHTS => Self::Lights(match decode(params) {
                Ok(p) => p,
                Err(e) => return Some(Err(e)),
            }),
            method::ARMOR => Self::Armor(match decode(params) {
                Ok(p) => p,
                Err(e) => return Some(Err(e)),
            }),
            method::PWM => Self::Pwm(match decode(params) {
                Ok(p) => p,
                Err(e) => return Some(Err(e)),
            }),
            method::TELEMETRY => Self::Telemetry,
            method::SUBSCRIBE => Self::Subscribe(match decode(params) {
                Ok(p) => p,
                Err(e) => return Some(Err(e)),
            }),
            method::FRAME => Self::Frame(match decode(params) {
                Ok(p) => p,
                Err(e) => return Some(Err(e)),
            }),
            method::VERSION => Self::Version,
            method::WHEELS => Self::Wheels(match decode(params) {
                Ok(p) => p,
                Err(e) => return Some(Err(e)),
            }),
            method::VOLUME => Self::Volume(match decode(params) {
                Ok(p) => p,
                Err(e) => return Some(Err(e)),
            }),
            method::BLASTER => Self::Blaster(match decode(params) {
                Ok(p) => p,
                Err(e) => return Some(Err(e)),
            }),
            other if other.starts_with("s1.") => {
                return Some(Err(Error::new(
                    code::METHOD_NOT_FOUND,
                    format!("{other} is not on the S1's list (version {S1_API_VERSION})"),
                )));
            }
            _ => return None,
        }))
    }

    /// One of each, with every member set: what the unknown-member test and the CLI's help use.
    pub fn every_call() -> Vec<Self> {
        vec![
            Self::Follow(FollowParams {
                mode: Follow::GimbalLead,
                velocity_frame: VelocityFrame::Gimbal,
                gimbal_fpv: false,
            }),
            Self::Wheels(WheelsParams {
                rpm: [100, 100, 100, 100],
            }),
            Self::Volume(VolumeParams { percent: 50 }),
            Self::Blaster(BlasterParams {
                times: 1,
                gel: false,
            }),
            Self::Gimbal(GimbalAction::Recenter {
                yaw_speed: 100,
                pitch_speed: 100,
            }),
            Self::Gimbal(GimbalAction::Speed {
                yaw: 30.0,
                pitch: -15.0,
            }),
            Self::Goto(GotoParams {
                x: 1.0,
                y: -0.5,
                yaw: 90.0,
                speed: 0.5,
                yaw_speed: 30.0,
                stop: false,
            }),
            Self::Lights(LightsParams {
                effect: LightEffect::Breath,
                r: 0,
                g: 0,
                b: 255,
                t1: 500,
                t2: 500,
                groups: 0x3f,
                blaster: false,
            }),
            Self::Armor(ArmorParams { k: 1.0 }),
            Self::Pwm(PwmParams {
                mask: 0x3f,
                duty: Some(500),
                hz: Some(1000),
            }),
            Self::Telemetry,
            Self::Subscribe(SubscribeParams { events: true }),
            Self::Frame(FrameParams {
                hex: "55".to_owned(),
            }),
            Self::Version,
        ]
    }
}
