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
            eprintln!("dphone: cannot reach the main role at {}: {e}", socket.display());
            return 1;
        }
    };
    if let Err(e) = stream.write_all(&encode_request(args)) {
        eprintln!("dphone: {e}");
        return 1;
    }
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    loop {
        match read_frame(&mut stream) {
            Ok(Some((FRAME_STDOUT, data))) => {
                let mut out = stdout.lock();
                if out.write_all(&data).and_then(|_| out.flush()).is_err() {
                    return 1;
                }
            }
            Ok(Some((FRAME_STDERR, data))) => {
                let _ = stderr.lock().write_all(&data);
            }
            Ok(Some((FRAME_EXIT, data))) => {
                return match <[u8; 4]>::try_from(&data[..]) {
                    Ok(b) => u32::from_be_bytes(b) as i32,
                    Err(_) => 1,
                };
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                eprintln!("dphone: the main role closed the connection");
                return 1;
            }
            Err(e) => {
                eprintln!("dphone: {e}");
                return 1;
            }
        }
    }
}
