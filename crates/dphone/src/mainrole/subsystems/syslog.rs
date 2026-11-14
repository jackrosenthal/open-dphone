//! Remote syslog (plan section 5): when the stock settings `enable_logging`
//! and `log_server` say so, every log record is also sent to that server
//! as a syslog datagram (`log_port`, default 514), filtered by `log_level`.
//!
//! This file keeps the destination current: it follows the configuration,
//! and resolves a server given by name once the network can.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use dphone_config::view::LogSettings;
use dphone_config::Config;
use dphone_state::Topic;
use log::{info, warn};
use tokio::sync::watch;

use super::net::NetHandle;
use crate::mainrole::Context;

async fn resolve(net: &NetHandle, host: &str) -> Option<IpAddr> {
    if let Ok(ip) = host.trim().parse::<IpAddr>() {
        return Some(ip);
    }
    let resolver = net.resolver()?;
    resolver.lookup_ipv4(host).await.ok()?.first().map(|ip| IpAddr::V4(*ip))
}

async fn run(ctx: Context, net: NetHandle, mut settings: watch::Receiver<LogSettings>) {
    let network = ctx.store.subscribe(Topic::Network);
    let mut current: Option<SocketAddr> = None;
    loop {
        let want = settings.borrow_and_update().clone();
        let level = crate::logging::level_from_setting(&want.level);
        let target = match (&want.server, want.remote_enabled) {
            (Some(host), true) => match resolve(&net, host).await {
                Some(ip) => Some(SocketAddr::new(ip, want.port)),
                // Not resolvable yet; try again when the network changes.
                None => None,
            },
            _ => None,
        };
        let unresolved = want.remote_enabled && target.is_none();
        if target != current || target.is_some() {
            match crate::logging::set_remote(target, level) {
                Ok(()) => {
                    if target != current {
                        match target {
                            Some(t) => info!("syslog: sending to {t} at level {level}"),
                            None if !unresolved => info!("syslog: remote logging off"),
                            None => {}
                        }
                    }
                    current = target;
                }
                Err(e) => warn!("syslog: {e}"),
            }
        }
        tokio::select! {
            changed = settings.changed() => {
                if changed.is_err() {
                    return;
                }
            }
            _ = network.changed(), if unresolved => {
                // Let the resolver settle after an address change.
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

/// Handle for configuration changes.
#[derive(Clone)]
pub struct Syslog {
    settings: Arc<watch::Sender<LogSettings>>,
}

impl Syslog {
    pub fn apply_config(&self, config: &Config) {
        let new = config.logging();
        self.settings.send_if_modified(|s| {
            let changed = *s != new;
            *s = new.clone();
            changed
        });
    }
}

pub fn start(ctx: &Context, net: NetHandle, config: Option<&Config>) -> Syslog {
    let initial = config.map(Config::logging).unwrap_or(LogSettings {
        level: "information".into(),
        remote_enabled: false,
        server: None,
        port: 514,
    });
    let (tx, rx) = watch::channel(initial);
    tokio::spawn(run(ctx.clone(), net, rx));
    Syslog { settings: Arc::new(tx) }
}
