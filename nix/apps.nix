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
  # Build one cargo workspace of apps (`dir`, relative to the repository
  # root) to WebAssembly modules, flat, under $out/.
  wasmModules =
    { pname, dir }:
    rust.buildCargo {
      inherit pname;
      version = "3.0.0";
      src = lib.fileset.toSource {
        inherit root;
        fileset = lib.fileset.unions [
          (root + "/${dir}")
          # app-sdk inherits its package fields from the firmware workspace.
          (root + "/Cargo.toml")
          (root + "/crates/app-sdk")
          # The Box Pusher puzzle itself, which the app `boxpusher` is built on.
          (root + "/crates/boxpusher-game")
          # Likewise the Light Up puzzle, for the app `lightup`.
          (root + "/crates/lightup-game")
          (root + "/crates/dphone-apps/wit")
        ];
      };
      cargoRoot = dir;
      cargoDeps = rustPlatform.importCargoLock { lockFile = root + "/${dir}/Cargo.lock"; };
      env.CARGO_BUILD_TARGET = wasmTarget;
      target = wasmTarget;
      commands = [
        [
          "build"
          "--release"
          "--workspace"
        ]
      ];
      artifacts = [ "*.wasm" ];
    };
  modules = wasmModules {
    pname = "dphone-apps-wasm";
    dir = "apps";
  };

  # What the host tests of `crates/dphone-apps` run: the built-in apps and
  # the test-only guests, compiled for the build machine's interpreter.  The
  # tests otherwise build these themselves with cargo, which a sandboxed
  # build cannot do (no network for the apps' dependencies); the `tests`
  # check hands them this directory as DPHONE_APPS_STAGED.
  guests = "crates/dphone-apps/tests/guests";
  testFixture =
    runCommand "dphone-apps-test-fixture-3.0.0" { nativeBuildInputs = [ dphone-apps-build ]; }
      ''
        dphone-apps-build stage \
          --apps ${root + "/apps"} \
          --wasm-dir ${modules} \
          --out $out \
          --target pulley64
        dphone-apps-build stage \
          --apps ${root + "/${guests}"} \
          --wasm-dir ${
            wasmModules {
              pname = "dphone-apps-test-guests-wasm";
              dir = guests;
            }
          } \
          --out $out \
          --target pulley64
      '';
in
runCommand "dphone-apps-3.0.0"
  {
    nativeBuildInputs = [ dphone-apps-build ];
    passthru = { inherit modules testFixture; };
  }
  ''
    dphone-apps-build stage \
      --apps ${root + "/apps"} \
      --wasm-dir ${modules} \
      --out $out \
      --target phone --no-wasm
  ''
