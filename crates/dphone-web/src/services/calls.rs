//! `CallsService`.

use super::{command, watch};
use crate::connect::{ConnectError, Sink};
use crate::http::Ctx;
use crate::{convert, pb};
use dphone_state::{self as st, CallId, Command, ForwardKind, Reply, Topic};

fn snapshot(app: &crate::App) -> pb::CallsSnapshot {
    convert::calls_snapshot(&app.store.snapshot())
}

pub async fn get_calls(ctx: Ctx, _req: pb::GetCallsRequest) -> Result<pb::GetCallsResponse, ConnectError> {
    Ok(pb::GetCallsResponse { snapshot: Some(snapshot(&ctx.app)) })
}

pub async fn watch_calls(
    ctx: Ctx,
    _req: pb::WatchCallsRequest,
    sink: Sink<pb::WatchCallsResponse>,
) -> Result<(), ConnectError> {
    let topics = [Topic::Calls, Topic::Audio, Topic::Features, Topic::Mwi, Topic::CallLog];
    watch(ctx, topics, sink, false, |app| pb::WatchCallsResponse { snapshot: Some(snapshot(app)) }).await
}

pub async fn dial(ctx: Ctx, req: pb::DialRequest) -> Result<pb::DialResponse, ConnectError> {
    let target = req.target.trim().to_owned();
    if target.is_empty() {
        return Err(ConnectError::invalid("nothing to dial"));
    }
    if target.len() > 256 || target.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(ConnectError::invalid("not a number or SIP address"));
    }
    let account = match req.account {
        Some(i) => Some(st::AccountId(u8::try_from(i).map_err(|_| ConnectError::invalid("no such account"))?)),
        None => None,
    };
    let path = convert::audio_path_from(req.path);
    match command(&ctx.app.store, Command::Dial { target, account, path }).await? {
        Reply::Call(id) => Ok(pb::DialResponse { call: id.0 }),
        _ => Ok(pb::DialResponse { call: 0 }),
    }
}

pub async fn answer(ctx: Ctx, req: pb::AnswerRequest) -> Result<pb::AnswerResponse, ConnectError> {
    let path = convert::audio_path_from(req.path);
    command(&ctx.app.store, Command::Answer { call: CallId(req.call), path }).await?;
    Ok(pb::AnswerResponse {})
}

pub async fn hangup(ctx: Ctx, req: pb::HangupRequest) -> Result<pb::HangupResponse, ConnectError> {
    command(&ctx.app.store, Command::Hangup { call: CallId(req.call) }).await?;
    Ok(pb::HangupResponse {})
}

pub async fn hold(ctx: Ctx, req: pb::HoldRequest) -> Result<pb::HoldResponse, ConnectError> {
    command(&ctx.app.store, Command::Hold { call: CallId(req.call) }).await?;
    Ok(pb::HoldResponse {})
}

pub async fn resume(ctx: Ctx, req: pb::ResumeRequest) -> Result<pb::ResumeResponse, ConnectError> {
    command(&ctx.app.store, Command::Resume { call: CallId(req.call) }).await?;
    Ok(pb::ResumeResponse {})
}

pub async fn transfer(ctx: Ctx, req: pb::TransferRequest) -> Result<pb::TransferResponse, ConnectError> {
    let call = CallId(req.call);
    let cmd = match req.to {
        Some(pb::transfer_request::To::Target(t)) if !t.trim().is_empty() => {
            Command::TransferBlind { call, target: t.trim().to_owned() }
        }
        Some(pb::transfer_request::To::ConsultCall(c)) => Command::TransferAttended { call, consult: CallId(c) },
        _ => return Err(ConnectError::invalid("no transfer target")),
    };
    command(&ctx.app.store, cmd).await?;
    Ok(pb::TransferResponse {})
}

pub async fn send_dtmf(ctx: Ctx, req: pb::SendDtmfRequest) -> Result<pb::SendDtmfResponse, ConnectError> {
    if req.digits.is_empty() || req.digits.len() > 64 {
        return Err(ConnectError::invalid("1 to 64 digits"));
    }
    if let Some(bad) = req.digits.chars().find(|c| !matches!(c, '0'..='9' | '*' | '#' | 'A'..='D')) {
        return Err(ConnectError::invalid(format!("not a DTMF digit: {bad:?}")));
    }
    for digit in req.digits.chars() {
        command(&ctx.app.store, Command::Dtmf { call: CallId(req.call), digit }).await?;
    }
    Ok(pb::SendDtmfResponse {})
}

pub async fn set_mute(ctx: Ctx, req: pb::SetMuteRequest) -> Result<pb::SetMuteResponse, ConnectError> {
    command(&ctx.app.store, Command::SetMute { on: req.muted }).await?;
    Ok(pb::SetMuteResponse {})
}

pub async fn set_dnd(ctx: Ctx, req: pb::SetDndRequest) -> Result<pb::SetDndResponse, ConnectError> {
    command(&ctx.app.store, Command::SetDnd { on: req.dnd }).await?;
    Ok(pb::SetDndResponse {})
}

pub async fn set_forward(ctx: Ctx, req: pb::SetForwardRequest) -> Result<pb::SetForwardResponse, ConnectError> {
    let kind = match req.kind.as_str() {
        "always" => ForwardKind::Always,
        "busy" => ForwardKind::Busy,
        "no_answer" => ForwardKind::NoAnswer,
        other => return Err(ConnectError::invalid(format!("unknown forward kind {other:?}"))),
    };
    let no_answer_secs = u16::try_from(req.no_answer_secs).ok().filter(|s| *s > 0);
    command(&ctx.app.store, Command::SetForward { kind, target: req.target.trim().to_owned(), no_answer_secs })
        .await?;
    Ok(pb::SetForwardResponse {})
}

pub async fn get_call_log(ctx: Ctx, _req: pb::GetCallLogRequest) -> Result<pb::GetCallLogResponse, ConnectError> {
    let log = ctx.app.store.get::<st::CallLog>();
    Ok(pb::GetCallLogResponse { entries: convert::call_log(&log), missed_unseen: log.missed_unseen })
}

pub async fn clear_call_log(
    ctx: Ctx,
    _req: pb::ClearCallLogRequest,
) -> Result<pb::ClearCallLogResponse, ConnectError> {
    command(&ctx.app.store, Command::ClearCallLog).await?;
    Ok(pb::ClearCallLogResponse {})
}
