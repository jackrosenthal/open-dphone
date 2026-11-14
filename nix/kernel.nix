# Mainline Linux v7.3 with the bcmring port: kernel/patches applied in name
# order, kernel/files copied over the tree, configured from kernel/configs.
# One build serves every model; only the appended DTB differs.
#
# Output layout:
#   zImage                       XZ-compressed kernel, no DTB appended
#   dtbs/<board>.dtb             the seven board devicetrees
#   lib/modules/<release>/...    stripped modules plus the depmod indexes
#   config                       the resolved .config
{
  lib,
  stdenv,
  fetchurl,
  buildPackages,
  pkgsCross,
  bc,
  bison,
  flex,
  perl,
  xz,
  kmod,
  openssl,
  kernelSrc, # path: the repository's kernel/ directory
  defconfig ? "bcmring_tiny_defconfig",
}:
let
  version = "7.3";
  cross = pkgsCross.arm-embedded.buildPackages;
  patchDir = kernelSrc + "/patches";
  patches = map (n: patchDir + "/${n}") (
    lib.sort lib.lessThan (
      lib.attrNames (
        lib.filterAttrs (n: t: t == "regular" && lib.hasSuffix ".patch" n) (builtins.readDir patchDir)
      )
    )
  );
  makeFlags = [
    "ARCH=arm"
    "CROSS_COMPILE=arm-none-eabi-"
    "O=build"
    "HOSTCC=cc"
    "KBUILD_BUILD_USER=open-dphone"
    "KBUILD_BUILD_HOST=nix"
    "KBUILD_BUILD_TIMESTAMP=1970-01-01"
  ];
in
stdenv.mkDerivation {
  pname = "linux-bcmring";
  inherit version patches makeFlags;

  src = fetchurl {
    url = "mirror://kernel/linux/kernel/v7.x/linux-${version}.tar.xz";
    hash = "sha256-hzY8JPEOBjIes2J2VSEJTpRHH2v/i84taz5A858ASVM=";
  };

  depsBuildBuild = [ buildPackages.stdenv.cc ];
  nativeBuildInputs = [
    cross.gcc
    cross.binutils
    bc
    bison
    flex
    perl
    xz
    kmod
    openssl
  ];

  # The compiler wrapper's hardening flags are for hosted userspace.
  hardeningDisable = [ "all" ];
  enableParallelBuilding = true;

  postPatch = ''
    cp -r --no-preserve=mode ${kernelSrc + "/files"}/. .
    cp ${kernelSrc + "/configs"}/* arch/arm/configs/
    patchShebangs scripts
  '';

  configurePhase = ''
    runHook preConfigure
    make $makeFlags ${defconfig}
    runHook postConfigure
  '';

  buildPhase = ''
    runHook preBuild
    make $makeFlags -j$NIX_BUILD_CORES zImage dtbs modules
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p $out/dtbs
    cp build/arch/arm/boot/zImage $out/zImage
    cp build/arch/arm/boot/dts/broadcom/*-digium-*.dtb $out/dtbs/
    cp build/.config $out/config
    make $makeFlags INSTALL_MOD_PATH=$out INSTALL_MOD_STRIP=1 DEPMOD=depmod modules_install
    rm -f $out/lib/modules/*/build $out/lib/modules/*/source
    runHook postInstall
  '';

  dontStrip = true;
  dontPatchELF = true;

  passthru.boards = {
    d40 = "bcm11170-digium-d40";
    d45 = "bcm11109-digium-d45";
    d50 = "bcm11170-digium-d50";
    d60 = "bcm11189-digium-d60";
    d62 = "bcm11188-digium-d62";
    d65 = "bcm11188-digium-d65";
    d70 = "bcm11109-digium-d70";
  };

  meta.license = lib.licenses.gpl2Only;
}
