//! Diagnostics over a local Unix socket.
//!
//! `dphone <command> [args...]` connects to the running main role, sends the
//! command, and relays the reply to its own stdout, stderr, and exit status.
//! Subsystems register their commands in the [`Registry`].
//!
//! ## Wire protocol
//!
//! Every frame is one type byte, a big-endian `u32` length, and that many
//! payload bytes.
//!
//! | Type | Direction | Payload |
//! | --- | --- | --- |
//! | `r` | client to server, once | arguments joined by NUL bytes; the first is the command name |
//! | `o` | server to client | bytes for stdout |
//! | `e` | server to client | bytes for stderr |
//! | `x` | server to client, last | exit status as a big-endian `u32` |
//!
//! The server closes the connection after `x`.  A client that goes away
//! early (e.g., Ctrl-C on `dphone log -f`) cancels the command.

pub mod builtin;
pub mod client;
pub mod server;

use std::io::{self, Read, Write};

pub use server::{Output, Registry};

pub const FRAME_REQUEST: u8 = b'r';
pub const FRAME_STDOUT: u8 = b'o';
pub const FRAME_STDERR: u8 = b'e';
pub const FRAME_EXIT: u8 = b'x';

/// Largest payload either side accepts.
pub const MAX_FRAME: usize = 1 << 20;

/// Exit status for "the main role does not know this command".
pub const EXIT_UNKNOWN: i32 = 127;
/// Exit status for "this command exists but its subsystem is not wired in".
pub const EXIT_UNAVAILABLE: i32 = 69;

pub fn encode_frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn encode_request(args: &[String]) -> Vec<u8> {
    encode_frame(FRAME_REQUEST, args.join("\0").as_bytes())
}

pub fn decode_request(payload: &[u8]) -> Option<Vec<String>> {
    let text = std::str::from_utf8(payload).ok()?;
    if text.is_empty() {
        return None;
    }
    Some(text.split('\0').map(str::to_string).collect())
}

/// Blocking frame write.
pub fn write_frame(w: &mut impl Write, kind: u8, payload: &[u8]) -> io::Result<()> {
    w.write_all(&encode_frame(kind, payload))
}

/// Blocking frame read.  `Ok(None)` at a clean end of stream.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut head = [0u8; 5];
    match r.read(&mut head[..1])? {
        0 => return Ok(None),
        _ => r.read_exact(&mut head[1..])?,
    }
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    Ok(Some((head[0], payload)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, FRAME_STDOUT, b"hello").unwrap();
        write_frame(&mut buf, FRAME_EXIT, &3u32.to_be_bytes()).unwrap();
        let mut r = &buf[..];
        assert_eq!(read_frame(&mut r).unwrap(), Some((FRAME_STDOUT, b"hello".to_vec())));
        assert_eq!(read_frame(&mut r).unwrap(), Some((FRAME_EXIT, vec![0, 0, 0, 3])));
        assert_eq!(read_frame(&mut r).unwrap(), None);
    }

    #[test]
    fn requests() {
        let args = vec!["config".to_string(), "get".to_string(), "a b".to_string()];
        let frame = encode_request(&args);
        assert_eq!(frame[0], FRAME_REQUEST);
        assert_eq!(decode_request(&frame[5..]), Some(args));
        assert_eq!(decode_request(b""), None);
    }
}
