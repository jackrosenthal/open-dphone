//! State store types to protobuf messages.

use crate::pb;
use dphone_state as st;
use dphone_state::clock::mono_ms;

fn secs_until(mono: u64) -> u32 {
    (mono.saturating_sub(mono_ms()) / 1000).min(u32::MAX as u64) as u32
}

pub fn skin(model: st::Model) -> &'static str {
    match model.skin() {
        st::Skin::SmallGray => "small_gray",
        st::Skin::LargeGray => "large_gray",
        st::Skin::Color => "color",
    }
}

pub fn system(s: &st::System, uptime_secs: u64) -> pb::SystemInfo {
    pb::SystemInfo {
        model: s.model.name().to_owned(),
        version: s.version.dotted(),
        phase: match s.phase {
            st::Phase::Starting => pb::SystemPhase::Starting,
            st::Phase::NetworkWait => pb::SystemPhase::NetworkWait,
            st::Phase::Provisioning => pb::SystemPhase::Provisioning,
            st::Phase::Running => pb::SystemPhase::Running,
            st::Phase::Recovery => pb::SystemPhase::Recovery,
            st::Phase::ShuttingDown => pb::SystemPhase::ShuttingDown,
        } as i32,
        mac: s.mac_string(),
        hostname: s.hostname.clone(),
        time_synced: s.time_synced,
        timezone: s.timezone.clone(),
        last_reboot_reason: s.last_reboot_reason.clone(),
        dev_mode: s.dev_mode,
        simulated: s.simulated,
        mem_total_kib: s.mem_total_kib,
        uptime_secs,
        unix_time: st::clock::unix_secs(),
        skin: skin(s.model).to_owned(),
        user_agent: s.model.user_agent(&s.version),
    }
}

fn link(l: &st::PortLink) -> pb::PortLink {
    pb::PortLink { up: l.up, speed: l.speed as u32, full_duplex: l.full_duplex }
}

pub fn network(n: &st::Network, mac: String) -> pb::NetworkStatus {
    let opt = |a: Option<std::net::Ipv4Addr>| a.map(|a| a.to_string()).unwrap_or_default();
    let lldp = n.lldp.clone().unwrap_or_default();
    pb::NetworkStatus {
        phase: match n.phase {
            st::NetPhase::Down => pb::NetPhase::Down,
            st::NetPhase::Configuring => pb::NetPhase::Configuring,
            st::NetPhase::Up => pb::NetPhase::Up,
            st::NetPhase::Failed => pb::NetPhase::Failed,
        } as i32,
        detail: n.detail.clone(),
        lan: Some(link(&n.lan)),
        pc: Some(link(&n.pc)),
        pc_port_enabled: n.pc_port_enabled,
        mac,
        ipv4_source: match n.ipv4.source {
            st::AddrSource::Dhcp => "dhcp",
            st::AddrSource::Static => "static",
            _ => "",
        }
        .to_owned(),
        ipv4_address: opt(n.ipv4.address),
        ipv4_prefix_len: n.ipv4.prefix_len as u32,
        ipv4_gateway: opt(n.ipv4.gateway),
        lease_secs: n.ipv4.lease_secs,
        dhcp_server: opt(n.ipv4.dhcp_server),
        ipv6_addresses: n.ipv6.iter().map(|a| format!("{}/{}", a.address, a.prefix_len)).collect(),
        dns: n.dns.iter().map(|a| a.to_string()).collect(),
        domain: n.domain.clone(),
        dhcp_tftp_server: n.dhcp_tftp_server.clone(),
        ntp_servers: n.ntp_servers.clone(),
        ntp_synced: n.ntp_synced_mono_ms.is_some(),
        vlan_source: match n.vlan.source {
            st::VlanSource::None => "",
            st::VlanSource::Config => "config",
            st::VlanSource::Lldp => "lldp",
            st::VlanSource::Dhcp => "dhcp",
        }
        .to_owned(),
        voice_vlan: n.vlan.voice_id.map(u32::from),
        pc_vlan: n.vlan.pc_id.map(u32::from),
        vlan_priority: n.vlan.voice_priority as u32,
        dscp: n.vlan.dscp as u32,
        lldp_present: n.lldp.is_some(),
        lldp_system_name: lldp.system_name,
        lldp_port_id: lldp.port_id,
        lldp_chassis_id: lldp.chassis_id,
        lldp_port_description: lldp.port_description,
        profile: n.profile.clone(),
    }
}

fn server(s: &st::DiscoveredServer) -> pb::ProvisioningServer {
    pb::ProvisioningServer {
        source: match s.source {
            st::DiscoverySource::Saved => "saved",
            st::DiscoverySource::DhcpOption66 => "dhcp",
            st::DiscoverySource::Mdns => "mdns",
            st::DiscoverySource::Manual => "manual",
        }
        .to_owned(),
        name: s.name.clone(),
        address: s.address.clone(),
    }
}

pub fn provisioning(p: &st::Provisioning) -> pb::ProvisioningStatus {
    use pb::ProvisionPhase as P;
    let (phase, failure_reason, failure_code) = match &p.phase {
        st::ProvisionPhase::Idle => (P::Idle, String::new(), None),
        st::ProvisionPhase::Discovering => (P::Discovering, String::new(), None),
        st::ProvisionPhase::AwaitingServer => (P::AwaitingServer, String::new(), None),
        st::ProvisionPhase::Handshaking => (P::Handshaking, String::new(), None),
        st::ProvisionPhase::AwaitingUser => (P::AwaitingUser, String::new(), None),
        st::ProvisionPhase::FetchingConfig => (P::FetchingConfig, String::new(), None),
        st::ProvisionPhase::Applying => (P::Applying, String::new(), None),
        st::ProvisionPhase::Provisioned => (P::Provisioned, String::new(), None),
        st::ProvisionPhase::Standalone => (P::Standalone, String::new(), None),
        st::ProvisionPhase::Failed { reason, code } => (P::Failed, reason.clone(), *code),
    };
    pb::ProvisioningStatus {
        phase: phase as i32,
        failure_reason,
        failure_code,
        discovered: p.discovered.iter().map(server).collect(),
        server: p.server.as_ref().map(server),
        server_user_agent: p.server_user_agent.clone(),
        users: p
            .users
            .iter()
            .map(|u| pb::ProvisionUser { id: u.id.clone(), name: u.name.clone(), needs_pin: u.needs_pin })
            .collect(),
        assigned_user: p.assigned_user.clone(),
        attached: p.attached,
        config_md5: p.config_md5.clone(),
        offered_firmware: p.offered_firmware.clone().unwrap_or_default(),
        last_success_unix: p.last_success_unix.unwrap_or(0),
    }
}

pub fn transport(t: st::Transport) -> pb::SipTransport {
    match t {
        st::Transport::Udp => pb::SipTransport::Udp,
        st::Transport::Tcp => pb::SipTransport::Tcp,
        st::Transport::Tls => pb::SipTransport::Tls,
    }
}

pub fn accounts(a: &st::Accounts) -> Vec<pb::AccountStatus> {
    a.accounts
        .iter()
        .map(|acc| {
            use pb::RegistrationState as R;
            let mut out = pb::AccountStatus {
                index: acc.id.0 as u32,
                label: if acc.label.is_empty() { acc.user.clone() } else { acc.label.clone() },
                user: acc.user.clone(),
                server: acc.server.clone(),
                dpma: acc.dpma,
                transport: transport(acc.transport) as i32,
                is_default: a.default_account == Some(acc.id),
                ..Default::default()
            };
            out.state = match &acc.registration {
                st::Registration::Disabled => R::Disabled,
                st::Registration::Unregistered => R::Unregistered,
                st::Registration::Registering => R::Registering,
                st::Registration::Registered { host, expires_mono_ms } => {
                    out.on_alternate_host = *host == st::ActiveHost::Alternate;
                    out.expires_in_secs = secs_until(*expires_mono_ms);
                    R::Registered
                }
                st::Registration::Failed { code, reason, retry_mono_ms } => {
                    out.code = *code as u32;
                    out.reason = reason.clone();
                    out.retry_in_secs = secs_until(*retry_mono_ms);
                    R::Failed
                }
            } as i32;
            out
        })
        .collect()
}

pub fn party(p: &st::Party) -> pb::Party {
    pb::Party { name: p.name.clone(), number: p.number.clone(), uri: p.uri.clone() }
}

pub fn audio_path(p: st::AudioPath) -> pb::AudioPath {
    match p {
        st::AudioPath::None => pb::AudioPath::None,
        st::AudioPath::Handset => pb::AudioPath::Handset,
        st::AudioPath::Headset => pb::AudioPath::Headset,
        st::AudioPath::Speaker => pb::AudioPath::Speaker,
    }
}

pub fn audio_path_from(p: i32) -> Option<st::AudioPath> {
    match pb::AudioPath::try_from(p).ok()? {
        pb::AudioPath::Unspecified => None,
        pb::AudioPath::None => Some(st::AudioPath::None),
        pb::AudioPath::Handset => Some(st::AudioPath::Handset),
        pb::AudioPath::Headset => Some(st::AudioPath::Headset),
        pb::AudioPath::Speaker => Some(st::AudioPath::Speaker),
    }
}

fn end_cause(c: &st::EndCause) -> String {
    match c {
        st::EndCause::LocalHangup => "Hung up".into(),
        st::EndCause::RemoteHangup => "Remote party hung up".into(),
        st::EndCause::Rejected => "Declined".into(),
        st::EndCause::Cancelled { answered_elsewhere: true } => "Answered elsewhere".into(),
        st::EndCause::Cancelled { answered_elsewhere: false } => "Cancelled".into(),
        st::EndCause::Busy => "Busy".into(),
        st::EndCause::NoAnswer => "No answer".into(),
        st::EndCause::Transferred => "Transferred".into(),
        st::EndCause::Failed { code, reason } => format!("{code} {reason}"),
        st::EndCause::Error(e) => e.clone(),
    }
}

pub fn call(c: &st::Call, focused: bool) -> pb::Call {
    use pb::CallState as S;
    let (mut held_locally, mut held_remotely, mut cause) = (false, false, String::new());
    let state = match &c.state {
        st::CallState::Dialing => S::Dialing,
        st::CallState::Proceeding => S::Proceeding,
        st::CallState::Ringback { .. } => S::Ringback,
        st::CallState::Ringing => S::Ringing,
        st::CallState::Active => S::Active,
        st::CallState::Held { local, remote } => {
            held_locally = *local;
            held_remotely = *remote;
            S::Held
        }
        st::CallState::Transferring => S::Transferring,
        st::CallState::Ended(e) => {
            cause = end_cause(e);
            S::Ended
        }
    };
    pb::Call {
        id: c.id.0,
        account: c.account.0 as u32,
        direction: match c.direction {
            st::Direction::Incoming => pb::CallDirection::Incoming,
            st::Direction::Outgoing => pb::CallDirection::Outgoing,
        } as i32,
        state: state as i32,
        remote: Some(party(&c.remote)),
        started_unix: c.started_unix,
        duration_secs: c.duration_secs(mono_ms()),
        codec: c.codec.map(|c| c.name().to_owned()).unwrap_or_default(),
        srtp: c.srtp,
        muted: c.muted,
        in_conference: c.in_conference,
        held_locally,
        held_remotely,
        end_cause: cause,
        focused,
        rx_packets: c.stats.rx_packets,
        tx_packets: c.stats.tx_packets,
        rx_lost: c.stats.rx_lost,
        jitter_ms: c.stats.jitter_ms as u32,
        paging: c.paging,
    }
}

pub fn audio(a: &st::Audio) -> pb::AudioStatus {
    let (dsp, dsp_detail) = match &a.dsp {
        st::DspStatus::NotStarted => ("not_started", String::new()),
        st::DspStatus::Starting => ("starting", String::new()),
        st::DspStatus::Running => ("running", String::new()),
        st::DspStatus::Failed(e) => ("failed", e.clone()),
    };
    pb::AudioStatus {
        path: audio_path(a.path) as i32,
        off_hook: a.off_hook,
        headset_mode: a.headset_mode,
        muted: a.muted,
        handset_volume: a.handset_volume as u32,
        headset_volume: a.headset_volume as u32,
        speaker_volume: a.speaker_volume as u32,
        ringer_volume: a.ringer_volume as u32,
        ringing: a.ringing,
        dsp: dsp.to_owned(),
        dsp_detail,
    }
}

pub fn calls_snapshot(s: &st::State) -> pb::CallsSnapshot {
    pb::CallsSnapshot {
        calls: s.calls.calls.iter().map(|c| call(c, s.calls.focused == Some(c.id))).collect(),
        audio: Some(audio(&s.audio)),
        features: Some(pb::FeatureStatus {
            dnd: s.features.dnd,
            forward_always: s.features.forward.always.clone(),
            forward_busy: s.features.forward.busy.clone(),
            forward_no_answer: s.features.forward.no_answer.clone(),
            forward_no_answer_secs: s.features.forward.no_answer_secs as u32,
        }),
        mwi: s
            .mwi
            .accounts
            .iter()
            .map(|m| pb::MwiStatus {
                account: m.account.0 as u32,
                waiting: m.waiting,
                new_messages: m.new,
                old_messages: m.old,
            })
            .collect(),
        missed_unseen: s.call_log.missed_unseen,
    }
}

pub fn call_log(l: &st::CallLog) -> Vec<pb::CallLogEntry> {
    l.entries
        .iter()
        .map(|e| pb::CallLogEntry {
            id: e.id,
            kind: match e.kind {
                st::CallLogKind::Missed => pb::CallLogKind::Missed,
                st::CallLogKind::Received => pb::CallLogKind::Received,
                st::CallLogKind::Placed => pb::CallLogKind::Placed,
            } as i32,
            party: Some(party(&e.party)),
            account: e.account.0 as u32,
            start_unix: e.start_unix,
            duration_secs: e.duration_secs,
        })
        .collect()
}

pub fn firmware(u: &st::Update, version: &st::FirmwareVersion) -> pb::FirmwareStatus {
    use pb::UpdatePhase as P;
    let (phase, percent, error) = match &u.phase {
        st::UpdatePhase::Idle => (P::Idle, 0, String::new()),
        st::UpdatePhase::Receiving { percent } => (P::Receiving, *percent, String::new()),
        st::UpdatePhase::Checking => (P::Checking, 0, String::new()),
        st::UpdatePhase::Erasing { percent } => (P::Erasing, *percent, String::new()),
        st::UpdatePhase::WritingKernel { percent } => (P::WritingKernel, *percent, String::new()),
        st::UpdatePhase::WritingRootfs { percent } => (P::WritingRootfs, *percent, String::new()),
        st::UpdatePhase::Verifying { percent } => (P::Verifying, *percent, String::new()),
        st::UpdatePhase::Committing => (P::Committing, 0, String::new()),
        st::UpdatePhase::Complete => (P::Complete, 100, String::new()),
        st::UpdatePhase::Failed(e) => (P::Failed, 0, e.clone()),
    };
    let (trial, trial_boots_left, rollback_reason) = match &u.trial {
        st::TrialBoot::None => ("", 0, String::new()),
        st::TrialBoot::Pending { boots_left } => ("pending", *boots_left as u32, String::new()),
        st::TrialBoot::RolledBack { reason } => ("rolled_back", 0, reason.clone()),
    };
    pb::FirmwareStatus {
        phase: phase as i32,
        percent: percent as u32,
        error,
        running_version: version.dotted(),
        running_bank: match u.running_bank {
            Some(st::Bank::A) => "A",
            Some(st::Bank::B) => "B",
            None => "",
        }
        .to_owned(),
        trial: trial.to_owned(),
        trial_boots_left,
        rollback_reason,
        incoming_version: u.incoming_version.clone(),
        incoming_is_stock: u.incoming_is_stock,
        uploaded: None,
    }
}

pub fn health(h: &st::Health) -> Vec<pb::SubsystemHealth> {
    st::Subsystem::ALL
        .iter()
        .map(|s| {
            use pb::HealthState as H;
            let (state, detail) = match h.status(*s) {
                st::HealthStatus::Stopped => (H::Stopped, String::new()),
                st::HealthStatus::Starting => (H::Starting, String::new()),
                st::HealthStatus::Ok => (H::Ok, String::new()),
                st::HealthStatus::Degraded(d) => (H::Degraded, d.clone()),
                st::HealthStatus::Failed(d) => (H::Failed, d.clone()),
            };
            pb::SubsystemHealth { subsystem: format!("{s:?}").to_lowercase(), state: state as i32, detail }
        })
        .collect()
}

pub fn apps(a: &st::Apps) -> pb::AppsSnapshot {
    pb::AppsSnapshot {
        apps: a
            .installed
            .iter()
            .map(|i| pb::App {
                name: i.name.clone(),
                id: i.id.clone(),
                display_name: if i.display_name.is_empty() { i.name.clone() } else { i.display_name.clone() },
                description: i.description.clone(),
                kind: i.kind.clone(),
                has_idle: i.has_idle,
                hidden: i.hidden,
                builtin: i.builtin,
                running: i.running,
                foreground: a.foreground.as_deref() == Some(i.name.as_str()),
            })
            .collect(),
        foreground: a.foreground.clone().unwrap_or_default(),
        idle_app: a.idle_app.clone().unwrap_or_default(),
    }
}

pub fn log_level(l: st::Level) -> pb::LogLevel {
    match l {
        st::Level::Error => pb::LogLevel::Error,
        st::Level::Warn => pb::LogLevel::Warn,
        st::Level::Info => pb::LogLevel::Info,
        st::Level::Debug => pb::LogLevel::Debug,
        st::Level::Trace => pb::LogLevel::Trace,
    }
}

pub fn log_entry(e: &st::LogEntry) -> pb::LogEntry {
    pb::LogEntry {
        seq: e.seq,
        unix_ms: e.unix_ms,
        mono_ms: e.mono_ms,
        level: log_level(e.level) as i32,
        target: e.target.clone(),
        message: e.message.clone(),
    }
}

pub fn lamp_color(c: st::LampColor) -> &'static str {
    match c {
        st::LampColor::Off => "off",
        st::LampColor::Red => "red",
        st::LampColor::Green => "green",
        st::LampColor::Amber => "amber",
    }
}

pub fn lamp_blink(b: st::Blink) -> &'static str {
    match b {
        st::Blink::Solid => "solid",
        st::Blink::Slow => "slow",
        st::Blink::Fast => "fast",
        st::Blink::Wink => "wink",
    }
}
