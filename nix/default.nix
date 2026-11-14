# Everything the flake exports.  Each artifact is its own small derivation:
#
#   kernel             zImage, DTBs, modules (one build for all models)
#   kernel-<model>     boot image for the k0/k1 slot
#   dphone             the firmware binary (ARMv6, static)
#   dphone-image       host tool: EFF, OTA, kernel image, rootfs
#   brush, coreutils   shell and core utilities (ARMv6, static)
#   apps               built-in phone apps (.wasm)
#   web                frontend bundle
#   rootfs-tree        the root file system as a directory
#   rootfs             the JFFS2 image
#   eff-<model>        flashable image, one per model
#   all                the seven EFFs in one directory
#
# Parts of the tree that do not exist yet (apps/, web/) are left out of the
# outputs rather than failing evaluation.
{
  pkgs,
  lib,
}:
let
  root = ../.;
  has = p: builtins.pathExists (root + "/${p}");

  rustToolchain = pkgs.rust-bin.stable.latest.minimal.override {
    extensions = [
      "clippy"
      "rustfmt"
      "rust-src"
    ];
    targets = [
      "arm-unknown-linux-musleabi"
      "wasm32-wasip2"
      "wasm32-unknown-unknown"
    ];
  };
  rustPlatform = pkgs.makeRustPlatform {
    cargo = rustToolchain;
    rustc = rustToolchain;
  };
  rust = pkgs.callPackage ./rust.nix { inherit rustToolchain rustPlatform; };

  userland = pkgs.callPackage ./userland.nix { inherit rustPlatform rust; };
  kernel = pkgs.callPackage ./kernel.nix { kernelSrc = root + "/kernel"; };
  dspFirmware = pkgs.callPackage ./dsp-firmware.nix { };

  web =
    if has "web/pnpm-lock.yaml" then
      # The hash of the fetched pnpm store; when web/pnpm-lock.yaml changes,
      # set it to lib.fakeHash, build, and copy the hash Nix reports.
      pkgs.callPackage ./web.nix { pnpmDepsHash = "sha256-95kN11RWZZX7toWE8RD+RYruv58MfEVDC3VrxIwNws0="; }
    else
      null;

  workspace = pkgs.callPackage ./workspace.nix {
    inherit rustPlatform rust;
    webDist = web;
  };
  apps =
    if has "apps/Cargo.lock" then
      pkgs.callPackage ./apps.nix {
        inherit rustPlatform rust;
        inherit (workspace) dphone-apps-build;
      }
    else
      null;

  image = pkgs.callPackage ./image.nix {
    inherit (workspace) dphone dphone-image version;
    inherit (userland) brush coreutils applets;
    inherit kernel dspFirmware apps;
  };

  # Development images: SSH on, with one public key compiled in.  The key
  # is not in the repository; name it in the environment and build with
  # --impure:
  #
  #   DPHONE_DEV_SSH_KEY="$(cat ~/.ssh/id_ed25519.pub)" nix build --impure .#eff-d60-dev
  devSshKey = builtins.getEnv "DPHONE_DEV_SSH_KEY";
  devWorkspace = pkgs.callPackage ./workspace.nix {
    inherit rustPlatform rust devSshKey;
    webDist = web;
  };
  devImage = pkgs.callPackage ./image.nix {
    inherit (devWorkspace) dphone dphone-image version;
    inherit (userland) brush coreutils applets;
    inherit kernel dspFirmware apps;
    dev = true;
  };
  devEffs = lib.mapAttrs' (
    m: v:
    lib.nameValuePair "eff-${m}-dev" (
      if devSshKey == "" then
        throw "eff-${m}-dev needs an SSH public key: set DPHONE_DEV_SSH_KEY and build with --impure"
      else
        v
    )
  ) devImage.effs;

  effs = lib.mapAttrs' (m: v: lib.nameValuePair "eff-${m}" v) image.effs;
  kernels = lib.mapAttrs' (m: v: lib.nameValuePair "kernel-${m}" v) image.kernelImages;
in
{
  packages = {
    inherit kernel;
    inherit (userland) brush coreutils;
    inherit (workspace) dphone dphone-image;
    rootfs-tree = image.rootfsTree;
    inherit (image) rootfs;
    all = pkgs.linkFarm "open-dphone-${workspace.version}" (
      lib.mapAttrsToList (m: v: {
        name = v.name;
        path = v;
      }) image.effs
    );
    default = image.effs.d60;
  }
  // kernels
  // effs
  // devEffs
  // lib.optionalAttrs (web != null) { inherit web; }
  // lib.optionalAttrs (apps != null) { inherit apps; };

  checks = {
    inherit (workspace) tests clippy;
    inherit kernel;
    inherit (userland) brush coreutils;
    inherit (workspace) dphone;
    # The rootfs image under the kernel's JFFS2 driver, on simulated NAND.
    jffs2-mount = pkgs.callPackage ./jffs2-mount-test.nix {
      image = image.rootfs;
      tree = image.rootfsTree;
    };
  };

  devShell = pkgs.mkShell {
    packages = [
      rustToolchain
      pkgs.protobuf
      pkgs.buf
      pkgs.nodejs
      pkgs.qemu
      pkgs.pkgsCross.arm-embedded.buildPackages.gcc
      pkgs.pkgsCross.arm-embedded.buildPackages.binutils
      pkgs.bc
      pkgs.bison
      pkgs.flex
      pkgs.perl
      pkgs.kmod
      pkgs.dtc
      pkgs.nixfmt
    ];
  };
}
