//! The updater against the HAL's simulated flash at the phone's real
//! geometry, and (when `DPHONE_VENDOR_DIR` points at the reverse-engineering
//! directory holding the D60 NAND dump and vendor EFFs) against the real
//! flash contents, compared with what the stock updater wrote.

use std::path::PathBuf;

use dphone_hal::Hal;
use dphone_state::{Bank, Model};
use dphone_update::{Updater, boot2, ctrl, inspect, trial};
use eff::ctrl::{CtrlRecord, Side};
use eff::EffHeader;

const BLOCK: usize = 128 * 1024;
const PAGE: usize = 2048;

fn prng(seed: u32, len: usize) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 8) as u8
        })
        .collect()
}

fn kernel(seed: u32, len: usize) -> Vec<u8> {
    let mut k = prng(seed, len);
    k[0x24..0x28].copy_from_slice(&boot2::ZIMAGE_MAGIC.to_le_bytes());
    k[0x28..0x2c].copy_from_slice(&0x0200_0000u32.to_le_bytes());
    k[0x2c..0x30].copy_from_slice(&(0x0200_0000u32 + len as u32).to_le_bytes());
    k
}

#[test]
fn install_on_the_hal_sim_at_real_geometry() {
    for model in [Model::D60, Model::D40] {
        let mut hal = Hal::sim(model);
        let sim = hal.sim.clone().expect("sim handle");
        let name = model.to_string();

        // Bank A: a running image; bank B: empty.
        let (ka, ra) = (kernel(1, 1_500_000), prng(2, 3 * BLOCK + 17));
        let ota_a = eff::ota::pack(&ka, &ra, "old", &format!("2_9_28_{name}_firmware.bin")).unwrap();
        sim.mtd_load("k0", &ka).unwrap();
        sim.mtd_load("root0", &ra).unwrap();
        let rec = CtrlRecord {
            active: 0,
            sides: [Side::from_header(&eff::ota::Ota::parse(&ota_a).unwrap().header.app), Side::INVALID],
        };
        let repl = dphone_update::replication_for(model.family());
        ctrl::write(hal.mtd.open("ctrl").unwrap().as_mut(), &rec, repl).unwrap();

        // New image near the kernel slot's limit with two bad blocks in it.
        let kn = kernel(3, 2 * 1024 * 1024 - 2 * BLOCK - 4096);
        let mut rn = prng(4, 37 * BLOCK + 5000);
        rn[5 * BLOCK..7 * BLOCK].fill(0xff);
        let ota = eff::ota::pack(&kn, &rn, "new", &format!("3_0_0_{name}_firmware.bin")).unwrap();
        let eff = eff::envelope::pack(&EffHeader::new(&name, "3_0_0").unwrap(), &ota, [7; 8]).unwrap();
        sim.mtd_set_bad("k1", 0, true);
        sim.mtd_set_bad("k1", 9, true);
        sim.mtd_set_bad("root1", 2, true);
        sim.mtd_set_bad("ctrl", 2, true);

        let cmdline = "console=ttyAMA0,115200n8 root=/dev/mtdblock6 rootfstype=jffs2 ro";
        let done = Updater::new(hal.mtd.as_mut(), model, cmdline, tempfile::tempdir().unwrap().path()).install_eff(&eff[..]).unwrap();
        assert_eq!((done.bank, done.previous), (Bank::B, Bank::A));

        // Independent reading of the raw partition contents.
        let k1 = sim.mtd_contents("k1").unwrap();
        let mut got = Vec::new();
        for b in (0..16).filter(|b| *b != 0 && *b != 9) {
            got.extend_from_slice(&k1[b * BLOCK..(b + 1) * BLOCK]);
        }
        assert_eq!(&got[..kn.len()], &kn[..]);
        assert!(got[kn.len()..].iter().all(|b| *b == 0xff));
        let r1 = sim.mtd_contents("root1").unwrap();
        let mut got = Vec::new();
        for b in (0..r1.len() / BLOCK).filter(|b| *b != 2) {
            got.extend_from_slice(&r1[b * BLOCK..(b + 1) * BLOCK]);
        }
        assert_eq!(&got[..rn.len()], &rn[..]);
        assert!(got[rn.len()..].iter().all(|b| *b == 0xff));

        let c = sim.mtd_contents("ctrl").unwrap();
        let rec = CtrlRecord::parse(&c[..304]).unwrap();
        assert_eq!(rec.active, 1);
        assert_eq!(rec.sides[1].header().unwrap().name(), format!("3_0_0_{name}_firmware.bin"));
        let copies = (0..c.len() / PAGE).filter(|p| c[p * PAGE..p * PAGE + 304] == c[..304]).count();
        match model {
            Model::D60 => assert_eq!(copies, 3 * 64),
            _ => assert_eq!(copies, 3),
        }

        let boot = boot2::simulate(hal.mtd.as_mut()).unwrap();
        assert_eq!(boot.side, Bank::B);
        assert_eq!(boot2::load_kernel(hal.mtd.as_mut(), Bank::B).unwrap().unwrap(), kn);

        let status = inspect(hal.mtd.as_mut(), cmdline, None).unwrap();
        assert_eq!((status.running, status.next_boot), (Some(Bank::A), Bank::B));
        assert_eq!(status.banks[1].kernel_bad_blocks, 2);

        // And back.
        trial::rollback(hal.mtd.as_mut(), model, "root=/dev/mtdblock7", Bank::A).unwrap();
        assert_eq!(boot2::simulate(hal.mtd.as_mut()).unwrap().side, Bank::A);
    }
}

fn vendor_dir() -> Option<PathBuf> {
    match std::env::var_os("DPHONE_VENDOR_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None => {
            eprintln!("skipped: set DPHONE_VENDOR_DIR to the directory holding dumps/d60-20261108 and the vendor EFFs");
            None
        }
    }
}

/// The D60 dump has, in bank 0, the image `2_9_28_D60_firmware.eff` as the
/// **stock** updater wrote it.  Installing that same EFF into bank 1 of a
/// simulated flash loaded from the dump must reproduce stock's bytes: the
/// kernel and root partitions in full, and the bank's header in `ctrl`.
#[test]
fn installing_the_vendor_eff_reproduces_what_stock_wrote() {
    let Some(dir) = vendor_dir() else { return };
    let dump = dir.join("dumps/d60-20261108/nand-data.bin");
    let eff_path = dir.join("2_9_28_D60_firmware.eff");
    if !dump.exists() || !eff_path.exists() {
        eprintln!("skipped: {} or {} is missing", dump.display(), eff_path.display());
        return;
    }
    let nand = std::fs::read(&dump).unwrap();
    let mut hal = Hal::sim(Model::D60);
    let sim = hal.sim.clone().unwrap();
    // The dump's partition table equals the simulator's (checked against
    // the dump's size and the ctrl contents below).
    let mut off = 0usize;
    let mut at = std::collections::HashMap::new();
    for p in hal.mtd.partitions().unwrap() {
        at.insert(p.name.clone(), (off, p.size as usize));
        sim.mtd_load(&p.name, &nand[off..off + p.size as usize]).unwrap();
        off += p.size as usize;
    }
    let part = |n: &str| {
        let (o, s) = at[n];
        &nand[o..o + s]
    };
    let before = CtrlRecord::parse(&part("ctrl")[..304]).expect("dump ctrl record");
    assert_eq!(before.active, 0);

    let cmdline = "console=ttyAMA0,115200n8 vmalloc=256m  digium_board=D60 digium_boot_rev=3 \
                   root=/dev/mtdblock6 rootfstype=jffs2 ro bootmemheap";
    let status = inspect(hal.mtd.as_mut(), cmdline, None).unwrap();
    assert!(status.banks[0].bootable() && status.banks[1].bootable(), "{status:?}");
    assert_eq!(status.ctrl_copies, (4, 4, 0));

    let file = std::io::BufReader::new(std::fs::File::open(&eff_path).unwrap());
    let done = Updater::new(hal.mtd.as_mut(), Model::D60, cmdline, tempfile::tempdir().unwrap().path()).install_eff(file).unwrap();
    assert_eq!((done.bank, done.is_stock, done.version.as_str()), (Bank::B, true, "2_9_28"));

    // Byte for byte what stock's updater left in bank 0.
    assert!(sim.mtd_contents("k1").unwrap() == part("k0"), "kernel partition differs from stock's");
    assert!(sim.mtd_contents("root1").unwrap() == part("root0"), "root partition differs from stock's");
    let c = sim.mtd_contents("ctrl").unwrap();
    let after = CtrlRecord::parse(&c[..304]).unwrap();
    assert_eq!(after.active, 1);
    assert_eq!(after.sides[1].raw, before.sides[0].raw, "header as stock stored it");
    assert_eq!(after.sides[0].raw, before.sides[0].raw);
    // Stock's layout on the new family: the record in every page, 0xff
    // padded (the dump has 256 such pages).
    for p in 0..c.len() / PAGE {
        assert!(c[p * PAGE..(p + 1) * PAGE] == c[..PAGE], "ctrl page {p}");
        assert!(part("ctrl")[p * PAGE + 304..(p + 1) * PAGE].iter().all(|b| *b == 0xff));
    }
    assert!(c[304..PAGE].iter().all(|b| *b == 0xff));
    // The record CRC stock computed for "active = 1" on the live phone.
    assert_eq!(&c[300..304], &0x4ab36360u32.to_be_bytes());
    assert_eq!(boot2::simulate(hal.mtd.as_mut()).unwrap().side, Bank::B);
}

/// Old family: our writer's layout against the D40 and D50 `ctrl` dumps
/// (one copy at the start of each block).
#[test]
fn old_family_ctrl_layout_matches_the_dumps() {
    let Some(dir) = vendor_dir() else { return };
    for (sub, model) in [("dumps/d40-20261112/ctrl.bin", Model::D40), ("dumps/d50-20261112/ctrl.bin", Model::D50)] {
        let path = dir.join(sub);
        if !path.exists() {
            eprintln!("skipped: {} is missing", path.display());
            continue;
        }
        let dump = std::fs::read(&path).unwrap();
        let rec = CtrlRecord::parse(&dump[..304]).unwrap();
        let mut hal = Hal::sim(model);
        let sim = hal.sim.clone().unwrap();
        let repl = dphone_update::replication_for(model.family());
        ctrl::write(hal.mtd.open("ctrl").unwrap().as_mut(), &rec, repl).unwrap();
        assert!(sim.mtd_contents("ctrl").unwrap() == dump, "{sub}: our ctrl image differs from stock's");
    }
}
