//! Structure checks against the stock root images.  Set `DPHONE_VENDOR_DIR`
//! (see the `eff` crate's vendor tests); without it these pass with a notice,
//! unless `DPHONE_TEST_REQUIRE_VENDOR` is set (the Nix check sets it), in
//! which case they fail.

use jffs2_image::{Kind, Options, read_image, write_image};
use std::path::PathBuf;

fn stock_images() -> Vec<PathBuf> {
    let required = std::env::var_os("DPHONE_TEST_REQUIRE_VENDOR").is_some();
    let Some(dir) = std::env::var_os("DPHONE_VENDOR_DIR").map(PathBuf::from) else {
        assert!(
            !required,
            "DPHONE_TEST_REQUIRE_VENDOR is set, but DPHONE_VENDOR_DIR is not"
        );
        eprintln!(
            "SKIPPED: set DPHONE_VENDOR_DIR to the directory with the vendor firmware to run this test"
        );
        return Vec::new();
    };
    let mut out = vec![dir.join("ota-extracted-d60/rootfs.jffs2")];
    for m in std::fs::read_dir(dir.join("firmware-series"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let p = m.path().join("rootfs.jffs2");
        if p.exists() {
            out.push(p);
        }
    }
    out.sort();
    if required {
        // The D60's is first in the list; the D40's is the other one that
        // must be there.
        let d40 = dir.join("firmware-series/D40/rootfs.jffs2");
        assert!(out.contains(&d40), "no D40 root image: {out:?}");
    }
    out
}

#[test]
fn stock_images_read_and_rewrite() {
    for path in stock_images() {
        let image = std::fs::read(&path).unwrap();
        let (tree, stats) =
            read_image(&image, 0x20000).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        eprintln!(
            "{}: {} entries; {stats:?}",
            path.display(),
            tree.entries.len()
        );
        // The properties our writer copies.
        assert_eq!(stats.cleanmarkers, 0);
        assert_eq!(stats.summary_nodes, 0);
        assert_eq!(stats.nodes_crossing_blocks, 0);
        assert_eq!(stats.junk_bytes, 0);
        assert_eq!(stats.bad_crc_nodes, 0);
        assert!(stats.max_dsize <= 4096);
        assert!(!stats.root_has_inode_node);
        assert!(matches!(
            tree.get("etc/phone-model").map(|e| &e.kind),
            Some(Kind::File(_))
        ));
        assert!(matches!(
            tree.get("sbin/init").map(|e| &e.kind),
            Some(Kind::Symlink(_) | Kind::File(_))
        ));

        // Our image of the same tree reads back identical and is about the same size.
        let ours = write_image(&tree, &Options::default()).unwrap();
        let (back, ours_stats) = read_image(&ours, 0x20000).unwrap();
        assert!(back == tree, "{}: rewritten tree differs", path.display());
        assert_eq!(ours_stats.dirents, stats.dirents);
        let ratio = ours.len() as f64 / image.len() as f64;
        eprintln!(
            "  rewritten: {} bytes ({:.3} of stock); {ours_stats:?}",
            ours.len(),
            ratio
        );
        assert!((0.9..1.05).contains(&ratio), "size ratio {ratio}");
    }
}
