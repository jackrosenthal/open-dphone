# The cargo workspace: `dphone` for the phone, the two host tools
# (`dphone-image`, `dphone-apps-build`), and the test and lint checks.
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
  protobuf,
  openssh,
  iproute2,
  qemu-user,
  vendorFirmware, # the stock images (vendor-firmware.nix), given the host tool
  asterisk, # Asterisk with res_dpma_rs (asterisk.nix)
  webDist ? null, # built frontend, embedded by dphone-web (DPHONE_WEB_DIST)
  appsStaged ? null, # apps compiled for the build machine (DPHONE_APPS_STAGED)
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

  stock = vendorFirmware { dphone-image = imageTool.package; };

  checksStage = twoStage ({
    pname = "dphone-checks";
    cargoExtraArgs = "--locked --workspace";
    # Debug profile: the checks do not ship.
    CARGO_PROFILE = "dev";
  });
in
{
  inherit src version;

  # The firmware binary: static, ARMv6 soft-float.
  dphone = phone.package;

  # Host tools.  `dphone-image`: EFF, OTA, kernel image, and JFFS2.
  # `dphone-apps-build`: componentizes and precompiles the WebAssembly apps.
  dphone-image = imageTool.package;
  dphone-apps-build = appsTool.package;

  # Unit and integration tests of every crate, on the build machine.
  tests = craneLib.cargoTest (
    checksStage.full
    // {
      cargoArtifacts = checksStage.deps;
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
      # The comparisons with the stock firmware: the images of all seven
      # models, and the D60's root file system for its `configmerge`.
      # Without the variable these tests skip themselves; here they must
      # run.
      DPHONE_TEST_REQUIRE_VENDOR = "1";
      DPHONE_VENDOR_DIR = "${stock.dir}";
      DPHONE_STOCK_ROOTFS = "${stock.rootfs}";
      # Provisioning and registration against Asterisk with res_dpma_rs
      # (crates/dphone/tests/asterisk.rs), likewise required here.
      DPHONE_TEST_REQUIRE_ASTERISK = "1";
      DPHONE_TEST_ASTERISK = "${asterisk.server}";
      DPHONE_TEST_ASTERISK_MODULES = "${asterisk.modules}";
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
      cargoClippyExtraArgs = "--all-targets -- -D warnings";
    }
  );

  # Dependency layers, exported so they can be kept alive across garbage
  # collection.
  deps = {
    phone = phone.deps;
    image-tool = imageTool.deps;
    apps-tool = appsTool.deps;
    checks = checksStage.deps;
  };
}
