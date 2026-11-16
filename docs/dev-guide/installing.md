# Installing a firmware image

## The two firmware banks

A phone's flash holds two complete firmware installations:

| Bank | Kernel partition | Root file system partition |
| --- | --- | --- |
| A | `k0` | `root0` |
| B | `k1` | `root1` |

A record in the `ctrl` partition selects the bank that the boot loader starts.  An installation always writes the bank that is not running and then selects that bank, so the running firmware stays intact in the other bank.  nvdata is shared by both banks.

The boot loader starts whatever the record selects and never falls back.  The firmware returns the phone to the previous bank after a failed update; see [The trial boot](#the-trial-boot).

## From stock firmware

Open DPhone images have the format of stock firmware images, so a phone that runs stock firmware installs an Open DPhone image as any firmware: from a [firmware entry in the DPMA configuration on the PBX](../open-dpma/administration.md#moving-phones-between-stock-firmware-and-open-dphone), or from the firmware page of the stock web interface.  Stock firmware writes the image to the bank that is not running, and stock firmware stays in the other bank.

Two points matter to a developer:

- **The first boot from stock firmware has no trial.**  Stock firmware records no trial boot, so nothing returns the phone to stock firmware when the installed image fails to start.  Install a released image from stock firmware, and try builds from a working Open DPhone with [`dphone ramboot`](ramboot.md#start-an-image-from-ram).
- **Access carries over.**  nvdata is shared, so the phone keeps the network settings and the web interface's password, and the SSH keys in `/nvdata/root/.ssh/authorized_keys` are accepted once [SSH is switched on](debug-access.md#ssh).

## Keep the PBX from replacing a test image

A phone that a DPMA server provisions installs the firmware that the configuration on the PBX offers, as a phone with stock firmware does.  On a development phone, the PBX's image then replaces a test image at the next provisioning.  Either of the following stops the phone from following the PBX's firmware entries:

- On the **System** page of the web interface, in the **Firmware** card, switch on **Ignore DPMA firmware updates**.
- With [Open DPMA](../open-dpma/index.md), set [`dphone_ignore_provisioned_firmware=yes`](../open-dpma/open-dphone-options.md#dphone_ignore_provisioned_firmware) in the phone's section of `res_digium_phone.conf`.

## From Open DPhone

### With the web interface

On the **System** page, the **Firmware** card shows the running version and the active bank.  **Upload Firmware** takes an `.eff` file.  The phone writes the upload to the bank that is not running and verifies the bank, then shows the version, the model, and the size of the upload.  **Install** selects the new image, and a reboot starts the image.  **Discard** leaves the selection unchanged.

### With `dphone update install`

`dphone update install` takes the image from one of three places.  In each case the image is written to the bank that is not running, read back and compared, and selected for the next boot.

=== "Standard input"

    ```sh
    ssh root@192.0.2.20 dphone update install - < result
    ```

    The image goes from the workstation straight into flash, and no copy is stored on the phone.  `result` is the link that `nix build .#eff-d60` leaves.

=== "A URL"

    ```sh
    dphone update install http://192.0.2.5:8000/3_0_0_D60_firmware.eff
    ```

    The phone downloads the image straight into flash.  Both `http://` and `https://` work.  The command does not check the certificate of an HTTPS server; the checksums inside the image are checked.

=== "A file"

    ```sh
    scp result root@192.0.2.20:/tmp/image.eff
    ssh root@192.0.2.20 dphone update install /tmp/image.eff
    ```

    A file is read and checked in full before anything is written.  `/tmp` is held in the phone's RAM, so delete the file afterward.

An image from standard input or a URL cannot be read twice, so the checks that need the whole image run after the write.  When such an image turns out to be truncated or damaged, the bank that is not running has lost the old contents and is not selectable, and nothing else has changed: the running bank, the selection, and the trial record are as before.

After a successful installation, restart the phone to start the new image:

```sh
ssh root@192.0.2.20 dphone reboot
```

The command refuses an image for another model.  One update runs at a time, and no update starts while an installed image is still on trial; see [`dphone update abandon`](#give-up-an-image-on-trial).

| Option | Effect |
| --- | --- |
| `--bank a` or `--bank b` | Names the bank to write.  The default is the bank that is not running.  Naming the running bank is refused. |
| `--allow-unbootable-other` | Goes ahead even when the bank that stays is not bootable.  The option is for recovery work from a system [started from RAM](ramboot.md#start-an-image-from-ram). |

## The trial boot

An Open DPhone image that `dphone update install` or the web interface installs starts on trial.  The updater writes a trial record to nvdata (`/nvdata/open/trial`) before selecting the new bank, and the new image must prove healthy before the record is removed.

- **Each boot of the new bank is counted.**  The early-boot stage in the new kernel image counts the boot before the root file system is needed.  A kernel panic, a watchdog reset, or an init role that dies each cost one boot.  After three boots without a confirmation, the stage selects the previous bank again and restarts the phone.
- **A root file system that cannot start ends the trial at once.**  When the root file system does not mount or has no init program to run, the stage returns to the previous bank without using up the remaining boots.
- **A healthy main role confirms the trial.**  The init role removes the record when the main role reports the phone healthy.  `dphone health` shows the conditions behind that report.
- **A main role that keeps failing ends the trial.**  Five failures of the main role within one boot of the trial return the phone to the previous bank.  A boot in which the main role does not become healthy within 300 seconds ends with a restart, and the boot stays counted.

While a trial is pending, the **Firmware** card shows the number of boots left, with **Keep** to confirm the image by hand and **Roll Back** to return to the previous firmware.  After an automatic return, the card says that the last update was rolled back, and why.

The trial does not cover a kernel that stops before the early-boot stage runs.  The boot loader starts the same kernel again at every power-on, and the phone stays down until the flash is rewritten through a JTAG adapter.  For that reason, [start an image with a changed kernel from RAM](ramboot.md#start-an-image-from-ram) before installing the image.

A stock firmware image installed from Open DPhone gets no trial, because stock firmware does not count trial boots.

## Inspect the banks

```sh
dphone update status
```

The command reads both banks back and reports:

- the bank that the phone runs from, the bank that the `ctrl` record selects, and the bank that the boot loader will start next,
- whether the `ctrl` record and the copies of the record are valid,
- for each bank: the version, whether the boot loader would start the bank, the number of bad blocks, and whether the kernel and the root file system match the lengths and checksums in the bank's header, and
- the pending trial, when there is one.

The boot loader checks only the header and the first words of the kernel, so the comparison of the contents is the only evidence that a bank holds the image that the header describes.

## Switch to the other bank

```sh
dphone update switch-bank b
dphone reboot
```

`switch-bank` selects bank `a` or bank `b` for the next boot and writes no image.  The command is refused unless the named bank holds an image that the boot loader would start.  `switch-bank` is the quick way between two installed images, e.g., back to the stock firmware that the first installation left in the other bank.

`switch-bank` removes a trial record, so a bank selected with `switch-bank` is not on trial.

## Put a selected bank on trial

```sh
dphone update switch-bank b
dphone update arm-trial
dphone reboot
```

`arm-trial` records a trial of the bank that the `ctrl` record selects for the next boot, for an image that was selected by other means than an installation, e.g., with `switch-bank`.  The phone then returns to the other bank when the image does not come up healthy within three boots.  `switch-bank` comes first, because `switch-bank` removes a trial record.

The bank to return to is the running bank.  When the running system is from the selected bank, or from neither bank because the kernel was started from RAM, name the bank to return to with `--return-to a` or `--return-to b`.  The command refuses to put stock firmware on trial.

## Give up an image on trial

```sh
dphone update abandon
```

`abandon` selects the bank that the phone would return to and removes the trial record, so that another image can be installed.  When the phone already runs the image that is given up, restart the phone afterward.

Use `abandon` after installing an image and then finding a mistake, before or after the reboot: a corrected image can be installed at once, without waiting for the trial to end.

## Return to stock firmware

- When stock firmware is still in the other bank, [switch to that bank](#switch-to-the-other-bank).
- Otherwise install a stock firmware image for the phone's model, with the web interface or with `dphone update install`.  Both accept stock images; the web interface asks for a confirmation first.

Stock firmware has no command to select the other bank, so the way from stock firmware back to Open DPhone is another installation [from stock firmware](#from-stock-firmware), even when an Open DPhone image is still in the other bank.
