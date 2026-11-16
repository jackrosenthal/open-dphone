# When a phone does not start

## What the firmware does unattended

The init role supervises the main role and guards the system:

| Failure | Response |
| --- | --- |
| The main role exits or crashes. | The init role starts the main role again, after a pause that doubles with each failure in a row, up to 60 seconds. |
| The main role hangs: no heartbeat for 120 seconds after a start, or for 30 seconds later. | The init role kills the main role and starts the main role again. |
| The main role fails three times in a row. | The init role starts the main role in safe mode: only the network, the SSH server (when the SSH setting is on), and the diagnostic commands.  The user interface, telephony, audio, the apps, and the web interface are not started. |
| The main role fails at all. | The [serial console](debug-access.md#the-serial-console-on-the-ehs-port) gets the EHS port, whatever the setting says, until a main role that works decides again. |
| No main role has sent a heartbeat for 600 seconds. | The init role restarts the phone. |
| The kernel or the init role hangs. | The hardware watchdog, which only the init role feeds, resets the phone. |
| nvdata cannot be mounted. | Nothing is erased.  A file system in RAM stands in, the phone runs with default settings, and the web interface shows a **Recovery** page. |
| A newly installed image does not come up healthy. | The [trial boot](installing.md#the-trial-boot) returns the phone to the previous bank. |

During a trial boot, safe mode is not used: the image on trial must show that the full main role works, and five failures return the phone to the previous bank.

## What to do

### The main role keeps failing, and the phone is reachable

Log in over SSH or on the serial console.  The [commands that run in the calling process](dphone-command.md#two-kinds-of-commands), among them `dmesg` and `dphone update`, work without a main role.

1. Read `dmesg` for the init role's account of each start and failure.  The records of a kernel panic are the files in `/nvdata/open/crash/`, or still in `/sys/fs/pstore/` when no main role has started since the panic (see [Logs and diagnostics](diagnostics.md#crash-records)).  A main role in safe mode also answers `dphone log`, with the entries since the start in safe mode.
2. To keep the phone from restarting during the investigation, create the file that suspends the restart after 600 seconds without a heartbeat:

    ```sh
    touch /run/dphone/no-reboot
    ```

3. Choose the way out: `dphone update abandon` for an image on trial, `dphone update switch-bank` for the image in the other bank, or `dphone update install` for a corrected image.  [Installing a firmware image](installing.md) describes each.

### A pushed binary does not work

A binary pushed with `cargo xtask deploy` that fails quickly three times in a row is ignored, and the installed binary runs.  A reboot removes the pushed binary in every case, because `/tmp` is held in RAM.

### The phone is not reachable over the network

Connect a cable to the EHS port ([the header and the cable](debug-access.md#the-header-and-the-cable)).  The console has the port when the setting says so, and also after any failure of the main role.  From the console's shell, `dphone net` shows the state of the network, and `dphone net plain` applies the plainest arrangement: DHCP, no VLAN, and the PC port down.

### An image started from RAM does not work

Restart the phone, or remove and restore the power.  Nothing was written to the firmware banks, so the installed firmware starts.  Without any action, the time limit of `dphone ramboot` returns the phone after 900 seconds, and the watchdog resets a kernel that hangs.

### An installed image does not work

Wait.  The trial boot allows the new image three boots, and each failed boot ends with a restart: by the watchdog, by the kernel after a panic, or by the init role when the main role is not healthy within 300 seconds.  After the third boot, the early-boot stage selects the previous bank.  Removing and restoring the power also ends a boot, and the boot stays counted.

Once the previous firmware is running again, the **Firmware** card of the web interface states why the update was rolled back.

### nvdata cannot be mounted

Restart the phone first; a mount that failed once can succeed at the next start.  The **Recovery** page of the web interface also offers **Erase nvdata**, which deletes every setting, account, contact, key, and app, and the phone's MAC address.  Treat the erase as the last resort: the MAC address is the identity that the network and the PBX know the phone by.

### The kernel in flash does not start

The boot loader starts the kernel of the selected bank with the watchdog off, and has no fallback.  A kernel in flash that stops before the early-boot stage runs is started again at every power-on, and no command can reach the phone.  The trial boot cannot help, because the stage that counts the boots is part of the kernel image that fails.

The flash must then be rewritten through a JTAG adapter, a procedure that the Dev Guide does not cover.  The protection is to [start every image with a changed kernel from RAM](ramboot.md#start-an-image-from-ram) and confirm that the image comes up, before installing the image.
