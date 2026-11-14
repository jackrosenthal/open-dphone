//! Diagnostics that drive the phone as a user would: keys, the hook, calls,
//! apps, and a screenshot of what is on the display.  For SSH sessions and
//! for testing without touching the phone.

use std::sync::Arc;
use std::time::Duration;

use dphone_hal::{PixelFormat, Screen};
use dphone_state::{Command, Key, Store};

use super::ui::ScreenTap;
use crate::diag::Output;
use crate::mainrole::Context;

async fn send(store: &Store, cmd: Command) -> Result<String, String> {
    match tokio::time::timeout(Duration::from_secs(15), store.send(cmd)).await {
        Ok(Ok(reply)) => Ok(format!("{reply:?}\n")),
        Ok(Err(e)) => Err(format!("{e:?}")),
        Err(_) => Err("no answer within 15 s".into()),
    }
}

/// A frame as a binary PGM (gray) or PPM (color) picture.
fn netpbm(frame: &dphone_hal::Frame) -> Vec<u8> {
    let (w, h) = (frame.info.width as usize, frame.info.height as usize);
    let mut out = Vec::with_capacity(w * h * 3 + 32);
    match frame.info.format {
        PixelFormat::R8 => {
            out.extend_from_slice(format!("P5\n{w} {h}\n255\n").as_bytes());
            for y in 0..h {
                out.extend_from_slice(&frame.pixels[y * frame.info.stride..y * frame.info.stride + w]);
            }
        }
        PixelFormat::Xrgb8888 => {
            out.extend_from_slice(format!("P6\n{w} {h}\n255\n").as_bytes());
            for y in 0..h {
                for px in frame.pixels[y * frame.info.stride..y * frame.info.stride + w * 4].as_chunks::<4>().0 {
                    out.extend_from_slice(&[px[2], px[1], px[0]]);
                }
            }
        }
    }
    out
}

pub fn register(ctx: &Context, tap: Option<Arc<ScreenTap>>) {
    let store = ctx.store.clone();
    ctx.diag.register(
        "key",
        "press keys as if on the keypad: key NAME... (e.g., key 1 2 3 soft1; names as in 'dphone key list')",
        move |args: Vec<String>, out: Output| {
            let store = store.clone();
            async move {
                if args.first().map(String::as_str) == Some("list") {
                    out.out("0-9 * # up down left right select cancel soft1..soft4 line1..line6 vup vdn mute spkr hdst hold xfer\nredial msgs conf info blfleft blfright status dir apps menu\n").await;
                    return 0;
                }
                if args.is_empty() {
                    out.err("usage: dphone key NAME...\n").await;
                    return 2;
                }
                for name in &args {
                    let Some(key) = Key::from_name(name) else {
                        out.err(&format!("dphone key: unknown key '{name}'\n")).await;
                        return 2;
                    };
                    for down in [true, false] {
                        if let Err(e) = send(&store, Command::InjectKey { key, down }).await {
                            out.err(&format!("dphone key: {e}\n")).await;
                            return 1;
                        }
                        tokio::time::sleep(Duration::from_millis(60)).await;
                    }
                }
                0
            }
        },
    );

    let store = ctx.store.clone();
    ctx.diag.register("hook", "lift or replace the handset: hook off|on", move |args: Vec<String>, out: Output| {
        let store = store.clone();
        async move {
            let off_hook = match args.first().map(String::as_str) {
                Some("off") => true,
                Some("on") => false,
                _ => {
                    out.err("usage: dphone hook off|on\n").await;
                    return 2;
                }
            };
            match send(&store, Command::InjectHook { off_hook }).await {
                Ok(_) => 0,
                Err(e) => {
                    out.err(&format!("dphone hook: {e}\n")).await;
                    1
                }
            }
        }
    });

    let store = ctx.store.clone();
    ctx.diag.register(
        "call",
        "call dial NUMBER | answer | hangup | list",
        move |args: Vec<String>, out: Output| {
            let store = store.clone();
            async move {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                let calls = store.get::<dphone_state::Calls>();
                let result = match args.as_slice() {
                    ["dial", number] => send(&store, Command::Dial { target: number.to_string(), account: None, path: None }).await,
                    ["answer"] => match calls.calls.first() {
                        Some(c) => send(&store, Command::Answer { call: c.id, path: None }).await,
                        None => Err("no call".into()),
                    },
                    ["hangup"] => {
                        let mut last = Err("no call".to_string());
                        for c in &calls.calls {
                            last = send(&store, Command::Hangup { call: c.id }).await;
                        }
                        last
                    }
                    [] | ["list"] => Ok(format!("{calls:#?}\n")),
                    _ => Err("usage: dphone call dial NUMBER | answer | hangup | list".into()),
                };
                match result {
                    Ok(text) => {
                        out.out(&text).await;
                        0
                    }
                    Err(e) => {
                        out.err(&format!("dphone call: {e}\n")).await;
                        1
                    }
                }
            }
        },
    );

    let store = ctx.store.clone();
    ctx.diag.register("app", "app launch NAME | close NAME | list", move |args: Vec<String>, out: Output| {
        let store = store.clone();
        async move {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let result = match args.as_slice() {
                ["launch", name] => send(&store, Command::LaunchApp { name: name.to_string() }).await,
                ["close", name] => send(&store, Command::CloseApp { name: name.to_string() }).await,
                [] | ["list"] => Ok(store
                    .get::<dphone_state::Apps>()
                    .installed
                    .iter()
                    .map(|a| format!("{:<12} {}{}\n", a.name, a.display_name, if a.running { " (running)" } else { "" }))
                    .collect()),
                _ => Err("usage: dphone app launch NAME | close NAME | list".into()),
            };
            match result {
                Ok(text) => {
                    out.out(&text).await;
                    0
                }
                Err(e) => {
                    out.err(&format!("dphone app: {e}\n")).await;
                    1
                }
            }
        }
    });

    ctx.diag.register(
        "screenshot",
        "write what is on the display to a file: screenshot FILE [side]  (PPM or PGM)",
        move |args: Vec<String>, out: Output| {
            let tap = tap.clone();
            async move {
                let Some(file) = args.first() else {
                    out.err("usage: dphone screenshot FILE [side]\n").await;
                    return 2;
                };
                let screen = if args.get(1).map(String::as_str) == Some("side") { Screen::Side } else { Screen::Main };
                let Some(frame) = tap.as_ref().and_then(|t| t.capture(screen)) else {
                    out.err("dphone screenshot: nothing has been drawn on that screen\n").await;
                    return 1;
                };
                match std::fs::write(file, netpbm(&frame)) {
                    Ok(()) => {
                        out.out(&format!("{}x{} written to {file}\n", frame.info.width, frame.info.height)).await;
                        0
                    }
                    Err(e) => {
                        out.err(&format!("dphone screenshot: {file}: {e}\n")).await;
                        1
                    }
                }
            }
        },
    );
}
