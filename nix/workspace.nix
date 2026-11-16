# The cargo workspace: `dphone` for the phone, the two host tools
# (`dphone-image`, `dphone-apps-build`), the DPMA module for the tests'
# Asterisk (`res_open_dpma`), and the test and lint checks.
#
# Each product is built in two derivations (crane).  The first compiles only
# the third-party dependencies, from Cargo.lock and the manifests alone, so
# it is rebuilt only when those change.  The second compiles the workspace's
# own crates on top of it.  A change to our code therefore recompiles our
# crates and relinks; it does not recompile the dependencies.
{
  lib,
  craneLib,
  rust,
  rustPlatform,
  protobuf,
  openssh,
  iproute2,
  qemu-user,
  stock, # the stock images (vendor-firmware.nix)
  asterisk, # Asterisk, its headers, and its modules with ours (asterisk.nix)
  webDist ? null, # built frontend, embedded by dphone-web (DPHONE_WEB_DIST)
  appsStaged ? null, # apps compiled for the build machine (DPHONE_APPS_STAGED)
  appsUnitTests ? null, # the apps' own tests, passed (apps.nix)
  appsClippy ? null, # the apps' own lints, passed (apps.nix)
}:
let
  root = ../.;
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + "/Cargo.toml")
      (root + "/Cargo.lock")
      (root + "/.cargo")
      (root + "/crates")
      (root + "/xtask")
      # The DPMA module for Asterisk and its libraries, two of which the
      # firmware uses too (dpma-proto and dpma-bytes).
      (root + "/open-dpma")
      # The Light Up app's game library, which the host's test of that app
      # (crates/dphone-apps/tests/lightup.rs) links, and the manifest of
      # the apps' workspace, from which it inherits its package fields.
      (root + "/apps/Cargo.toml")
      (root + "/apps/lightup/game")
      # Service definitions, read by the build scripts of dphone-config and
      # dphone-web.
      (root + "/proto")
      # Fonts and SVG artwork, rasterized by dphone-ui's build script.
      (root + "/assets")
    ];
  };
  version = (lib.importTOML (root + "/Cargo.toml")).workspace.package.version;

  common = {
    inherit src version;
    strictDeps = true;
    doCheck = false;
    nativeBuildInputs = [ protobuf ];
    # The hardening wrapper flags are for the native C compiler; they must
    # not leak into the ARM link, which rust-lld does by itself.
    hardeningDisable = [ "all" ];
  };

  # The workspace profile (fat LTO, one codegen unit) is for the phone.  The
  # host tools only run during the build, so they are built the quick way.
  hostEnv = {
    CARGO_PROFILE_RELEASE_LTO = "off";
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "16";
    CARGO_PROFILE_RELEASE_OPT_LEVEL = "2";
  };

  # args.cargoExtraArgs selects the packages; the same selection is used for
  # the dependency layer so that feature unification matches.
  twoStage =
    args:
    let
      full = common // args;
      cargoArtifacts = craneLib.buildDepsOnly (full // { pname = "${args.pname}-deps"; });
    in
    {
      deps = cargoArtifacts;
      inherit full;
      package = craneLib.buildPackage (full // { inherit cargoArtifacts; });
    };

  phone = twoStage (
    {
      pname = "dphone";
      cargoExtraArgs = "--locked -p dphone";
      dontStrip = true;
      dontPatchELF = true;
    }
    // rust.armEnv
    // lib.optionalAttrs (webDist != null) { DPHONE_WEB_DIST = "${webDist}"; }
  );

  # The early-boot stage for the kernel's initramfs: no standard library,
  # so it is built to abort on a panic (the workspace profile unwinds), and
  # for size above all.
  early = twoStage (
    {
      pname = "dphone-early";
      cargoExtraArgs = "--locked -p dphone-early";
      dontStrip = true;
      dontPatchELF = true;
      CARGO_PROFILE_RELEASE_PANIC = "abort";
      CARGO_PROFILE_RELEASE_OPT_LEVEL = "z";
    }
    // rust.armEnv
  );

  imageTool = twoStage (
    {
      pname = "dphone-image";
      cargoExtraArgs = "--locked -p dphone-image";
    }
    // hostEnv
  );

  appsTool = twoStage (
    {
      pname = "dphone-apps-build";
      cargoExtraArgs = "--locked -p dphone-apps --features build-tool --bin dphone-apps-build";
    }
    // hostEnv
  );

  # `res_open_dpma` generates its bindings from the configured headers of the
  # Asterisk it is loaded into (`ASTERISK_SOURCE_DIR`, read by dpma-sys).
  # It has a profile of its own: it runs on a server, not on the phone.
  module = twoStage {
    pname = "res_open_dpma";
    cargoExtraArgs = "--locked -p res_open_dpma";
    CARGO_PROFILE = "module";
    nativeBuildInputs = common.nativeBuildInputs ++ [ rustPlatform.bindgenHook ];
    ASTERISK_SOURCE_DIR = "${asterisk.server}/share/asterisk-source";
  };

  # The crates that need Asterisk's headers are left out: the module's
  # tests cannot link outside Asterisk, and `cargo xtask ci` in open-dpma/
  # builds, lints, and tests all three against every Asterisk release that
  # the module supports (.github/workflows/ci.yml).
  checksStage = twoStage ({
    pname = "dphone-checks";
    cargoExtraArgs = "--locked --workspace --exclude dpma-sys --exclude dpma-ast --exclude res_open_dpma";
    # Debug profile: the checks do not ship.
    CARGO_PROFILE = "dev";
  });
in
{
  inherit src version;

  # The firmware binary: static, ARMv6 soft-float.
  dphone = phone.package;

  # `/init` of the kernel's initramfs: static, ARMv6 soft-float, a few
  # kilobytes.
  dphone-early = early.package;

  # Host tools.  `dphone-image`: EFF, OTA, kernel image, and JFFS2.
  # `dphone-apps-build`: componentizes and precompiles the WebAssembly apps.
  dphone-image = imageTool.package;
  dphone-apps-build = appsTool.package;

  # The DPMA module, `lib/libres_open_dpma.so`, for the Asterisk of
  # asterisk.nix.
  res_open_dpma = module.package;

  # Unit and integration tests of every crate, on the build machine.
  tests = craneLib.cargoTest (
    checksStage.full
    // {
      cargoArtifacts = checksStage.deps;
      # The check is that this builds; nothing builds on its target
      # directory, so it is not archived into the output.
      doInstallCargoArtifacts = false;
      doCheck = true;
      # Every failure in one run, not only the first crate's.
      cargoTestExtraArgs = "--no-fail-fast";
      # The OpenSSH client, for the test that logs in to the embedded SSH
      # server and pushes a file with `scp`; that test skips itself on a
      # machine without one.
      # qemu-arm, for the tests that run the stock `configmerge`; `ip`, for
      # the tests that run in a network namespace of their own.
      nativeBuildInputs = checksStage.full.nativeBuildInputs ++ [
        openssh
        iproute2
        qemu-user
      ];
      DPHONE_TEST_REQUIRE_SSH = "1";
      # The comparisons with the stock firmware: the D40 and D60 images,
      # and the D60's root file system for its `configmerge`.
      # Without the variable these tests skip themselves; here they must
      # run.
      DPHONE_TEST_REQUIRE_VENDOR = "1";
      DPHONE_VENDOR_DIR = "${stock.dir}";
      DPHONE_STOCK_ROOTFS = "${stock.rootfs}";
      # Provisioning and registration against Asterisk with res_open_dpma
      # of this repository
      # (crates/dphone/tests/asterisk.rs), likewise required here.
      DPHONE_TEST_REQUIRE_ASTERISK = "1";
      DPHONE_TEST_ASTERISK = "${asterisk.server}";
      DPHONE_TEST_ASTERISK_MODULES = "${asterisk.modules}";
    }
    // lib.optionalAttrs (appsUnitTests != null) {
      # `apps/` is a cargo workspace of its own, which `cargo test` here
      # does not reach: its tests run in a derivation of their own, and
      # this check does not pass unless that one did.
      inherit appsUnitTests;
    }
    // lib.optionalAttrs (appsStaged != null) {
      # A writable copy, as the tests' own build of the apps would leave:
      # they copy files out of it and then edit the copies, and a copy of a
      # store file is read-only.
      preCheck = ''
        cp -r ${appsStaged} "$NIX_BUILD_TOP/apps-staged"
        chmod -R u+w "$NIX_BUILD_TOP/apps-staged"
        export DPHONE_APPS_STAGED="$NIX_BUILD_TOP/apps-staged"
      '';
    }
  );

  clippy = craneLib.cargoClippy (
    checksStage.full
    // {
      cargoArtifacts = checksStage.deps;
      doInstallCargoArtifacts = false;
      cargoClippyExtraArgs = "--all-targets -- -D warnings";
    }
    // lib.optionalAttrs (appsClippy != null) {
      # Likewise the apps' workspace, linted in a derivation of its own.
      inherit appsClippy;
    }
  );

  # Dependency layers, exported so they can be kept alive across garbage
  # collection.
  deps = {
    phone = phone.deps;
    image-tool = imageTool.deps;
    apps-tool = appsTool.deps;
    module = module.deps;
    checks = checksStage.deps;
  };
}
