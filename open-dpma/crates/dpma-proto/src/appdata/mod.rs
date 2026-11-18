//! App data: the store of keys and values that the server keeps for the
//! apps of Open Desk Phone phones (`AppDataRequest` / `AppDataResponse`).
//! The official DPMA has no such request; a phone that sends one to it
//! gets no answer.
//!
//! # The store
//!
//! Each app (by its name on the phone) has two scopes of keys:
//!
//! - `shared`: one set of keys for all phones;
//! - `user`: one set of keys for each user.
//!
//! The user is the phone section (`type=phone`) the request's
//! configuration token belongs to, so the server decides it, not the
//! phone.  The server also decides the tenant from that section, and keeps
//! a separate store for each tenant; nothing in the protocol names one.
//!
//! An entry has a key, a value (bytes), a version, the user who set it,
//! and the time it was set.  The version of an entry differs from every
//! version the key had before, so a writer that names the version it read
//! (`expect`) replaces exactly that entry or learns that someone else
//! wrote first.
//!
//! # The request
//!
//! An `AppDataRequest` needs a session and carries the configuration
//! token, as a ConfigRequest does; a token that does not match is
//! answered with message error -192.  Its body is one JSON object:
//!
//! ```text
//! {"op":"hello","watch":["APP",...]}
//! {"op":"get","app":"APP","scope":"shared"|"user","key":"KEY"}
//! {"op":"list","app":"APP","scope":S,"prefix":"P","after":"KEY"}
//! {"op":"set","app":"APP","scope":S,"key":"KEY","value":"BASE64","expect":V}
//! {"op":"remove","app":"APP","scope":S,"key":"KEY","expect":V}
//! ```
//!
//! `hello` names the apps whose shared keys the phone wants to hear
//! about; the list replaces the one the server had for the phone.
//! `list` answers the entries whose keys start with `prefix` and sort
//! after `after` (both may be empty or absent), in key order, at most
//! [`LIST_MAX`] of them and [`LIST_BYTES_MAX`] bytes of values.  `expect`
//! is optional: without it `set` and `remove` are unconditional; `0`
//! means the key must not be set; any other number is the version the
//! entry must have.
//!
//! # The response
//!
//! The body of the `AppDataResponse` is one JSON object.  Every one but
//! an error names the user:
//!
//! ```text
//! {"user":U,"result":"hello"}
//! {"user":U,"result":"entry","entry":E|null}              (get)
//! {"user":U,"result":"entries","entries":[E,...],"more":true|false}
//! {"user":U,"result":"stored","entry":E}                  (set)
//! {"user":U,"result":"removed"}
//! {"user":U,"result":"conflict","entry":E|null}           (expect did not hold)
//! {"result":"error","code":"invalid"|"limit"|"failed","message":"TEXT"}
//!
//! U = {"id":"PHONE","name":"NAME","extension":"EXTEN"}
//! E = {"key":"KEY","value":"BASE64","version":V,"modified":UNIX,
//!      "author":{"id":"PHONE","name":"NAME"}}
//! ```
//!
//! # The event
//!
//! When a shared key of an app is set or removed, the server sends the
//! event [`EVENT_CHANGE`] to the other phones of the same tenant that
//! watch the app, with the body `{"app":"APP","scope":"shared","key":"KEY"}`.

pub mod store;

use base64::Engine;
use serde_json::{json, Map, Value};

/// The request type.
pub const REQUEST: &str = "AppDataRequest";
/// The response type.
pub const RESPONSE: &str = "AppDataResponse";
/// The event sent to watching phones when a shared key changes.
pub const EVENT_CHANGE: &str = "opendeskphone.appdata.change";

/// The longest key, in bytes.
pub const KEY_MAX: usize = 64;
/// The largest value, in bytes.
pub const VALUE_MAX: usize = 4096;
/// The most keys an app's shared scope holds.
pub const SHARED_KEYS_MAX: usize = 1024;
/// The most keys an app holds for one user.
pub const USER_KEYS_MAX: usize = 64;
/// The most entries one `list` answers.
pub const LIST_MAX: usize = 64;
/// The most bytes of values one `list` answers (an entry that would pass
/// it is left for the next `list`, unless it is the first).
pub const LIST_BYTES_MAX: usize = 16 * 1024;
/// The most apps a phone watches.
pub const WATCH_MAX: usize = 32;

/// Whose keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// The app's keys that all phones share.
    Shared,
    /// The app's keys of the phone's user.
    User,
}

impl Scope {
    pub fn name(self) -> &'static str {
        match self {
            Scope::Shared => "shared",
            Scope::User => "user",
        }
    }

    pub fn from_name(s: &str) -> Option<Scope> {
        match s {
            "shared" => Some(Scope::Shared),
            "user" => Some(Scope::User),
            _ => None,
        }
    }

    /// The most keys of an app in this scope.
    pub fn keys_max(self) -> usize {
        match self {
            Scope::Shared => SHARED_KEYS_MAX,
            Scope::User => USER_KEYS_MAX,
        }
    }
}

/// The user of a phone, as the server knows them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct User {
    /// The name of the phone section; it identifies the user.
    pub id: String,
    /// The name to show: the section's `full_name`, else its name.
    pub name: String,
    /// The extension of the user's first line (may be empty).
    pub extension: String,
}

/// One key of the store.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub value: Vec<u8>,
    /// Differs from every version the key had before; never 0.
    pub version: u64,
    /// Seconds since the Unix epoch, by the server's clock.
    pub modified: u64,
    /// The `id` of the user who set it.
    pub author_id: String,
    /// The `name` of the user who set it, as it was then.
    pub author_name: String,
}

/// What a phone asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Hello {
        watch: Vec<String>,
    },
    Get {
        app: String,
        scope: Scope,
        key: String,
    },
    List {
        app: String,
        scope: Scope,
        prefix: String,
        after: String,
    },
    Set {
        app: String,
        scope: Scope,
        key: String,
        value: Vec<u8>,
        expect: Option<u64>,
    },
    Remove {
        app: String,
        scope: Scope,
        key: String,
        expect: Option<u64>,
    },
}

/// Why the server refused a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    /// The request is not one the server understands (a bad app name,
    /// key, value, or document).
    Invalid,
    /// The scope holds as many keys as it may, or the value is too large.
    Limit,
    /// The server could not read or write its store.
    Failed,
}

impl ErrorCode {
    pub fn name(self) -> &'static str {
        match self {
            ErrorCode::Invalid => "invalid",
            ErrorCode::Limit => "limit",
            ErrorCode::Failed => "failed",
        }
    }

    fn from_name(s: &str) -> ErrorCode {
        match s {
            "invalid" => ErrorCode::Invalid,
            "limit" => ErrorCode::Limit,
            _ => ErrorCode::Failed,
        }
    }
}

/// What the server answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Hello,
    Entry(Option<Entry>),
    Entries { entries: Vec<Entry>, more: bool },
    Stored(Entry),
    Removed,
    Conflict(Option<Entry>),
    Error { code: ErrorCode, message: String },
}

/// An `AppDataResponse`: the outcome, and the user (absent with an error).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub user: Option<User>,
    pub outcome: Outcome,
}

impl Response {
    pub fn error(code: ErrorCode, message: impl Into<String>) -> Response {
        Response {
            user: None,
            outcome: Outcome::Error {
                code,
                message: message.into(),
            },
        }
    }
}

/// A change the server tells watching phones about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub app: String,
    pub scope: Scope,
    pub key: String,
}

/// Whether `name` can be the name of an app: 1 to 64 characters, the
/// first a letter or digit, the rest letters, digits, `_`, `.`, or `,`
/// (the rule of the phone's app manifest).
pub fn valid_app(name: &str) -> bool {
    let mut chars = name.chars();
    (1..=64).contains(&name.len())
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ','))
}

fn key_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-' | b'/' | b':')
}

/// Whether `key` can be a key: 1 to [`KEY_MAX`] characters, each a
/// letter, a digit, or one of `.`, `_`, `-`, `/`, and `:`.
pub fn valid_key(key: &str) -> bool {
    (1..=KEY_MAX).contains(&key.len()) && key.bytes().all(key_char)
}

/// Whether `prefix` can start a key (it may be empty).
pub fn valid_prefix(prefix: &str) -> bool {
    prefix.len() <= KEY_MAX && prefix.bytes().all(key_char)
}

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn un_b64(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(text).ok()
}

fn text(v: &Value, name: &str) -> Option<String> {
    v.get(name)?.as_str().map(str::to_string)
}

fn text_or_empty(v: &Value, name: &str) -> Result<String, String> {
    match v.get(name) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(format!("`{name}` is not a string")),
    }
}

impl Op {
    /// The body of the request.
    pub fn encode(&self) -> Vec<u8> {
        let mut m = Map::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_string(), v);
        };
        match self {
            Op::Hello { watch } => {
                put("op", "hello".into());
                put("watch", json!(watch));
            }
            Op::Get { app, scope, key } => {
                put("op", "get".into());
                put("app", app.as_str().into());
                put("scope", scope.name().into());
                put("key", key.as_str().into());
            }
            Op::List {
                app,
                scope,
                prefix,
                after,
            } => {
                put("op", "list".into());
                put("app", app.as_str().into());
                put("scope", scope.name().into());
                put("prefix", prefix.as_str().into());
                put("after", after.as_str().into());
            }
            Op::Set {
                app,
                scope,
                key,
                value,
                expect,
            } => {
                put("op", "set".into());
                put("app", app.as_str().into());
                put("scope", scope.name().into());
                put("key", key.as_str().into());
                put("value", b64(value).into());
                if let Some(v) = expect {
                    put("expect", json!(v));
                }
            }
            Op::Remove {
                app,
                scope,
                key,
                expect,
            } => {
                put("op", "remove".into());
                put("app", app.as_str().into());
                put("scope", scope.name().into());
                put("key", key.as_str().into());
                if let Some(v) = expect {
                    put("expect", json!(v));
                }
            }
        }
        crate::json::dump(&Value::Object(m)).into_bytes()
    }

    /// Reads the body of a request.  The error is a text for the
    /// `invalid` answer.  Names, keys, and sizes are checked here, so an
    /// `Op` that was decoded is within the limits of the protocol.
    pub fn decode(body: &[u8]) -> Result<Op, String> {
        let v: Value =
            serde_json::from_slice(body).map_err(|_| "the request is not JSON".to_string())?;
        if !v.is_object() {
            return Err("the request is not a JSON object".into());
        }
        let op = text(&v, "op").ok_or("the request names no `op`")?;
        if op == "hello" {
            let mut watch = Vec::new();
            match v.get("watch") {
                None | Some(Value::Null) => {}
                Some(Value::Array(items)) => {
                    for item in items {
                        let app = item.as_str().filter(|a| valid_app(a));
                        let app = app.ok_or("`watch` holds something that is not an app name")?;
                        if !watch.iter().any(|w| w == app) {
                            watch.push(app.to_string());
                        }
                    }
                }
                Some(_) => return Err("`watch` is not an array".into()),
            }
            if watch.len() > WATCH_MAX {
                return Err(format!("a phone watches at most {WATCH_MAX} apps"));
            }
            return Ok(Op::Hello { watch });
        }
        let app = text(&v, "app")
            .filter(|a| valid_app(a))
            .ok_or("`app` is not an app name")?;
        let scope = text(&v, "scope")
            .and_then(|s| Scope::from_name(&s))
            .ok_or("`scope` is neither `shared` nor `user`")?;
        let key = || {
            text(&v, "key")
                .filter(|k| valid_key(k))
                .ok_or_else(|| "`key` is not a key".to_string())
        };
        let expect = || match v.get("expect") {
            None | Some(Value::Null) => Ok(None),
            Some(e) => e
                .as_u64()
                .map(Some)
                .ok_or_else(|| "`expect` is not a version".to_string()),
        };
        match op.as_str() {
            "get" => Ok(Op::Get {
                app,
                scope,
                key: key()?,
            }),
            "list" => {
                let prefix = text_or_empty(&v, "prefix")?;
                let after = text_or_empty(&v, "after")?;
                if !valid_prefix(&prefix) || !valid_prefix(&after) {
                    return Err("`prefix` or `after` cannot be part of a key".into());
                }
                Ok(Op::List {
                    app,
                    scope,
                    prefix,
                    after,
                })
            }
            "set" => {
                let value = text(&v, "value")
                    .and_then(|t| un_b64(&t))
                    .ok_or("`value` is not base64")?;
                if value.len() > VALUE_MAX {
                    return Err(format!("a value is at most {VALUE_MAX} bytes"));
                }
                Ok(Op::Set {
                    app,
                    scope,
                    key: key()?,
                    value,
                    expect: expect()?,
                })
            }
            "remove" => Ok(Op::Remove {
                app,
                scope,
                key: key()?,
                expect: expect()?,
            }),
            other => Err(format!("unknown `op` {other:?}")),
        }
    }
}

impl Entry {
    fn to_json(&self) -> Value {
        json!({
            "key": self.key,
            "value": b64(&self.value),
            "version": self.version,
            "modified": self.modified,
            "author": {"id": self.author_id, "name": self.author_name},
        })
    }

    fn from_json(v: &Value) -> Option<Entry> {
        let author = v.get("author");
        let author_text = |name: &str| author.and_then(|a| text(a, name)).unwrap_or_default();
        Some(Entry {
            key: text(v, "key")?,
            value: un_b64(v.get("value")?.as_str()?)?,
            version: v.get("version")?.as_u64()?,
            modified: v.get("modified").and_then(Value::as_u64).unwrap_or(0),
            author_id: author_text("id"),
            author_name: author_text("name"),
        })
    }
}

fn optional_entry(v: &Value) -> Result<Option<Entry>, String> {
    match v.get("entry") {
        None | Some(Value::Null) => Ok(None),
        Some(e) => Entry::from_json(e)
            .map(Some)
            .ok_or_else(|| "the response's entry is malformed".to_string()),
    }
}

impl Response {
    /// The body of the response.
    pub fn encode(&self) -> Vec<u8> {
        let mut m = Map::new();
        if let Some(u) = &self.user {
            m.insert(
                "user".into(),
                json!({"id": u.id, "name": u.name, "extension": u.extension}),
            );
        }
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_string(), v);
        };
        let entry = |e: &Option<Entry>| e.as_ref().map_or(Value::Null, Entry::to_json);
        match &self.outcome {
            Outcome::Hello => put("result", "hello".into()),
            Outcome::Entry(e) => {
                put("result", "entry".into());
                put("entry", entry(e));
            }
            Outcome::Entries { entries, more } => {
                put("result", "entries".into());
                put(
                    "entries",
                    Value::Array(entries.iter().map(Entry::to_json).collect()),
                );
                put("more", (*more).into());
            }
            Outcome::Stored(e) => {
                put("result", "stored".into());
                put("entry", e.to_json());
            }
            Outcome::Removed => put("result", "removed".into()),
            Outcome::Conflict(e) => {
                put("result", "conflict".into());
                put("entry", entry(e));
            }
            Outcome::Error { code, message } => {
                put("result", "error".into());
                put("code", code.name().into());
                put("message", message.as_str().into());
            }
        }
        crate::json::dump(&Value::Object(m)).into_bytes()
    }

    /// Reads the body of a response.
    pub fn decode(body: &[u8]) -> Result<Response, String> {
        let v: Value =
            serde_json::from_slice(body).map_err(|_| "the response is not JSON".to_string())?;
        let user = v.get("user").filter(|u| u.is_object()).map(|u| User {
            id: text(u, "id").unwrap_or_default(),
            name: text(u, "name").unwrap_or_default(),
            extension: text(u, "extension").unwrap_or_default(),
        });
        let result = text(&v, "result").ok_or("the response has no `result`")?;
        let outcome = match result.as_str() {
            "hello" => Outcome::Hello,
            "entry" => Outcome::Entry(optional_entry(&v)?),
            "entries" => {
                let items = v
                    .get("entries")
                    .and_then(Value::as_array)
                    .ok_or("the response has no `entries`")?;
                let entries = items
                    .iter()
                    .map(Entry::from_json)
                    .collect::<Option<Vec<_>>>()
                    .ok_or("an entry of the response is malformed")?;
                Outcome::Entries {
                    entries,
                    more: v.get("more").and_then(Value::as_bool).unwrap_or(false),
                }
            }
            "stored" => Outcome::Stored(optional_entry(&v)?.ok_or("the response has no `entry`")?),
            "removed" => Outcome::Removed,
            "conflict" => Outcome::Conflict(optional_entry(&v)?),
            "error" => Outcome::Error {
                code: ErrorCode::from_name(&text(&v, "code").unwrap_or_default()),
                message: text(&v, "message").unwrap_or_default(),
            },
            other => return Err(format!("unknown `result` {other:?}")),
        };
        Ok(Response { user, outcome })
    }
}

impl Change {
    /// The body of the event.
    pub fn encode(&self) -> Vec<u8> {
        crate::json::dump(&json!({
            "app": self.app,
            "scope": self.scope.name(),
            "key": self.key,
        }))
        .into_bytes()
    }

    /// Reads the body of the event.
    pub fn decode(body: &[u8]) -> Option<Change> {
        let v: Value = serde_json::from_slice(body).ok()?;
        Some(Change {
            app: text(&v, "app").filter(|a| valid_app(a))?,
            scope: Scope::from_name(&text(&v, "scope")?)?,
            key: text(&v, "key")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Entry {
        Entry {
            key: "best/7x7".into(),
            value: vec![0, 1, 2, 255],
            version: 12,
            modified: 1_794_607_500,
            author_id: "phone101".into(),
            author_name: "Alice \"A\" Example".into(),
        }
    }

    #[test]
    fn requests_round_trip() {
        let ops = [
            Op::Hello {
                watch: vec!["lightup".into(), "desksparrow".into()],
            },
            Op::Get {
                app: "lightup".into(),
                scope: Scope::Shared,
                key: "best/7x7".into(),
            },
            Op::List {
                app: "lightup".into(),
                scope: Scope::User,
                prefix: "best/".into(),
                after: String::new(),
            },
            Op::Set {
                app: "a".into(),
                scope: Scope::User,
                key: "k".into(),
                value: vec![0; VALUE_MAX],
                expect: Some(0),
            },
            Op::Set {
                app: "a".into(),
                scope: Scope::Shared,
                key: "k".into(),
                value: Vec::new(),
                expect: None,
            },
            Op::Remove {
                app: "a".into(),
                scope: Scope::Shared,
                key: "k".into(),
                expect: Some(7),
            },
        ];
        for op in ops {
            assert_eq!(Op::decode(&op.encode()), Ok(op));
        }
    }

    #[test]
    fn the_request_document() {
        let op = Op::Set {
            app: "lightup".into(),
            scope: Scope::Shared,
            key: "best".into(),
            value: b"hi".to_vec(),
            expect: Some(3),
        };
        assert_eq!(
            String::from_utf8(op.encode()).unwrap(),
            r#"{"op":"set","app":"lightup","scope":"shared","key":"best","value":"aGk=","expect":3}"#
        );
    }

    #[test]
    fn bad_requests_are_refused() {
        let bad = [
            "",
            "[]",
            r#"{"app":"a"}"#,
            r#"{"op":"fly","app":"a","scope":"shared"}"#,
            r#"{"op":"get","app":"../a","scope":"shared","key":"k"}"#,
            r#"{"op":"get","app":"a","scope":"tenant","key":"k"}"#,
            r#"{"op":"get","app":"a","scope":"shared","key":""}"#,
            r#"{"op":"get","app":"a","scope":"shared","key":"a b"}"#,
            r#"{"op":"get","app":"a","scope":"shared","key":"a%"}"#,
            r#"{"op":"set","app":"a","scope":"shared","key":"k","value":"***"}"#,
            r#"{"op":"set","app":"a","scope":"shared","key":"k","value":"","expect":-1}"#,
            r#"{"op":"list","app":"a","scope":"shared","prefix":"a b"}"#,
            r#"{"op":"hello","watch":["a","b/c"]}"#,
            r#"{"op":"hello","watch":"a"}"#,
        ];
        for doc in bad {
            assert!(Op::decode(doc.as_bytes()).is_err(), "{doc}");
        }
        let long = format!(
            r#"{{"op":"get","app":"a","scope":"shared","key":"{}"}}"#,
            "k".repeat(KEY_MAX + 1)
        );
        assert!(Op::decode(long.as_bytes()).is_err());
        let big = Op::Set {
            app: "a".into(),
            scope: Scope::Shared,
            key: "k".into(),
            value: vec![0; VALUE_MAX + 1],
            expect: None,
        };
        assert!(Op::decode(&big.encode()).is_err());
        // A list may leave out what it does not narrow by.
        assert_eq!(
            Op::decode(br#"{"op":"list","app":"a","scope":"user"}"#),
            Ok(Op::List {
                app: "a".into(),
                scope: Scope::User,
                prefix: String::new(),
                after: String::new(),
            })
        );
    }

    #[test]
    fn responses_round_trip() {
        let user = Some(User {
            id: "phone101".into(),
            name: "Alice".into(),
            extension: "101".into(),
        });
        let outcomes = [
            Outcome::Hello,
            Outcome::Entry(None),
            Outcome::Entry(Some(entry())),
            Outcome::Entries {
                entries: vec![entry(), entry()],
                more: true,
            },
            Outcome::Stored(entry()),
            Outcome::Removed,
            Outcome::Conflict(None),
            Outcome::Conflict(Some(entry())),
        ];
        for outcome in outcomes {
            let r = Response {
                user: user.clone(),
                outcome,
            };
            assert_eq!(Response::decode(&r.encode()), Ok(r));
        }
        let e = Response::error(ErrorCode::Limit, "too many keys");
        assert_eq!(Response::decode(&e.encode()), Ok(e));
        assert!(Response::decode(b"<error/>").is_err());
        assert!(Response::decode(br#"{"result":"stored"}"#).is_err());
    }

    #[test]
    fn changes_round_trip() {
        let c = Change {
            app: "lightup".into(),
            scope: Scope::Shared,
            key: "best/7x7".into(),
        };
        assert_eq!(
            String::from_utf8(c.encode()).unwrap(),
            r#"{"app":"lightup","scope":"shared","key":"best/7x7"}"#
        );
        assert_eq!(Change::decode(&c.encode()), Some(c));
        assert_eq!(Change::decode(b"{}"), None);
    }

    #[test]
    fn names() {
        for a in ["lightup", "a", "Box_Pusher.2", "9lives"] {
            assert!(valid_app(a), "{a}");
        }
        for a in ["", "_a", "a/b", "a b", &"a".repeat(65)] {
            assert!(!valid_app(a), "{a}");
        }
        for k in ["k", "best/7x7-easy", "a.b_c:d", &"k".repeat(KEY_MAX)] {
            assert!(valid_key(k), "{k}");
        }
        for k in ["", "a b", "a%", "é", &"k".repeat(KEY_MAX + 1)] {
            assert!(!valid_key(k), "{k}");
        }
    }
}
