// SPDX-License-Identifier: GPL-2.0-or-later
//! Turns the committed replay JSON files (one-time DSP jobs observed on a
//! D60 and a D40) into static tables.
//!
//! The JSON lists submissions as they were seen in the shared RAM, and the
//! DSP rewrites a submission's header in place with its reply values while
//! the frame is still there.  So some argument values in the JSON are
//! replies, and some jobs appear twice.  Three clean-ups are applied (the
//! same ones the earlier `replay-convert.py` applied):
//!
//!  1. Query jobs (PVE op 6 and op 7) take no arguments: arg1 and arg2 are
//!     written as 0.
//!  2. In the later parts of a split boot trace (`boot_partN`, N > 1), jobs
//!     with time 0 were already in the ring when that part began; they
//!     repeat the end of the previous part and are dropped.
//!  3. A job directly followed by one that differs only in its arguments is
//!     the same frame seen before and after the DSP rewrote it; the first is
//!     kept.  Set-one-register jobs and queries are exempt.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::{env, fs, path::Path};

use serde_json::Value;

fn u(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn main() {
    let dir = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("data");
    let mut out = String::new();
    let mut bodies: HashMap<Vec<u16>, usize> = HashMap::new();
    let mut body_defs = String::new();
    let mut tables = String::new();

    for (file, ident, family) in [
        ("replay-new-family-d60.json", "NEW_FAMILY", "New"),
        ("replay-old-family-d40.json", "OLD_FAMILY", "Old"),
    ] {
        let path = dir.join(file);
        println!("cargo:rerun-if-changed={}", path.display());
        let doc: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let want = if family == "New" { "new" } else { "old" };
        assert_eq!(doc["family"].as_str(), Some(want), "{file}: wrong family");

        writeln!(tables, "pub static {ident}: ReplayTable = ReplayTable {{ family: Family::{family}, sequences: &[").unwrap();
        for (name, seq) in doc["sequences"].as_object().unwrap() {
            let later_part = name
                .strip_prefix("boot_part")
                .and_then(|n| n.parse::<u32>().ok())
                .is_some_and(|n| n > 1);
            let derived = seq.get("derived").and_then(Value::as_str) == Some("static");
            writeln!(tables, "  Sequence {{ name: {name:?}, derived_static: {derived}, jobs: &[").unwrap();
            let mut prev: Option<(u64, u64, u64, u64, u64, Vec<u16>)> = None;
            for j in seq["jobs"].as_array().unwrap() {
                let (cid, op) = (u(j, "client_id"), u(j, "op"));
                let (mut a1, mut a2) = (u(j, "arg1"), u(j, "arg2"));
                let body: Vec<u16> = j["body"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|w| u16::from_str_radix(w.as_str().unwrap(), 16).unwrap())
                    .collect();
                assert_eq!(
                    u(j, "frame_words"),
                    u(j, "header_words") + body.len() as u64,
                    "{file}: inconsistent frame in {name}"
                );
                if later_part && u(j, "t_us") == 0 {
                    continue;
                }
                let query = cid == 1 && (op == 6 || op == 7);
                let set_one = (cid == 3 && op == 4) || (cid == 4 && op == 6);
                if query {
                    a1 = 0;
                    a2 = 0;
                }
                let key = (cid, u(j, "channel"), u(j, "instance"), u(j, "codec"), op, body.clone());
                if prev.as_ref() == Some(&key) && !set_one && !query {
                    continue;
                }
                prev = Some(key);
                let next = bodies.len();
                let id = *bodies.entry(body.clone()).or_insert_with(|| {
                    write!(body_defs, "static B{next}: [u16; {}] = [", body.len()).unwrap();
                    for w in &body {
                        write!(body_defs, "0x{w:04x},").unwrap();
                    }
                    body_defs.push_str("];\n");
                    next
                });
                writeln!(
                    tables,
                    "    ReplayJob {{ t_us: {}, client: {cid}, channel: {}, instance: {}, codec: {}, op: {op}, arg1: {a1}, arg2: {a2}, frame_words: {}, body: &B{id} }},",
                    u(j, "t_us"), u(j, "channel"), u(j, "instance"), u(j, "codec"), u(j, "frame_words"),
                )
                .unwrap();
            }
            tables.push_str("  ] },\n");
        }
        tables.push_str("] };\n");
    }

    out.push_str(&body_defs);
    out.push_str(&tables);
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("replay_tables.rs"), out).unwrap();
    println!("cargo:rerun-if-changed=build.rs");
}
