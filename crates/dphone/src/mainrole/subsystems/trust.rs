//! Provisioned certificates in nvdata, kept the way stock keeps them
//! (`dphone_config::trust`): the trust bundle `/nvdata/certs/certs.crt` and
//! the client certificates and keys in `/nvdata/clientcerts/`.
//!
//! The files follow the merged configuration.  They are written on the
//! blocking pool; when something changed, the configuration hub is told
//! once more so that the users of the files (SIP over TLS) read them again.

use std::sync::Arc;

use dphone_config::trust::{BASE_BUNDLE_PATH, trust_bundle};
use dphone_config::{Config, Nvdata};
use log::{info, warn};
use tokio::sync::Notify;

use super::SharedConfig;
use super::system::ConfigHub;
use crate::mainrole::Context;

/// The base bundle of this image's root file system, if it has one.
fn base_bundle() -> Option<Vec<u8>> {
    std::fs::read(BASE_BUNDLE_PATH).ok().filter(|b| !b.is_empty())
}

/// The PEM roots TLS clients on the phone trust.
///
/// With a base bundle in the image this is `certs.crt` as
/// [`Nvdata::update_trust`] writes it.  An image without one cannot
/// rebuild that file, so it leaves the file as it finds it (on a phone that
/// has run stock, stock's bundle) and adds the provisioned certificates to
/// what the file holds.
pub fn roots(nv: &Nvdata, cfg: &Config) -> Vec<u8> {
    let on_disk = nv.trust_bundle().ok().flatten();
    match (base_bundle(), on_disk) {
        (Some(_), Some(file)) => file,
        (Some(base), None) => trust_bundle(cfg, Some(&base)),
        (None, file) => {
            let mut out = file.unwrap_or_default();
            out.extend_from_slice(&trust_bundle(cfg, None));
            out
        }
    }
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
                    let base = base_bundle();
                    // Without a base bundle the trust file is not ours to
                    // rebuild (see `roots`); client certificates still are.
                    match base {
                        Some(base) => nv.update_trust(&cfg, Some(&base)),
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
