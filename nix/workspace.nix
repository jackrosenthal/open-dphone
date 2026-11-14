# The cargo workspace: `dphone` for the phone, `dphone-image` for the build
# machine, and the test and lint checks.
#
# dpma-rs is a sibling checkout during development (`../dpma-rs` from the
# repository root).  The derivations reproduce that layout from the pinned
# `dpma-rs` flake input, so a path dependency such as
# `../../../dpma-rs/crates/dpma-proto` resolves in both places.
{
  lib,
  rustPlatform,
  rust,
  dpma-rs,
  protobuf,
  webDist ? null, # built frontend, embedded by dphone-web (DPHONE_WEB_DIST)
}:
let
  root = ../.;
  optionalPath = p: lib.optional (builtins.pathExists p) p;
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions (
      [
        (root + "/Cargo.toml")
        (root + "/Cargo.lock")
        (root + "/.cargo")
        (root + "/crates")
        (root + "/xtask")
      ]
      ++ optionalPath (root + "/proto")
    );
  };
  cargoDeps = rustPlatform.importCargoLock {
    lockFile = root + "/Cargo.lock";
    allowBuiltinFetchGit = true;
  };
  version = (lib.importTOML (root + "/Cargo.toml")).workspace.package.version;
  hasCrate = name: builtins.pathExists (root + "/crates/${name}/Cargo.toml");

  build =
    args:
    rust.buildCargo (
      {
        inherit src cargoDeps version;
        nativeBuildInputs = [ protobuf ];
        preBuild = ''
          ln -sfn ${dpma-rs} "$NIX_BUILD_TOP/dpma-rs"
        '';
        env = lib.optionalAttrs (webDist != null) { DPHONE_WEB_DIST = "${webDist}"; };
      }
      // args
    );
in
{
  inherit
    src
    cargoDeps
    version
    hasCrate
    ;

  # The firmware binary: static, ARMv6 soft-float.
  dphone = build {
    pname = "dphone";
    target = rust.armTarget;
    commands = [
      [
        "build"
        "--release"
        "-p"
        "dphone"
      ]
    ];
    bins = [ "dphone" ];
  };

  # Host tool: EFF, OTA, kernel image, and rootfs assembly.
  dphone-image = build {
    pname = "dphone-image";
    commands = [
      [
        "build"
        "--release"
        "-p"
        "dphone-image"
      ]
    ];
    bins = [ "dphone-image" ];
  };

  # Unit and integration tests of every crate, on the build machine.
  tests = build {
    pname = "dphone-tests";
    commands = [
      [
        "test"
        "--workspace"
      ]
    ];
  };

  clippy = build {
    pname = "dphone-clippy";
    commands = [
      [
        "clippy"
        "--workspace"
        "--all-targets"
        "--"
        "-D"
        "warnings"
      ]
      [
        "clippy"
        "-p"
        "dphone"
        "--target"
        rust.armTarget
        "--"
        "-D"
        "warnings"
      ]
    ];
  };
}
