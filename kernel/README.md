# Kernel

Mainline Linux v7.3 plus the BCMRING port for the Digium D-series phones.

| Path | Contents |
| --- | --- |
| `patches/NNNN-<group>.patch` | Changes to files that exist in v7.3, one patch per subsystem group.  Apply in order with `patch -p1`. |
| `files/` | Files the port adds, at their kernel-tree paths.  Copy over the tree. |
| `configs/` | Defconfigs; they belong in `arch/arm/configs/`.  `bcmring_tiny_defconfig` is the one that ships.  `bcmring_defconfig` is a larger debugging config that does not fit the 2 MiB slot. |

## Working on the port

Develop in a normal git checkout of tag `v7.3`, then regenerate this directory from it.  All three commands live in `xtask/src/kernel.rs`.

```text
cargo xtask kernel-prepare <pristine v7.3 tree>   # apply patches, files, and configs
cargo xtask kernel-sync <working tree>            # regenerate patches/, files/, configs/
cargo xtask kernel-build <tree> <out>             # build and collect the results
```

`kernel-sync` reads `git diff HEAD` and the untracked files of the tree, so the tree must be a git checkout whose `HEAD` is v7.3.

## Build

`kernel-build` runs the following; the Nix derivation runs the same steps.

```text
make O=<out>/obj ARCH=arm CROSS_COMPILE=arm-none-eabi- LOCALVERSION= \
     KBUILD_BUILD_USER=dphone KBUILD_BUILD_HOST=open-dphone \
     bcmring_tiny_defconfig
make ... zImage dtbs modules
make ... modules_install INSTALL_MOD_PATH=<out>/modules INSTALL_MOD_STRIP=1 DEPMOD=true
```

`LOCALVERSION=` (empty, on the command line) fixes the release string at `7.3.0`; without it a git checkout builds `7.3.0-dirty`.

Output layout:

| Path | Contents |
| --- | --- |
| `<out>/images/<model>-zImage-dtb.raw` | zImage followed by that model's DTB, zero-padded to a multiple of 4 bytes.  **Not yet bootable from flash:** see "Boot image header" below. |
| `<out>/images/<model>.dtb`, `<out>/images/zImage` | The parts. |
| `<out>/modules/lib/modules/7.3.0/kernel/...` | Stripped modules.  There is no `modules.dep`; see "Module load order". |
| `<out>/config` | The full `.config` used. |

The zImage is the same for all seven models; only the DTB differs.  The build fails if any image is larger than the 2 MiB slot less one 128 KiB bad block.

| Model | DTB | Image bytes (2026-11-13) |
| --- | --- | --- |
| D40 | `bcm11170-digium-d40.dtb` | 1,759,612 |
| D45 | `bcm11109-digium-d45.dtb` | 1,759,564 |
| D50 | `bcm11170-digium-d50.dtb` | 1,761,644 |
| D60 | `bcm11189-digium-d60.dtb` | 1,760,488 |
| D62 | `bcm11188-digium-d62.dtb` | 1,761,408 |
| D65 | `bcm11188-digium-d65.dtb` | 1,761,408 |
| D70 | `bcm11109-digium-d70.dtb` | 1,762,132 |

That leaves about 335 KiB in the slot: room for two bad blocks and about 73 KiB more.

## Boot image header

boot2 copies the image from `k0`/`k1` to the address in zImage header word `0x28`, for a length of word `0x2c` minus word `0x28`, and enters it there.  A mainline zImage has 0 and its own length (without the DTB) in those words, so the image tool (`dphone-image`) must set, little-endian:

- word `0x28` = load address `0x02000000`;
- word `0x2c` = load address + length of the whole file (zImage + DTB + padding).

The word at `0x24` is the zImage magic `0x016f2818` and must be checked first.  This is what `~/d60-re/kernel-re/boot/d60bootimg.py zimage --load 0x02000000` does; its `check_span` also lists what the load range must not overlap.  Reference pair for testing the Rust tool: `~/d60-re/kernel-re/build/images/d60-tiny-zImage-dtb.raw` (input) and `d60-tiny-k.bin` (output); they differ only in those two words.

The RAM-boot loader (`~/d60-re/kernel-re/ramboot`) takes the unpatched `.raw` file and refuses a patched one.

## What userspace sees

Kernel command line: the DTB's `console=ttyAMA0,115200n8 earlycon=pl011,0x80012000 panic=10`, then boot2's line, e.g.,

```text
console=ttyAMA0,115200n8 vmalloc=256m  digium_board=D60 digium_boot_rev=3 mtdparts=bcm_umi-nand:... root=/dev/mtdblock7 rootfstype=jffs2 ro bootmemheap
```

The kernel passes what it does not recognize to init: `bootmemheap` arrives as `argv[1]` of `/sbin/init`, and `digium_board=D60` and `digium_boot_rev=3` arrive in its environment.

| Function | Name |
| --- | --- |
| MTD | Device `bcm_umi-nand`; partitions from `mtdparts=`: `boot1`, `boot2`, `ctrl`, `nvdata`, `k0`, `k1`, `root0`, `root1`, `userdata` (find them by name in `/proc/mtd`).  2 KiB pages, 128 KiB blocks, 2 OOB bytes available (JFFS2 cleanmarker). |
| Root | JFFS2 (zlib and rtime compressors only), read-only, `devtmpfs` mounted on `/dev` by the kernel. |
| Console | `ttyAMA0` (UART A); `ttyAMA1` is UART B. |
| Network | `eth0` is the DSA conduit (do not put addresses on it); `lan` and `pc` are the two jacks.  Bridge and 802.1Q VLAN (with bridge VLAN filtering) are built in.  All three start with a random MAC address. |
| Keys | evdev device named `bcmring-keypad` (matrix) and a `gpio-keys` device with the hook switch (`KEY_PHONE`, down while the handset is on the cradle).  Codes are the `DIGIUM_KEYS_PROPOSED` set in `files/arch/arm/boot/dts/broadcom/digium-keys.h`.  Find the devices by capability; the event numbers are not fixed. |
| Lamps | LED class devices under `/sys/class/leds/` (e.g., `msg-green`, `line0-red`), on/off only. |
| Backlight | `/sys/class/backlight/*`; maximum 32 (color) or 31 (gray). |
| Display, new family | DRM device `card0` (PL111), XRGB8888 480x272, built in.  No fbdev. |
| Display, old family | DRM device from the `digium-gray-lcd` module (needs `spi-pl022` first); contrast under `/sys/class/lcd/`. |
| Audio | ALSA card "Digium Phone" (modules): PCM 0 speakerphone, 1 handset, 2 headset. |
| DSP | `/dev/bcmring-vpm` (misc device) and a remoteproc instance; firmware `dspapp1107_lcc.bin` from `/lib/firmware`.  The remoteproc does not start by itself. |
| Watchdog | `/dev/watchdog` (SP805).  boot2 hands off with it disabled; it starts when opened. |
| Random numbers | `/dev/hwrng`. |
| Crash log | pstore/ramoops console, 16 KiB in on-chip RAM that survives a reset: mount `pstore` on `/sys/fs/pstore` and read `console-ramoops-0`. |

## Module load order

Load with `finit_module` in this order (no `modules.dep` is shipped).  Paths are under `/lib/modules/7.3.0/kernel/`.

Old family only (D40, D45, D50, D70), first, for the display:

1. `drivers/spi/spi-pl022.ko`
2. `drivers/video/backlight/lcd.ko`
3. `drivers/gpu/drm/drm_shmem_helper.ko`
4. `drivers/gpu/drm/tiny/digium-gray-lcd.ko`

All models:

1. `drivers/dma/dw/dw_dmac_core.ko`
2. `drivers/dma/dw/dw_dmac.ko`
3. `sound/soundcore.ko`
4. `sound/core/snd.ko`
5. `sound/core/snd-timer.ko`
6. `sound/core/snd-pcm.ko`
7. `sound/core/snd-pcm-dmaengine.ko`
8. `sound/soc/snd-soc-core.ko`
9. `sound/soc/codecs/snd-soc-simple-amplifier.ko`
10. `sound/soc/bcm/snd-soc-bcmring-apm.ko`
11. `sound/soc/bcm/snd-soc-bcmring-digium-card.ko`
12. `drivers/remoteproc/bcmring_vpm.ko` (after `dw_dmac`)

`regmap-i2c.ko` and `regmap-spi.ko` are built as a side effect and are not needed.

## Testing a kernel without flashing it

The RAM-boot loader in `~/d60-re/kernel-re/ramboot` (see its `NOTES.md`, section 6) starts a mainline image out of the running stock firmware.  It accepts `<out>/images/d60-zImage-dtb.raw` with the word `any-image` in `/var/ramboot/options`, and `/var/ramboot/cmdline` replaces the command line, so `root=` can point at any partition.  The stock watchdog is running at that hand-off; this kernel keeps it fed for 120 s (`CONFIG_WATCHDOG_OPEN_TIMEOUT`) and then stops unless userspace has opened `/dev/watchdog`, so a boot that never reaches a working init returns to stock by itself.  A panic reboots after 10 s through the kernel's own restart path, which has not been exercised yet.

## Status

Run on hardware (D60 and D40, RAM boot only, with the larger config): the platform, Ethernet, display, sound, DSP, LEDs, and the ramoops console.  Not run: the NAND driver, this exact config (bridge, VLAN, high-resolution timers, and the options removed for size), and the `DIGIUM_KEYS_PROPOSED` key codes.  Nothing has booted from flash.  The D45, D50, D62, D65, and D70 devicetrees are extrapolated.
