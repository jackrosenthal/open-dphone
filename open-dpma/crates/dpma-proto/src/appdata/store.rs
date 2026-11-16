//! The server's side of app data, independent of Asterisk: what each
//! operation does to a store of paths and text values ([`Backend`], which
//! the module implements with AstDB).
//!
//! The paths, with `/` between their parts:
//!
//! ```text
//! seq                              the last version given out
//! watch/<phone>                    the apps a phone watches, by spaces
//! @<tenant>/<app>/s/<key>          a shared entry
//! @<tenant>/<app>/u/<user>/<key>   an entry of a user
//! ```
//!
//! `@` alone is the store of the phones that have no tenant key.  A
//! phone's or a user's name is written with [`escape`].  The value of an
//! entry is `{"v":VERSION,"t":UNIX,"a":"AUTHOR ID","n":"AUTHOR
//! NAME","d":"BASE64 VALUE"}`.
//!
//! Nothing here locks: the caller runs one operation at a time.

use super::{
    b64, un_b64, valid_app, valid_key, Change, Entry, ErrorCode, Op, Outcome, Scope, User,
    LIST_BYTES_MAX, LIST_MAX, VALUE_MAX,
};
use serde_json::{json, Value};

/// The longest path: the module's AstDB keys, family and all, stay under
/// Asterisk's 256 bytes.
pub const PATH_MAX: usize = 230;
/// The longest tenant key.
pub const TENANT_MAX: usize = 32;

/// A store of text values by path.
pub trait Backend {
    fn get(&self, path: &str) -> Option<String>;
    /// False when the value could not be written.
    fn put(&mut self, path: &str, value: &str) -> bool;
    fn del(&mut self, path: &str);
    /// Every (path, value) below `dir`.  It may hold more than that
    /// (AstDB matches without regard to case and treats `_` and `%` as
    /// wildcards); the store looks only at what is exactly below `dir`.
    fn tree(&self, dir: &str) -> Vec<(String, String)>;
}

/// A [`Backend`] in memory, for tests and simulators.
#[derive(Clone, Debug, Default)]
pub struct MemoryBackend(pub std::collections::BTreeMap<String, String>);

impl Backend for MemoryBackend {
    fn get(&self, path: &str) -> Option<String> {
        self.0.get(path).cloned()
    }
    fn put(&mut self, path: &str, value: &str) -> bool {
        self.0.insert(path.to_string(), value.to_string());
        true
    }
    fn del(&mut self, path: &str) {
        self.0.remove(path);
    }
    fn tree(&self, dir: &str) -> Vec<(String, String)> {
        // As AstDB does: without regard to case.
        let dir = format!("{}/", dir.to_ascii_lowercase());
        self.0
            .iter()
            .filter(|(k, _)| k.to_ascii_lowercase().starts_with(&dir))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

/// Whether `key` can be a tenant key: 1 to [`TENANT_MAX`] characters,
/// each a letter, a digit, or one of `.`, `_`, and `-`.
pub fn valid_tenant(key: &[u8]) -> bool {
    (1..=TENANT_MAX).contains(&key.len())
        && key
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

/// A name as one part of a path: letters, digits, `.`, and `-` as they
/// are, every other byte as `=` and two hex digits.
pub fn escape(name: &[u8]) -> String {
    let mut out = String::with_capacity(name.len());
    for &c in name {
        if c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-') {
            out.push(c as char);
        } else {
            out.push_str(&format!("={c:02X}"));
        }
    }
    out
}

/// The name [`escape`] wrote, or `None` for anything else.
pub fn unescape(part: &str) -> Option<Vec<u8>> {
    let b = part.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' {
            let hex = part.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(out)
}

/// Who asks: the server takes both from its configuration of the phone.
#[derive(Clone, Copy, Debug)]
pub struct Who<'a> {
    /// The phone's tenant key (empty: none).
    pub tenant: &'a [u8],
    /// The name of the phone section, as the configuration has it.
    pub phone: &'a [u8],
    pub user: &'a User,
}

fn scope_dir(who: &Who, app: &str, scope: Scope) -> String {
    let tenant = String::from_utf8_lossy(who.tenant);
    match scope {
        Scope::Shared => format!("@{tenant}/{app}/s"),
        Scope::User => format!("@{tenant}/{app}/u/{}", escape(who.phone)),
    }
}

fn record(e: &Entry) -> String {
    crate::json::dump(&json!({
        "v": e.version,
        "t": e.modified,
        "a": e.author_id,
        "n": e.author_name,
        "d": b64(&e.value),
    }))
}

fn parse_record(key: &str, text: &str) -> Option<Entry> {
    let v: Value = serde_json::from_str(text).ok()?;
    let s = |name: &str| {
        v.get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    Some(Entry {
        key: key.to_string(),
        value: un_b64(v.get("d")?.as_str()?)?,
        version: v.get("v")?.as_u64()?,
        modified: v.get("t").and_then(Value::as_u64).unwrap_or(0),
        author_id: s("a"),
        author_name: s("n"),
    })
}

/// The entries of a scope, in key order.
fn entries(b: &impl Backend, dir: &str) -> Vec<Entry> {
    let below = format!("{dir}/");
    let mut found: Vec<Entry> = b
        .tree(dir)
        .into_iter()
        .filter_map(|(path, value)| {
            let key = path.strip_prefix(&below)?;
            valid_key(key).then(|| parse_record(key, &value)).flatten()
        })
        .collect();
    found.sort_by(|a, b| a.key.cmp(&b.key));
    found
}

fn error(code: ErrorCode, message: &str) -> (Outcome, Option<Change>) {
    (
        Outcome::Error {
            code,
            message: message.to_string(),
        },
        None,
    )
}

/// Whether an entry (or its absence) is what the writer expects.
fn holds(expect: Option<u64>, current: Option<&Entry>) -> bool {
    match expect {
        None => true,
        Some(0) => current.is_none(),
        Some(v) => current.is_some_and(|e| e.version == v),
    }
}

/// Does `op` for `who` at time `now` (seconds since the Unix epoch).
/// Returns the answer, and the change to tell the watching phones of the
/// tenant about, if there is one.  `hello` changes only the watch list
/// ([`set_watch`]) and is answered here as it is.
pub fn apply(b: &mut impl Backend, who: &Who, op: &Op, now: u64) -> (Outcome, Option<Change>) {
    let (app, scope, key) = match op {
        Op::Hello { .. } => return (Outcome::Hello, None),
        Op::Get { app, scope, key }
        | Op::Set {
            app, scope, key, ..
        }
        | Op::Remove {
            app, scope, key, ..
        } => (app, *scope, Some(key)),
        Op::List { app, scope, .. } => (app, *scope, None),
    };
    if !valid_app(app) || key.is_some_and(|k| !valid_key(k)) {
        return error(ErrorCode::Invalid, "not an app name or not a key");
    }
    let dir = scope_dir(who, app, scope);
    let path = key.map(|k| format!("{dir}/{k}"));
    if path.as_ref().is_some_and(|p| p.len() > PATH_MAX) || dir.len() > PATH_MAX {
        return error(
            ErrorCode::Failed,
            "the names of the app, the user, and the key are too long together",
        );
    }
    let current = |b: &dyn Fn(&str) -> Option<String>| {
        let (path, key) = (path.as_ref()?, key?);
        parse_record(key, &b(path)?)
    };
    let change = |key: &str| {
        (scope == Scope::Shared).then(|| Change {
            app: app.clone(),
            scope,
            key: key.to_string(),
        })
    };
    match op {
        Op::Hello { .. } => unreachable!("answered above"),
        Op::Get { .. } => (Outcome::Entry(current(&|p| b.get(p))), None),
        Op::List { prefix, after, .. } => {
            let mut out = Vec::new();
            let mut bytes = 0;
            let mut more = false;
            for e in entries(b, &dir) {
                if !e.key.starts_with(prefix.as_str()) || e.key.as_str() <= after.as_str() {
                    continue;
                }
                if out.len() >= LIST_MAX
                    || (!out.is_empty() && bytes + e.value.len() > LIST_BYTES_MAX)
                {
                    more = true;
                    break;
                }
                bytes += e.value.len();
                out.push(e);
            }
            (Outcome::Entries { entries: out, more }, None)
        }
        Op::Set { value, expect, .. } => {
            let (path, key) = (path.as_ref().unwrap(), key.unwrap());
            if value.len() > VALUE_MAX {
                return error(ErrorCode::Limit, "the value is too large");
            }
            let old = current(&|p| b.get(p));
            if !holds(*expect, old.as_ref()) {
                return (Outcome::Conflict(old), None);
            }
            if old.is_none() && entries(b, &dir).len() >= scope.keys_max() {
                return error(ErrorCode::Limit, "the app has as many keys as it may");
            }
            let seq = b
                .get("seq")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            let version = seq.max(old.as_ref().map_or(0, |e| e.version)) + 1;
            let entry = Entry {
                key: key.clone(),
                value: value.clone(),
                version,
                modified: now,
                author_id: who.user.id.clone(),
                author_name: who.user.name.clone(),
            };
            if !b.put("seq", &version.to_string()) || !b.put(path, &record(&entry)) {
                return error(ErrorCode::Failed, "the server could not write its store");
            }
            (Outcome::Stored(entry), change(key))
        }
        Op::Remove { expect, .. } => {
            let (path, key) = (path.as_ref().unwrap(), key.unwrap());
            let old = current(&|p| b.get(p));
            if !holds(*expect, old.as_ref()) {
                return (Outcome::Conflict(old), None);
            }
            if old.is_none() {
                return (Outcome::Removed, None);
            }
            b.del(path);
            (Outcome::Removed, change(key))
        }
    }
}

fn watch_path(phone: &[u8]) -> String {
    format!("watch/{}", escape(phone))
}

/// Makes `apps` the apps that `phone` watches.
pub fn set_watch(b: &mut impl Backend, phone: &[u8], apps: &[String]) {
    let path = watch_path(phone);
    if path.len() > PATH_MAX {
        return;
    }
    if apps.is_empty() {
        b.del(&path);
    } else {
        b.put(&path, &apps.join(" "));
    }
}

/// The phones that watch `app`, by name.
pub fn watchers(b: &impl Backend, app: &str) -> Vec<Vec<u8>> {
    b.tree("watch")
        .into_iter()
        .filter(|(_, apps)| apps.split(' ').any(|a| a == app))
        .filter_map(|(path, _)| unescape(path.strip_prefix("watch/")?))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::{SHARED_KEYS_MAX, USER_KEYS_MAX};
    use super::*;

    fn user(id: &str) -> User {
        User {
            id: id.into(),
            name: format!("User {id}"),
            extension: "100".into(),
        }
    }

    struct Phone {
        tenant: &'static str,
        user: User,
    }

    impl Phone {
        fn new(tenant: &'static str, id: &str) -> Phone {
            Phone {
                tenant,
                user: user(id),
            }
        }
        fn who(&self) -> Who<'_> {
            Who {
                tenant: self.tenant.as_bytes(),
                phone: self.user.id.as_bytes(),
                user: &self.user,
            }
        }
        fn run(&self, b: &mut MemoryBackend, op: Op) -> (Outcome, Option<Change>) {
            apply(b, &self.who(), &op, 1000)
        }
        fn set(
            &self,
            b: &mut MemoryBackend,
            scope: Scope,
            key: &str,
            value: &[u8],
            expect: Option<u64>,
        ) -> Outcome {
            self.run(
                b,
                Op::Set {
                    app: "game".into(),
                    scope,
                    key: key.into(),
                    value: value.to_vec(),
                    expect,
                },
            )
            .0
        }
        fn get(&self, b: &mut MemoryBackend, scope: Scope, key: &str) -> Option<Entry> {
            match self
                .run(
                    b,
                    Op::Get {
                        app: "game".into(),
                        scope,
                        key: key.into(),
                    },
                )
                .0
            {
                Outcome::Entry(e) => e,
                other => panic!("{other:?}"),
            }
        }
        fn list(
            &self,
            b: &mut MemoryBackend,
            scope: Scope,
            prefix: &str,
            after: &str,
        ) -> (Vec<String>, bool) {
            match self
                .run(
                    b,
                    Op::List {
                        app: "game".into(),
                        scope,
                        prefix: prefix.into(),
                        after: after.into(),
                    },
                )
                .0
            {
                Outcome::Entries { entries, more } => {
                    (entries.into_iter().map(|e| e.key).collect(), more)
                }
                other => panic!("{other:?}"),
            }
        }
    }

    fn stored(o: Outcome) -> Entry {
        match o {
            Outcome::Stored(e) => e,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn shared_keys_are_shared_and_user_keys_are_not() {
        let mut b = MemoryBackend::default();
        let (alice, bob) = (Phone::new("", "alice"), Phone::new("", "bob"));
        let e = stored(alice.set(&mut b, Scope::Shared, "best", b"53", None));
        assert_eq!(
            (e.author_id.as_str(), e.author_name.as_str()),
            ("alice", "User alice")
        );
        assert_eq!(e.modified, 1000);
        assert_eq!(bob.get(&mut b, Scope::Shared, "best"), Some(e));
        stored(alice.set(&mut b, Scope::User, "level", b"7", None));
        assert_eq!(bob.get(&mut b, Scope::User, "level"), None);
        assert_eq!(alice.get(&mut b, Scope::User, "level").unwrap().value, b"7");
        // Another app has other keys.
        let other = bob.run(
            &mut b,
            Op::Get {
                app: "other".into(),
                scope: Scope::Shared,
                key: "best".into(),
            },
        );
        assert_eq!(other.0, Outcome::Entry(None));
    }

    #[test]
    fn tenants_do_not_see_each_other() {
        let mut b = MemoryBackend::default();
        let north = Phone::new("north", "alice");
        let north2 = Phone::new("north", "bob");
        let south = Phone::new("south", "carol");
        // Tenant keys that differ only in case are different tenants.
        let shouting = Phone::new("NORTH", "dave");
        let none = Phone::new("", "erin");
        stored(north.set(&mut b, Scope::Shared, "best", b"n", None));
        assert_eq!(
            north2.get(&mut b, Scope::Shared, "best").unwrap().value,
            b"n"
        );
        for other in [&south, &shouting, &none] {
            assert_eq!(other.get(&mut b, Scope::Shared, "best"), None);
            assert_eq!(other.list(&mut b, Scope::Shared, "", ""), (vec![], false));
        }
        stored(south.set(&mut b, Scope::Shared, "best", b"s", Some(0)));
        stored(none.set(&mut b, Scope::Shared, "best", b"0", Some(0)));
        assert_eq!(
            north.get(&mut b, Scope::Shared, "best").unwrap().value,
            b"n"
        );
        assert_eq!(none.get(&mut b, Scope::Shared, "best").unwrap().value, b"0");
        // A user's keys are the tenant's too.
        stored(north.set(&mut b, Scope::User, "level", b"3", None));
        let moved = Phone::new("south", "alice");
        assert_eq!(moved.get(&mut b, Scope::User, "level"), None);
    }

    #[test]
    fn a_writer_that_expects_a_version_does_not_overwrite_another() {
        let mut b = MemoryBackend::default();
        let (alice, bob) = (Phone::new("", "alice"), Phone::new("", "bob"));
        // Both read "nothing", both write: the second learns of the first.
        let first = stored(alice.set(&mut b, Scope::Shared, "top", b"alice", Some(0)));
        let second = bob.set(&mut b, Scope::Shared, "top", b"bob", Some(0));
        assert_eq!(second, Outcome::Conflict(Some(first.clone())));
        // Bob merges and writes over the version he was shown.
        let merged = stored(bob.set(
            &mut b,
            Scope::Shared,
            "top",
            b"alice,bob",
            Some(first.version),
        ));
        assert!(merged.version > first.version);
        // Alice still holds the old version.
        assert_eq!(
            alice.set(&mut b, Scope::Shared, "top", b"x", Some(first.version)),
            Outcome::Conflict(Some(merged.clone()))
        );
        // Without an expectation the write goes through.
        let forced = stored(alice.set(&mut b, Scope::Shared, "top", b"x", None));
        assert!(forced.version > merged.version);
        // A version that is expected of a key that is gone is a conflict.
        let gone = bob.run(
            &mut b,
            Op::Remove {
                app: "game".into(),
                scope: Scope::Shared,
                key: "nothing".into(),
                expect: Some(4),
            },
        );
        assert_eq!(gone.0, Outcome::Conflict(None));
    }

    #[test]
    fn a_key_set_again_after_removal_has_a_new_version() {
        let mut b = MemoryBackend::default();
        let alice = Phone::new("", "alice");
        let first = stored(alice.set(&mut b, Scope::Shared, "k", b"1", None));
        let removed = alice.run(
            &mut b,
            Op::Remove {
                app: "game".into(),
                scope: Scope::Shared,
                key: "k".into(),
                expect: Some(first.version),
            },
        );
        assert_eq!(removed.0, Outcome::Removed);
        assert!(removed.1.is_some());
        let again = stored(alice.set(&mut b, Scope::Shared, "k", b"2", Some(0)));
        assert!(again.version > first.version);
        assert_eq!(
            alice.set(&mut b, Scope::Shared, "k", b"3", Some(first.version)),
            Outcome::Conflict(Some(again))
        );
        // Removing what is not there succeeds and tells nobody.
        let nothing = alice.run(
            &mut b,
            Op::Remove {
                app: "game".into(),
                scope: Scope::Shared,
                key: "absent".into(),
                expect: None,
            },
        );
        assert_eq!(nothing, (Outcome::Removed, None));
    }

    #[test]
    fn changes_are_told_for_shared_keys_only() {
        let mut b = MemoryBackend::default();
        let alice = Phone::new("", "alice");
        let op = |scope| Op::Set {
            app: "game".into(),
            scope,
            key: "k".into(),
            value: vec![1],
            expect: None,
        };
        let (_, change) = alice.run(&mut b, op(Scope::Shared));
        assert_eq!(
            change,
            Some(Change {
                app: "game".into(),
                scope: Scope::Shared,
                key: "k".into()
            })
        );
        assert_eq!(alice.run(&mut b, op(Scope::User)).1, None);
        // A refused write changes nothing.
        let refused = alice.run(
            &mut b,
            Op::Set {
                app: "game".into(),
                scope: Scope::Shared,
                key: "k".into(),
                value: vec![2],
                expect: Some(0),
            },
        );
        assert_eq!(refused.1, None);
    }

    #[test]
    fn lists_are_in_key_order_and_in_pieces() {
        let mut b = MemoryBackend::default();
        let alice = Phone::new("", "alice");
        for i in 0..(LIST_MAX + 6) {
            stored(alice.set(&mut b, Scope::Shared, &format!("n/{i:03}"), b"v", None));
        }
        stored(alice.set(&mut b, Scope::Shared, "other", b"v", None));
        // Case matters, though AstDB's trees ignore it.
        stored(alice.set(&mut b, Scope::Shared, "N/000", b"v", None));
        let (first, more) = alice.list(&mut b, Scope::Shared, "n/", "");
        assert_eq!(first.len(), LIST_MAX);
        assert!(more);
        assert_eq!(first[0], "n/000");
        let (rest, more) = alice.list(&mut b, Scope::Shared, "n/", first.last().unwrap());
        assert_eq!(rest.len(), 6);
        assert!(!more);
        assert_eq!(rest.last().unwrap(), &format!("n/{:03}", LIST_MAX + 5));
        let (all, _) = alice.list(&mut b, Scope::Shared, "", "n/zzz");
        assert_eq!(all, ["other"]);
        // Large values end a piece early; a piece always has an entry.
        let mut b = MemoryBackend::default();
        for i in 0..6 {
            stored(alice.set(
                &mut b,
                Scope::Shared,
                &format!("big{i}"),
                &vec![0; VALUE_MAX],
                None,
            ));
        }
        let (piece, more) = alice.list(&mut b, Scope::Shared, "", "");
        assert_eq!(piece.len(), LIST_BYTES_MAX / VALUE_MAX);
        assert!(more);
    }

    #[test]
    fn a_scope_holds_only_so_many_keys() {
        let mut b = MemoryBackend::default();
        let alice = Phone::new("", "alice");
        for (scope, max) in [
            (Scope::User, USER_KEYS_MAX),
            (Scope::Shared, SHARED_KEYS_MAX),
        ] {
            for i in 0..max {
                stored(alice.set(&mut b, scope, &format!("k{i}"), b"", None));
            }
            assert!(matches!(
                alice.set(&mut b, scope, "one-more", b"", None),
                Outcome::Error {
                    code: ErrorCode::Limit,
                    ..
                }
            ));
            // A key that is there can still be written.
            stored(alice.set(&mut b, scope, "k0", b"again", None));
        }
        // Another user has room.
        stored(Phone::new("", "bob").set(&mut b, Scope::User, "k", b"", None));
    }

    #[test]
    fn names_that_are_too_long_together_are_refused() {
        let mut b = MemoryBackend::default();
        let long = Phone::new("", &"u".repeat(PATH_MAX));
        assert!(matches!(
            long.set(&mut b, Scope::User, "k", b"", None),
            Outcome::Error {
                code: ErrorCode::Failed,
                ..
            }
        ));
        // The shared scope does not name the user.
        stored(long.set(&mut b, Scope::Shared, "k", b"", None));
    }

    #[test]
    fn escaped_names() {
        for name in [&b"phone101"[..], b"Front Desk/2", b"a=b", b"\xff_x", b""] {
            let e = escape(name);
            assert!(e
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-=".contains(&c)));
            assert_eq!(unescape(&e).as_deref(), Some(name));
        }
        assert_eq!(escape(b"Front Desk/2"), "Front=20Desk=2F2");
        assert_eq!(unescape("a=2"), None);
        assert_eq!(unescape("a=zz"), None);
        assert!(valid_tenant(b"north-office_1.a"));
        for t in [&b""[..], b"a b", b"a/b", b"a@b", &[b'a'; TENANT_MAX + 1]] {
            assert!(!valid_tenant(t));
        }
    }

    #[test]
    fn watch_lists() {
        let mut b = MemoryBackend::default();
        set_watch(&mut b, b"phone a", &["game".into(), "other".into()]);
        set_watch(&mut b, b"phoneb", &["game".into()]);
        let mut w = watchers(&b, "game");
        w.sort();
        assert_eq!(w, [b"phone a".to_vec(), b"phoneb".to_vec()]);
        assert_eq!(watchers(&b, "other"), [b"phone a".to_vec()]);
        assert!(watchers(&b, "gam").is_empty());
        // A new list replaces the old one; an empty one removes it.
        set_watch(&mut b, b"phone a", &["other".into()]);
        assert_eq!(watchers(&b, "game"), [b"phoneb".to_vec()]);
        set_watch(&mut b, b"phoneb", &[]);
        assert!(watchers(&b, "game").is_empty());
        assert!(b.get("watch/phoneb").is_none());
    }
}
