//! The stock `ConfigMerge` algorithm, reimplemented from the firmware's
//! merge routine and checked against the stock binary's output.
//!
//! The rules, in the order the stock code applies them when merging a later
//! element `b` into an accumulated element `a`:
//!
//! 1. Both elements first have their own children "self-merged": children
//!    that do not match the selected network are removed, and later siblings
//!    with the same name and identity are folded into the first one.
//! 2. If `a` is frozen (`conflict="freeze"`), `b` is ignored.
//! 3. If `b` does not match the selected network, it is ignored.
//! 4. If either is `replace` (explicitly, or by default for `accounts`,
//!    `use_proxy_for`, `blf_items`, and `smart_blf`), or `b` is frozen, `b`
//!    replaces `a` wholesale.
//! 5. Otherwise `b`'s attributes are set on `a`, and `b`'s children are
//!    merged into the children of `a` with the same name and identity, or
//!    appended.
//!
//! Identity is the `id` attribute, except for the elements in [`ID_ATTRS`].

use std::net::Ipv4Addr;

use crate::xml::{Element, Node};

/// Elements whose identity attribute is not `id`.
const ID_ATTRS: &[(&str, &str)] = &[
    ("account", "index"),
    ("use_proxy_for", "host"),
    ("contacts", "id"),
    ("alert", "alert_info"),
    ("firmware", "model"),
    ("public_firmware", "url"),
    ("send_dnd", "status"),
];

/// Elements that are replaced, not merged, unless they say otherwise.
const DEFAULT_REPLACE: &[&str] = &["accounts", "use_proxy_for", "blf_items", "smart_blf"];

/// Setting IDs renamed on the way in (old, new).
const RENAMED_SETTINGS: &[(&str, &str)] = &[("network_vlan_qos", "rtp_qos")];

/// Network facts the merge needs: the phone's current IPv4 address, used to
/// pick a network profile by longest matching CIDR prefix and to filter
/// elements carrying a `network="a.b.c.d/n"` attribute.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetContext {
    pub ip: Option<Ipv4Addr>,
}

struct State {
    ip: u32,
    selected: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Conflict {
    Merge,
    Replace,
    Freeze,
}

fn id_value(e: &Element) -> &str {
    let attr = ID_ATTRS.iter().find(|(n, _)| *n == e.name).map_or("id", |(_, a)| a);
    e.attr(attr).unwrap_or("")
}

fn update_setting_id(e: &mut Element) {
    if e.name != "setting" {
        return;
    }
    if let Some(id) = e.attr("id")
        && let Some((_, new)) = RENAMED_SETTINGS.iter().find(|(old, _)| *old == id)
    {
        e.set_attr("id", new);
    }
}

fn conflict(e: &Element) -> Conflict {
    let policy = match e.attr("conflict") {
        Some(c) => c,
        None if DEFAULT_REPLACE.contains(&e.name.as_str()) => "replace",
        None => "",
    };
    match policy {
        "replace" => Conflict::Replace,
        "freeze" => Conflict::Freeze,
        _ => Conflict::Merge,
    }
}

/// Parse `a[.b[.c[.d]]][/bits]` the way the stock `NetUtil::parse_cidr` does.
/// Returns (network, mask) in host byte order.
pub fn parse_cidr(t: &str) -> Option<(u32, u32)> {
    let (addr_part, bits_part) = match t.split_once('/') {
        Some((a, b)) => (a, Some(b)),
        None => (t, None),
    };
    let mut addr: u32 = 0;
    let mut n = 0;
    for part in addr_part.split('.') {
        if n == 4 || part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let v: u32 = part.parse().ok().filter(|v| *v <= 255)?;
        addr = (addr << 8) | v;
        n += 1;
    }
    if n < 4 {
        addr <<= (4 - n) * 8;
    }
    let bits: u32 = match bits_part {
        None => 32,
        Some(b) => {
            let digits: String = b.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
            // strtol returns 0 when there are no digits.
            let v: u32 = if digits.is_empty() { 0 } else { digits.parse().ok()? };
            if v > 32 {
                return None;
            }
            v
        }
    };
    let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
    Some((addr & mask, mask))
}

fn network_match(e: &Element, st: &State) -> bool {
    if let Some(id) = e.attr("network_id") {
        return st.selected.as_deref() == Some(id);
    }
    match e.attr("network") {
        None => true,
        Some(n) => parse_cidr(n).is_some_and(|(net, mask)| st.ip & mask == net),
    }
}

fn element_at(e: &mut Element, idx: usize) -> &mut Element {
    match &mut e.children[idx] {
        Node::Element(c) => c,
        _ => unreachable!("index is an element index"),
    }
}

fn self_merge_children(e: &mut Element, st: &State) {
    if e.name == "blf_items" || e.name == "smart_blf" {
        return;
    }
    let mut i = 0;
    while i < e.children.len() {
        if !matches!(e.children[i], Node::Element(_)) {
            i += 1;
            continue;
        }
        let cur = element_at(e, i);
        update_setting_id(cur);
        if !network_match(cur, st) {
            e.children.remove(i);
            continue;
        }
        self_merge_children(cur, st);
        let mut j = i + 1;
        while j < e.children.len() {
            if !matches!(e.children[j], Node::Element(_)) {
                j += 1;
                continue;
            }
            let sib = element_at(e, j);
            update_setting_id(sib);
            if !network_match(sib, st) {
                e.children.remove(j);
                continue;
            }
            let same = {
                let (Node::Element(a), Node::Element(b)) = (&e.children[i], &e.children[j]) else {
                    unreachable!()
                };
                a.name == b.name
                    && id_value(a) == id_value(b)
                    && a.name != "blf_items"
                    && a.name != "smart_blf"
            };
            if same {
                let Node::Element(sib) = e.children.remove(j) else { unreachable!() };
                merge(element_at(e, i), sib, st);
                continue;
            }
            j += 1;
        }
        i += 1;
    }
}

fn merge(a: &mut Element, mut b: Element, st: &State) {
    self_merge_children(a, st);
    self_merge_children(&mut b, st);
    let ca = conflict(a);
    let cb = conflict(&b);
    if ca == Conflict::Freeze {
        return;
    }
    if !network_match(&b, st) {
        return;
    }
    if ca == Conflict::Replace || cb == Conflict::Replace || cb == Conflict::Freeze {
        *a = b;
        return;
    }
    // An empty login password in a later layer keeps the earlier one.
    if b.name == "setting" && b.attr("id") == Some("login_password") && b.attr("value") == Some("") {
        let earlier = a.attr("value").unwrap_or("").to_owned();
        b.set_attr("value", &earlier);
    }
    for (k, v) in &b.attrs {
        // Empty account credentials do not erase earlier ones.
        if b.name == "account" && (k == "password" || k == "passcode") && v.is_empty() {
            continue;
        }
        a.set_attr(k, v);
    }
    if b.elements().next().is_none() {
        a.children.retain(|n| !matches!(n, Node::Text(_)));
        if let Some(t) = b.text() {
            a.children.push(Node::Text(t.to_owned()));
        }
        return;
    }
    for node in b.children {
        let Node::Element(child) = node else { continue };
        let target = a.children.iter_mut().find_map(|n| match n {
            Node::Element(e) if e.name == child.name && id_value(e) == id_value(&child) => Some(e),
            _ => None,
        });
        match target {
            Some(t) => merge(t, child, st),
            None => a.children.push(Node::Element(child)),
        }
    }
}

/// Merge one more layer into the accumulated configuration.
///
/// The network profile is chosen afresh for every layer: an explicit
/// `<netselect id>` in the accumulated configuration wins; otherwise the
/// `<networks>` list (the accumulated one, else the incoming one) is searched
/// for the matching CIDR with the longest prefix, first match winning ties.
pub fn merge_layer(acc: &mut Element, layer: Element, net: &NetContext) {
    let ip = net.ip.map_or(0, u32::from);
    let mut selected = acc.child("netselect").and_then(|n| n.attr("id")).map(str::to_owned);
    if selected.is_none()
        && let Some(networks) = acc.child("networks").or_else(|| layer.child("networks"))
    {
        let mut best: i32 = -1;
        for n in networks.children_named("network") {
            let Some((netw, mask)) = n.attr("cidr").and_then(parse_cidr) else { continue };
            let bits = mask.count_ones() as i32;
            if ip & mask == netw && bits > best {
                selected = n.attr("id").map(str::to_owned);
                best = bits;
            }
        }
    }
    merge(acc, layer, &State { ip, selected });
}

/// Merge layers in order, starting from an empty `<config>`, as the stock
/// `configmerge` tool does with its arguments.
pub fn merge_layers(layers: impl IntoIterator<Item = Element>, net: &NetContext) -> Element {
    let mut acc = Element::new("config");
    for l in layers {
        merge_layer(&mut acc, l, net);
    }
    acc
}

/// The network profile the merged configuration selects, by the same rule
/// [`merge_layer`] uses.
pub fn selected_network(config: &Element, net: &NetContext) -> Option<String> {
    if let Some(id) = config.child("netselect").and_then(|n| n.attr("id")) {
        return Some(id.to_owned());
    }
    let ip = net.ip.map_or(0, u32::from);
    let mut best: i32 = -1;
    let mut selected = None;
    for n in config.child("networks")?.children_named("network") {
        let Some((netw, mask)) = n.attr("cidr").and_then(parse_cidr) else { continue };
        let bits = mask.count_ones() as i32;
        if ip & mask == netw && bits > best {
            selected = n.attr("id").map(str::to_owned);
            best = bits;
        }
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr() {
        assert_eq!(parse_cidr("10.0.0.0/8"), Some((0x0a00_0000, 0xff00_0000)));
        assert_eq!(parse_cidr("10.1.2.3"), Some((0x0a01_0203, u32::MAX)));
        assert_eq!(parse_cidr("10/8"), Some((0x0a00_0000, 0xff00_0000)));
        assert_eq!(parse_cidr("0.0.0.0/0"), Some((0, 0)));
        assert_eq!(parse_cidr("10.0.0.0/33"), None);
        assert_eq!(parse_cidr("300.0.0.0/8"), None);
        assert_eq!(parse_cidr("x"), None);
    }
}
