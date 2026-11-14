//! Provisioned certificates in nvdata, kept the way stock keeps them
//! (`dphone_config::trust`): the trust bundle `/nvdata/certs/certs.crt` and
//! the client certificates and keys in `/nvdata/clientcerts/`.
//!
//! The trust bundle is the image's base bundle (the Mozilla CA bundle, at
//! stock's path `/usr/share/ssl/certs/cacert.pem`) followed by the
//! provisioned certificates; like stock, the phone rebuilds it from those
//! two alone, so a bundle stock left behind is replaced at the first start.
//!
//! The files follow the merged configuration.  They are written on the
//! blocking pool; when something changed, the configuration hub is told
//! once more so that the users of the files read them again.

use std::sync::{Arc, OnceLock};

use dphone_config::trust::BASE_BUNDLE_PATH;
use dphone_config::{Config, Nvdata};
use log::{info, warn};
use tokio::sync::Notify;

use super::SharedConfig;
use super::system::ConfigHub;
use crate::mainrole::Context;

/// The base bundle of the image's root file system (public certificate
/// authorities), read once: the root file system does not change under a
/// running phone.  `None` where the file is missing, i.e., outside an image.
fn base_bundle() -> Option<&'static [u8]> {
    static BASE: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    BASE.get_or_init(|| {
        let base = std::fs::read(BASE_BUNDLE_PATH).ok().filter(|b| !b.is_empty());
        if base.is_none() {
            warn!("certificates: no base bundle at {BASE_BUNDLE_PATH}; only provisioned certificates are trusted");
        }
        base
    })
    .as_deref()
}

/// The PEM roots TLS clients on the phone trust: the base bundle followed
/// by the provisioned certificates, which is also what `certs.crt` holds
/// (see [`dphone_config::trust::roots`]).  Every TLS client that checks its
/// server takes its roots from here.
pub fn roots(nv: &Nvdata, cfg: &Config) -> Vec<u8> {
    let base = base_bundle();
    // The file matters only where there is no base bundle.
    let on_disk = if base.is_none() { nv.trust_bundle().ok().flatten() } else { None };
    dphone_config::trust::roots(cfg, base, on_disk.as_deref())
}

pub fn start(ctx: &Context, config: Option<SharedConfig>, hub: &ConfigHub) {
    let Some(config) = config else { return };
    // Nothing persists in recovery, and an image that leaves stock's files
    // alone does not write these either.
    if ctx.recovery.is_some() || !super::stock_writes_allowed(ctx) {
        return;
    }
    let wake = Arc::new(Notify::new());
    {
        let wake = wake.clone();
        hub.on_change(move |_| wake.notify_one());
    }
    let (store, hub) = (ctx.store.clone(), hub.clone());
    tokio::spawn(async move {
        loop {
            let (cfg, nv) = {
                let c = config.lock().unwrap();
                (c.config(), c.nvdata().clone())
            };
            let job = {
                let cfg = cfg.clone();
                tokio::task::spawn_blocking(move || {
                    // Without a base bundle the trust file is not ours to
                    // rebuild (see `roots`); client certificates still are.
                    match base_bundle() {
                        Some(base) => nv.update_trust(&cfg, Some(base)),
                        None => nv.update_client_certs(&cfg),
                    }
                })
            };
            match job.await {
                Ok(Ok(change)) => {
                    for skipped in &change.skipped {
                        warn!("certificates: skipped {skipped}");
                    }
                    if !change.is_empty() {
                        info!("certificates: wrote {}", change.written.join(", "));
                        // The listeners include this task; its next pass
                        // finds nothing to write and stops there.
                        hub.changed(&store, &cfg);
                    }
                }
                Ok(Err(e)) => warn!("certificates: {e}"),
                Err(e) => warn!("certificates: {e}"),
            }
            wake.notified().await;
        }
    });
}
