# The vendor's stock firmware images.  They are proprietary, so they are
# never in the repository; Nix downloads each from the vendor when
# something needs it, and checks it against the hash here.  The whole
# repository uses exactly two: the D40's for the old family and the D60's
# for the new one.  Two things need them:
#
# - An image build needs the voice DSP firmware, which is a file in the
#   stock root file system (dsp-firmware.nix).
# - The checks compare against them: the EFF/OTA round trips, the JFFS2
#   structure checks, and the stock `configmerge` (run under qemu-arm) that
#   our configuration merge must agree with.
#
# The downloads are fixed-output derivations: Nix looks for the result by
# its hash before it fetches.  A store that already holds an image builds
# without the network, whether or not the vendor still serves it.  To put a
# copy you have into the store by hand:
#
#   nix store add-file 2_9_28_D60_firmware.eff
{
  lib,
  fetchurl,
  runCommand,
  dphone-image, # the host tool (not the `dphone` package: see dsp-firmware.nix)
}:
let
  # The releases the tests' expectations were taken from.
  images = {
    D40 = {
      version = "2_9_26";
      sha256 = "7b038b4fb03026cdb49bff3ec3809804912be6f6a591abd1d4ebc6a0e9ac9edd";
    };
    D60 = {
      version = "2_9_28";
      sha256 = "2e55006abb7e6e51979c2556865900b4d38afffd504fa8aaa360b08f59f6dd02";
    };
  };

  eff =
    model:
    { version, sha256 }:
    let
      name = "${version}_${model}_firmware.eff";
    in
    fetchurl {
      inherit name sha256;
      url = "https://phones.dl.sangoma.com/firmware/asterisk/${version}/${name}";
    };
  effs = lib.mapAttrs eff images;

  # Each image taken apart with our own tool: DIR/kernel.bin and
  # DIR/rootfs.jffs2.
  parts = lib.mapAttrs (
    model: file:
    runCommand "stock-${lib.toLower model}-parts" { nativeBuildInputs = [ dphone-image ]; } ''
      dphone-image eff unpack ${file} firmware.ota
      dphone-image ota unpack firmware.ota $out
    ''
  ) effs;
in
{
  inherit effs parts;

  # The files in the layout the tests read (`DPHONE_VENDOR_DIR`), which is
  # the layout of the reverse-engineering tree they were written against.
  dir = runCommand "stock-firmware" { } (
    ''
      mkdir -p $out/ota-extracted-d60 $out/firmware-series
      ln -s ${effs.D60} $out/${effs.D60.name}
      ln -s ${parts.D60}/rootfs.jffs2 $out/ota-extracted-d60/rootfs.jffs2
    ''
    + lib.concatStrings (
      lib.mapAttrsToList (
        model: file:
        lib.optionalString (model != "D60") ''
          mkdir $out/firmware-series/${model}
          ln -s ${file} $out/firmware-series/${model}/${file.name}
          ln -s ${parts.${model}}/rootfs.jffs2 $out/firmware-series/${model}/rootfs.jffs2
        ''
      ) effs
    )
  );

  # The stock D60 root file system as a directory (`DPHONE_STOCK_ROOTFS`),
  # for running its programs under qemu-arm.  Device nodes are left out.
  rootfs = runCommand "stock-d60-rootfs" { nativeBuildInputs = [ dphone-image ]; } ''
    dphone-image jffs2 extract ${parts.D60}/rootfs.jffs2 $out > extract.log
    test -x $out/app/bin/configmerge
  '';
}
