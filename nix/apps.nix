# The built-in phone apps: Rust compiled to WebAssembly.  `apps/` is its own
# cargo workspace (it is excluded from the firmware workspace) and depends on
# `crates/app-sdk` by path.
{
  lib,
  rustPlatform,
  rust,
  wasmTarget ? "wasm32-wasip2",
}:
let
  root = ../.;
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + "/apps")
      (root + "/crates/app-sdk")
      (root + "/crates/dphone-apps/wit")
    ];
  };
in
rust.buildCargo {
  pname = "dphone-apps-wasm";
  version = "3.0.0";
  inherit src;
  cargoRoot = "apps";
  cargoDeps = rustPlatform.importCargoLock { lockFile = root + "/apps/Cargo.lock"; };
  env = rust.sizeProfileEnv // {
    CARGO_BUILD_TARGET = wasmTarget;
  };
  target = wasmTarget;
  commands = [
    [
      "build"
      "--release"
      "--workspace"
    ]
  ];
  # Every app's .wasm, flat, under $out/.
  artifacts = [ "*.wasm" ];
}
