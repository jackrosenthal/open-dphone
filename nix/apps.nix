# The built-in phone apps.  `apps/` is its own cargo workspace (excluded
# from the firmware workspace); its crates are plain cdylibs for
# wasm32-unknown-unknown.  `dphone-apps-build stage` then turns each module
# into a component precompiled to Pulley bytecode for the phone and lays
# out <name>/{app.json,<name>.cwasm,data files}, which the image installs
# as /usr/share/dphone/apps.
#
# The compiled code is tied to the Wasmtime version and engine settings of
# `crates/dphone-apps`, so the staging tool comes from the same source as
# the phone binary (`dphone-apps-build` in workspace.nix).
{
  lib,
  runCommand,
  rustPlatform,
  rust,
  dphone-apps-build,
}:
let
  root = ../.;
  wasmTarget = "wasm32-unknown-unknown";
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + "/apps")
      # app-sdk inherits its package fields from the firmware workspace.
      (root + "/Cargo.toml")
      (root + "/crates/app-sdk")
      (root + "/crates/dphone-apps/wit")
    ];
  };
  modules = rust.buildCargo {
    pname = "dphone-apps-wasm";
    version = "3.0.0";
    inherit src;
    cargoRoot = "apps";
    cargoDeps = rustPlatform.importCargoLock { lockFile = root + "/apps/Cargo.lock"; };
    env.CARGO_BUILD_TARGET = wasmTarget;
    target = wasmTarget;
    commands = [
      [
        "build"
        "--release"
        "--workspace"
      ]
    ];
    # Every app's module, flat, under $out/.
    artifacts = [ "*.wasm" ];
  };
in
runCommand "dphone-apps-3.0.0"
  {
    nativeBuildInputs = [ dphone-apps-build ];
    passthru = { inherit modules; };
  }
  ''
    dphone-apps-build stage \
      --apps ${root + "/apps"} \
      --wasm-dir ${modules} \
      --out $out \
      --target phone --no-wasm
  ''
