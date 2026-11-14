//! `SshService` (the SSH server's switch and its authorized keys), the
//! provisioned-firmware policy of `FirmwareService`, and the serial console
//! switch of `SystemService`: Open DPhone's own
//! settings, which live in `/nvdata/dphone_settings.txtpb`.

use super::{config_error, with_config};
use crate::http::Ctx;
use crate::rpc::{Code, ConnectError};
use crate::{pb, sshkeys, App};
use dphone_config::{parse_authorized_key, ConfigStore, Resolved, SettingSource};
use std::sync::Arc;

pub(crate) fn bool_setting(r: Resolved<bool>) -> pb::BoolSetting {
    pb::BoolSetting {
        value: r.value,
        source: match r.source {
            SettingSource::Default => pb::SettingSource::SETTING_SOURCE_DEFAULT,
            SettingSource::Local => pb::SettingSource::SETTING_SOURCE_LOCAL,
            SettingSource::Provisioned => pb::SettingSource::SETTING_SOURCE_PROVISIONED,
        }
        .into(),
        locked: r.locked,
    }
}

fn locked() -> ConnectError {
    ConnectError::new(Code::FailedPrecondition, "This setting is fixed by the provisioning server.")
}

fn key_message(key: sshkeys::PublicKey, provisioned: bool) -> pb::SshKey {
    pb::SshKey { r#type: key.key_type, fingerprint: key.fingerprint, comment: key.comment, bits: key.bits, provisioned }
}

/// The keys in a list of `authorized_keys` lines that are real keys.
fn described(lines: &[String]) -> Vec<sshkeys::PublicKey> {
    lines
        .iter()
        .filter_map(|l| parse_authorized_key(l))
        .filter_map(|k| sshkeys::describe_existing(k.key_type, k.blob, k.comment))
        .collect()
}

fn status(c: &ConfigStore, listening: Option<bool>) -> Result<pb::SshStatus, ConnectError> {
    let config = c.config();
    let local = described(&c.nvdata().authorized_keys().map_err(|e| config_error(e.into()))?);
    let provisioned = described(&config.provisioned_ssh_keys());
    let mut keys: Vec<pb::SshKey> = Vec::new();
    for key in provisioned {
        // A key that is also in the local file is listed once, as provisioned:
        // removing the local copy would not stop it from being accepted.
        keys.push(key_message(key, true));
    }
    for key in local {
        if !keys.iter().any(|k| k.fingerprint == key.fingerprint) {
            keys.push(key_message(key, false));
        }
    }
    keys.sort_by_key(|k| k.provisioned);
    Ok(pb::SshStatus { enabled: Some(bool_setting(config.ssh_enabled())).into(), keys, listening })
}

async fn current(app: &Arc<App>) -> Result<pb::SshStatus, ConnectError> {
    let listening = app.ssh.as_ref().and_then(|s| s.listening());
    with_config(app, false, move |c| status(c, listening)).await
}

pub async fn get_ssh(ctx: Ctx, _req: pb::GetSshRequest) -> Result<pb::GetSshResponse, ConnectError> {
    Ok(pb::GetSshResponse { status: Some(current(&ctx.app).await?).into() })
}

pub async fn set_ssh_enabled(
    ctx: Ctx,
    req: pb::SetSshEnabledRequest,
) -> Result<pb::SetSshEnabledResponse, ConnectError> {
    let app = ctx.app.clone();
    let enabled = req.enabled;
    let control = app.ssh.clone();
    with_config(&app, true, move |c| {
        if c.config().ssh_enabled().locked {
            return Err(locked());
        }
        match &control {
            // The integrator stores the setting and starts or stops the
            // listener.  The configuration lock is released first: its
            // implementation takes it.
            Some(_) => Ok(()),
            None => c.update_dphone_settings(|s| s.ssh_enabled = Some(enabled)).map(|_| ()).map_err(config_error),
        }
    })
    .await?;
    if let Some(control) = app.ssh.clone() {
        tokio::task::spawn_blocking(move || control.set_enabled(enabled))
            .await
            .map_err(|e| ConnectError::internal(format!("SSH control task failed: {e}")))?
            .map_err(|e| ConnectError::internal(format!("cannot switch the SSH server: {e}")))?;
    }
    log::info!("SSH server switched {} from the web UI ({})", if enabled { "on" } else { "off" }, ctx.peer);
    Ok(pb::SetSshEnabledResponse { status: Some(current(&app).await?).into() })
}

pub async fn add_ssh_key(ctx: Ctx, req: pb::AddSshKeyRequest) -> Result<pb::AddSshKeyResponse, ConnectError> {
    // Nothing of a refused input reaches the log or the error text: it may
    // be a private key.
    let key = sshkeys::parse_new_key(&req.key).map_err(|e| ConnectError::new(Code::InvalidArgument, e.to_string()))?;
    drop(req);
    let app = ctx.app.clone();
    let fingerprint = key.fingerprint.clone();
    with_config(&app, false, move |c| {
        let known = described(&c.ssh_authorized_keys().map_err(config_error)?);
        if known.iter().any(|k| k.fingerprint == key.fingerprint) {
            return Err(ConnectError::new(Code::AlreadyExists, "That key is already authorized."));
        }
        c.nvdata().add_authorized_key(&key.line()).map_err(|e| config_error(e.into()))?;
        Ok(())
    })
    .await?;
    log::info!("SSH key {fingerprint} added from the web UI ({})", ctx.peer);
    Ok(pb::AddSshKeyResponse { status: Some(current(&app).await?).into() })
}

pub async fn remove_ssh_key(
    ctx: Ctx,
    req: pb::RemoveSshKeyRequest,
) -> Result<pb::RemoveSshKeyResponse, ConnectError> {
    let app = ctx.app.clone();
    let fingerprint = req.fingerprint;
    let wanted = fingerprint.clone();
    with_config(&app, false, move |c| {
        let lines = c.nvdata().authorized_keys().map_err(|e| config_error(e.into()))?;
        let local = described(&lines);
        let Some(key) = local.iter().find(|k| k.fingerprint == wanted) else {
            let provisioned = described(&c.config().provisioned_ssh_keys());
            return Err(if provisioned.iter().any(|k| k.fingerprint == wanted) {
                ConnectError::new(Code::FailedPrecondition, "That key comes from the provisioning server.")
            } else {
                ConnectError::not_found("no such key")
            });
        };
        // The file may hold the key with different base64 padding or
        // options; match lines by the key they decode to.
        for line in &lines {
            if let Some(parts) = parse_authorized_key(line)
                && sshkeys::describe_existing(parts.key_type, parts.blob, "").is_some_and(|k| k.blob == key.blob)
            {
                c.nvdata().remove_authorized_key(parts.blob).map_err(|e| config_error(e.into()))?;
            }
        }
        Ok(())
    })
    .await?;
    log::info!("SSH key {fingerprint} removed from the web UI ({})", ctx.peer);
    Ok(pb::RemoveSshKeyResponse { status: Some(current(&app).await?).into() })
}

// ------------------------------------------------- provisioned firmware

pub async fn get_provisioned_firmware_policy(
    ctx: Ctx,
    _req: pb::GetProvisionedFirmwarePolicyRequest,
) -> Result<pb::GetProvisionedFirmwarePolicyResponse, ConnectError> {
    let ignore = with_config(&ctx.app, false, |c| Ok(bool_setting(c.config().ignore_provisioned_firmware()))).await?;
    Ok(pb::GetProvisionedFirmwarePolicyResponse { ignore: Some(ignore).into() })
}

pub async fn set_provisioned_firmware_policy(
    ctx: Ctx,
    req: pb::SetProvisionedFirmwarePolicyRequest,
) -> Result<pb::SetProvisionedFirmwarePolicyResponse, ConnectError> {
    let ignore = req.ignore;
    let setting = with_config(&ctx.app, true, move |c| {
        if c.config().ignore_provisioned_firmware().locked {
            return Err(locked());
        }
        let config = c.update_dphone_settings(|s| s.ignore_provisioned_firmware = Some(ignore)).map_err(config_error)?;
        Ok(bool_setting(config.ignore_provisioned_firmware()))
    })
    .await?;
    Ok(pb::SetProvisionedFirmwarePolicyResponse { ignore: Some(setting).into() })
}

// ------------------------------------------------------- serial console

pub async fn get_serial_console(
    ctx: Ctx,
    _req: pb::GetSerialConsoleRequest,
) -> Result<pb::GetSerialConsoleResponse, ConnectError> {
    let setting = with_config(&ctx.app, false, |c| Ok(bool_setting(c.config().serial_console()))).await?;
    Ok(pb::GetSerialConsoleResponse { serial_console: Some(setting).into() })
}

pub async fn set_serial_console(
    ctx: Ctx,
    req: pb::SetSerialConsoleRequest,
) -> Result<pb::SetSerialConsoleResponse, ConnectError> {
    let on = req.serial_console;
    let setting = with_config(&ctx.app, true, move |c| {
        if c.config().serial_console().locked {
            return Err(locked());
        }
        let config = c.update_dphone_settings(|s| s.serial_console = Some(on)).map_err(config_error)?;
        Ok(bool_setting(config.serial_console()))
    })
    .await?;
    log::info!("serial console switched {} from the web UI ({})", if on { "on" } else { "off" }, ctx.peer);
    Ok(pb::SetSerialConsoleResponse { serial_console: Some(setting).into() })
}
