//! The socket: `robotd`'s server, trimmed to what this body answers.
//!
//! Claiming the endpoint, serving connections, notifications versus requests, the per-subscriber
//! decimation of `robot.state` — all as `robotd/src/main.rs` does them, so a client cannot tell
//! which daemon is behind `/run/robotd.sock` until it asks something only legs can do. Those
//! calls are refused *with a reason* (`IntentResult::refused`): a route that exists in the
//! contract is served, never answered "method not found" — that is for routes that are not ours.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use duck_ipc_proto as proto;
use s1_duml::cmd::SystemLed;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::api::{self, S1Call};
use crate::intents::{FollowMode, Intents};
use crate::state::State;
use crate::{MAX_LINE, MODEL_API, SOCKET_MODE};

/// Take the advisory lock beside the socket. Never unlinked, even at shutdown: the kernel releases
/// it on exit, and deleting it could leave two contenders locking different inodes under one name.
pub fn claim_lock(socket_path: &Path) -> std::io::Result<std::fs::File> {
    use std::fs::{OpenOptions, TryLockError};
    use std::io::{Error, ErrorKind};

    if let Some(parent) = socket_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut lock_path = socket_path.as_os_str().to_os_string();
    lock_path.push(".lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    match lock.try_lock() {
        Ok(()) => (),
        Err(TryLockError::WouldBlock) => {
            return Err(Error::new(
                ErrorKind::AddrInUse,
                "another daemon owns this socket",
            ));
        }
        Err(TryLockError::Error(e)) => return Err(e),
    }
    Ok(lock)
}

/// Lock, then bind. A regular file or symlink at the path is refused; a dead socket left by an
/// older daemon is removed only after it refuses a connection.
pub async fn claim_socket(socket_path: &Path) -> std::io::Result<(std::fs::File, UnixListener)> {
    use std::io::ErrorKind;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};

    let lock = claim_lock(socket_path)?;
    match std::fs::symlink_metadata(socket_path) {
        Ok(existing) if !existing.file_type().is_socket() => {
            return Err(std::io::Error::new(
                ErrorKind::AddrInUse,
                "the socket path exists and is not a socket; refusing to bind over it",
            ));
        }
        Ok(_) => (),
        Err(e) if e.kind() == ErrorKind::NotFound => (),
        Err(e) => return Err(e),
    }
    let listener = match UnixListener::bind(socket_path) {
        Ok(listener) => listener,
        Err(e) if e.kind() == ErrorKind::AddrInUse => {
            match tokio::time::timeout(Duration::from_secs(1), UnixStream::connect(socket_path))
                .await
            {
                Ok(Err(probe)) if probe.kind() == ErrorKind::ConnectionRefused => {
                    tracing::warn!(path = %socket_path.display(), "removing stale socket");
                    std::fs::remove_file(socket_path)?;
                    UnixListener::bind(socket_path)?
                }
                Ok(Err(probe)) => return Err(probe),
                _ => return Err(e),
            }
        }
        Err(e) => return Err(e),
    };
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(SOCKET_MODE))?;
    Ok((lock, listener))
}

pub async fn serve(
    state: Arc<State>,
    intents: Arc<Intents>,
    listener: UnixListener,
    socket_path: PathBuf,
) -> std::io::Result<()> {
    tracing::info!(path = %socket_path.display(), mode = format!("{SOCKET_MODE:o}"), model_api = MODEL_API, "serving robot IPC");
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(error = %e, "accept failed");
                continue;
            }
        };
        let state = Arc::clone(&state);
        let intents = Arc::clone(&intents);
        tokio::spawn(async move {
            if let Err(e) = handle(state, intents, stream).await {
                tracing::debug!(error = %e, "connection ended");
            }
        });
    }
}

pub async fn handle(
    state: Arc<State>,
    intents: Arc<Intents>,
    stream: UnixStream,
) -> std::io::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();
    let mut states: Option<tokio::sync::broadcast::Receiver<proto::RobotState>> = None;
    let mut events: Option<tokio::sync::broadcast::Receiver<api::Event>> = None;
    let mut decimate = Duration::ZERO;
    let mut last_sent: Option<Instant> = None;

    loop {
        // Pending events go out before the next line is awaited; the broadcast buffer holds 64.
        if let Some(rx) = events.as_mut() {
            while let Ok(event) = rx.try_recv() {
                write_line(&mut write_half, &event_notification(&event)).await?;
            }
        }
        let line = match states.as_mut() {
            None => match events.as_mut() {
                None => lines.next_line().await?,
                Some(rx) => tokio::select! {
                    line = lines.next_line() => line?,
                    received = rx.recv() => {
                        if let Ok(event) = received {
                            write_line(&mut write_half, &event_notification(&event)).await?;
                        }
                        continue;
                    }
                },
            },
            Some(rx) => tokio::select! {
                line = lines.next_line() => line?,
                received = rx.recv() => {
                    match received {
                        Ok(frame) => {
                            let due = last_sent.map(|at| at.elapsed() >= decimate).unwrap_or(true);
                            if due {
                                last_sent = Some(Instant::now());
                                write_line(&mut write_half, &proto::Request::notify_state(&frame)).await?;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::debug!(dropped = n, "state subscriber fell behind");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
                    }
                    continue;
                }
            },
        };
        let Some(line) = line else { return Ok(()) };
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > MAX_LINE {
            write_line(
                &mut write_half,
                &proto::Response::err(
                    None,
                    proto::Error::new(proto::code::INVALID_REQUEST, "request too large"),
                ),
            )
            .await?;
            continue;
        }
        let request: proto::Request = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(e) => {
                write_line(
                    &mut write_half,
                    &proto::Response::err(
                        None,
                        proto::Error::new(proto::code::PARSE_ERROR, e.to_string()),
                    ),
                )
                .await?;
                continue;
            }
        };
        // `s1.*`: this body's own list, outside the duck contract. No remote transport reaches it
        // today (`mediad` refuses what `as_call` cannot parse), and every method is equal here:
        // no per-method permission. How it reaches the browser is an open decision of the owner.
        if let Some(parsed) = S1Call::parse(&request.method, request.params.as_ref()) {
            let Some(id) = request.id.clone() else {
                continue;
            };
            let response = match parsed {
                Ok(S1Call::Subscribe(p)) => {
                    events = p.events.then(|| state.event_tx.subscribe());
                    dispatch_s1(&state, &intents, id, &S1Call::Subscribe(p))
                }
                Ok(call) => dispatch_s1(&state, &intents, id, &call),
                Err(e) => proto::Response::err(Some(id), e),
            };
            write_line(&mut write_half, &response).await?;
            continue;
        }
        let call = request.as_call();
        // Notifications get no reply; continuous intents arrive this way at up to 100 Hz, and so
        // do the pad's sounds (`pad-map` notifies `robot.sound` on a trigger's edge).
        let Some(id) = request.id.clone() else {
            if let Ok(call) = call {
                apply_notification(&state, &intents, &call);
            }
            continue;
        };
        if let Ok(proto::Call::RobotSubscribe(params)) = &call {
            decimate = params
                .hz
                .filter(|hz| *hz > 0)
                .map(|hz| Duration::from_secs_f64(1.0 / f64::from(hz)))
                .unwrap_or(Duration::ZERO);
            states = Some(state.state_tx.subscribe());
            last_sent = None;
        }
        let response = match call {
            Ok(call) => dispatch(&state, &intents, id, &call),
            Err(e) => proto::Response::err(Some(id), e),
        };
        write_line(&mut write_half, &response).await?;
    }
}

/// The S1's own list, answered. Every call that moves something becomes a frame from
/// `s1_duml::cmd` queued for the loop (where it differs from DJI's library, `s1-commands.md`).
pub fn dispatch_s1(
    state: &State,
    intents: &Intents,
    id: proto::Id,
    call: &S1Call,
) -> proto::Response {
    use s1_duml::cmd;
    let accepted = |id| proto::Response::ok(Some(id), &proto::IntentResult::accepted());
    let refused =
        |id, why: String| proto::Response::ok(Some(id), &proto::IntentResult::refused(&why));
    // Task id for the controller's tasks (position, recentre): any byte but 0; the controller
    // keys progress pushes by it.
    const TASK_ID: u8 = 7;
    match call {
        S1Call::Version => proto::Response::ok(
            Some(id),
            &api::VersionResult {
                s1_api_version: api::S1_API_VERSION,
                daemon: env!("CARGO_PKG_VERSION").to_owned(),
            },
        ),
        S1Call::Telemetry => proto::Response::ok(Some(id), &state.telemetry_result()),
        // The connection handler switches the stream on; this is the acknowledgement.
        S1Call::Subscribe(_) => accepted(id),
        S1Call::Frame(p) => {
            let hex = &p.hex;
            let bytes: Result<Vec<u8>, _> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("zz"), 16))
                .collect();
            let Ok(bytes) = bytes else {
                return refused(id, "hex is not hex".to_owned());
            };
            match s1_duml::Frame::decode(&bytes) {
                Ok(frame) => {
                    intents.request_frame(frame);
                    accepted(id)
                }
                Err(e) => refused(id, format!("not a frame: {e}")),
            }
        }
        S1Call::Follow(p) => {
            let mode = match p.mode {
                api::Follow::Free => FollowMode::Free,
                api::Follow::GimbalLead => FollowMode::GimbalLead,
                api::Follow::ChassisLead => FollowMode::ChassisLead,
            };
            intents.set_follow(mode);
            // Gimbal work mode: 2 follows the chassis, 1 holds its heading (FPV), 0 free.
            let gimbal_mode = match (mode, p.gimbal_fpv) {
                (FollowMode::ChassisLead, _) => 2,
                (_, true) => 1,
                _ => 0,
            };
            intents.request_frame(cmd::gimbal_workmode(0, gimbal_mode));
            let frame = match p.velocity_frame {
                api::VelocityFrame::Body => 1,
                api::VelocityFrame::Gimbal => 2,
            };
            intents.request_frame(cmd::chassis_stick_overlay(0, frame));
            accepted(id)
        }
        S1Call::Gimbal(p) => {
            let clamp_speed = |v: u16| v.clamp(5, 540);
            match *p {
                api::GimbalAction::Recenter {
                    yaw_speed,
                    pitch_speed,
                } => intents.request_frame(cmd::gimbal_recenter(
                    0,
                    TASK_ID,
                    clamp_speed(yaw_speed),
                    clamp_speed(pitch_speed),
                )),
                // A rate, re-sent by the loop every tick while fresh; `stop` is the zero rate,
                // sent once.
                api::GimbalAction::Speed { yaw, pitch } => {
                    intents.set_gimbal_rate([yaw.clamp(-540.0, 540.0), pitch.clamp(-540.0, 540.0)]);
                }
                api::GimbalAction::Stop => intents.set_gimbal_rate([0.0; 2]),
                api::GimbalAction::Sleep => {
                    intents.request_frame(cmd::gimbal_suspend_resume(0, true))
                }
                api::GimbalAction::Wake => {
                    intents.request_frame(cmd::gimbal_suspend_resume(0, false))
                }
            }
            accepted(id)
        }
        S1Call::Goto(p) => {
            if p.stop {
                intents.stop_position_task();
                intents.request_frame(cmd::chassis_position_stop(0, TASK_ID));
                return accepted(id);
            }
            let cm = |m: f64| (m.clamp(-5.0, 5.0) * 100.0).round() as i16;
            // DJI's library negates the yaw it is given (`update_position`: `-yaw_angle`), so a
            // positive yaw here is counter-clockwise, as `robot.move`'s is.
            let yaw_ddeg = (-p.yaw.clamp(-1800.0, 1800.0) * 10.0).round() as i16;
            let vel = (p.speed.clamp(0.1, 2.5) * 100.0).round() as u8;
            let omega = (p.yaw_speed.clamp(10.0, 540.0) * 10.0).round() as u16;
            intents.request_frame(cmd::chassis_position(
                0,
                TASK_ID,
                0,
                cm(p.x),
                cm(p.y),
                yaw_ddeg,
                vel,
                omega,
            ));
            // The wheels are the controller's until the task could have finished (distance at
            // the asked speed, turn at the asked rate, two seconds of slack), until `--stop`, or
            // until a driver asks for motion.
            let travel = p.x.hypot(p.y) / f64::from(vel) * 100.0
                + f64::from(yaw_ddeg.unsigned_abs()) / f64::from(omega)
                + 2.0;
            intents.start_position_task(std::time::Duration::from_secs_f64(travel));
            accepted(id)
        }
        S1Call::Lights(p) => {
            if p.blaster {
                // DJI's library drives the launcher's LED with control 7 and the effect code of
                // `rm_define` (`GunCtrl.set_led(mode)` → `Gun.set_led(mode, 7)`; on 0, off 1,
                // breath 2, flash 3, marquee 4), and the stock's own muzzle flash on the UART is
                // control 7, effect 3 (`gun_led_fire_stock_real.bin`).
                let mode = match p.effect {
                    api::LightEffect::Solid => 0,
                    api::LightEffect::Off => 1,
                    api::LightEffect::Breath => 2,
                    api::LightEffect::Flash => 3,
                    api::LightEffect::Marquee => 4,
                };
                intents.request_frame(cmd::gun_led(0, 7, mode, p.r, p.g, p.b, 1, p.t1, p.t2));
            } else {
                intents.request_led(cmd::SystemLed {
                    effect: p.effect as u8,
                    led_mask: 0xff,
                    r: p.r,
                    g: p.g,
                    b: p.b,
                    repeat: 0,
                    t1: p.t1,
                    t2: p.t2,
                    comp_mask: p.groups & 0x3f,
                });
            }
            accepted(id)
        }
        S1Call::Armor(p) => {
            if !(0.1..=5.0).contains(&p.k) {
                return refused(id, "k must be within 0.1..=5".to_owned());
            }
            intents.request_frame(cmd::armor_sensitivity(0, cmd::host::LED, p.k));
            accepted(id)
        }
        S1Call::Pwm(p) => {
            if p.duty.is_none() && p.hz.is_none() {
                return refused(id, "give duty (per mille) and/or hz".to_owned());
            }
            if let Some(hz) = p.hz {
                intents.request_frame(cmd::chassis_pwm_freq(0, p.mask & 0x3f, [hz.min(50_000); 6]));
            }
            if let Some(duty) = p.duty {
                intents.request_frame(cmd::chassis_pwm_value(
                    0,
                    p.mask & 0x3f,
                    [duty.min(1000); 6],
                ));
            }
            accepted(id)
        }
        S1Call::Wheels(p) => {
            if p.rpm.iter().any(|r| r.abs() > 1000) {
                return refused(id, "rpm is within ±1000".to_owned());
            }
            intents.set_wheels(p.rpm);
            accepted(id)
        }
        S1Call::Volume(p) => {
            let control = state.volume_control.lock().ok().and_then(|c| c.clone());
            let Some(control) = control else {
                return refused(
                    id,
                    "no mixer control named yet (--volume-control, from the bench's amixer dump)"
                        .to_owned(),
                );
            };
            let percent = p.percent.min(100);
            match std::process::Command::new("amixer")
                .args(["-q", "-c", "0", "sset", &control, &format!("{percent}%")])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
            {
                Ok(st) if st.success() => accepted(id),
                Ok(st) => refused(id, format!("amixer exited with {st}")),
                Err(e) => refused(id, format!("amixer: {e}")),
            }
        }
        S1Call::Blaster(p) => {
            if p.gel && !state.blaster_gel_allowed.load(Ordering::Relaxed) {
                return refused(
                    id,
                    "gel beads need the daemon's --blaster-gel key and the tank out".to_owned(),
                );
            }
            if !(1..=8).contains(&p.times) {
                return refused(id, "times is 1..=8".to_owned());
            }
            intents.request_frame(cmd::gun_fire(0, cmd::host::GUN, !p.gel, p.times));
            // The launcher fires `times` on its own cadence; the loop dresses each shot with the
            // muzzle light and the shot sound, as the stock does beside its fire command.
            intents.request_volley(p.times);
            accepted(id)
        }
    }
}

/// What a notification may do: a continuous intent, or a sound (the pad's triggers arrive as
/// notifications, and a quack that waited for a reply nobody reads would never play). Anything
/// else is a request in the contract and is ignored without an id, as `robotd` ignores it.
pub fn apply_notification(state: &State, intents: &Intents, call: &proto::Call) -> bool {
    if apply_intent(intents, call) {
        return true;
    }
    if let proto::Call::RobotSound(p) = call
        && let Ok(mut slot) = state.sound.lock()
        && let Some(sound) = slot.as_mut()
    {
        if let Err(why) = sound.play_held(p.tag, p.hold) {
            tracing::debug!(why, "sound notification not played");
        }
        return true;
    }
    false
}

/// A continuous intent, from a notification or a request alike.
pub fn apply_intent(intents: &Intents, call: &proto::Call) -> bool {
    match call {
        proto::Call::RobotMove(p) => {
            intents.set_twist([p.vx, p.vy, p.vyaw]);
            true
        }
        proto::Call::RobotHead(p) => {
            intents.set_head([p.neck_pitch, p.head_pitch, p.head_yaw, p.head_roll]);
            true
        }
        _ => false,
    }
}

/// The seven colours `robot.flashlight` knows, as RGB for the system LEDs.
pub fn flashlight_rgb(color: proto::FlashlightColor) -> (u8, u8, u8) {
    use proto::FlashlightColor::*;
    match color {
        White => (255, 255, 255),
        Red => (255, 0, 0),
        Green => (0, 255, 0),
        Blue => (0, 0, 255),
        Yellow => (255, 255, 0),
        Cyan => (0, 255, 255),
        Magenta => (255, 0, 255),
    }
}

/// Answer one request. Synchronous and allocation-light: these answers must be available when
/// everything else is broken.
pub fn dispatch(
    state: &State,
    intents: &Intents,
    id: proto::Id,
    call: &proto::Call,
) -> proto::Response {
    let accepted = |id| proto::Response::ok(Some(id), &proto::IntentResult::accepted());
    let refused = |id, why: &str| proto::Response::ok(Some(id), &proto::IntentResult::refused(why));
    match call {
        proto::Call::Hello(_) => proto::Response::ok(
            Some(id),
            &proto::HelloResult {
                api_version: proto::API_VERSION,
                daemon_version: proto::semver::Version::parse(env!("CARGO_PKG_VERSION")).ok(),
                revision: proto::build_info!().revision.map(str::to_owned),
            },
        ),
        proto::Call::RobotMove(_) | proto::Call::RobotHead(_) => {
            apply_intent(intents, call);
            accepted(id)
        }
        proto::Call::RobotStop => {
            intents.stop();
            accepted(id)
        }
        // Start on the pad. On this body "enable the policy" means "arm the chassis": heartbeat
        // on, wheels obeying. Off zeroes the wheels and lets the chassis disarm.
        proto::Call::RobotEnable(p) => {
            let on = if p.toggle { !intents.enabled() } else { p.on };
            intents.set_enabled(on);
            // `robotd`'s shape for a toggle: accepted, with the state the robot ended in as the
            // reason, which is the only trustworthy account a toggling client can get.
            proto::Response::ok(
                Some(id),
                &proto::IntentResult {
                    accepted: true,
                    reason: Some(if on { "armed" } else { "disarmed" }.to_owned()),
                },
            )
        }
        proto::Call::RobotInit => {
            intents.set_enabled(true);
            accepted(id)
        }
        proto::Call::RobotRelax => {
            intents.set_enabled(false);
            accepted(id)
        }
        proto::Call::RobotHealth => proto::Response::ok(Some(id), &state.health()),
        proto::Call::RobotSafeToRestart => {
            let snap = intents.snapshot();
            let moving = state.armed.load(Ordering::Relaxed)
                && snap.twist != [0.0; 3]
                && snap.twist_age < Duration::from_millis(500);
            proto::Response::ok(
                Some(id),
                &proto::SafeToRestartResult {
                    safe: !moving,
                    reason: moving.then(|| "the chassis is being driven".to_owned()),
                },
            )
        }
        proto::Call::RobotModelApi => proto::Response::ok(
            Some(id),
            &proto::ModelApiResult {
                model_api: MODEL_API,
            },
        ),
        proto::Call::RobotRemoteSessionActive => {
            proto::Response::ok(Some(id), &serde_json::json!({"active": false}))
        }
        // `netpadd` asks and picks the pad's wheel limits (`[pad_roller]`) when the answer is
        // `roller`; anything else would drive a mecanum base with a biped's gait limits.
        proto::Call::RobotMode => proto::Response::ok(
            Some(id),
            &proto::ModeResult {
                mode: "roller".to_owned(),
            },
        ),
        proto::Call::RobotPolicies => proto::Response::ok(
            Some(id),
            &proto::PoliciesResult {
                mode: "roller".to_owned(),
                enabled: intents.enabled(),
                slots: Vec::new(),
                skills: Vec::new(),
                sitting: None,
                homed: None,
                change_error: None,
            },
        ),
        proto::Call::RobotSubscribe(_) => proto::Response::ok(
            Some(id),
            &proto::SubscribeResult {
                accepted: true,
                walk: None,
                stand: None,
                unavailable: Some(
                    "this body runs no policy network: a mecanum chassis is driven by kinematics"
                        .to_owned(),
                ),
                sitstand: None,
                ground_pick: None,
                skills: Vec::new(),
            },
        ),
        proto::Call::RobotFlashlight(p) => {
            // `toggle` wins over `on`, as the contract says, and it inverts what the *last call*
            // left — whether that call toggled or set. The pad's HOME button only toggles and
            // `robotctl flashlight off` only sets, so a toggle keeping its own count would switch
            // "off" LEDs that were already off.
            let on = if p.toggle {
                !state.flashlight_on.load(Ordering::Relaxed)
            } else {
                p.on
            };
            state.flashlight_on.store(on, Ordering::Relaxed);
            let (r, g, b) = flashlight_rgb(p.color);
            intents.request_led(if on {
                SystemLed::solid(r, g, b)
            } else {
                SystemLed::off()
            });
            accepted(id)
        }
        // Routes that exist in the contract and that this body cannot honour: refused with the
        // reason, never "method not found". `robot.do`'s wording contains "not driving", which
        // is what `pad-map`'s pad-loss handler reads to stop asking for a sit.
        proto::Call::RobotDo(_) => refused(
            id,
            "the policy is not driving: this body has no skills yet (phase 2)",
        ),
        proto::Call::RobotPose(_) => refused(id, "this body has no legs to pose"),
        proto::Call::RobotMouth(_) => refused(id, "this body has no mouth"),
        // The S1's own speaker through `aplay`, from a bank of wavs named by tag. Refused with the
        // file's name when the bank lacks it, so a bench without a bank says so.
        proto::Call::RobotSound(p) => match state.sound.lock() {
            Ok(mut slot) => match slot.as_mut() {
                None => refused(id, "no sound bank on this daemon (--sounds)"),
                Some(sound) => match sound.play_held(p.tag, p.hold) {
                    Ok(_) => accepted(id),
                    Err(why) => refused(id, &why),
                },
            },
            Err(_) => refused(id, "the speaker is poisoned"),
        },
        proto::Call::RobotRest | proto::Call::RobotShutdown => refused(
            id,
            "this body neither sits nor powers itself off; use robot.relax",
        ),
        // The gaze form of `robot.head`: the point becomes the two gimbal angles and feeds the
        // same slot, and the answer is the contract's `LookResult` — the shape `robotctl robot
        // look` decodes (an `IntentResult` here fails it with "missing field `head`", which the
        // test in `tests/dispatch.rs` holds). No valid proof on the robot.
        proto::Call::RobotLook(p) => {
            let (head, clamped) = crate::tick::gaze_head(p);
            intents.set_head([
                head.neck_pitch,
                head.head_pitch,
                head.head_yaw,
                head.head_roll,
            ]);
            proto::Response::ok(Some(id), &proto::LookResult { head, clamped })
        }
        proto::Call::RobotSetMode(_) => refused(id, "this body is always on wheels"),
        // The skill table and the pad bindings: truthfully empty — this body has no one-shot
        // skills until phase 2 — in the shape the clients decode.
        proto::Call::RobotSkills => proto::Response::ok(
            Some(id),
            &proto::SkillsResult {
                skills: Vec::new(),
                built_in: Vec::new(),
            },
        ),
        proto::Call::PadBindings => proto::Response::ok(
            Some(id),
            &proto::PadBindingsResult {
                bindings: Vec::new(),
            },
        ),
        proto::Call::RobotTheremin(_) => refused(
            id,
            "no depth sensor on this body: there is no theremin to play",
        ),
        proto::Call::RobotChorale(_) | proto::Call::ChoraleSubscribe => {
            refused(id, "no voice on this body yet, so no chorale (phase 2)")
        }
        proto::Call::RobotModel => refused(
            id,
            "no kinematic model for this body yet (phase 5 brings the URDF)",
        ),
        proto::Call::RobotVetbook
        | proto::Call::RobotVetbookNote(_)
        | proto::Call::RobotVetbookTally(_) => {
            refused(id, "this body keeps no vet book yet (phase 2)")
        }
        proto::Call::RobotLoadPolicy(_)
        | proto::Call::RobotReloadPolicies
        | proto::Call::RobotSetSkill(_)
        | proto::Call::RobotRemoveSkill(_)
        | proto::Call::PadBind(_) => refused(
            id,
            "this body runs no policy network and has no skills yet (phase 2)",
        ),
        proto::Call::RobotRebootMotors(_) => refused(
            id,
            "the S1's motors are behind its motion controller; power-cycle the robot",
        ),
        other => proto::Response::err(
            Some(id),
            proto::Error::new(
                proto::code::METHOD_NOT_FOUND,
                format!("{} is not served by s1d", other.method()),
            ),
        ),
    }
}

/// `s1.event` as a notification: no id, the event as params.
pub fn event_notification(event: &api::Event) -> proto::Request {
    proto::Request {
        jsonrpc: "2.0".to_owned(),
        id: None,
        method: api::method::EVENT.to_owned(),
        params: serde_json::to_value(event).ok(),
    }
}

pub async fn write_line<T: serde::Serialize>(
    out: &mut tokio::net::unix::OwnedWriteHalf,
    message: &T,
) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(message)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    line.push(b'\n');
    out.write_all(&line).await?;
    out.flush().await
}
