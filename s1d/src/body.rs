//! The seam between the loop and the UART.
//!
//! Two bytes-in, bytes-out methods and nothing else, so the loop can be driven against a file in a
//! test and against `/dev/ttyS3` on the robot with the same code. A simulator (`--sim`, a later
//! phase) is a third implementation of this trait, not a change to the loop.

use std::io::{Read, Write};
use std::time::Duration;

pub trait S1Body: Send {
    /// One whole frame to the body. Blocking is fine: a UART at the baud the bench measures takes
    /// a fraction of a tick to swallow 27 bytes, and a port that stalls is a fault the health
    /// verdict should see as consecutive write errors.
    fn write_frame(&mut self, bytes: &[u8]) -> std::io::Result<()>;

    /// Whatever the body has pushed since the last call, up to `buf.len()` bytes; `0` when nothing.
    /// Must not block the tick: the serial port is opened with a millisecond timeout for this.
    fn read_available(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
}

/// The real thing.
pub struct Serial {
    port: Box<dyn serialport::SerialPort>,
}

/// How long one read may wait for bytes. A tick is 10 ms; this is a tenth of it.
const READ_TIMEOUT: Duration = Duration::from_millis(1);

impl Serial {
    /// Open `path` at `baud`, 8N1. The baud is a flag rather than a constant because the PoC
    /// inherited the driver's termios and never learned the number; the bench measures it with
    /// `stty` before the stock firmware is stopped.
    pub fn open(path: &str, baud: u32) -> std::io::Result<Self> {
        let port = serialport::new(path, baud)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .flow_control(serialport::FlowControl::None)
            .timeout(READ_TIMEOUT)
            .open()
            .map_err(std::io::Error::other)?;
        Ok(Self { port })
    }
}

impl S1Body for Serial {
    fn write_frame(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.port.write_all(bytes)?;
        self.port.flush()
    }

    fn read_available(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.port.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(0),
            Err(e) => Err(e),
        }
    }
}

/// A body made of a file: frames are appended as written, nothing is ever read back.
///
/// What `--fake <path>` runs against, and what the tests decode afterwards with `s1_duml::Decoder`
/// to see exactly what the loop would have put on the wire.
pub struct FakeFile {
    file: std::fs::File,
    /// Bytes to hand back on `read_available`, for a test that wants the loop to see telemetry.
    pub inbound: Vec<u8>,
}

impl FakeFile {
    pub fn create(path: &std::path::Path) -> std::io::Result<Self> {
        Ok(Self {
            file: std::fs::File::create(path)?,
            inbound: Vec::new(),
        })
    }
}

impl S1Body for FakeFile {
    fn write_frame(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.file.write_all(bytes)?;
        self.file.flush()
    }

    fn read_available(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inbound.len().min(buf.len());
        buf[..n].copy_from_slice(&self.inbound[..n]);
        self.inbound.drain(..n);
        Ok(n)
    }
}
