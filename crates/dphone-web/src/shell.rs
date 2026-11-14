//! The web shell's server side: a shell on a pseudo-terminal per session.
//!
//! [`ShellHost`] starts the process; everything else (reading, writing,
//! resizing, ending) works on the terminal's master side and the child
//! handle, so an integrator's host only decides what to run and with which
//! environment.  [`PtyShellHost`] is the host used when none is supplied.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::unix::AsyncFd;
use tokio::io::Interest;
use tokio::sync::Notify;

/// Shells that may be open at once.
pub const MAX_SHELLS: usize = 2;
/// A shell with no input for this long is ended.
pub const SHELL_IDLE: Duration = Duration::from_secs(30 * 60);
/// A shell whose page has not been heard from for this long is ended.  The
/// page sends input, or an empty write as a heartbeat, far more often; a
/// connection that died without closing (the network dropped, the browser
/// froze the page) is not otherwise noticed while the shell prints nothing.
pub const CLIENT_SILENCE: Duration = Duration::from_secs(75);
/// Largest input accepted in one write.
pub const MAX_INPUT: usize = 64 << 10;

/// A terminal's size in character cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellSize {
    pub cols: u16,
    pub rows: u16,
}

impl ShellSize {
    /// Clamps a requested size to something a program can live with.
    pub fn clamped(cols: u32, rows: u32) -> Self {
        let pick = |v: u32, default: u16, max: u32| if v == 0 { default } else { v.clamp(2, max) as u16 };
        ShellSize { cols: pick(cols, 80, 500), rows: pick(rows, 24, 200) }
    }

    fn winsize(self) -> libc::winsize {
        libc::winsize { ws_row: self.rows, ws_col: self.cols, ws_xpixel: 0, ws_ypixel: 0 }
    }
}

/// A started shell: the master side of its terminal and the process, which
/// leads its own session with the terminal as controlling terminal.
pub struct ShellProcess {
    pub master: OwnedFd,
    pub child: Child,
}

/// Starts shells for the web terminal.
pub trait ShellHost: Send + Sync {
    fn spawn(&self, size: ShellSize) -> io::Result<ShellProcess>;
}

/// The default [`ShellHost`]: runs the first installed of a list of
/// programs on a new pseudo-terminal.
#[derive(Clone, Debug)]
pub struct PtyShellHost {
    programs: Vec<PathBuf>,
    clear_env: bool,
    env: Vec<(OsString, OsString)>,
    dir: Option<PathBuf>,
}

impl Default for PtyShellHost {
    /// `$DPHONE_SHELL` if set (a simulator convenience, as for SSH
    /// sessions), else `/bin/brush`, else `/bin/sh`; the server's own
    /// environment; `$HOME` as the working directory.
    fn default() -> Self {
        let mut programs = Vec::new();
        if let Some(s) = std::env::var_os("DPHONE_SHELL").filter(|s| !s.is_empty()) {
            programs.push(PathBuf::from(s));
        }
        programs.push("/bin/brush".into());
        programs.push("/bin/sh".into());
        PtyShellHost { programs, clear_env: false, env: Vec::new(), dir: std::env::var_os("HOME").map(PathBuf::from) }
    }
}

impl PtyShellHost {
    /// Runs `program`.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        PtyShellHost { programs: vec![program.into()], clear_env: false, env: Vec::new(), dir: None }
    }

    /// Starts from an empty environment instead of the server's.
    pub fn env_clear(mut self) -> Self {
        self.clear_env = true;
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn envs<K: Into<OsString>, V: Into<OsString>>(mut self, vars: impl IntoIterator<Item = (K, V)>) -> Self {
        self.env.extend(vars.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    /// The working directory (`/` when it does not exist).
    pub fn current_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dir = Some(dir.into());
        self
    }
}

impl ShellHost for PtyShellHost {
    fn spawn(&self, size: ShellSize) -> io::Result<ShellProcess> {
        let program = self
            .programs
            .iter()
            .find(|p| p.exists())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no shell installed"))?;
        let (master, slave) = open_pty(size)?;
        let mut cmd = Command::new(program);
        if self.clear_env {
            cmd.env_clear();
        }
        cmd.env("TERM", "xterm-256color");
        cmd.envs(self.env.iter().map(|(k, v)| (k, v)));
        cmd.current_dir(self.dir.as_ref().filter(|d| d.is_dir()).cloned().unwrap_or_else(|| "/".into()));
        cmd.stdin(Stdio::from(slave.try_clone()?));
        cmd.stdout(Stdio::from(slave.try_clone()?));
        cmd.stderr(Stdio::from(slave));
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                // A server started under `nohup` ignores hang-ups, and one
                // started in the background by a script ignores interrupt
                // and quit; an ignored signal stays ignored across exec.
                // The shell and its jobs must end when the terminal goes
                // away, and Ctrl-C must interrupt what runs in it.
                for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGQUIT] {
                    libc::signal(signal, libc::SIG_DFL);
                }
                Ok(())
            });
        }
        let child = cmd.spawn()?;
        // Dropping `cmd` closes our copies of the slave side, so the master
        // reports hang-up once the session's processes are gone.
        drop(cmd);
        Ok(ShellProcess { master, child })
    }
}

/// Opens a pseudo-terminal pair of the given size.  Both descriptors are
/// close-on-exec; the child gets the slave through its standard streams.
fn open_pty(size: ShellSize) -> io::Result<(OwnedFd, OwnedFd)> {
    let mut master = -1;
    let mut slave = -1;
    let ws = size.winsize();
    // SAFETY: plain libc call with valid out-pointers.
    let r = unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), &ws) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openpty returned two descriptors we now own.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    for fd in [&master, &slave] {
        // SAFETY: fcntl on a descriptor we own.
        unsafe {
            let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFD);
            libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, flags | libc::FD_CLOEXEC);
        }
    }
    Ok((master, slave))
}

/// How a shell ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ShellExit {
    pub code: i32,
    pub signal: i32,
    /// Set when the server ended the session ("idle", "closed", ...).
    pub reason: &'static str,
}

impl ShellExit {
    fn from_status(status: ExitStatus) -> Self {
        ShellExit { code: status.code().unwrap_or(0), signal: status.signal().unwrap_or(0), reason: "" }
    }
}

/// One open shell.
pub(crate) struct Session {
    pub id: String,
    /// The web session that opened it; only that session may write to it.
    pub owner: String,
    master: AsyncFd<OwnedFd>,
    child: Mutex<Child>,
    last_input: Mutex<Instant>,
    last_heard: Mutex<Instant>,
    close: Notify,
}

impl Session {
    /// Reads what the shell printed: `Ok(0)` when every process holding
    /// the terminal is gone.
    pub async fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut guard = self.master.readable().await?;
            let r = guard.try_io(|fd| {
                // SAFETY: reading into a buffer we own.
                let n = unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
                if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
            });
            match r {
                Ok(Ok(n)) => return Ok(n),
                // Linux reports a hung-up terminal as EIO.
                Ok(Err(e)) if e.raw_os_error() == Some(libc::EIO) => return Ok(0),
                Ok(Err(e)) if e.kind() == io::ErrorKind::Interrupted => {}
                Ok(Err(e)) => return Err(e),
                Err(_would_block) => {}
            }
        }
    }

    /// Reads without waiting; `None` when nothing is ready.
    pub fn try_read(&self, buf: &mut [u8]) -> Option<usize> {
        // SAFETY: reading into a buffer we own; the descriptor is non-blocking.
        let n = unsafe { libc::read(self.master.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
        (n > 0).then_some(n as usize)
    }

    /// Sends input to the shell.  When the terminal's input queue is full
    /// (a few kilobytes; the program has to read it first), this waits and
    /// tries again, yielding to the reactor so the output side keeps
    /// draining: a program echoing a large paste would otherwise block on
    /// its own output and never read the rest.
    pub async fn write(&self, mut data: &[u8]) -> io::Result<()> {
        self.heard();
        if !data.is_empty() {
            *self.last_input.lock().unwrap() = Instant::now();
        }
        while !data.is_empty() {
            // SAFETY: writing from a buffer we borrow; the descriptor is
            // non-blocking.
            let n = unsafe { libc::write(self.master.as_raw_fd(), data.as_ptr().cast(), data.len()) };
            if n > 0 {
                data = &data[n as usize..];
                continue;
            }
            if n < 0 {
                let e = io::Error::last_os_error();
                if !matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) {
                    return Err(e);
                }
            }
            // Full (a pseudo-terminal reports that as a short or empty
            // write).  Its readiness events are not dependable for this,
            // so poll.
            tokio::time::sleep(Duration::from_millis(4)).await;
        }
        Ok(())
    }

    /// Changes the terminal's size; the kernel signals the foreground
    /// program (SIGWINCH).
    pub fn resize(&self, size: ShellSize) -> io::Result<()> {
        self.heard();
        let ws = size.winsize();
        // SAFETY: ioctl with a valid winsize on a descriptor we own.
        let r = unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &ws) };
        if r < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    /// Asks the stream task to end the session.
    pub fn request_close(&self) {
        self.close.notify_one();
    }

    pub async fn close_requested(&self) {
        self.close.notified().await
    }

    pub fn idle_for(&self) -> Duration {
        self.last_input.lock().unwrap().elapsed()
    }

    /// Notes that the page is still there.
    pub fn heard(&self) {
        *self.last_heard.lock().unwrap() = Instant::now();
    }

    pub fn silent_for(&self) -> Duration {
        self.last_heard.lock().unwrap().elapsed()
    }

    /// The shell's exit status, once it has exited.
    pub fn try_exit(&self) -> Option<ShellExit> {
        match self.child.lock().unwrap().try_wait() {
            Ok(Some(status)) => Some(ShellExit::from_status(status)),
            Ok(None) => None,
            // Someone else reaped it (a process-wide reaper): gone, status unknown.
            Err(_) => Some(ShellExit::default()),
        }
    }

    fn signal_group(&self, signal: libc::c_int) {
        let pid = self.child.lock().unwrap().id() as libc::pid_t;
        signal_session(pid, signal);
    }

    /// Ends the shell and whatever it started: hang up, wait briefly, then
    /// kill; the child is reaped either way.
    pub async fn terminate(&self) -> ShellExit {
        if let Some(exit) = self.try_exit() {
            // Stragglers that outlived the shell in its process group.
            self.signal_group(libc::SIGHUP);
            return exit;
        }
        self.signal_group(libc::SIGHUP);
        self.signal_group(libc::SIGCONT);
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if let Some(exit) = self.try_exit() {
                return exit;
            }
        }
        self.signal_group(libc::SIGKILL);
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if let Some(exit) = self.try_exit() {
                return exit;
            }
        }
        ShellExit::default()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // The backstop if the stream task was torn down without running
        // `terminate`: never leave a root shell behind.
        let child = self.child.get_mut().unwrap();
        if let Ok(None) = child.try_wait() {
            signal_session(child.id() as libc::pid_t, libc::SIGKILL);
            let _ = child.wait();
        }
    }
}

/// Signals every process of the session that the shell `leader` leads.
///
/// The shell's own process group is not enough.  A shell with job control
/// puts each command in a group of its own, the hang-up from closing the
/// terminal reaches only the foreground group, and whether a shell passes a
/// hang-up on to its background jobs differs from shell to shell (bash does;
/// ash does not).  The session is what they all share.
fn signal_session(leader: libc::pid_t, signal: libc::c_int) {
    // SAFETY: kill on a process group we started.
    unsafe {
        libc::kill(-leader, signal);
    }
    let Ok(entries) = std::fs::read_dir("/proc") else { return };
    for pid in entries.filter_map(|e| e.ok()?.file_name().to_str()?.parse::<libc::pid_t>().ok()) {
        // "PID (COMM) STATE PPID PGRP SESSION ..."; COMM may hold spaces and
        // parentheses, so the fields are counted from its last ')'.
        let session = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok().and_then(|stat| {
            stat.rsplit_once(')')?.1.split_ascii_whitespace().nth(3)?.parse::<libc::pid_t>().ok()
        });
        if session == Some(leader) {
            // SAFETY: kill on a process of the session we started.
            unsafe {
                libc::kill(pid, signal);
            }
        }
    }
}

/// The open shells.
pub(crate) struct Shells {
    host: Arc<dyn ShellHost>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

/// Why a shell could not be opened.
pub(crate) enum OpenError {
    TooMany,
    Spawn(io::Error),
}

impl Shells {
    pub fn new(host: Arc<dyn ShellHost>) -> Self {
        Shells { host, sessions: Mutex::new(HashMap::new()) }
    }

    /// Open terminals.
    pub fn count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    /// Starts a shell for a web session.  Must run inside the tokio
    /// runtime (the terminal is registered with its reactor).
    pub fn open(&self, owner: &str, size: ShellSize) -> Result<Arc<Session>, OpenError> {
        let mut sessions = self.sessions.lock().unwrap();
        if sessions.len() >= MAX_SHELLS {
            return Err(OpenError::TooMany);
        }
        let ShellProcess { master, child } = self.host.spawn(size).map_err(OpenError::Spawn)?;
        // SAFETY: fcntl on a descriptor we own.
        unsafe {
            let flags = libc::fcntl(master.as_raw_fd(), libc::F_GETFL);
            libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        // SAFETY: `master` is an open descriptor that the `AsyncFd` owns for
        // its whole life; `OwnedFd::as_raw_fd` always returns that descriptor.
        let master = unsafe { AsyncFd::register_with_interest(master, Interest::READABLE) }
            .map_err(|e| OpenError::Spawn(e.into()))?;
        let session = Arc::new(Session {
            id: crate::auth::random_token(),
            owner: owner.to_owned(),
            master,
            child: Mutex::new(child),
            last_input: Mutex::new(Instant::now()),
            last_heard: Mutex::new(Instant::now()),
            close: Notify::new(),
        });
        sessions.insert(session.id.clone(), session.clone());
        Ok(session)
    }

    /// The session with this id, if `owner` opened it.
    pub fn get(&self, id: &str, owner: &str) -> Option<Arc<Session>> {
        self.sessions.lock().unwrap().get(id).filter(|s| s.owner == owner).cloned()
    }

    pub fn remove(&self, id: &str) {
        self.sessions.lock().unwrap().remove(id);
    }
}
