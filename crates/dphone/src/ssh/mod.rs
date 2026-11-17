//! The embedded SSH server (section 11): public-key authentication only,
//! shell and exec sessions, and SFTP for pushing builds.
//!
//! Built on `russh` with RustCrypto primitives only.  The server runs on
//! the main role's single-threaded runtime.
//!
//! ## When it runs
//!
//! Off by default, on every image.  One input turns it on: the "SSH
//! server" switch, handed to [`set_enabled`] by whoever reads the settings
//! (this module does not know where the switch is stored).  The switch
//! applies at once, by opening or closing the listening socket; while it is
//! off there is no listening socket at all, and there is none before the
//! settings have been read.  Turning SSH off leaves established sessions
//! running, as stopping `sshd` does, so that whoever turned it off from an
//! SSH session is not cut off mid-command; they end when their clients
//! disconnect.
//!
//! Who may log in is decided by keys alone: the files named by
//! [`authorized_key_files`], read at each attempt.  No key is compiled in.
//! In the host simulator the server exists only when `--ssh-port N` or
//! `--ssh-listen ADDR` is given (and listens there when switched on), and
//! `--no-ssh` leaves it out for tests.

pub mod keys;
pub mod pty;
pub mod session;
pub mod sftp;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use log::{debug, info, warn};
use russh::server::Config;
use russh::{MethodKind, MethodSet, Preferred, SshId};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use crate::mainrole::{Context, Options};

/// Simultaneous connections, authenticated or not.
pub const MAX_CONNECTIONS: usize = 6;
/// Time a connection gets to authenticate.
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(60);
/// Sessions (shell, exec, SFTP) per connection.
pub const MAX_CHANNELS: usize = 8;

/// The `authorized_keys` files consulted at each login attempt: the keys
/// added on the phone (`/nvdata/open/authorized_keys`, never the stock
/// firmware's file), and the keys a provisioning server supplied.  Either
/// may be absent.
pub fn authorized_key_files(paths: &crate::paths::Paths) -> Vec<PathBuf> {
    vec![paths.authorized_keys(), paths.open_dir().join("provisioned_authorized_keys")]
}

/// Decide where SSH listens.  `None` only in the simulator without an
/// explicit address, or with `--no-ssh`.
pub fn listen_address(ctx: &Context, opts: &Options) -> Option<SocketAddr> {
    if opts.no_ssh {
        return None;
    }
    if let Some(addr) = opts.ssh_address() {
        return Some(addr);
    }
    if ctx.sim {
        return None;
    }
    Some(SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, 22)))
}

/// The run-time switch: whether the server listens.
fn switch() -> &'static watch::Sender<bool> {
    static SWITCH: OnceLock<watch::Sender<bool>> = OnceLock::new();
    SWITCH.get_or_init(|| watch::channel(false).0)
}

/// Turn the listener on or off (the user's "SSH server" switch).  Takes
/// effect at once; established sessions are left alone.  Harmless when the
/// server was never started, and callable from any thread.
pub fn set_enabled(enabled: bool) {
    if switch().send_replace(enabled) != enabled {
        info!("ssh: {}", if enabled { "enabled" } else { "disabled; established sessions stay" });
    }
}

/// Whether the listener is switched on.
pub fn enabled() -> bool {
    *switch().borrow()
}

fn server_config(host_key: russh::keys::PrivateKey) -> Config {
    let mut methods = MethodSet::empty();
    methods.push(MethodKind::PublicKey);
    Config {
        server_id: SshId::Standard(format!("SSH-2.0-OpenDPhone_{}", crate::VERSION)),
        methods,
        auth_rejection_time: Duration::from_secs(1),
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![host_key],
        max_auth_attempts: 6,
        inactivity_timeout: Some(Duration::from_secs(3600)),
        keepalive_interval: Some(Duration::from_secs(60)),
        keepalive_max: 3,
        nodelay: true,
        // No compression: nothing to gain on a LAN, and inflating attacker
        // input before authentication is a memory risk on a 128 MiB phone.
        preferred: Preferred { compression: std::borrow::Cow::Borrowed(&[russh::compression::NONE]), ..Preferred::default() },
        ..Config::default()
    }
}

async fn bind(listen: SocketAddr) -> std::io::Result<TcpListener> {
    match TcpListener::bind(listen).await {
        Ok(l) => Ok(l),
        // No IPv6 in the kernel: fall back to IPv4.
        Err(_) if listen.ip().is_unspecified() && listen.is_ipv6() => {
            TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], listen.port()))).await
        }
        Err(e) => Err(e),
    }
}

/// Start the server.  It listens on `listen` once (and while) it is
/// switched on; nothing is bound before that.
pub async fn start(ctx: &Context, listen: SocketAddr) -> std::io::Result<()> {
    let host_key = keys::load_or_create_host_key(&ctx.paths.ssh_host_key())?;
    let config = Arc::new(server_config(host_key));
    if keys::authorized_keys(&authorized_key_files(&ctx.paths)).is_empty() {
        info!("ssh: no authorized keys yet; nobody can log in until one is added");
    }
    let ctx = ctx.clone();
    let active = Arc::new(AtomicUsize::new(0));
    let serve: Serve = Arc::new(move |socket, peer| connection(&ctx, &config, &active, socket, peer));
    tokio::spawn(supervise(None, listen, serve));
    Ok(())
}

/// What to do with an accepted connection (spawn its task, or refuse it).
type Serve = Arc<dyn Fn(TcpStream, SocketAddr) + Send + Sync>;

/// Listen while SSH is switched on: accept until it is switched off, close
/// the listening socket, and bind it again when it is switched back on.
async fn supervise(listener: Option<TcpListener>, mut addr: SocketAddr, serve: Serve) {
    let mut switch = switch().subscribe();
    let mut listener = listener;
    loop {
        if !*switch.borrow_and_update() {
            // Switched off: no listening socket at all, until switched on.
            listener = None;
            if switch.changed().await.is_err() {
                return;
            }
            continue;
        }
        let l = match listener.take() {
            Some(l) => l,
            None => match bind(addr).await {
                Ok(l) => {
                    // With port 0 (tests), keep the port the first bind got.
                    addr = l.local_addr().unwrap_or(addr);
                    info!("ssh listening on {addr}");
                    l
                }
                Err(e) => {
                    warn!("ssh: cannot listen on {addr}: {e}");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            },
        };
        loop {
            tokio::select! {
                accepted = l.accept() => match accepted {
                    Ok((socket, peer)) => serve(socket, peer),
                    Err(e) => {
                        warn!("ssh accept: {e}");
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                },
                _ = switch.changed() => {
                    if !*switch.borrow_and_update() {
                        break;
                    }
                }
            }
        }
    }
}

/// Serve one accepted connection on a task of its own, within the cap.
fn connection(ctx: &Context, config: &Arc<Config>, active: &Arc<AtomicUsize>, socket: TcpStream, peer: SocketAddr) {
    if active.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
        warn!("ssh: too many connections; refusing {peer}");
        return;
    }
    active.fetch_add(1, Ordering::Relaxed);
    let _ = socket.set_nodelay(true);
    let config = config.clone();
    let active = active.clone();
    let authed = Arc::new(AtomicBool::new(false));
    let handler = session::Handler::new(ctx.clone(), peer, authed.clone());
    tokio::spawn(async move {
        debug!("ssh: connection from {peer}");
        match russh::server::run_stream(config, socket, handler).await {
            Ok(mut running) => {
                let handle = running.handle();
                let result = tokio::select! {
                    r = &mut running => r,
                    _ = tokio::time::sleep(AUTH_TIMEOUT), if !authed.load(Ordering::Relaxed) => {
                        if authed.load(Ordering::Relaxed) {
                            (&mut running).await
                        } else {
                            info!("ssh: {peer} did not authenticate in time");
                            let _ = handle
                                .disconnect(russh::Disconnect::ByApplication, "authentication timeout".into(), "en".into())
                                .await;
                            Ok(())
                        }
                    }
                };
                if let Err(e) = result {
                    debug!("ssh: {peer}: {e}");
                }
            }
            Err(e) => debug!("ssh: {peer}: setup failed: {e}"),
        }
        active.fetch_sub(1, Ordering::Relaxed);
        debug!("ssh: {peer} closed");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn greet(addr: SocketAddr) -> std::io::Result<TcpStream> {
        let mut s = TcpStream::connect(addr).await?;
        let mut b = [0u8; 2];
        tokio::time::timeout(Duration::from_secs(5), s.read_exact(&mut b)).await??;
        assert_eq!(&b, b"hi");
        Ok(s)
    }

    /// The switch closes and reopens the listening socket and leaves an
    /// established connection alone.
    #[tokio::test]
    async fn switch_closes_and_reopens_the_listener() {
        let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let addr = listener.local_addr().unwrap();
        // Each connection: greet, then echo until the peer closes.
        let serve: Serve = Arc::new(|mut socket, _peer| {
            tokio::spawn(async move {
                let _ = socket.write_all(b"hi").await;
                let mut b = [0u8; 16];
                while let Ok(n) = socket.read(&mut b).await {
                    if n == 0 || socket.write_all(&b[..n]).await.is_err() {
                        break;
                    }
                }
            });
        });
        set_enabled(true);
        tokio::spawn(supervise(Some(listener), addr, serve));

        // On: served.
        assert!(enabled());
        let mut held = greet(addr).await.unwrap();

        // Off: the port refuses; the established connection still works.
        set_enabled(false);
        let mut refused = false;
        for _ in 0..100 {
            if TcpStream::connect(addr).await.is_err() {
                refused = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(refused, "the listener stayed open");
        held.write_all(b"x").await.unwrap();
        let mut b = [0u8; 1];
        held.read_exact(&mut b).await.unwrap();
        assert_eq!(b[0], b'x');

        // On again: served again, on the same address.
        set_enabled(true);
        let mut reopened = false;
        for _ in 0..100 {
            if greet(addr).await.is_ok() {
                reopened = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(reopened, "the listener did not reopen");
    }
}
