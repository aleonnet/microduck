//! `s1ctl` — the S1's own list from a shell, the way `robotctl` is the duck's.
//!
//! ```text
//! s1ctl follow gimbal_lead
//! s1ctl gimbal recenter | speed 30 -15 | stop | sleep | wake
//! s1ctl goto 1.0 -0.5 90 | goto --stop
//! s1ctl lights breath 0 0 255 --t1 500 --t2 500 [--groups 0x3f] [--blaster]
//! s1ctl armor 1.0 · s1ctl pwm --duty 500 --hz 1000 · s1ctl wheels 100 100 100 100
//! s1ctl volume 50 · s1ctl blaster [--times 1] [--gel] · s1ctl telemetry · s1ctl version
//! s1ctl events          (prints s1.event notifications until Ctrl-C)
//! s1ctl frame 55…       (one raw DUML frame, diagnostic)
//! ```
//!
//! Talks NDJSON on the robot socket (`--socket`), or on a TCP port when the Mac reaches the
//! robot through `adb forward tcp:9000 localfilesystem:/data/debian/run/robotd.sock` (`--tcp`).

use std::io::{BufRead, BufReader, Write};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use duck_ipc_proto as proto;
use s1d::api::{self, S1Call};

#[derive(Parser)]
#[command(version, about = "The RoboMaster S1's own list, from a shell")]
struct Args {
    #[arg(long, default_value = proto::socket::ROBOT)]
    socket: String,
    /// `host:port` instead of the unix socket (through an adb forward from the Mac).
    #[arg(long)]
    tcp: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Who leads whom: free, gimbal_lead (chassis follows the gimbal) or chassis_lead.
    Follow {
        mode: String,
        #[arg(long, default_value = "body")]
        velocity_frame: String,
        #[arg(long)]
        gimbal_fpv: bool,
    },
    /// recenter [yaw_speed pitch_speed] | speed <yaw °/s> <pitch °/s> | stop | sleep | wake
    Gimbal {
        action: String,
        #[arg(allow_hyphen_values = true)]
        values: Vec<f64>,
    },
    /// Move x y (m) and yaw (°, DJI's signs by its library, not measured) and stop; `--stop` abandons it.
    Goto {
        #[arg(allow_hyphen_values = true, num_args = 0..=3)]
        xyyaw: Vec<f64>,
        #[arg(long, default_value_t = 0.5)]
        speed: f64,
        #[arg(long, default_value_t = 30.0)]
        yaw_speed: f64,
        #[arg(long)]
        stop: bool,
    },
    /// effect (off|solid|breath|flash|marquee) r g b
    Lights {
        effect: String,
        #[arg(num_args = 0..=3)]
        rgb: Vec<u8>,
        #[arg(long, default_value_t = 0)]
        t1: u16,
        #[arg(long, default_value_t = 0)]
        t2: u16,
        #[arg(long, default_value = "0x3f")]
        groups: String,
        #[arg(long)]
        blaster: bool,
    },
    /// Hit sensitivity factor (1.0 = factory).
    Armor { k: f32 },
    /// The six PWM ports.
    Pwm {
        #[arg(long, default_value = "0x3f")]
        mask: String,
        #[arg(long)]
        duty: Option<u16>,
        #[arg(long)]
        hz: Option<u16>,
    },
    /// Per-wheel rpm: front-right front-left back-left back-right.
    Wheels {
        #[arg(allow_hyphen_values = true, num_args = 4)]
        rpm: Vec<i16>,
    },
    /// Speaker volume, percent.
    Volume { percent: u8 },
    /// Fire: infrared unless --gel (and the daemon's key).
    Blaster {
        #[arg(long, default_value_t = 1)]
        times: u8,
        #[arg(long)]
        gel: bool,
    },
    /// Everything the motion controller pushes.
    Telemetry,
    /// The list's version.
    Version,
    /// Print s1.event notifications until Ctrl-C.
    Events,
    /// One raw DUML frame, hex.
    Frame { hex: String },
}

fn parse_u16(s: &str) -> Result<u16, String> {
    let v = s.trim_start_matches("0x");
    if s.starts_with("0x") {
        u16::from_str_radix(v, 16).map_err(|e| e.to_string())
    } else {
        s.parse()
            .map_err(|e: std::num::ParseIntError| e.to_string())
    }
}

fn build(command: &Command) -> Result<S1Call, String> {
    Ok(match command {
        Command::Follow {
            mode,
            velocity_frame,
            gimbal_fpv,
        } => S1Call::Follow(api::FollowParams {
            mode: serde_json::from_value(serde_json::json!(mode)).map_err(|e| e.to_string())?,
            velocity_frame: serde_json::from_value(serde_json::json!(velocity_frame))
                .map_err(|e| e.to_string())?,
            gimbal_fpv: *gimbal_fpv,
        }),
        Command::Gimbal { action, values } => {
            let action = match (action.as_str(), values.as_slice()) {
                ("recenter", []) => api::GimbalAction::Recenter {
                    yaw_speed: 100,
                    pitch_speed: 100,
                },
                ("recenter", [y, p]) => api::GimbalAction::Recenter {
                    yaw_speed: *y as u16,
                    pitch_speed: *p as u16,
                },
                ("speed", [y, p]) => api::GimbalAction::Speed { yaw: *y, pitch: *p },
                ("stop", []) => api::GimbalAction::Stop,
                ("sleep", []) => api::GimbalAction::Sleep,
                ("wake", []) => api::GimbalAction::Wake,
                _ => {
                    return Err(
                        "gimbal: recenter [yaw pitch] | speed yaw pitch | stop | sleep | wake"
                            .to_owned(),
                    );
                }
            };
            S1Call::Gimbal(action)
        }
        Command::Goto {
            xyyaw,
            speed,
            yaw_speed,
            stop,
        } => S1Call::Goto(api::GotoParams {
            x: xyyaw.first().copied().unwrap_or(0.0),
            y: xyyaw.get(1).copied().unwrap_or(0.0),
            yaw: xyyaw.get(2).copied().unwrap_or(0.0),
            speed: *speed,
            yaw_speed: *yaw_speed,
            stop: *stop,
        }),
        Command::Lights {
            effect,
            rgb,
            t1,
            t2,
            groups,
            blaster,
        } => S1Call::Lights(api::LightsParams {
            effect: serde_json::from_value(serde_json::json!(effect)).map_err(|e| e.to_string())?,
            r: rgb.first().copied().unwrap_or(0),
            g: rgb.get(1).copied().unwrap_or(0),
            b: rgb.get(2).copied().unwrap_or(0),
            t1: *t1,
            t2: *t2,
            groups: parse_u16(groups)?,
            blaster: *blaster,
        }),
        Command::Armor { k } => S1Call::Armor(api::ArmorParams { k: *k }),
        Command::Pwm { mask, duty, hz } => S1Call::Pwm(api::PwmParams {
            mask: parse_u16(mask)? as u8,
            duty: *duty,
            hz: *hz,
        }),
        Command::Wheels { rpm } => S1Call::Wheels(api::WheelsParams {
            rpm: [rpm[0], rpm[1], rpm[2], rpm[3]],
        }),
        Command::Volume { percent } => S1Call::Volume(api::VolumeParams { percent: *percent }),
        Command::Blaster { times, gel } => S1Call::Blaster(api::BlasterParams {
            times: *times,
            gel: *gel,
        }),
        Command::Telemetry => S1Call::Telemetry,
        Command::Version => S1Call::Version,
        Command::Events => S1Call::Subscribe(api::SubscribeParams { events: true }),
        Command::Frame { hex } => S1Call::Frame(api::FrameParams { hex: hex.clone() }),
    })
}

fn main() -> ExitCode {
    let args = Args::parse();
    let call = match build(&args.command) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let mut stream: Box<dyn ReadWrite> = match &args.tcp {
        Some(addr) => match std::net::TcpStream::connect(addr) {
            Ok(s) => Box::new(s),
            Err(e) => {
                eprintln!("cannot reach {addr}: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => match std::os::unix::net::UnixStream::connect(&args.socket) {
            Ok(s) => Box::new(s),
            Err(e) => {
                eprintln!("cannot reach {}: {e}", args.socket);
                return ExitCode::FAILURE;
            }
        },
    };
    let request = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": call.method(), "params": call.params()});
    if writeln!(stream, "{request}").is_err() {
        eprintln!("write failed");
        return ExitCode::FAILURE;
    }
    let follow_events = matches!(args.command, Command::Events);
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                print!("{line}");
                if !follow_events {
                    break;
                }
            }
        }
    }
    ExitCode::SUCCESS
}

trait ReadWrite: std::io::Read + std::io::Write {}
impl<T: std::io::Read + std::io::Write> ReadWrite for T {}
