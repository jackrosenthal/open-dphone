# The voice DSP firmware, `dspapp1107_lcc.bin`.  It is proprietary, so it is
# never in the repository.  There are two variants: one for the old family
# (D40, D45, D50, and D70) and one for the new family (D60, D62, and D65).
# Each is a file in the root file system of the vendor's stock firmware, at
# /lib/firmware/dspapp1107_lcc.bin, and every model of a family carries the
# same file.
#
# The build takes each from a stock image that Nix downloads from the vendor
# (vendor-firmware.nix): the image is taken apart with our own
# `opendeskphone-image`, and the one file is kept.  Nothing is done by hand.
#
# The extraction is a fixed-output derivation, so the file is pinned by the
# hash here, and a store that already holds it needs neither the stock image
# nor the network.  The hashes are those of the files in stock release
# 2_9_26 (old family) and 2_9_28 (new family).
#
# `opendeskphone-image` is the host tool, built from the workspace by itself;
# the `opendeskphone` package and the images that carry this firmware are not
# among its inputs, so nothing here depends on what it feeds.
{
  runCommand,
  opendeskphone-image,
  stock, # vendor-firmware.nix
}:
let
  variant =
    {
      family,
      model, # the stock image it is taken from
      sha256,
    }:
    runCommand "dspapp1107_lcc-${family}-family.bin"
      {
        nativeBuildInputs = [ opendeskphone-image ];
        outputHashMode = "flat";
        outputHashAlgo = "sha256";
        outputHash = sha256;
      }
      ''
        opendeskphone-image jffs2 extract ${stock.parts.${model}}/rootfs.jffs2 rootfs > extract.log
        # Stock has it executable; a flat fixed output is a plain file.
        install -m 0644 rootfs/lib/firmware/dspapp1107_lcc.bin $out
      '';
in
{
  old = variant {
    family = "old";
    model = "D40";
    sha256 = "4a0ecb6a11ae1a23a3c485f21f1f3c7d17c398a28e2f0beb8645bc2ebcf6ea4b";
  };
  new = variant {
    family = "new";
    model = "D60";
    sha256 = "1dbb68e82e8ab6db913639813bc6bd099aeca5f0e0adae77d6e8ccb46c0c371c";
  };
}
