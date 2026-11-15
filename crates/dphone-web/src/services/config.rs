//! `ConfigService`, `ContactsService`, and the stored-configuration parts
//! of `AccountsService` and `NetworkService`.

use super::with_config;
use crate::rpc::ConnectError;
use crate::http::Ctx;
use crate::{convert, pb, App};
use dphone_config::contacts::parse_contacts;
use dphone_config::view::Transport;
use dphone_config::{ConfigStore, Element, Layer, Node};
use dphone_state::{self as st, Command};
use std::net::Ipv4Addr;
use std::sync::Arc;

fn config_of(app: &App) -> Arc<dphone_config::Config> {
    app.config.lock().unwrap_or_else(|p| p.into_inner()).config()
}

fn flag(b: bool) -> &'static str {
    if b { "1" } else { "0" }
}

fn clean(what: &str, s: &str, max: usize) -> Result<(), ConnectError> {
    if s.len() > max {
        return Err(ConnectError::invalid_argument(format!("{what} is too long")));
    }
    if s.chars().any(char::is_control) {
        return Err(ConnectError::invalid_argument(format!("{what} contains a control character")));
    }
    Ok(())
}

// -------------------------------------------------------------- settings

/// Settings whose values are not sent to the browser.
fn is_secret(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    ["password", "passcode", "secret", "_pin", "psk"].iter().any(|w| id.contains(w))
}

fn valid_setting_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn local_ids(c: &ConfigStore) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    if let Ok(Some(root)) = c.layer(Layer::Ui) {
        ids.extend(root.children_named("setting").filter_map(|e| e.attr("id").map(str::to_owned)));
    }
    ids
}

pub async fn list_settings(ctx: Ctx, req: pb::ListSettingsRequest) -> Result<pb::ListSettingsResponse, ConnectError> {
    with_config(&ctx.app, false, move |c| {
        let cfg = c.config();
        let local = local_ids(c);
        let mut settings: Vec<pb::Setting> = cfg
            .settings()
            .filter(|(id, _)| id.starts_with(&req.prefix))
            .map(|(id, value)| {
                let secret = is_secret(id);
                pb::Setting {
                    id: id.to_owned(),
                    value: if secret { String::new() } else { value.to_owned() },
                    frozen: cfg.setting_frozen(id),
                    local: local.contains(id),
                    secret,
                }
            })
            .collect();
        settings.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(pb::ListSettingsResponse { settings, generation: cfg.generation() })
    })
    .await
}

pub async fn set_settings(ctx: Ctx, req: pb::SetSettingsRequest) -> Result<pb::SetSettingsResponse, ConnectError> {
    if req.changes.is_empty() {
        return Err(ConnectError::invalid_argument("no changes"));
    }
    for ch in &req.changes {
        if !valid_setting_id(&ch.id) {
            return Err(ConnectError::invalid_argument(format!("not a setting ID: {:?}", ch.id)));
        }
        if ch.id == "login_password" {
            return Err(ConnectError::invalid_argument("use ChangePassword to change the web password"));
        }
        clean(&ch.id, &ch.value, 4096)?;
    }
    with_config(&ctx.app, true, move |c| {
        let changes: Vec<(&str, &str)> = req.changes.iter().map(|ch| (ch.id.as_str(), ch.value.as_str())).collect();
        let cfg = c.set_many(&changes).map_err(crate::services::config_error)?;
        Ok(pb::SetSettingsResponse { generation: cfg.generation() })
    })
    .await
}

pub async fn unset_settings(
    ctx: Ctx,
    req: pb::UnsetSettingsRequest,
) -> Result<pb::UnsetSettingsResponse, ConnectError> {
    if req.ids.is_empty() {
        return Err(ConnectError::invalid_argument("no settings named"));
    }
    if let Some(bad) = req.ids.iter().find(|id| !valid_setting_id(id) || *id == "login_password") {
        return Err(ConnectError::invalid_argument(format!("cannot unset {bad:?}")));
    }
    with_config(&ctx.app, true, move |c| {
        let mut generation = c.config().generation();
        for id in &req.ids {
            generation = c.unset(id).map_err(crate::services::config_error)?.generation();
        }
        Ok(pb::UnsetSettingsResponse { generation })
    })
    .await
}

// ---------------------------------------------------------- idle softkeys

/// The softkeys in effect and every softkey the phone offers: the
/// functions, then an entry per app (the contacts app is "contacts").
fn idle_softkeys(app: &App, cfg: &dphone_config::Config) -> pb::IdleSoftkeys {
    let set = cfg.idle_softkeys();
    let choice = |name: &str, label: &str| pb::IdleSoftkeyChoice { name: name.to_owned(), label: label.to_owned() };
    let mut choices: Vec<pb::IdleSoftkeyChoice> = [
        ("call_log", "Call Log"),
        ("contacts", "Contacts"),
        ("status", "Status"),
        ("info", "Info"),
        ("menu", "Menu"),
        ("apps", "Apps"),
        ("dnd", "DND"),
        ("forward", "Forward"),
        ("redial", "Redial"),
    ]
    .iter()
    .map(|(name, label)| choice(name, label))
    .collect();
    // The apps the phone has (one that is switched off is not among them).
    let apps = app.store.get::<st::Apps>();
    for a in apps.installed.iter().filter(|a| a.name != "contacts") {
        let label = if a.display_name.is_empty() { &a.name } else { &a.display_name };
        choices.push(choice(&format!("app:{}", a.name), &format!("{label} (app)")));
    }
    // A softkey in effect for an app the phone does not have keeps its
    // entry (the phone does not show it).
    for key in &set.keys {
        if !choices.iter().any(|c| c.name == key.name) {
            choices.push(choice(&key.name, key.name.strip_prefix("app:").unwrap_or(&key.name)));
        }
    }
    pb::IdleSoftkeys {
        softkeys: set.keys.into_iter().map(|k| k.name).collect(),
        source: super::ssh::setting_source(set.source).into(),
        locked: set.locked,
        choices,
    }
}

pub async fn get_idle_softkeys(
    ctx: Ctx,
    _req: pb::GetIdleSoftkeysRequest,
) -> Result<pb::GetIdleSoftkeysResponse, ConnectError> {
    let keys = idle_softkeys(&ctx.app, &config_of(&ctx.app));
    Ok(pb::GetIdleSoftkeysResponse { idle_softkeys: Some(keys).into() })
}

pub async fn set_idle_softkeys(
    ctx: Ctx,
    req: pb::SetIdleSoftkeysRequest,
) -> Result<pb::SetIdleSoftkeysResponse, ConnectError> {
    let list = req.softkeys.join(",");
    if req.softkeys.iter().any(|k| k.contains(',')) || list.len() > 512 {
        return Err(ConnectError::invalid_argument("not a softkey list"));
    }
    dphone_config::settings::parse_idle_softkeys(&list).map_err(ConnectError::invalid_argument)?;
    let app = ctx.app.clone();
    let keys = with_config(&ctx.app, true, move |c| {
        if c.config().idle_softkeys().locked {
            return Err(super::ssh::locked());
        }
        let config = c
            .update_dphone_settings(|s| s.idle_softkeys = (!list.is_empty()).then_some(list))
            .map_err(crate::services::config_error)?;
        Ok(idle_softkeys(&app, &config))
    })
    .await?;
    Ok(pb::SetIdleSoftkeysResponse { idle_softkeys: Some(keys).into() })
}

// --------------------------------------------------------------- network

pub(crate) fn network_settings(app: &App) -> pb::NetworkSettings {
    let cfg = config_of(app);
    let n = cfg.network();
    let t = cfg.time();
    let ip = |a: Option<Ipv4Addr>| a.map(|a| a.to_string()).unwrap_or_default();
    pb::NetworkSettings {
        dhcp: n.dhcp,
        static_ip: ip(n.static_ip),
        subnet_mask: ip(n.subnet_mask),
        gateway: ip(n.gateway),
        primary_dns: ip(n.dns.first().copied()),
        secondary_dns: ip(n.dns.get(1).copied()),
        domain: n.domain.unwrap_or_default(),
        vlan_discovery_mode: n.vlan_discovery_mode,
        vlan_id: n.vlan_id.map(u32::from),
        pc_vlan_id: n.pc_vlan_id.map(u32::from),
        pc_qos: n.pc_qos as u32,
        pc_port_mode: n.pc_port_mode,
        lan_port_mode: n.lan_port_mode,
        sip_dscp: n.sip_dscp as u32,
        rtp_dscp: n.rtp_dscp as u32,
        sip_qos: n.sip_qos as u32,
        rtp_qos: n.rtp_qos as u32,
        dot1x_passthrough: n.dot1x_passthrough,
        ntp_server: t.ntp_server,
        time_zone: t.time_zone,
        ipv6_enabled: n.ipv6_enabled,
    }
}

pub async fn set_network_settings(
    ctx: Ctx,
    req: pb::SetNetworkSettingsRequest,
) -> Result<pb::SetNetworkSettingsResponse, ConnectError> {
    let s = req.settings.ok_or_else(|| ConnectError::invalid_argument("no settings"))?;
    let ip = |what: &str, v: &str| -> Result<(), ConnectError> {
        if v.is_empty() || v.parse::<Ipv4Addr>().is_ok() {
            Ok(())
        } else {
            Err(ConnectError::invalid_argument(format!("{what} is not an IPv4 address")))
        }
    };
    ip("The address", &s.static_ip)?;
    ip("The subnet mask", &s.subnet_mask)?;
    ip("The gateway", &s.gateway)?;
    ip("The primary DNS server", &s.primary_dns)?;
    ip("The secondary DNS server", &s.secondary_dns)?;
    if !s.dhcp {
        if s.static_ip.is_empty() || s.subnet_mask.is_empty() {
            return Err(ConnectError::invalid_argument("a static configuration needs an address and a subnet mask"));
        }
        let mask = u32::from(s.subnet_mask.parse::<Ipv4Addr>().expect("checked"));
        if mask == 0 || mask.leading_ones() + mask.trailing_zeros() != 32 {
            return Err(ConnectError::invalid_argument("the subnet mask is not contiguous"));
        }
    }
    let vlan = |what: &str, v: Option<u32>| match v {
        Some(id) if !(1..=4094).contains(&id) => Err(ConnectError::invalid_argument(format!("{what} must be 1 to 4094"))),
        _ => Ok(()),
    };
    vlan("The VLAN ID", s.vlan_id)?;
    vlan("The PC port VLAN ID", s.pc_vlan_id)?;
    if s.sip_dscp > 63 || s.rtp_dscp > 63 {
        return Err(ConnectError::invalid_argument("DSCP values run 0 to 63"));
    }
    if s.sip_qos > 7 || s.rtp_qos > 7 || s.pc_qos > 7 {
        return Err(ConnectError::invalid_argument("802.1p priorities run 0 to 7"));
    }
    let mode = s.vlan_discovery_mode.to_ascii_uppercase();
    if !matches!(mode.as_str(), "NONE" | "MANUAL" | "LLDP") {
        return Err(ConnectError::invalid_argument("VLAN discovery is NONE, MANUAL, or LLDP"));
    }
    if mode == "MANUAL" && s.vlan_id.is_none() {
        return Err(ConnectError::invalid_argument("manual VLAN needs a VLAN ID"));
    }
    for (what, v) in [
        ("The domain", &s.domain),
        ("The NTP server", &s.ntp_server),
        ("The time zone", &s.time_zone),
        ("The PC port mode", &s.pc_port_mode),
        ("The LAN port mode", &s.lan_port_mode),
    ] {
        clean(what, v, 255)?;
    }
    let num = |v: Option<u32>| v.map(|v| v.to_string()).unwrap_or_default();
    let mut changes: Vec<(&'static str, String)> = vec![
        ("network_enable_dhcp", flag(s.dhcp).into()),
        ("network_static_ip_address", s.static_ip.clone()),
        ("network_subnet_mask", s.subnet_mask.clone()),
        ("network_default_gateway", s.gateway.clone()),
        ("network_primary_dns_server", s.primary_dns.clone()),
        ("network_secondary_dns_server", s.secondary_dns.clone()),
        ("network_domain_name", s.domain.clone()),
        ("network_vlan_discovery_mode", mode),
        ("network_vlan_id", num(s.vlan_id)),
        ("pc_vlan_id", num(s.pc_vlan_id)),
        ("pc_qos", s.pc_qos.to_string()),
        ("sip_dscp", s.sip_dscp.to_string()),
        ("rtp_dscp", s.rtp_dscp.to_string()),
        ("sip_qos", s.sip_qos.to_string()),
        ("rtp_qos", s.rtp_qos.to_string()),
        ("8021x_passthrough", flag(s.dot1x_passthrough).into()),
        ("network_ipv6_enabled", flag(s.ipv6_enabled).into()),
    ];
    // Left alone when the form sends them empty.
    for (id, v) in [
        ("pc_port_mode", &s.pc_port_mode),
        ("lan_port_mode", &s.lan_port_mode),
        ("ntp_server", &s.ntp_server),
        ("time_zone", &s.time_zone),
    ] {
        if !v.is_empty() {
            changes.push((id, v.clone()));
        }
    }
    with_config(&ctx.app, true, move |c| {
        let cfg = c.config();
        // Write only what differs, so untouched settings stay provisioned
        // (and frozen ones do not fail the whole save).
        let diff: Vec<(&str, &str)> = changes
            .iter()
            .filter(|(id, v)| cfg.setting(id).unwrap_or("") != v.as_str())
            .map(|(id, v)| (*id, v.as_str()))
            .collect();
        if !diff.is_empty() {
            c.set_many(&diff).map_err(crate::services::config_error)?;
        }
        Ok(())
    })
    .await?;
    if req.apply {
        // Not awaited: the address may change under this very connection.
        ctx.app.store.post(Command::ReconfigureNetwork);
    }
    Ok(pb::SetNetworkSettingsResponse {})
}

// -------------------------------------------------------------- accounts

fn pb_transport(t: Transport) -> buffa::EnumValue<pb::SipTransport> {
    (match t {
        Transport::Udp => pb::SipTransport::Udp,
        Transport::Tcp => pb::SipTransport::Tcp,
        Transport::Tls => pb::SipTransport::Tls,
    })
    .into()
}

fn transport_name(t: buffa::EnumValue<pb::SipTransport>) -> &'static str {
    match t.as_known().unwrap_or(pb::SipTransport::Udp) {
        pb::SipTransport::Tcp => "tcp",
        pb::SipTransport::Tls => "tls",
        _ => "udp",
    }
}

fn account_config(a: &dphone_config::view::Account) -> pb::AccountConfig {
    let primary = a.primary.clone().unwrap_or_default();
    let alt = a.alternate.clone();
    pb::AccountConfig {
        index: a.index,
        account_id: a.account_id.clone(),
        enabled: a.enabled,
        register: a.register,
        username: a.username.clone(),
        authname: a.authname.clone(),
        password: String::new(),
        has_password: !a.password.is_empty() || primary.password.is_some(),
        display_name: if a.caller_id.is_empty() { a.display.clone() } else { a.caller_id.clone() },
        line_label: a.line_label.clone(),
        server: primary.server.clone(),
        port: primary.port as u32,
        transport: pb_transport(primary.transport),
        alternate_server: alt.as_ref().map(|h| h.server.clone()).unwrap_or_default(),
        alternate_port: alt.as_ref().map(|h| h.port as u32).unwrap_or(0),
        alternate_transport: alt.as_ref().map(|h| pb_transport(h.transport)).unwrap_or_default(),
        outbound_proxy: a.outbound_proxy.clone().unwrap_or_default(),
        outbound_port: a.outbound_port.map(u32::from).unwrap_or(0),
        voicemail: a.voicemail.clone().unwrap_or_default(),
        srtp: primary.media_encryption.as_deref().is_some_and(|m| m.eq_ignore_ascii_case("sdes")),
        reregister_secs: primary.reregister,
    }
}

pub async fn list_accounts(ctx: Ctx, _req: pb::ListAccountsRequest) -> Result<pb::ListAccountsResponse, ConnectError> {
    let status = convert::accounts(&ctx.app.store.get::<st::Accounts>());
    with_config(&ctx.app, false, move |c| {
        let cfg = c.config();
        let local = matches!(c.layer(Layer::Ui), Ok(Some(root)) if root.child("accounts").is_some());
        Ok(pb::ListAccountsResponse {
            accounts: cfg.accounts().iter().map(account_config).collect(),
            status,
            max_accounts: cfg.max_accounts() as u32,
            local,
        })
    })
    .await
}

fn set_or_remove(e: &mut Element, name: &str, value: &str) {
    if value.is_empty() { e.remove_attr(name) } else { e.set_attr(name, value) }
}

fn port(what: &str, p: u32) -> Result<u32, ConnectError> {
    if p > 65535 { Err(ConnectError::invalid_argument(format!("{what} is not a port number"))) } else { Ok(p) }
}

/// Builds the stock `<account>` element for an edited account, starting
/// from the stored element so attributes this form does not model survive.
fn account_element(a: &pb::AccountConfig, existing: Option<&dphone_config::view::Account>) -> Result<Element, ConnectError> {
    for (what, v) in [
        ("The user name", &a.username),
        ("The authentication name", &a.authname),
        ("The password", &a.password),
        ("The display name", &a.display_name),
        ("The line label", &a.line_label),
        ("The server", &a.server),
        ("The alternate server", &a.alternate_server),
        ("The outbound proxy", &a.outbound_proxy),
        ("The voicemail number", &a.voicemail),
        ("The account ID", &a.account_id),
    ] {
        clean(what, v, 255)?;
    }
    if a.username.is_empty() {
        return Err(ConnectError::invalid_argument(format!("account {} needs a user name", a.index + 1)));
    }
    if a.server.is_empty() {
        return Err(ConnectError::invalid_argument(format!("account {} needs a server", a.index + 1)));
    }
    let mut e = existing.map(|x| x.raw.clone()).unwrap_or_else(|| Element::new("account"));
    e.set_attr("index", &a.index.to_string());
    e.set_attr("status", flag(a.enabled));
    e.set_attr("register", flag(a.register));
    e.set_attr("account_id", if a.account_id.is_empty() { &a.username } else { &a.account_id });
    e.set_attr("username", &a.username);
    set_or_remove(&mut e, "authname", if a.authname == a.username { "" } else { &a.authname });
    if !a.password.is_empty() {
        e.set_attr("password", &a.password);
    } else if existing.is_none() {
        e.set_attr("password", "");
    }
    set_or_remove(&mut e, "caller_id", &a.display_name);
    set_or_remove(&mut e, "display", &a.display_name);
    set_or_remove(&mut e, "line_label", &a.line_label);
    set_or_remove(&mut e, "voicemail", &a.voicemail);
    set_or_remove(&mut e, "outbound_proxy", &a.outbound_proxy);
    let out_port = port("The outbound proxy port", a.outbound_port)?;
    set_or_remove(&mut e, "outbound_port", &if out_port == 0 { String::new() } else { out_port.to_string() });
    // The host records carry these; drop account-level copies.
    for name in ["server", "port", "transport"] {
        e.remove_attr(name);
    }
    let reregister = if a.reregister_secs == 0 { 300 } else { a.reregister_secs.clamp(30, 86_400) };
    {
        let tls = transport_name(a.transport) == "tls";
        let p = match port("The server port", a.port)? {
            0 if tls => 5061,
            0 => 5060,
            p => p,
        };
        let h = e.child_or_insert("host_primary");
        h.set_attr("server", &a.server);
        h.set_attr("port", &p.to_string());
        h.set_attr("transport", transport_name(a.transport));
        h.set_attr("reregister", &reregister.to_string());
        if a.srtp { h.set_attr("media_encryption", "sdes") } else { h.remove_attr("media_encryption") }
    }
    if a.alternate_server.is_empty() {
        e.remove_children(|c| c.name == "host_alternate");
    } else {
        let tls = transport_name(a.alternate_transport) == "tls";
        let p = match port("The alternate server port", a.alternate_port)? {
            0 if tls => 5061,
            0 => 5060,
            p => p,
        };
        let h = e.child_or_insert("host_alternate");
        h.set_attr("server", &a.alternate_server);
        h.set_attr("port", &p.to_string());
        h.set_attr("transport", transport_name(a.alternate_transport));
        if h.attr("reregister").is_none() {
            h.set_attr("reregister", &reregister.to_string());
        }
        if a.srtp { h.set_attr("media_encryption", "sdes") } else { h.remove_attr("media_encryption") }
    }
    Ok(e)
}

pub async fn set_accounts(ctx: Ctx, req: pb::SetAccountsRequest) -> Result<pb::SetAccountsResponse, ConnectError> {
    with_config(&ctx.app, true, move |c| {
        let cfg = c.config();
        let existing = cfg.accounts();
        let max = cfg.max_accounts() as u32;
        let mut seen = std::collections::HashSet::new();
        let mut elements = Vec::new();
        let mut accounts = req.accounts.clone();
        accounts.sort_by_key(|a| a.index);
        for a in &accounts {
            if a.index >= max {
                return Err(ConnectError::invalid_argument(format!("this model has {max} accounts")));
            }
            if !seen.insert(a.index) {
                return Err(ConnectError::invalid_argument(format!("account {} is given twice", a.index + 1)));
            }
            elements.push(account_element(a, existing.iter().find(|e| e.index == a.index))?);
        }
        c.set_local_accounts(&elements).map_err(crate::services::config_error)?;
        Ok(pb::SetAccountsResponse {})
    })
    .await
}

pub async fn clear_local_accounts(
    ctx: Ctx,
    _req: pb::ClearLocalAccountsRequest,
) -> Result<pb::ClearLocalAccountsResponse, ConnectError> {
    with_config(&ctx.app, true, |c| {
        c.clear_local_accounts().map_err(crate::services::config_error)?;
        Ok(pb::ClearLocalAccountsResponse {})
    })
    .await
}

// -------------------------------------------------------------- contacts

fn list_name(name: &str) -> Result<(), ConnectError> {
    let ok = !name.is_empty()
        && name.len() <= 100
        && !name.starts_with('.')
        && !name.ends_with(".tmp")
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if ok { Ok(()) } else { Err(ConnectError::invalid_argument("list names use letters, digits, '-', '_', and '.'")) }
}

fn read_list(c: &ConfigStore, name: &str) -> Result<String, ConnectError> {
    let data = c
        .nvdata()
        .contact_list(name)
        .map_err(|e| ConnectError::internal(e.to_string()))?
        .ok_or_else(|| ConnectError::not_found(format!("no contact list named {name}")))?;
    String::from_utf8(data).map_err(|_| ConnectError::new(crate::rpc::Code::DataLoss, "the list is not text"))
}

pub async fn list_contact_lists(
    ctx: Ctx,
    _req: pb::ListContactListsRequest,
) -> Result<pb::ListContactListsResponse, ConnectError> {
    with_config(&ctx.app, false, |c| {
        let mut names = c.nvdata().contact_list_names().unwrap_or_default();
        names.retain(|n| list_name(n).is_ok());
        names.sort();
        let lists = names
            .into_iter()
            .map(|name| match read_list(c, &name).and_then(|t| parse_contacts(&t).map_err(|e| ConnectError::invalid_argument(e.to_string()))) {
                Ok(groups) => pb::ContactListInfo {
                    name,
                    groups: groups.len() as u32,
                    contacts: groups.iter().map(|g| g.contacts.len() as u32).sum(),
                    error: String::new(),
                },
                Err(e) => pb::ContactListInfo { name, groups: 0, contacts: 0, error: e.message.unwrap_or_default() },
            })
            .collect();
        Ok(pb::ListContactListsResponse { lists })
    })
    .await
}

pub async fn get_contact_list(
    ctx: Ctx,
    req: pb::GetContactListRequest,
) -> Result<pb::GetContactListResponse, ConnectError> {
    list_name(&req.name)?;
    with_config(&ctx.app, false, move |c| {
        let text = read_list(c, &req.name)?;
        let groups = parse_contacts(&text).map_err(|e| ConnectError::new(crate::rpc::Code::DataLoss, e.to_string()))?;
        Ok(pb::GetContactListResponse {
            groups: groups
                .into_iter()
                .map(|g| pb::ContactGroup {
                    id: g.id,
                    name: g.group_name,
                    contacts: g
                        .contacts
                        .into_iter()
                        .map(|k| pb::Contact {
                            id: k.id,
                            first_name: k.first_name,
                            last_name: k.last_name,
                            organization: k.organization,
                            contact_type: k.contact_type,
                            subscribe_to: k.subscribe_to.unwrap_or_default(),
                            numbers: k
                                .numbers
                                .into_iter()
                                .map(|n| pb::ContactNumber { dial: n.dial, label: n.label, primary: n.primary })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        })
    })
    .await
}

/// Serializes groups in the stock shape, reusing stored elements so that
/// attributes this form does not model survive an edit.
fn contacts_document(groups: &[pb::ContactGroup], old: &[dphone_config::contacts::ContactGroup]) -> Result<Element, ConnectError> {
    let mut root = Element::new("phonebooks");
    let mut used_ids = std::collections::HashSet::new();
    for (gi, g) in groups.iter().enumerate() {
        clean("A group name", &g.name, 255)?;
        clean("A group ID", &g.id, 255)?;
        let mut ge = Element::new("contacts");
        ge.set_attr("group_name", &g.name);
        ge.set_attr("editable", "1");
        ge.set_attr("id", if g.id.is_empty() { "local" } else { &g.id });
        let old_group = old.iter().find(|o| o.id == g.id).or(old.get(gi));
        for k in &g.contacts {
            for (what, v) in [
                ("A first name", &k.first_name),
                ("A last name", &k.last_name),
                ("An organization", &k.organization),
                ("A contact ID", &k.id),
                ("A contact type", &k.contact_type),
                ("A presence extension", &k.subscribe_to),
            ] {
                clean(what, v, 255)?;
            }
            if k.first_name.is_empty() && k.last_name.is_empty() && k.organization.is_empty() {
                return Err(ConnectError::invalid_argument("a contact needs a name or an organization"));
            }
            let old_contact =
                old_group.and_then(|og| og.contacts.iter().find(|oc| !k.id.is_empty() && oc.id == k.id));
            let mut ce = old_contact.map(|oc| oc.raw.clone()).unwrap_or_else(|| Element::new("contact"));
            let mut id = k.id.clone();
            if id.is_empty() || !used_ids.insert(id.clone()) {
                // New contact (or a duplicated ID): give it a fresh one.
                id = format!("web-{}", &crate::auth::random_token()[..12]);
                used_ids.insert(id.clone());
            }
            ce.set_attr("id", &id);
            ce.set_attr("first_name", &k.first_name);
            ce.set_attr("last_name", &k.last_name);
            ce.set_attr("organization", &k.organization);
            ce.set_attr("contact_type", if k.contact_type.is_empty() { "sip" } else { &k.contact_type });
            set_or_remove(&mut ce, "subscribe_to", &k.subscribe_to);
            ce.remove_children(|c| c.name == "numbers");
            let mut numbers = Element::new("numbers");
            let primary_index = k.numbers.iter().position(|n| n.primary).unwrap_or(0);
            for (ni, n) in k.numbers.iter().enumerate() {
                clean("A number", &n.dial, 255)?;
                clean("A number label", &n.label, 255)?;
                if n.dial.is_empty() {
                    continue;
                }
                let mut ne = old_contact
                    .and_then(|oc| oc.numbers.iter().find(|on| on.dial == n.dial))
                    .map(|on| on.raw.clone())
                    .unwrap_or_else(|| Element::new("number"));
                ne.set_attr("dial", &n.dial);
                ne.set_attr("label", &n.label);
                if ni == primary_index { ne.set_attr("primary", "1") } else { ne.remove_attr("primary") }
                numbers.children.push(Node::Element(ne));
            }
            ce.children.push(Node::Element(numbers));
            ge.children.push(Node::Element(ce));
        }
        root.children.push(Node::Element(ge));
    }
    Ok(root)
}

pub async fn put_contact_list(
    ctx: Ctx,
    req: pb::PutContactListRequest,
) -> Result<pb::PutContactListResponse, ConnectError> {
    list_name(&req.name)?;
    let app = ctx.app.clone();
    with_config(&ctx.app, false, move |c| {
        let old = read_list(c, &req.name).ok().and_then(|t| parse_contacts(&t).ok()).unwrap_or_default();
        let doc = contacts_document(&req.groups, &old)?;
        c.nvdata()
            .write_contact_list(&req.name, doc.to_document().as_bytes())
            .map_err(|e| ConnectError::internal(e.to_string()))?;
        Ok(())
    })
    .await?;
    app.store.post(Command::ReloadContacts);
    Ok(pb::PutContactListResponse {})
}

pub async fn delete_contact_list(
    ctx: Ctx,
    req: pb::DeleteContactListRequest,
) -> Result<pb::DeleteContactListResponse, ConnectError> {
    list_name(&req.name)?;
    let app = ctx.app.clone();
    with_config(&ctx.app, false, move |c| {
        read_list(c, &req.name)?;
        c.nvdata()
            .remove(&format!("contact_lists/{}", req.name))
            .map_err(|e| ConnectError::internal(e.to_string()))
    })
    .await?;
    app.store.post(Command::ReloadContacts);
    Ok(pb::DeleteContactListResponse {})
}

pub async fn export_contact_list(
    ctx: Ctx,
    req: pb::ExportContactListRequest,
) -> Result<pb::ExportContactListResponse, ConnectError> {
    list_name(&req.name)?;
    with_config(&ctx.app, false, move |c| Ok(pb::ExportContactListResponse { xml: read_list(c, &req.name)? })).await
}

pub async fn import_contact_list(
    ctx: Ctx,
    req: pb::ImportContactListRequest,
) -> Result<pb::ImportContactListResponse, ConnectError> {
    list_name(&req.name)?;
    let groups = parse_contacts(&req.xml).map_err(|e| ConnectError::invalid_argument(format!("not a contact list: {e}")))?;
    if groups.is_empty() {
        return Err(ConnectError::invalid_argument("the document holds no <contacts> group"));
    }
    let contacts = groups.iter().map(|g| g.contacts.len() as u32).sum();
    let app = ctx.app.clone();
    with_config(&ctx.app, false, move |c| {
        c.nvdata().write_contact_list(&req.name, req.xml.as_bytes()).map_err(|e| ConnectError::internal(e.to_string()))
    })
    .await?;
    app.store.post(Command::ReloadContacts);
    Ok(pb::ImportContactListResponse { contacts })
}
