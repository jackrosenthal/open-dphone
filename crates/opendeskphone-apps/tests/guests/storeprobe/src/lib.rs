//! A test guest without a screen that uses the app's keys on the PBX by
//! itself and logs what happens, for tests that run whole phones against
//! a real PBX (`crates/opendeskphone/tests/asterisk.rs`).
//!
//! What it does is in its `local` key `role`, which the test writes
//! before the phone starts; without one it does nothing.
//!
//! - `writer`: when the PBX is in reach, sets the shared key `greeting`
//!   and the user's key `progress`.
//! - `reader`: when the PBX is in reach, and whenever another phone
//!   changes a shared key, reads both.

use opendeskphone_app_sdk::prelude::*;
use opendeskphone_app_sdk::{log_info, read_string};

/// What the writer leaves in the shared key.
const GREETING: &[u8] = b"hello from the writer";
/// What the writer leaves in its user's key.
const PROGRESS: &[u8] = b"level 3";

#[derive(Default)]
struct StoreProbe {
    role: String,
    /// The requests under way, each with the key it is for.
    asked: Vec<(u32, &'static str)>,
}

/// The id of a request to a PBX location, which is never answered at once.
fn pending(reply: StorageReply) -> u32 {
    match reply {
        StorageReply::Pending(id) => id,
        StorageReply::Done(outcome) => panic!("answered at once: {outcome:?}"),
    }
}

impl StoreProbe {
    fn act(&mut self) {
        let who = storage::user().map(|u| format!("{} ({})", u.name, u.id));
        log_info(&format!("store in reach; the user is {who:?}"));
        match self.role.as_str() {
            "writer" => {
                let id = pending(storage::set(StorageLocation::PbxShared, "greeting", GREETING, None));
                self.asked.push((id, "greeting"));
                let id = pending(storage::set(StorageLocation::PbxPrivate, "progress", PROGRESS, None));
                self.asked.push((id, "progress"));
            }
            "reader" => self.read(),
            _ => {}
        }
    }

    fn read(&mut self) {
        let id = pending(storage::get(StorageLocation::PbxShared, "greeting"));
        self.asked.push((id, "greeting"));
        let id = pending(storage::get(StorageLocation::PbxPrivate, "progress"));
        self.asked.push((id, "progress"));
    }
}

impl App for StoreProbe {
    fn start(&mut self) {
        self.role = read_string("role").unwrap_or_default().trim().to_string();
        if self.role.is_empty() {
            return;
        }
        log_info(&format!("role {}", self.role));
        storage::watch(true);
        if storage::online() {
            self.act();
        }
    }

    fn handle(&mut self, event: Event) {
        if self.role.is_empty() {
            return;
        }
        match event {
            Event::StorageOnline => self.act(),
            Event::StorageChanged(change) => {
                log_info(&format!("changed {:?} {}", change.location, change.key));
                if self.role == "reader" {
                    self.read();
                }
            }
            Event::Storage((id, outcome)) => {
                let Some(at) = self.asked.iter().position(|(i, _)| *i == id) else { return };
                let (_, key) = self.asked.remove(at);
                let text = |value: &[u8]| String::from_utf8_lossy(value).into_owned();
                let said = match outcome {
                    StorageOutcome::Stored(e) => {
                        format!("stored {key}={:?} by {}", text(&e.value), e.author_name)
                    }
                    StorageOutcome::Entry(Some(e)) => {
                        format!("read {key}={:?} by {}", text(&e.value), e.author_name)
                    }
                    StorageOutcome::Entry(None) => format!("read {key}: not set"),
                    other => format!("{key}: {other:?}"),
                };
                log_info(&said);
            }
            _ => {}
        }
    }
}

opendeskphone_app_sdk::export_app! {
    app: StoreProbe,
    name: "storeprobe",
    id: "test.storeprobe",
    display_name: "Store Probe",
    kind: AppKind::Background,
    hidden: true,
}
