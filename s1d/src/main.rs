//! `s1d` — see `lib.rs`. This file is the flags and the three threads: the control loop, the
//! button watcher, and the tokio runtime that serves the socket.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use clap::Parser;
use duck_ipc_proto as proto;
use s1d::body::{FakeFile, S1Body, Serial};
use s1d::intents::Intents;
use s1d::state::State;
use s1d::tick::{Config, Heartbeat, TwistMode};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "The RoboMaster S1's body: robot.* on the microduck socket, DUML on the UART"
)]
struct Args {
    #[arg(long, default_value = proto::socket::ROBOT)]
    socket: PathBuf,
    /// The motion controller's UART.
    #[arg(long, default_value = "/dev/ttyS3")]
    port: String,
    /// Baud of that UART. Measured on the robot with `stty` while the stock firmware runs; there
    /// is no default because a guess here is a chassis that ignores every frame.
    #[arg(long, required_unless_present = "fake")]
    baud: Option<u32>,
    /// Write frames to this file instead of a UART. No robot, no serial port: what the tests and
    /// a laptop run against.
    #[arg(long)]
    fake: Option<PathBuf>,
    /// Loop rate. 100 is what the stock firmware's heartbeat runs at.
    #[arg(long, default_value_t = 100)]
    hz: u32,
    /// Velocity older than this is treated as zero. Matches `[safety] deadman_ms` on a duck.
    #[arg(long, default_value_t = 500)]
    deadman_ms: u64,
    /// Which heartbeat bytes: `proroklab` (armed the chassis on the bench) or `stock` (what the
    /// firmware sends, not yet tried by us).
    #[arg(long, default_value = "proroklab", value_parser = ["proroklab", "stock"])]
    heartbeat: String,
    /// How the twist reaches the chassis: `wheels` (kinematics here, proved) or `speed` (the
    /// controller's own, to try).
    #[arg(long, default_value = "wheels", value_parser = ["wheels", "speed"])]
    twist: String,
    /// The controller's button, if any. Absent or unreadable is not an error.
    #[arg(long, default_value = "/dev/input/event0")]
    button: PathBuf,
    /// Telemetry rate asked of the motion controller, Hz: 1, 5, 10, 20 or 50.
    #[arg(long, default_value_t = 20)]
    dds_hz: u16,
    /// Directory of `<tag>.wav` files for `robot.sound`; none means the route is refused.
    #[arg(long)]
    sounds: Option<PathBuf>,
    /// ALSA device of the speaker.
    #[arg(long, default_value = "hw:0,0")]
    audio_device: String,
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();
    proto::log_startup_identity!("s1d");

    if !s1_duml::dds::FREQUENCIES.contains(&args.dds_hz) {
        tracing::error!(
            dds_hz = args.dds_hz,
            "--dds-hz must be one of {:?}",
            s1_duml::dds::FREQUENCIES
        );
        return ExitCode::FAILURE;
    }

    let cfg = Config {
        hz: args.hz,
        deadman: Duration::from_millis(args.deadman_ms),
        heartbeat: if args.heartbeat == "stock" {
            Heartbeat::Stock
        } else {
            Heartbeat::Proroklab
        },
        twist_mode: if args.twist == "speed" {
            TwistMode::Speed
        } else {
            TwistMode::Wheels
        },
        dds_hz: args.dds_hz,
        ..Config::default()
    };

    let mut body: Box<dyn S1Body> = match (&args.fake, args.baud) {
        (Some(path), _) => {
            tracing::warn!(path = %path.display(), "--fake: frames go to a file, no robot");
            match FakeFile::create(path) {
                Ok(f) => Box::new(f),
                Err(e) => {
                    tracing::error!(error = %e, "cannot create the fake body file");
                    return ExitCode::FAILURE;
                }
            }
        }
        (None, Some(baud)) => match Serial::open(&args.port, baud) {
            Ok(s) => Box::new(s),
            Err(e) => {
                tracing::error!(port = %args.port, baud, error = %e, "cannot open the UART");
                return ExitCode::FAILURE;
            }
        },
        (None, None) => unreachable!("clap requires --baud without --fake"),
    };

    let state = Arc::new(State::new(args.hz));
    if let Some(bank) = &args.sounds {
        if let Ok(mut slot) = state.sound.lock() {
            *slot = Some(s1d::sound::Sound::new(
                bank.clone(),
                args.audio_device.clone(),
            ));
        }
        tracing::info!(bank = %bank.display(), device = %args.audio_device, "sound bank");
    }
    let intents = Arc::new(Intents::new());

    let (lock, listener) = match s1d::ipc::claim_socket(&args.socket).await {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!(path = %args.socket.display(), error = %e, "cannot claim the socket");
            return ExitCode::FAILURE;
        }
    };

    let loop_thread = {
        let (state, intents) = (Arc::clone(&state), Arc::clone(&intents));
        std::thread::Builder::new()
            .name("control".into())
            .spawn(move || s1d::tick::run(body.as_mut(), state, intents, cfg))
            .expect("spawn the control thread")
    };
    if args.fake.is_none() {
        let (state, button) = (Arc::clone(&state), args.button.clone());
        let _ = std::thread::Builder::new()
            .name("button".into())
            .spawn(move || s1d::button::watch(button, state));
    }

    let code = tokio::select! {
        result = s1d::ipc::serve(Arc::clone(&state), Arc::clone(&intents), listener, args.socket.clone()) => match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                tracing::error!(error = %e, "IPC server failed");
                ExitCode::FAILURE
            }
        },
        () = shutdown() => {
            tracing::info!("shutting down");
            ExitCode::SUCCESS
        }
    };
    state.shutdown.store(true, Ordering::Relaxed);
    let _ = loop_thread.join();
    let _ = std::fs::remove_file(&args.socket);
    drop(lock);
    code
}

/// SIGTERM (a supervisor's stop) or SIGINT (Ctrl-C).
async fn shutdown() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "cannot listen for SIGTERM");
            return std::future::pending().await;
        }
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}
