//! DSP replay tables from shared-RAM traces.
//!
//! `json` lists the one-time (configuration) LCC job submissions seen in
//! VRM1/VRM3 traces, in order, with whole bodies.  `convert` turns such a
//! JSON file into the line format the DSP replay tools read.  Both are
//! offline and read-only on their inputs.

use serde_json::{Map, Value, json};
use std::collections::HashMap;

const VRM1: u32 = 0x5652_4d31;
const VRM3: u32 = 0x5652_4d33;

fn client_name(id: u16) -> Option<&'static str> {
    match id {
        1 => Some("PVE"),
        3 => Some("SBAEC"),
        4 => Some("HEC"),
        8 => Some("BAND"),
        _ => None,
    }
}

fn header_words(id: u16) -> usize {
    match id {
        1 | 3 => 10,
        _ => 9,
    }
}

/// Per-frame and polling operations: everything that is not configuration.
fn per_frame(id: u16, op: u16) -> bool {
    match id {
        1 => matches!(op, 2 | 5),
        3 => matches!(op, 7 | 8 | 9 | 17),
        4 => matches!(op, 2 | 3 | 8 | 14),
        8 => matches!(op, 1 | 2 | 4 | 5),
        _ => false,
    }
}

fn op_name(id: u16, op: u16) -> &'static str {
    match (id, op) {
        (1, 0) => "enc_init",
        (1, 3) => "dec_init",
        (1, 6) => "query_codec",
        (1, 7) => "query_rate_code",
        (3, 0) => "init",
        (3, 1) => "enable",
        (3, 2) => "reset",
        (3, 3) => "reset_cold",
        (3, 4) => "set_reg1",
        (3, 5) => "set_regs",
        (3, 6) => "get_regs",
        (3, 13) => "deinit",
        (3, 14) => "set_bits",
        (3, 15) => "clr_bits",
        (3, 16) => "get_reg1",
        (4, 0) => "init",
        (4, 1) => "deinit",
        (4, 4) => "enable",
        (4, 5) => "set_regs",
        (4, 6) => "set_reg1",
        (4, 7) => "get_regs",
        (4, 9) => "reset",
        (4, 10) => "reset_coeff",
        (4, 12) => "set_bulk_delay",
        (4, 13) => "get_bulk_delay",
        (4, 15) => "set_low_complexity",
        (8, 0) => "split_init",
        (8, 3) => "combine_init",
        _ => "?",
    }
}

fn u32le(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

fn words(d: &[u8]) -> Vec<u16> {
    d.as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect()
}

/// Complete frames of the current pass over a ring: (index, payload words).
fn frames(w: &[u16]) -> Vec<(usize, &[u16])> {
    let mut out = Vec::new();
    if w.len() < 3 {
        return out;
    }
    let mut wr = (w[0] & 0x7fff) as usize;
    // New-family producers wrap right after a frame that ends near the top,
    // so the write index reads 0 while the whole previous pass is in place.
    if wr == 0 {
        wr = w.len() - 2;
    }
    let mut i = 0;
    while i < wr {
        let Some(&len) = w.get(2 + i) else { break };
        let len = len as usize;
        if len == 0 || len & 0x8000 != 0 || i + 1 + len > wr || 3 + i + len > w.len() {
            break;
        }
        let p = &w[3 + i..3 + i + len];
        // A frame caught while the producer was still writing shows up as nonsense.
        if len < 9 || p[0] != 0 || p[1] >= 32 || client_name(p[2]).is_none() {
            break;
        }
        out.push((i, p));
        i += 1 + len;
    }
    out
}

/// Every newly seen frame in a trace: (microseconds, ring, payload).
fn jobs(data: &[u8]) -> Result<Vec<(u32, usize, Vec<u16>)>, String> {
    let mut out = Vec::new();
    let mut img: Option<Vec<u8>> = None;
    let mut n = 0usize;
    let mut prev: HashMap<usize, HashMap<usize, Vec<u16>>> = HashMap::new();
    let mut o = 0;
    while o + 16 <= data.len() {
        let (magic, kind, us, len) = (
            u32le(data, o),
            u32le(data, o + 4),
            u32le(data, o + 8),
            u32le(data, o + 12) as usize,
        );
        if magic != VRM1 && magic != VRM3 {
            return Err(format!("bad magic at {o}"));
        }
        o += 16;
        let p = &data[o..(o + len).min(data.len())];
        o += len;
        // The rings this record lets us look at.
        let mut rings: Vec<(usize, Vec<u16>)> = Vec::new();
        if kind == 0xff {
            n = p.len().saturating_sub(0x10) / 8;
            img = Some(p.to_vec());
        } else if magic == VRM3 {
            let img = img
                .as_mut()
                .ok_or("difference record before a full image")?;
            let mut q = 0;
            while q + 4 <= p.len() {
                let off = u16::from_le_bytes([p[q], p[q + 1]]) as usize;
                let l = u16::from_le_bytes([p[q + 2], p[q + 3]]) as usize;
                q += 4;
                if q + l > p.len() || off + l > img.len() {
                    return Err("difference record out of range".into());
                }
                img[off..off + l].copy_from_slice(&p[q..q + l]);
                q += l;
            }
        } else {
            rings.push((kind as usize, words(p)));
        }
        if rings.is_empty() {
            let img = img.as_ref().expect("set above");
            for k in 0..4 {
                let start = 0x10 + k * 2 * n;
                if start + 2 * n <= img.len() {
                    rings.push((k, words(&img[start..start + 2 * n])));
                }
            }
        }
        rings.sort_by_key(|r| std::cmp::Reverse(r.0));
        for (k, w) in rings {
            let mut cur = HashMap::new();
            for (i, p) in frames(&w) {
                if prev.get(&k).and_then(|m| m.get(&i)).map(|v| v.as_slice()) != Some(p) {
                    out.push((us, k, p.to_vec()));
                }
                cur.insert(i, p.to_vec());
            }
            prev.insert(k, cur);
        }
    }
    Ok(out)
}

fn sequence(data: &[u8]) -> Result<Vec<Value>, String> {
    let mut out: Vec<Value> = Vec::new();
    let mut prev: Option<Map<String, Value>> = None;
    for (us, ring, p) in jobs(data)? {
        if ring != 1 {
            continue;
        }
        let cid = p[2];
        let pve = cid == 1;
        let h = header_words(cid);
        if p.len() < h {
            continue;
        }
        let op = if pve { p[6] } else { p[5] };
        if per_frame(cid, op) {
            continue;
        }
        let body: Vec<Value> = p[h..]
            .iter()
            .map(|x| Value::String(format!("{x:04x}")))
            .collect();
        let mut j = Map::new();
        j.insert("t_us".into(), json!(us));
        j.insert("client".into(), json!(client_name(cid).unwrap()));
        j.insert("client_id".into(), json!(cid));
        j.insert("channel".into(), json!(p[1]));
        j.insert("instance".into(), json!(p[4]));
        j.insert("op".into(), json!(op));
        j.insert("op_name".into(), json!(op_name(cid, op)));
        j.insert("arg1".into(), json!(p[if pve { 7 } else { 6 }]));
        j.insert("arg2".into(), json!(p[if pve { 8 } else { 7 }]));
        j.insert("frame_words".into(), json!(p.len()));
        j.insert("header_words".into(), json!(h));
        j.insert("body".into(), Value::Array(body));
        if pve {
            j.insert("codec".into(), json!(p[5]));
        }
        let mut key = j.clone();
        key.shift_remove("t_us");
        if prev.as_ref() == Some(&key) {
            continue; // the same frame seen again after a ring pass
        }
        prev = Some(key);
        out.push(Value::Object(j));
    }
    Ok(out)
}

/// Build the replay JSON document from `(name, trace path)` pairs.
pub fn make_json(
    family: &str,
    traces: &[(String, std::path::PathBuf)],
) -> Result<(String, Vec<(String, usize)>), String> {
    let mut sequences = Map::new();
    let mut counts = Vec::new();
    for (name, path) in traces {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let jobs = sequence(&data).map_err(|e| format!("{}: {e}", path.display()))?;
        counts.push((name.clone(), jobs.len()));
        let source = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        sequences.insert(name.clone(), json!({"source": source, "jobs": jobs}));
    }
    let doc = json!({
        "family": family,
        "word_size_bits": 16,
        "note": "One-time LCC job submissions in order; bodies are 16-bit words in hex as submitted. \
                 Header fields (arg1, arg2) may already hold reply values; see NOTES.md sections 3.11 and 7.",
        "sequences": sequences,
    });
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    serde::Serialize::serialize(&doc, &mut ser).map_err(|e| e.to_string())?;
    Ok((String::from_utf8(buf).expect("JSON is UTF-8"), counts))
}

/// Statistics of the clean-ups `convert` applied.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ConvertStats {
    pub queries_zeroed: usize,
    pub time0_repeats_dropped: usize,
    pub rewritten_states_dropped: usize,
}

/// Turn a replay JSON document into the `.rpl` line format:
///
/// ```text
/// F <family>
/// S <sequence name>
/// J <t_us> <client> <channel> <instance> <codec> <op> <arg1> <arg2> <frame_words> <nbody> <w0> ...
/// ```
///
/// Unless `raw`, three heuristics are applied: query jobs (PVE op 6 and 7)
/// get zero arguments; in `boot_partN` with N > 1, jobs at time 0 repeat the
/// previous part and are dropped; and a job directly followed by one that
/// differs only in its arguments is the same frame before and after the DSP
/// rewrote it, so only the first is kept (set-one-register jobs and queries
/// are exempt).
pub fn convert(
    source_name: &str,
    json_text: &str,
    raw: bool,
) -> Result<(String, ConvertStats), String> {
    let doc: Value = serde_json::from_str(json_text).map_err(|e| e.to_string())?;
    let mut stats = ConvertStats::default();
    let mut out = vec![
        format!(
            "# derived from {source_name} by replay-convert.py{}",
            if raw { " --raw" } else { "" }
        ),
        format!("F {}", doc["family"].as_str().ok_or("missing family")?),
    ];
    let num = |j: &Value, k: &str| -> Result<u64, String> {
        j[k].as_u64().ok_or(format!("missing or bad {k}"))
    };
    for (name, seq) in doc["sequences"].as_object().ok_or("missing sequences")? {
        out.push(format!("S {name}"));
        let later_part = name
            .strip_prefix("boot_part")
            .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|n| n.parse::<u64>().ok())
            .is_some_and(|n| n > 1);
        let mut prev: Option<(u64, u64, u64, u64, u64, Vec<String>)> = None;
        for j in seq["jobs"].as_array().ok_or("missing jobs")? {
            let (cid, op) = (num(j, "client_id")?, num(j, "op")?);
            let (mut a1, mut a2) = (num(j, "arg1")?, num(j, "arg2")?);
            let codec = j.get("codec").and_then(Value::as_u64).unwrap_or(0);
            let body: Vec<String> = j["body"]
                .as_array()
                .ok_or("missing body")?
                .iter()
                .map(|w| {
                    w.as_str()
                        .map(str::to_owned)
                        .ok_or("body word is not a string")
                })
                .collect::<Result<_, _>>()?;
            if num(j, "frame_words")? != num(j, "header_words")? + body.len() as u64 {
                return Err(format!("inconsistent frame in {name}"));
            }
            let query = cid == 1 && (op == 6 || op == 7);
            let set_one_reg = (cid == 3 && op == 4) || (cid == 4 && op == 6);
            if !raw {
                if later_part && num(j, "t_us")? == 0 {
                    stats.time0_repeats_dropped += 1;
                    continue;
                }
                if query {
                    stats.queries_zeroed += usize::from(a1 != 0 || a2 != 0);
                    a1 = 0;
                    a2 = 0;
                }
                let key = (
                    cid,
                    num(j, "channel")?,
                    num(j, "instance")?,
                    codec,
                    op,
                    body.clone(),
                );
                if prev.as_ref() == Some(&key) && !set_one_reg && !query {
                    stats.rewritten_states_dropped += 1;
                    continue;
                }
                prev = Some(key);
            }
            let mut line = format!(
                "J {} {cid} {} {} {codec} {op} {a1} {a2} {} {}",
                num(j, "t_us")?,
                num(j, "channel")?,
                num(j, "instance")?,
                num(j, "frame_words")?,
                body.len()
            );
            for w in &body {
                line.push(' ');
                line.push_str(w);
            }
            out.push(line);
        }
    }
    Ok((out.join("\n") + "\n", stats))
}
