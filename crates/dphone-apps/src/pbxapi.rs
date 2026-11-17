//! The app interface `pbx`: the request document of each method of the
//! PBX's application server, and the typed result that the method's
//! answer is read into.  The names and the fields are those Open DPMA
//! implements (`open-dpma/crates/res_open_dpma/src/method_api`), which
//! are the original DPMA's.
//!
//! The server writes numbers both as JSON numbers and as strings, and
//! writes a list of one element as the element itself; the readers here
//! take either.

use crate::host::bindings::dphone::app::types::*;
use serde_json::{Map, Value};

/// How an answer's `result` is read: `None` when it is not the method's.
pub(crate) type Reader = fn(&Value) -> Option<PbxResult>;

/// A method, its parameters, and how its answer is read.
pub(crate) struct Call {
    pub method: &'static str,
    pub parameters: Map<String, Value>,
    pub read: Reader,
}

impl Call {
    fn new(method: &'static str, read: Reader) -> Call {
        Call { method, parameters: Map::new(), read }
    }

    fn with(mut self, name: &str, value: impl Into<Value>) -> Call {
        self.parameters.insert(name.to_string(), value.into());
        self
    }
}

fn done(_: &Value) -> Option<PbxResult> {
    Some(PbxResult::Done)
}

/// The text of a string, a number, or a boolean; "" for anything else.
fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// A count: a number, or a string that holds one.
fn count(v: &Value) -> Option<u32> {
    match v {
        Value::Number(n) => n.as_u64().map(|n| n.min(u32::MAX as u64) as u32),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_u64().is_some_and(|n| n != 0),
        Value::String(s) => ["1", "true", "yes", "on"].iter().any(|t| s.trim().eq_ignore_ascii_case(t)),
        _ => false,
    }
}

/// The elements of a list; a single element stands for a list of one.
fn items(v: &Value) -> Vec<&Value> {
    match v {
        Value::Null => Vec::new(),
        Value::Array(a) => a.iter().collect(),
        other => vec![other],
    }
}

fn ids(messages: Vec<String>) -> Value {
    Value::Array(messages.into_iter().map(Value::String).collect())
}

/// The queue parameter of the queue actions: `all` for every queue.
fn queue_id(queue: Option<String>) -> String {
    queue.unwrap_or_else(|| "all".to_string())
}

fn presence_state(v: &Value) -> PbxPresenceState {
    match v.as_str().unwrap_or_default().to_ascii_lowercase().as_str() {
        "unavailable" => PbxPresenceState::Unavailable,
        "available" => PbxPresenceState::Available,
        "away" => PbxPresenceState::Away,
        "xa" => PbxPresenceState::Xa,
        "chat" => PbxPresenceState::Chat,
        "dnd" => PbxPresenceState::Dnd,
        _ => PbxPresenceState::NotSet,
    }
}

fn membership(v: &Value) -> QueueMembership {
    match v.as_str().unwrap_or_default() {
        "logged_in" => QueueMembership::LoggedIn,
        "permanent" => QueueMembership::Permanent,
        "logged_out" => QueueMembership::LoggedOut,
        _ => QueueMembership::Manager,
    }
}

/// Seconds since the Unix epoch of a voicemail message's date: a number,
/// or app_voicemail's text in UTC (`Sat Jan  1 12:00:00 AM UTC 2022`).
pub(crate) fn voicemail_date(v: &Value) -> Option<u64> {
    if let Some(n) = count(v) {
        return Some(n as u64);
    }
    let s = v.as_str()?;
    let f: Vec<&str> = s.split_whitespace().collect();
    let [_, month, day, time, half, zone, year] = f[..] else {
        return None;
    };
    if !["UTC", "GMT"].contains(&zone) {
        return None;
    }
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = MONTHS.iter().position(|m| *m == month)? as i64 + 1;
    let day: i64 = day.parse().ok()?;
    let year: i64 = year.parse().ok()?;
    let mut hms = time.split(':').map(|p| p.parse::<i64>());
    let (Some(Ok(h)), Some(Ok(m)), Some(Ok(sec))) = (hms.next(), hms.next(), hms.next()) else {
        return None;
    };
    let h = match (half, h) {
        ("AM", 12) => 0,
        ("AM", h) => h,
        ("PM", 12) => 12,
        ("PM", h) => h + 12,
        _ => return None,
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + m * 60 + sec).ok()
}

fn read_presence(r: &Value) -> Option<PbxResult> {
    let p = r.get("presence")?;
    Some(PbxResult::Presence(PbxPresence {
        option_id: count(&p["id"]).unwrap_or(0),
        state: presence_state(&p["presence"]),
        sub_state: text(&p["sub_presence"]),
        message: text(&p["message"]),
    }))
}

fn read_presence_options(r: &Value) -> Option<PbxResult> {
    let options = r.get("presence_options")?;
    Some(PbxResult::PresenceOptions(
        items(&options["presence_option"])
            .into_iter()
            .map(|o| PbxPresenceOption {
                id: count(&o["id"]).unwrap_or(0),
                state: presence_state(&o["presence"]),
                sub_state: text(&o["sub_presence"]),
                by_user: o["owner"].as_str() == Some("user"),
                active: truthy(&o["active"]),
            })
            .collect(),
    ))
}

fn read_parking_lots(r: &Value) -> Option<PbxResult> {
    let lots = r.get("parking_lots")?;
    Some(PbxResult::ParkingLots(
        items(&lots["parking_lot"])
            .into_iter()
            .map(|lot| ParkingLot {
                name: text(&lot["account_id"]),
                first_space: text(&lot["parking_lot_start"]),
                last_space: text(&lot["parking_lot_stop"]),
                timeout: count(&lot["timeout"]).unwrap_or(0),
                calls: items(&lot["parked_calls"]["parked_call"])
                    .into_iter()
                    .map(|c| ParkedCall {
                        space: text(&c["lotnum"]),
                        caller_name: text(&c["from_caller_id_name"]),
                        caller_number: text(&c["from_caller_id_number"]),
                        duration: count(&c["duration"]).unwrap_or(0),
                    })
                    .collect(),
            })
            .collect(),
    ))
}

fn read_queues(r: &Value) -> Option<PbxResult> {
    let queues = r.get("callQueues")?;
    Some(PbxResult::Queues(
        items(&queues["callQueue"])
            .into_iter()
            .map(|q| CallQueue {
                id: text(&q["account_id"]),
                name: text(&q["name"]),
                membership: membership(&q["loggedin_status"]),
                paused: !q["paused_time"].is_null(),
                members: count(&q["total_members"]),
                members_logged_in: count(&q["members_logged_in"]),
                callers_waiting: count(&q["callers_waiting"]),
            })
            .collect(),
    ))
}

fn read_queue_status(r: &Value) -> Option<PbxResult> {
    let q = r.get("callQueue")?;
    let o = &q["overview"];
    let n = |k: &str| count(&o[k]).unwrap_or(0);
    let detail = &q["detailed_view"];
    Some(PbxResult::QueueStatus(QueueStatus {
        id: text(&q["account_id"]),
        name: text(&q["name"]),
        members: n("total_members"),
        members_logged_in: n("logged_in_count"),
        members_on_call: n("members_on_call"),
        callers_waiting: n("callers_waiting"),
        calls: n("total_calls"),
        completed_calls: n("completed_calls_today"),
        abandoned_calls: n("abandoned_calls"),
        member_list: items(&detail["queue_members"]["queue_member"])
            .into_iter()
            .map(|m| QueueMember {
                name: text(&m["fullname"]),
                membership: membership(&m["loggedin_status"]),
                on_call: count(&m["call_duration"]),
            })
            .collect(),
        caller_list: items(&detail["waiting_callers"]["waiting_caller"])
            .into_iter()
            .map(|c| QueueCaller {
                name: text(&c["talking_to_name"]),
                number: text(&c["talking_to_number"]),
                position: count(&c["position"]).unwrap_or(0),
                waiting: count(&c["waiting_duration"]).unwrap_or(0),
            })
            .collect(),
    }))
}

fn read_voicemail(r: &Value) -> Option<PbxResult> {
    let m = r.get("messages")?;
    Some(PbxResult::Voicemail(VoicemailPage {
        folder: text(&m["folder"]),
        page: count(&m["page_number"]).unwrap_or(1),
        pages: count(&m["total_pages"]).unwrap_or(1),
        total: count(&m["total_items"]).unwrap_or(0),
        messages: items(&m["message"])
            .into_iter()
            .map(|m| VoicemailMessage {
                id: text(&m["id"]),
                caller_name: text(&m["caller_id_name"]),
                caller_number: text(&m["caller_id_number"]),
                date: voicemail_date(&m["date_UTC"])
                    .or_else(|| voicemail_date(&m["date"]))
                    .unwrap_or(0),
                duration: count(&m["duration"]).unwrap_or(0),
                read: truthy(&m["read"]),
            })
            .collect(),
    }))
}

fn read_voicemail_folders(r: &Value) -> Option<PbxResult> {
    let f = r.get("folders")?;
    Some(PbxResult::VoicemailFolders(
        items(&f["folder"])
            .into_iter()
            .map(|f| VoicemailFolder { name: text(&f["name"]), count: count(&f["count"]).unwrap_or(0) })
            .collect(),
    ))
}

pub(crate) fn presence_info() -> Call {
    Call::new("switchvox.users.presence.getInfo", read_presence)
}

pub(crate) fn set_presence(option_id: u32, message: String) -> Call {
    Call::new("switchvox.users.presence.update", done)
        .with("presence_option_id", option_id.to_string())
        .with("message", message)
}

pub(crate) fn presence_options() -> Call {
    Call::new("switchvox.users.presence.options.getList", read_presence_options)
}

pub(crate) fn parking_lots() -> Call {
    Call::new("switchvox.users.parkingLots.getList", read_parking_lots)
}

/// `line` is the call's account (its `id`), `call_id` the call's SIP
/// Call-ID.
pub(crate) fn recording(start: bool, line: String, call_id: String) -> Call {
    let method = if start {
        "switchvox.users.currentCalls.startRecording"
    } else {
        "switchvox.users.currentCalls.stopRecording"
    };
    Call::new(method, done).with("requested_account_id", line).with("call_id", call_id)
}

pub(crate) fn queues() -> Call {
    Call::new("switchvox.users.callQueues.getList", read_queues)
}

pub(crate) fn queue_status(queue: String) -> Call {
    Call::new("switchvox.users.callQueues.getTodaysStatus", read_queue_status)
        .with("call_queue_account_id", queue)
}

pub(crate) fn queue_login(queue: Option<String>) -> Call {
    Call::new("switchvox.users.callQueues.login", done).with("call_queue_account_id", queue_id(queue))
}

pub(crate) fn queue_logout(queue: Option<String>) -> Call {
    Call::new("switchvox.users.callQueues.logout", done).with("call_queue_account_id", queue_id(queue))
}

pub(crate) fn queue_pause(queue: Option<String>, reason: String) -> Call {
    Call::new("switchvox.users.callQueues.pause", done)
        .with("call_queue_account_id", queue_id(queue))
        .with("reason", reason)
}

pub(crate) fn queue_resume(queue: Option<String>) -> Call {
    Call::new("switchvox.users.callQueues.resume", done).with("call_queue_account_id", queue_id(queue))
}

pub(crate) fn add_contact(phonebook: String, extension: String, name: String) -> Call {
    Call::new("switchvox.users.phonebooks.entries.add", done)
        .with("phonebook_id", phonebook)
        .with("extension", extension)
        .with("name", name)
}

pub(crate) fn voicemail_messages(
    folder: String,
    sort: VoicemailSort,
    descending: bool,
    per_page: u32,
    page: u32,
) -> Call {
    let field = match sort {
        VoicemailSort::Date => "date",
        VoicemailSort::Id => "id",
    };
    Call::new("switchvox.users.voicemail.getList", read_voicemail)
        .with("folder", folder)
        .with("sort_field", field)
        .with("sort_order", if descending { "DESC" } else { "ASC" })
        .with("items_per_page", per_page.max(1))
        .with("page_number", page.max(1).to_string())
}

pub(crate) fn voicemail_folders() -> Call {
    Call::new("switchvox.users.voicemail.getFolderList", read_voicemail_folders)
}

pub(crate) fn voicemail_move(source: String, target: String, messages: Vec<String>) -> Call {
    Call::new("switchvox.users.voicemail.move", done)
        .with("folder_from", source)
        .with("folder_to", target)
        .with("message_ids", ids(messages))
}

pub(crate) fn voicemail_remove(folder: String, messages: Vec<String>) -> Call {
    Call::new("switchvox.users.voicemail.remove", done)
        .with("folder", folder)
        .with("message_ids", ids(messages))
}

/// `account_from` is the app's account, which the host adds.
pub(crate) fn voicemail_forward(account: String, folder: String, messages: Vec<String>) -> Call {
    Call::new("switchvox.users.voicemail.forward", done)
        .with("account_to", account)
        .with("folder", folder)
        .with("message_ids", ids(messages))
}

pub(crate) fn voicemail_mark(read: bool, folder: String, messages: Vec<String>) -> Call {
    let method = if read {
        "switchvox.users.voicemail.markRead"
    } else {
        "switchvox.users.voicemail.markUnread"
    };
    Call::new(method, done).with("folder", folder).with("message_ids", ids(messages))
}

pub(crate) fn voicemail_play(folder: String, message: String) -> Call {
    Call::new("switchvox.users.voicemail.play", done)
        .with("folder", folder)
        .with("message_id", message)
}

/// The error of an answer's `response.errors`: the first error's code
/// and message.
pub(crate) fn server_error(errors: &Value) -> PbxServerError {
    let first = items(&errors["error"]).into_iter().next().unwrap_or(errors);
    let message = text(&first["message"]);
    PbxServerError {
        code: count(&first["code"]).unwrap_or(0),
        message: if message.is_empty() { "The server reported an error.".to_string() } else { message.trim().to_string() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn voicemail_dates() {
        assert_eq!(voicemail_date(&json!("1794607500")), Some(1_794_607_500));
        assert_eq!(voicemail_date(&json!(12)), Some(12));
        assert_eq!(voicemail_date(&json!("Sat Jan  1 12:00:00 AM UTC 2022")), Some(1_640_995_200));
        assert_eq!(voicemail_date(&json!("Fri Nov 13 10:05:00 PM UTC 2026")), Some(1_794_607_500));
        // A zone the phone cannot place.
        assert_eq!(voicemail_date(&json!("Fri Nov 13 03:05:00 PM MST 2026")), None);
    }

    #[test]
    fn answers_are_read_leniently() {
        let r = json!({"messages": {"folder": "INBOX", "page_number": "1", "total_pages": "1",
            "total_items": "1", "message": {"id": "INBOX-1", "caller_id_name": "Bob",
            "caller_id_number": 102, "date": "Sat Jan  1 12:00:00 AM UTC 2022",
            "date_UTC": "Sat Jan  1 12:00:00 AM UTC 2022", "duration": "7", "read": "No"}}});
        let Some(PbxResult::Voicemail(page)) = read_voicemail(&r) else { panic!() };
        assert_eq!(page.messages.len(), 1);
        let m = &page.messages[0];
        assert_eq!((m.caller_number.as_str(), m.date, m.duration, m.read), ("102", 1_640_995_200, 7, false));
        let r = json!({"callQueues": {"callQueue": [
            {"account_id": "sales", "name": "sales", "loggedin_status": "logged_in", "paused_time": "1", "total_members": 3},
            {"account_id": "support", "name": "support", "loggedin_status": "manager"}]}});
        let Some(PbxResult::Queues(q)) = read_queues(&r) else { panic!() };
        assert_eq!((q[0].membership, q[0].paused, q[0].members), (QueueMembership::LoggedIn, true, Some(3)));
        assert_eq!((q[1].membership, q[1].paused, q[1].members), (QueueMembership::Manager, false, None));
        // Not the method's answer.
        assert!(read_queues(&json!({"success": {}})).is_none());
        let e = server_error(&json!({"error": {"code": 27168, "message": "Your account is not permitted to run this action: mailbox "}}));
        assert_eq!((e.code, e.message.as_str()), (27168, "Your account is not permitted to run this action: mailbox"));
    }
}
