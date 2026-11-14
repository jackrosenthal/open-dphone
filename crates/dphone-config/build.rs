//! Generates the `DphoneSettings` message type (with protobuf text-format
//! support) from `proto/dphone/settings/v1/settings.proto`.  Rust only:
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
    let name = "dphone/settings/v1/settings.proto";
    println!("cargo:rerun-if-changed={}", root.join(name).display());

    let fds = protox::Compiler::new([&root])
        .expect("proto include path")
        .include_imports(true)
        .open_file(name)
        .expect("compiling settings.proto")
        .file_descriptor_set();
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
    let files = buffa_codegen::generate(&fds.file, &[name.to_owned()], &config).expect("buffa-codegen");
    for f in files {
        fs::write(out.join(&f.name), f.content).expect("writing generated code");
    }
}
