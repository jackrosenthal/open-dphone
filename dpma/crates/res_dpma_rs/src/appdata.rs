//! App data for Open DPhone phones (`AppDataRequest`): the store of keys
//! and values that the phones' apps share through the server.  Not a
//! part of the official DPMA.
//!
//! `dpma_proto::appdata` defines the protocol and
//! `dpma_proto::appdata::store` what each operation does; this file
//! gives it AstDB (family [`DB_FAMILY`], so the data outlasts Asterisk),
//! the user and the tenant of the asking phone from the phone
//! configuration, and the event to the phones that watch an app.
//!
//! The tenant is the phone section's `dphone_app_data_tenant`.  Phones
//! with the same key share a store, phones without one share another,
//! and no request can name a tenant.  A phone whose option is set to
//! something that cannot be a key has no store.

use crate::users;
use dpma_ast::{db, dpma_log};
use dpma_proto::appdata::store::{self, Backend, Who};
use dpma_proto::appdata::{ErrorCode, Op, Outcome, Response, User, EVENT_CHANGE};
use std::sync::Mutex;

/// AstDB family of the store (`store` documents the keys below it).
pub const DB_FAMILY: &str = "DPhoneAppData";

/// One operation runs at a time: a write reads the entry it replaces.
static STORE: Mutex<()> = Mutex::new(());

/// The store in AstDB.
struct AstDb;

impl Backend for AstDb {
    fn get(&self, path: &str) -> Option<String> {
        String::from_utf8(db::get(DB_FAMILY, path)?).ok()
    }

    fn put(&mut self, path: &str, value: &str) -> bool {
        db::put(DB_FAMILY, path, value)
    }

    fn del(&mut self, path: &str) {
        db::del(DB_FAMILY, path);
    }

    fn tree(&self, dir: &str) -> Vec<(String, String)> {
        let prefix = format!("/{DB_FAMILY}/");
        db::tree(&format!("{DB_FAMILY}/{dir}"))
            .into_iter()
            .filter_map(|(key, value)| {
                let key = String::from_utf8(key).ok()?;
                let path = key.strip_prefix(&prefix)?.to_string();
                Some((path, String::from_utf8(value).ok()?))
            })
            .collect()
    }
}

/// What the configuration says of a phone: its tenant key and its user.
fn identity(phone: &[u8]) -> Option<(Vec<u8>, User)> {
    // The primary line is read under its lock (see "Line locks" in
    // users::objects), taken before the registry lock.
    let primary = users::build_config::primary_line_name(phone)?;
    let _line = users::objects::lock_lines([primary.as_slice()]);
    let g = users::read();
    let reg = g.as_ref()?;
    let p = reg.phones.get(phone)?;
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    let extension = users::build_config::primary_line(reg, p)
        .map(|l| text(l.dialable_exten()))
        .unwrap_or_default();
    let name = if p.full_name.is_empty() {
        &p.name
    } else {
        &p.full_name
    };
    Some((
        p.dphone_app_data_tenant.clone(),
        User {
            id: text(&p.name),
            name: text(name),
            extension,
        },
    ))
}

/// The tenant key of a phone, if the phone exists.
fn tenant_of(phone: &[u8]) -> Option<Vec<u8>> {
    users::read()
        .as_ref()?
        .phones
        .get(phone)
        .map(|p| p.dphone_app_data_tenant.clone())
}

/// The body of the AppDataResponse to the request `body` of phone
/// `phone` (the phone its configuration token belongs to).  Tells the
/// watching phones of the tenant when a shared key changed.
pub fn answer(phone: &[u8], body: &[u8]) -> Vec<u8> {
    let op = match Op::decode(body) {
        Ok(op) => op,
        Err(why) => return Response::error(ErrorCode::Invalid, why).encode(),
    };
    let Some((tenant, user)) = identity(phone) else {
        return Response::error(ErrorCode::Failed, "the phone is not configured").encode();
    };
    if !tenant.is_empty() && !store::valid_tenant(&tenant) {
        return Response::error(
            ErrorCode::Failed,
            "the phone's dphone_app_data_tenant is not a tenant key",
        )
        .encode();
    }
    let who = Who {
        tenant: &tenant,
        phone,
        user: &user,
    };
    let (outcome, tell) = {
        let _one = STORE.lock().unwrap_or_else(|e| e.into_inner());
        let mut b = AstDb;
        if let Op::Hello { watch } = &op {
            store::set_watch(&mut b, phone, watch);
        }
        let (outcome, change) = store::apply(&mut b, &who, &op, crate::session::now() as u64);
        let tell = change.map(|c| (store::watchers(&b, &c.app), c));
        (outcome, tell)
    };
    if let Some((watchers, change)) = tell {
        let event = change.encode();
        for other in watchers {
            if other == phone {
                continue;
            }
            // A phone hears only of its tenant's store.  A phone that is
            // gone from the configuration, or has no session to send to,
            // watches no longer.
            let reachable = match tenant_of(&other) {
                Some(t) if t != tenant => continue,
                Some(_) => {
                    users::runtime::send_event(&other, EVENT_CHANGE.as_bytes(), &event).is_ok()
                }
                None => false,
            };
            if !reachable {
                let _one = STORE.lock().unwrap_or_else(|e| e.into_inner());
                store::set_watch(&mut AstDb, &other, &[]);
            }
        }
    }
    if let Outcome::Error { code, message } = &outcome {
        dpma_log!(
            "App data request of phone '{}' refused ({}): {}",
            phone,
            code.name(),
            message
        );
        return Response {
            user: None,
            outcome,
        }
        .encode();
    }
    Response {
        user: Some(user),
        outcome,
    }
    .encode()
}
