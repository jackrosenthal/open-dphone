//! `ShellService`: the web terminal.

use crate::http::Ctx;
use crate::rpc::{Code, ConnectError, Sink};
use crate::shell::{OpenError, Session, ShellExit, ShellSize, CLIENT_SILENCE, MAX_INPUT, MAX_SHELLS, SHELL_IDLE};
use crate::{pb, App};
use std::sync::Arc;
use std::time::Duration;

/// Output is gathered for this long after the first byte, so a program
/// printing in small writes costs one message, not one per write.
const COALESCE: Duration = Duration::from_millis(8);
/// Largest output message.
const MAX_CHUNK: usize = 32 << 10;
const READ_BUF: usize = 8 << 10;
/// How often the stream checks on the shell, the web session, and idleness.
const TICK: Duration = Duration::from_millis(250);
/// The web session is kept alive this often while a terminal is open, as
/// the other streams do.
const TOUCH: Duration = Duration::from_secs(20);

type Event = pb::__buffa::oneof::open_shell_response::Event;

fn message(event: Event) -> pb::OpenShellResponse {
    pb::OpenShellResponse { event: Some(event) }
}

fn owner(ctx: &Ctx) -> Result<String, ConnectError> {
    ctx.session.as_ref().map(|s| s.token.clone()).ok_or_else(|| ConnectError::new(Code::Unauthenticated, "log in first"))
}

fn session(ctx: &Ctx, id: &str) -> Result<Arc<Session>, ConnectError> {
    let owner = owner(ctx)?;
    ctx.app.shells.get(id, &owner).ok_or_else(|| ConnectError::not_found("no such shell; it has ended"))
}

pub async fn open_shell(
    ctx: Ctx,
    req: pb::OpenShellRequest,
    sink: Sink<pb::OpenShellResponse>,
) -> Result<(), ConnectError> {
    let app = ctx.app.clone();
    let token = owner(&ctx)?;
    let size = ShellSize::clamped(req.cols, req.rows);
    let session = match app.shells.open(&token, size) {
        Ok(s) => s,
        Err(OpenError::TooMany) => {
            return Err(ConnectError::new(
                Code::ResourceExhausted,
                format!("{MAX_SHELLS} shells are already open.  Close one first."),
            ));
        }
        Err(OpenError::Spawn(e)) => return Err(ConnectError::internal(format!("cannot start a shell: {e}"))),
    };
    log::info!("web shell opened from {}", ctx.peer);
    let exit = run(&app, &token, &session, &sink).await;
    // Whatever ended the stream, the shell must not outlive it.
    app.shells.remove(&session.id);
    let exit = match exit {
        Some(exit) => exit,
        None => {
            // The client went away.
            session.terminate().await;
            log::info!("web shell from {} closed (client gone)", ctx.peer);
            return Ok(());
        }
    };
    let reason = exit.reason;
    let exit = if reason.is_empty() { exit } else { ShellExit { reason, ..session.terminate().await } };
    session.terminate().await;
    log::info!("web shell from {} ended ({})", ctx.peer, if reason.is_empty() { "exited" } else { reason });
    let _ = sink
        .send(&message(Event::Exited(Box::new(pb::ShellExited {
            exit_code: exit.code,
            signal: exit.signal,
            reason: exit.reason.to_owned(),
        }))))
        .await;
    Ok(())
}

/// Pumps the terminal's output to the client until the shell ends
/// (`Some`), or the client goes away (`None`).
async fn run(app: &App, token: &str, session: &Session, sink: &Sink<pb::OpenShellResponse>) -> Option<ShellExit> {
    sink.send(&message(Event::Started(Box::new(pb::ShellStarted { session_id: session.id.clone() })))).await.ok()?;
    let mut buf = vec![0u8; READ_BUF];
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_touch = tokio::time::Instant::now();
    loop {
        tokio::select! {
            r = session.read(&mut buf) => match r {
                Ok(0) | Err(_) => {
                    // Every process on the terminal is gone.
                    return Some(session.try_exit().unwrap_or_default());
                }
                Ok(n) => {
                    let mut out = buf[..n].to_vec();
                    tokio::time::sleep(COALESCE).await;
                    while out.len() < MAX_CHUNK {
                        match session.try_read(&mut buf) {
                            Some(n) => out.extend_from_slice(&buf[..n]),
                            None => break,
                        }
                    }
                    // Waits while the client is slow: output is not read
                    // from the terminal faster than it is delivered, so a
                    // runaway command blocks instead of filling memory.
                    sink.send(&message(Event::Output(out))).await.ok()?;
                }
            },
            _ = tick.tick() => {
                if let Some(exit) = session.try_exit() {
                    // The shell exited; pass on what it printed last.
                    let mut out = Vec::new();
                    while out.len() < MAX_CHUNK {
                        match session.try_read(&mut buf) {
                            Some(n) => out.extend_from_slice(&buf[..n]),
                            None => break,
                        }
                    }
                    if !out.is_empty() {
                        sink.send(&message(Event::Output(out))).await.ok()?;
                    }
                    return Some(exit);
                }
                if session.idle_for() >= SHELL_IDLE {
                    return Some(ShellExit { reason: "idle", ..Default::default() });
                }
                if session.silent_for() >= CLIENT_SILENCE {
                    return Some(ShellExit { reason: "page gone", ..Default::default() });
                }
                if last_touch.elapsed() >= TOUCH {
                    last_touch = tokio::time::Instant::now();
                    if app.sessions.touch(token).is_none() {
                        return Some(ShellExit { reason: "logged out", ..Default::default() });
                    }
                }
            }
            _ = session.close_requested() => return Some(ShellExit { reason: "closed", ..Default::default() }),
            _ = sink.closed() => return None,
        }
    }
}

pub async fn write_shell(ctx: Ctx, req: pb::WriteShellRequest) -> Result<pb::WriteShellResponse, ConnectError> {
    let session = session(&ctx, &req.session_id)?;
    if req.data.len() > MAX_INPUT {
        return Err(ConnectError::new(Code::InvalidArgument, "too much input in one request"));
    }
    // A program that stops reading its terminal must not hold the request
    // (and the connection) forever.
    match tokio::time::timeout(Duration::from_secs(10), session.write(&req.data)).await {
        Ok(Ok(())) => Ok(pb::WriteShellResponse {}),
        Ok(Err(e)) => Err(ConnectError::new(Code::Aborted, format!("the shell did not take the input: {e}"))),
        Err(_) => Err(ConnectError::new(Code::DeadlineExceeded, "the program is not reading its input")),
    }
}

pub async fn resize_shell(ctx: Ctx, req: pb::ResizeShellRequest) -> Result<pb::ResizeShellResponse, ConnectError> {
    let session = session(&ctx, &req.session_id)?;
    session
        .resize(ShellSize::clamped(req.cols, req.rows))
        .map_err(|e| ConnectError::internal(format!("cannot resize the terminal: {e}")))?;
    Ok(pb::ResizeShellResponse {})
}

pub async fn close_shell(ctx: Ctx, req: pb::CloseShellRequest) -> Result<pb::CloseShellResponse, ConnectError> {
    // Closing a shell that already ended is not an error.
    if let Ok(session) = session(&ctx, &req.session_id) {
        session.request_close();
    }
    Ok(pb::CloseShellResponse {})
}
