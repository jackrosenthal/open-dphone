//! Checks against the vendor images, `test-inputs/vendor` (see the
//! `test-inputs` crate).  With the feature `test-inputs/required` (the Nix
//! check), the D40 and D60 images must be there; any others in the
//! directory are exercised too.

use eff::envelope::{self, EffReader, EffWriter};
use eff::ota::{self, Ota, OtaHeader, OtaReader};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn vendor_dir() -> Option<PathBuf> {
    test_inputs::input("vendor")
}

fn vendor_required() -> bool {
    test_inputs::REQUIRED
}

fn d60_ota(dir: &Path) -> Vec<u8> {
    let eff = std::fs::read(dir.join("D60/2_9_28_D60_firmware.eff")).expect("D60/2_9_28_D60_firmware.eff");
    envelope::unpack(&eff).unwrap().1
}

/// Every vendor EFF found, in the model directories.
fn vendor_effs(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(models) = std::fs::read_dir(dir) {
        for m in models.flatten() {
            if let Ok(files) = std::fs::read_dir(m.path()) {
                for f in files.flatten() {
                    if f.path().extension().is_some_and(|e| e == "eff") {
                        out.push(f.path());
                    }
                }
            }
        }
    }
    out.sort();
    out
}

#[test]
fn vendor_crcs_and_boundaries() {
    let Some(dir) = vendor_dir() else { return };
    let data = d60_ota(&dir);
    let o = Ota::parse(&data).unwrap();
    assert_eq!((o.kernel.len(), o.rootfs.len()), (1_961_984, 30_000_456));
    assert_eq!(&o.rootfs[..2], &[0x85, 0x19]);
    assert_eq!(o.header.app.crc(), 0x659b_259a);
    assert_eq!(o.header.app.name(), "2_9_28_D60_firmware.bin");
    assert_eq!(o.header.app.date(), "Tue Oct 27 13:05:08 2026");
}

#[test]
fn corruption_rejected() {
    let Some(dir) = vendor_dir() else { return };
    let original = d60_ota(&dir);
    for offset in [0, 4, 8, 12, 28, 156, 160, original.len() - 1] {
        let mut bad = original.clone();
        bad[offset] ^= 1;
        assert!(Ota::parse(&bad).is_err(), "offset {offset}");
    }
    assert!(Ota::parse(&original[..159]).is_err());
    assert!(Ota::parse(&original[..original.len() - 1]).is_err());
    let mut longer = original.clone();
    longer.push(0);
    assert!(Ota::parse(&longer).is_err());
}

#[test]
fn pack_modified_components() {
    let Some(dir) = vendor_dir() else { return };
    let original = d60_ota(&dir);
    let o = Ota::parse(&original).unwrap();
    // Unchanged repack is byte-identical.
    assert_eq!(
        ota::pack_with_header(o.header.app, o.kernel, o.rootfs).unwrap(),
        original
    );
    assert_eq!(
        ota::pack(
            o.kernel,
            o.rootfs,
            &o.header.app.date(),
            &o.header.app.name()
        )
        .unwrap(),
        original
    );
    // Changed lengths, content, and text.
    let mut kernel = o.kernel.to_vec();
    kernel.extend_from_slice(b"new bytes");
    let rootfs = &o.rootfs[..o.rootfs.len() - 32];
    let modified = ota::pack(&kernel, rootfs, "test date", "modified.bin").unwrap();
    let m = Ota::parse(&modified).unwrap();
    assert_eq!((m.kernel, m.rootfs), (&kernel[..], rootfs));
    assert_eq!(m.header.app.date(), "test date");
    assert_eq!(m.header.app.name(), "modified.bin");
}

#[test]
fn every_vendor_eff_round_trips_byte_for_byte() {
    let Some(dir) = vendor_dir() else { return };
    let files = vendor_effs(&dir);
    let mut models = Vec::new();
    for path in &files {
        let original = std::fs::read(path).unwrap();
        // Streaming decrypt.
        let mut r = EffReader::new(&original[..]).unwrap();
        let header = r.header().clone();
        let salt = *r.salt();
        let label = *r.label();
        assert_eq!(
            label,
            header.label(),
            "{}: label is '<model> <version>'",
            path.display()
        );
        let mut payload = Vec::new();
        r.read_to_end(&mut payload).unwrap();
        // OTA: full parse, streaming parse, exact rebuild.  (The D80's
        // payload is an Android OTA package instead.)
        let parts = if envelope::is_d80(&header.model) {
            assert_eq!(&payload[..4], b"PK\x03\x04", "{}", path.display());
            format!("Android OTA package {}", payload.len())
        } else {
            let o = Ota::parse(&payload).unwrap();
            assert_eq!(
                ota::pack_with_header(o.header.app, o.kernel, o.rootfs).unwrap(),
                payload
            );
            let mut s = OtaReader::new(&payload[..]).unwrap();
            s.next_component().unwrap().finish().unwrap();
            s.next_component().unwrap().finish().unwrap();
            s.finish().unwrap();
            format!("kernel {} rootfs {}", o.kernel.len(), o.rootfs.len())
        };
        // EFF: exact rebuild with the saved salt, through the streaming writer.
        let mut w = EffWriter::new(Vec::with_capacity(original.len()), &header, salt).unwrap();
        w.write_all(&payload).unwrap();
        let rebuilt = w.finish().unwrap();
        assert!(
            rebuilt == original,
            "{}: rebuilt EFF differs",
            path.display()
        );
        eprintln!(
            "{}: {} {} {parts} ok",
            path.display(),
            header.model,
            header.version
        );
        models.push(header.model);
    }
    assert!(models.iter().any(|m| m == "D60"));
    if vendor_required() {
        assert!(models.iter().any(|m| m == "D40"), "no D40 image: {models:?}");
    }
}

#[test]
fn header_only_parse() {
    let Some(dir) = vendor_dir() else { return };
    let data = d60_ota(&dir);
    let h = OtaHeader::parse(data[..160].try_into().unwrap()).unwrap();
    assert_eq!(h.total_len(), data.len() as u64);
    assert_eq!(h.to_bytes(), data[..160]);
}
