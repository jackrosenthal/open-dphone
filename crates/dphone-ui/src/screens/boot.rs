//! Base screens that replace the idle screen: boot progress, recovery,
//! provisioning, and firmware update.

use super::*;
use dphone_state::{NetPhase, Phase, ProvisionPhase, UpdatePhase};

/// Boot and shutdown progress.
pub(crate) struct Boot;

impl Screen for Boot {
    fn name(&self) -> String {
        "boot".into()
    }
    fn wallpaper(&self) -> bool {
        true
    }
    fn softkeys(&self, cx: &Cx) -> Vec<Soft> {
        if cx.st.system.phase == Phase::NetworkWait {
            vec![soft(1, tr::NETWORK), soft(0, ""), soft(0, ""), soft(4, tr::STATUS)]
        } else {
            Vec::new()
        }
    }
    fn softkey(&mut self, _cx: &Cx, id: u32) -> Nav {
        match id {
            1 => Nav::Push(menus::network_menu()),
            4 => Nav::Push(menus::about()),
            _ => Nav::Stay,
        }
    }
    fn key(&mut self, _cx: &Cx, _key: Key) -> Nav {
        Nav::Ignored
    }
    fn draw(&mut self, cx: &Cx, c: &mut Canvas, area: IRect) {
        let net = &cx.st.network;
        let text = match cx.st.system.phase {
            Phase::ShuttingDown => tr::SHUTTING_DOWN.to_string(),
            Phase::NetworkWait => match net.phase {
                NetPhase::Failed if !net.detail.is_empty() => format!("{}: {}", tr::NETWORK_FAILED, net.detail),
                NetPhase::Failed => tr::NETWORK_FAILED.to_string(),
                _ if !net.lan.up => tr::NO_LINK.to_string(),
                NetPhase::Configuring => tr::GETTING_ADDRESS.to_string(),
                _ => tr::WAITING_NETWORK.to_string(),
            },
            _ => tr::STARTING.to_string(),
        };
        splash(c, cx.th, area, true, "", &text, None);
    }
}

/// nvdata could not be mounted.
pub(crate) struct Recovery;

impl Screen for Recovery {
    fn name(&self) -> String {
        "recovery".into()
    }
    fn softkeys(&self, _cx: &Cx) -> Vec<Soft> {
        vec![soft(1, tr::STATUS), soft(2, tr::NETWORK), soft(0, ""), soft(4, tr::REBOOT)]
    }
    fn softkey(&mut self, _cx: &Cx, id: u32) -> Nav {
        match id {
            1 => Nav::Push(menus::about()),
            2 => Nav::Push(menus::network_status()),
            4 => Nav::Push(Confirm::boxed(tr::RESTART_Q, |cx| {
                cx.post(Command::Reboot { reason: "recovery screen".into() });
                Nav::Pop
            })),
            _ => Nav::Stay,
        }
    }
    fn key(&mut self, _cx: &Cx, _key: Key) -> Nav {
        Nav::Ignored
    }
    fn draw(&mut self, cx: &Cx, c: &mut Canvas, area: IRect) {
        let th = cx.th;
        c.fill(area, th.bg);
        let (title, body) = area.split_top(th.title_h);
        widgets::title_bar(c, th, title, tr::RECOVERY_TITLE);
        let mut text = tr::RECOVERY_TEXT.to_string();
        if let Some(ip) = cx.st.network.ipv4.address {
            text = format!("{text} http://{ip}/");
        }
        let f = th.font(if th.gray { Font::Small } else { Font::Normal });
        let lh = if th.gray { th.line_h - 3 } else { th.line_h };
        c.text_wrapped(f, body.inset_xy(th.pad, 2), &text, th.fg, HAlign::Left, lh);
    }
}

/// DPMA provisioning, following `Provisioning::phase`.
#[derive(Default)]
pub(crate) struct Provision {
    servers: ListState,
    users: ListState,
}

fn manual_server() -> Box<dyn Screen> {
    EditScreen::boxed("provision-server", tr::PROV_ENTER_SERVER, "", InputMode::Text, |cx, text| {
        let t = text.trim();
        if !t.is_empty() {
            cx.post(Command::StartProvisioning { server: Some(t.to_string()) });
        }
        Nav::Pop
    })
}

impl Provision {
    fn server_rows(cx: &Cx) -> usize {
        cx.st.provisioning.discovered.len() + 2
    }
    fn pick_server(&self, cx: &Cx) -> Nav {
        let d = &cx.st.provisioning.discovered;
        let i = self.servers.selected;
        if let Some(s) = d.get(i) {
            cx.post(Command::StartProvisioning { server: Some(s.address.clone()) });
            Nav::Stay
        } else if i == d.len() {
            Nav::Push(manual_server())
        } else {
            cx.post(Command::SkipProvisioning);
            Nav::Stay
        }
    }
    fn pick_user(&self, cx: &Cx) -> Nav {
        let Some(u) = cx.st.provisioning.users.get(self.users.selected) else { return Nav::Stay };
        let id = u.id.clone();
        if u.needs_pin {
            Nav::Push(EditScreen::boxed("provision-pin", tr::PROV_ENTER_PIN, "", InputMode::Pin, move |cx, pin| {
                cx.post(Command::SelectUser { id: id.clone(), pin: pin.to_string() });
                Nav::Pop
            }))
        } else {
            cx.post(Command::SelectUser { id, pin: String::new() });
            Nav::Stay
        }
    }
}

impl Screen for Provision {
    fn name(&self) -> String {
        "provisioning".into()
    }
    fn softkeys(&self, cx: &Cx) -> Vec<Soft> {
        match &cx.st.provisioning.phase {
            ProvisionPhase::AwaitingServer | ProvisionPhase::AwaitingUser => {
                vec![soft(1, tr::SELECT), soft(0, ""), soft(0, ""), soft(4, tr::STATUS)]
            }
            ProvisionPhase::Failed { .. } => {
                vec![soft(2, tr::RETRY), soft(3, tr::MANUAL), soft(5, tr::SKIP), soft(4, tr::STATUS)]
            }
            ProvisionPhase::Idle | ProvisionPhase::Discovering => {
                vec![soft(3, tr::MANUAL), soft(0, ""), soft(5, tr::SKIP), soft(4, tr::STATUS)]
            }
            _ => Vec::new(),
        }
    }
    fn softkey(&mut self, cx: &Cx, id: u32) -> Nav {
        match id {
            1 => self.key(cx, Key::Select),
            2 => {
                cx.post(Command::StartProvisioning { server: None });
                Nav::Stay
            }
            3 => Nav::Push(manual_server()),
            4 => Nav::Push(menus::about()),
            5 => {
                cx.post(Command::SkipProvisioning);
                Nav::Stay
            }
            _ => Nav::Stay,
        }
    }
    fn key(&mut self, cx: &Cx, key: Key) -> Nav {
        let (st, len) = match cx.st.provisioning.phase {
            ProvisionPhase::AwaitingServer => (&mut self.servers, Self::server_rows(cx)),
            ProvisionPhase::AwaitingUser => (&mut self.users, cx.st.provisioning.users.len()),
            _ => return Nav::Ignored,
        };
        match key {
            Key::Up => {
                st.step(-1, len, true);
                Nav::Stay
            }
            Key::Down => {
                st.step(1, len, true);
                Nav::Stay
            }
            Key::Select => match cx.st.provisioning.phase {
                ProvisionPhase::AwaitingServer => self.pick_server(cx),
                _ => self.pick_user(cx),
            },
            _ => Nav::Ignored,
        }
    }
    fn draw(&mut self, cx: &Cx, c: &mut Canvas, area: IRect) {
        let th = cx.th;
        let p = &cx.st.provisioning;
        let f = th.font(Font::Normal);
        let listing = |c: &mut Canvas, title: &str, rows: Vec<String>, st: &mut ListState| {
            c.fill(area, th.bg);
            let (t, body) = area.split_top(th.title_h);
            widgets::title_bar(c, th, t, title);
            widgets::list(c, th, widgets::body_rows(th, body), rows.len(), st, th.row_h, true, |c, r, i, _, fg| {
                c.text_in(f, r.inset_xy(th.pad * 2, 0), &rows[i], fg, HAlign::Left);
            });
        };
        match &p.phase {
            ProvisionPhase::AwaitingServer => {
                let mut rows: Vec<String> = p
                    .discovered
                    .iter()
                    .map(|s| if s.name.is_empty() { s.address.clone() } else { format!("{} ({})", s.name, s.address) })
                    .collect();
                rows.push(tr::PROV_ENTER_MANUALLY.into());
                rows.push(tr::PROV_NO_SERVER.into());
                listing(c, tr::PROV_SELECT_SERVER, rows, &mut self.servers);
            }
            ProvisionPhase::AwaitingUser => {
                let rows = p
                    .users
                    .iter()
                    .map(|u| if u.name.is_empty() || u.name == u.id { u.id.clone() } else { format!("{}  {}", u.id, u.name) })
                    .collect();
                listing(c, tr::PROV_SELECT_USER, rows, &mut self.users);
            }
            ProvisionPhase::Failed { reason, code } => {
                let text = match code {
                    Some(code) => format!("{reason} ({code})"),
                    None => reason.clone(),
                };
                splash(c, th, area, false, tr::PROV_FAILED, &text, None);
            }
            other => {
                let text = match other {
                    ProvisionPhase::Handshaking => tr::PROV_CONTACTING,
                    ProvisionPhase::FetchingConfig => tr::PROV_FETCHING,
                    ProvisionPhase::Applying | ProvisionPhase::Provisioned | ProvisionPhase::Standalone => tr::PROV_APPLYING,
                    _ => tr::PROV_DISCOVERING,
                };
                let server = p.server.as_ref().map(|s| s.address.as_str()).unwrap_or("");
                let text = if server.is_empty() { text.to_string() } else { format!("{text}\n{server}") };
                splash(c, th, area, false, "", &text, None);
            }
        }
    }
}

/// Firmware update progress.  Keys are ignored while flashing.
pub(crate) struct UpdateScreen;

/// (label, percent) for an update phase.
pub(crate) fn update_progress(p: &UpdatePhase) -> (&'static str, Option<u32>) {
    match p {
        UpdatePhase::Idle => ("", None),
        UpdatePhase::Receiving { percent } => (tr::UPD_RECEIVING, Some(*percent as u32)),
        UpdatePhase::Checking => (tr::UPD_CHECKING, None),
        UpdatePhase::Erasing { percent } => (tr::UPD_ERASING, Some(*percent as u32)),
        UpdatePhase::WritingKernel { percent } => (tr::UPD_KERNEL, Some(*percent as u32)),
        UpdatePhase::WritingRootfs { percent } => (tr::UPD_ROOTFS, Some(*percent as u32)),
        UpdatePhase::Verifying { percent } => (tr::UPD_VERIFYING, Some(*percent as u32)),
        UpdatePhase::Committing => (tr::UPD_COMMITTING, None),
        UpdatePhase::Complete => (tr::UPD_COMPLETE, Some(100)),
        UpdatePhase::Failed(_) => (tr::UPD_FAILED, None),
    }
}

impl Screen for UpdateScreen {
    fn name(&self) -> String {
        "update".into()
    }
    fn key(&mut self, _cx: &Cx, _key: Key) -> Nav {
        Nav::Stay
    }
    fn hook(&mut self, _cx: &Cx, _off: bool) -> Nav {
        Nav::Stay
    }
    fn draw(&mut self, cx: &Cx, c: &mut Canvas, area: IRect) {
        let u = &cx.st.update;
        let (label, pct) = update_progress(&u.phase);
        let mut text = label.to_string();
        if !u.incoming_version.is_empty() {
            text = format!("{text} ({})", u.incoming_version);
        }
        if !matches!(u.phase, UpdatePhase::Complete) {
            text = format!("{text}\n{}", tr::UPDATE_WARNING);
        }
        splash(c, cx.th, area, false, tr::UPDATE_TITLE, &text, pct);
    }
}
