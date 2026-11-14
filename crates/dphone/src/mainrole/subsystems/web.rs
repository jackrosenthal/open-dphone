//! dphone-web: the web UI and its Connect RPC services.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use dphone_hal::{PixelFormat, Screen};
use dphone_state::{HealthStatus, Subsystem};
use dphone_web::{
    FileFirmwareSink, FirmwareInfo, MirrorFormat, MirrorFrame, MirrorScreen, ScreenSource, WebOptions, WebServer,
};
use log::{error, info, warn};

use super::SharedConfig;
use super::ui::ScreenTap;
use crate::mainrole::{Context, Options};

struct Mirror(Arc<ScreenTap>);

impl ScreenSource for Mirror {
    fn generation(&self) -> u64 {
        self.0.generation()
    }

    fn capture(&self, screen: MirrorScreen) -> Option<MirrorFrame> {
        let frame = self.0.capture(match screen {
            MirrorScreen::Main => Screen::Main,
            MirrorScreen::Side => Screen::Side,
        })?;
        Some(MirrorFrame {
            width: frame.info.width,
            height: frame.info.height,
            stride: frame.info.stride,
            format: match frame.info.format {
                PixelFormat::Xrgb8888 => MirrorFormat::Xrgb8888,
                PixelFormat::R8 => MirrorFormat::Gray8,
            },
            pixels: frame.pixels,
        })
    }
}

/// Read an uploaded image's label: version, model, and whether it is stock.
fn inspect(path: &std::path::Path) -> std::io::Result<FirmwareInfo> {
    let file = std::io::BufReader::new(std::fs::File::open(path)?);
    let reader = eff::EffReader::new(file).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    let header = reader.header();
    Ok(FirmwareInfo {
        version: header.version.trim().to_string(),
        model: header.model.trim().to_string(),
        is_stock: dphone_update::is_stock_version(&header.version),
    })
}

pub fn start(ctx: &Context, opts: &Options, config: Option<SharedConfig>, tap: Option<Arc<ScreenTap>>) {
    let Some(config) = config else {
        error!("web: no configuration; the web UI is not started");
        ctx.store.set_health(Subsystem::Web, HealthStatus::Failed("no configuration".into()));
        return;
    };
    let access = config.lock().unwrap().config().access();
    if !access.web_ui_enabled && !ctx.dev {
        info!("web: disabled by configuration (web_ui_enabled)");
        ctx.store.set_health(Subsystem::Web, HealthStatus::Stopped);
        return;
    }
    let mut options = WebOptions::default();
    if ctx.sim {
        // An unprivileged port on this machine only; no HTTPS listener.
        let port = opts.web_port.unwrap_or(8080);
        options.http = Some(SocketAddr::from((Ipv4Addr::LOCALHOST, port)));
        options.https = None;
        options.upload_dir = ctx.paths.run.join("upload");
    } else {
        if let Some(port) = opts.web_port {
            options.http = Some(SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)));
        }
        if !access.https_enabled {
            options.https = None;
        }
    }
    ctx.store.set_health(Subsystem::Web, HealthStatus::Starting);
    let ctx = ctx.clone();
    tokio::spawn(async move {
        loop {
            let mut server = WebServer::new(ctx.store.clone(), config.clone(), options.clone()).with_firmware_sink(Box::new(
                FileFirmwareSink::new(options.upload_dir.clone()).with_inspector(inspect),
            ));
            if let Some(tap) = &tap {
                server = server.with_screen(Arc::new(Mirror(tap.clone())));
            }
            match server.bind().await {
                Ok(bound) => {
                    info!("web: listening (http {:?}, https {:?})", options.http, options.https);
                    ctx.store.set_health(Subsystem::Web, HealthStatus::Ok);
                    if let Err(e) = bound.serve().await {
                        error!("web: server stopped: {e}");
                    } else {
                        warn!("web: server ended");
                    }
                }
                Err(e) => error!("web: cannot bind: {e}"),
            }
            ctx.store.set_health(Subsystem::Web, HealthStatus::Failed("not listening".into()));
            // An IPv6 wildcard that cannot be bound (no IPv6 in the
            // kernel): fall back to IPv4 for the next attempt.
            for addr in [&mut options.http, &mut options.https].into_iter().flatten() {
                if addr.is_ipv6() && addr.ip().is_unspecified() {
                    *addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, addr.port()));
                }
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });
}
