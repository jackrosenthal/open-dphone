//! Diagnostic commands the core provides, and placeholders for the ones
//! other subsystems take over.

use std::time::Duration;

use clap::Parser;

use super::Output;
use crate::mainrole::Context;
use crate::mainrole::subsystems::{ConfigCommand, StateCommand, hal::HalCommand};

/// Check that the main role answers
#[derive(Debug, Parser)]
#[command(name = "ping")]
pub struct PingCommand {}

/// Show the running version and model
#[derive(Debug, Parser)]
#[command(name = "version")]
pub struct VersionCommand {}

/// Show the conditions behind the healthy report
///
/// The exit status is 0 when the main role reports itself healthy to the
/// init role, and 1 when it does not.
#[derive(Debug, Parser)]
#[command(name = "health")]
pub struct HealthCommand {}

/// Show the log ring
#[derive(Debug, Parser)]
#[command(name = "log")]
pub struct LogCommand {
    /// Show the last COUNT entries
    #[arg(short = 'n', long = "lines", value_name = "COUNT", default_value_t = 200)]
    count: usize,
    /// Keep showing entries as they are logged
    #[arg(short, long)]
    follow: bool,
}

/// Restart the main role only
#[derive(Debug, Parser)]
#[command(name = "restart")]
pub struct RestartCommand {}

/// Register the built-in commands.
pub fn register(ctx: &Context) {
    let reg = &ctx.diag;

    reg.register(|_: PingCommand, out: Output| async move {
        out.out("pong\n").await;
        0
    });

    reg.register({
        let ctx = ctx.clone();
        move |_: VersionCommand, out: Output| {
            let ctx = ctx.clone();
            async move {
                out.out(&format!(
                    "{} {} ({}) model {}{}{}\nuptime {} s\n",
                    crate::PRODUCT,
                    crate::VERSION,
                    crate::stock_version(),
                    ctx.model,
                    if ctx.sim { " simulator" } else { "" },
                    if ctx.recovery.is_some() { " RECOVERY" } else { "" },
                    ctx.started.elapsed().as_secs(),
                ))
                .await;
                0
            }
        }
    });

    reg.register({
        let health = ctx.health.clone();
        move |_: HealthCommand, out: Output| {
            let health = health.clone();
            async move {
                let mut text = String::new();
                for (name, ok) in health.status() {
                    text.push_str(&format!("{name:<12} {}\n", if ok { "ok" } else { "waiting" }));
                }
                text.push_str(if health.is_healthy() { "healthy\n" } else { "not healthy\n" });
                out.out(&text).await;
                if health.is_healthy() { 0 } else { 1 }
            }
        }
    });

    reg.register(log_command);

    reg.register({
        let ctx = ctx.clone();
        move |_: RestartCommand, out: Output| {
            let ctx = ctx.clone();
            async move {
                out.out("restarting the main role\n").await;
                ctx.request_exit(0);
                0
            }
        }
    });

    // Placeholders: each is replaced when its subsystem registers the command.
    reg.register_unavailable::<StateCommand>("not available: the state store is not up yet");
    reg.register_unavailable::<ConfigCommand>("not available: the configuration is not loaded yet");
    reg.register_unavailable::<HalCommand>("not available: the hardware layer is not open yet");
}

async fn log_command(args: LogCommand, out: Output) -> i32 {
    let follow = args.follow;
    let mut next = 0u64;
    let mut limit = args.count;
    let mut watch = crate::logging::subscribe();
    loop {
        let entries = crate::logging::recent(next, limit);
        let mut text = String::new();
        for e in &entries {
            text.push_str(&e.format());
            text.push('\n');
            next = e.seq + 1;
        }
        if !text.is_empty() {
            out.out(&text).await;
        }
        if !follow || out.is_closed() {
            return 0;
        }
        limit = usize::MAX;
        match watch.as_mut() {
            Some(w) => {
                tokio::select! {
                    r = w.changed() => if r.is_err() { return 0 },
                    _ = out.closed() => return 0,
                }
            }
            None => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
}
