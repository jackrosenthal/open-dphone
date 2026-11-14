# Boot images, the root file system, and the per-model EFFs.  Every step is
# one call of the `dphone-image` tool; the root file system's contents are
# listed here and laid out with plain installs.
{
  lib,
  runCommand,
  dphone-image,
  kernel,
  dphone,
  brush,
  coreutils,
  applets,
  dspFirmware,
  apps ? null,
  version, # e.g., "3.0.0"
}:
let
  # Stock-style version string: 3_0_0.
  wireVersion = lib.replaceStrings [ "." ] [ "_" ] version;

  models = {
    d40 = {
      name = "D40";
      family = "old";
    };
    d45 = {
      name = "D45";
      family = "old";
    };
    d50 = {
      name = "D50";
      family = "old";
    };
    d70 = {
      name = "D70";
      family = "old";
    };
    d60 = {
      name = "D60";
      family = "new";
    };
    d62 = {
      name = "D62";
      family = "new";
    };
    d65 = {
      name = "D65";
      family = "new";
    };
  };

  # zImage + the model's DTB, padded, header-patched for boot2 (load address
  # 0x02000000).  The tool fails when the result does not fit the 2 MiB
  # k0/k1 slot.
  kernelImage =
    model: info:
    runCommand "kernel-${model}-${version}" { nativeBuildInputs = [ dphone-image ]; } ''
      mkdir -p $out
      dphone-image kernel-image \
        --model ${model} \
        --zimage ${kernel}/zImage \
        --dtb ${kernel}/dtbs/${kernel.boards.${model}}.dtb \
        --load 0x02000000 \
        --out $out/kernel.bin
    '';

  # The root file system tree.  It is the same for every model: both DSP
  # firmware variants are present, each in its family's directory, and the
  # init role points the kernel's firmware loader at the right one.
  rootfsTree = runCommand "dphone-rootfs-tree-${version}" { } ''
    mkdir -p $out/{bin,sbin,etc/dphone,lib/firmware/old-family,lib/firmware/new-family,usr/share/dphone/apps}
    mkdir -p $out/{dev,proc,sys,tmp,run,var,nvdata,root}

    install -m 0755 ${dphone}/bin/dphone $out/sbin/init
    ln -s ../sbin/init $out/bin/dphone
    # System commands `dphone` provides under its own name.
    for c in mount umount insmod rmmod lsmod modprobe reboot poweroff halt dmesg; do
      ln -s ../sbin/init $out/bin/$c
    done

    install -m 0755 ${brush}/bin/brush $out/bin/brush
    ln -s brush $out/bin/sh
    install -m 0755 ${coreutils}/bin/coreutils $out/bin/coreutils
    for a in ${lib.escapeShellArgs applets}; do
      ln -s coreutils $out/bin/$a
    done
    ln -s test "$out/bin/["

    cp -r ${kernel}/lib/modules $out/lib/modules

    install -m 0644 ${dspFirmware.old} $out/lib/firmware/old-family/dspapp1107_lcc.bin
    install -m 0644 ${dspFirmware.new} $out/lib/firmware/new-family/dspapp1107_lcc.bin

    ${lib.optionalString (
      apps != null
    ) "cp -r ${apps}/. $out/usr/share/dphone/apps/ && chmod -R u+w $out/usr/share/dphone/apps"}

    printf 'root:x:0:0:root:/nvdata/root:/bin/brush\n' > $out/etc/passwd
    printf 'root:x:0:\ntty:x:5:\n' > $out/etc/group
    printf '/bin/brush\n/bin/sh\n' > $out/etc/shells
    printf 'Open DPhone ${wireVersion}\n' > $out/etc/dphone-release
  '';

  # JFFS2 image of the tree: little-endian, 128 KiB erase blocks, all files
  # owned by root.
  rootfs = runCommand "dphone-rootfs-${version}.jffs2" { nativeBuildInputs = [ dphone-image ]; } ''
    dphone-image jffs2 build ${rootfsTree} $out \
      --mtime 0 \
      --node dev/console:c:5:1:600 \
      --node dev/null:c:1:3:666
  '';

  eff =
    model: info:
    runCommand "${wireVersion}_${info.name}_firmware.eff" { nativeBuildInputs = [ dphone-image ]; } ''
      dphone-image image build \
        --model ${info.name} \
        --version ${wireVersion} \
        --kernel ${kernelImage model info}/kernel.bin \
        --rootfs-image ${rootfs} \
        --mtime 0 \
        --out $out
    '';
in
{
  inherit models rootfsTree rootfs;
  kernelImages = lib.mapAttrs kernelImage models;
  effs = lib.mapAttrs eff models;
}
