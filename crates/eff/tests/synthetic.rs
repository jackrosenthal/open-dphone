//! Tests that need no vendor files.

use eff::ctrl::{CtrlRecord, Side};
use eff::envelope::{self, EffHeader, EffReader, EffWriter};
use eff::ota::{self, AppHeader, Component, Ota, OtaReader};
use std::io::{Read, Write};

fn sample(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            (x >> 24) as u8
        })
        .collect()
}

#[test]
fn ota_round_trip_and_streaming() {
    let kernel = sample(70_001, 1);
    let rootfs = sample(300_007, 2);
    let image = ota::pack(
        &kernel,
        &rootfs,
        "Fri Nov 13 00:00:00 2026",
        "3_0_0_D60_firmware.bin",
    )
    .unwrap();
    let parsed = Ota::parse(&image).unwrap();
    assert_eq!(parsed.kernel, kernel);
    assert_eq!(parsed.rootfs, rootfs);
    assert_eq!(parsed.header.app.name(), "3_0_0_D60_firmware.bin");

    let mut r = OtaReader::new(&image[..]).unwrap();
    let mut k = Vec::new();
    let mut c = r.next_component().unwrap();
    assert_eq!(c.which(), Component::Kernel);
    c.read_to_end(&mut k).unwrap();
    c.finish().unwrap();
    let mut f = Vec::new();
    let mut c = r.next_component().unwrap();
    c.read_to_end(&mut f).unwrap();
    c.finish().unwrap();
    assert!(r.next_component().is_none());
    r.finish().unwrap();
    assert_eq!((k, f), (kernel, rootfs));

    // A corrupt rootfs byte is caught by the streaming reader too.
    let mut bad = image.clone();
    *bad.last_mut().unwrap() ^= 1;
    let mut r = OtaReader::new(&bad[..]).unwrap();
    r.next_component().unwrap().finish().unwrap();
    assert!(r.next_component().unwrap().finish().is_err());
}

#[test]
fn text_fields_are_limited() {
    assert!(AppHeader::new(b"k", b"r", &"x".repeat(64), "n").is_err());
    assert!(AppHeader::new(b"k", b"r", "d", &"x".repeat(63)).is_ok());
    assert!(AppHeader::new(b"", b"r", "d", "n").is_err());
}

#[test]
fn eff_round_trip_all_tail_lengths() {
    let header = EffHeader::new("D60", "3_0_0").unwrap();
    for extra in [
        0usize,
        1,
        15,
        16,
        17,
        16 * 1024 - 400 - 1,
        16 * 1024 - 400,
        16 * 1024 - 400 + 1,
        40_000,
    ] {
        let mut payload = eff::OTA_MAGIC.to_be_bytes().to_vec();
        payload.extend(sample(extra, extra as u32));
        let file = envelope::pack(&header, &payload, *b"saltsalt").unwrap();
        assert_eq!(&file[..10], b"D60 3_0_0\0");
        assert_eq!((file.len() - 116) % 16, 0);
        let (h, back) = envelope::unpack(&file).unwrap();
        assert_eq!(h, header);
        assert_eq!(back, payload, "tail {extra}");

        // One-byte reads exercise the read-ahead logic.
        let mut r = EffReader::new(OneByte(&file)).unwrap();
        let mut slow = Vec::new();
        r.read_to_end(&mut slow).unwrap();
        assert_eq!(slow, payload);
    }
}

struct OneByte<'a>(&'a [u8]);
impl Read for OneByte<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.0.is_empty() || buf.is_empty() {
            return Ok(0);
        }
        buf[0] = self.0[0];
        self.0 = &self.0[1..];
        Ok(1)
    }
}

#[test]
fn eff_rejects_damage() {
    let header = EffHeader::new("D60", "3_0_0").unwrap();
    let mut payload = eff::OTA_MAGIC.to_be_bytes().to_vec();
    payload.extend(sample(1000, 9));
    let file = envelope::pack(&header, &payload, [7; 8]).unwrap();
    assert!(envelope::unpack(&file[..file.len() - 1]).is_err());
    assert!(envelope::unpack(&file[..file.len() - 16]).is_err());
    let mut bad = file.clone();
    bad[104] ^= 1;
    assert!(envelope::unpack(&bad).is_err());
    let mut bad = file.clone();
    bad[120] ^= 1; // first cipher block: garbles the magic text
    assert!(envelope::unpack(&bad).is_err());
    assert!(EffReader::with_passphrase(&file[..], b"wrong").is_err());
    assert!(EffHeader::new("", "1").is_err());
    assert!(EffHeader::new(&"M".repeat(100), "1").is_err());
}

#[test]
fn d80_has_its_own_passphrase_and_a_zip_payload() {
    let header = EffHeader::new("D80", "1_12_14").unwrap();
    let mut zip = b"PK\x03\x04".to_vec();
    zip.extend(sample(5000, 4));
    let file = envelope::pack(&header, &zip, [3; 8]).unwrap();
    assert_eq!(&file[..12], b"D80 1_12_14\0");
    assert_eq!(
        envelope::unpack(&file).unwrap(),
        (header.clone(), zip.clone())
    );
    assert!(EffReader::with_passphrase(&file[..], eff::D80_OTA_PASSPHRASE).is_ok());
    assert!(EffReader::with_passphrase(&file[..], eff::OTA_PASSPHRASE).is_err());
    let other = envelope::pack(
        &EffHeader::new("D60", "3_0_0").unwrap(),
        &ota_payload(),
        [3; 8],
    )
    .unwrap();
    assert!(EffReader::with_passphrase(&other[..], eff::D80_OTA_PASSPHRASE).is_err());
    // A D80 EFF carries a zip; the other models' an OTA container.
    assert!(envelope::pack(&header, &ota_payload(), [3; 8]).is_err());
    assert!(envelope::pack(&EffHeader::new("D60", "3_0_0").unwrap(), &zip, [3; 8]).is_err());
}

fn ota_payload() -> Vec<u8> {
    let mut payload = eff::OTA_MAGIC.to_be_bytes().to_vec();
    payload.extend(sample(1000, 5));
    payload
}

#[test]
fn eff_writer_streams() {
    let header = EffHeader::new("D40", "3_0_0").unwrap();
    let mut payload = eff::OTA_MAGIC.to_be_bytes().to_vec();
    payload.extend(sample(100_000, 3));
    let mut w = EffWriter::new(Vec::new(), &header, [1; 8]).unwrap();
    for chunk in payload.chunks(777) {
        w.write_all(chunk).unwrap();
    }
    let streamed = w.finish().unwrap();
    assert_eq!(streamed, envelope::pack(&header, &payload, [1; 8]).unwrap());
}

#[test]
fn ctrl_round_trip_and_side_selection() {
    let a = AppHeader::new(b"kernel-a", b"rootfs-a", "date a", "a.bin").unwrap();
    let b = AppHeader::new(b"kernel-b", b"rootfs-b", "date b", "b.bin").unwrap();
    let mut rec = CtrlRecord {
        active: 1,
        sides: [Side::from_header(&a), Side::from_header(&b)],
    };
    let bytes = rec.to_bytes();
    assert_eq!(CtrlRecord::parse(&bytes).unwrap(), rec);
    assert_eq!(rec.boot_side(), 1);
    rec.sides[1] = Side::INVALID;
    assert_eq!(rec.boot_side(), 0);
    let again = CtrlRecord::parse(&rec.to_bytes()).unwrap();
    assert!(!again.sides[1].is_valid());
    assert_eq!(again.sides[0].header().unwrap(), a);
    rec.active = 0;
    rec.sides = [Side::INVALID, Side::from_header(&b)];
    assert_eq!(rec.boot_side(), 1);
    rec.sides = [Side::INVALID, Side::INVALID];
    assert_eq!(rec.boot_side(), 0);
    let mut bad = bytes;
    bad[20] ^= 1;
    assert!(CtrlRecord::parse(&bad).is_err());
}
