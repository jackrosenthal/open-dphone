//! Files the firmware keeps of its own state, in protobuf text format: the
//! alarm clock's alarms, the call log, and do-not-disturb and call
//! forwarding (`proto/opendeskphone/settings/v1/alarms.proto` and
//! `calls.proto`).  Stock never reads them, and stock's factory reset
//! deletes them.

use buffa::text::{TextFormat, decode_from_str, encode_to_string_pretty};

pub use crate::settings::pb::{
    Alarm, Alarms, CallLog, CallLogEntry, CallLogKind, Features, Forward, Party, PendingAlarm,
};
pub use buffa::{EnumValue, MessageField};

/// The alarm clock's alarms ([`Alarms`]), relative to the nvdata root.
pub const ALARMS_FILE: &str = "opendeskphone/alarms.txtpb";
/// The call log ([`CallLog`]), relative to the nvdata root.
pub const CALL_LOG_FILE: &str = "opendeskphone/calllog.txtpb";
/// Do-not-disturb and call forwarding ([`Features`]), relative to the
/// nvdata root.
pub const FEATURES_FILE: &str = "opendeskphone/features.txtpb";

/// The text of a file: a comment naming the message type, then the fields
/// that are set.
pub fn to_text<M: TextFormat>(type_name: &str, message: &M) -> String {
    format!("# {type_name}, protobuf text format.\n{}", encode_to_string_pretty(message))
}

/// A file's text as a message.
pub fn from_text<M: TextFormat + Default>(text: &str) -> Result<M, String> {
    decode_from_str(text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let alarms = Alarms {
            alarms: vec![Alarm { id: 3, hour: 7, minute: 30, days: 0x3e, tone: "Osmium".into(), enabled: true, since: 9 }],
            last_id: 3,
            pending: MessageField::some(PendingAlarm { id: 3, until: 10, unattended: 1 }),
        };
        let text = to_text("opendeskphone.settings.v1.Alarms", &alarms);
        assert!(text.starts_with("# opendeskphone.settings.v1.Alarms, protobuf text format.\n"), "{text}");
        assert!(text.contains("tone: \"Osmium\""), "{text}");
        assert_eq!(from_text::<Alarms>(&text).unwrap(), alarms);

        let log = CallLog {
            entries: vec![CallLogEntry {
                id: 1,
                kind: EnumValue::from(CallLogKind::CALL_LOG_KIND_MISSED),
                party: MessageField::some(Party { name: "Ann".into(), number: "100".into(), uri: String::new() }),
                account: 0,
                start_unix: 5,
                duration_secs: 0,
            }],
            missed_unseen: 1,
        };
        let text = to_text("opendeskphone.settings.v1.CallLog", &log);
        assert!(text.contains("CALL_LOG_KIND_MISSED"), "{text}");
        assert_eq!(from_text::<CallLog>(&text).unwrap(), log);
        assert!(from_text::<CallLog>("entries {").is_err());
    }
}
