//! The S1's camera opened exactly as the stock camera process opens it, and not otherwise.
//!
//! Two generic streams (`v4l2-ctl --stream-mmap`, 2026-10-30 17:01 and 18:3x) rebooted the robot;
//! both had skipped the input selection and the three private controls and asked for a format the
//! driver refused. The stock's own sequence was then read whole off its `strace`
//! (`camera/dji_camera_v4l2_decoded.strace` l.312–510 and `camera/dji_camera_start.strace`
//! l.1056–1064, 2026-10-30), and this module replays it step by step — [`SCRIPT`] is that
//! sequence with the driver's recorded answers — through [`Video`], which is the real device on
//! the robot and a scripted stand-in in the tests. **Every answer is held to the recording before
//! the next step is taken, and the stream is switched on only when all of them matched**: a
//! driver that answers differently from how it answered the stock is a driver this code does not
//! know, and it says so ([`Divergence`]) instead of guessing.
//!
//! Beyond the recording, two things: the wait before each dequeue is a `poll` (the stock opened
//! the device non-blocking and its dequeues never returned `EAGAIN`, so it waits somewhere the
//! trace did not cover), and the stream is switched off and the buffers unmapped at the end — the
//! stock was killed mid-stream in every capture, and the device then answered `ENODEV` until a
//! reboot; `STREAMOFF` is the UAPI's own teardown, not a frame of DJI's.

use std::fmt;
use std::io;

use crate::v4l2::*;

/// What a step of the sequence does on the device. The real one is `linux::Device`; the tests'
/// one answers from the recording and writes down what it was asked.
pub trait Video {
    /// `ioctl(fd, request, arg)`; `arg` is the struct's bytes, read and written in place. `Ok(rc)`
    /// for a non-negative return, `Err` with the OS error otherwise.
    fn ioctl(&mut self, request: u32, arg: &mut [u8]) -> io::Result<i32>;
    /// `mmap` of one driver buffer: `length` bytes at `offset`, shared, read/write.
    fn map(&mut self, length: usize, offset: u32) -> io::Result<Mapping>;
    /// Wait until a frame can be dequeued, at most `timeout_ms`; `Ok(false)` on timeout.
    fn wait_readable(&mut self, timeout_ms: i32) -> io::Result<bool>;
}

/// One mapped driver buffer. The real one unmaps on drop; the stand-in owns a `Vec`.
pub struct Mapping {
    ptr: *mut u8,
    len: usize,
    owned: Option<Vec<u8>>,
}

impl Mapping {
    /// A mapping over memory this process owns (the stand-in's).
    pub fn owned(len: usize) -> Self {
        let mut v = vec![0u8; len];
        let ptr = v.as_mut_ptr();
        Self {
            ptr,
            len,
            owned: Some(v),
        }
    }

    /// A mapping the kernel made; unmapped on drop.
    ///
    /// # Safety
    /// `ptr` must be a live `mmap` of `len` bytes that nothing else unmaps.
    pub unsafe fn from_raw(ptr: *mut u8, len: usize) -> Self {
        Self {
            ptr,
            len,
            owned: None,
        }
    }

    pub fn bytes(&self) -> &[u8] {
        // SAFETY: `ptr`/`len` are either our own Vec or a live mapping (constructor contract).
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // An owned Vec frees itself; a kernel mapping is unmapped here.
        #[cfg(target_os = "linux")]
        if self.owned.is_none() {
            // SAFETY: a kernel mapping of exactly this length, made by `Device::map`.
            unsafe {
                libc::munmap(self.ptr.cast(), self.len);
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = self.owned.is_none();
    }
}

/// The three private controls the stock sets before the format, in order, with their values
/// (`V4L2_CTRL_CLASS_CAMERA + 0x191b/0x1934/0x1935`; the driver names none of them).
pub const PRIVATE_CONTROLS: [(u32, i32); 3] = [
    (CTRL_CLASS_CAMERA + 0x191b, 0),
    (CTRL_CLASS_CAMERA + 0x1934, 30),
    (CTRL_CLASS_CAMERA + 0x1935, 0),
];
pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;
/// What the driver answered the stock's `S_FMT`: the field it chose and the sizes.
pub const BYTES_PER_LINE: u32 = 1920;
pub const SIZE_IMAGE: u32 = 1_382_432;
pub const BUFFERS: u32 = 5;
/// The length of each buffer mapping, as the stock mapped them (`sizeimage` rounded to pages).
pub const MAP_LENGTH: u32 = 1_384_448;
pub const SENSOR: &str = "ov5695";
pub const DRIVER: &str = "comip";
/// What the stock passed `CROPCAP` as type (0, not a buffer type) and what the driver answered.
pub const CROPCAP_TYPE: u32 = 0;
pub const SENSOR_FULL: (u32, u32) = (2592, 1944);
/// `G_INPUT` after `S_INPUT 0` answered 1 on the robot; recorded, not explained.
pub const G_INPUT_ANSWER: u32 = 1;

/// The sequence, named step by step — what a [`Divergence`] points at and what the stand-in's
/// recording is compared against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    QueryCap,
    EnumInput(u32),
    SetInput,
    CropCap,
    GetInput,
    SetControl(u32),
    SetFormat,
    RequestBuffers,
    QueryBuffer(u32),
    Map(u32),
    Queue(u32),
    StreamOn,
}

/// The recording's step list — the order the stock camera process issued them (strace lines 312,
/// 314, 315, 316, 317, 318, 495–497, 498, 499, 500–504 interleaved with the mmaps, 505–509, 510).
pub const SCRIPT: [Step; 27] = [
    Step::QueryCap,
    Step::EnumInput(0),
    Step::EnumInput(1),
    Step::SetInput,
    Step::CropCap,
    Step::GetInput,
    Step::SetControl(PRIVATE_CONTROLS[0].0),
    Step::SetControl(PRIVATE_CONTROLS[1].0),
    Step::SetControl(PRIVATE_CONTROLS[2].0),
    Step::SetFormat,
    Step::RequestBuffers,
    Step::QueryBuffer(0),
    Step::Map(0),
    Step::QueryBuffer(1),
    Step::Map(1),
    Step::QueryBuffer(2),
    Step::Map(2),
    Step::QueryBuffer(3),
    Step::Map(3),
    Step::QueryBuffer(4),
    Step::Map(4),
    Step::Queue(0),
    Step::Queue(1),
    Step::Queue(2),
    Step::Queue(3),
    Step::Queue(4),
    Step::StreamOn,
];

/// Why the sequence stopped before the stream: the step, what the recording says, what the
/// driver said. Printed whole by `s1-cam`, so the next session reads the difference, not a guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    pub step: Step,
    pub expected: String,
    pub got: String,
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the driver diverged from the stock's recording at {:?}: expected {}, got {}",
            self.step, self.expected, self.got
        )
    }
}

impl std::error::Error for Divergence {}

#[derive(Debug)]
pub enum Error {
    Diverged(Divergence),
    Io { step: Step, source: io::Error },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Diverged(d) => d.fmt(f),
            Error::Io { step, source } => write!(f, "{step:?}: {source}"),
        }
    }
}

impl std::error::Error for Error {}

fn io_at(step: Step) -> impl FnOnce(io::Error) -> Error {
    move |source| Error::Io { step, source }
}

fn diverged<T>(
    step: Step,
    expected: impl fmt::Display,
    got: impl fmt::Display,
) -> Result<T, Error> {
    Err(Error::Diverged(Divergence {
        step,
        expected: expected.to_string(),
        got: got.to_string(),
    }))
}

/// A `#[repr(C)]` struct as the bytes `ioctl` reads and writes.
fn bytes_of<T: Copy>(v: &mut T) -> &mut [u8] {
    // SAFETY: every struct passed here is `#[repr(C)]` plain data, and `ioctl` writes within its size.
    unsafe { std::slice::from_raw_parts_mut((v as *mut T).cast::<u8>(), std::mem::size_of::<T>()) }
}

/// The camera, streaming, after the whole recording matched.
pub struct Session<V: Video> {
    dev: V,
    maps: Vec<Mapping>,
    pub format: PixFormat,
}

/// One dequeued frame: the buffer's index (to requeue) and how many bytes the driver filled.
pub struct Frame<'a> {
    pub index: u32,
    pub sequence: u32,
    pub timestamp: Timeval32,
    pub data: &'a [u8],
}

impl<V: Video> Session<V> {
    /// Replay the recording on `dev`, holding every answer to it; the stream is on when this
    /// returns `Ok`, and nothing past the diverging step has been sent when it returns `Err`.
    pub fn open(mut dev: V) -> Result<Self, Error> {
        let mut cap = Capability {
            driver: [0; 16],
            card: [0; 32],
            bus_info: [0; 32],
            version: 0,
            capabilities: 0,
            device_caps: 0,
            reserved: [0; 3],
        };
        dev.ioctl(VIDIOC_QUERYCAP, bytes_of(&mut cap))
            .map_err(io_at(Step::QueryCap))?;
        let driver = cstr(&cap.driver);
        if driver != DRIVER
            || cap.capabilities & (CAP_VIDEO_CAPTURE | CAP_STREAMING)
                != CAP_VIDEO_CAPTURE | CAP_STREAMING
        {
            return diverged(
                Step::QueryCap,
                format!("driver {DRIVER} with capture and streaming"),
                format!("driver {driver} caps 0x{:08x}", cap.capabilities),
            );
        }

        let mut input = Input {
            index: 0,
            name: [0; 32],
            type_: 0,
            audioset: 0,
            tuner: 0,
            std: 0,
            status: 0,
            capabilities: 0,
            reserved: [0; 3],
        };
        dev.ioctl(VIDIOC_ENUMINPUT, bytes_of(&mut input))
            .map_err(io_at(Step::EnumInput(0)))?;
        let name = cstr(&input.name);
        if name != SENSOR || input.type_ != INPUT_TYPE_CAMERA {
            return diverged(
                Step::EnumInput(0),
                format!("{SENSOR}, a camera"),
                format!("{name}, type {}", input.type_),
            );
        }
        let mut second = input;
        second.index = 1;
        second.name = [0; 32];
        match dev.ioctl(VIDIOC_ENUMINPUT, bytes_of(&mut second)) {
            Err(_) => {}
            Ok(_) => {
                return diverged(
                    Step::EnumInput(1),
                    "no second input (EINVAL)",
                    format!("a second input {}", cstr(&second.name)),
                );
            }
        }

        let mut idx: u32 = 0;
        dev.ioctl(VIDIOC_S_INPUT, bytes_of(&mut idx))
            .map_err(io_at(Step::SetInput))?;

        let mut crop = CropCap {
            type_: CROPCAP_TYPE,
            ..CropCap::default()
        };
        dev.ioctl(VIDIOC_CROPCAP, bytes_of(&mut crop))
            .map_err(io_at(Step::CropCap))?;
        if (crop.bounds.width, crop.bounds.height) != SENSOR_FULL {
            return diverged(
                Step::CropCap,
                format!("bounds {}x{}", SENSOR_FULL.0, SENSOR_FULL.1),
                format!("bounds {}x{}", crop.bounds.width, crop.bounds.height),
            );
        }

        let mut got_input: u32 = 0;
        dev.ioctl(VIDIOC_G_INPUT, bytes_of(&mut got_input))
            .map_err(io_at(Step::GetInput))?;
        if got_input != G_INPUT_ANSWER {
            return diverged(Step::GetInput, G_INPUT_ANSWER, got_input);
        }

        for (id, value) in PRIVATE_CONTROLS {
            let mut ctl = Control { id, value };
            dev.ioctl(VIDIOC_S_CTRL, bytes_of(&mut ctl))
                .map_err(io_at(Step::SetControl(id)))?;
            if ctl.value != value {
                return diverged(Step::SetControl(id), value, ctl.value);
            }
        }

        let mut fmt = Format::capture(PixFormat {
            width: WIDTH,
            height: HEIGHT,
            pixelformat: PIX_FMT_NV12,
            field: FIELD_ANY,
            colorspace: COLORSPACE_JPEG,
            ..PixFormat::default()
        });
        dev.ioctl(VIDIOC_S_FMT, bytes_of(&mut fmt))
            .map_err(io_at(Step::SetFormat))?;
        let p = fmt.pix;
        if (
            p.width,
            p.height,
            p.pixelformat,
            p.field,
            p.bytesperline,
            p.sizeimage,
        ) != (
            WIDTH,
            HEIGHT,
            PIX_FMT_NV12,
            FIELD_INTERLACED,
            BYTES_PER_LINE,
            SIZE_IMAGE,
        ) {
            return diverged(
                Step::SetFormat,
                format!(
                    "{WIDTH}x{HEIGHT} NV12 field {FIELD_INTERLACED} bpl {BYTES_PER_LINE} size {SIZE_IMAGE}"
                ),
                format!(
                    "{}x{} fourcc 0x{:08x} field {} bpl {} size {}",
                    p.width, p.height, p.pixelformat, p.field, p.bytesperline, p.sizeimage
                ),
            );
        }

        let mut req = RequestBuffers {
            count: BUFFERS,
            type_: BUF_TYPE_VIDEO_CAPTURE,
            memory: MEMORY_MMAP,
            ..RequestBuffers::default()
        };
        dev.ioctl(VIDIOC_REQBUFS, bytes_of(&mut req))
            .map_err(io_at(Step::RequestBuffers))?;
        if req.count != BUFFERS {
            return diverged(Step::RequestBuffers, BUFFERS, req.count);
        }

        let mut maps = Vec::with_capacity(BUFFERS as usize);
        for i in 0..BUFFERS {
            let mut buf = Buffer {
                index: i,
                type_: BUF_TYPE_VIDEO_CAPTURE,
                memory: MEMORY_MMAP,
                ..Buffer::default()
            };
            dev.ioctl(VIDIOC_QUERYBUF, bytes_of(&mut buf))
                .map_err(io_at(Step::QueryBuffer(i)))?;
            if buf.length != MAP_LENGTH || buf.m_offset != i * MAP_LENGTH {
                return diverged(
                    Step::QueryBuffer(i),
                    format!("length {MAP_LENGTH} at offset {}", i * MAP_LENGTH),
                    format!("length {} at offset {}", buf.length, buf.m_offset),
                );
            }
            let map = dev
                .map(buf.length as usize, buf.m_offset)
                .map_err(io_at(Step::Map(i)))?;
            maps.push(map);
        }
        for i in 0..BUFFERS {
            let mut buf = Buffer {
                index: i,
                type_: BUF_TYPE_VIDEO_CAPTURE,
                memory: MEMORY_MMAP,
                ..Buffer::default()
            };
            dev.ioctl(VIDIOC_QBUF, bytes_of(&mut buf))
                .map_err(io_at(Step::Queue(i)))?;
        }
        let mut kind = BUF_TYPE_VIDEO_CAPTURE;
        dev.ioctl(VIDIOC_STREAMON, bytes_of(&mut kind))
            .map_err(io_at(Step::StreamOn))?;
        Ok(Self {
            dev,
            maps,
            format: p,
        })
    }

    /// Wait up to `timeout_ms` for a frame and dequeue it; `Ok(None)` when none came in time.
    /// The frame borrows its buffer: call [`Self::requeue`] with its index once done with it.
    pub fn dequeue(&mut self, timeout_ms: i32) -> io::Result<Option<Frame<'_>>> {
        if !self.dev.wait_readable(timeout_ms)? {
            return Ok(None);
        }
        let mut buf = Buffer {
            type_: BUF_TYPE_VIDEO_CAPTURE,
            memory: MEMORY_MMAP,
            ..Buffer::default()
        };
        self.dev.ioctl(VIDIOC_DQBUF, bytes_of(&mut buf))?;
        let map = self.maps.get(buf.index as usize).ok_or_else(|| {
            io::Error::other(format!(
                "the driver dequeued buffer {} of {}",
                buf.index,
                self.maps.len()
            ))
        })?;
        let used = (buf.bytesused as usize).min(map.len());
        Ok(Some(Frame {
            index: buf.index,
            sequence: buf.sequence,
            timestamp: buf.timestamp,
            data: &map.bytes()[..used],
        }))
    }

    /// Give a dequeued buffer back to the driver.
    pub fn requeue(&mut self, index: u32) -> io::Result<()> {
        let mut buf = Buffer {
            index,
            type_: BUF_TYPE_VIDEO_CAPTURE,
            memory: MEMORY_MMAP,
            ..Buffer::default()
        };
        self.dev.ioctl(VIDIOC_QBUF, bytes_of(&mut buf)).map(|_| ())
    }

    /// Switch the stream off and unmap the buffers (the UAPI's teardown; the stock never reached
    /// it in any capture, having been killed mid-stream).
    pub fn close(mut self) -> io::Result<()> {
        let mut kind = BUF_TYPE_VIDEO_CAPTURE;
        self.dev.ioctl(VIDIOC_STREAMOFF, bytes_of(&mut kind))?;
        self.maps.clear();
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub mod linux {
    //! The real device: `open`, `ioctl`, `mmap`, `poll` through libc, the way the stock opens it
    //! (`O_RDWR | O_NONBLOCK`).
    use super::{Mapping, Video};
    use std::ffi::CString;
    use std::io;
    use std::os::unix::io::RawFd;

    pub struct Device {
        fd: RawFd,
    }

    impl Device {
        pub fn open(path: &str) -> io::Result<Self> {
            let c = CString::new(path).map_err(|_| io::Error::other("path with NUL"))?;
            // SAFETY: a valid C string; the flags are the stock's.
            let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK) };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { fd })
        }
    }

    impl Drop for Device {
        fn drop(&mut self) {
            // SAFETY: our own descriptor, closed once.
            unsafe {
                libc::close(self.fd);
            }
        }
    }

    impl Video for Device {
        fn ioctl(&mut self, request: u32, arg: &mut [u8]) -> io::Result<i32> {
            // SAFETY: `arg` is the struct the request's size encodes (held to the oracle in
            // `v4l2::tests`), writable for its whole length.
            let rc = unsafe {
                libc::ioctl(
                    self.fd,
                    request as libc::c_ulong as _,
                    arg.as_mut_ptr().cast::<libc::c_void>(),
                )
            };
            if rc < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(rc)
            }
        }

        fn map(&mut self, length: usize, offset: u32) -> io::Result<Mapping> {
            // SAFETY: a shared read/write mapping of a driver buffer at the offset QUERYBUF gave;
            // unmapped by `Mapping::drop`.
            let ptr = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    length,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    self.fd,
                    // `off_t` is 32-bit on the robot; the five offsets are under 7 MB.
                    offset as libc::off_t,
                )
            };
            if ptr == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: a fresh mapping of `length` bytes, ours to unmap.
            Ok(unsafe { Mapping::from_raw(ptr.cast::<u8>(), length) })
        }

        fn wait_readable(&mut self, timeout_ms: i32) -> io::Result<bool> {
            let mut pfd = libc::pollfd {
                fd: self.fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one pollfd, ours.
            let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(rc > 0 && pfd.revents & libc::POLLIN != 0)
        }
    }
}

#[cfg(test)]
pub mod tests {
    //! The stand-in device answers each request as the driver answered the stock (the recording),
    //! and writes down what it was asked; the tests hold the module's sequence to `SCRIPT` and
    //! prove it stops before the stream on the first answer that differs.
    use super::*;

    /// What the stand-in says back; the recording by default, one answer bent for a test.
    #[derive(Clone)]
    pub struct Answers {
        pub sizeimage: u32,
        pub second_input: bool,
        pub g_input: u32,
        pub bytesused: u32,
    }

    impl Default for Answers {
        /// The recording's own numbers as literals, never the module's constants: a constant
        /// planted wrong must make the module and this stand-in disagree (`cam-refute.sh`).
        fn default() -> Self {
            Self {
                sizeimage: 1_382_432,
                second_input: false,
                g_input: 1,
                bytesused: 1_382_432,
            }
        }
    }

    #[derive(Default)]
    pub struct Scripted {
        pub answers: Answers,
        pub asked: Vec<Step>,
        pub queued: Vec<u32>,
        pub streaming: bool,
        pub frames: u32,
    }

    fn as_struct<T: Copy>(arg: &mut [u8]) -> &mut T {
        assert_eq!(
            arg.len(),
            std::mem::size_of::<T>(),
            "the request's struct size"
        );
        // SAFETY: the test passes the struct the request names, same size, plain data.
        unsafe { &mut *arg.as_mut_ptr().cast::<T>() }
    }

    fn einval() -> io::Error {
        io::Error::from_raw_os_error(22)
    }

    impl Video for Scripted {
        fn ioctl(&mut self, request: u32, arg: &mut [u8]) -> io::Result<i32> {
            match request {
                VIDIOC_QUERYCAP => {
                    let c: &mut Capability = as_struct(arg);
                    c.driver[..5].copy_from_slice(b"comip");
                    c.card[..5].copy_from_slice(b"comip");
                    c.bus_info[..12].copy_from_slice(b"comip-camera");
                    c.version = 0x0001_0000;
                    c.capabilities = CAP_VIDEO_CAPTURE | CAP_STREAMING;
                    self.asked.push(Step::QueryCap);
                }
                VIDIOC_ENUMINPUT => {
                    let i: &mut Input = as_struct(arg);
                    self.asked.push(Step::EnumInput(i.index));
                    match i.index {
                        0 => {
                            i.name[..6].copy_from_slice(b"ov5695");
                            i.type_ = INPUT_TYPE_CAMERA;
                        }
                        1 if self.answers.second_input => {
                            i.name[..4].copy_from_slice(b"mock");
                            i.type_ = INPUT_TYPE_CAMERA;
                        }
                        _ => return Err(einval()),
                    }
                }
                VIDIOC_S_INPUT => {
                    assert_eq!(*as_struct::<u32>(arg), 0);
                    self.asked.push(Step::SetInput);
                }
                VIDIOC_CROPCAP => {
                    let c: &mut CropCap = as_struct(arg);
                    assert_eq!(c.type_, CROPCAP_TYPE, "the stock passed type 0");
                    c.bounds = Rect {
                        left: 0,
                        top: 0,
                        width: SENSOR_FULL.0,
                        height: SENSOR_FULL.1,
                    };
                    c.defrect = c.bounds;
                    c.pixelaspect = Fract {
                        numerator: 1,
                        denominator: 1,
                    };
                    self.asked.push(Step::CropCap);
                }
                VIDIOC_G_INPUT => {
                    *as_struct::<u32>(arg) = self.answers.g_input;
                    self.asked.push(Step::GetInput);
                }
                VIDIOC_S_CTRL => {
                    let c: &mut Control = as_struct(arg);
                    self.asked.push(Step::SetControl(c.id));
                }
                VIDIOC_S_FMT => {
                    let f: &mut Format = as_struct(arg);
                    assert_eq!(f.type_, BUF_TYPE_VIDEO_CAPTURE);
                    assert_eq!(
                        (
                            f.pix.width,
                            f.pix.height,
                            f.pix.pixelformat,
                            f.pix.field,
                            f.pix.colorspace
                        ),
                        (WIDTH, HEIGHT, PIX_FMT_NV12, FIELD_ANY, COLORSPACE_JPEG),
                        "asked as the stock asked"
                    );
                    f.pix.field = FIELD_INTERLACED;
                    f.pix.bytesperline = BYTES_PER_LINE;
                    f.pix.sizeimage = self.answers.sizeimage;
                    self.asked.push(Step::SetFormat);
                }
                VIDIOC_REQBUFS => {
                    let r: &mut RequestBuffers = as_struct(arg);
                    assert_eq!(
                        (r.type_, r.memory, r.count),
                        (BUF_TYPE_VIDEO_CAPTURE, MEMORY_MMAP, BUFFERS)
                    );
                    self.asked.push(Step::RequestBuffers);
                }
                VIDIOC_QUERYBUF => {
                    let b: &mut Buffer = as_struct(arg);
                    assert_eq!((b.type_, b.memory), (BUF_TYPE_VIDEO_CAPTURE, MEMORY_MMAP));
                    b.length = MAP_LENGTH;
                    b.m_offset = b.index * MAP_LENGTH;
                    b.flags = BUF_FLAG_MAPPED;
                    self.asked.push(Step::QueryBuffer(b.index));
                }
                VIDIOC_QBUF => {
                    let b: &mut Buffer = as_struct(arg);
                    assert_eq!((b.type_, b.memory), (BUF_TYPE_VIDEO_CAPTURE, MEMORY_MMAP));
                    self.queued.push(b.index);
                    if !self.streaming {
                        self.asked.push(Step::Queue(b.index));
                    }
                }
                VIDIOC_DQBUF => {
                    let b: &mut Buffer = as_struct(arg);
                    assert!(self.streaming, "a dequeue before the stream is on");
                    let index = self.queued.first().copied().ok_or_else(einval)?;
                    self.queued.remove(0);
                    b.index = index;
                    b.bytesused = self.answers.bytesused;
                    b.sequence = self.frames;
                    b.timestamp = Timeval32 {
                        sec: 1,
                        usec: 33_000 * self.frames as i32,
                    };
                    self.frames += 1;
                }
                VIDIOC_STREAMON => {
                    assert_eq!(*as_struct::<u32>(arg), BUF_TYPE_VIDEO_CAPTURE);
                    self.streaming = true;
                    self.asked.push(Step::StreamOn);
                }
                VIDIOC_STREAMOFF => {
                    self.streaming = false;
                }
                other => panic!("the stock never issued ioctl 0x{other:08x}"),
            }
            Ok(0)
        }

        fn map(&mut self, length: usize, offset: u32) -> io::Result<Mapping> {
            assert_eq!(length, MAP_LENGTH as usize);
            assert_eq!(offset % MAP_LENGTH, 0);
            self.asked.push(Step::Map(offset / MAP_LENGTH));
            let mut m = Mapping::owned(length);
            // a recognisable payload per buffer
            if let Some(v) = m.owned.as_mut() {
                v.fill((offset / MAP_LENGTH) as u8 + 1);
            }
            Ok(m)
        }

        fn wait_readable(&mut self, _timeout_ms: i32) -> io::Result<bool> {
            Ok(self.streaming && !self.queued.is_empty())
        }
    }

    /// The whole recording, in order, and nothing else, before the stream goes on; then frames
    /// come out of the buffers in the order they were queued, each requeued by hand.
    #[test]
    fn the_sequence_is_the_stocks_recording_and_frames_flow() {
        let dev = Scripted::default();
        let mut s = Session::open(dev).expect("the recording matches itself");
        assert_eq!(s.dev.asked, SCRIPT.to_vec());
        assert_eq!(s.format.sizeimage, 1_382_432, "the recording's size");
        assert_eq!(s.format.field, FIELD_INTERLACED);
        for expect in 0..7u32 {
            let (index, seq, len, first) = {
                let f = s.dequeue(100).expect("io").expect("a frame");
                (f.index, f.sequence, f.data.len(), f.data[0])
            };
            assert_eq!(index, expect % BUFFERS);
            assert_eq!(seq, expect);
            assert_eq!(len, 1_382_432, "bytesused, not the mapping");
            assert_eq!(
                first,
                (expect % BUFFERS) as u8 + 1,
                "the frame reads its own buffer"
            );
            s.requeue(index).unwrap();
        }
        s.close().unwrap();
    }

    /// A driver that answers the format with another size is not the stock's: the sequence stops
    /// there, names it, and the stream is never switched on.
    #[test]
    fn a_different_answer_stops_before_the_stream() {
        let mut dev = Scripted::default();
        dev.answers.sizeimage = 1_382_400;
        let err = Session::open(dev).err().expect("diverged");
        match err {
            Error::Diverged(d) => {
                assert_eq!(d.step, Step::SetFormat);
                assert!(d.got.contains("size 1382400"), "{d}");
                assert!(d.expected.contains("size 1382432"), "{d}");
            }
            other => panic!("{other}"),
        }
    }

    /// The device is consumed on error, so the proof that nothing past the divergence was sent is
    /// taken from a stand-in that records into a shared log.
    #[test]
    fn nothing_past_the_divergence_is_sent() {
        use std::sync::{Arc, Mutex};
        struct Shared(Scripted, Arc<Mutex<Vec<Step>>>);
        impl Video for Shared {
            fn ioctl(&mut self, r: u32, a: &mut [u8]) -> io::Result<i32> {
                let rc = self.0.ioctl(r, a);
                *self.1.lock().unwrap() = self.0.asked.clone();
                rc
            }
            fn map(&mut self, l: usize, o: u32) -> io::Result<Mapping> {
                self.0.map(l, o)
            }
            fn wait_readable(&mut self, t: i32) -> io::Result<bool> {
                self.0.wait_readable(t)
            }
        }
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut inner = Scripted::default();
        inner.answers.second_input = true;
        let err = Session::open(Shared(inner, Arc::clone(&log)))
            .err()
            .expect("diverged");
        assert!(
            matches!(
                err,
                Error::Diverged(Divergence {
                    step: Step::EnumInput(1),
                    ..
                })
            ),
            "{err}"
        );
        let asked = log.lock().unwrap().clone();
        assert_eq!(asked, SCRIPT[..3].to_vec(), "stopped at the third step");
        assert!(!asked.contains(&Step::StreamOn));
    }

    #[test]
    fn a_divergence_prints_the_step_and_both_values() {
        let d = Divergence {
            step: Step::GetInput,
            expected: "1".into(),
            got: "0".into(),
        };
        assert_eq!(
            d.to_string(),
            "the driver diverged from the stock's recording at GetInput: expected 1, got 0"
        );
    }
}
