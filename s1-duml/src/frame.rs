//! One DUML/V1 frame, encoded and decoded.
//!
//! ```text
//!  0     1     2     3     4      5        6 7    8      9       10      11..    n-2 n-1
//!  0x55  len   ver   crc8  sender receiver seq    attri  cmd_set cmd_id  data    crc16
//! ```
//!
//! `len` is the whole frame including both CRCs; `ver` carries the two high bits of the length in
//! its low bits and `0x4` above them; `attri` is `is_ack << 7 | need_ack << 5` with `need_ack` two bits
//! wide ([`Ack`]). This is `Msg.pack` in
//! DJI's `protocol.py` and `write_to()` in proroklab's `robomaster_sdk_can`, and it is what
//! `tests/fixtures/heartbeat_real.bin` — read off the robot's UART — decodes as.

use crate::crc::{CRC8_SEED, CRC16_SEED, crc8, crc16};

/// Start of frame.
pub const SOF: u8 = 0x55;
/// Header plus both CRCs: a frame with no data is this long.
pub const OVERHEAD: usize = 13;
/// Where the data starts.
pub const DATA_OFFSET: usize = 11;
/// The longest frame the two length bits allow.
pub const MAX_LEN: usize = 0x3ff;

/// The two "need ack" bits of the attribute byte, as `Msg.pack` writes them (`need_ack << 5`):
/// `0 for no need, 1 for ack now, 2 for need when finish` — the SDK's own words, and `2` is its
/// default for every command. The stock controller sends `2` on its DDS requests (measured
/// 2026-10-30, `dds_add_{node,sub}_stock_real.bin`); without it the motion controller stayed silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ack {
    #[default]
    None = 0,
    Now = 1,
    WhenDone = 2,
}

impl Ack {
    fn from_attri(attri: u8) -> Result<Self, DecodeError> {
        match (attri >> 5) & 0x3 {
            0 => Ok(Self::None),
            1 => Ok(Self::Now),
            2 => Ok(Self::WhenDone),
            _ => Err(DecodeError::BadAttri(attri)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub sender: u8,
    pub receiver: u8,
    pub seq: u16,
    pub is_ack: bool,
    /// What the sender wants back; the stock asks `WhenDone` on every command it expects answered.
    pub ack: Ack,
    pub cmd_set: u8,
    pub cmd_id: u8,
    pub data: Vec<u8>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("frame too short: have {have} bytes, need {need}")]
    TooShort { have: usize, need: usize },
    #[error("not a frame: first byte 0x{0:02x}, expected 0x55")]
    BadSof(u8),
    #[error("header CRC mismatch: computed 0x{computed:02x}, frame says 0x{found:02x}")]
    HeaderCrc { computed: u8, found: u8 },
    #[error("frame CRC mismatch: computed 0x{computed:04x}, frame says 0x{found:04x}")]
    BodyCrc { computed: u16, found: u16 },
    #[error("data of {0} bytes does not fit a frame")]
    TooLong(usize),
    #[error("header declares {declared} bytes, a frame is at least {OVERHEAD}")]
    BadLength { declared: usize },
    #[error("attribute byte 0x{0:02x} has both ack bits set, which the SDK never writes")]
    BadAttri(u8),
}

impl Frame {
    /// A request frame: not an ack, not asking for one. What every command the PoC sent looks like.
    pub fn request(
        sender: u8,
        receiver: u8,
        seq: u16,
        cmd_set: u8,
        cmd_id: u8,
        data: Vec<u8>,
    ) -> Self {
        Self {
            sender,
            receiver,
            seq,
            is_ack: false,
            ack: Ack::None,
            cmd_set,
            cmd_id,
            data,
        }
    }

    /// The same frame asking for `ack`.
    pub fn with_ack(mut self, ack: Ack) -> Self {
        self.ack = ack;
        self
    }

    /// How many bytes this frame takes on the wire.
    pub fn wire_len(&self) -> usize {
        self.data.len() + OVERHEAD
    }

    /// The bytes that go on the wire. Identical to the lab's Python `build()`, which was checked
    /// against the robot — the test `real_heartbeat_round_trips` holds it there.
    pub fn encode(&self) -> Result<Vec<u8>, DecodeError> {
        let size = self.wire_len();
        if size > MAX_LEN {
            return Err(DecodeError::TooLong(self.data.len()));
        }
        let mut buf = vec![0u8; size];
        buf[0] = SOF;
        buf[1] = (size & 0xff) as u8;
        buf[2] = (((size >> 8) & 0x3) as u8) | 0x4;
        buf[3] = crc8(CRC8_SEED, &buf[0..3]);
        buf[4] = self.sender;
        buf[5] = self.receiver;
        buf[6] = (self.seq & 0xff) as u8;
        buf[7] = (self.seq >> 8) as u8;
        buf[8] = (u8::from(self.is_ack) << 7) | ((self.ack as u8) << 5);
        buf[9] = self.cmd_set;
        buf[10] = self.cmd_id;
        buf[DATA_OFFSET..DATA_OFFSET + self.data.len()].copy_from_slice(&self.data);
        let cp = crc16(CRC16_SEED, &buf[0..size - 2]);
        buf[size - 2] = (cp & 0xff) as u8;
        buf[size - 1] = (cp >> 8) as u8;
        Ok(buf)
    }

    /// The declared length of a frame whose first four bytes are `head`, or `None` if `head` does
    /// not start a frame (wrong SOF or header CRC). Enough to know how many bytes to wait for.
    pub fn declared_len(head: &[u8]) -> Option<usize> {
        if head.len() < 4 || head[0] != SOF {
            return None;
        }
        if crc8(CRC8_SEED, &head[0..3]) != head[3] {
            return None;
        }
        Some(usize::from(head[1]) | (usize::from(head[2] & 0x3) << 8))
    }

    /// Decode exactly one frame from `bytes`, which must be the whole frame.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() < OVERHEAD {
            return Err(DecodeError::TooShort {
                have: bytes.len(),
                need: OVERHEAD,
            });
        }
        if bytes[0] != SOF {
            return Err(DecodeError::BadSof(bytes[0]));
        }
        let computed = crc8(CRC8_SEED, &bytes[0..3]);
        if computed != bytes[3] {
            return Err(DecodeError::HeaderCrc {
                computed,
                found: bytes[3],
            });
        }
        let size = usize::from(bytes[1]) | (usize::from(bytes[2] & 0x3) << 8);
        // A valid header CRC over a length that cannot hold the header itself: the `Decoder`
        // skips these, and the public decoder must refuse them rather than index backwards.
        if size < OVERHEAD {
            return Err(DecodeError::BadLength { declared: size });
        }
        if bytes.len() < size {
            return Err(DecodeError::TooShort {
                have: bytes.len(),
                need: size,
            });
        }
        let computed = crc16(CRC16_SEED, &bytes[0..size - 2]);
        let found = u16::from(bytes[size - 2]) | (u16::from(bytes[size - 1]) << 8);
        if computed != found {
            return Err(DecodeError::BodyCrc { computed, found });
        }
        Ok(Self {
            sender: bytes[4],
            receiver: bytes[5],
            seq: u16::from(bytes[6]) | (u16::from(bytes[7]) << 8),
            is_ack: bytes[8] & 0x80 != 0,
            ack: Ack::from_attri(bytes[8])?,
            cmd_set: bytes[9],
            cmd_id: bytes[10],
            data: bytes[DATA_OFFSET..size - 2].to_vec(),
        })
    }
}

/// Frames out of a byte stream that arrives in whatever pieces the UART driver hands over.
///
/// A UART has no framing: a read may start mid-frame, and a corrupted byte must cost one frame, not
/// the rest of the session. So this walks to the next `0x55` whose header CRC holds, waits until the
/// declared length has arrived, checks the trailing CRC, and on failure drops one byte and tries
/// again from the next candidate. `dropped()` counts the bytes thrown away, for `robot.health`.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    dropped: u64,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes thrown away while resynchronising, cumulative.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Feed bytes; get every complete, valid frame they finished.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Frame> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            // Skip to a plausible header.
            let start = match self.buf.iter().position(|&b| b == SOF) {
                Some(i) => i,
                None => {
                    self.dropped += self.buf.len() as u64;
                    self.buf.clear();
                    break;
                }
            };
            if start > 0 {
                self.dropped += start as u64;
                self.buf.drain(..start);
            }
            if self.buf.len() < 4 {
                break;
            }
            let Some(size) = Frame::declared_len(&self.buf[..4]) else {
                // A 0x55 inside somebody else's data: not a header. Drop it and look again.
                self.dropped += 1;
                self.buf.drain(..1);
                continue;
            };
            if size < OVERHEAD {
                self.dropped += 1;
                self.buf.drain(..1);
                continue;
            }
            if self.buf.len() < size {
                break;
            }
            match Frame::decode(&self.buf[..size]) {
                Ok(frame) => {
                    self.buf.drain(..size);
                    out.push(frame);
                }
                Err(_) => {
                    self.dropped += 1;
                    self.buf.drain(..1);
                }
            }
        }
        out
    }
}
