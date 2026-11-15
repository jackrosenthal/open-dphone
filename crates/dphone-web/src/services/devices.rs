//! `BluetoothService`, `PagingService`, and `ExpansionService`: state
//! snapshots, and commands for the subsystems that own them.

use super::{command, watch};
use crate::http::Ctx;
use crate::rpc::{Code, ConnectError, Sink};
use crate::{convert, pb, App};
use dphone_state::{self as st, Command, Topic};

// ------------------------------------------------------------- Bluetooth

fn bluetooth_snapshot(app: &App) -> pb::BluetoothSnapshot {
    convert::bluetooth(&app.store.get::<st::Bluetooth>())
}

/// Refuses on a model without Bluetooth, where no subsystem would answer.
fn bluetooth_model(ctx: &Ctx) -> Result<(), ConnectError> {
    if ctx.app.store.model().bluetooth() {
        Ok(())
    } else {
        Err(ConnectError::new(Code::Unimplemented, "Bluetooth is not supported on this model."))
    }
}

/// A device address as the subsystem takes it: six hexadecimal pairs.
fn address(text: &str) -> Result<String, ConnectError> {
    let text = text.trim();
    let ok = text.len() == 17
        && text.split(':').count() == 6
        && text.split(':').all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()));
    if ok {
        Ok(text.to_ascii_uppercase())
    } else {
        Err(ConnectError::invalid_argument("not a Bluetooth address"))
    }
}

pub async fn get_bluetooth(ctx: Ctx, _req: pb::GetBluetoothRequest) -> Result<pb::GetBluetoothResponse, ConnectError> {
    Ok(pb::GetBluetoothResponse { snapshot: Some(bluetooth_snapshot(&ctx.app)).into() })
}

pub async fn watch_bluetooth(
    ctx: Ctx,
    _req: pb::WatchBluetoothRequest,
    sink: Sink<pb::WatchBluetoothResponse>,
) -> Result<(), ConnectError> {
    watch(ctx, Topic::Bluetooth, sink, false, |app| pb::WatchBluetoothResponse {
        snapshot: Some(bluetooth_snapshot(app)).into(),
    })
    .await
}

pub async fn set_bluetooth_enabled(
    ctx: Ctx,
    req: pb::SetBluetoothEnabledRequest,
) -> Result<pb::SetBluetoothEnabledResponse, ConnectError> {
    bluetooth_model(&ctx)?;
    command(&ctx.app.store, Command::SetBluetooth { on: req.enabled }).await?;
    Ok(pb::SetBluetoothEnabledResponse {})
}

pub async fn set_bluetooth_discovery(
    ctx: Ctx,
    req: pb::SetBluetoothDiscoveryRequest,
) -> Result<pb::SetBluetoothDiscoveryResponse, ConnectError> {
    bluetooth_model(&ctx)?;
    command(&ctx.app.store, Command::BluetoothDiscovery { on: req.discovering }).await?;
    Ok(pb::SetBluetoothDiscoveryResponse {})
}

pub async fn pair_bluetooth_device(
    ctx: Ctx,
    req: pb::PairBluetoothDeviceRequest,
) -> Result<pb::PairBluetoothDeviceResponse, ConnectError> {
    bluetooth_model(&ctx)?;
    command(&ctx.app.store, Command::BluetoothPair { addr: address(&req.address)? }).await?;
    Ok(pb::PairBluetoothDeviceResponse {})
}

pub async fn answer_bluetooth_pairing(
    ctx: Ctx,
    req: pb::AnswerBluetoothPairingRequest,
) -> Result<pb::AnswerBluetoothPairingResponse, ConnectError> {
    bluetooth_model(&ctx)?;
    let addr = address(&req.address)?;
    // Legacy PINs are 1 to 16 bytes.
    let pin = req.pin.filter(|p| !p.is_empty());
    if pin.as_ref().is_some_and(|p| p.len() > 16) {
        return Err(ConnectError::invalid_argument("PIN must be 16 characters or fewer."));
    }
    command(&ctx.app.store, Command::BluetoothAnswerPairing { addr, accept: req.accept, pin }).await?;
    Ok(pb::AnswerBluetoothPairingResponse {})
}

pub async fn remove_bluetooth_device(
    ctx: Ctx,
    req: pb::RemoveBluetoothDeviceRequest,
) -> Result<pb::RemoveBluetoothDeviceResponse, ConnectError> {
    bluetooth_model(&ctx)?;
    command(&ctx.app.store, Command::BluetoothUnpair { addr: address(&req.address)? }).await?;
    Ok(pb::RemoveBluetoothDeviceResponse {})
}

pub async fn connect_bluetooth_device(
    ctx: Ctx,
    req: pb::ConnectBluetoothDeviceRequest,
) -> Result<pb::ConnectBluetoothDeviceResponse, ConnectError> {
    bluetooth_model(&ctx)?;
    command(&ctx.app.store, Command::BluetoothConnect { addr: address(&req.address)? }).await?;
    Ok(pb::ConnectBluetoothDeviceResponse {})
}

pub async fn disconnect_bluetooth_device(
    ctx: Ctx,
    _req: pb::DisconnectBluetoothDeviceRequest,
) -> Result<pb::DisconnectBluetoothDeviceResponse, ConnectError> {
    bluetooth_model(&ctx)?;
    command(&ctx.app.store, Command::BluetoothDisconnect).await?;
    Ok(pb::DisconnectBluetoothDeviceResponse {})
}

// ---------------------------------------------------------------- paging

fn paging_snapshot(app: &App) -> pb::PagingSnapshot {
    convert::paging(&app.store.get::<st::Paging>())
}

pub async fn get_paging(ctx: Ctx, _req: pb::GetPagingRequest) -> Result<pb::GetPagingResponse, ConnectError> {
    Ok(pb::GetPagingResponse { snapshot: Some(paging_snapshot(&ctx.app)).into() })
}

pub async fn watch_paging(
    ctx: Ctx,
    _req: pb::WatchPagingRequest,
    sink: Sink<pb::WatchPagingResponse>,
) -> Result<(), ConnectError> {
    watch(ctx, Topic::Paging, sink, false, |app| pb::WatchPagingResponse { snapshot: Some(paging_snapshot(app)).into() })
        .await
}

pub async fn start_page(ctx: Ctx, req: pb::StartPageRequest) -> Result<pb::StartPageResponse, ConnectError> {
    if !ctx.app.store.get::<st::Paging>().broadcasts.iter().any(|g| g.id == req.id) {
        return Err(ConnectError::not_found("no such page group"));
    }
    command(&ctx.app.store, Command::StartPage { id: req.id }).await?;
    Ok(pb::StartPageResponse {})
}

pub async fn stop_page(ctx: Ctx, _req: pb::StopPageRequest) -> Result<pb::StopPageResponse, ConnectError> {
    command(&ctx.app.store, Command::StopPage).await?;
    Ok(pb::StopPageResponse {})
}

// ------------------------------------------------------------- expansion

fn expansion_snapshot(app: &App) -> pb::ExpansionSnapshot {
    convert::expansion(&app.store.get::<st::Expansion>())
}

pub async fn get_expansion(ctx: Ctx, _req: pb::GetExpansionRequest) -> Result<pb::GetExpansionResponse, ConnectError> {
    Ok(pb::GetExpansionResponse { snapshot: Some(expansion_snapshot(&ctx.app)).into() })
}

pub async fn watch_expansion(
    ctx: Ctx,
    _req: pb::WatchExpansionRequest,
    sink: Sink<pb::WatchExpansionResponse>,
) -> Result<(), ConnectError> {
    watch(ctx, Topic::Expansion, sink, false, |app| pb::WatchExpansionResponse {
        snapshot: Some(expansion_snapshot(app)).into(),
    })
    .await
}
