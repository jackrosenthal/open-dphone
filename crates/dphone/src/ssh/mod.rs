//! The embedded SSH server (section 11): public-key authentication only,
//! shell and exec sessions, and SFTP for pushing builds.
//!
//! Built on `russh` with RustCrypto primitives only.  The server runs on
//! the main role's single-threaded runtime.
//!
//! ## When it runs
//!
//! - `dphone main --ssh-port N` or `--ssh-listen ADDR`: always.
//! - On a phone: in development mode (a key baked in with
//!   `DPHONE_DEV_SSH_KEY` at build time, `dphone.dev` on the kernel command
//!   line, `/etc/dphone/dev`, or `/nvdata/open/dev`), or when
//!   `/nvdata/open/ssh-enabled` exists.  The latter stands in for the
//!   configuration setting until the config crate is wired.
//! - Otherwise off, as release images should be.

pub mod keys;
pub mod pty;
pub mod session;
pub mod sftp;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use log::{debug, info, warn};
use russh::server::Config;
use russh::{MethodKind, MethodSet, Preferred, SshId};
use tokio::net::TcpListener;

use crate::mainrole::{Context, Options};

/// Simultaneous connections, authenticated or not.
pub const MAX_CONNECTIONS: usize = 6;
/// Time a connection gets to authenticate.
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(60);
/// Sessions (shell, exec, SFTP) per connection.
pub const MAX_CHANNELS: usize = 8;

/// A public key baked in at build time for development images: set
/// `DPHONE_DEV_SSH_KEY` to one `authorized_keys` line when compiling.
pub fn dev_key() -> Option<&'static str> {
    option_env!("DPHONE_DEV_SSH_KEY").map(str::trim).filter(|k| !k.is_empty())
}

/// Decide whether SSH runs and where.
pub fn listen_address(ctx: &Context, opts: &Options) -> Option<SocketAddr> {
    if opts.no_ssh {
        return None;
    }
    if let Some(addr) = opts.ssh_listen {
        return Some(addr);
    }
    if ctx.sim {
        return None;
    }
    if ctx.dev || ctx.paths.open_dir().join("ssh-enabled").exists() {
        return Some(SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, 22)));
    }
    None
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

/// Start the server.  Returns the bound address.
pub async fn start(ctx: &Context, listen: SocketAddr) -> std::io::Result<SocketAddr> {
    let host_key = keys::load_or_create_host_key(&ctx.paths.ssh_host_key())?;
    let config = Arc::new(server_config(host_key));
    let listener = match TcpListener::bind(listen).await {
        Ok(l) => l,
        // No IPv6 in the kernel: fall back to IPv4.
        Err(_) if listen.ip().is_unspecified() && listen.is_ipv6() => {
            TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], listen.port()))).await?
        }
        Err(e) => return Err(e),
    };
    let addr = listener.local_addr()?;
    if keys::authorized_keys(&ctx.paths).is_empty() {
        warn!("ssh: no authorized keys ({}); nobody can log in", ctx.paths.authorized_keys().display());
    }
    tokio::spawn(accept_loop(ctx.clone(), listener, config));
    Ok(addr)
}

async fn accept_loop(ctx: Context, listener: TcpListener, config: Arc<Config>) {
    let active = Arc::new(AtomicUsize::new(0));
    loop {
        let (socket, peer) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                warn!("ssh accept: {e}");
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        if active.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
            warn!("ssh: too many connections; refusing {peer}");
            continue;
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
}
