//! `AlarmsService`: the alarm clock's state, and commands for the
//! subsystem that owns it.

use super::{command, watch};
use crate::http::Ctx;
use crate::rpc::{ConnectError, Sink};
use crate::{convert, pb, App};
use opendeskphone_state::{self as st, Command, Reply, Topic};

fn snapshot(app: &App) -> pb::AlarmsSnapshot {
    convert::alarms(&app.store.get::<st::Alarms>())
}

pub async fn get_alarms(ctx: Ctx, _req: pb::GetAlarmsRequest) -> Result<pb::GetAlarmsResponse, ConnectError> {
    Ok(pb::GetAlarmsResponse { snapshot: Some(snapshot(&ctx.app)).into() })
}

pub async fn watch_alarms(
    ctx: Ctx,
    _req: pb::WatchAlarmsRequest,
    sink: Sink<pb::WatchAlarmsResponse>,
) -> Result<(), ConnectError> {
    watch(ctx, Topic::Alarms, sink, false, |app| pb::WatchAlarmsResponse { snapshot: Some(snapshot(app)).into() }).await
}

/// The alarm of a request, with what cannot be an alarm refused here: the
/// subsystem would refuse it too, but as a failed command.
fn alarm(a: Option<&pb::Alarm>) -> Result<st::Alarm, ConnectError> {
    let a = a.ok_or_else(|| ConnectError::invalid_argument("An alarm is required."))?;
    if a.hour > 23 || a.minute > 59 {
        return Err(ConnectError::invalid_argument("Time must be between 00:00 and 23:59."));
    }
    if a.days > st::Alarm::EVERY_DAY.into() {
        return Err(ConnectError::invalid_argument("Days must be days of the week."));
    }
    Ok(st::Alarm {
        id: a.id,
        hour: a.hour as u8,
        minute: a.minute as u8,
        days: a.days as u8,
        tone: a.tone.clone(),
        enabled: a.enabled,
    })
}

pub async fn set_alarm(ctx: Ctx, req: pb::SetAlarmRequest) -> Result<pb::SetAlarmResponse, ConnectError> {
    let alarm = alarm(req.alarm.as_option())?;
    let alarms = ctx.app.store.get::<st::Alarms>();
    if alarm.id != 0 && alarms.get(alarm.id).is_none() {
        return Err(ConnectError::not_found("no such alarm"));
    }
    if alarm.id == 0 && alarms.alarms.len() >= st::Alarms::MAX {
        return Err(ConnectError::new(
            crate::rpc::Code::ResourceExhausted,
            format!("The phone keeps at most {} alarms.", st::Alarms::MAX),
        ));
    }
    let id = alarm.id;
    match command(&ctx.app.store, Command::SetAlarm { alarm }).await? {
        Reply::Id(id) => Ok(pb::SetAlarmResponse { id: id as u32 }),
        _ => Ok(pb::SetAlarmResponse { id }),
    }
}

pub async fn remove_alarm(ctx: Ctx, req: pb::RemoveAlarmRequest) -> Result<pb::RemoveAlarmResponse, ConnectError> {
    if ctx.app.store.get::<st::Alarms>().get(req.id).is_none() {
        return Err(ConnectError::not_found("no such alarm"));
    }
    command(&ctx.app.store, Command::RemoveAlarm { id: req.id }).await?;
    Ok(pb::RemoveAlarmResponse {})
}

pub async fn snooze_alarm(ctx: Ctx, _req: pb::SnoozeAlarmRequest) -> Result<pb::SnoozeAlarmResponse, ConnectError> {
    command(&ctx.app.store, Command::SnoozeAlarm).await?;
    Ok(pb::SnoozeAlarmResponse {})
}

pub async fn dismiss_alarm(ctx: Ctx, _req: pb::DismissAlarmRequest) -> Result<pb::DismissAlarmResponse, ConnectError> {
    command(&ctx.app.store, Command::DismissAlarm).await?;
    Ok(pb::DismissAlarmResponse {})
}
