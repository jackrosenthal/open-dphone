//! Diagnostic commands the core provides, and placeholders for the ones
//! other subsystems take over.

use std::time::Duration;

use super::{EXIT_UNAVAILABLE, Output};
use crate::mainrole::Context;
use crate::sys::Shutdown;

/// Register the built-in commands.
pub fn register(ctx: &Context) {
    let reg = &ctx.diag;

    reg.register("help", "list diagnostic commands", {
        let reg = reg.clone();
        move |_args, out| {
            let reg = reg.clone();
            async move {
                let mut text = String::from("Commands answered by the running main role:\n");
                for (name, help) in reg.list() {
                    text.push_str(&format!("  {name:<10} {help}\n"));
                }
                text.push_str(crate::cli::LOCAL_HELP);
                out.out(&text).await;
                0
            }
        }
    });

    reg.register("ping", "check that the main role answers", |_args, out: Output| async move {
        out.out("pong\n").await;
        0
    });

    reg.register("version", "show the running version and model", {
        let ctx = ctx.clone();
        move |_args, out| {
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

    reg.register("health", "show the conditions behind the healthy report", {
        let health = ctx.health.clone();
        move |_args, out| {
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

    reg.register("log", "show the log ring: log [-n COUNT] [-f]", log_command);

    reg.register("reboot", "reboot the phone: reboot [--halt|--poweroff] [REASON...]", {
        let ctx = ctx.clone();
        move |args: Vec<String>, out| {
            let ctx = ctx.clone();
            async move {
                let mut kind = Shutdown::Reboot;
                let mut words = Vec::new();
                for a in args {
                    match a.as_str() {
                        "--halt" => kind = Shutdown::Halt,
                        "--poweroff" => kind = Shutdown::PowerOff,
                        _ => words.push(a),
                    }
                }
                let reason = if words.is_empty() { "dphone: requested".to_string() } else { format!("dphone: {}", words.join(" ")) };
                out.out("going down\n").await;
                ctx.request_shutdown(kind, &reason);
                0
            }
        }
    });

    reg.register("restart", "restart the main role only", {
        let ctx = ctx.clone();
        move |_args, out| {
            let ctx = ctx.clone();
            async move {
                out.out("restarting the main role\n").await;
                ctx.request_exit(0);
                0
            }
        }
    });

    // Placeholders: each is replaced when its subsystem registers the name.
    for (name, help, owner) in [
        ("state", "dump the state store", "dphone-state"),
        ("config", "config get <id> | config set <id> <value>", "dphone-config"),
        ("hal", "poke hardware: LEDs, tones, display patterns, DSP jobs", "dphone-hal"),
        ("net", "show network state", "dphone-net"),
    ] {
        reg.register(name, help, move |_args, out: Output| async move {
            out.err(&format!("dphone {name}: not available: {owner} is not wired into this build\n")).await;
            EXIT_UNAVAILABLE
        });
    }
}

async fn log_command(args: Vec<String>, out: Output) -> i32 {
    let mut follow = false;
    let mut count = 200usize;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-f" | "--follow" => follow = true,
            "-n" => match it.next().and_then(|v| v.parse().ok()) {
                Some(n) => count = n,
                None => {
                    out.err("dphone log: -n needs a number\n").await;
                    return 2;
                }
            },
            other => {
                out.err(&format!("dphone log: unknown option '{other}'\n")).await;
                return 2;
            }
        }
    }
    let mut next = 0u64;
    let mut limit = count;
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
