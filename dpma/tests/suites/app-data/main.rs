//! The app-data suite: the app data of Open DPhone phones
//! (`AppDataRequest`) end to end with the phone simulator.
//!
//! ```text
//! cargo xtask test app-data
//! ```
//!
//! runs it in the test container after building the devtest module, as
//!
//! ```text
//! cargo test -p dpma-tests --test app-data -- --ignored [--no-prepare] [-v] [PART...]
//! ```
//!
//! The suite prepares the instance (`--no-prepare` skips this).
//!
//! The phones (`data/app-data/res_digium_phone_app_data.conf`) are of two
//! tenants, `north` and `south`, each set through a template; two phones
//! have no tenant key, and one has a key that cannot be one.  The parts
//! check what a phone reads and writes, that a tenant sees only its own
//! store, that writers who name the version they read never lose each
//! other's entries, who hears of a change, and that the store outlasts
//! Asterisk.

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{anyhow, Result};
use bstr::ByteSlice;
use dpma_proto::appdata::{
    Change, Entry, ErrorCode, Op, Outcome, Response, Scope, EVENT_CHANGE, LIST_MAX,
};
use dpma_tests::harness::{fixtures, setup, ConfFile, Ctx, Suite};
use dpma_tests::sim::{Options, Phone};

/// The test phones.
const PHONES_CONF: &str = include_str!("../../data/app-data/res_digium_phone_app_data.conf");
/// Every test phone, with its MAC.
const PHONES: [(&str, &str); 6] = [
    ("north1", "000fd3002101"),
    ("north2", "000fd3002102"),
    ("south1", "000fd3002103"),
    ("plain1", "000fd3002104"),
    ("plain2", "000fd3002105"),
    ("badkey", "000fd3002106"),
];
/// The app whose data the parts use.
const APP: &str = "game";

/// Whether the store was emptied (once per run, before the first part).
static READY: AtomicBool = AtomicBool::new(false);

/// A part of the suite.
type Part = fn(&mut Ctx) -> Result<()>;

/// Runs a part.  Before the first one, sets the authentication mode and
/// empties the store.
fn part(ctx: &mut Ctx, f: Part) -> Result<()> {
    if !READY.swap(true, Ordering::SeqCst) {
        ConfFile::new(&ctx.inst.conf("res_digium_phone.conf")).set_options(
            "general",
            &[
                ("config_auth", Some(&b"mac"[..])),
                ("userlist_auth", Some(&b"disabled"[..])),
            ],
        )?;
        ctx.ami()?.command("module reload res_dpma_rs.so")?;
        ctx.cli("database deltree DPhoneAppData");
    }
    f(ctx)
}

/// A simulated phone of the configuration, with a session and a token.
fn phone_with(opts: Options, name: &str) -> Result<Phone> {
    let mac = PHONES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, mac)| *mac)
        .ok_or_else(|| anyhow!("no test phone {name}"))?;
    let p = fixtures::phone(opts.mac(mac))?;
    let ex = p.get_token().send()?;
    if !ex.ok() {
        return Err(anyhow!("no token for {name}:\n{}", ex.describe(0)));
    }
    Ok(p)
}

fn phone(ctx: &Ctx, name: &str) -> Result<Phone> {
    phone_with(ctx.sim_options(), name)
}

/// Sends one operation and reads the answer.
fn ask(p: &Phone, op: &Op) -> Result<Response> {
    let ex = p.request("AppDataRequest").body(op.encode()).send()?;
    if !ex.ok() {
        return Err(anyhow!("{op:?} was not answered:\n{}", ex.describe(0)));
    }
    let reply = ex.reply().expect("an answered exchange has a reply");
    Response::decode(&reply.payload).map_err(|e| anyhow!("{e}: {:?}", reply.payload))
}

fn set_op(scope: Scope, key: &str, value: &[u8], expect: Option<u64>) -> Op {
    Op::Set {
        app: APP.into(),
        scope,
        key: key.into(),
        value: value.to_vec(),
        expect,
    }
}

fn set(p: &Phone, scope: Scope, key: &str, value: &[u8], expect: Option<u64>) -> Result<Outcome> {
    Ok(ask(p, &set_op(scope, key, value, expect))?.outcome)
}

fn get(p: &Phone, scope: Scope, key: &str) -> Result<Option<Entry>> {
    let op = Op::Get {
        app: APP.into(),
        scope,
        key: key.into(),
    };
    match ask(p, &op)?.outcome {
        Outcome::Entry(e) => Ok(e),
        other => Err(anyhow!("get answered {other:?}")),
    }
}

fn list(p: &Phone, scope: Scope, prefix: &str, after: &str) -> Result<(Vec<String>, bool)> {
    let op = Op::List {
        app: APP.into(),
        scope,
        prefix: prefix.into(),
        after: after.into(),
    };
    match ask(p, &op)?.outcome {
        Outcome::Entries { entries, more } => {
            Ok((entries.into_iter().map(|e| e.key).collect(), more))
        }
        other => Err(anyhow!("list answered {other:?}")),
    }
}

fn value_of(e: &Option<Entry>) -> Option<&[u8]> {
    e.as_ref().map(|e| e.value.as_slice())
}

/// What one phone writes and another reads, and what the answers say.
fn store(ctx: &mut Ctx) -> Result<()> {
    let (a, b) = (phone(ctx, "north1")?, phone(ctx, "north2")?);
    let r = ask(&a, &set_op(Scope::Shared, "store/best", b"53 seconds", None))?;
    let user = r.user.clone().unwrap_or_default();
    ctx.c.same(
        "the answer names the user",
        (user.id.as_str(), user.name.as_str(), user.extension.as_str()),
        ("north1", "North One", "6101"),
    );
    let Outcome::Stored(written) = r.outcome else {
        ctx.c.fail("set is answered with the entry", format!("{:?}", r.outcome));
        return Ok(());
    };
    ctx.c.same(
        "the entry names its author",
        (written.author_id.as_str(), written.author_name.as_str()),
        ("north1", "North One"),
    );
    ctx.c.check("the entry has a version", written.version > 0, "");
    ctx.c.check("the entry has a time", written.modified > 1_700_000_000, "");
    let r = ask(
        &b,
        &Op::Get {
            app: APP.into(),
            scope: Scope::Shared,
            key: "store/best".into(),
        },
    )?;
    ctx.c.same(
        "another phone reads the shared entry",
        r.outcome,
        Outcome::Entry(Some(written.clone())),
    );
    let user = r.user.unwrap_or_default();
    ctx.c.same(
        "a phone without a full name is named as its section",
        (user.id.as_str(), user.name.as_str(), user.extension.as_str()),
        ("north2", "north2", "6102"),
    );

    set(&a, Scope::User, "store/level", b"7", None)?;
    ctx.c.same(
        "a user's entry is the user's",
        value_of(&get(&a, Scope::User, "store/level")?),
        Some(&b"7"[..]),
    );
    ctx.c.same(
        "another user does not have it",
        get(&b, Scope::User, "store/level")?,
        None,
    );
    ctx.c.same(
        "nor is it a shared entry",
        get(&b, Scope::Shared, "store/level")?,
        None,
    );
    let other_app = ask(
        &b,
        &Op::Get {
            app: "other".into(),
            scope: Scope::Shared,
            key: "store/best".into(),
        },
    )?;
    ctx.c.same(
        "another app has other keys",
        other_app.outcome,
        Outcome::Entry(None),
    );

    // A value of every byte, and the largest one.
    let bytes: Vec<u8> = (0..=255).collect();
    set(&a, Scope::Shared, "store/bytes", &bytes, None)?;
    ctx.c.same(
        "a value is bytes",
        value_of(&get(&b, Scope::Shared, "store/bytes")?),
        Some(bytes.as_slice()),
    );
    let big = vec![0xa5; dpma_proto::appdata::VALUE_MAX];
    set(&a, Scope::Shared, "store/big", &big, None)?;
    ctx.c.same(
        "the largest value comes back whole",
        value_of(&get(&b, Scope::Shared, "store/big")?),
        Some(big.as_slice()),
    );

    // Lists: by prefix, in order, in pieces.
    for i in 0..(LIST_MAX + 3) {
        set(&a, Scope::Shared, &format!("list/{i:03}"), b"x", None)?;
    }
    let (first, more) = list(&b, Scope::Shared, "list/", "")?;
    ctx.c.same("a list is cut", (first.len(), more), (LIST_MAX, true));
    let (rest, more) = list(&b, Scope::Shared, "list/", first.last().map_or("", |k| k))?;
    ctx.c.same("and goes on after a key", (rest.len(), more), (3, false));
    ctx.c.same(
        "in key order",
        (first.first().cloned(), rest.last().cloned()),
        (
            Some("list/000".to_string()),
            Some(format!("list/{:03}", LIST_MAX + 2)),
        ),
    );

    // Removing.
    let removed = ask(
        &b,
        &Op::Remove {
            app: APP.into(),
            scope: Scope::Shared,
            key: "store/bytes".into(),
            expect: None,
        },
    )?;
    ctx.c.same("remove", removed.outcome, Outcome::Removed);
    ctx.c
        .same("a removed key is gone", get(&a, Scope::Shared, "store/bytes")?, None);

    // What the server refuses.
    let ex = a.request("AppDataRequest").body("{}").send()?;
    let refused = ex
        .reply()
        .and_then(|r| Response::decode(&r.payload).ok())
        .map(|r| r.outcome);
    ctx.c.check(
        "a request that is not one is refused",
        matches!(
            refused,
            Some(Outcome::Error {
                code: ErrorCode::Invalid,
                ..
            })
        ),
        format!("{refused:?}"),
    );
    let ex = a
        .request("AppDataRequest")
        .body(set_op(Scope::Shared, "store/best", b"x", None).encode())
        .config_token("north1/000fd3999999/1/2")
        .send()?;
    let error = ex
        .reply()
        .and_then(|r| r.error.as_ref())
        .map(|e| e.to_str_lossy().into_owned());
    ctx.c.same(
        "a token that does not match is answered with -192",
        error.as_deref(),
        Some("-192"),
    );
    ctx.c.same(
        "and nothing is written",
        value_of(&get(&b, Scope::Shared, "store/best")?),
        Some(&b"53 seconds"[..]),
    );
    let out = ctx.cli_text("digium_phones show phone north1");
    ctx.c.contains(
        "the phone's tenant is shown",
        &out,
        &["Open DPhone app data tenant: north"],
    );
    Ok(())
}

/// A phone reads and writes the store of its tenant, and no other.
fn tenants(ctx: &mut Ctx) -> Result<()> {
    let north1 = phone(ctx, "north1")?;
    let north2 = phone(ctx, "north2")?;
    let south = phone(ctx, "south1")?;
    let plain1 = phone(ctx, "plain1")?;
    let plain2 = phone(ctx, "plain2")?;
    let badkey = phone(ctx, "badkey")?;

    set(&north1, Scope::Shared, "tenants/k", b"north", None)?;
    ctx.c.same(
        "a phone of the same tenant reads it",
        value_of(&get(&north2, Scope::Shared, "tenants/k")?),
        Some(&b"north"[..]),
    );
    for (name, p) in [("another tenant", &south), ("a phone without a tenant", &plain1)] {
        ctx.c.same(
            format!("{name} does not read it"),
            get(p, Scope::Shared, "tenants/k")?,
            None,
        );
        ctx.c.same(
            format!("{name} does not list it"),
            list(p, Scope::Shared, "tenants/", "")?,
            (vec![], false),
        );
    }
    // Each store has the key for itself: none of these writes meets
    // another's entry.
    for (name, p, value) in [
        ("another tenant", &south, &b"south"[..]),
        ("a phone without a tenant", &plain1, &b"plain"[..]),
    ] {
        let outcome = set(p, Scope::Shared, "tenants/k", value, Some(0))?;
        ctx.c.check(
            format!("{name} sets the key as a new one"),
            matches!(outcome, Outcome::Stored(_)),
            format!("{outcome:?}"),
        );
    }
    ctx.c.same(
        "the tenant's entry is as it was",
        value_of(&get(&north2, Scope::Shared, "tenants/k")?),
        Some(&b"north"[..]),
    );
    ctx.c.same(
        "the other tenant has its own",
        value_of(&get(&south, Scope::Shared, "tenants/k")?),
        Some(&b"south"[..]),
    );
    ctx.c.same(
        "the phones without a tenant share theirs",
        value_of(&get(&plain2, Scope::Shared, "tenants/k")?),
        Some(&b"plain"[..]),
    );
    // Removing in one store leaves the others.
    ask(
        &south,
        &Op::Remove {
            app: APP.into(),
            scope: Scope::Shared,
            key: "tenants/k".into(),
            expect: None,
        },
    )?;
    ctx.c.same(
        "a tenant removes only its own entry",
        (
            value_of(&get(&north1, Scope::Shared, "tenants/k")?),
            value_of(&get(&plain1, Scope::Shared, "tenants/k")?),
        ),
        (Some(&b"north"[..]), Some(&b"plain"[..])),
    );

    // A request cannot name a tenant: whatever else it carries is ignored.
    let sneaky = br#"{"op":"get","app":"game","scope":"shared","key":"tenants/k","tenant":"north"}"#;
    let ex = south.request("AppDataRequest").body(sneaky).send()?;
    let got = ex
        .reply()
        .and_then(|r| Response::decode(&r.payload).ok())
        .map(|r| r.outcome);
    ctx.c.same(
        "a request that names a tenant gets its own tenant's store",
        got,
        Some(Outcome::Entry(None)),
    );

    // A key that cannot be one: no store at all, not the store of the
    // phones without a key.
    let outcome = ask(
        &badkey,
        &Op::Get {
            app: APP.into(),
            scope: Scope::Shared,
            key: "tenants/k".into(),
        },
    )?
    .outcome;
    ctx.c.check(
        "a phone whose tenant key cannot be one has no store",
        matches!(
            outcome,
            Outcome::Error {
                code: ErrorCode::Failed,
                ..
            }
        ),
        format!("{outcome:?}"),
    );

    // Where the entries are in AstDB.
    let db = ctx.cli_text("database show DPhoneAppData");
    ctx.c.contains(
        "each tenant has its part of the AstDB family",
        &db,
        &[
            "/DPhoneAppData/@north/game/s/tenants/k",
            "/DPhoneAppData/@/game/s/tenants/k",
        ],
    );
    ctx.c.lacks(
        "a removed entry is not in AstDB",
        &db,
        &["/DPhoneAppData/@south/game/s/tenants/k"],
    );
    Ok(())
}

/// Splits a value of comma-separated names.
fn names(value: &[u8]) -> Vec<String> {
    value
        .split_str(",")
        .filter(|n| !n.is_empty())
        .map(|n| n.to_str_lossy().into_owned())
        .collect()
}

/// Adds `name` to the list in `key`, as an app does: read, change, write
/// over the version that was read, and again when someone else wrote
/// first.  Returns how often someone else did.
fn add_name(p: &Phone, key: &str, name: &str) -> Result<usize> {
    let mut current = get(p, Scope::Shared, key)?;
    for conflicts in 0..1000 {
        let mut list = current.as_ref().map(|e| names(&e.value)).unwrap_or_default();
        list.push(name.to_string());
        let expect = current.as_ref().map_or(0, |e| e.version);
        match set(p, Scope::Shared, key, list.join(",").as_bytes(), Some(expect))? {
            Outcome::Stored(_) => return Ok(conflicts),
            Outcome::Conflict(now) => current = now,
            other => return Err(anyhow!("set answered {other:?}")),
        }
    }
    Err(anyhow!("the write never went through"))
}

/// Writers that name the version they read do not lose each other's
/// entries.
fn conflict(ctx: &mut Ctx) -> Result<()> {
    let (a, b) = (phone(ctx, "north1")?, phone(ctx, "north2")?);
    let first = set(&a, Scope::Shared, "conflict/k", b"a", Some(0))?;
    let Outcome::Stored(first) = first else {
        ctx.c.fail("a new key is set", format!("{first:?}"));
        return Ok(());
    };
    ctx.c.same(
        "a second writer of a new key is shown the first's entry",
        set(&b, Scope::Shared, "conflict/k", b"b", Some(0))?,
        Outcome::Conflict(Some(first.clone())),
    );
    let second = set(&b, Scope::Shared, "conflict/k", b"a,b", Some(first.version))?;
    let Outcome::Stored(second) = second else {
        ctx.c.fail("a write over the version read", format!("{second:?}"));
        return Ok(());
    };
    ctx.c.check(
        "a write gives the entry a newer version",
        second.version > first.version,
        format!("{} then {}", first.version, second.version),
    );
    ctx.c.same(
        "a write over an old version is refused",
        set(&a, Scope::Shared, "conflict/k", b"x", Some(first.version))?,
        Outcome::Conflict(Some(second.clone())),
    );
    let removal = ask(
        &a,
        &Op::Remove {
            app: APP.into(),
            scope: Scope::Shared,
            key: "conflict/k".into(),
            expect: Some(first.version),
        },
    )?;
    ctx.c.same(
        "and so is a removal",
        removal.outcome,
        Outcome::Conflict(Some(second)),
    );

    // Two phones add to one list at the same time, twenty names each.
    const EACH: usize = 20;
    let writers: Vec<_> = ["north1", "north2"]
        .into_iter()
        .map(|who| {
            let opts = ctx.sim_options();
            std::thread::spawn(move || -> Result<usize> {
                let p = phone_with(opts, who)?;
                let mut conflicts = 0;
                for i in 0..EACH {
                    conflicts += add_name(&p, "conflict/list", &format!("{who}-{i}"))?;
                }
                Ok(conflicts)
            })
        })
        .collect();
    let mut conflicts = 0;
    for w in writers {
        conflicts += w.join().map_err(|_| anyhow!("a writer panicked"))??;
    }
    // The writers were north1 and north2 again, with sessions of their
    // own, so the phones from above have none now.
    drop((a, b));
    let end = get(&phone(ctx, "north1")?, Scope::Shared, "conflict/list")?;
    let mut got = end.as_ref().map(|e| names(&e.value)).unwrap_or_default();
    got.sort();
    let mut want: Vec<String> = ["north1", "north2"]
        .iter()
        .flat_map(|who| (0..EACH).map(move |i| format!("{who}-{i}")))
        .collect();
    want.sort();
    ctx.c.same("every name of both writers is in the list", got, want);
    ctx.c
        .note(format!("the writers met each other {conflicts} times"));
    Ok(())
}

/// Tells the server which apps a phone watches.
fn hello(p: &Phone, apps: &[&str]) -> Result<()> {
    let op = Op::Hello {
        watch: apps.iter().map(|a| a.to_string()).collect(),
    };
    match ask(p, &op)?.outcome {
        Outcome::Hello => Ok(()),
        other => Err(anyhow!("hello answered {other:?}")),
    }
}

/// The next change event a phone gets within `secs`.
fn next_change(p: &Phone, secs: f64) -> Option<Change> {
    let r = p.next_event(Duration::from_secs_f64(secs), Some(EVENT_CHANGE))?;
    Change::decode(&r.payload)
}

/// Who hears of a change: the other phones of the tenant that watch the
/// app.
fn watch(ctx: &mut Ctx) -> Result<()> {
    let north1 = phone(ctx, "north1")?;
    let north2 = phone(ctx, "north2")?;
    let south = phone(ctx, "south1")?;
    let plain1 = phone(ctx, "plain1")?;
    let plain2 = phone(ctx, "plain2")?;
    for p in [&north1, &north2, &south, &plain1] {
        hello(p, &[APP])?;
    }
    hello(&plain2, &["other"])?;

    set(&north1, Scope::Shared, "watch/k", b"1", None)?;
    let change = Change {
        app: APP.into(),
        scope: Scope::Shared,
        key: "watch/k".into(),
    };
    ctx.c.same(
        "a watching phone of the tenant hears of a change",
        next_change(&north2, 5.0),
        Some(change.clone()),
    );
    ctx.c.same(
        "the writer does not",
        next_change(&north1, 1.0),
        None,
    );
    ctx.c.same(
        "a phone of another tenant does not",
        next_change(&south, 0.5),
        None,
    );
    ctx.c.same(
        "a phone without a tenant does not",
        next_change(&plain1, 0.5),
        None,
    );

    set(&plain1, Scope::Shared, "watch/k", b"1", None)?;
    ctx.c.same(
        "a phone that watches another app does not hear of this one",
        next_change(&plain2, 1.0),
        None,
    );
    ctx.c.same(
        "nor does a tenant hear of the phones without one",
        next_change(&north2, 0.5),
        None,
    );
    hello(&plain2, &["other", APP])?;
    ask(
        &plain1,
        &Op::Remove {
            app: APP.into(),
            scope: Scope::Shared,
            key: "watch/k".into(),
            expect: None,
        },
    )?;
    ctx.c.same(
        "a removal is a change, and the phones without a tenant hear of each other",
        next_change(&plain2, 5.0),
        Some(change.clone()),
    );

    set(&north1, Scope::User, "watch/mine", b"1", None)?;
    set(&north1, Scope::Shared, "watch/k", b"2", Some(0))?;
    ctx.c.same(
        "a user's key and a refused write are not changes",
        next_change(&north2, 1.0),
        None,
    );

    hello(&north2, &[])?;
    set(&north1, Scope::Shared, "watch/k", b"3", None)?;
    ctx.c.same(
        "a phone that watches no longer does not hear",
        next_change(&north2, 1.0),
        None,
    );
    let db = ctx.cli_text("database show DPhoneAppData/watch");
    ctx.c.contains(
        "the watch lists are in AstDB",
        &db,
        &["/DPhoneAppData/watch/north1", "/DPhoneAppData/watch/plain2"],
    );
    ctx.c.lacks(
        "an empty one is not",
        &db,
        &["/DPhoneAppData/watch/north2"],
    );
    Ok(())
}

/// The store outlasts Asterisk.
fn restart(ctx: &mut Ctx) -> Result<()> {
    let written = {
        let a = phone(ctx, "north1")?;
        set(&a, Scope::User, "restart/level", b"12", None)?;
        match set(&a, Scope::Shared, "restart/k", b"kept", None)? {
            Outcome::Stored(e) => e,
            other => return Err(anyhow!("set answered {other:?}")),
        }
    };
    ctx.drop_ami();
    ctx.inst.stop()?;
    ctx.inst.start()?;
    let (a, b) = (phone(ctx, "north1")?, phone(ctx, "north2")?);
    ctx.c.same(
        "a shared entry is there after a restart, as it was",
        get(&b, Scope::Shared, "restart/k")?,
        Some(written.clone()),
    );
    ctx.c.same(
        "and so is a user's",
        value_of(&get(&a, Scope::User, "restart/level")?),
        Some(&b"12"[..]),
    );
    ctx.c.same(
        "another tenant still does not read it",
        get(&phone(ctx, "south1")?, Scope::Shared, "restart/k")?,
        None,
    );
    let again = set(&b, Scope::Shared, "restart/k", b"new", Some(written.version))?;
    ctx.c.check(
        "versions go on from where they were",
        matches!(&again, Outcome::Stored(e) if e.version > written.version),
        format!("{again:?}"),
    );
    Ok(())
}

fn main() -> ExitCode {
    let mut suite = Suite::new("app-data").prepare(|inst| {
        fixtures::prepare(inst, |inst| {
            fixtures::sim_setup(inst)?;
            setup::install_snippet(
                inst,
                "res_digium_phone_app_data.conf",
                "res_digium_phone.conf",
                PHONES_CONF.as_bytes(),
            )
        })
    });
    let parts: [(&str, Part); 5] = [
        ("store", store),
        ("tenants", tenants),
        ("conflict", conflict),
        ("watch", watch),
        ("restart", restart),
    ];
    for (name, f) in parts {
        suite = suite.part(name, move |ctx| part(ctx, f));
    }
    suite.main()
}
