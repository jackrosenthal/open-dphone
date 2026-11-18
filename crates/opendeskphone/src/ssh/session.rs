//! One SSH connection: authentication and its session channels.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use log::{debug, info, warn};
use russh::keys::PublicKey;
use russh::server::{Auth, Msg, Session};
use russh::{Channel, ChannelId, ChannelMsg, MethodKind, MethodSet, Pty, Sig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use super::pty::{self, AsyncMaster};
use crate::mainrole::Context;

/// The only login name.  The phone has one user.
pub const USER: &str = "root";

/// After the process exits, how long output from lingering background
/// children is still relayed.
const LINGER: Duration = Duration::from_millis(300);

enum Control {
    Resize(u16, u16),
    Signal(libc::c_int),
}

#[derive(Default)]
struct ChannelState {
    channel: Option<Channel<Msg>>,
    pty: Option<(String, u16, u16)>,
    env: Vec<(String, String)>,
    control: Option<mpsc::UnboundedSender<Control>>,
}

pub struct Handler {
    ctx: Context,
    peer: SocketAddr,
    authed: Arc<AtomicBool>,
    channels: HashMap<ChannelId, ChannelState>,
}

impl Handler {
    pub fn new(ctx: Context, peer: SocketAddr, authed: Arc<AtomicBool>) -> Self {
        Handler { ctx, peer, authed, channels: HashMap::new() }
    }

    fn reject() -> Auth {
        let mut methods = MethodSet::empty();
        methods.push(MethodKind::PublicKey);
        Auth::Reject { proceed_with_methods: Some(methods), partial_success: false }
    }

    fn check_key(&self, user: &str, key: &PublicKey) -> bool {
        user == USER && super::keys::is_authorized(&super::authorized_key_files(&self.ctx.paths), key)
    }

    /// Start a shell (`command` is `None`) or run a command on a channel.
    fn start_process(&mut self, id: ChannelId, command: Option<String>) -> std::io::Result<()> {
        let state = self.channels.get_mut(&id).ok_or_else(|| std::io::Error::other("no such channel"))?;
        let channel = state.channel.take().ok_or_else(|| std::io::Error::other("channel already in use"))?;
        let shell = find_shell(self.ctx.shell.as_deref()).ok_or_else(|| std::io::Error::other("no shell installed"))?;

        let mut cmd = Command::new(&shell);
        if let Some(command) = &command {
            cmd.arg("-c").arg(command);
        }
        if !self.ctx.sim {
            cmd.env_clear();
            cmd.envs(crate::init::shell_environment(&self.ctx.paths, &shell));
        } else if self.ctx.paths.home().is_dir() {
            cmd.env("HOME", self.ctx.paths.home());
        }
        cmd.env(crate::paths::DIAG_SOCKET_ENV, self.ctx.paths.diag_socket());
        cmd.env("SSH_CONNECTION", format!("{} {} 0.0.0.0 22", self.peer.ip(), self.peer.port()));
        for (k, v) in &state.env {
            cmd.env(k, v);
        }
        let home = self.ctx.paths.home();
        cmd.current_dir(if home.is_dir() { home } else { "/".into() });
        cmd.kill_on_drop(true);

        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let peer = self.peer;
        let what = command.clone().unwrap_or_else(|| "shell".to_string());
        match state.pty.clone() {
            Some((term, cols, rows)) => {
                let pair = pty::open(cols, rows)?;
                cmd.env("TERM", term);
                cmd.stdin(Stdio::from(pair.slave.try_clone()?));
                cmd.stdout(Stdio::from(pair.slave.try_clone()?));
                cmd.stderr(Stdio::from(pair.slave));
                // SAFETY: only async-signal-safe calls between fork and exec.
                unsafe {
                    cmd.pre_exec(|| {
                        if libc::setsid() < 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                        libc::ioctl(0, libc::TIOCSCTTY as _, 0);
                        Ok(())
                    });
                }
                let child = cmd.spawn()?;
                // Drop our copies of the slave so the master sees hang-up
                // when the session's processes are gone.
                drop(cmd);
                let master = AsyncMaster::new(pair.master)?;
                info!("ssh: {peer}: {what} (pty, pid {})", child.id().unwrap_or(0));
                tokio::spawn(run_pty(channel, child, master, control_rx));
            }
            None => {
                cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
                cmd.process_group(0);
                let child = cmd.spawn()?;
                drop(cmd);
                info!("ssh: {peer}: {what} (pid {})", child.id().unwrap_or(0));
                tokio::spawn(run_piped(channel, child, control_rx));
            }
        }
        state.control = Some(control_tx);
        Ok(())
    }
}

/// The shell for sessions: `configured` (`dphone main --shell`) if given,
/// else the first installed of [`crate::init::SHELLS`].
pub fn find_shell(configured: Option<&str>) -> Option<String> {
    if let Some(s) = configured.filter(|s| !s.is_empty()) {
        return Some(s.to_string());
    }
    crate::init::SHELLS.iter().find(|s| std::path::Path::new(s).exists()).map(|s| s.to_string())
}

/// Client environment variables we pass on, as OpenSSH's default
/// `AcceptEnv` does.
fn env_allowed(name: &str) -> bool {
    name == "LANG" || name == "COLORTERM" || name.starts_with("LC_")
}

fn signal_number(sig: &Sig) -> Option<libc::c_int> {
    Some(match sig {
        Sig::ABRT => libc::SIGABRT,
        Sig::ALRM => libc::SIGALRM,
        Sig::FPE => libc::SIGFPE,
        Sig::HUP => libc::SIGHUP,
        Sig::ILL => libc::SIGILL,
        Sig::INT => libc::SIGINT,
        Sig::KILL => libc::SIGKILL,
        Sig::PIPE => libc::SIGPIPE,
        Sig::QUIT => libc::SIGQUIT,
        Sig::SEGV => libc::SIGSEGV,
        Sig::TERM => libc::SIGTERM,
        Sig::USR1 => libc::SIGUSR1,
        Sig::Custom(_) => return None,
    })
}

fn exit_code(status: std::process::ExitStatus) -> u32 {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(code), _) => code as u32,
        (None, Some(sig)) => 128 + sig as u32,
        (None, None) => 255,
    }
}

/// Signal the session's process group.
fn signal_group(child: &Child, sig: libc::c_int) {
    if let Some(pid) = child.id() {
        let _ = crate::sys::kill(-(pid as libc::pid_t), sig);
    }
}

/// End a session whose client has gone: hang up, then kill.
async fn hang_up(child: &mut Child) -> Option<std::process::ExitStatus> {
    signal_group(child, libc::SIGHUP);
    if let Ok(status) = tokio::time::timeout(Duration::from_secs(1), child.wait()).await {
        return status.ok();
    }
    signal_group(child, libc::SIGKILL);
    child.wait().await.ok()
}

async fn finish(channel: Channel<Msg>, status: Option<std::process::ExitStatus>) {
    if let Some(status) = status {
        let _ = channel.exit_status(exit_code(status)).await;
    }
    let _ = channel.eof().await;
    let _ = channel.close().await;
}

async fn run_pty(
    mut channel: Channel<Msg>,
    mut child: Child,
    master: AsyncMaster,
    mut control: mpsc::UnboundedReceiver<Control>,
) {
    let mut buf = vec![0u8; 8192];
    let mut exit = None;
    let mut client_gone = false;
    let linger = tokio::time::sleep(Duration::MAX);
    tokio::pin!(linger);
    loop {
        tokio::select! {
            r = master.read(&mut buf) => match r {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if channel.data(&buf[..n]).await.is_err() {
                        client_gone = true;
                        break;
                    }
                }
            },
            msg = channel.wait() => match msg {
                Some(ChannelMsg::Data { data }) => {
                    if master.write_all(&data).await.is_err() {
                        break;
                    }
                }
                // End of input on a terminal is Ctrl-D.
                Some(ChannelMsg::Eof) => { let _ = master.write_all(&[4]).await; }
                Some(ChannelMsg::Close) | None => {
                    client_gone = true;
                    break;
                }
                Some(_) => {}
            },
            Some(c) = control.recv() => match c {
                Control::Resize(cols, rows) => { let _ = pty::set_size(master.raw(), cols, rows); }
                Control::Signal(sig) => signal_group(&child, sig),
            },
            status = child.wait(), if exit.is_none() => {
                exit = status.ok();
                linger.as_mut().reset(tokio::time::Instant::now() + LINGER);
            }
            _ = &mut linger, if exit.is_some() => break,
        }
    }
    if exit.is_none() {
        exit = if client_gone { hang_up(&mut child).await } else { child.wait().await.ok() };
    }
    drop(master);
    if !client_gone {
        finish(channel, exit).await;
    }
}

async fn run_piped(mut channel: Channel<Msg>, mut child: Child, mut control: mpsc::UnboundedReceiver<Control>) {
    let mut stdin = child.stdin.take();
    let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        finish(channel, None).await;
        return;
    };
    let mut out_buf = vec![0u8; 16384];
    let mut err_buf = vec![0u8; 4096];
    let (mut out_done, mut err_done) = (false, false);
    let mut exit = None;
    let mut client_gone = false;
    let linger = tokio::time::sleep(Duration::MAX);
    tokio::pin!(linger);
    while !(out_done && err_done && exit.is_some()) {
        tokio::select! {
            r = stdout.read(&mut out_buf), if !out_done => match r {
                Ok(0) | Err(_) => out_done = true,
                Ok(n) => {
                    if channel.data(&out_buf[..n]).await.is_err() {
                        client_gone = true;
                        break;
                    }
                }
            },
            r = stderr.read(&mut err_buf), if !err_done => match r {
                Ok(0) | Err(_) => err_done = true,
                Ok(n) => {
                    if channel.extended_data(1, &err_buf[..n]).await.is_err() {
                        client_gone = true;
                        break;
                    }
                }
            },
            msg = channel.wait() => match msg {
                Some(ChannelMsg::Data { data }) => {
                    if let Some(pipe) = stdin.as_mut()
                        && pipe.write_all(&data).await.is_err() {
                            stdin = None;
                        }
                }
                Some(ChannelMsg::Eof) => stdin = None,
                Some(ChannelMsg::Close) | None => {
                    client_gone = true;
                    break;
                }
                Some(_) => {}
            },
            Some(c) = control.recv() => {
                if let Control::Signal(sig) = c {
                    signal_group(&child, sig);
                }
            }
            status = child.wait(), if exit.is_none() => {
                exit = status.ok();
                linger.as_mut().reset(tokio::time::Instant::now() + LINGER);
            }
            _ = &mut linger, if exit.is_some() => break,
        }
    }
    if exit.is_none() {
        exit = if client_gone { hang_up(&mut child).await } else { child.wait().await.ok() };
    }
    if !client_gone {
        finish(channel, exit).await;
    }
}

impl russh::server::Handler for Handler {
    type Error = russh::Error;

    async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
        Ok(Self::reject())
    }

    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<Auth, Self::Error> {
        Ok(Self::reject())
    }

    async fn auth_publickey_offered(&mut self, user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        Ok(if self.check_key(user, key) { Auth::Accept } else { Self::reject() })
    }

    async fn auth_publickey(&mut self, user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        if self.check_key(user, key) {
            info!("ssh: {} logged in as {user}", self.peer);
            self.authed.store(true, Ordering::Relaxed);
            Ok(Auth::Accept)
        } else {
            warn!("ssh: {} rejected (user {user})", self.peer);
            Ok(Self::reject())
        }
    }

    async fn authentication_banner(&mut self) -> Result<Option<String>, Self::Error> {
        Ok(Some(format!("{} {} ({})\r\n", crate::PRODUCT, crate::VERSION, self.ctx.model)))
    }

    async fn channel_open_session(&mut self, channel: Channel<Msg>, _session: &mut Session) -> Result<bool, Self::Error> {
        if self.channels.len() >= super::MAX_CHANNELS {
            return Ok(false);
        }
        self.channels.insert(channel.id(), ChannelState { channel: Some(channel), ..Default::default() });
        Ok(true)
    }

    async fn channel_close(&mut self, channel: ChannelId, _session: &mut Session) -> Result<(), Self::Error> {
        self.channels.remove(&channel);
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        match self.channels.get_mut(&channel) {
            Some(state) => {
                state.pty = Some((term.to_string(), col_width.min(1000) as u16, row_height.min(1000) as u16));
                session.channel_success(channel)
            }
            None => session.channel_failure(channel),
        }
    }

    async fn env_request(&mut self, channel: ChannelId, name: &str, value: &str, session: &mut Session) -> Result<(), Self::Error> {
        match self.channels.get_mut(&channel) {
            Some(state) if env_allowed(name) && state.env.len() < 32 && value.len() < 1024 => {
                state.env.push((name.to_string(), value.to_string()));
                session.channel_success(channel)
            }
            _ => session.channel_failure(channel),
        }
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        match self.start_process(channel, None) {
            Ok(()) => session.channel_success(channel),
            Err(e) => {
                warn!("ssh: {}: shell: {e}", self.peer);
                session.channel_failure(channel)
            }
        }
    }

    async fn exec_request(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).into_owned();
        match self.start_process(channel, Some(command)) {
            Ok(()) => session.channel_success(channel),
            Err(e) => {
                warn!("ssh: {}: exec: {e}", self.peer);
                session.channel_failure(channel)
            }
        }
    }

    async fn subsystem_request(&mut self, channel: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
        let taken = match (name, self.channels.get_mut(&channel)) {
            ("sftp", Some(state)) => state.channel.take(),
            _ => None,
        };
        match taken {
            Some(ch) => {
                debug!("ssh: {}: sftp", self.peer);
                session.channel_success(channel)?;
                let home = self.ctx.paths.home();
                let start = if home.is_dir() { home } else { "/".into() };
                let stream = super::sftp::CloseOnDrop::new(ch.into_stream(), session.handle(), channel);
                russh_sftp::server::run(stream, super::sftp::Sftp::new(start)).await;
                Ok(())
            }
            None => session.channel_failure(channel),
        }
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(tx) = self.channels.get(&channel).and_then(|s| s.control.as_ref()) {
            let _ = tx.send(Control::Resize(col_width.min(1000) as u16, row_height.min(1000) as u16));
        }
        Ok(())
    }

    async fn signal(&mut self, channel: ChannelId, signal: Sig, _session: &mut Session) -> Result<(), Self::Error> {
        if let (Some(tx), Some(sig)) =
            (self.channels.get(&channel).and_then(|s| s.control.as_ref()), signal_number(&signal))
        {
            let _ = tx.send(Control::Signal(sig));
        }
        Ok(())
    }
}
