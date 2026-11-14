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
  dpma-rs,
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
    if has "web/package-lock.json" then
      pkgs.callPackage ./web.nix { npmDepsHash = lib.fakeHash; }
    else
      null;
  apps =
    if has "apps/Cargo.lock" then pkgs.callPackage ./apps.nix { inherit rustPlatform rust; } else null;

  workspace = pkgs.callPackage ./workspace.nix {
    inherit rustPlatform rust dpma-rs;
    webDist = web;
  };

  image = pkgs.callPackage ./image.nix {
    inherit (workspace) dphone dphone-image version;
    inherit (userland) brush coreutils applets;
    inherit kernel dspFirmware apps;
  };

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
