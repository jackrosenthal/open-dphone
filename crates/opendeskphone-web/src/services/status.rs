//! `StatusService`, the live parts of `NetworkService` and
//! `AccountsService`, and `ProvisioningService`.

use super::{command, watch};
use crate::rpc::{ConnectError, Sink};
use crate::http::Ctx;
use crate::{convert, pb, App};
use opendeskphone_state::{self as st, Command, Topic, TopicSet};

pub(crate) fn firmware_status(app: &App) -> pb::FirmwareStatus {
    let mut f = convert::firmware(&app.store.get::<st::Update>(), &app.store.get::<st::System>().version);
    let uploads = app.uploads.lock().unwrap();
    f.uploaded = uploads
        .iter()
        .find_map(|(id, u)| u.firmware.as_ref().map(|info| super::uploaded_firmware(app, id, u.size, info))).into();
    f
}

fn snapshot(app: &App) -> pb::StatusSnapshot {
    let s = app.store.snapshot();
    pb::StatusSnapshot {
        system: Some(convert::system(&s.system, app.started.elapsed().as_secs())).into(),
        network: Some(convert::network(&s.network, s.system.mac_string())).into(),
        provisioning: Some(convert::provisioning(&s.provisioning)).into(),
        accounts: convert::accounts(&s.accounts),
        calls: Some(convert::calls_snapshot(&s)).into(),
        firmware: Some(firmware_status(app)).into(),
        health: convert::health(&s.health),
        healthy: s.health.healthy,
        config_generation: s.config.generation,
        restart_needed: s.config.restart_needed,
        config_error: s.config.error.clone(),
    }
}

pub async fn get_status(ctx: Ctx, _req: pb::GetStatusRequest) -> Result<pb::GetStatusResponse, ConnectError> {
    Ok(pb::GetStatusResponse { status: Some(snapshot(&ctx.app)).into() })
}

pub async fn watch_status(
    ctx: Ctx,
    _req: pb::WatchStatusRequest,
    sink: Sink<pb::WatchStatusResponse>,
) -> Result<(), ConnectError> {
    let topics = TopicSet::ALL.without(Topic::Log).without(Topic::Ui).without(Topic::Keys);
    watch(ctx, topics, sink, true, |app| pb::WatchStatusResponse { status: Some(snapshot(app)).into() }).await
}

// --------------------------------------------------------------- network

fn network_status(app: &App) -> pb::NetworkStatus {
    convert::network(&app.store.get::<st::Network>(), app.store.get::<st::System>().mac_string())
}

pub async fn get_network(ctx: Ctx, _req: pb::GetNetworkRequest) -> Result<pb::GetNetworkResponse, ConnectError> {
    let settings = super::config::network_settings(&ctx.app);
    Ok(pb::GetNetworkResponse { status: Some(network_status(&ctx.app)).into(), settings: Some(settings).into() })
}

pub async fn watch_network(
    ctx: Ctx,
    _req: pb::WatchNetworkRequest,
    sink: Sink<pb::WatchNetworkResponse>,
) -> Result<(), ConnectError> {
    watch(ctx, [Topic::Network, Topic::System], sink, false, |app| pb::WatchNetworkResponse {
        status: Some(network_status(app)).into(),
    })
    .await
}

pub async fn renew_dhcp(ctx: Ctx, _req: pb::RenewDhcpRequest) -> Result<pb::RenewDhcpResponse, ConnectError> {
    command(&ctx.app.store, Command::RenewDhcp).await?;
    Ok(pb::RenewDhcpResponse {})
}

pub async fn sync_time(ctx: Ctx, _req: pb::SyncTimeRequest) -> Result<pb::SyncTimeResponse, ConnectError> {
    command(&ctx.app.store, Command::SyncTime).await?;
    Ok(pb::SyncTimeResponse {})
}

// ---------------------------------------------------------- provisioning

fn provisioning(app: &App) -> pb::ProvisioningStatus {
    convert::provisioning(&app.store.get::<st::Provisioning>())
}

pub async fn get_provisioning(
    ctx: Ctx,
    _req: pb::GetProvisioningRequest,
) -> Result<pb::GetProvisioningResponse, ConnectError> {
    Ok(pb::GetProvisioningResponse { status: Some(provisioning(&ctx.app)).into() })
}

pub async fn watch_provisioning(
    ctx: Ctx,
    _req: pb::WatchProvisioningRequest,
    sink: Sink<pb::WatchProvisioningResponse>,
) -> Result<(), ConnectError> {
    watch(ctx, Topic::Provisioning, sink, false, |app| pb::WatchProvisioningResponse {
        status: Some(provisioning(app)).into(),
    })
    .await
}

pub async fn start_provisioning(
    ctx: Ctx,
    req: pb::StartProvisioningRequest,
) -> Result<pb::StartProvisioningResponse, ConnectError> {
    let server = Some(req.server.trim().to_owned()).filter(|s| !s.is_empty());
    command(&ctx.app.store, Command::StartProvisioning { server }).await?;
    Ok(pb::StartProvisioningResponse {})
}

pub async fn select_user(ctx: Ctx, req: pb::SelectUserRequest) -> Result<pb::SelectUserResponse, ConnectError> {
    command(&ctx.app.store, Command::SelectUser { id: req.id, pin: req.pin }).await?;
    Ok(pb::SelectUserResponse {})
}

pub async fn reprovision(ctx: Ctx, _req: pb::ReprovisionRequest) -> Result<pb::ReprovisionResponse, ConnectError> {
    command(&ctx.app.store, Command::Reprovision).await?;
    Ok(pb::ReprovisionResponse {})
}

pub async fn forget_provisioning(
    ctx: Ctx,
    _req: pb::ForgetProvisioningRequest,
) -> Result<pb::ForgetProvisioningResponse, ConnectError> {
    command(&ctx.app.store, Command::ForgetProvisioning).await?;
    Ok(pb::ForgetProvisioningResponse {})
}

pub async fn skip_provisioning(
    ctx: Ctx,
    _req: pb::SkipProvisioningRequest,
) -> Result<pb::SkipProvisioningResponse, ConnectError> {
    command(&ctx.app.store, Command::SkipProvisioning).await?;
    Ok(pb::SkipProvisioningResponse {})
}

// -------------------------------------------------------------- accounts

pub async fn watch_accounts(
    ctx: Ctx,
    _req: pb::WatchAccountsRequest,
    sink: Sink<pb::WatchAccountsResponse>,
) -> Result<(), ConnectError> {
    watch(ctx, Topic::Accounts, sink, false, |app| pb::WatchAccountsResponse {
        status: convert::accounts(&app.store.get::<st::Accounts>()),
    })
    .await
}

pub async fn reregister(ctx: Ctx, req: pb::ReregisterRequest) -> Result<pb::ReregisterResponse, ConnectError> {
    let account = match req.index {
        Some(i) => Some(st::AccountId(u8::try_from(i).map_err(|_| ConnectError::invalid_argument("no such account"))?)),
        None => None,
    };
    command(&ctx.app.store, Command::Reregister { account }).await?;
    Ok(pb::ReregisterResponse {})
}
