//! Does the duck detector run on this CPU, how fast, and does it still see anything?
//!
//! ```text
//! ORT_DYLIB_PATH=/usr/lib/libonnxruntime.so.1.28.0 s1-infer --model duck_detect.onnx --frames /opt/robot/frames
//! ```
//!
//! `duck-detect`'s own bench (`duck-bench`) is the NPU path — `rknn::Model` — and the S1 has no
//! NPU. This is the same measurement over `duck_detect::onnx::Model`, which is what the S1 can run
//! and what `mediad` runs on a board whose NPU is off. It is stage 12 of `docs/project/s1-bringup.md`
//! on both tiers: cross-compiled for the robot, built natively for the Mac, the same frames in.
//!
//! Frames are decoded up front so the JPEG/PNG decode is not in the timed loop; the model is run
//! once per frame per pass; the report is latency percentiles, CPU seconds per frame where the
//! kernel says (`/proc/self/stat`, Linux only), and detections per frame — because a model that
//! runs and sees nothing looks exactly like one that works.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::Parser;
use duck_detect::{decode, letterbox_rgb, onnx::Model};

#[derive(Parser)]
#[command(about = "Run the duck detector's ONNX path on this CPU and report what it costs")]
struct Args {
    /// The `.onnx`, e.g. `duck_detect.onnx` from the Hub (`pollen-robotics/microduck-duck-detector`).
    #[arg(long)]
    model: PathBuf,
    /// A directory of JPEGs or PNGs.
    #[arg(long)]
    frames: PathBuf,
    /// Timed runs skipped first, so the first-call costs are not the answer.
    #[arg(long, default_value_t = 3)]
    warmup: usize,
    /// How many times the whole set is run. At least one: a run with no inference has no number.
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u16).range(1..))]
    passes: u16,
    /// Detection threshold, on this model's own score scale.
    #[arg(long, default_value_t = 0.35)]
    threshold: f32,
    /// Inferences per second; `0` runs flat out, which is the ceiling and also what heats a board.
    #[arg(long, default_value_t = 2.0)]
    hz: f64,
}

/// CPU seconds this process has used, user + system, from `/proc/self/stat`. `None` where there is
/// no such file (macOS), so the report says "not measured" rather than zero.
fn cpu_seconds() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let tail = stat.rsplit_once(')').map(|(_, rest)| rest)?;
    let fields: Vec<&str> = tail.split_whitespace().collect();
    let utime: f64 = fields.get(11)?.parse().ok()?;
    let stime: f64 = fields.get(12)?.parse().ok()?;
    // `_SC_CLK_TCK` is 100 on every Linux this runs on; asked for rather than assumed.
    unsafe extern "C" {
        fn sysconf(name: i32) -> i64;
    }
    // SAFETY: `sysconf(2)` (`_SC_CLK_TCK` on Linux) reads one constant and touches nothing.
    let ticks = unsafe { sysconf(2) };
    let ticks = if ticks > 0 { ticks as f64 } else { 100.0 };
    Some((utime + stime) / ticks)
}

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    match sorted.len() {
        0 => Duration::ZERO,
        n => sorted[((n - 1) as f64 * fraction).round() as usize],
    }
}

fn images(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(directory)
        .with_context(|| format!("cannot read {}", directory.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.extension().is_some_and(|ext| {
                ["jpg", "jpeg", "png"]
                    .iter()
                    .any(|known| ext.eq_ignore_ascii_case(known))
            })
        })
        .collect();
    found.sort();
    if found.is_empty() {
        bail!("no jpeg or png in {}", directory.display());
    }
    Ok(found)
}

fn main() -> Result<()> {
    let args = Args::parse();
    let paths = images(&args.frames)?;
    let mut model = Model::open(&args.model)?;
    let (height, width, channels) = model.input;
    if channels != 3 || width != height {
        bail!("this bench only knows a square RGB model, got {width}×{height}×{channels}");
    }
    println!(
        "model {}×{}×{} from {}\n{} frames, {} passes, threshold {}",
        width,
        height,
        channels,
        args.model.display(),
        paths.len(),
        args.passes,
        args.threshold
    );

    let mut frames = Vec::new();
    for path in &paths {
        let image = image::open(path)
            .with_context(|| format!("cannot decode {}", path.display()))?
            .to_rgb8();
        frames.push(image);
    }

    let mut square = Vec::new();
    let mut raw = Vec::new();
    let fit = |image: &image::RgbImage, square: &mut Vec<u8>| {
        letterbox_rgb(
            image.as_raw(),
            image.width() as usize,
            image.height() as usize,
            width,
            square,
        )
    };
    for _ in 0..args.warmup {
        let letterbox = fit(&frames[0], &mut square);
        model.infer(&square, &mut raw)?;
        let _ = decode(&raw, letterbox, args.threshold, 0.5);
    }

    let period = (args.hz > 0.0).then(|| Duration::from_secs_f64(1.0 / args.hz));
    let mut next = Instant::now();
    let mut latencies: Vec<Duration> = Vec::with_capacity(frames.len() * usize::from(args.passes));
    let mut detections = 0usize;
    let mut frames_with_one = 0usize;
    let cpu_before = cpu_seconds();
    let wall_before = Instant::now();

    for pass in 0..args.passes {
        for image in &frames {
            if let Some(period) = period {
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                }
                next += period;
            }
            let letterbox = fit(image, &mut square);
            let started = Instant::now();
            model.infer(&square, &mut raw)?;
            let found = decode(&raw, letterbox, args.threshold, 0.5);
            latencies.push(started.elapsed());
            if pass == 0 {
                detections += found.len();
                if !found.is_empty() {
                    frames_with_one += 1;
                }
            }
        }
    }

    let wall = wall_before.elapsed();
    latencies.sort();
    let millis = |d: Duration| d.as_secs_f64() * 1e3;
    println!(
        "latency    p50 {:.1} ms · p95 {:.1} ms · max {:.1} ms over {} inferences",
        millis(percentile(&latencies, 0.50)),
        millis(percentile(&latencies, 0.95)),
        millis(*latencies.last().expect("at least one inference")),
        latencies.len()
    );
    match period {
        Some(_) => println!("paced at   {:.1} Hz", args.hz),
        None => println!(
            "throughput {:.2} fps flat out",
            latencies.len() as f64 / wall.as_secs_f64()
        ),
    }
    match (cpu_before, cpu_seconds()) {
        (Some(before), Some(after)) => {
            let per_frame_ms = 1e3 * (after - before) / latencies.len() as f64;
            println!("cpu        {per_frame_ms:.1} ms of CPU per frame");
        }
        _ => println!("cpu        not measured (no /proc/self/stat on this host)"),
    }
    println!(
        "detections {detections} in {} frames, {frames_with_one} frames with at least one",
        frames.len()
    );
    Ok(())
}
