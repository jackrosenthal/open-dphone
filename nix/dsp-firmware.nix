# The voice DSP firmware, `dspapp1107_lcc.bin`.  It is proprietary, so it is
# never in the repository; whoever builds an image supplies it.  There are
# two variants: one for the old family (D40, D45, D50, D70) and one for the
# new family (D60, D62, D65).  Both come from a stock firmware's root file
# system, at /lib/firmware/dspapp1107_lcc.bin.
#
# Two ways to supply the files:
#
# 1. Pure (the default).  Add each file to the Nix store once; the build then
#    finds it by hash:
#
#      nix store add-file --name dspapp1107_lcc-old-family.bin <D40 rootfs>/lib/firmware/dspapp1107_lcc.bin
#      nix store add-file --name dspapp1107_lcc-new-family.bin <D60 rootfs>/lib/firmware/dspapp1107_lcc.bin
#
#    The hashes below are those of the files in stock release 2_9_28.
#
# 2. Impure, for a different firmware release.  Name the files in the
#    environment and pass --impure:
#
#      DPHONE_DSP_FIRMWARE_OLD=/path/old.bin DPHONE_DSP_FIRMWARE_NEW=/path/new.bin \
#        nix build --impure .#eff-d60
#
# Without either, the build stops with a message naming the missing file.
{ lib, requireFile }:
let
  variant =
    {
      family,
      envVar,
      sha256,
      models,
      example,
    }:
    let
      fromEnv = builtins.getEnv envVar;
      name = "dspapp1107_lcc-${family}-family.bin";
    in
    if fromEnv != "" then
      builtins.path {
        path = fromEnv;
        inherit name;
      }
    else
      requireFile {
        inherit name sha256;
        message = ''
          Missing proprietary input: the ${family}-family voice DSP firmware (${models}).

          Open DPhone cannot distribute this file.  Take it from a stock firmware's
          root file system (/lib/firmware/dspapp1107_lcc.bin on a ${example}) and run:

            nix store add-file --name ${name} /path/to/dspapp1107_lcc.bin

          or set ${envVar}=/path/to/dspapp1107_lcc.bin and build with --impure.
          See nix/dsp-firmware.nix.
        '';
      };
in
{
  old = variant {
    family = "old";
    envVar = "DPHONE_DSP_FIRMWARE_OLD";
    sha256 = "4a0ecb6a11ae1a23a3c485f21f1f3c7d17c398a28e2f0beb8645bc2ebcf6ea4b";
    models = "D40, D45, D50, and D70";
    example = "D40";
  };
  new = variant {
    family = "new";
    envVar = "DPHONE_DSP_FIRMWARE_NEW";
    sha256 = "1dbb68e82e8ab6db913639813bc6bd099aeca5f0e0adae77d6e8ccb46c0c371c";
    models = "D60, D62, and D65";
    example = "D60";
  };
}
