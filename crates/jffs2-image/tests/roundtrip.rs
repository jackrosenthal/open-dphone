use jffs2_image::{Compression, Entry, Kind, Options, Tree, read_image, write_image};

fn sample(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            (x >> 24) as u8
        })
        .collect()
}

fn tree() -> Tree {
    let t0 = 1_700_000_000;
    let mut t = Tree::new();
    t.dir("etc", 0o755, t0).unwrap();
    t.file(
        "etc/passwd",
        b"root:x:0:0:root:/root:/bin/sh\n".to_vec(),
        0o644,
        t0,
    )
    .unwrap();
    t.file("etc/empty", Vec::new(), 0o600, t0).unwrap();
    t.symlink("sbin/init", "../bin/dphone", t0).unwrap();
    t.char_dev("dev/console", 5, 1, 0o600, t0).unwrap();
    t.char_dev("dev/big", 300, 70000, 0o600, t0).unwrap();
    t.insert(
        "dev/blk",
        Entry {
            kind: Kind::BlockDev {
                major: 31,
                minor: 3,
            },
            mode: 0o660,
            uid: 0,
            gid: 6,
            mtime: t0,
        },
    )
    .unwrap();
    t.insert(
        "run/fifo",
        Entry {
            kind: Kind::Fifo,
            mode: 0o644,
            uid: 1000,
            gid: 1000,
            mtime: t0,
        },
    )
    .unwrap();
    // Compressible, incompressible, sparse-looking, and odd-sized files.
    t.file(
        "bin/text",
        b"The quick brown fox jumps over the lazy dog.\n".repeat(5000),
        0o755,
        t0,
    )
    .unwrap();
    t.file("bin/noise", sample(700_001, 5), 0o755, t0).unwrap();
    t.file("bin/zeros", vec![0u8; 123_457], 0o644, t0).unwrap();
    let mut mixed = sample(300_000, 9);
    for (i, b) in mixed.iter_mut().enumerate() {
        if (i / 5000) % 2 == 0 {
            *b = b'a' + (i % 7) as u8;
        }
    }
    t.file("usr/share/mixed", mixed, 0o644, t0).unwrap();
    for i in 0..300 {
        t.file(
            &format!("usr/lib/many/file-{i:03}"),
            sample(i * 37, i as u32),
            0o644,
            t0,
        )
        .unwrap();
    }
    t
}

#[test]
fn round_trip_every_compression_mode() {
    let t = tree();
    for compression in [
        Compression::ZlibThenRtime,
        Compression::Rtime,
        Compression::None,
    ] {
        for erase_block in [0x20000, 0x4000] {
            let opts = Options {
                compression,
                erase_block,
                ..Default::default()
            };
            let image = write_image(&t, &opts).unwrap();
            let (back, stats) = read_image(&image, erase_block).unwrap();
            assert_eq!(back, t, "{compression:?} {erase_block:#x}");
            assert_eq!(stats.nodes_crossing_blocks, 0);
            assert_eq!(stats.junk_bytes, 0);
            assert_eq!(stats.bad_crc_nodes, 0);
            assert_eq!(stats.cleanmarkers, 0);
            assert!(stats.max_dsize <= 4096);
            assert_eq!(image.len() % 4, 0);
            // Reproducible.
            assert_eq!(write_image(&t, &opts).unwrap(), image);
        }
    }
}

#[test]
fn compression_shrinks_and_padding_option() {
    let t = tree();
    let z = write_image(&t, &Options::default()).unwrap();
    let n = write_image(
        &t,
        &Options {
            compression: Compression::None,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(z.len() < n.len());
    let padded = write_image(
        &t,
        &Options {
            pad_last_block: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(padded.len() % 0x20000, 0);
    assert_eq!(&padded[..z.len()], &z[..]);
    assert!(padded[z.len()..].iter().all(|&b| b == 0xff));
}

#[test]
fn bad_trees_are_refused() {
    let mut t = Tree::new();
    assert!(t.file("a/../b", Vec::new(), 0o644, 0).is_err());
    t.file("a", Vec::new(), 0o644, 0).unwrap();
    assert!(t.file("a/b", Vec::new(), 0o644, 0).is_err());
}

#[test]
fn host_directory_round_trip() {
    let dir = std::env::temp_dir().join(format!("jffs2-image-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let src = dir.join("src");
    std::fs::create_dir_all(src.join("a/b")).unwrap();
    std::fs::write(src.join("a/b/file"), b"contents").unwrap();
    std::os::unix::fs::symlink("b/file", src.join("a/link")).unwrap();
    let t = Tree::from_dir(&src, false, Some(42)).unwrap();
    let image = write_image(&t, &Options::default()).unwrap();
    let (back, _) = read_image(&image, 0x20000).unwrap();
    assert_eq!(back, t);
    let skipped = back.extract(&dir.join("out")).unwrap();
    assert!(skipped.is_empty());
    assert_eq!(
        std::fs::read(dir.join("out/a/b/file")).unwrap(),
        b"contents"
    );
    assert_eq!(
        std::fs::read_link(dir.join("out/a/link")).unwrap().to_str(),
        Some("b/file")
    );
    assert_eq!(
        Tree::from_dir(&dir.join("out"), false, Some(42)).unwrap(),
        t
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
