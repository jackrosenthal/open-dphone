# Kernel

This directory holds the Open Desk Phone kernel: mainline Linux 7.3 with a port to the Broadcom BCMRING family of system-on-chip processors, which the Digium D40, D45, D50, D60, D62, D65, and D70 phones are built on.  The Linux source itself is not in this repository.  The build downloads the Linux 7.3 release and applies what is here.

## What is in this directory

| Path | Contents |
| --- | --- |
| `patches/` | Changes to files that exist in Linux 7.3, with one patch file for each kernel subsystem.  The build applies them in the order of their file names with `patch -p1`. |
| `files/` | Files that the port adds (drivers, device trees, device tree binding documents, and headers), stored at the paths they have in the kernel tree.  The build copies them into the patched tree. |
| `configs/` | Kernel configurations.  The build copies them to `arch/arm/configs/`.  The firmware uses `bcmring_tiny_defconfig`.  `bcmring_defconfig` is a larger configuration that the firmware build does not use. |

## Building

Nix builds the kernel, and `nix/kernel.nix` is the recipe.

| Command | Result |
| --- | --- |
| `nix build .#kernel` | The compressed kernel (`zImage`), one device tree for each model (`dtbs/`), the loadable modules (`lib/modules/7.3.0/`), and the full configuration that was used (`config`). |
| `nix build .#kernel-d60` | `kernel.bin`, the file that is written to the kernel partition of a D60.  There is one such output for each model. |
| `nix build .#kernel-source` | The Linux 7.3 tree with the port applied and nothing built. |
| `nix build .#eff-d60` | The complete firmware image for a D60, which contains `kernel.bin`. |

The compiled kernel is the same for all seven models, and only the device tree differs.  Any change under `kernel/` makes Nix build the whole kernel again, because a Nix build cannot reuse object files from an earlier build.

## Constraints to know about

- **The kernel partition is 2 MiB.**  The kernel and the device tree together must fit in it, and the build fails if they do not.  This is why several drivers (the grayscale display, the sound drivers, the DMA controller driver, and the voice processor driver) are built as modules and stored in the root file system.  The firmware loads them when it starts, from the list in `crates/opendeskphone/src/init/modules.rs`.
- **The kernel image carries the early-boot stage.**  The build puts one program, `crates/opendeskphone-early`, into the kernel as its initramfs (`CONFIG_INITRAMFS_SOURCE`, from `crates/opendeskphone-early/initramfs.list`).  It is the first program the kernel runs: it counts trial boots after a firmware update, selects the previous firmware bank when the new one does not come up, and otherwise mounts the root file system named by `root=` and runs its `/sbin/init`.  The kernel does not mount the root file system by itself any more.  A kernel configured without `CONFIG_INITRAMFS_SOURCE` (a developer's own build) mounts the root file system as before.
- **The boot loader needs a prepared image.**  The phone's boot loader reads the address to load the kernel at, and the length to load, from two words in the header of the kernel image.  A mainline `zImage` does not have these values.  The `kernel-<model>` outputs do: they hold the `zImage` followed by the model's device tree, with the two header words filled in by `opendeskphone-image` (`crates/opendeskphone-image/src/kernel.rs`).
- **The boot loader adds its own kernel command line.**  The command line starts with the `bootargs` property of the device tree (`files/arch/arm/boot/dts/broadcom/bcmring-digium-common.dtsi`).  The boot loader then appends the flash partition table (`mtdparts=`), the root file system (`root=`), and a few more words, and those cannot be changed.  To add a kernel parameter, add it to `bootargs`.
- **Flash partitions must be found by name.**  The partition table comes from the boot loader, and partition offsets differ between phones, so user space looks partitions up by name in `/proc/mtd` and never by number.
- **The serial console is silent by default.**  The phone's serial port is also the port for an electronic hook switch headset adapter, so the kernel writes no messages to it unless it is told to.  To see kernel messages on the serial port, add `amba_pl011.console_output=1` to the kernel command line, or write 1 to `/sys/module/amba_pl011/parameters/console_output` on a running phone.
- **The kernel log survives a reset.**  The kernel keeps the last 8 KiB of its console output, and the last 8 KiB of the kernel log from a panic, in a small memory that keeps its contents across a reset.  After the next boot, mount the `pstore` file system on `/sys/fs/pstore` to read them.
- **The boot loader has no watchdog and no fallback.**  It starts the kernel of the selected firmware bank with the watchdog timer switched off.  If a kernel in flash stops before the firmware starts, nothing resets the phone, and every later power-on starts the same kernel again.  The phone then does not boot until its flash is rewritten through a JTAG adapter.  Try a new kernel from RAM before installing it, as described below.

## Working on the kernel

The port is developed in an ordinary git checkout of the Linux `v7.3` tag, outside this repository.  The `patches/`, `files/`, and `configs/` directories are generated from that checkout.

To create the checkout with the port applied:

```text
git clone --depth 1 --branch v7.3 https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git ~/linux-bcmring
cp -r --no-preserve=mode "$(nix build .#kernel-source --no-link --print-out-paths)"/. ~/linux-bcmring/
```

The checkout's `HEAD` stays at `v7.3`, so `git status` and `git diff` in it show exactly the port.

After editing the checkout, regenerate this directory and build:

```text
cargo xtask kernel-sync ~/linux-bcmring
nix build .#kernel
```

`kernel-sync` replaces the contents of all three directories.  It writes every modified file of the checkout into a patch in `patches/`, with one numbered patch for each subsystem, copies every new file to `files/`, and copies the files under `arch/arm/configs/` to `configs/`.  It reads `git diff HEAD` and the list of untracked files, so the checkout's `HEAD` must be `v7.3`.

## Trying a kernel without installing it

On a phone that runs Open Desk Phone, this command starts the kernel, the device tree, and the root file system of a firmware image from RAM and writes nothing to flash:

```text
dphone ramboot IMAGE.eff
```

`IMAGE.eff` is the firmware image that `nix build .#eff-<model>` produces.  A reboot or a power cycle brings the installed firmware back.  If the new kernel stops, the watchdog timer resets the phone, and the installed firmware starts.  `dphone ramboot --help` lists the options, and `crates/opendeskphone-update/src/ramboot.rs` explains how the command works.

The command depends on the following parts of the kernel:

- The running kernel must be built with `CONFIG_KEXEC`, which lets one kernel start another.
- The new kernel must be built with `CONFIG_MTD_PHRAM`, which presents a range of RAM as a flash device.  The command places the image's root file system in RAM and passes its location in the `phram.phram=` kernel parameter.
- The watchdog timer keeps running while the new kernel starts.  The new kernel keeps it from expiring for 120 seconds (`CONFIG_WATCHDOG_OPEN_TIMEOUT`).  After that, the firmware must have opened `/dev/watchdog`, or the phone resets.
- The command passes the kernel parameter `bcmring.coldsim` to the new kernel.  With it, the kernel resets the peripherals and stops their clocks early in its start, so that the drivers find the peripherals as they are after a power-on, and not as the running system left them.
