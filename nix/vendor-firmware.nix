# The stock firmware images the checks compare against: the EFF/OTA round
# trips, the JFFS2 structure checks, and the stock `configmerge` (run under
# qemu-arm) that our configuration merge must agree with.  They are
# proprietary, so they are never in the repository; whoever runs the checks
# supplies them.  Building an image needs none of them.
#
# Download each file from the vendor and add it to the Nix store once; the
# checks then find it by hash:
#
#   curl -fLO https://phones.dl.sangoma.com/firmware/asterisk/2_9_28/2_9_28_D60_firmware.eff
#   nix store add-file 2_9_28_D60_firmware.eff
#
# and likewise for the other six.  Without a file, the check stops with a
# message naming it.
{
  lib,
  requireFile,
  runCommand,
  dphone-image, # the host tool
}:
let
  # The releases the tests' expectations were taken from.
  images = {
    D40 = {
      version = "2_9_26";
      sha256 = "7b038b4fb03026cdb49bff3ec3809804912be6f6a591abd1d4ebc6a0e9ac9edd";
    };
    D45 = {
      version = "2_9_26";
      sha256 = "b45ee927cb5a98948dc7a669eec970d2f2dbe0290c7b6b402b287da221752a96";
    };
    D50 = {
      version = "2_9_26";
      sha256 = "396a0cc94bb6432e7cbb08f46b38f6117ed6aa7123fe0f153064ce73f50a7fba";
    };
    D60 = {
      version = "2_9_28";
      sha256 = "2e55006abb7e6e51979c2556865900b4d38afffd504fa8aaa360b08f59f6dd02";
    };
    D62 = {
      version = "2_9_28";
      sha256 = "f4d2e0638257d11f82ff1583c4a070c07b70eff14ced648b3d090a1b70eb5d47";
    };
    D65 = {
      version = "2_9_28";
      sha256 = "39f6116489d072963b34780d01f91eacb5100e41bf8b06a901399c62722656a1";
    };
    D70 = {
      version = "2_9_26";
      sha256 = "92f42f9cf418e8d9b89d381c065fe76fcbd92315c14c10dc1c73995c5958cb0e";
    };
  };

  eff =
    model:
    { version, sha256 }:
    let
      name = "${version}_${model}_firmware.eff";
      url = "https://phones.dl.sangoma.com/firmware/asterisk/${version}/${name}";
    in
    requireFile {
      inherit name sha256;
      message = ''
        Missing proprietary input: the stock ${model} firmware, ${name}.

        The checks compare against the stock images, and Open DPhone cannot
        distribute them.  Download it from the vendor and add it to the store:

          curl -fLO ${url}
          nix store add-file ${name}

        See nix/vendor-firmware.nix.
      '';
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
