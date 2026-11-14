//! Client side: `dphone <command> [args...]`.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;

use super::{FRAME_EXIT, FRAME_STDERR, FRAME_STDOUT, encode_request, read_frame};

/// Send `args` to the main role at `socket` and relay the reply.  Returns
/// the exit status for this process.
pub fn run(socket: &Path, args: &[String]) -> i32 {
    let mut stream = match UnixStream::connect(socket) {
        Ok(s) => s,
        Err(e) => {
            crate::errln!("dphone: cannot reach the main role at {}: {e}", socket.display());
            return 1;
        }
    };
    if let Err(e) = stream.write_all(&encode_request(args)) {
        crate::errln!("dphone: {e}");
        return 1;
    }
    loop {
        match read_frame(&mut stream) {
            Ok(Some((FRAME_STDOUT, data))) => {
                crate::out::stdout_bytes(&data);
                // Nobody is reading (e.g., `dphone log -f | head`): stop,
                // which also ends a command that follows something forever.
                if crate::out::stdout_gone() {
                    return 1;
                }
            }
            Ok(Some((FRAME_STDERR, data))) => crate::out::stderr_bytes(&data),
            Ok(Some((FRAME_EXIT, data))) => {
                return match <[u8; 4]>::try_from(&data[..]) {
                    Ok(b) => u32::from_be_bytes(b) as i32,
                    Err(_) => 1,
                };
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                crate::errln!("dphone: the main role closed the connection");
                return 1;
            }
            Err(e) => {
                crate::errln!("dphone: {e}");
                return 1;
            }
        }
    }
}
