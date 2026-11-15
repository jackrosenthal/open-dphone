//! Generates the `DphoneSettings` and `Palette` message types (with
//! protobuf text-format support) from
//! `proto/dphone/settings/v1/settings.proto` and
//! `proto/dphone/palette/v1/palette.proto`.  Rust only:
//! protox compiles the proto, so no `protoc` is needed, and buffa-codegen
//! emits the message type.

use std::env;
use std::fs;
use std::path::PathBuf;

use buffa::Message as _;
use buffa_codegen::generated::descriptor::FileDescriptorSet;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let root = manifest.join("../../proto");
    let names = ["dphone/settings/v1/settings.proto", "dphone/palette/v1/palette.proto"];
    for name in names {
        println!("cargo:rerun-if-changed={}", root.join(name).display());
    }

    let mut compiler = protox::Compiler::new([&root]).expect("proto include path");
    compiler.include_imports(true);
    for name in names {
        compiler.open_file(name).unwrap_or_else(|e| panic!("compiling {name}: {e}"));
    }
    let fds = compiler.file_descriptor_set();
    let bytes = prost::Message::encode_to_vec(&fds);
    let fds = FileDescriptorSet::decode_from_slice(&bytes).expect("decoding descriptor set");

    let mut config = buffa_codegen::CodeGenConfig::default();
    config.generate_text = true;
    config.generate_views = false;
    config.generate_json = false;
    // The text format names fields, so numbered unknown-field storage would
    // never be filled; fields this firmware does not know are kept as text
    // by `settings.rs` instead.
    config.preserve_unknown_fields = false;
    config.file_per_package = true;
    let files = buffa_codegen::generate(&fds.file, &names.map(str::to_owned), &config).expect("buffa-codegen");
    for f in files {
        fs::write(out.join(&f.name), f.content).expect("writing generated code");
    }
}
