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

use crate::intents::Intents;
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
    let mut decimate = Duration::ZERO;
    let mut last_sent: Option<Instant> = None;

    loop {
        let line = match states.as_mut() {
            None => lines.next_line().await?,
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
        let call = request.as_call();
        // Notifications get no reply; continuous intents arrive this way at up to 100 Hz.
        let Some(id) = request.id.clone() else {
            if let Ok(call) = call {
                apply_intent(&intents, &call);
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
        proto::Call::RobotSound(_) => refused(
            id,
            "no voice on this body yet (phase 2: aplay on the S1 speaker)",
        ),
        proto::Call::RobotRest | proto::Call::RobotShutdown => refused(
            id,
            "this body neither sits nor powers itself off; use robot.relax",
        ),
        proto::Call::RobotLook(_) => {
            refused(id, "gaze on the gimbal arrives in phase 2; send robot.head")
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
