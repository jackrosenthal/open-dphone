# Dev Guide

The Open DPhone firmware is a mainline Linux kernel and a single Rust program, `dphone`, that runs as both init and the phone application.  This guide covers building the firmware from source, running a build on a phone, and debugging a phone that runs the build.

## Architecture

`dphone` is installed at `/sbin/init` on the phone and takes on a different role depending on how the program is started.

| Component | Description |
| --- | --- |
| Init role | `dphone` running as process 1.  The init role mounts the file systems, loads the kernel modules, feeds the hardware watchdog, and starts and supervises the main role. |
| Main role | `dphone main`, the phone application: the user interface, SIP, audio, the DPMA client, the app runtime, the web interface, and the SSH server. |
| Early-boot stage | `dphone-early`, a small program built into the kernel image.  The stage runs before the root file system is mounted and counts the boots of a newly installed image. |
| Firmware banks | Two sets of flash partitions, bank A (`k0` and `root0`) and bank B (`k1` and `root1`).  Each bank holds a kernel and a root file system.  The `ctrl` partition records which bank the boot loader starts. |
| nvdata | The flash partition mounted at `/nvdata`, which holds everything that survives an update: the settings, the SSH keys, the installed apps, and the phone's MAC address.  Both banks share nvdata, and so does stock firmware. |
| EFF image | A firmware image file (`.eff`) for one model, in the stock firmware image format. |

## Development workflow

Each step below takes longer than the one before but tests more, so a change typically moves down the list.

1. Run the tests and the [simulator](simulator.md) on the workstation.
2. Push the changed `dphone` binary to a phone with [`cargo xtask deploy`](test-builds.md#binary-deploy).  A reboot restores the installed binary.
3. Boot the whole image from RAM with [`dphone ramboot`](test-builds.md#ram-boot).
4. Install the image with [`dphone update install`](firmware-updates.md#dphone-update-install).

!!! danger "Always boot a changed kernel from RAM first"

    A kernel in flash that hangs early in the boot cannot be replaced from the phone.  Recovery requires a JTAG adapter.
