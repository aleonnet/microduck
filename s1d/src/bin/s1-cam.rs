//! A frame from the S1's camera through the stock camera process's own call sequence, and the
//! rate the device sustains — stages 7 and 11 of `docs/project/s1-bringup.md`.
//!
//! ```text
//! s1-cam --device /dev/video0 --frames 3 --seconds 10 --out /tmp/cam
//! ```
//!
//! The sequence is `s1d::cam` — the stock's recording replayed with every answer held to it, the
//! stream switched on only when all of them matched. A divergence is printed as JSON and exits 3
//! with nothing streamed: the two generic streams that rebooted the robot are what this refuses
//! to repeat. Frames are written raw (NV12, `bytesused` bytes each, `frame_000.nv12` …) for the
//! Mac to convert; the rate is counted over `--seconds` after the frames are saved, with the
//! sequence numbers the driver stamps (gaps are drops) and the process's CPU time.

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    use std::process::ExitCode;
    use std::time::Instant;

    use clap::Parser;
    use s1d::cam::{Error, Session, linux::Device};

    #[derive(Parser)]
    #[command(
        about = "The S1's camera, opened as the stock opens it: frames to files, then the rate"
    )]
    struct Args {
        #[arg(long, default_value = "/dev/video0")]
        device: String,
        /// Frames to save before the rate is measured.
        #[arg(long, default_value_t = 3)]
        frames: u32,
        /// Seconds to count frames for after the saved ones; 0 skips the measurement.
        #[arg(long, default_value_t = 10.0)]
        seconds: f64,
        #[arg(long, default_value = "/tmp/cam")]
        out: std::path::PathBuf,
        /// How long to wait for one frame before calling the stream stalled.
        #[arg(long, default_value_t = 2000)]
        timeout_ms: i32,
    }

    fn cpu_seconds() -> (f64, f64) {
        let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: a zeroed rusage for this process, filled by the kernel.
        unsafe {
            libc::getrusage(libc::RUSAGE_SELF, &mut ru);
        }
        let s = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
        (s(ru.ru_utime), s(ru.ru_stime))
    }

    let args = Args::parse();
    let dev = match Device::open(&args.device) {
        Ok(d) => d,
        Err(e) => {
            println!(
                "{}",
                serde_json::json!({"error": format!("open {}: {e}", args.device)})
            );
            return ExitCode::from(2);
        }
    };
    let mut session = match Session::open(dev) {
        Ok(s) => s,
        Err(Error::Diverged(d)) => {
            println!(
                "{}",
                serde_json::json!({"diverged": {"step": format!("{:?}", d.step), "expected": d.expected, "got": d.got}, "streamed": false})
            );
            return ExitCode::from(3);
        }
        Err(Error::Io { step, source }) => {
            println!(
                "{}",
                serde_json::json!({"error": format!("{step:?}: {source}"), "streamed": false})
            );
            return ExitCode::from(2);
        }
    };
    if let Err(e) = std::fs::create_dir_all(&args.out) {
        println!(
            "{}",
            serde_json::json!({"error": format!("mkdir {}: {e}", args.out.display())})
        );
        return ExitCode::from(2);
    }

    let mut saved = Vec::new();
    let mut first_sequence = None;
    let mut last_sequence = None;
    let mut counted = 0u64;
    let mut stalled = false;
    let started = Instant::now();
    let (cpu_u0, cpu_s0) = cpu_seconds();
    let mut measuring_from: Option<Instant> = None;
    loop {
        let frame = match session.dequeue(args.timeout_ms) {
            Ok(Some(f)) => f,
            Ok(None) => {
                stalled = true;
                break;
            }
            Err(e) => {
                println!(
                    "{}",
                    serde_json::json!({"error": format!("dequeue: {e}"), "streamed": true})
                );
                let _ = session.close();
                return ExitCode::from(2);
            }
        };
        let (index, sequence, len) = (frame.index, frame.sequence, frame.data.len());
        first_sequence.get_or_insert(sequence);
        last_sequence = Some(sequence);
        if (saved.len() as u32) < args.frames {
            let path = args.out.join(format!("frame_{:03}.nv12", saved.len()));
            if let Err(e) = std::fs::write(&path, frame.data) {
                println!(
                    "{}",
                    serde_json::json!({"error": format!("write {}: {e}", path.display())})
                );
                let _ = session.close();
                return ExitCode::from(2);
            }
            saved.push(serde_json::json!({"path": path, "bytes": len, "sequence": sequence}));
            if saved.len() as u32 == args.frames {
                measuring_from = Some(Instant::now());
            }
        } else if let Some(from) = measuring_from {
            counted += 1;
            if from.elapsed().as_secs_f64() >= args.seconds {
                session.requeue(index).ok();
                break;
            }
        } else {
            break;
        }
        if let Err(e) = session.requeue(index) {
            println!(
                "{}",
                serde_json::json!({"error": format!("requeue {index}: {e}")})
            );
            let _ = session.close();
            return ExitCode::from(2);
        }
    }
    let (cpu_u1, cpu_s1) = cpu_seconds();
    let measured_s = measuring_from
        .map(|f| f.elapsed().as_secs_f64())
        .unwrap_or(0.0);
    let span = match (first_sequence, last_sequence) {
        (Some(a), Some(b)) => b.wrapping_sub(a) as u64 + 1,
        _ => 0,
    };
    let delivered = counted + saved.len() as u64;
    let report = serde_json::json!({
        "streamed": true,
        "format": {"width": session.format.width, "height": session.format.height,
                   "fourcc": "NV12", "field": session.format.field,
                   "bytesperline": session.format.bytesperline, "sizeimage": session.format.sizeimage},
        "frames_saved": saved,
        "rate": {"frames": counted, "seconds": measured_s,
                 "fps": if measured_s > 0.0 { counted as f64 / measured_s } else { 0.0 }},
        "driver_sequence_span": span, "frames_delivered": delivered,
        "dropped_by_driver": span.saturating_sub(delivered),
        "cpu_seconds": {"user": cpu_u1 - cpu_u0, "system": cpu_s1 - cpu_s0,
                        "wall": started.elapsed().as_secs_f64()},
        "stalled": stalled,
    });
    let closed = session.close();
    println!("{report}");
    if let Err(e) = closed {
        eprintln!("streamoff: {e}");
    }
    if stalled {
        ExitCode::from(4)
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("s1-cam drives a V4L2 device; it runs on the robot (Linux), not here");
    std::process::ExitCode::from(2)
}
