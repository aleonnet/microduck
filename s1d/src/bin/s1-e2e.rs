//! `s1-e2e` — phase 1 end to end, every item of the body through the duck's own mechanism.
//!
//! The mechanism is the one a duck is driven with: a gamepad's state as UDP datagrams
//! (`pad_map::wire`, what `duckctl udp-pad` sends) into `netpadd`, which turns them into
//! `robot.*` calls on the robot socket; `robotctl` and `s1ctl` on that same socket; and `s1d`
//! behind it putting DUML frames on the wire. This program plays the pad and the shells, in the
//! same order, in two settings:
//!
//! - **`fake`** — no robot. It starts `s1d --fake` (frames to a file, pushes from a file),
//!   `netpadd`, and runs the real `robotctl` and `s1ctl` binaries beside it; every item is then
//!   judged on the wire: the bytes the loop wrote, decoded and compared with what DJI's own
//!   library sends (`s1_duml::cmd`, oracle-tested) or with the real frames captured off the
//!   robot. This is the pre-test the bench runs before the robot is plugged in.
//! - **`robot`** — the robot's `s1d` and `netpadd` in the chroot, reached through
//!   `adb forward tcp:9000 … /run/robotd.sock` (`--tcp`) and the robot's own UDP port (`--udp`).
//!   The same items, the same order; the proof is the daemon's answer, the telemetry it reports
//!   and the person at the robot saying what moved (the witness, asked on a terminal).
//!
//! Exit status is the verdict: 0 when every item passed, 1 otherwise. The table it prints is what
//! the record `docs/project/s1-bringup.md` quotes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use pad_map::Buttons;
use pad_map::wire::Packet;
use s1_duml::cmd::{self, host, id};
use s1_duml::{Decoder, Frame, dds};
use s1d::tick::{DDS_MSG_ID, DDS_PUSH_LEN};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    version,
    about = "Phase 1 of the S1 end to end, through the duck's mechanism"
)]
struct Args {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// No robot: s1d --fake and netpadd started here, every item judged on the wire.
    Fake {
        /// Keep the working directory (logs, the wire file) instead of deleting it.
        #[arg(long)]
        keep: bool,
        /// Where to write the working files (default: a fresh directory under the temp dir).
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Only items whose name starts with one of these (e.g. `F2 F10 pad`).
        #[arg(long, num_args = 1..)]
        only: Vec<String>,
    },
    /// The robot's daemons: the socket through an adb forward, the pad over UDP to the robot.
    Robot {
        /// `host:port` of the forwarded robot socket.
        #[arg(long, default_value = "127.0.0.1:9000")]
        tcp: String,
        /// `host:port` of the robot's `netpadd` (its Wi-Fi address, `[netpad] port`).
        #[arg(long)]
        udp: String,
        /// Every call, answer, event and witness, appended here.
        #[arg(long)]
        log: PathBuf,
        /// Only items whose name starts with one of these.
        #[arg(long, num_args = 1..)]
        only: Vec<String>,
        /// Seconds to wait for armour hits, infrared and the button in the events item.
        #[arg(long, default_value_t = 30)]
        events_window: u64,
    },
}

fn main() -> ExitCode {
    let args = Args::parse();
    let outcome = match args.mode {
        Mode::Fake { keep, dir, only } => fake::run(keep, dir, &only),
        Mode::Robot {
            tcp,
            udp,
            log,
            only,
            events_window,
        } => robot::run(&tcp, &udp, &log, &only, events_window),
    };
    match outcome {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("s1-e2e: {e}");
            ExitCode::FAILURE
        }
    }
}

/// One line of the verdict table.
struct Row {
    item: String,
    expected: String,
    proof: String,
    ok: bool,
}

fn print_table(rows: &[Row]) -> bool {
    let w = rows.iter().map(|r| r.item.len()).max().unwrap_or(4).max(4);
    println!("\n{:<w$}  result  proof", "item", w = w);
    let mut all = true;
    for r in rows {
        all &= r.ok;
        println!(
            "{:<w$}  {}  {}  — expected: {}",
            r.item,
            if r.ok { "OK   " } else { "FAIL " },
            r.proof,
            r.expected,
            w = w
        );
    }
    let passed = rows.iter().filter(|r| r.ok).count();
    println!(
        "\n{passed} of {} items passed{}",
        rows.len(),
        if all { "" } else { " — FAILED" }
    );
    all
}

fn wanted(only: &[String], item: &str) -> bool {
    only.is_empty() || only.iter().any(|w| item.starts_with(w.as_str()))
}

// ---------------------------------------------------------------------------------------------
// The socket client: NDJSON, one request, one answer — what robotctl and s1ctl do.

trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

struct Client {
    reader: BufReader<Box<dyn Stream>>,
    writer: Box<dyn Stream>,
    next_id: u64,
    log: Option<std::fs::File>,
}

impl Client {
    fn unix(path: &Path) -> std::io::Result<Self> {
        let s = UnixStream::connect(path)?;
        s.set_read_timeout(Some(Duration::from_secs(5)))?;
        let w = s.try_clone()?;
        Ok(Self::new(Box::new(s), Box::new(w)))
    }

    fn tcp(addr: &str) -> std::io::Result<Self> {
        let s = TcpStream::connect(addr)?;
        s.set_read_timeout(Some(Duration::from_secs(5)))?;
        let w = s.try_clone()?;
        Ok(Self::new(Box::new(s), Box::new(w)))
    }

    fn new(reader: Box<dyn Stream>, writer: Box<dyn Stream>) -> Self {
        Self {
            reader: BufReader::new(reader),
            writer,
            next_id: 1,
            log: None,
        }
    }

    /// Send `method` with `params`; the `result` member of the answer (or the whole error).
    fn call(&mut self, method: &str, params: Value) -> std::io::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.writer, "{req}")?;
        self.writer.flush()?;
        let mut line = String::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line)? == 0 {
                return Err(std::io::Error::other("the daemon closed the connection"));
            }
            let v: Value = serde_json::from_str(line.trim()).unwrap_or(Value::Null);
            if v.get("id").and_then(Value::as_u64) != Some(id) {
                continue; // a notification (state, event) on this connection
            }
            if let Some(log) = self.log.as_mut() {
                let _ = writeln!(log, "{} {method} {params} -> {}", stamp(), line.trim());
            }
            return Ok(v.get("result").cloned().unwrap_or(v));
        }
    }

    /// Read notifications of `method` for up to `window`.
    fn notifications(&mut self, method: &str, window: Duration) -> Vec<Value> {
        let deadline = Instant::now() + window;
        let mut out = Vec::new();
        let mut line = String::new();
        while Instant::now() < deadline {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let v: Value = serde_json::from_str(line.trim()).unwrap_or(Value::Null);
                    if v.get("method").and_then(Value::as_str) == Some(method) {
                        if let Some(log) = self.log.as_mut() {
                            let _ = writeln!(log, "{} {}", stamp(), line.trim());
                        }
                        out.push(v["params"].clone());
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(_) => break,
            }
        }
        out
    }
}

/// A daemon's log without the colour escapes `tracing` writes even to a file.
fn plain(log: &str) -> String {
    let mut out = String::with_capacity(log.len());
    let mut chars = log.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn stamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", now.as_secs(), now.subsec_millis())
}

fn accepted(v: &Value) -> bool {
    v.get("accepted").and_then(Value::as_bool) == Some(true)
}

// ---------------------------------------------------------------------------------------------
// The pad: datagrams as `duckctl udp-pad` sends them.

struct Pad {
    socket: UdpSocket,
    seq: u32,
    started: Instant,
}

impl Pad {
    fn new(target: &str) -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.connect(target)?;
        Ok(Self {
            socket,
            seq: 0,
            started: Instant::now(),
        })
    }

    fn send(&mut self, sticks: [f64; 4], triggers: [f64; 2], held: Buttons) {
        self.seq += 1;
        let t_ms = self.started.elapsed().as_millis() as u32;
        let p = Packet::from_axes(self.seq, t_ms, sticks, triggers, held);
        let _ = self.socket.send(&p.encode());
    }

    /// Hold a state for `for_ms` at 30 Hz, the pad's rate.
    fn hold(&mut self, sticks: [f64; 4], triggers: [f64; 2], held: Buttons, for_ms: u64) {
        let end = Instant::now() + Duration::from_millis(for_ms);
        while Instant::now() < end {
            self.send(sticks, triggers, held);
            std::thread::sleep(Duration::from_millis(33));
        }
    }

    /// Press and release a button, idle sticks, with a quiet frame before and after so the edge is
    /// seen once.
    fn tap(&mut self, button: Buttons) {
        self.hold([0.0; 4], [0.0; 2], Buttons::NONE, 100);
        self.hold([0.0; 4], [0.0; 2], button, 120);
        self.hold([0.0; 4], [0.0; 2], Buttons::NONE, 200);
    }
}

// ---------------------------------------------------------------------------------------------
// The items, shared by both settings: what to do, and what a person must see on the robot.

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A frame as the loop would have written it, with the sequence number zeroed on both sides.
fn wire_hex(f: &Frame) -> String {
    let mut f = f.clone();
    f.seq = 0;
    hex(&f.encode().unwrap())
}

fn wheel_rpm(f: &Frame) -> [i16; 4] {
    let h = |i: usize| i16::from_le_bytes([f.data[i], f.data[i + 1]]);
    [h(0), h(2), h(4), h(6)]
}

/// A telemetry push of ours: battery 77 %, gimbal yaw +90.0° pitch −15.0°, picked up, x = 1.5 m,
/// yaw 45°, wheel 1 at 123 rpm — distinct values so the readback is unmistakable.
fn our_push(seq: u16) -> Frame {
    let mut data = vec![0u8, DDS_MSG_ID];
    let mut s = vec![0u8; DDS_PUSH_LEN];
    let mut at = dds::PUSH_TIMESTAMP_LEN;
    s[at..at + 2].copy_from_slice(&123i16.to_le_bytes()); // esc: wheel 1 rpm
    at += dds::Esc::SIZE;
    s[at..at + 4].copy_from_slice(&1.5f32.to_le_bytes()); // position x
    at += dds::Position::SIZE + dds::Velocity::SIZE + dds::Imu::SIZE;
    s[at..at + 4].copy_from_slice(&45.0f32.to_le_bytes()); // attitude yaw
    at += dds::Attitude::SIZE;
    s[at..at + 4].copy_from_slice(&(1u32 << 4).to_le_bytes()); // sa_status: picked up
    at += dds::SaStatus::SIZE;
    s[at + 2..at + 4].copy_from_slice(&(-150i16).to_le_bytes()); // gimbal pitch
    s[at + 4..at + 6].copy_from_slice(&900i16.to_le_bytes()); // gimbal yaw
    at += dds::GimbalPos::SIZE + dds::Stick::SIZE;
    s[at + 8] = 77; // battery percent
    data.extend_from_slice(&s);
    Frame::request(
        host::MOTION_CONTROLLER,
        host::BASE,
        seq,
        id::DDS_PUSH.0,
        id::DDS_PUSH.1,
        data,
    )
}

fn armor_hit_frame() -> Frame {
    Frame::request(3, 9, 0, 0x3f, 0x02, vec![0x31, 0x34, 0x12, 0x02, 0x00])
}

fn ir_hit_frame() -> Frame {
    Frame::request(3, 9, 0, 0x3f, 0x10, vec![0x21, 0x01, 0x03])
}

/// A DDS answer from the controller: `cmd_id` with `retcode` first, as the SDK unpacks it.
fn dds_answer(cmd_id: u8, data: Vec<u8>) -> Frame {
    let mut f = Frame::request(host::MOTION_CONTROLLER, host::BASE, 0, 0x48, cmd_id, data);
    f.is_ack = true;
    f
}

// ---------------------------------------------------------------------------------------------

mod fake {
    use super::*;

    struct Daemons {
        dir: PathBuf,
        s1d: Child,
        netpadd: Child,
        wire: PathBuf,
        inbound: PathBuf,
        socket: PathBuf,
        tool_log: PathBuf,
        s1d_log: PathBuf,
        button_fifo: PathBuf,
        pad_port: u16,
        bin: PathBuf,
    }

    impl Drop for Daemons {
        fn drop(&mut self) {
            let _ = self.netpadd.kill();
            let _ = self.netpadd.wait();
            let _ = self.s1d.kill();
            let _ = self.s1d.wait();
        }
    }

    /// 44-byte RIFF header and no samples: enough for `aplay`'s stand-in, which only records
    /// which file it was asked to play.
    fn write_wav(path: &Path) -> std::io::Result<()> {
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&36u32.to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&16000u32.to_le_bytes());
        b.extend_from_slice(&32000u32.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&0u32.to_le_bytes());
        std::fs::write(path, b)
    }

    /// The `[pad_roller]` of `deploy/s1/robotd.toml`, verbatim: a mecanum base strafes, so the
    /// prototype roller's "no strafe" default must not be what the pad test runs under.
    const PAD_ROLLER: &str = "[pad_roller]\nvx_min = -0.5\nvx_max = 0.6\nvy_min = -0.5\nvy_max = 0.5\nvyaw_min = -1.0\nvyaw_max = 1.0\n";

    fn free_udp_port() -> std::io::Result<u16> {
        Ok(UdpSocket::bind("127.0.0.1:0")?.local_addr()?.port())
    }

    fn start(dir: PathBuf) -> Result<Daemons, String> {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let bin = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .ok_or("cannot locate the binaries beside this one")?;
        for b in ["s1d", "s1ctl", "netpadd", "robotctl"] {
            if !bin.join(b).is_file() {
                return Err(format!(
                    "{} is missing: cargo build -p s1d -p netpadd -p robotctl",
                    bin.join(b).display()
                ));
            }
        }
        // The speaker and the mixer, stood in for: each records its argument line.
        let fakebin = dir.join("bin");
        std::fs::create_dir_all(&fakebin).map_err(|e| e.to_string())?;
        let tool_log = dir.join("tools.log");
        for tool in ["aplay", "amixer"] {
            let path = fakebin.join(tool);
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\necho \"{tool} $*\" >> \"{}\"\n",
                    tool_log.display()
                ),
            )
            .map_err(|e| e.to_string())?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
        let bank = dir.join("sounds");
        std::fs::create_dir_all(&bank).map_err(|e| e.to_string())?;
        for tag in ["alarm", "greet", "inquire", "peck", "chirp", "coo", "wheee"] {
            write_wav(&bank.join(format!("{tag}.wav"))).map_err(|e| e.to_string())?;
        }
        let button_fifo = dir.join("button.fifo");
        let st = Command::new("mkfifo")
            .arg(&button_fifo)
            .status()
            .map_err(|e| format!("mkfifo: {e}"))?;
        if !st.success() {
            return Err("mkfifo failed".to_owned());
        }
        let pad_port = free_udp_port().map_err(|e| e.to_string())?;
        let config = dir.join("robotd.toml");
        std::fs::write(
            &config,
            format!(
                "[netpad]\nenabled = true\nport = {pad_port}\nmax_hz = 30\nstale_ms = 300\n{PAD_ROLLER}"
            ),
        )
        .map_err(|e| e.to_string())?;

        let socket = dir.join("robotd.sock");
        let wire = dir.join("wire.bin");
        let inbound = dir.join("inbound.bin");
        let s1d_log = dir.join("s1d.log");
        let path_env = format!(
            "{}:{}",
            fakebin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let s1d = Command::new(bin.join("s1d"))
            .args(["--fake"])
            .arg(&wire)
            .arg("--fake-in")
            .arg(&inbound)
            .arg("--socket")
            .arg(&socket)
            .arg("--sounds")
            .arg(&bank)
            .args(["--volume-control", "Speaker", "--dds-hz", "20"])
            .arg("--button")
            .arg(&button_fifo)
            .env("PATH", &path_env)
            .env("RUST_LOG", "info")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&s1d_log).map_err(|e| e.to_string())?)
            .spawn()
            .map_err(|e| format!("s1d: {e}"))?;
        let mut d = Daemons {
            dir,
            s1d,
            netpadd: Command::new("true").spawn().map_err(|e| e.to_string())?,
            wire,
            inbound,
            socket: socket.clone(),
            tool_log,
            s1d_log,
            button_fifo,
            pad_port,
            bin: bin.clone(),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !d.socket.exists() || Client::unix(&d.socket).is_err() {
            if Instant::now() > deadline {
                return Err(format!(
                    "s1d did not open {} — {}",
                    d.socket.display(),
                    std::fs::read_to_string(&d.s1d_log).unwrap_or_default()
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let netpadd_log = d.dir.join("netpadd.log");
        d.netpadd = Command::new(bin.join("netpadd"))
            .arg("--socket")
            .arg(&socket)
            .arg("--config")
            .arg(&config)
            .env("RUST_LOG", "info")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&netpadd_log).map_err(|e| e.to_string())?)
            .spawn()
            .map_err(|e| format!("netpadd: {e}"))?;
        // Up when it says so (it asks robot.mode first, then binds the port).
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let log = std::fs::read_to_string(&netpadd_log).unwrap_or_default();
            if log.contains("listening for a pad over UDP") {
                break;
            }
            if Instant::now() > deadline || d.netpadd.try_wait().ok().flatten().is_some() {
                return Err(format!("netpadd did not come up — {log}"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(d)
    }

    /// Everything on the wire so far, decoded.
    struct Wire {
        path: PathBuf,
        seen: usize,
    }

    impl Wire {
        fn all(&self) -> Vec<Frame> {
            let bytes = std::fs::read(&self.path).unwrap_or_default();
            Decoder::new().push(&bytes)
        }

        /// Frames written since the last `mark`.
        fn since_mark(&mut self) -> Vec<Frame> {
            let all = self.all();
            let new = all[self.seen.min(all.len())..].to_vec();
            self.seen = all.len();
            new
        }

        fn mark(&mut self) {
            self.seen = self.all().len();
        }
    }

    fn feed(d: &Daemons, frames: &[Frame]) -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new().append(true).open(&d.inbound)?;
        for fr in frames {
            f.write_all(&fr.encode().unwrap())?;
        }
        f.flush()
    }

    fn settle() {
        std::thread::sleep(Duration::from_millis(250));
    }

    fn run_bin(d: &Daemons, name: &str, args: &[&str]) -> (bool, String) {
        let out = Command::new(d.bin.join(name))
            .args(args)
            .env("RUST_LOG", "warn")
            .output();
        match out {
            Ok(o) => (
                o.status.success(),
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                ),
            ),
            Err(e) => (false, e.to_string()),
        }
    }

    fn s1ctl(d: &Daemons, args: &[&str]) -> (bool, String) {
        let mut full = vec!["--socket", d.socket.to_str().unwrap()];
        full.extend_from_slice(args);
        let (ok, out) = run_bin(d, "s1ctl", &full);
        let v: Value = serde_json::from_str(out.trim()).unwrap_or(Value::Null);
        (ok && accepted(&v["result"]), out.trim().to_owned())
    }

    fn robotctl(d: &Daemons, args: &[&str]) -> (bool, String) {
        let mut full = vec!["--robot-socket", d.socket.to_str().unwrap()];
        full.extend_from_slice(args);
        run_bin(d, "robotctl", &full)
    }

    pub fn run(keep: bool, dir: Option<PathBuf>, only: &[String]) -> Result<bool, String> {
        let dir = dir
            .unwrap_or_else(|| std::env::temp_dir().join(format!("s1-e2e-{}", std::process::id())));
        let d = start(dir.clone())?;
        println!("s1-e2e fake: working in {}", d.dir.display());
        let mut rows: Vec<Row> = Vec::new();
        let mut wire = Wire {
            path: d.wire.clone(),
            seen: 0,
        };
        let mut client = Client::unix(&d.socket).map_err(|e| e.to_string())?;
        let mut pad = Pad::new(&format!("127.0.0.1:{}", d.pad_port)).map_err(|e| e.to_string())?;

        // A closure-free item runner: name, what a person sees on the robot, and a judge over
        // the frames written during the item.
        macro_rules! item {
            ($name:expr, $expected:expr, $act:block, $judge:expr) => {
                if wanted(only, $name) {
                    wire.mark();
                    // A closure, called once: the block uses `?` and `return Err` for its early
                    // exits, which a plain block cannot.
                    #[allow(clippy::redundant_closure_call)]
                    let act_result: Result<String, String> =
                        (|| -> Result<String, String> { $act })();
                    settle();
                    let frames = wire.since_mark();
                    let (ok, proof) = match act_result {
                        Ok(note) => {
                            let judge: &dyn Fn(&[Frame]) -> (bool, String) = &$judge;
                            let (ok, proof) = judge(&frames);
                            (
                                ok,
                                if note.is_empty() {
                                    proof
                                } else {
                                    format!("{note}; {proof}")
                                },
                            )
                        }
                        Err(why) => (false, why),
                    };
                    println!("[{}] {}", if ok { "OK" } else { "FAIL" }, $name);
                    rows.push(Row {
                        item: $name.to_owned(),
                        expected: $expected.to_owned(),
                        proof,
                        ok,
                    });
                }
            };
        }
        let count = |frames: &[Frame], cmd: (u8, u8)| {
            frames
                .iter()
                .filter(|f| (f.cmd_set, f.cmd_id) == cmd)
                .count()
        };
        let find_hex = |frames: &[Frame], want: &str| -> (bool, String) {
            let hit = frames.iter().any(|f| wire_hex(f) == want);
            (
                hit,
                if hit {
                    format!("wire has {want}")
                } else {
                    format!(
                        "wire lacks {want}; had {:?}",
                        frames
                            .iter()
                            .map(|f| format!("{:02x}/{:02x}", f.cmd_set, f.cmd_id))
                            .collect::<Vec<_>>()
                    )
                },
            )
        };

        // --- stage 5 through the pad ------------------------------------------------------
        item!(
            "health robotctl",
            "robotctl health --json reports the loop healthy at 100 Hz",
            {
                let (_, out) = robotctl(&d, &["health", "--json"]);
                let v: Value = serde_json::from_str(&out).map_err(|e| format!("{e}: {out}"))?;
                if v["robot"]["healthy"] != Value::Bool(true) {
                    return Err(format!("robot.healthy is not true: {}", v["robot"]));
                }
                Ok(format!(
                    "healthy at {} Hz",
                    v["robot"]["control_loop"]["achieved_hz"]
                ))
            },
            |_| (true, "answered".to_owned())
        );
        item!(
            "pad Start arms",
            "Start on the pad: the chassis arms (work mode 1, heartbeat streaming); nothing moves",
            {
                pad.tap(Buttons::START);
                Ok(String::new())
            },
            |f: &[Frame]| {
                let wm = f
                    .iter()
                    .filter(|x| (x.cmd_set, x.cmd_id) == id::CHASSIS_WORKMODE && x.data == [1])
                    .count();
                let hb = count(f, id::HEARTBEAT);
                (
                    wm == 5 && hb >= 10,
                    format!("{wm} work-mode-1 frames, {hb} heartbeats in the window"),
                )
            }
        );
        item!(
            "pad left stick forward",
            "left stick up: all four wheels drive forward (wheels in the air)",
            {
                pad.hold([0.0, 1.0, 0.0, 0.0], [0.0; 2], Buttons::NONE, 500);
                Ok(String::new())
            },
            |f: &[Frame]| {
                let moving: Vec<[i16; 4]> = f
                    .iter()
                    .filter(|x| (x.cmd_set, x.cmd_id) == id::CHASSIS_WHEEL_SPEED)
                    .map(wheel_rpm)
                    .filter(|w| w.iter().any(|&r| r != 0))
                    .collect();
                // On the wire the left wheels are negated (the PoC's convention): forward is
                // fr > 0, fl < 0, bl < 0, br > 0 with equal magnitude.
                let fwd = moving
                    .iter()
                    .any(|w| w[0] > 0 && w[1] < 0 && w[2] < 0 && w[3] > 0 && w[0] == -w[1]);
                (
                    fwd,
                    format!(
                        "{} moving wheel frames, e.g. {:?}",
                        moving.len(),
                        moving.first()
                    ),
                )
            }
        );
        item!(
            "pad right stick turns",
            "right stick left: the chassis turns on the spot",
            {
                pad.hold([0.0, 0.0, -1.0, 0.0], [0.0; 2], Buttons::NONE, 400);
                Ok(String::new())
            },
            |f: &[Frame]| {
                let turning = f
                    .iter()
                    .filter(|x| (x.cmd_set, x.cmd_id) == id::CHASSIS_WHEEL_SPEED)
                    .map(wheel_rpm)
                    .any(|w| w.iter().all(|&r| r != 0) && w[0] != -w[1]);
                (
                    turning,
                    "wheel frames with the two sides opposed".to_owned(),
                )
            }
        );
        item!(
            "pad left stick strafe",
            "left stick sideways: the wheels run the strafe pattern",
            {
                pad.hold([1.0, 0.0, 0.0, 0.0], [0.0; 2], Buttons::NONE, 400);
                Ok(String::new())
            },
            |f: &[Frame]| {
                let moving: Vec<[i16; 4]> = f
                    .iter()
                    .filter(|x| (x.cmd_set, x.cmd_id) == id::CHASSIS_WHEEL_SPEED)
                    .map(wheel_rpm)
                    .filter(|w| w.iter().any(|&r| r != 0))
                    .collect();
                let strafe = moving
                    .iter()
                    .any(|w| w.iter().all(|&r| r != 0) && (w[0] > 0) != (w[3] > 0));
                (
                    strafe,
                    format!(
                        "{} moving wheel frames, e.g. {:?}",
                        moving.len(),
                        moving.first()
                    ),
                )
            }
        );
        item!(
            "pad sticks released",
            "sticks centred: every wheel stops",
            {
                pad.hold([0.0; 4], [0.0; 2], Buttons::NONE, 400);
                Ok(String::new())
            },
            |f: &[Frame]| {
                let last: Vec<[i16; 4]> = f
                    .iter()
                    .filter(|x| (x.cmd_set, x.cmd_id) == id::CHASSIS_WHEEL_SPEED)
                    .map(wheel_rpm)
                    .collect();
                let tail = last.iter().rev().take(10).all(|w| *w == [0; 4]);
                (
                    tail && !last.is_empty(),
                    format!("last wheel frames zero: {tail}"),
                )
            }
        );
        item!(
            "pad D-pad right + right stick moves the gimbal",
            "D-pad right, then right stick: the gimbal yaws and pitches with the stick",
            {
                pad.tap(Buttons::RIGHT);
                pad.hold([0.0, 0.0, 1.0, 0.5], [0.0; 2], Buttons::NONE, 400);
                Ok(String::new())
            },
            |f: &[Frame]| {
                let moved = f
                    .iter()
                    .filter(|x| (x.cmd_set, x.cmd_id) == id::GIMBAL_ROTATE)
                    .any(|x| x.data.iter().any(|&b| b != 0));
                (
                    moved,
                    format!("{} gimbal frames", count(f, id::GIMBAL_ROTATE)),
                )
            }
        );
        item!(
            "pad right stick centred, back to drive mode",
            "the gimbal returns to centre; D-pad left",
            {
                pad.hold([0.0; 4], [0.0; 2], Buttons::NONE, 300);
                pad.tap(Buttons::LEFT);
                Ok(String::new())
            },
            |f: &[Frame]| (count(f, id::HEARTBEAT) > 0, "still armed".to_owned())
        );
        item!(
            "pad Home toggles the lights",
            "Home: every LED white; Home again: off",
            {
                pad.tap(Buttons::HOME);
                pad.tap(Buttons::HOME);
                Ok(String::new())
            },
            |f: &[Frame]| {
                let leds: Vec<&Frame> = f
                    .iter()
                    .filter(|x| (x.cmd_set, x.cmd_id) == id::SYSTEM_LED)
                    .collect();
                let on = leds
                    .iter()
                    .any(|x| x.data[0] == 1 && x.data[3..6] == [255, 255, 255]);
                let off = leds.iter().any(|x| x.data[0] == 0);
                (
                    on && off,
                    format!("{} LED frames: white solid {on}, off {off}", leds.len()),
                )
            }
        );
        item!(
            "pad RT quacks, LT rides",
            "right trigger: the chirp plays on the speaker; left trigger: the wheee",
            {
                pad.hold([0.0; 4], [0.0, 1.0], Buttons::NONE, 150);
                pad.hold([0.0; 4], [0.0, 0.0], Buttons::NONE, 150);
                pad.hold([0.0; 4], [1.0, 0.0], Buttons::NONE, 150);
                pad.hold([0.0; 4], [0.0, 0.0], Buttons::NONE, 150);
                settle();
                let log = std::fs::read_to_string(&d.tool_log).unwrap_or_default();
                let chirp = log.contains("chirp.wav");
                let wheee = log.contains("wheee.wav");
                if chirp && wheee {
                    Ok("aplay ran chirp.wav and wheee.wav".to_owned())
                } else {
                    Err(format!("aplay log: {log:?}"))
                }
            },
            |_| (true, String::new())
        );
        item!(
            "pad Start disarms",
            "Start again: the wheels zero and the heartbeat stops",
            {
                pad.tap(Buttons::START);
                std::thread::sleep(Duration::from_millis(300));
                wire.mark();
                std::thread::sleep(Duration::from_millis(300));
                Ok(String::new())
            },
            |f: &[Frame]| {
                let hb = count(f, id::HEARTBEAT);
                (
                    hb == 0,
                    format!("{hb} heartbeats in 300 ms after the disarm"),
                )
            }
        );

        // --- stage 5 through the shells -----------------------------------------------------
        item!(
            "s1ctl version",
            "the list's version answers",
            {
                let (_, out) = s1ctl(&d, &["version"]);
                let v: Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
                if v["result"]["s1_api_version"] != json!(s1d::api::S1_API_VERSION) {
                    return Err(out);
                }
                Ok(out)
            },
            |_| (true, String::new())
        );
        item!(
            "robot.enable arms (socket)",
            "armed by the socket, as robotctl's clients do",
            {
                let r = client
                    .call("robot.enable", json!({"on": true, "toggle": false}))
                    .map_err(|e| e.to_string())?;
                if !accepted(&r) {
                    return Err(r.to_string());
                }
                Ok(String::new())
            },
            |f: &[Frame]| (count(f, id::HEARTBEAT) > 5, "heartbeat on".to_owned())
        );
        item!(
            "F16 s1ctl wheels",
            "all four wheels forward at 100 rpm for the deadman's window",
            {
                let (ok, out) = s1ctl(&d, &["wheels", "100", "100", "100", "100"]);
                if !ok {
                    return Err(out);
                }
                std::thread::sleep(Duration::from_millis(150));
                let r = client
                    .call("robot.stop", json!({}))
                    .map_err(|e| e.to_string())?;
                if accepted(&r) {
                    Ok(String::new())
                } else {
                    Err(r.to_string())
                }
            },
            |f: &[Frame]| {
                let wheels: Vec<[i16; 4]> = f
                    .iter()
                    .filter(|x| (x.cmd_set, x.cmd_id) == id::CHASSIS_WHEEL_SPEED)
                    .map(wheel_rpm)
                    .collect();
                let hit = wheels.contains(&[100, -100, -100, 100]);
                let stopped = wheels.last() == Some(&[0; 4]);
                (
                    hit && stopped,
                    format!(
                        "wire wheel frame [100, -100, -100, 100] {hit}; zero after robot.stop {stopped}"
                    ),
                )
            }
        );
        item!(
            "F1 s1ctl follow gimbal_lead",
            "chassis follows the gimbal: a forward twist leaves as the follow command, not wheel speeds",
            {
                let (ok, out) = s1ctl(&d, &["follow", "gimbal_lead"]);
                if !ok {
                    return Err(out);
                }
                for _ in 0..10 {
                    client
                        .call("robot.move", json!({"vx": 0.2, "vy": 0.0, "vyaw": 0.0}))
                        .map_err(|e| e.to_string())?;
                    std::thread::sleep(Duration::from_millis(30));
                }
                Ok(String::new())
            },
            |f: &[Frame]| {
                let (a, pa) = find_hex(f, &wire_hex(&cmd::gimbal_workmode(0, 0)));
                let (b, pb) = find_hex(f, &wire_hex(&cmd::chassis_stick_overlay(0, 1)));
                let follows = count(f, id::CHASSIS_FOLLOW);
                let moving_wheels = f
                    .iter()
                    .filter(|x| (x.cmd_set, x.cmd_id) == id::CHASSIS_WHEEL_SPEED)
                    .map(wheel_rpm)
                    .filter(|w| *w != [0; 4])
                    .count();
                (
                    a && b && follows >= 5 && moving_wheels == 0,
                    format!(
                        "{pa}; {pb}; {follows} follow frames, {moving_wheels} moving wheel frames"
                    ),
                )
            }
        );
        item!(
            "F1 s1ctl follow chassis_lead",
            "gimbal follows the chassis (gimbal work mode 2)",
            {
                let (ok, out) = s1ctl(&d, &["follow", "chassis_lead"]);
                if ok { Ok(String::new()) } else { Err(out) }
            },
            |f: &[Frame]| find_hex(f, &wire_hex(&cmd::gimbal_workmode(0, 2)))
        );
        item!(
            "F7 s1ctl follow free --velocity-frame gimbal",
            "velocities read in the gimbal's heading (stick overlay 2)",
            {
                let (ok, out) = s1ctl(&d, &["follow", "free", "--velocity-frame", "gimbal"]);
                if ok { Ok(String::new()) } else { Err(out) }
            },
            |f: &[Frame]| find_hex(f, &wire_hex(&cmd::chassis_stick_overlay(0, 2)))
        );
        item!(
            "F4 s1ctl follow free --gimbal-fpv",
            "gimbal holds its heading relative to the body (work mode 1)",
            {
                let (ok, out) = s1ctl(&d, &["follow", "free", "--gimbal-fpv"]);
                if ok { Ok(String::new()) } else { Err(out) }
            },
            |f: &[Frame]| find_hex(f, &wire_hex(&cmd::gimbal_workmode(0, 1)))
        );
        item!(
            "F1 s1ctl follow free",
            "chassis and gimbal independent again",
            {
                let (ok, out) = s1ctl(&d, &["follow", "free"]);
                if ok { Ok(String::new()) } else { Err(out) }
            },
            |f: &[Frame]| find_hex(f, &wire_hex(&cmd::gimbal_workmode(0, 0)))
        );
        item!(
            "F2 s1ctl gimbal recenter",
            "the gimbal recentres at 100 °/s",
            {
                let (ok, out) = s1ctl(&d, &["gimbal", "recenter"]);
                if ok { Ok(String::new()) } else { Err(out) }
            },
            |f: &[Frame]| find_hex(f, "551604fcc9040000403fb2070805640000006400bfa0")
        );
        item!(
            "F3 s1ctl gimbal speed 30 -15, then stop",
            "the gimbal sweeps yaw right at 30 °/s and pitch down at 15 °/s, then stops",
            {
                let (ok, out) = s1ctl(&d, &["gimbal", "speed", "30", "-15"]);
                if !ok {
                    return Err(out);
                }
                std::thread::sleep(Duration::from_millis(100));
                let (ok, out) = s1ctl(&d, &["gimbal", "stop"]);
                if ok { Ok(String::new()) } else { Err(out) }
            },
            |f: &[Frame]| {
                let (a, pa) = find_hex(f, "5514046dc904000000040c2c0100006affdc8c4f");
                let (b, pb) = find_hex(f, &wire_hex(&cmd::gimbal_speed(0, 0, 0, 0)));
                (a && b, format!("{pa}; {pb}"))
            }
        );
        item!(
            "F5 s1ctl gimbal sleep, wake",
            "the gimbal folds down, then levels again",
            {
                let (ok, out) = s1ctl(&d, &["gimbal", "sleep"]);
                if !ok {
                    return Err(out);
                }
                std::thread::sleep(Duration::from_millis(100));
                let (ok, out) = s1ctl(&d, &["gimbal", "wake"]);
                if ok { Ok(String::new()) } else { Err(out) }
            },
            |f: &[Frame]| {
                let (a, pa) = find_hex(f, "550f04a2c904000020040db52ae825");
                let (b, pb) = find_hex(f, "550f04a2c904000020040df27e273a");
                (a && b, format!("{pa}; {pb}"))
            }
        );
        item!(
            "F6 s1ctl goto 1 -0.5 -90, then --stop",
            "the chassis runs 1 m forward, 0.5 m right, turns 90° clockwise and stops on its own; --stop abandons it",
            {
                let (ok, out) = s1ctl(&d, &["goto", "1", "-0.5", "-90"]);
                if !ok {
                    return Err(out);
                }
                std::thread::sleep(Duration::from_millis(100));
                let (ok, out) = s1ctl(&d, &["goto", "--stop"]);
                if ok { Ok(String::new()) } else { Err(out) }
            },
            |f: &[Frame]| {
                let (a, pa) = find_hex(f, "551a04b1c9c30000403f25070800006400ceff8403322c016440");
                let (b, pb) = find_hex(f, "55170438c9c30000403f2507010000000000000000406c");
                (a && b, format!("{pa}; {pb}"))
            }
        );
        item!(
            "F10 s1ctl lights: solid red, breath blue, flash green, marquee white, off",
            "every LED solid red, then breathing blue, flashing green, a white marquee, then off",
            {
                for args in [
                    vec!["lights", "solid", "255", "0", "0"],
                    vec![
                        "lights", "breath", "0", "0", "255", "--t1", "500", "--t2", "500",
                    ],
                    vec![
                        "lights", "flash", "0", "255", "0", "--t1", "250", "--t2", "250",
                    ],
                    vec![
                        "lights", "marquee", "255", "255", "255", "--t1", "100", "--t2", "100",
                    ],
                    vec!["lights", "off"],
                ] {
                    let (ok, out) = s1ctl(&d, &args);
                    if !ok {
                        return Err(out);
                    }
                    std::thread::sleep(Duration::from_millis(60));
                }
                Ok(String::new())
            },
            |f: &[Frame]| {
                let (a, pa) = find_hex(f, "551a04b109180000003f3201ff00ff000000000000003f0084d3");
                let (b, pb) = find_hex(f, "551a04b109180000003f3202ff000000ff00f401f4013f00bc8b");
                let effects: Vec<u8> = f
                    .iter()
                    .filter(|x| (x.cmd_set, x.cmd_id) == id::SYSTEM_LED)
                    .map(|x| x.data[0])
                    .collect();
                (
                    a && b && effects == [1, 2, 3, 4, 0],
                    format!("{pa}; {pb}; effects on the wire {effects:?}"),
                )
            }
        );
        item!(
            "F9 s1ctl lights --blaster solid white",
            "the blaster's own LED lights white",
            {
                let (ok, out) = s1ctl(
                    &d,
                    &[
                        "lights",
                        "solid",
                        "255",
                        "255",
                        "255",
                        "--t1",
                        "100",
                        "--t2",
                        "100",
                        "--blaster",
                    ],
                );
                if ok { Ok(String::new()) } else { Err(out) }
            },
            |f: &[Frame]| find_hex(f, "551604fcc9170000403f5510ffffff0164006400767c")
        );
        item!(
            "robotctl flashlight on red, off",
            "every LED solid red through the duck's own command, then off",
            {
                let (ok, out) = robotctl(&d, &["flashlight", "on", "--color", "red"]);
                if !ok {
                    return Err(out);
                }
                std::thread::sleep(Duration::from_millis(60));
                let (ok, out) = robotctl(&d, &["flashlight", "off"]);
                if ok { Ok(String::new()) } else { Err(out) }
            },
            |f: &[Frame]| {
                let (a, pa) = find_hex(f, "551a04b109180000003f3201ff00ff000000000000003f0084d3");
                let off = f
                    .iter()
                    .any(|x| (x.cmd_set, x.cmd_id) == id::SYSTEM_LED && x.data[0] == 0);
                (a && off, format!("{pa}; off frame {off}"))
            }
        );
        item!(
            "F11 s1ctl armor 1.0",
            "hit thresholds at the factory values (no visible change; the armour host answers)",
            {
                let (ok, out) = s1ctl(&d, &["armor", "1.0"]);
                if ok { Ok(String::new()) } else { Err(out) }
            },
            |f: &[Frame]| {
                let want = cmd::armor_sensitivity(0, host::LED, 1.0);
                let hit = f.iter().any(|x| {
                    x.receiver == host::LED
                        && (x.cmd_set, x.cmd_id) == id::ARMOR_SENSITIVITY
                        && x.data == want.data
                });
                (
                    hit,
                    format!(
                        "armour frame to host 0x18 with the library's {} bytes",
                        want.data.len()
                    ),
                )
            }
        );
        item!(
            "F12 s1ctl pwm --duty 500 --hz 1000",
            "the six PWM ports at 1 kHz, 50 % (a scope would show it)",
            {
                let (ok, out) = s1ctl(&d, &["pwm", "--duty", "500", "--hz", "1000"]);
                if ok { Ok(String::new()) } else { Err(out) }
            },
            |f: &[Frame]| {
                let (a, pa) = find_hex(f, "551a04b1c9c30000403f2b3fe803e803e803e803e803e803ae19");
                let (b, pb) = find_hex(f, "551a04b1c9c30000403f3c3ff401f401f401f401f401f401e052");
                (a && b, format!("{pa}; {pb}"))
            }
        );
        item!(
            "F17 s1ctl blaster (infrared), --gel refused",
            "the infrared emitter fires once; gel is refused without the key",
            {
                let (ok, out) = s1ctl(&d, &["blaster"]);
                if !ok {
                    return Err(out);
                }
                let (ok, out) = s1ctl(&d, &["blaster", "--gel"]);
                if ok {
                    return Err(format!("gel was accepted without the key: {out}"));
                }
                if !out.contains("--blaster-gel") {
                    return Err(out);
                }
                Ok("gel refused naming the key".to_owned())
            },
            |f: &[Frame]| {
                let hit = f.iter().any(|x| {
                    x.receiver == host::GUN
                        && (x.cmd_set, x.cmd_id) == id::GUN_FIRE
                        && x.data == [0x11]
                });
                (
                    hit,
                    "fire frame to the blaster host, single infrared".to_owned(),
                )
            }
        );
        item!(
            "s1ctl volume 50",
            "the speaker's mixer control set to 50 %",
            {
                let (ok, out) = s1ctl(&d, &["volume", "50"]);
                if !ok {
                    return Err(out);
                }
                let log = std::fs::read_to_string(&d.tool_log).unwrap_or_default();
                if log.contains("amixer -q -c 0 sset Speaker 50%") {
                    Ok("amixer -q -c 0 sset Speaker 50%".to_owned())
                } else {
                    Err(format!("amixer not run as expected: {log:?}"))
                }
            },
            |_| (true, String::new())
        );
        item!(
            "robot.sound greet (socket), robotctl quack",
            "the speaker plays the greeting, then the chirp",
            {
                let before = std::fs::read_to_string(&d.tool_log)
                    .unwrap_or_default()
                    .len();
                let r = client
                    .call("robot.sound", json!({"tag": "greet"}))
                    .map_err(|e| e.to_string())?;
                if !accepted(&r) {
                    return Err(r.to_string());
                }
                // The next sound cuts the previous one (as on the duck); the stand-in needs a
                // moment to have recorded its line before it is cut.
                settle();
                let (ok, out) = robotctl(&d, &["quack"]);
                if !ok {
                    return Err(out);
                }
                settle();
                let log = std::fs::read_to_string(&d.tool_log).unwrap_or_default();
                let log = &log[before.min(log.len())..];
                let greet = log.contains("greet.wav");
                let chirp = log.contains("chirp.wav");
                if greet && chirp {
                    Ok("aplay ran greet.wav and chirp.wav".to_owned())
                } else {
                    Err(format!("aplay log: {log:?}"))
                }
            },
            |_| (true, String::new())
        );
        item!(
            "s1ctl frame (raw)",
            "one raw frame reaches the wire as given",
            {
                let (ok, out) = s1ctl(&d, &["frame", "550f04a2c904000040044c0200cf21"]);
                if ok { Ok(String::new()) } else { Err(out) }
            },
            |f: &[Frame]| find_hex(f, "550f04a2c904000040044c0200cf21")
        );

        // --- telemetry, events, the button ----------------------------------------------
        item!(
            "F18 telemetry subscription on the wire",
            "s1d registers node 9 with the stock's exact bytes and subscribes nine subjects under msg 0x5d",
            { Ok(String::new()) },
            |_: &[Frame]| {
                let all = wire.all();
                const ADD_NODE_REAL: &[u8] =
                    include_bytes!("../../../s1-duml/tests/fixtures/dds_add_node_stock_real.bin");
                let node = all.iter().any(|x| x.encode().unwrap() == ADD_NODE_REAL);
                let sub = all
                    .iter()
                    .find(|x| (x.cmd_set, x.cmd_id) == id::DDS_ADD_SUB);
                let sub_ok = sub.is_some_and(|x| {
                    x.data[0] == host::BASE
                        && x.data[1] == DDS_MSG_ID
                        && x.data[2] == 0x03
                        && x.data[4] == 9
                });
                let reset = all
                    .iter()
                    .any(|x| (x.cmd_set, x.cmd_id) == id::DDS_RESET_NODE);
                (
                    node && sub_ok && reset,
                    format!(
                        "add-node byte-identical to the stock's {node}; reset {reset}; subscribe node 9 msg 0x5d flags 3 nine subjects {sub_ok}"
                    ),
                )
            }
        );
        item!(
            "F18 telemetry push decoded and acked",
            "a push of ours lands in s1.telemetry (battery 77 %, gimbal 90/−15, picked up) and is acked like the stock does",
            {
                feed(&d, &[dds_answer(1, vec![0x00, 0x03]), our_push(0x1234)])
                    .map_err(|e| e.to_string())?;
                settle();
                let (ok, out) = s1ctl(&d, &["telemetry"]);
                let v: Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
                let r = &v["result"];
                let good = r["pushes"] == json!(1)
                    && r["battery"]["percent"] == json!(77)
                    && r["gimbal_deg"] == json!([90.0, -15.0])
                    && r["status"]["picked_up"] == json!(true)
                    && r["position_m"][0] == json!(1.5)
                    && r["wheels_rpm"][0] == json!(123);
                if !(ok || r.is_object()) || !good {
                    return Err(out);
                }
                let log = plain(&std::fs::read_to_string(&d.s1d_log).unwrap_or_default());
                if !log.contains("meaning=\"accepted\"") {
                    return Err("the DDS answer 0x00 was not logged as accepted".to_owned());
                }
                Ok("telemetry = battery 77, gimbal [90,-15], picked_up, x 1.5, wheel 123; answer logged accepted".to_owned())
            },
            |f: &[Frame]| find_hex(f, &wire_hex(&dds::push_ack(0x1234)))
        );
        item!(
            "robot.state carries the gimbal from telemetry (robotctl monitor)",
            "robot.state head = the gimbal's reported angle, odometry x 1.5, picked_up",
            {
                let err_path = d.dir.join("monitor.err");
                let mut child = Command::new(d.bin.join("robotctl"))
                    .args([
                        "--robot-socket",
                        d.socket.to_str().unwrap(),
                        "monitor",
                        "--json",
                        "--hz",
                        "10",
                    ])
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(std::fs::File::create(&err_path).map_err(|e| e.to_string())?)
                    .spawn()
                    .map_err(|e| e.to_string())?;
                let stdout = child.stdout.take().unwrap();
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                });
                let deadline = Instant::now() + Duration::from_secs(3);
                let mut found = None;
                while Instant::now() < deadline {
                    match rx.recv_timeout(Duration::from_millis(200)) {
                        Ok(line) => {
                            // `--json` prints the notification whole: the state is its `params`.
                            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                                let state = v.get("params").cloned().unwrap_or(v);
                                if state.get("head").is_some() {
                                    found = Some(state);
                                    break;
                                }
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(_) => break,
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
                let v = found.ok_or_else(|| {
                    format!(
                        "robotctl monitor printed no state in 3 s; stderr: {}",
                        std::fs::read_to_string(&err_path)
                            .unwrap_or_default()
                            .trim()
                    )
                })?;
                let yaw = v["head"][2].as_f64().unwrap_or(0.0);
                let x = v["odom"]["position"][0].as_f64().unwrap_or(0.0);
                let picked = v["safety"]["picked_up"] == json!(true);
                if (yaw - 90f64.to_radians()).abs() < 1e-3 && (x - 1.5).abs() < 1e-6 && picked {
                    Ok(format!("head yaw {yaw:.4} rad, odom x {x}, picked_up"))
                } else {
                    Err(format!("state: {v}"))
                }
            },
            |_| (true, String::new())
        );
        item!(
            "events: armour hit, infrared hit, button (s1ctl events)",
            "a tap on plate 3, an infrared hit and the controller button each arrive as s1.event",
            {
                let mut child = Command::new(d.bin.join("s1ctl"))
                    .args(["--socket", d.socket.to_str().unwrap(), "events"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| e.to_string())?;
                let stdout = child.stdout.take().unwrap();
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                });
                // The subscription's own answer comes first.
                let _ = rx.recv_timeout(Duration::from_secs(2));
                feed(&d, &[armor_hit_frame(), ir_hit_frame()]).map_err(|e| e.to_string())?;
                // The button: one press, one release, as evdev writes them on this host.
                {
                    let mut fifo = std::fs::OpenOptions::new()
                        .write(true)
                        .open(&d.button_fifo)
                        .map_err(|e| format!("button fifo: {e}"))?;
                    let n = s1d::button::EVENT_LEN;
                    let time = n - 8;
                    for value in [1i32, 0] {
                        let mut raw = vec![0u8; n];
                        raw[time..time + 2].copy_from_slice(&1u16.to_ne_bytes());
                        raw[time + 2..time + 4].copy_from_slice(&0x74u16.to_ne_bytes());
                        raw[time + 4..].copy_from_slice(&value.to_ne_bytes());
                        fifo.write_all(&raw).map_err(|e| e.to_string())?;
                    }
                    fifo.flush().map_err(|e| e.to_string())?;
                }
                let deadline = Instant::now() + Duration::from_secs(3);
                let mut kinds = Vec::new();
                while Instant::now() < deadline && kinds.len() < 3 {
                    if let Ok(line) = rx.recv_timeout(Duration::from_millis(200))
                        && let Ok(v) = serde_json::from_str::<Value>(&line)
                        && v["method"] == json!("s1.event")
                    {
                        kinds.push(v["params"].clone());
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
                let has = |kind: &str| kinds.iter().any(|k| k["kind"] == json!(kind));
                let plate = kinds
                    .iter()
                    .find(|k| k["kind"] == json!("armor_hit"))
                    .map(|k| k["plate"].clone());
                if has("armor_hit") && has("ir_hit") && has("button") && plate == Some(json!(3)) {
                    Ok(format!("events: {kinds:?}"))
                } else {
                    Err(format!("events seen: {kinds:?}"))
                }
            },
            |_| (true, String::new())
        );
        item!(
            "robot.enable off disarms (socket)",
            "the wheels zero ten times and the heartbeat stops",
            {
                let r = client
                    .call("robot.enable", json!({"on": false, "toggle": false}))
                    .map_err(|e| e.to_string())?;
                if !accepted(&r) {
                    return Err(r.to_string());
                }
                std::thread::sleep(Duration::from_millis(100));
                wire.mark();
                std::thread::sleep(Duration::from_millis(300));
                Ok(String::new())
            },
            |f: &[Frame]| {
                let hb = count(f, id::HEARTBEAT);
                (hb == 0, format!("{hb} heartbeats after the disarm"))
            }
        );
        item!(
            "wire integrity",
            "everything the loop wrote decodes as whole frames",
            {
                let bytes = std::fs::read(&d.wire).map_err(|e| e.to_string())?;
                let mut dec = Decoder::new();
                let n = dec.push(&bytes).len();
                if dec.dropped() == 0 {
                    Ok(format!("{n} frames, 0 bytes dropped"))
                } else {
                    Err(format!("{} bytes dropped by the decoder", dec.dropped()))
                }
            },
            |_| (true, String::new())
        );

        let all = print_table(&rows);
        let report = d.dir.join("report.txt");
        let mut text = String::new();
        for r in &rows {
            text.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                if r.ok { "OK" } else { "FAIL" },
                r.item,
                r.proof,
                r.expected
            ));
        }
        let _ = std::fs::write(&report, text);
        println!("report: {}", report.display());
        drop(client);
        let dir = d.dir.clone();
        drop(d);
        if keep {
            println!("kept {}", dir.display());
        } else if all {
            let _ = std::fs::remove_dir_all(&dir);
        } else {
            println!("kept {} (failures)", dir.display());
        }
        Ok(all)
    }
}

// ---------------------------------------------------------------------------------------------

mod robot {
    use super::*;

    fn witness(log: &mut std::fs::File, item: &str, expected: &str) -> String {
        println!("\n[{item}] expected: {expected}");
        let seen = if atty_stdin() {
            print!("  what happened? (enter = as expected) ");
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            let t = line.trim();
            if t.is_empty() {
                "as expected".to_owned()
            } else {
                t.to_owned()
            }
        } else {
            "(no terminal: witness asked in the chat)".to_owned()
        };
        let _ = writeln!(log, "{} WITNESS {item}: {seen}", stamp());
        seen
    }

    fn atty_stdin() -> bool {
        use std::io::IsTerminal;
        std::io::stdin().is_terminal()
    }

    pub fn run(
        tcp: &str,
        udp: &str,
        log_path: &Path,
        only: &[String],
        events_window: u64,
    ) -> Result<bool, String> {
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .map_err(|e| format!("{}: {e}", log_path.display()))?;
        let mut client = Client::tcp(tcp).map_err(|e| format!("robot socket {tcp}: {e}"))?;
        client.log = Some(log.try_clone().map_err(|e| e.to_string())?);
        let mut pad = Pad::new(udp).map_err(|e| format!("pad to {udp}: {e}"))?;
        let mut rows: Vec<Row> = Vec::new();

        macro_rules! item {
            ($name:expr, $expected:expr, $act:block) => {
                if wanted(only, $name) {
                    #[allow(clippy::redundant_closure_call)]
                    let r: Result<String, String> = (|| -> Result<String, String> { $act })();
                    let seen = witness(&mut log, $name, $expected);
                    let (ok, proof) = match r {
                        Ok(p) => (
                            !seen.to_lowercase().contains("no") || seen == "as expected",
                            format!("{p}; witness: {seen}"),
                        ),
                        Err(e) => (false, e),
                    };
                    rows.push(Row {
                        item: $name.to_owned(),
                        expected: $expected.to_owned(),
                        proof,
                        ok,
                    });
                }
            };
        }
        let call = |c: &mut Client, m: &str, p: Value| -> Result<Value, String> {
            let r = c.call(m, p).map_err(|e| e.to_string())?;
            if r.get("accepted").is_some() && !accepted(&r) {
                return Err(format!("{m} refused: {r}"));
            }
            Ok(r)
        };

        item!("health", "the daemon reports the loop healthy at 100 Hz", {
            let r = call(&mut client, "robot.health", json!({}))?;
            if r["healthy"] != json!(true) {
                return Err(format!("not healthy: {r}"));
            }
            Ok(format!(
                "healthy at {} Hz",
                r["control_loop"]["achieved_hz"]
            ))
        });
        println!("\nWHEELS OFF THE GROUND from here on.");
        item!(
            "pad Start arms",
            "Start on the pad: the chassis arms; nothing moves",
            {
                pad.tap(Buttons::START);
                Ok("Start tapped (robot.init)".to_owned())
            }
        );
        item!(
            "pad left stick forward",
            "left stick up: all four wheels forward for 2 s",
            {
                pad.hold([0.0, 1.0, 0.0, 0.0], [0.0; 2], Buttons::NONE, 2000);
                pad.hold([0.0; 4], [0.0; 2], Buttons::NONE, 300);
                Ok("stick held 2 s".to_owned())
            }
        );
        item!(
            "pad right stick turns",
            "right stick left: the chassis turns on the spot for 2 s",
            {
                pad.hold([0.0, 0.0, -1.0, 0.0], [0.0; 2], Buttons::NONE, 2000);
                pad.hold([0.0; 4], [0.0; 2], Buttons::NONE, 300);
                Ok("stick held 2 s".to_owned())
            }
        );
        item!(
            "pad left stick strafe",
            "left stick left: the strafe pattern for 2 s",
            {
                pad.hold([1.0, 0.0, 0.0, 0.0], [0.0; 2], Buttons::NONE, 2000);
                pad.hold([0.0; 4], [0.0; 2], Buttons::NONE, 300);
                Ok("stick held 2 s".to_owned())
            }
        );
        item!(
            "pad D-pad right + right stick moves the gimbal",
            "D-pad right, then right stick: the gimbal yaws right and pitches up, then returns",
            {
                pad.tap(Buttons::RIGHT);
                pad.hold([0.0, 0.0, 1.0, 0.5], [0.0; 2], Buttons::NONE, 1500);
                pad.hold([0.0; 4], [0.0; 2], Buttons::NONE, 1000);
                pad.tap(Buttons::LEFT);
                Ok("stick held 1.5 s, D-pad left".to_owned())
            }
        );
        item!(
            "pad Home toggles the lights",
            "Home: every LED white; 2 s later Home again: off",
            {
                pad.tap(Buttons::HOME);
                std::thread::sleep(Duration::from_secs(2));
                pad.tap(Buttons::HOME);
                Ok("Home tapped twice".to_owned())
            }
        );
        item!(
            "pad RT quacks, LT rides",
            "right trigger: a sound; left trigger: another",
            {
                pad.hold([0.0; 4], [0.0, 1.0], Buttons::NONE, 200);
                pad.hold([0.0; 4], [0.0, 0.0], Buttons::NONE, 800);
                pad.hold([0.0; 4], [1.0, 0.0], Buttons::NONE, 200);
                pad.hold([0.0; 4], [0.0, 0.0], Buttons::NONE, 300);
                Ok("triggers pulled".to_owned())
            }
        );
        item!(
            "pad Start disarms",
            "Start again: wheels stop, chassis disarmed",
            {
                pad.tap(Buttons::START);
                Ok("Start tapped (robot.enable toggle)".to_owned())
            }
        );

        item!("s1 version", "the list's version answers", {
            let r = call(&mut client, "s1.version", json!({}))?;
            Ok(r.to_string())
        });
        item!("robot.enable arms (socket)", "armed; nothing moves", {
            call(
                &mut client,
                "robot.enable",
                json!({"on": true, "toggle": false}),
            )?;
            Ok("armed".to_owned())
        });
        item!(
            "F16 wheels 100 rpm",
            "all four wheels forward at 100 rpm for 2 s",
            {
                for _ in 0..20 {
                    call(
                        &mut client,
                        "s1.wheels",
                        json!({"rpm": [100, 100, 100, 100]}),
                    )?;
                    std::thread::sleep(Duration::from_millis(100));
                }
                call(&mut client, "robot.stop", json!({}))?;
                Ok("20 wheel calls".to_owned())
            }
        );
        item!(
            "F1 follow gimbal_lead",
            "turn the gimbal by hand 45°: the chassis turns after it; then free",
            {
                call(&mut client, "s1.follow", json!({"mode": "gimbal_lead"}))?;
                for _ in 0..30 {
                    call(
                        &mut client,
                        "robot.move",
                        json!({"vx": 0.0, "vy": 0.0, "vyaw": 0.0}),
                    )?;
                    std::thread::sleep(Duration::from_millis(100));
                }
                Ok("gimbal_lead held 3 s".to_owned())
            }
        );
        item!(
            "F1 follow chassis_lead",
            "the gimbal follows the chassis as it turns (turn by vyaw 2 s)",
            {
                call(&mut client, "s1.follow", json!({"mode": "chassis_lead"}))?;
                for _ in 0..20 {
                    call(
                        &mut client,
                        "robot.move",
                        json!({"vx": 0.0, "vy": 0.0, "vyaw": 0.6}),
                    )?;
                    std::thread::sleep(Duration::from_millis(100));
                }
                call(&mut client, "robot.stop", json!({}))?;
                call(&mut client, "s1.follow", json!({"mode": "free"}))?;
                Ok("chassis_lead, turned 2 s, free".to_owned())
            }
        );
        item!(
            "F7 velocity frame gimbal",
            "no visible change; answers only",
            {
                call(
                    &mut client,
                    "s1.follow",
                    json!({"mode": "free", "velocity_frame": "gimbal"}),
                )?;
                call(&mut client, "s1.follow", json!({"mode": "free"}))?;
                Ok("accepted".to_owned())
            }
        );
        item!(
            "F4 gimbal FPV",
            "the gimbal holds its heading relative to the body",
            {
                call(
                    &mut client,
                    "s1.follow",
                    json!({"mode": "free", "gimbal_fpv": true}),
                )?;
                std::thread::sleep(Duration::from_secs(1));
                call(&mut client, "s1.follow", json!({"mode": "free"}))?;
                Ok("accepted".to_owned())
            }
        );
        item!("F2 gimbal recenter", "the gimbal recentres", {
            call(
                &mut client,
                "s1.gimbal",
                json!({"action": "recenter", "yaw_speed": 100, "pitch_speed": 100}),
            )?;
            std::thread::sleep(Duration::from_secs(2));
            Ok("accepted".to_owned())
        });
        item!(
            "F3 gimbal speed",
            "the gimbal sweeps yaw right at 30 °/s and pitch down 15 °/s for 1 s, stops, recentres",
            {
                call(
                    &mut client,
                    "s1.gimbal",
                    json!({"action": "speed", "yaw": 30.0, "pitch": -15.0}),
                )?;
                std::thread::sleep(Duration::from_secs(1));
                call(&mut client, "s1.gimbal", json!({"action": "stop"}))?;
                std::thread::sleep(Duration::from_millis(500));
                call(
                    &mut client,
                    "s1.gimbal",
                    json!({"action": "recenter", "yaw_speed": 100, "pitch_speed": 100}),
                )?;
                std::thread::sleep(Duration::from_secs(2));
                Ok("accepted".to_owned())
            }
        );
        item!(
            "F5 gimbal sleep, wake",
            "the gimbal folds down for 3 s, then levels",
            {
                call(&mut client, "s1.gimbal", json!({"action": "sleep"}))?;
                std::thread::sleep(Duration::from_secs(3));
                call(&mut client, "s1.gimbal", json!({"action": "wake"}))?;
                std::thread::sleep(Duration::from_secs(2));
                Ok("accepted".to_owned())
            }
        );
        item!(
            "F6 goto",
            "wheels run the pattern of 1 m forward, 0.5 m right, 90° clockwise, then stop on their own",
            {
                call(
                    &mut client,
                    "s1.goto",
                    json!({"x": 1.0, "y": -0.5, "yaw": -90.0, "speed": 0.5, "yaw_speed": 30.0}),
                )?;
                std::thread::sleep(Duration::from_secs(6));
                call(&mut client, "s1.goto", json!({"stop": true}))?;
                Ok("accepted".to_owned())
            }
        );
        item!(
            "F10 lights: solid red, breath blue, flash green, marquee white, off",
            "every LED solid red, then breathing blue, flashing green, a white marquee, then off (2.5 s each)",
            {
                for (e, r, g, b, t1, t2) in [
                    ("solid", 255, 0, 0, 0, 0),
                    ("breath", 0, 0, 255, 500, 500),
                    ("flash", 0, 255, 0, 250, 250),
                    ("marquee", 255, 255, 255, 100, 100),
                    ("off", 0, 0, 0, 0, 0),
                ] {
                    call(
                        &mut client,
                        "s1.lights",
                        json!({"effect": e, "r": r, "g": g, "b": b, "t1": t1, "t2": t2}),
                    )?;
                    std::thread::sleep(Duration::from_millis(2500));
                }
                Ok("accepted".to_owned())
            }
        );
        item!(
            "F9 blaster LED white",
            "the blaster's LED lights white for 2 s",
            {
                call(
                    &mut client,
                    "s1.lights",
                    json!({"effect": "solid", "r": 255, "g": 255, "b": 255, "t1": 100, "t2": 100, "blaster": true}),
                )?;
                std::thread::sleep(Duration::from_secs(2));
                call(
                    &mut client,
                    "s1.lights",
                    json!({"effect": "off", "blaster": true}),
                )?;
                Ok("accepted".to_owned())
            }
        );
        item!(
            "robot.flashlight red, off",
            "every LED red through the duck's command for 2 s, then off",
            {
                call(
                    &mut client,
                    "robot.flashlight",
                    json!({"on": true, "toggle": false, "color": "red"}),
                )?;
                std::thread::sleep(Duration::from_secs(2));
                call(
                    &mut client,
                    "robot.flashlight",
                    json!({"on": false, "toggle": false, "color": "red"}),
                )?;
                Ok("accepted".to_owned())
            }
        );
        item!("F11 armor 1.0", "no visible change; the answer", {
            call(&mut client, "s1.armor", json!({"k": 1.0}))?;
            Ok("accepted".to_owned())
        });
        item!(
            "F12 pwm",
            "a scope on a PWM port shows 1 kHz at 50 %; answers only here",
            {
                call(&mut client, "s1.pwm", json!({"duty": 500, "hz": 1000}))?;
                Ok("accepted".to_owned())
            }
        );
        item!(
            "F17 blaster infrared",
            "the infrared emitter fires once (no gel: tank out)",
            {
                call(&mut client, "s1.blaster", json!({"times": 1}))?;
                let r = client
                    .call("s1.blaster", json!({"times": 1, "gel": true}))
                    .map_err(|e| e.to_string())?;
                if accepted(&r) {
                    return Err("gel accepted without the key".to_owned());
                }
                Ok("infrared accepted, gel refused".to_owned())
            }
        );
        item!(
            "s1 volume 50",
            "the speaker at 50 % (needs --volume-control on the daemon)",
            {
                let r = client
                    .call("s1.volume", json!({"percent": 50}))
                    .map_err(|e| e.to_string())?;
                Ok(r.to_string())
            }
        );
        item!(
            "robot.sound greet, chirp",
            "the speaker plays two sounds",
            {
                call(&mut client, "robot.sound", json!({"tag": "greet"}))?;
                std::thread::sleep(Duration::from_secs(2));
                call(&mut client, "robot.sound", json!({"tag": "chirp"}))?;
                std::thread::sleep(Duration::from_secs(2));
                Ok("accepted".to_owned())
            }
        );
        item!(
            "F18 telemetry",
            "pushes > 0 with a plausible battery %, attitude and gimbal angle",
            {
                std::thread::sleep(Duration::from_secs(6));
                let r = call(&mut client, "s1.telemetry", json!({}))?;
                let pushes = r["pushes"].as_u64().unwrap_or(0);
                if pushes == 0 {
                    return Err(format!("no pushes: {r}"));
                }
                Ok(r.to_string())
            }
        );
        item!(
            "robot.state (subscribe once)",
            "one state frame with the gimbal angle in head and odometry",
            {
                let r = client
                    .call("robot.subscribe", json!({"hz": 5}))
                    .map_err(|e| e.to_string())?;
                let frames = client.notifications("robot.state", Duration::from_secs(2));
                match frames.first() {
                    Some(f) => Ok(format!("head {} odom {} ({r})", f["head"], f["odom"])),
                    None => Err("no robot.state in 2 s".to_owned()),
                }
            }
        );
        item!(
            "events: armour, infrared, button",
            "tap each armour plate, aim an infrared source, press the button; each arrives as s1.event",
            {
                call(&mut client, "s1.subscribe", json!({"events": true}))?;
                println!("  {events_window} s window: TAP the plates, PRESS the button now");
                let events = client.notifications("s1.event", Duration::from_secs(events_window));
                call(&mut client, "s1.subscribe", json!({"events": false}))?;
                if events.is_empty() {
                    return Err("no event arrived in the window".to_owned());
                }
                Ok(format!("{} events: {events:?}", events.len()))
            }
        );
        item!("robot.enable off", "disarmed; wheels zero", {
            call(
                &mut client,
                "robot.enable",
                json!({"on": false, "toggle": false}),
            )?;
            Ok("disarmed".to_owned())
        });

        let all = print_table(&rows);
        for r in &rows {
            let _ = writeln!(
                log,
                "{} RESULT {} {} | {}",
                stamp(),
                if r.ok { "OK" } else { "FAIL" },
                r.item,
                r.proof
            );
        }
        Ok(all)
    }
}
