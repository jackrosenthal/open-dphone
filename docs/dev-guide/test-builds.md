# Test builds

Open DPhone has two ways to run new code on a phone without writing to flash.  Both require a phone that already runs Open DPhone with [SSH](ssh-and-serial-console.md#ssh) turned on.

| | Binary deploy | RAM boot |
| --- | --- | --- |
| Command | `cargo xtask deploy` on the workstation | `dphone ramboot` on the phone |
| Replaces | The main role | The kernel, the device tree, and the whole root file system |
| Keeps | The kernel, the init role, and the root file system | nvdata |
| Undo | Reboot | Reboot, power cycle, or wait for the time limit |

Flashing an image is the slowest way to test a change and, for a kernel change, the riskiest.  The boot loader has no watchdog and no fallback, so a kernel in flash that hangs before the firmware starts leaves a phone that only a JTAG adapter can recover.

## Binary deploy

For a change to the main role, i.e., anything in the `dphone` binary other than the init role, one command builds the binary for the phone, copies the binary over SSH, and restarts the main role:

```sh
cargo xtask deploy 192.0.2.20
```

The argument is the phone's address or host name.  The login is `root` unless the argument has the form `LOGIN@HOST`.  Pass `--no-build` to push the binary from the previous build.

The command copies the binary to `/tmp/dphone` on the phone.  Whenever that file exists, the init role runs `/tmp/dphone` as the main role in place of the installed binary.  `/tmp` is in RAM, so a reboot restores the installed binary.  To restore the installed binary without a reboot:

```sh
ssh root@192.0.2.20 'rm /tmp/dphone && dphone restart'
```

If a pushed binary fails three times in a row, each time within 30 seconds of starting, the init role ignores the pushed binary until the next boot and runs the installed binary again.

### Limitations

A pushed binary does not update the following:

- **The init role.**  Process 1 remains the installed binary.  A change to the init role requires an image.
- **The built-in apps.**  The apps are compiled files in the root file system, tied to the version of the app runtime in the binary.  If a change alters the runtime, the pushed binary refuses to load the installed apps.
- **The web frontend, unless built.**  The binary embeds `web/dist`.  Run `pnpm build` in `web/` before deploying, or the pushed binary serves a placeholder page.  See [Web frontend](building.md#web-frontend).

## RAM boot

`dphone ramboot` boots an image's kernel, device tree, and root file system from RAM.  Nothing is written to the firmware banks or to the `ctrl` partition.  Use the command to test a kernel before flashing, or to test a whole image without a flash write.

The image is the same EFF image that the build produces and that `dphone update install` installs.  The command accepts only an Open DPhone image for the phone's model.

While loading, the command briefly needs enough free memory for two copies of the image, and a copy in `/tmp` would use up that memory.  Pipe the image to the command over SSH so that no copy is stored on the phone:

```sh
ssh root@192.0.2.20 dphone ramboot - < result
```

`result` is the link that `nix build .#eff-d60` creates.  The command verifies the image, prints the memory layout and the kernel command line, and hands control to the new kernel.  The SSH session ends, and the phone is reachable again in about a minute.

In the RAM-booted system:

- nvdata is the phone's real nvdata.  The settings, the network address, and the SSH keys are unchanged, so the same `ssh` command works.  Anything that the RAM-booted system writes to nvdata persists after the phone returns to the installed firmware.
- `dphone update status` reports that the phone is running from neither bank.
- A pending trial boot is neither counted nor confirmed.
- The full `dphone update` command is available.  Once an image has proved to work, you can [install the image](firmware-updates.md#dphone-update-install) from the RAM-booted system.

### Return to the installed firmware

A RAM-booted system returns to the installed firmware in any of these cases:

- You run `dphone reboot` or power-cycle the phone.
- The kernel hangs.  The hardware watchdog keeps running across the handover and resets the phone.
- The time limit expires.  The limit is 900 seconds unless `--max-uptime` sets another.

The time limit ensures that an unreachable system, e.g., one without a working network, gives the phone back without intervention.  To stay past the limit, create the file `/run/dphone/stay` in the RAM-booted system:

```sh
ssh root@192.0.2.20 touch /run/dphone/stay
```

### Options

| Option | Description |
| --- | --- |
| `--dry-run` | Verifies the image, prints what would be loaded and passed to the kernel, and stops. |
| `--max-uptime <seconds>` | Sets the time limit.  The default is 900.  A value of 0 disables the limit. |
| `--append <arguments>` | Adds kernel parameters, e.g., `amba_pl011.console_output=1` to get [early kernel messages](ssh-and-serial-console.md#early-kernel-messages) on the serial console. |

For example, to boot an image with no time limit:

```sh
ssh root@192.0.2.20 dphone ramboot --max-uptime 0 - < result
```

### Kernel requirements

A kernel configuration change can break `dphone ramboot` for later runs.  The command depends on the following:

- The running kernel must have `CONFIG_KEXEC`.
- The new kernel must have `CONFIG_MTD_PHRAM`, because the root file system is placed in RAM and presented to the kernel as a flash device.
- The new kernel must keep the watchdog from expiring until the init role opens `/dev/watchdog`.  `CONFIG_WATCHDOG_OPEN_TIMEOUT` allows 120 seconds, and a system that takes longer to start is reset.
- The new kernel receives the parameter `bcmring.coldsim`, which makes the kernel reset the peripherals early so that drivers find the hardware in the power-on state.

See `kernel/README.md` for details, and `crates/dphone-update/src/ramboot.rs` for how the image is laid out in memory.
