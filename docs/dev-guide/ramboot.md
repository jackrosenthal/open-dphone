# Trying a build without installing

Writing an image to flash is the slowest way to try a change, and for a kernel change the riskiest: the boot loader has no watchdog and no fallback, so a kernel in flash that hangs before the firmware starts leaves a phone that only a JTAG adapter can revive.  Open DPhone has two ways to run new code on a phone while the flash stays as found.  Both need a phone that already runs Open DPhone and has [SSH](debug-access.md#ssh) switched on.

| | Push a changed binary | Start an image from RAM |
| --- | --- | --- |
| Command | `cargo xtask deploy` on the workstation | `dphone ramboot` on the phone |
| What is replaced | The main role | The kernel, the device tree, and the whole root file system |
| What stays | The kernel, the init role, and the root file system | nvdata |
| The way back | A reboot | A reboot, a power cycle, or the time limit |

## Push a changed binary

For a change to the main role, i.e., to anything in the `dphone` binary apart from the init role, one command builds the binary for the phone, copies the binary over SSH, and restarts the main role:

```sh
cargo xtask deploy 192.0.2.20
```

The argument is the phone's address or host name.  The login is `root` unless the argument has the form `LOGIN@HOST`.  `--no-build` pushes the binary that the last build left.

The command copies the binary to `/tmp/dphone` on the phone.  The init role runs `/tmp/dphone` as the main role whenever that file exists, in place of the installed binary.  `/tmp` is held in RAM, so a reboot brings the installed binary back.  To go back without a reboot:

```sh
ssh root@192.0.2.20 'rm /tmp/dphone && dphone restart'
```

A pushed binary that fails three times in a row, each time within 30 seconds of starting, is ignored until the next boot, and the installed binary runs again.

Three things do not come with the pushed binary:

- **The init role.**  Process 1 stays the installed binary.  A change to the init role needs an image.
- **The built-in apps.**  The apps are compiled files in the root file system, tied to the version of the app runtime in the binary.  When a change alters the runtime, the pushed binary refuses to load the installed apps.
- **The web frontend, unless built.**  The binary embeds `web/dist`.  Run `pnpm build` in `web/` before the deploy, or the pushed binary serves a placeholder page (see [Building from source](building.md#the-cargo-workspace)).

## Start an image from RAM

`dphone ramboot` starts the kernel, the device tree, and the root file system of an image from RAM.  Nothing is written to the firmware banks or to the `ctrl` partition, so the command serves to try a kernel before the kernel goes into flash, and to try a whole image without a flash write.

The image is the EFF image that the build produces and that `dphone update install` installs.  Only an Open DPhone image for the phone's model is accepted.

Loading needs free memory for the image twice for a moment, and a copy of the image in `/tmp` would occupy the same RAM.  Feed the image to the command over SSH, so that no copy is stored on the phone:

```sh
ssh root@192.0.2.20 dphone ramboot - < result
```

`result` is the link that `nix build .#eff-d60` leaves.  The command verifies the image, prints the memory layout and the kernel command line, and hands over.  The SSH session ends, and the phone answers again in about a minute.

In the started system:

- **nvdata is the phone's nvdata.**  The settings, the network address, and the SSH keys are the same, so the same `ssh` command reaches the started system.  Whatever the started system writes to nvdata stays after the return to the installed firmware.
- **`dphone update status` reports that the phone runs from neither bank.**
- **A pending trial boot is neither counted nor confirmed.**

### The way back

The started system returns to the installed firmware:

- at `dphone reboot` or a power cycle,
- when the kernel hangs, because the hardware watchdog keeps running across the hand-over and resets the phone, and
- after a time limit, 900 seconds unless `--max-uptime` says otherwise.

The time limit exists so that a started system that cannot be reached, e.g., one without a working network, gives the phone back unattended.  To stay past the limit, create the file `/run/dphone/stay` in the started system:

```sh
ssh root@192.0.2.20 touch /run/dphone/stay
```

`--max-uptime 0` starts the image with no limit.

### Options

| Option | Effect |
| --- | --- |
| `--dry-run` | Verifies the image and prints what would be loaded and passed to the kernel, then stops. |
| `--max-uptime <seconds>` | Sets the time limit.  The default is 900, and 0 means no limit. |
| `--append <arguments>` | Adds kernel parameters, e.g., `amba_pl011.console_output=1` to see the kernel's messages on the [serial console](debug-access.md#the-serial-console-on-the-ehs-port) from the start of the boot. |

For example, to start an image with no time limit:

```sh
ssh root@192.0.2.20 dphone ramboot --max-uptime 0 - < result
```

### What the command depends on

A kernel change can break `dphone ramboot` for later runs, so a developer who changes the kernel configuration needs to know what the command relies on:

- The running kernel must have `CONFIG_KEXEC`.
- The started kernel must have `CONFIG_MTD_PHRAM`, because the root file system is placed in RAM and presented to the kernel as a flash device.
- The started kernel must keep the watchdog from expiring until the init role opens `/dev/watchdog`.  `CONFIG_WATCHDOG_OPEN_TIMEOUT` allows 120 seconds; a started system that takes longer is reset.
- The started kernel receives the parameter `bcmring.coldsim`, which makes the kernel reset the peripherals early, so that the drivers find the hardware as after a power-on.

`kernel/README.md` has the details, and `crates/dphone-update/src/ramboot.rs` explains how the image is laid out in memory.

### Install the image that was tried

A system started from RAM has the full `dphone update` command, so an image that has proved to work can be installed from the started system as well as from the installed firmware; see [Installing a firmware image](installing.md).
