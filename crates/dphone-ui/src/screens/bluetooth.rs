//! The Bluetooth application (D65): on/off, the pairing question, the
//! connected and paired headsets, and discovery.
//!
//! Everything shown comes from the `Bluetooth` state topic, and everything
//! done is a command of the Bluetooth domain; the screens keep no state of
//! their own apart from the PIN being typed.

use super::*;
use crate::strings as tr;
use dphone_state::{BluetoothDevice, Command};

fn label(d: &BluetoothDevice) -> String {
    if d.name.is_empty() { d.addr.clone() } else { d.name.clone() }
}

/// The application's main screen.
pub(crate) fn menu() -> Box<dyn Screen> {
    Menu::boxed("bluetooth", tr::BLUETOOTH, |cx| {
        let bt = &cx.st.bluetooth;
        let mut items = Vec::new();
        if !bt.present {
            items.push(Item::new(tr::BT_NO_CONTROLLER, |_| Nav::Stay));
            if !bt.error.is_empty() {
                items.push(Item::new(bt.error.clone(), |_| Nav::Stay));
            }
            return items;
        }
        // A pairing that waits for an answer comes first.
        if let Some(q) = &bt.pairing {
            let who = if q.name.is_empty() { q.addr.clone() } else { q.name.clone() };
            let addr = q.addr.clone();
            if q.pin_needed {
                items.push(Item::new(format!("{} {who}", tr::BT_ENTER_PIN), move |_| Nav::Push(Box::new(PinEntry::new(addr.clone())))));
            } else {
                // Six digits to compare with the other device's display.
                let digits = q.passkey.map(|p| format!("{p:06}")).unwrap_or_default();
                items.push(
                    Item::new(format!("{} {who}", tr::BT_PAIR_WITH), move |cx| {
                        cx.post(Command::BluetoothAnswerPairing { addr: addr.clone(), accept: true, pin: None });
                        Nav::Stay
                    })
                    .value(digits),
                );
            }
            let addr = q.addr.clone();
            items.push(Item::new(tr::BT_REFUSE, move |cx| {
                cx.post(Command::BluetoothAnswerPairing { addr: addr.clone(), accept: false, pin: None });
                Nav::Stay
            }));
        }
        let on = bt.enabled;
        items.push(
            Item::new(tr::BLUETOOTH, move |cx| {
                cx.post(Command::SetBluetooth { on: !on });
                Nav::Stay
            })
            .value(if on { tr::ON } else { tr::OFF }),
        );
        if !on {
            return items;
        }
        for d in &bt.paired {
            let connected = d.addr == bt.connected;
            let device = d.clone();
            items.push(
                Item::new(label(d), move |_| Nav::Push(paired(device.clone())))
                    .value(if connected { tr::BT_CONNECTED } else { tr::BT_PAIRED }),
            );
        }
        items.push(Item::new(tr::BT_ADD_DEVICE, |cx| {
            cx.post(Command::BluetoothDiscovery { on: true });
            Nav::Push(discovery())
        }));
        if !bt.error.is_empty() {
            items.push(Item::new(bt.error.clone(), |_| Nav::Stay));
        }
        items
    })
}

/// What can be done with one paired device.
fn paired(device: BluetoothDevice) -> Box<dyn Screen> {
    let title = label(&device);
    Menu::boxed("bluetooth-device", title, move |cx| {
        let addr = device.addr.clone();
        let connected = cx.st.bluetooth.connected == addr;
        let mut items = Vec::new();
        if connected {
            items.push(Item::new(tr::BT_DISCONNECT, |cx| {
                cx.post(Command::BluetoothDisconnect);
                Nav::Pop
            }));
        } else {
            let a = addr.clone();
            items.push(Item::new(tr::BT_CONNECT, move |cx| {
                cx.post(Command::BluetoothConnect { addr: a.clone() });
                Nav::Pop
            }));
        }
        let name = label(&device);
        items.push(Item::new(tr::BT_FORGET, move |_| {
            let a = addr.clone();
            Nav::Push(Confirm::boxed(format!("{} {name}?", tr::BT_FORGET), move |cx| {
                cx.post(Command::BluetoothUnpair { addr: a.clone() });
                Nav::Home
            }))
        }));
        items
    })
}

/// Devices in range; selecting one pairs with it.
fn discovery() -> Box<dyn Screen> {
    Menu::boxed("bluetooth-discovery", tr::BT_ADD_DEVICE, |cx| {
        let bt = &cx.st.bluetooth;
        let mut items: Vec<Item> = bt
            .found
            .iter()
            .filter(|d| !bt.paired.iter().any(|p| p.addr == d.addr))
            .map(|d| {
                let addr = d.addr.clone();
                let pairing = bt.pairing_with == d.addr;
                Item::new(label(d), move |cx| {
                    cx.post(Command::BluetoothDiscovery { on: false });
                    cx.post(Command::BluetoothPair { addr: addr.clone() });
                    // Back to the main screen, where the pairing question
                    // and the result appear.
                    Nav::Pop
                })
                .value(if pairing { tr::BT_PAIRING } else { "" })
            })
            .collect();
        if bt.discovering {
            items.push(Item::new(tr::BT_SEARCHING, |_| Nav::Stay));
        } else {
            items.push(Item::new(tr::BT_SEARCH_AGAIN, |cx| {
                cx.post(Command::BluetoothDiscovery { on: true });
                Nav::Stay
            }));
        }
        items
    })
}

/// Typing the PIN of a device that pairs the old way.
pub(crate) struct PinEntry {
    addr: String,
    pin: String,
}

impl PinEntry {
    pub fn new(addr: String) -> PinEntry {
        PinEntry { addr, pin: String::new() }
    }
    fn submit(&self, cx: &Cx) -> Nav {
        if self.pin.is_empty() {
            return Nav::Stay;
        }
        cx.post(Command::BluetoothAnswerPairing { addr: self.addr.clone(), accept: true, pin: Some(self.pin.clone()) });
        Nav::Pop
    }
}

impl Screen for PinEntry {
    fn name(&self) -> String {
        "bluetooth-pin".into()
    }
    fn softkeys(&self, _cx: &Cx) -> Vec<Soft> {
        vec![soft(1, tr::OK), soft(0, ""), soft(3, tr::DELETE), soft(4, tr::BACK)]
    }
    fn softkey(&mut self, cx: &Cx, id: u32) -> Nav {
        match id {
            1 => self.submit(cx),
            3 => {
                self.pin.pop();
                Nav::Stay
            }
            _ => Nav::Pop,
        }
    }
    fn key(&mut self, cx: &Cx, key: Key) -> Nav {
        match key {
            Key::Digit(d) if d <= 9 && self.pin.len() < 16 => {
                self.pin.push((b'0' + d) as char);
                Nav::Stay
            }
            Key::Select => self.submit(cx),
            Key::Cancel => Nav::Pop,
            _ => Nav::Ignored,
        }
    }
    fn dirty(&self) -> bool {
        true
    }
    fn draw(&mut self, cx: &Cx, c: &mut Canvas, area: IRect) {
        let th = cx.th;
        c.fill(area, th.bg);
        let f = th.font(Font::Normal);
        let inner = area.inset(th.pad * 2);
        let text = format!("{}\n{}", tr::BT_ENTER_PIN, self.pin);
        let lines = 2;
        let y = inner.y + ((inner.h - lines * th.line_h) / 2).max(0);
        c.text_wrapped(f, IRect::new(inner.x, y, inner.w, inner.bottom() - y), &text, th.fg, HAlign::Center, th.line_h);
    }
}
