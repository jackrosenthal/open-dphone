//! The Connect services.  [`dispatch`] routes a path to its handler.

mod auth;
mod calls;
mod config;
mod status;
mod system;

use crate::connect::{self, Body, Code, ConnectError, Message, RpcRequest, Sink};
use crate::http::Ctx;
use crate::{pb, App, FirmwareInfo};
use dphone_config::{ConfigError, ConfigStore};
use dphone_state::{Command, CommandError, Reply, Store, TopicSet};
use http::Response;
use std::time::Duration;

/// How long a command may take before the RPC gives up.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
/// Bursts of state changes within this window become one stream message.
const COALESCE: Duration = Duration::from_millis(120);
/// A watch stream re-sends at least this often (which also keeps the
/// session alive while a page is open, and gets past idle proxies).
const REFRESH: Duration = Duration::from_secs(20);

macro_rules! routes {
    ($path:expr, $ctx:expr, $req:expr; $( $kind:ident $p:literal => $f:path, )*) => {
        match $path {
            $( concat!("/dphone.v1.", $p) => connect::$kind($ctx, $req, $f).await, )*
            other => connect::error_for(
                &$req,
                &ConnectError::new(Code::Unimplemented, format!("no such method: {other}")),
            ),
        }
    };
}

pub(crate) async fn dispatch(ctx: Ctx, path: &str, req: RpcRequest) -> Response<Body> {
    routes!(path, ctx, req;
        unary "AuthService/Login" => auth::login,
        unary "AuthService/Logout" => auth::logout,
        unary "AuthService/GetSession" => auth::get_session,
        unary "AuthService/ChangePassword" => auth::change_password,

        unary "StatusService/GetStatus" => status::get_status,
        stream "StatusService/WatchStatus" => status::watch_status,

        unary "NetworkService/GetNetwork" => status::get_network,
        stream "NetworkService/WatchNetwork" => status::watch_network,
        unary "NetworkService/SetNetworkSettings" => config::set_network_settings,
        unary "NetworkService/RenewDhcp" => status::renew_dhcp,
        unary "NetworkService/SyncTime" => status::sync_time,

        unary "ProvisioningService/GetProvisioning" => status::get_provisioning,
        stream "ProvisioningService/WatchProvisioning" => status::watch_provisioning,
        unary "ProvisioningService/StartProvisioning" => status::start_provisioning,
        unary "ProvisioningService/SelectUser" => status::select_user,
        unary "ProvisioningService/Reprovision" => status::reprovision,
        unary "ProvisioningService/ForgetProvisioning" => status::forget_provisioning,
        unary "ProvisioningService/SkipProvisioning" => status::skip_provisioning,

        unary "AccountsService/ListAccounts" => config::list_accounts,
        stream "AccountsService/WatchAccounts" => status::watch_accounts,
        unary "AccountsService/SetAccounts" => config::set_accounts,
        unary "AccountsService/ClearLocalAccounts" => config::clear_local_accounts,
        unary "AccountsService/Reregister" => status::reregister,

        unary "ConfigService/ListSettings" => config::list_settings,
        unary "ConfigService/SetSettings" => config::set_settings,
        unary "ConfigService/UnsetSettings" => config::unset_settings,

        unary "ContactsService/ListContactLists" => config::list_contact_lists,
        unary "ContactsService/GetContactList" => config::get_contact_list,
        unary "ContactsService/PutContactList" => config::put_contact_list,
        unary "ContactsService/DeleteContactList" => config::delete_contact_list,
        unary "ContactsService/ExportContactList" => config::export_contact_list,
        unary "ContactsService/ImportContactList" => config::import_contact_list,

        unary "CallsService/GetCalls" => calls::get_calls,
        stream "CallsService/WatchCalls" => calls::watch_calls,
        unary "CallsService/Dial" => calls::dial,
        unary "CallsService/Answer" => calls::answer,
        unary "CallsService/Hangup" => calls::hangup,
        unary "CallsService/Hold" => calls::hold,
        unary "CallsService/Resume" => calls::resume,
        unary "CallsService/Transfer" => calls::transfer,
        unary "CallsService/SendDtmf" => calls::send_dtmf,
        unary "CallsService/SetMute" => calls::set_mute,
        unary "CallsService/SetDnd" => calls::set_dnd,
        unary "CallsService/SetForward" => calls::set_forward,
        unary "CallsService/GetCallLog" => calls::get_call_log,
        unary "CallsService/ClearCallLog" => calls::clear_call_log,

        unary "AppsService/ListApps" => system::list_apps,
        stream "AppsService/WatchApps" => system::watch_apps,
        unary "AppsService/LaunchApp" => system::launch_app,
        unary "AppsService/CloseApp" => system::close_app,
        unary "AppsService/InstallApp" => system::install_app,
        unary "AppsService/RemoveApp" => system::remove_app,

        unary "FirmwareService/GetFirmware" => system::get_firmware,
        stream "FirmwareService/WatchFirmware" => system::watch_firmware,
        unary "FirmwareService/InstallFirmware" => system::install_firmware,
        unary "FirmwareService/DiscardUpload" => system::discard_upload,
        unary "FirmwareService/ConfirmTrial" => system::confirm_trial,
        unary "FirmwareService/Rollback" => system::rollback,

        unary "LogsService/GetLogs" => system::get_logs,
        stream "LogsService/TailLogs" => system::tail_logs,

        unary "ScreenService/GetScreenLayout" => system::get_screen_layout,
        stream "ScreenService/WatchScreen" => system::watch_screen,
        unary "ScreenService/PressKey" => system::press_key,
        unary "ScreenService/SetHook" => system::set_hook,

        unary "SystemService/Reboot" => system::reboot,
        unary "SystemService/FactoryReset" => system::factory_reset,
        unary "SystemService/SetTime" => system::set_time,
        unary "SystemService/CreateBackup" => system::create_backup,
        unary "SystemService/RestoreBackup" => system::restore_backup,
    )
}

impl From<CommandError> for ConnectError {
    fn from(e: CommandError) -> Self {
        let code = match &e {
            CommandError::NoHandler(_) | CommandError::Dropped => Code::Unavailable,
            CommandError::Timeout => Code::DeadlineExceeded,
            CommandError::Invalid(_) => Code::FailedPrecondition,
            CommandError::Failed(_) => Code::Unknown,
            CommandError::Denied(_) => Code::PermissionDenied,
        };
        let message = match e {
            CommandError::Invalid(s) | CommandError::Failed(s) | CommandError::Denied(s) => s,
            CommandError::NoHandler(d) => format!("the {d:?} subsystem is not running"),
            other => other.to_string(),
        };
        ConnectError::new(code, message)
    }
}

impl From<ConfigError> for ConnectError {
    fn from(e: ConfigError) -> Self {
        let code = match &e {
            ConfigError::Io(_) => Code::Internal,
            ConfigError::Parse(_) | ConfigError::TooManyAccounts { .. } => Code::InvalidArgument,
            ConfigError::Frozen(_) | ConfigError::ReadOnlyLayer(_) => Code::FailedPrecondition,
        };
        ConnectError::new(code, e.to_string())
    }
}

/// Sends a command to its subsystem and waits for the answer.
pub(crate) async fn command(store: &Store, cmd: Command) -> Result<Reply, ConnectError> {
    match tokio::time::timeout(COMMAND_TIMEOUT, store.send(cmd)).await {
        Ok(r) => Ok(r?),
        Err(_) => Err(CommandError::Timeout.into()),
    }
}

/// Runs a configuration operation off the reactor thread (writes fsync on
/// flash).  After a write, pass `changed = true` so the configuration
/// owner republishes.
pub(crate) async fn with_config<R, F>(app: &std::sync::Arc<App>, changed: bool, f: F) -> Result<R, ConnectError>
where
    R: Send + 'static,
    F: FnOnce(&mut ConfigStore) -> Result<R, ConnectError> + Send + 'static,
{
    let config = app.config.clone();
    let out = tokio::task::spawn_blocking(move || {
        let mut g = config.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut g)
    })
    .await
    .map_err(|e| ConnectError::internal(format!("configuration task failed: {e}")))??;
    if changed {
        // Nobody handling Config commands (the simulator, early boot) is
        // fine: the write itself has already been merged.
        app.store.post(Command::ReloadConfig);
    }
    Ok(out)
}

/// The body of every "watch" RPC: send a snapshot now, then again when a
/// subscribed topic changes and the snapshot differs.
pub(crate) async fn watch<Res, F>(
    ctx: Ctx,
    topics: impl Into<TopicSet>,
    sink: Sink<Res>,
    always_refresh: bool,
    make: F,
) -> Result<(), ConnectError>
where
    Res: Message + PartialEq,
    F: Fn(&App) -> Res + Send + 'static,
{
    let app = ctx.app.clone();
    let token = ctx.session.as_ref().map(|s| s.token.clone());
    let sub = app.store.subscribe(topics);
    let mut last: Option<Res> = None;
    loop {
        if let Some(t) = &token
            && app.sessions.touch(t).is_none() {
                return Err(ConnectError::new(Code::Unauthenticated, "session ended"));
            }
        let msg = make(&app);
        if always_refresh || last.as_ref() != Some(&msg) {
            sink.send(&msg).await?;
            last = Some(msg);
        }
        tokio::select! {
            _ = sub.changed() => tokio::time::sleep(COALESCE).await,
            _ = tokio::time::sleep(REFRESH) => {}
            _ = sink.closed() => return Ok(()),
        }
        let _ = sub.try_take();
    }
}

/// The message describing an uploaded firmware image.
pub(crate) fn uploaded_firmware(app: &App, id: &str, size: u64, info: &FirmwareInfo) -> pb::UploadedFirmware {
    let model = app.store.model();
    pb::UploadedFirmware {
        upload_id: id.to_owned(),
        size,
        version: info.version.clone(),
        model: info.model.clone(),
        is_stock: info.is_stock,
        model_matches: info.model.is_empty() || info.model.eq_ignore_ascii_case(model.name()),
    }
}
