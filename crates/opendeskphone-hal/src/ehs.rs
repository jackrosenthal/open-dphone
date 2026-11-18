//! Electronic hook switch (EHS) adapter on the serial port.
//!
//! The port (`/dev/ttyAMA0`) is shared with the serial console, so EHS and
//! the console are mutually exclusive.  The traits here are the byte
//! transport plus line settings, shared by both backends; the two adapter
//! protocols are in [`jabra`] and [`pehs`], and [`run`] is the detection
//! loop over them.

use crate::Result;
use std::time::Duration;

/// Serial line settings for an EHS adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SerialConfig {
    pub baud: u32,
    /// 7 or 8.
    pub data_bits: u8,
    pub parity: Parity,
    /// 1 or 2.
    pub stop_bits: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parity {
    None,
    Even,
    Odd,
}

impl Default for SerialConfig {
    fn default() -> Self {
        SerialConfig { baud: 9600, data_bits: 8, parity: Parity::None, stop_bits: 1 }
    }
}

/// An open EHS serial port.
pub trait EhsPort: Send {
    /// Reads up to `buf.len()` bytes; `Ok(0)` on timeout.
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize>;
    fn write(&mut self, data: &[u8]) -> Result<()>;
    /// Changes line settings.
    fn configure(&mut self, config: SerialConfig) -> Result<()>;
    /// Sets the modem-control outputs (some adapters signal on DTR/RTS).
    fn set_lines(&mut self, dtr: bool, rts: bool) -> Result<()>;
}

/// Access to the EHS port.
pub trait Ehs: Send {
    /// Opens the port, taking it away from the console.
    fn open(&mut self, config: SerialConfig) -> Result<Box<dyn EhsPort>>;
}

impl SerialConfig {
    /// Plantronics adapters: 9600 baud, 8 data bits, no parity, one stop bit.
    pub const PLANTRONICS: SerialConfig =
        SerialConfig { baud: 9600, data_bits: 8, parity: Parity::None, stop_bits: 1 };
    /// Jabra adapters: 2400 baud, 8 data bits, no parity, two stop bits.
    pub const JABRA: SerialConfig =
        SerialConfig { baud: 2400, data_bits: 8, parity: Parity::None, stop_bits: 2 };
}

/// Plantronics EHS framing, as recovered from the stock `ehs` daemon:
///
/// ```text
/// 02  HEX_LENGTH[2]  HEADER1  HEADER2  PAYLOAD[n]  HEX_CHECKSUM[2]  03
/// length = n + 6; total frame = n + 8 bytes
/// ```
///
/// Only the framing is here; what the packets mean is in [`pehs`].
pub mod plantronics {
    pub const STX: u8 = 0x02;
    pub const ETX: u8 = 0x03;
    /// Header bytes of the detection packet the stock daemon sends first.
    pub const DETECT: (u8, u8) = (0x3f, 0x50);
    /// Largest payload a two-digit hexadecimal length can describe.
    pub const MAX_PAYLOAD: usize = 0xff - 6;

    /// One decoded packet.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Packet {
        pub header: (u8, u8),
        pub payload: Vec<u8>,
    }

    /// The stock checksum: not a CRC or a plain sum.  Start with the first
    /// header byte; add the second and each payload byte; after any
    /// addition that exceeds 255 replace the accumulator with
    /// `(acc & 0x7f) << 1`; return the complement.
    pub fn checksum(header: (u8, u8), payload: &[u8]) -> u8 {
        let mut acc = header.0 as u32;
        for b in std::iter::once(header.1).chain(payload.iter().copied()) {
            acc += b as u32;
            if acc > 0xff {
                acc = (acc & 0x7f) << 1;
            }
        }
        !(acc as u8)
    }

    fn hex2(v: u8) -> [u8; 2] {
        const D: &[u8; 16] = b"0123456789ABCDEF";
        [D[(v >> 4) as usize], D[(v & 15) as usize]]
    }

    fn unhex2(b: &[u8]) -> Option<u8> {
        let d = |c: u8| (c as char).to_digit(16).map(|v| v as u8);
        Some(d(b[0])? << 4 | d(b[1])?)
    }

    /// Encodes a packet.  `None` if the payload is too long.  Hexadecimal
    /// digits are written in upper case (the stock format string was not
    /// recovered; the parser here accepts either case).
    pub fn encode(header: (u8, u8), payload: &[u8]) -> Option<Vec<u8>> {
        if payload.len() > MAX_PAYLOAD {
            return None;
        }
        let mut v = Vec::with_capacity(payload.len() + 8);
        v.push(STX);
        v.extend(hex2(payload.len() as u8 + 6));
        v.push(header.0);
        v.push(header.1);
        v.extend_from_slice(payload);
        v.extend(hex2(checksum(header, payload)));
        v.push(ETX);
        Some(v)
    }

    /// Why a frame was rejected.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum FrameError {
        BadLength,
        MissingEtx,
        BadChecksum,
    }

    /// Incremental frame parser.  Feed received bytes; packets and errors
    /// come out in order.  Bytes before an STX are skipped.
    #[derive(Default)]
    pub struct Parser {
        buf: Vec<u8>,
    }

    impl Parser {
        pub fn new() -> Self {
            Self::default()
        }

        /// Adds received bytes and returns everything they complete.
        pub fn feed(&mut self, data: &[u8]) -> Vec<Result<Packet, FrameError>> {
            self.buf.extend_from_slice(data);
            let mut out = Vec::new();
            loop {
                match self.buf.iter().position(|b| *b == STX) {
                    Some(i) => drop(self.buf.drain(..i)),
                    None => {
                        self.buf.clear();
                        break;
                    }
                }
                if self.buf.len() < 3 {
                    break;
                }
                let Some(len) = unhex2(&self.buf[1..3]).filter(|l| *l >= 6) else {
                    out.push(Err(FrameError::BadLength));
                    self.buf.drain(..1);
                    continue;
                };
                let total = len as usize + 2;
                if self.buf.len() < total {
                    break;
                }
                let frame: Vec<u8> = self.buf.drain(..total).collect();
                if frame[total - 1] != ETX {
                    out.push(Err(FrameError::MissingEtx));
                    continue;
                }
                let header = (frame[3], frame[4]);
                let payload = &frame[5..total - 3];
                match unhex2(&frame[total - 3..total - 1]) {
                    Some(c) if c == checksum(header, payload) => {
                        out.push(Ok(Packet { header, payload: payload.to_vec() }))
                    }
                    _ => out.push(Err(FrameError::BadChecksum)),
                }
            }
            out
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn checksum_rule() {
            // 0x3f + 0x50 = 0x8f, no overflow: complement is 0x70.
            assert_eq!(checksum(DETECT, &[]), 0x70);
            // 0xf0 + 0x20 = 0x110 > 0xff -> (0x10) << 1 = 0x20 -> !0x20.
            assert_eq!(checksum((0xf0, 0x20), &[]), 0xdf);
            // Then + 0xff = 0x11f -> (0x1f) << 1 = 0x3e -> !0x3e.
            assert_eq!(checksum((0xf0, 0x20), &[0xff]), 0xc1);
        }

        #[test]
        fn round_trip_and_errors() {
            let frame = encode(DETECT, b"\x02ab\x03").unwrap();
            assert_eq!(frame.len(), 4 + 8);
            assert_eq!(&frame[..3], b"\x020A");
            let mut p = Parser::new();
            let mut got = Vec::new();
            got.extend(p.feed(b"noise"));
            for chunk in frame.chunks(3) {
                got.extend(p.feed(chunk));
            }
            assert_eq!(got, vec![Ok(Packet { header: DETECT, payload: b"\x02ab\x03".to_vec() })]);

            let mut bad = encode((1, 2), b"xyz").unwrap();
            let n = bad.len();
            bad[n - 2] ^= 1;
            assert_eq!(p.feed(&bad), vec![Err(FrameError::BadChecksum)]);
            let mut bad = encode((1, 2), b"xyz").unwrap();
            bad[n - 1] = 0;
            assert_eq!(p.feed(&bad), vec![Err(FrameError::MissingEtx)]);
            // A bad length field resynchronizes on the next STX.
            let mut stream = b"\x02ZZ".to_vec();
            stream.extend(encode((9, 8), b"").unwrap());
            assert_eq!(
                p.feed(&stream),
                vec![Err(FrameError::BadLength), Ok(Packet { header: (9, 8), payload: vec![] })]
            );
            // Lower-case hex is accepted.
            let mut lower = encode((0xf0, 0x20), &[0xff; 5]).unwrap();
            lower.make_ascii_lowercase();
            assert!(matches!(p.feed(&lower)[..], [Ok(_)]));
            assert!(encode((0, 0), &[0; 250]).is_none());
        }
    }
}

mod link;
pub mod jabra;
pub mod pehs;
pub use link::{run, Adapter, EhsHost, EhsMode, HeadsetAction, Identity, PhoneStatus, Timing};

#[cfg(test)]
mod engine_tests;
