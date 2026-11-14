//! Build script, Rust only:
//!
//! 1. Compiles `proto/dphone/v1/*.proto` with protox (a pure-Rust protobuf
//!    compiler, so no `protoc` is needed), generating prost types and
//!    pbjson serde implementations (proto3 canonical JSON).
//! 2. Embeds the built frontend (`web/dist`) precompressed with gzip and
//!    brotli.  Without `web/dist`, a placeholder page is embedded, so the
//!    crate builds on machines without node.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let repo = manifest.join("../..");
    protos(&repo.join("proto"), &out);
    let dist = match env::var_os("DPHONE_WEB_DIST") {
        Some(p) => PathBuf::from(p),
        None => repo.join("web/dist"),
    };
    println!("cargo:rerun-if-env-changed=DPHONE_WEB_DIST");
    assets(&dist, &out);
}

fn protos(root: &Path, out: &Path) {
    let dir = root.join("dphone/v1");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "proto"))
        .collect();
    files.sort();
    let fds = protox::compile(&files, [root]).expect("compiling proto/");
    let mut cfg = prost_build::Config::new();
    cfg.out_dir(out);
    cfg.compile_fds(fds.clone()).expect("prost-build");
    let bytes = prost::Message::encode_to_vec(&fds);
    pbjson_build::Builder::new()
        .register_descriptors(&bytes)
        .expect("descriptors")
        .out_dir(out)
        .ignore_unknown_fields()
        .build(&[".dphone.v1"])
        .expect("pbjson-build");
}

fn walk(dir: &Path, base: &Path, into: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.map(|e| e.unwrap().path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, base, into);
        } else {
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            into.push((format!("/{rel}"), p));
        }
    }
}

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "json" | "map" => "application/json",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        "webmanifest" => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

fn compressible(path: &str) -> bool {
    !matches!(path.rsplit('.').next().unwrap_or(""), "png" | "woff2" | "ico" | "jpg" | "gz")
}

fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Open DPhone</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>body{font-family:system-ui,sans-serif;max-width:40rem;margin:4rem auto;padding:0 1rem;color:#1f2937}</style>
</head><body><h1>Open DPhone</h1>
<p>This build does not include the web interface (it was built without
<code>web/dist</code>).  The Connect RPC services are available.</p>
</body></html>
"#;

fn assets(dist: &Path, out: &Path) {
    println!("cargo:rerun-if-changed={}", dist.display());
    let mut files = Vec::new();
    let have_dist = dist.join("index.html").is_file();
    if have_dist {
        walk(dist, dist, &mut files);
    }
    let blob_dir = out.join("assets");
    let _ = fs::remove_dir_all(&blob_dir);
    fs::create_dir_all(&blob_dir).unwrap();

    let mut table = String::from("pub static ASSETS: &[Asset] = &[\n");
    let mut emit = |url: &str, data: &[u8], n: usize| {
        let ident = |ext: &str| blob_dir.join(format!("{n}.{ext}"));
        let etag = format!("\"{:016x}\"", fnv(data));
        // Vite puts content-hashed files under /assets/: cache those forever.
        let immutable = url.starts_with("/assets/");
        let (gz, br) = if compressible(url) && data.len() > 256 {
            let gz = gzip(data);
            let mut br = Vec::new();
            {
                let mut w = brotli::CompressorWriter::new(&mut br, 4096, 11, 22);
                w.write_all(data).unwrap();
            }
            (Some(gz), Some(br))
        } else {
            (None, None)
        };
        let field = |ext: &str, bytes: Option<&[u8]>| match bytes {
            Some(b) => {
                let p = ident(ext);
                fs::write(&p, b).unwrap();
                format!("Some(include_bytes!({:?}))", p.display().to_string())
            }
            None => "None".to_string(),
        };
        // Keep the identity copy only where no compressed copy exists; the
        // server inflates the gzip copy for the rare client without gzip.
        let raw = field("raw", if gz.is_some() { None } else { Some(data) });
        let gzf = field("gz", gz.as_deref());
        let brf = field("br", br.as_deref());
        writeln!(
            table,
            "    Asset {{ path: {url:?}, mime: {:?}, etag: {etag:?}, immutable: {immutable}, len: {}, raw: {raw}, gzip: {gzf}, brotli: {brf} }},",
            mime(url),
            data.len(),
        )
        .unwrap();
    };
    if have_dist {
        for (n, (url, path)) in files.iter().enumerate() {
            let data = fs::read(path).unwrap();
            emit(url, &data, n);
        }
    } else {
        emit("/index.html", PLACEHOLDER.as_bytes(), 0);
    }
    table.push_str("];\n");
    writeln!(table, "pub const HAVE_FRONTEND: bool = {have_dist};").unwrap();
    fs::write(out.join("assets.rs"), table).unwrap();
}

/// A gzip member around a raw deflate stream.
fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 2, 0xff];
    out.extend(miniz_oxide::deflate::compress_to_vec(data, 10));
    out.extend(crc32(data).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for b in data {
        crc ^= *b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb88320 } else { crc >> 1 };
        }
    }
    !crc
}
