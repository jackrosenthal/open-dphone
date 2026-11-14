# The cargo workspace: `dphone` for the phone, `dphone-image` for the build
# machine, and the test and lint checks.
{
  lib,
  rustPlatform,
  rust,
  protobuf,
  webDist ? null, # built frontend, embedded by dphone-web (DPHONE_WEB_DIST)
  # A development image's SSH public key, compiled into dphone.  Empty for
  # release images.
  devSshKey ? "",
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
      # Fonts and SVG artwork, rasterized by dphone-ui's build script.
      ++ optionalPath (root + "/assets")
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
        env =
          lib.optionalAttrs (webDist != null) { DPHONE_WEB_DIST = "${webDist}"; }
          // lib.optionalAttrs (devSshKey != "") { DPHONE_DEV_SSH_KEY = devSshKey; };
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

  # Host tool: componentizes and precompiles the WebAssembly apps.
  dphone-apps-build = build {
    pname = "dphone-apps-build";
    commands = [
      [
        "build"
        "--release"
        "-p"
        "dphone-apps"
        "--features"
        "build-tool"
        "--bin"
        "dphone-apps-build"
      ]
    ];
    bins = [ "dphone-apps-build" ];
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
