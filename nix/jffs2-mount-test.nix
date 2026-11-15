# Mount a JFFS2 image with the kernel's own JFFS2 driver, on simulated NAND
# with the phone's geometry (128 MiB, 2 KiB pages, 128 KiB erase blocks),
# and compare what the kernel reads back with the tree the image was made
# from; then the same on a RAM-backed MTD device, which is how an image
# started with `dphone ramboot` has its root.  Any JFFS2 complaint in the
# kernel log fails the test.
{
  lib,
  vmTools,
  runCommand,
  linux,
  mtdutils,
  kmod,
  diffutils,
  findutils,
  util-linux,
  image, # the JFFS2 image
  tree, # the directory it was built from
}:
let
  modules = lib.getOutput "modules" linux;
in
vmTools.runInLinuxVM (
  runCommand "jffs2-mount-test"
    {
      nativeBuildInputs = [
        mtdutils
        kmod
        diffutils
        findutils
        util-linux
      ];
      memSize = 768;
    }
    ''
      # Samsung K9F1G08U0x: 128 MiB, 2048-byte pages, 128 KiB blocks.
      modprobe -d ${modules} nandsim id_bytes=0xec,0xf1,0x00,0x95
      modprobe -d ${modules} jffs2
      grep -q 'mtd0: 08000000 00020000' /proc/mtd

      flash_erase -q /dev/mtd0 0 0
      nandwrite -q -p /dev/mtd0 ${image}

      mkdir /mnt
      mount -t jffs2 -o ro mtd0 /mnt

      # Same names, types, link targets, and file contents.
      # (The two device nodes exist only in the image.)
      diff -r --no-dereference -x console -x null ${tree} /mnt
      # Everything owned by root, whatever the build user was.
      test -z "$(find /mnt ! -uid 0 -o ! -gid 0)"

      # Force every node to be read, then look for complaints.
      find /mnt -type f -exec cat {} + > /dev/null
      umount /mnt

      # The same image the way `dphone ramboot` runs it: on a RAM-backed
      # MTD device with 128 KiB erase blocks, exactly as long as the image
      # rounded up to whole blocks (so with no spare block at all), mounted
      # read-only.  mtdram stands in for phram: the same kind of device
      # (MTD_RAM, no OOB, readable in place), filled with 0xff.
      blocks=$(( ($(stat -c %s ${image}) + 131071) / 131072 ))
      modprobe -d ${modules} mtdram total_size=$((blocks * 128)) erase_size=128
      grep -q "mtd1: $(printf %08x $((blocks * 131072))) 00020000" /proc/mtd
      dd if=${image} of=/dev/mtd1 bs=131072 status=none
      mkdir /ram
      mount -t jffs2 -o ro mtd1 /ram
      diff -r --no-dereference -x console -x null ${tree} /ram
      find /ram -type f -exec cat {} + > /dev/null
      umount /ram
      if dmesg | grep -i jffs2 | grep -iv 'version 2' ; then
        echo "the kernel's JFFS2 driver complained about the image" >&2
        exit 1
      fi
      touch $out
    ''
)
