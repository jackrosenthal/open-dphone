# Kernel

Mainline Linux v7.3 plus the BCMRING port for the Digium D-series phones.

| Path | Contents |
| --- | --- |
| `patches/NNNN-<group>.patch` | Changes to files that exist in v7.3, one patch per subsystem group.  Apply in order with `patch -p1`. |
| `files/` | Files the port adds, at their kernel-tree paths.  Copy over the tree. |
| `configs/` | Defconfigs; they belong in `arch/arm/configs/`.  `bcmring_tiny_defconfig` is the one that ships.  `bcmring_defconfig` is a larger debugging config that does not fit the 2 MiB slot. |

## Working on the port

The port is developed in a normal git checkout of tag `v7.3`, outside this repository, and this directory is regenerated from it.  To get such a tree with the port applied:

```text
git clone --depth 1 --branch v7.3 https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git ~/linux-bcmring
cp -r --no-preserve=mode "$(nix build .#kernel-source --no-link --print-out-paths)"/. ~/linux-bcmring/
```

`.#kernel-source` is v7.3 with `patches/` applied and `files/` and `configs/` copied in, and nothing built.  With a rootless Nix whose store is not at `/nix/store`, prefix the printed path with the store's root directory.  `HEAD` stays at v7.3, so `git status` and `git diff` in the tree show exactly the port.

After editing, regenerate this directory and build:

```text
cargo xtask kernel-sync ~/linux-bcmring           # rewrite patches/, files/, configs/
nix build .#kernel                                # zImage, DTBs, modules
nix build .#eff-d60                               # the whole image
```

`kernel-sync` reads `git diff HEAD` and the untracked files of the tree, so the tree must be a git checkout whose `HEAD` is v7.3.  It is the only kernel command in `xtask`; every build is a Nix derivation.

## Build

`nix/kernel.nix` fetches the v7.3 tarball, applies the port, and runs:

```text
make O=build ARCH=arm CROSS_COMPILE=arm-none-eabi- LOCALVERSION= \
     KBUILD_BUILD_USER=dphone KBUILD_BUILD_HOST=open-dphone KBUILD_BUILD_TIMESTAMP=1970-01-01 \
     bcmring_tiny_defconfig
make ... zImage dtbs modules
make ... modules_install INSTALL_MOD_PATH=$out INSTALL_MOD_STRIP=1 DEPMOD=true
```

`LOCALVERSION=` (empty, on the command line) fixes the release string at `7.3.0`.  Any change under `kernel/` rebuilds the kernel from scratch (about a minute); a pure build cannot reuse objects from an earlier one.

Flake outputs:

| Output | Contents |
| --- | --- |
| `.#kernel` | `zImage` (no DTB appended), `dtbs/<board>.dtb` for the seven boards, `lib/modules/7.3.0/kernel/...` (stripped; there is no `modules.dep`, see "Module load order"), and `config` (the full `.config` used). |
| `.#kernel-<model>` | `kernel.bin`: zImage followed by that model's DTB, zero-padded to a multiple of 4 bytes, with the header patched for boot2 (see "Boot image header").  This is what goes in the `k0`/`k1` slot. |
| `.#kernel-source` | The patched tree, unbuilt. |

The zImage is the same for all seven models; only the DTB differs.  `dphone-image kernel-image` fails the build if an image does not fit the 2 MiB slot.  The D60 image is about 1.76 MB, which leaves about 335 KiB: room for two bad blocks and about 73 KiB more.

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
| Network | `eth0` is the DSA conduit (do not put addresses on it); `lan` and `pc` are the two jacks.  Bridge and 802.1Q VLAN (with bridge VLAN filtering) are built in.  All three start with a random MAC address.  The 802.1p priority given to untagged frames received on `lan` or `pc` is the DCB default priority of the port (`dcb app replace dev pc default-prio 3`); it lands in bits 15:13 of the port default tag (switch page 0x34, register 0x10 + 2 × port), beside the port VLAN ID. |
| Keys | evdev device named `bcmring-keypad` (matrix) and a `gpio-keys` device with the hook switch (`KEY_PHONE`, down while the handset is on the cradle).  Codes are the `DIGIUM_KEYS_PROPOSED` set in `files/arch/arm/boot/dts/broadcom/digium-keys.h`.  Find the devices by capability; the event numbers are not fixed. |
| Lamps | LED class devices under `/sys/class/leds/` (e.g., `msg-green`, `line0-red`), on/off only. |
| Backlight | `/sys/class/backlight/*`; maximum 32 (color) or 31 (gray). |
| Display, new family | DRM device `card0` (PL111), XRGB8888 480x272, built in.  No fbdev. |
| Display, old family | DRM device from the `digium-gray-lcd` module (needs `spi-pl022` first); contrast under `/sys/class/lcd/`. |
| Audio | ALSA card "Digium Phone" (modules): PCM 0 speakerphone, 1 handset, 2 headset. |
| DSP | `/dev/bcmring-vpm` (misc device) and a remoteproc instance; firmware `dspapp1107_lcc.bin` from `/lib/firmware`.  The remoteproc does not start by itself. |
| Watchdog | `/dev/watchdog` (SP805).  boot2 hands off with it disabled; it starts when opened. |
| Random numbers | `/dev/hwrng`. |
| Bluetooth, D65 only | HCI device `hci0` from `hci_uart` (BCM20702 on UART B, `ttyAMA1`, H4 at 115200 baud).  No patch file is loaded: the stock firmware does not load one either, and none is shipped.  ALSA card "Digium Bluetooth": one mono S16_LE PCM device at 8 or 16 kHz on the SoC PCM/TDM interface, which drives the bit clock (1.024 MHz) and frame sync, so the controller must be set to slave on both. |
| Crash log | pstore/ramoops, 16 KiB of on-chip RAM that survives a reset (not DDR, which does not).  Mount `pstore` on `/sys/fs/pstore`: `dmesg-ramoops-0` is the last 8 KiB of the kernel log at a panic or oops, and exists only after one; `console-ramoops-0` is the last 8 KiB of the previous boot's console, and exists after every warm restart.  `echo c > /proc/sysrq-trigger` panics the kernel for a test (the keyboard and serial SysRq paths are off). |

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

D65 only, for Bluetooth, after the list above and only when Bluetooth is wanted:

1. `crypto/crypto.ko`
2. `crypto/crypto_algapi.ko`
3. `crypto/kpp.ko`
4. `crypto/ecc.ko`
5. `crypto/ecdh_generic.ko`
6. `lib/crc/crc16.ko`
7. `lib/crypto/libcryptoutils.ko`
8. `lib/crypto/libaes.ko`
9. `net/bluetooth/bluetooth.ko`
10. `net/bluetooth/rfcomm/rfcomm.ko`
11. `drivers/bluetooth/btbcm.ko`
12. `drivers/bluetooth/hci_uart.ko`
13. `sound/soc/codecs/snd-soc-bt-sco.ko`
14. `sound/soc/generic/snd-soc-simple-card-utils.ko`
15. `sound/soc/generic/snd-soc-simple-card.ko`
16. `sound/soc/bcm/snd-soc-bcmring-pcm.ko`

`regmap-i2c.ko` and `regmap-spi.ko` are built as a side effect and are not needed.

## Testing a kernel without flashing it

The RAM-boot loader in `~/d60-re/kernel-re/ramboot` (see its `NOTES.md`, section 6) starts a mainline image out of the running stock firmware.  It accepts an unpatched image (`zImage` from `.#kernel` followed by `dtbs/bcm11189-digium-d60.dtb`, zero-padded to a multiple of 4 bytes) with the word `any-image` in `/var/ramboot/options`, and `/var/ramboot/cmdline` replaces the command line, so `root=` can point at any partition.  The stock watchdog is running at that hand-off; this kernel keeps it fed for 120 s (`CONFIG_WATCHDOG_OPEN_TIMEOUT`) and then stops unless userspace has opened `/dev/watchdog`, so a boot that never reaches a working init returns to stock by itself.  A panic reboots after 10 s through the kernel's own restart path, which has not been exercised yet.

## Status

Run on hardware (D60 and D40, RAM boot only, with the larger config): the platform, Ethernet, display, sound, DSP, LEDs, and the ramoops console.  Not run: the NAND driver, this exact config (bridge, VLAN, high-resolution timers, and the options removed for size), and the `DIGIUM_KEYS_PROPOSED` key codes.  Nothing has booted from flash.  The D45, D50, D62, D65, and D70 devicetrees are extrapolated.  The D65 Bluetooth devicetree and the PCM/TDM driver (`bcmring-pcm.c`) have never run: no D65 was available.
