//! The V4L2 UAPI as the S1's kernel (3.10, ARM 32-bit) sees it: the structures and ioctl numbers
//! `s1-cam` puts on the wire, pinned to what the ARM compiler says they are.
//!
//! Nothing here is written from memory of the headers. `scripts/s1/v4l2-oracle.sh` builds
//! `v4l2-oracle.c` inside a `linux/arm/v7` bookworm container and prints every size, offset,
//! ioctl number and constant used below as JSON (`tests/fixtures/v4l2_arm32.json`); the test at
//! the bottom holds each `#[repr(C)]` type here to that file, field by field, and each constant to
//! its value. The S1's `strace` names the buffer ioctls `*_TIME32` for the same reason this matters:
//! a 32-bit `timeval` sits inside `v4l2_buffer`, and the ioctl number encodes the struct's size —
//! one offset wrong is a request the kernel refuses, or worse, accepts with the wrong meaning
//! (`mediad/src/exposure.rs` chose `v4l2-ctl` over the ioctl to avoid pinning these by hand; the
//! S1's driver exposes its controls by number only, so there is no tool to lean on here).
//!
//! The 6.1 headers the container carries and the 3.10 headers the robot runs agree on every
//! size and offset used (the buffer and format structs are padded to fixed sizes in both; the four
//! pixel-format fields 6.1 added sit in what 3.10 calls `priv`'s padding and are left zero).

/// `struct v4l2_capability`, 104 bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Capability {
    pub driver: [u8; 16],
    pub card: [u8; 32],
    pub bus_info: [u8; 32],
    pub version: u32,
    pub capabilities: u32,
    pub device_caps: u32,
    pub reserved: [u32; 3],
}

/// `struct v4l2_input`, 80 bytes (the `std` field is a `u64`, which aligns the tail to 8).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Input {
    pub index: u32,
    pub name: [u8; 32],
    pub type_: u32,
    pub audioset: u32,
    pub tuner: u32,
    pub std: u64,
    pub status: u32,
    pub capabilities: u32,
    pub reserved: [u32; 3],
}

/// `struct v4l2_control`, 8 bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Control {
    pub id: u32,
    pub value: i32,
}

/// `struct v4l2_rect`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
}

/// `struct v4l2_fract`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Fract {
    pub numerator: u32,
    pub denominator: u32,
}

/// `struct v4l2_cropcap`, 44 bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CropCap {
    pub type_: u32,
    pub bounds: Rect,
    pub defrect: Rect,
    pub pixelaspect: Fract,
}

/// `struct v4l2_pix_format`, 48 bytes in the 6.1 headers; the kernel 3.10 reads the first 32
/// (through `priv`) and the last four fields are the padding of its 200-byte union.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PixFormat {
    pub width: u32,
    pub height: u32,
    pub pixelformat: u32,
    pub field: u32,
    pub bytesperline: u32,
    pub sizeimage: u32,
    pub colorspace: u32,
    pub priv_: u32,
    pub flags: u32,
    pub ycbcr_enc: u32,
    pub quantization: u32,
    pub xfer_func: u32,
}

/// `struct v4l2_format` for a video capture: `type`, then the 200-byte union with `pix` first.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Format {
    pub type_: u32,
    pub pix: PixFormat,
    pub padding: [u8; 200 - std::mem::size_of::<PixFormat>()],
}

impl Format {
    pub fn capture(pix: PixFormat) -> Self {
        Self {
            type_: BUF_TYPE_VIDEO_CAPTURE,
            pix,
            padding: [0; 200 - std::mem::size_of::<PixFormat>()],
        }
    }
}

/// `struct v4l2_requestbuffers`, 20 bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct RequestBuffers {
    pub count: u32,
    pub type_: u32,
    pub memory: u32,
    pub capabilities: u32,
    pub flags: u8,
    pub reserved: [u8; 3],
}

/// `struct timeval` with 32-bit seconds — the layout the robot's kernel and libc share.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timeval32 {
    pub sec: i32,
    pub usec: i32,
}

/// `struct v4l2_timecode`, 16 bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Timecode {
    pub type_: u32,
    pub flags: u32,
    pub frames: u8,
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u8,
    pub userbits: [u8; 4],
}

/// `struct v4l2_buffer`, 68 bytes on ARM 32-bit (`m` is a 4-byte union: the MMAP offset is what
/// this driver fills; a user pointer or a planes pointer are the same 4 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Buffer {
    pub index: u32,
    pub type_: u32,
    pub bytesused: u32,
    pub flags: u32,
    pub field: u32,
    pub timestamp: Timeval32,
    pub timecode: Timecode,
    pub sequence: u32,
    pub memory: u32,
    pub m_offset: u32,
    pub length: u32,
    pub reserved2: u32,
    pub reserved: u32,
}

// ioctl request numbers for the sizes above (`_IOR`/`_IOW`/`_IOWR` of 'V'), as the oracle printed
// them; the test below is what makes these numbers trustworthy, not the arithmetic.
pub const VIDIOC_QUERYCAP: u32 = 0x8068_5600;
pub const VIDIOC_ENUMINPUT: u32 = 0xc050_561a;
pub const VIDIOC_G_INPUT: u32 = 0x8004_5626;
pub const VIDIOC_S_INPUT: u32 = 0xc004_5627;
pub const VIDIOC_CROPCAP: u32 = 0xc02c_563a;
pub const VIDIOC_G_CTRL: u32 = 0xc008_561b;
pub const VIDIOC_S_CTRL: u32 = 0xc008_561c;
pub const VIDIOC_G_FMT: u32 = 0xc0cc_5604;
pub const VIDIOC_S_FMT: u32 = 0xc0cc_5605;
pub const VIDIOC_REQBUFS: u32 = 0xc014_5608;
pub const VIDIOC_QUERYBUF: u32 = 0xc044_5609;
pub const VIDIOC_QBUF: u32 = 0xc044_560f;
pub const VIDIOC_DQBUF: u32 = 0xc044_5611;
pub const VIDIOC_STREAMON: u32 = 0x4004_5612;
pub const VIDIOC_STREAMOFF: u32 = 0x4004_5613;

pub const BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
pub const MEMORY_MMAP: u32 = 1;
/// `v4l2_fourcc('N', 'V', '1', '2')`.
pub const PIX_FMT_NV12: u32 = 0x3231_564e;
pub const FIELD_ANY: u32 = 0;
pub const FIELD_INTERLACED: u32 = 4;
pub const COLORSPACE_JPEG: u32 = 7;
pub const CTRL_CLASS_CAMERA: u32 = 0x009a_0000;
pub const CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
pub const CAP_STREAMING: u32 = 0x0400_0000;
pub const INPUT_TYPE_CAMERA: u32 = 2;
pub const BUF_FLAG_MAPPED: u32 = 0x1;
pub const BUF_FLAG_QUEUED: u32 = 0x2;
pub const BUF_FLAG_DONE: u32 = 0x4;

/// The name an ioctl goes by in a log or a divergence report.
pub fn ioctl_name(request: u32) -> &'static str {
    match request {
        VIDIOC_QUERYCAP => "QUERYCAP",
        VIDIOC_ENUMINPUT => "ENUMINPUT",
        VIDIOC_G_INPUT => "G_INPUT",
        VIDIOC_S_INPUT => "S_INPUT",
        VIDIOC_CROPCAP => "CROPCAP",
        VIDIOC_G_CTRL => "G_CTRL",
        VIDIOC_S_CTRL => "S_CTRL",
        VIDIOC_G_FMT => "G_FMT",
        VIDIOC_S_FMT => "S_FMT",
        VIDIOC_REQBUFS => "REQBUFS",
        VIDIOC_QUERYBUF => "QUERYBUF",
        VIDIOC_QBUF => "QBUF",
        VIDIOC_DQBUF => "DQBUF",
        VIDIOC_STREAMON => "STREAMON",
        VIDIOC_STREAMOFF => "STREAMOFF",
        _ => "ioctl?",
    }
}

/// A NUL-terminated fixed field as text.
pub fn cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    fn oracle() -> serde_json::Map<String, serde_json::Value> {
        let text = include_str!("../tests/fixtures/v4l2_arm32.json");
        serde_json::from_str::<serde_json::Value>(text)
            .expect("the oracle's JSON")
            .as_object()
            .expect("an object")
            .clone()
    }

    fn want(o: &serde_json::Map<String, serde_json::Value>, key: &str) -> u64 {
        o.get(key)
            .unwrap_or_else(|| panic!("the oracle has no {key}"))
            .as_u64()
            .unwrap_or_else(|| panic!("{key} is not a number"))
    }

    /// Every size and offset the ARM compiler printed, held against the Rust layouts. A failure
    /// here is a struct the kernel would read differently from how it was written.
    #[test]
    fn layouts_are_the_arm32_compilers() {
        let o = oracle();
        assert_eq!(o["arch"], "arm32");
        assert_eq!(want(&o, "sizeof_timeval"), 8, "32-bit time_t");
        let checks: &[(&str, u64)] = &[
            ("sizeof_v4l2_capability", size_of::<Capability>() as u64),
            (
                "off_v4l2_capability_driver",
                offset_of!(Capability, driver) as u64,
            ),
            (
                "off_v4l2_capability_card",
                offset_of!(Capability, card) as u64,
            ),
            (
                "off_v4l2_capability_bus_info",
                offset_of!(Capability, bus_info) as u64,
            ),
            (
                "off_v4l2_capability_version",
                offset_of!(Capability, version) as u64,
            ),
            (
                "off_v4l2_capability_capabilities",
                offset_of!(Capability, capabilities) as u64,
            ),
            ("sizeof_v4l2_input", size_of::<Input>() as u64),
            ("off_v4l2_input_index", offset_of!(Input, index) as u64),
            ("off_v4l2_input_name", offset_of!(Input, name) as u64),
            ("off_v4l2_input_type", offset_of!(Input, type_) as u64),
            ("sizeof_v4l2_control", size_of::<Control>() as u64),
            ("off_v4l2_control_id", offset_of!(Control, id) as u64),
            ("off_v4l2_control_value", offset_of!(Control, value) as u64),
            ("sizeof_v4l2_cropcap", size_of::<CropCap>() as u64),
            ("off_v4l2_cropcap_type", offset_of!(CropCap, type_) as u64),
            (
                "off_v4l2_cropcap_bounds",
                offset_of!(CropCap, bounds) as u64,
            ),
            (
                "off_v4l2_cropcap_defrect",
                offset_of!(CropCap, defrect) as u64,
            ),
            (
                "off_v4l2_cropcap_pixelaspect",
                offset_of!(CropCap, pixelaspect) as u64,
            ),
            ("sizeof_v4l2_format", size_of::<Format>() as u64),
            ("off_v4l2_format_type", offset_of!(Format, type_) as u64),
            ("off_v4l2_format_fmt", offset_of!(Format, pix) as u64),
            ("sizeof_v4l2_pix_format", size_of::<PixFormat>() as u64),
            (
                "off_v4l2_pix_format_width",
                offset_of!(PixFormat, width) as u64,
            ),
            (
                "off_v4l2_pix_format_height",
                offset_of!(PixFormat, height) as u64,
            ),
            (
                "off_v4l2_pix_format_pixelformat",
                offset_of!(PixFormat, pixelformat) as u64,
            ),
            (
                "off_v4l2_pix_format_field",
                offset_of!(PixFormat, field) as u64,
            ),
            (
                "off_v4l2_pix_format_bytesperline",
                offset_of!(PixFormat, bytesperline) as u64,
            ),
            (
                "off_v4l2_pix_format_sizeimage",
                offset_of!(PixFormat, sizeimage) as u64,
            ),
            (
                "off_v4l2_pix_format_colorspace",
                offset_of!(PixFormat, colorspace) as u64,
            ),
            (
                "off_v4l2_pix_format_priv",
                offset_of!(PixFormat, priv_) as u64,
            ),
            (
                "sizeof_v4l2_requestbuffers",
                size_of::<RequestBuffers>() as u64,
            ),
            (
                "off_v4l2_requestbuffers_count",
                offset_of!(RequestBuffers, count) as u64,
            ),
            (
                "off_v4l2_requestbuffers_type",
                offset_of!(RequestBuffers, type_) as u64,
            ),
            (
                "off_v4l2_requestbuffers_memory",
                offset_of!(RequestBuffers, memory) as u64,
            ),
            ("sizeof_v4l2_buffer", size_of::<Buffer>() as u64),
            ("off_v4l2_buffer_index", offset_of!(Buffer, index) as u64),
            ("off_v4l2_buffer_type", offset_of!(Buffer, type_) as u64),
            (
                "off_v4l2_buffer_bytesused",
                offset_of!(Buffer, bytesused) as u64,
            ),
            ("off_v4l2_buffer_flags", offset_of!(Buffer, flags) as u64),
            ("off_v4l2_buffer_field", offset_of!(Buffer, field) as u64),
            (
                "off_v4l2_buffer_timestamp",
                offset_of!(Buffer, timestamp) as u64,
            ),
            (
                "off_v4l2_buffer_timecode",
                offset_of!(Buffer, timecode) as u64,
            ),
            (
                "off_v4l2_buffer_sequence",
                offset_of!(Buffer, sequence) as u64,
            ),
            ("off_v4l2_buffer_memory", offset_of!(Buffer, memory) as u64),
            ("off_v4l2_buffer_m", offset_of!(Buffer, m_offset) as u64),
            ("off_v4l2_buffer_length", offset_of!(Buffer, length) as u64),
        ];
        for (key, ours) in checks {
            assert_eq!(*ours, want(&o, key), "{key}");
        }
    }

    /// Every request number and constant, held to the oracle's.
    #[test]
    fn ioctl_numbers_and_constants_are_the_oracles() {
        let o = oracle();
        let checks: &[(&str, u32)] = &[
            ("VIDIOC_QUERYCAP", VIDIOC_QUERYCAP),
            ("VIDIOC_ENUMINPUT", VIDIOC_ENUMINPUT),
            ("VIDIOC_G_INPUT", VIDIOC_G_INPUT),
            ("VIDIOC_S_INPUT", VIDIOC_S_INPUT),
            ("VIDIOC_CROPCAP", VIDIOC_CROPCAP),
            ("VIDIOC_G_CTRL", VIDIOC_G_CTRL),
            ("VIDIOC_S_CTRL", VIDIOC_S_CTRL),
            ("VIDIOC_G_FMT", VIDIOC_G_FMT),
            ("VIDIOC_S_FMT", VIDIOC_S_FMT),
            ("VIDIOC_REQBUFS", VIDIOC_REQBUFS),
            ("VIDIOC_QUERYBUF", VIDIOC_QUERYBUF),
            ("VIDIOC_QBUF", VIDIOC_QBUF),
            ("VIDIOC_DQBUF", VIDIOC_DQBUF),
            ("VIDIOC_STREAMON", VIDIOC_STREAMON),
            ("VIDIOC_STREAMOFF", VIDIOC_STREAMOFF),
            ("V4L2_BUF_TYPE_VIDEO_CAPTURE", BUF_TYPE_VIDEO_CAPTURE),
            ("V4L2_MEMORY_MMAP", MEMORY_MMAP),
            ("V4L2_PIX_FMT_NV12", PIX_FMT_NV12),
            ("V4L2_FIELD_ANY", FIELD_ANY),
            ("V4L2_FIELD_INTERLACED", FIELD_INTERLACED),
            ("V4L2_COLORSPACE_JPEG", COLORSPACE_JPEG),
            ("V4L2_CTRL_CLASS_CAMERA", CTRL_CLASS_CAMERA),
            ("V4L2_CAP_VIDEO_CAPTURE", CAP_VIDEO_CAPTURE),
            ("V4L2_CAP_STREAMING", CAP_STREAMING),
            ("V4L2_INPUT_TYPE_CAMERA", INPUT_TYPE_CAMERA),
            ("V4L2_BUF_FLAG_MAPPED", BUF_FLAG_MAPPED),
            ("V4L2_BUF_FLAG_QUEUED", BUF_FLAG_QUEUED),
            ("V4L2_BUF_FLAG_DONE", BUF_FLAG_DONE),
        ];
        for (key, ours) in checks {
            assert_eq!(u64::from(*ours), want(&o, key), "{key}");
        }
        // and the request numbers carry the struct sizes, as the UAPI builds them
        assert_eq!((VIDIOC_QUERYBUF >> 16) & 0x3fff, size_of::<Buffer>() as u32);
        assert_eq!((VIDIOC_S_FMT >> 16) & 0x3fff, size_of::<Format>() as u32);
        assert_eq!(
            (VIDIOC_QUERYCAP >> 16) & 0x3fff,
            size_of::<Capability>() as u32
        );
    }

    #[test]
    fn fixed_fields_read_as_text() {
        let mut d = [0u8; 16];
        d[..5].copy_from_slice(b"comip");
        assert_eq!(cstr(&d), "comip");
        assert_eq!(ioctl_name(VIDIOC_DQBUF), "DQBUF");
    }
}
