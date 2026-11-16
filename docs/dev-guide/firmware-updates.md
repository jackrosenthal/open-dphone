# Firmware updates

## Firmware banks

A phone's flash holds two complete firmware installations:

| Bank | Kernel partition | Root file system partition |
| --- | --- | --- |
| A | `k0` | `root0` |
| B | `k1` | `root1` |

A record in the `ctrl` partition selects the bank that the boot loader starts.  An installation always writes to the inactive bank and then selects that bank, so the running firmware stays intact in the other bank.  Both banks share nvdata.

The boot loader starts whichever bank the record selects and never falls back.  Rolling back after a failed update is the firmware's job; see [Trial boot](#trial-boot).

## Installation from stock firmware

A phone running stock firmware installs an Open DPhone image like any other firmware update, either through the stock web interface or through DPMA provisioning.  See [Installation](../index.md#installation) for the steps.  Stock firmware writes the image to the inactive bank, and stock firmware remains in the other bank.

!!! warning "The first boot from stock firmware has no trial"

    Stock firmware does not record a trial boot, so nothing returns the phone to stock firmware if the installed image fails to start.  Install only released images from stock firmware.  Test development builds from a working Open DPhone installation with [`dphone ramboot`](test-builds.md#ram-boot).

nvdata is shared with stock firmware, so the phone keeps the network settings and the web interface password.  The SSH keys in `/nvdata/root/.ssh/authorized_keys` are accepted once [SSH is turned on](ssh-and-serial-console.md#ssh).

## Provisioned firmware

A phone provisioned by a DPMA server installs the firmware that the PBX configuration offers, just as a phone running stock firmware does.  On a development phone, the PBX's image therefore replaces a test image at the next provisioning.  To make the phone ignore the PBX's firmware entries, do either of the following:

- On the **System** page of the web interface, turn on **Ignore DPMA firmware updates** in the **Firmware** card.
- With [Open DPMA](../open-dpma/index.md), set [`dphone_ignore_provisioned_firmware=yes`](../open-dpma/open-dphone-options.md#dphone_ignore_provisioned_firmware) in the phone's section of `res_digium_phone.conf`.

## Installation from Open DPhone

### Web interface

On the **System** page, the **Firmware** card shows the running version and the active bank.

1. Click **Upload Firmware** and choose an `.eff` file.  The phone writes the upload to the inactive bank, verifies the bank, and then shows the upload's version, model, and size.
2. Click **Install** to select the new image, or **Discard** to leave the selection unchanged.
3. Reboot the phone to start the new image.

### `dphone update install` { #dphone-update-install }

`dphone update install` reads an image from standard input, a URL, or a file.  In each case, the command writes the image to the inactive bank, reads the image back to verify the write, and selects the bank for the next boot.

=== "Standard input"

    ```sh
    ssh root@192.0.2.20 dphone update install - < result
    ```

    The image goes from the workstation straight into flash, and no copy is stored on the phone.  `result` is the link that `nix build .#eff-d60` creates.

=== "URL"

    ```sh
    dphone update install http://192.0.2.5:8000/3_0_0_D60_firmware.eff
    ```

    The phone downloads the image straight into flash.  Both `http://` and `https://` URLs work.  The command does not verify an HTTPS server's certificate, but does verify the checksums inside the image.

=== "File"

    ```sh
    scp result root@192.0.2.20:/tmp/image.eff
    ssh root@192.0.2.20 dphone update install /tmp/image.eff
    ```

    The command reads and verifies the whole file before writing anything.  `/tmp` is in the phone's RAM, so delete the file afterward.

After a successful installation, reboot the phone to start the new image:

```sh
ssh root@192.0.2.20 dphone reboot
```

An image from standard input or a URL cannot be read twice, so the checks that need the whole image run after the write.  If such an image turns out to be truncated or corrupt, the inactive bank has lost the previous contents and cannot be selected.  Nothing else changes: the running bank, the selection, and the trial record are as before.

The command rejects an image built for another model.  Only one update runs at a time, and no update can start while an installed image is still on trial; see [`dphone update abandon`](#dphone-update-abandon).

| Option | Description |
| --- | --- |
| `--bank a` or `--bank b` | Selects the bank to write.  The default is the inactive bank.  The command refuses to write the running bank. |
| `--allow-unbootable-other` | Proceeds even when the bank that is not being written is unbootable.  Intended for recovery work from a [RAM-booted](test-builds.md#ram-boot) system. |

## Trial boot

An Open DPhone image installed by `dphone update install` or the web interface starts on trial.  Before selecting the new bank, the updater writes a trial record to nvdata (`/nvdata/open/trial`).  The record is removed only after the new image proves healthy.

- **Each boot of the new bank is counted.**  The early-boot stage in the new kernel image counts the boot before the root file system is needed.  A kernel panic, a watchdog reset, or a crash of the init role each uses up one boot.  After three boots without confirmation, the stage selects the previous bank and restarts the phone.
- **An unusable root file system ends the trial immediately.**  If the root file system cannot be mounted or has no init program, the stage returns to the previous bank without using up the remaining boots.
- **A healthy main role confirms the trial.**  The init role removes the record when the main role reports that the phone is healthy.  `dphone health` shows the conditions behind that report.
- **A main role that keeps failing ends the trial.**  Five failures of the main role within one trial boot return the phone to the previous bank.  If the main role does not become healthy within 300 seconds, the phone restarts, and the boot still counts.

While a trial is pending, the **Firmware** card shows the number of boots left.  Click **Keep** to confirm the image manually, or **Roll Back** to return to the previous firmware.  After an automatic rollback, the card reports that the last update was rolled back, and why.

!!! danger "The trial does not protect against a kernel that fails early"

    If a kernel stops before the early-boot stage runs, the boot loader starts the same kernel again at every power-on, and the phone stays down until the flash is rewritten through a JTAG adapter.  Always [boot an image with a changed kernel from RAM](test-builds.md#ram-boot) before installing the image.

A stock firmware image installed from Open DPhone gets no trial, because stock firmware does not count trial boots.

## Bank management

### `dphone update status` { #dphone-update-status }

```sh
dphone update status
```

The command reads both banks back and reports:

- the bank that the phone is running from, the bank that the `ctrl` record selects, and the bank that the boot loader will start next,
- whether the `ctrl` record and the record's backup copies are valid,
- for each bank: the version, whether the boot loader would start the bank, the number of bad blocks, and whether the kernel and the root file system match the lengths and checksums in the bank's header, and
- the pending trial, if any.

The boot loader checks only the header and the first words of the kernel.  The content comparison is therefore the only evidence that a bank holds the image described by the header.

### `dphone update switch-bank` { #dphone-update-switch-bank }

```sh
dphone update switch-bank b
dphone reboot
```

`switch-bank` selects bank `a` or bank `b` for the next boot without writing an image.  The command fails unless the named bank holds an image that the boot loader would start.  Use `switch-bank` to move quickly between two installed images, e.g., back to the stock firmware that the first installation left in the other bank.

`switch-bank` removes any trial record, so a bank selected with `switch-bank` is not on trial.

### `dphone update arm-trial` { #dphone-update-arm-trial }

```sh
dphone update switch-bank b
dphone update arm-trial
dphone reboot
```

`arm-trial` puts the bank selected for the next boot on trial.  Use the command for an image that was selected by some means other than an installation, e.g., with `switch-bank`.  The phone then returns to the other bank if the image does not come up healthy within three boots.  Run `switch-bank` first, because `switch-bank` removes the trial record.

By default, the phone returns to the running bank.  If the phone is running from the selected bank, or from neither bank after a RAM boot, name the bank to return to with `--return-to a` or `--return-to b`.  The command refuses to put stock firmware on trial.

### `dphone update abandon` { #dphone-update-abandon }

```sh
dphone update abandon
```

`abandon` cancels a trial: the command selects the bank that the phone would roll back to and removes the trial record, so that another image can be installed.  If the phone is already running the abandoned image, reboot afterward.

Use `abandon` when you find a mistake in an image that you just installed, either before or after the reboot.  You can then install a corrected image right away without waiting for the trial to end.

## Return to stock firmware

How to return a phone to stock firmware depends on what the other bank holds:

- If stock firmware is still in the other bank, [switch to that bank](#dphone-update-switch-bank).
- Otherwise, install a stock firmware image for the phone's model through the web interface or with `dphone update install`.  Both accept stock images, and the web interface asks for confirmation first.

Stock firmware has no command to select the other bank.  Getting from stock firmware back to Open DPhone therefore takes another [installation from stock firmware](#installation-from-stock-firmware), even when an Open DPhone image is still in the other bank.
