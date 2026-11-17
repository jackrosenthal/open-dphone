# Mainline Linux v7.3 with the bcmring port: kernel/patches applied in name
# order, kernel/files copied over the tree, configured from kernel/configs.
# One build serves every model; only the appended DTB differs.
#
# The kernel image carries an initramfs with one program, the early-boot
# stage (crates/dphone-early): it counts trial boots and can select the
# previous bank before the root file system is needed, then mounts the
# root and runs its init.  The initramfs is not compressed by itself; it
# is compressed with the rest of the zImage.
#
# Output layout:
#   zImage                       XZ-compressed kernel, no DTB appended
#   dtbs/<board>.dtb             the seven board devicetrees
#   lib/modules/<release>/...    stripped modules; no depmod indexes (the
#                                init role carries its own load list)
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
  openssl,
  kernelSrc, # path: the repository's kernel/ directory
  earlyInit, # the built dphone-early ($out/bin/dphone-early, static ARM)
  initramfsList, # path: crates/dphone-early/initramfs.list
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
    "KBUILD_BUILD_USER=dphone"
    "KBUILD_BUILD_HOST=opendeskphone"
    # Fixed release string (7.3.0), so the module directory has one name.
    "LOCALVERSION="
    "KBUILD_BUILD_TIMESTAMP=1970-01-01"
  ];
  src = fetchurl {
    url = "mirror://kernel/linux/kernel/v7.x/linux-${version}.tar.xz";
    hash = "sha256-hzY8JPEOBjIes2J2VSEJTpRHH2v/i84taz5A858ASVM=";
  };

  applyPort = ''
    cp -r --no-preserve=mode ${kernelSrc + "/files"}/. .
    cp ${kernelSrc + "/configs"}/* arch/arm/configs/
  '';

  # The v7.3 tree with the port applied and nothing built: what a developer
  # copies over a git checkout of v7.3 to work on the port (kernel/README.md).
  source = stdenv.mkDerivation {
    pname = "linux-bcmring-source";
    inherit version src patches;
    postPatch = applyPort;
    dontConfigure = true;
    dontBuild = true;
    dontFixup = true;
    installPhase = "cp -r . $out";
  };
in
stdenv.mkDerivation {
  pname = "linux-bcmring";
  inherit version patches makeFlags;

  inherit src;

  depsBuildBuild = [ buildPackages.stdenv.cc ];
  nativeBuildInputs = [
    cross.gcc
    cross.binutils
    bc
    bison
    flex
    perl
    xz
    openssl
  ];

  # The compiler wrapper's hardening flags are for hosted userspace.
  hardeningDisable = [ "all" ];
  enableParallelBuilding = true;

  postPatch = applyPort + ''
    patchShebangs scripts
  '';

  configurePhase = ''
    runHook preConfigure
    make $makeFlags ${defconfig}
    sed 's|@INIT@|${earlyInit}/bin/dphone-early|' ${initramfsList} > initramfs.list
    scripts/config --file build/.config --set-str INITRAMFS_SOURCE "$PWD/initramfs.list"
    make $makeFlags olddefconfig
    grep -q '^CONFIG_INITRAMFS_COMPRESSION_NONE=y' build/.config
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
    make $makeFlags INSTALL_MOD_PATH=$out INSTALL_MOD_STRIP=1 DEPMOD=true modules_install
    rm -f $out/lib/modules/*/build $out/lib/modules/*/source
    runHook postInstall
  '';

  dontStrip = true;
  dontPatchELF = true;

  passthru.source = source;
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
