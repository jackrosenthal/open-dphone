//! Client side: `dphone <command> [args...]`.

use std::io::Write;
use std::os::fd::AsRawFd;
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
        // Wait for the next frame or for standard output to lose its reader,
        // whichever is first.  A failed write would also show the latter,
        // but a command that follows something quiet (`dphone log -f | head
        // -1` on an idle phone) may never write again.  (An ordinary command
        // still gets a moment to finish, so that its exit status survives.)
        if !wait_for_frame(&stream) {
            return 1;
        }
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

/// How long a command whose output nobody reads any more is still given to
/// finish.  An ordinary command answers well within this and keeps its exit
/// status; one that follows something never finishes, and is ended.
const UNREAD_GRACE_MS: libc::c_int = 1000;

/// Block until `stream` has something to read (or has ended).  Returns false
/// if standard output lost its reader and nothing arrived for
/// [`UNREAD_GRACE_MS`] after that.
fn wait_for_frame(stream: &UnixStream) -> bool {
    let mut fds = [
        libc::pollfd { fd: stream.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        // No events asked for: the kernel still reports an error (a pipe
        // with no reader) or a hang-up (a terminal or socket that went away).
        // A file, or a pipe somebody still reads, reports nothing.
        libc::pollfd { fd: libc::STDOUT_FILENO, events: 0, revents: 0 },
    ];
    let mut unread = crate::out::stdout_gone();
    loop {
        // Once the reader is known to be gone, only the stream is watched
        // (the gone reader would otherwise end every poll at once).
        let (count, timeout) = if unread { (1, UNREAD_GRACE_MS) } else { (2, -1) };
        // SAFETY: fds is a valid array of two pollfd.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), count, timeout) };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            // Cannot wait; fall back to the blocking read.
            return true;
        }
        if fds[0].revents != 0 {
            return true;
        }
        if unread {
            return false;
        }
        unread = fds[1].revents & (libc::POLLERR | libc::POLLHUP) != 0;
    }
}
