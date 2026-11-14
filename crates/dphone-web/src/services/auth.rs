//! `AuthService`.

use crate::auth as sess;
use crate::rpc::{Code, ConnectError};
use crate::http::Ctx;
use crate::pb;
use dphone_config::nvdata::WEB_USER;
use http::{header, HeaderValue};
use std::time::Duration;

/// Stock's factory password, flagged so the page can urge a change.
const DEFAULT_PASSWORD: &str = "789";

fn session_message(ctx: &Ctx, session: Option<&sess::Session>, default_password: bool) -> pb::Session {
    let sys = ctx.app.store.get::<dphone_state::System>();
    pb::Session {
        authenticated: session.is_some(),
        csrf_token: session.map(|s| s.csrf.clone()).unwrap_or_default(),
        user: session.map(|s| s.user.clone()).unwrap_or_default(),
        model: sys.model.name().to_owned(),
        version: sys.version.dotted(),
        hostname: sys.hostname.clone(),
        default_password,
        ..Default::default()
    }
}

async fn check_password(ctx: &Ctx, user: String, password: String) -> Result<bool, ConnectError> {
    super::with_config(&ctx.app, false, move |c| c.verify_web_password(&user, &password).map_err(crate::services::config_error)).await
}

pub async fn login(ctx: Ctx, req: pb::LoginRequest) -> Result<pb::LoginResponse, ConnectError> {
    if let Some(wait) = ctx.app.sessions.locked_for(ctx.peer) {
        return Err(ConnectError::new(
            Code::ResourceExhausted,
            format!("too many failed logins; try again in {} s", wait.as_secs().max(1)),
        ));
    }
    let user = if req.user.is_empty() { WEB_USER.to_owned() } else { req.user.clone() };
    let ok = check_password(&ctx, user.clone(), req.password.clone()).await?;
    if !ok {
        let lock = ctx.app.sessions.record_failure(ctx.peer);
        log::warn!("failed web login for {user:?} from {}", ctx.peer);
        // Slow every failure a little, whatever the lockout state.
        tokio::time::sleep(Duration::from_millis(400)).await;
        return Err(ConnectError::new(
            Code::Unauthenticated,
            match lock {
                Some(l) => format!("wrong user name or password; wait {} s before trying again", l.as_secs()),
                None => "wrong user name or password".to_owned(),
            },
        ));
    }
    ctx.app.sessions.record_success(ctx.peer);
    let session = ctx.app.sessions.create(&user);
    if let Ok(v) = HeaderValue::from_str(&sess::set_cookie(&session.token, ctx.secure)) {
        ctx.headers.add(header::SET_COOKIE, v);
    }
    log::info!("web login by {user:?} from {}", ctx.peer);
    Ok(pb::LoginResponse {
        session: Some(session_message(&ctx, Some(&session), req.password == DEFAULT_PASSWORD)).into(),
    })
}

pub async fn logout(ctx: Ctx, _req: pb::LogoutRequest) -> Result<pb::LogoutResponse, ConnectError> {
    if let Some(s) = &ctx.session {
        ctx.app.sessions.remove(&s.token);
    }
    ctx.headers.add(header::SET_COOKIE, HeaderValue::from_str(&sess::clear_cookie()).expect("ASCII"));
    Ok(pb::LogoutResponse {})
}

pub async fn get_session(ctx: Ctx, _req: pb::GetSessionRequest) -> Result<pb::GetSessionResponse, ConnectError> {
    // Disabled: the state and nothing else, whatever cookie came along.
    if !ctx.app.switch.enabled() {
        let session = pb::Session { web_disabled: true, ..Default::default() };
        return Ok(pb::GetSessionResponse { session: Some(session).into() });
    }
    let default_password = match &ctx.session {
        Some(s) => check_password(&ctx, s.user.clone(), DEFAULT_PASSWORD.to_owned()).await.unwrap_or(false),
        None => false,
    };
    Ok(pb::GetSessionResponse { session: Some(session_message(&ctx, ctx.session.as_ref(), default_password)).into() })
}

pub async fn change_password(
    ctx: Ctx,
    req: pb::ChangePasswordRequest,
) -> Result<pb::ChangePasswordResponse, ConnectError> {
    let session = ctx.session.clone().ok_or_else(|| ConnectError::new(Code::Unauthenticated, "log in first"))?;
    if req.new_password.len() < 4 || req.new_password.len() > 128 {
        return Err(ConnectError::invalid_argument("the new password must be 4 to 128 characters"));
    }
    // The stock digest file is `user:realm:hash` lines, so these cannot be
    // stored; refuse rather than write a file stock would misread.
    if req.new_password.chars().any(|c| c.is_control()) {
        return Err(ConnectError::invalid_argument("the new password contains a control character"));
    }
    if !check_password(&ctx, session.user.clone(), req.current_password.clone()).await? {
        tokio::time::sleep(Duration::from_millis(400)).await;
        return Err(ConnectError::new(Code::PermissionDenied, "the current password is wrong"));
    }
    let new = req.new_password.clone();
    super::with_config(&ctx.app, true, move |c| {
        c.set_web_password(&new).map_err(crate::services::config_error)?;
        Ok(())
    })
    .await?;
    ctx.app.sessions.remove_all_except(&session.token);
    log::info!("web password changed from {}", ctx.peer);
    Ok(pb::ChangePasswordResponse {})
}
