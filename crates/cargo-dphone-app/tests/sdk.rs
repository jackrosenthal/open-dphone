//! The manifest `dphone-app-sdk` writes at compile time is the one this
//! crate reads: the two are written by hand against the same
//! `manifest.proto`, and this holds them to each other.

use cargo_dphone_app::{AppType, Manifest};
use dphone_app_sdk::manifest::{AppKind, Manifest as Written};

/// A module that holds nothing but the section.
fn module(section: &[u8]) -> Vec<u8> {
    let name = dphone_app_sdk::manifest::SECTION.as_bytes();
    assert_eq!(name, cargo_dphone_app::manifest::SECTION.as_bytes());
    let mut body = vec![name.len() as u8];
    body.extend_from_slice(name);
    body.extend_from_slice(section);
    assert!(body.len() < 128 * 128);
    let mut m = b"\0asm\x01\0\0\0\0".to_vec();
    if body.len() < 128 {
        m.push(body.len() as u8);
    } else {
        m.extend([(body.len() % 128) as u8 | 0x80, (body.len() / 128) as u8]);
    }
    m.extend(body);
    m
}

#[test]
fn every_field_arrives() {
    const W: Written = Written {
        name: "probe",
        id: "org.example.probe",
        display_name: "Probe",
        display_name_in: &[("de_de", "Sonde"), ("en_us", "The Probe")],
        description: "Looks around",
        description_in: &[("de_de", "Sieht sich um")],
        kind: AppKind::Background,
        has_idle: true,
        hidden: true,
        icon: "pic.png",
        memory_kib: 8192,
        files: &[("pic.png", &[1, 2, 3]), ("tone.wav", &[0; 300])],
    }
    .checked();
    const LEN: usize = W.encoded_len();
    const SECTION: [u8; LEN] = W.encode::<LEN>();

    let (m, files) = Manifest::from_wasm(&module(&SECTION)).unwrap();
    assert_eq!(m.name, "probe");
    assert_eq!(m.id, "org.example.probe");
    // As stock: the `en_us` text before the unlocalized one.
    assert_eq!(m.display_name, "The Probe");
    assert_eq!(m.display_names.text, "Probe");
    assert_eq!(
        m.display_names.localized[0],
        ("de_de".into(), "Sonde".into())
    );
    assert_eq!(m.description, "Looks around");
    assert_eq!(m.descriptions.localized.len(), 1);
    assert_eq!(m.kind, AppType::Background);
    assert!(m.has_idle && m.hidden);
    assert_eq!(m.icon, "pic.png");
    assert_eq!(m.memory_kib, 8192);
    assert_eq!(m.wasm, "probe.wasm");
    assert!(m.capabilities.is_empty());
    assert_eq!(files.len(), 2);
    assert_eq!(files[0], ("pic.png".to_string(), vec![1, 2, 3]));
    assert_eq!(files[1].1.len(), 300);
    // What this crate writes for the same manifest is the same bytes.
    assert_eq!(m.section(&files), SECTION);
}

#[test]
fn the_least_manifest() {
    const W: Written = Written {
        name: "hello",
        display_name: "Hello",
        ..Written::EMPTY
    }
    .checked();
    const LEN: usize = W.encoded_len();
    let (m, files) = Manifest::from_wasm(&module(&W.encode::<LEN>())).unwrap();
    assert_eq!(
        (m.name.as_str(), m.display_name.as_str()),
        ("hello", "Hello")
    );
    assert_eq!(m.kind, AppType::Foreground);
    assert!(!m.has_idle && !m.hidden && files.is_empty());
}
