# Troubleshooting

## Automatic recovery

The init role supervises the main role and recovers from most failures without intervention:

| Failure | Response |
| --- | --- |
| The main role exits or crashes. | The init role restarts the main role after a delay that doubles with each consecutive failure, up to 60 seconds. |
| The main role hangs: no heartbeat for 120 seconds after a start, or for 30 seconds after that. | The init role kills and restarts the main role. |
| The main role fails three times in a row. | The init role starts the main role in safe mode, which runs only the network, the SSH server (if the SSH setting is on), and the diagnostic commands.  The user interface, telephony, audio, the apps, and the web interface are not started. |
| The main role fails, even once. | The [serial console](ssh-and-serial-console.md#serial-console) takes over the EHS port, regardless of the setting, until a working main role applies the setting again. |
| No main role has sent a heartbeat for 600 seconds. | The init role restarts the phone. |
| The kernel or the init role hangs. | The hardware watchdog, which only the init role feeds, resets the phone. |
| nvdata cannot be mounted. | Nothing is erased.  A file system in RAM takes the place of nvdata, the phone runs with default settings, and the web interface shows a **Recovery** page. |
| A newly installed image does not come up healthy. | The [trial boot](firmware-updates.md#trial-boot) returns the phone to the previous bank. |

Safe mode is not used during a trial boot.

## Main role keeps failing

If the phone is still reachable, log in over SSH or on the serial console.  [Local commands](dphone-command.md#local-and-remote-commands), including `dmesg` and `dphone update`, work without a main role.

1. Read `dmesg` for the init role's record of each start and failure.  The records of a kernel panic are in `/nvdata/open/crash/`, or still in `/sys/fs/pstore/` if no main role has started since the panic; see [Crash records](diagnostics.md#crash-records).  A main role in safe mode also answers `dphone log`, with the entries logged since safe mode started.
2. To keep the phone from restarting while you investigate, create the file that suspends the restart after 600 seconds without a heartbeat:

    ```sh
    touch /run/dphone/no-reboot
    ```

3. Recover with `dphone update abandon` for an image on trial, `dphone update switch-bank` to boot the image in the other bank, or `dphone update install` to install a corrected image.  See [Firmware updates](firmware-updates.md) for each command.

## Phone unreachable over the network

Connect a cable to the EHS port; see [Wiring](ssh-and-serial-console.md#wiring).  The console has the port when the setting says so, and also after any failure of the main role.  From the console's shell, `dphone net` shows the state of the network, and `dphone net plain` applies the simplest network configuration: DHCP, no VLAN, and the PC port down.

## Installed image fails

No action is needed: the [trial boot](firmware-updates.md#trial-boot) returns the phone to the previous bank after three failed boots.  Power-cycling the phone also ends a boot, and the boot still counts.

## nvdata fails to mount

Restart the phone first.  A mount that failed once may succeed at the next start.

The **Recovery** page of the web interface also offers **Erase nvdata**, which deletes every setting, account, contact, key, and app, along with the phone's MAC address.  Erase nvdata only as a last resort: the MAC address is how the network and the PBX identify the phone.

## Kernel in flash fails to start

The boot loader starts the selected bank's kernel with the watchdog off, and has no fallback.  If a kernel in flash stops before the early-boot stage runs, the boot loader starts the same kernel at every power-on, and no command can reach the phone.  The trial boot cannot help, because the stage that counts the boots is part of the failing kernel image.

The only fix is to rewrite the flash through a JTAG adapter.
