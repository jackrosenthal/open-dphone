# `dphone` command reference

The firmware's single program, `dphone`, is also the command-line tool on the phone.  Run `dphone` commands in a shell on the phone: over [SSH or the serial console](ssh-and-serial-console.md), or on the **Shell** page of the web interface.

`dphone --help` lists the commands, and `dphone <command> --help` shows the arguments and options for one command.

## Local and remote commands

Each command is either local or remote, and the difference matters when something is broken:

- **Local commands** run in the calling process and do not need the main role.  Local commands work on a phone whose main role is down or in safe mode.  The firmware update commands, `dphone ramboot`, and the system commands are local.
- **Remote commands** are sent over a local socket to the running main role, which does the work and returns the output.  When the main role is not running, a remote command fails with a message that the main role cannot be reached.

## Firmware

These commands are local.  See [Firmware updates](firmware-updates.md) and [Test builds](test-builds.md) for usage.

| Command | Description |
| --- | --- |
| `dphone update status` | Shows both firmware banks, the `ctrl` record, and the bank that starts next. |
| `dphone update install <file, -, or URL>` | Installs an image into the inactive bank and selects that bank. |
| `dphone update switch-bank <a or b>` | Selects a bank for the next boot without writing an image. |
| `dphone update arm-trial` | Puts the bank selected for the next boot on trial. |
| `dphone update abandon` | Cancels the trial of an installed image that is not yet confirmed. |
| `dphone update write-userdata <image>` | Erases the unused `userdata` partition and writes a root file system image there.  Use the command to boot a test kernel with a root file system outside both banks. |
| `dphone ramboot <image>` | Boots an image from RAM without installing anything. |

## System

These commands are local.  The root file system has no separate programs for these tasks.  `dphone` implements each one, and the image links each name to `dphone`, so `dmesg` and `dphone dmesg` are the same command.

| Command | Description |
| --- | --- |
| `reboot`, `poweroff`, `halt` | Restart the phone, power the phone off, or halt the system. |
| `dmesg` | Prints the kernel log. |
| `ps` | Lists all processes. |
| `mount`, `umount` | Mount a file system or list the mounts, and unmount a file system. |
| `lsmod`, `modprobe`, `insmod`, `rmmod` | List, load, and unload kernel modules. |

## Main role

| Command | Description |
| --- | --- |
| `dphone main` | Runs the main role.  On a phone, the init role starts the main role; run the command directly only for the [simulator](simulator.md).  `--safe` starts only SSH, the network, and the diagnostics, and `--verbose` logs at the debug level. |
| `dphone supervise` | Runs the init role's supervisor outside process 1.  Used by the tests. |

## Status and logs

These commands are remote.  See [Logs and diagnostics](diagnostics.md) for usage.

| Command | Description |
| --- | --- |
| `dphone ping` | Checks that the main role responds. |
| `dphone version` | Shows the running version and the model. |
| `dphone health` | Shows the conditions behind the health report that confirms a trial boot.  The exit status is 0 when the main role reports healthy, and 1 otherwise. |
| `dphone log` | Shows the most recent log entries.  `-n <count>` sets the number of entries, and `-f` follows the log. |
| `dphone crashlog` | Prints the records of the last kernel panic, which the main role saves in `/nvdata/open/crash/`. |
| `dphone state [topic]` | Dumps the state store, the shared state that every subsystem publishes: calls, accounts, network, audio, firmware, and more.  A topic limits the dump to the fields whose names contain the word, e.g., `dphone state audio`. |
| `dphone restart` | Restarts the main role only.  The kernel and the init role keep running. |

## Configuration

These commands are remote.

| Command | Description |
| --- | --- |
| `dphone config list [text]` | Lists the settings in the merged configuration as `ID=VALUE`, limited to the IDs that contain the text.  Secrets are masked. |
| `dphone config get <id>` | Prints the value of one setting. |
| `dphone config set <id> <value>` | Changes a setting and applies the change. |
| `dphone config reload` | Merges the configuration files again and applies the result, e.g., after you edit a file in nvdata. |

## Phone control

These commands are remote.

| Command | Description |
| --- | --- |
| `dphone key <key>...` | Presses keys in sequence, as if on the keypad, e.g., `dphone key 1 2 3 sk1`.  `dphone key --help` lists the key names. |
| `dphone hook off`, `dphone hook on` | Lift and replace the handset. |
| `dphone call` | Lists the calls.  `dial <number>` calls a number, `answer` answers the first call, and `hangup` ends every call. |
| `dphone app` | Lists the installed apps.  `launch` starts an app or brings a running app to the front, and `close` closes an app. |
| `dphone screenshot <file>` | Saves the contents of a display to an image file: PPM for a color display, and PGM for a grayscale display.  `--screen side` selects the second display on a D70.  A relative file name is resolved against the main role's working directory, so use an absolute path, e.g., `/tmp/screen.ppm`. |
| `dphone alarms` | Shows the alarm clock's alarms.  `add`, `remove`, `snooze`, and `dismiss` act on alarms. |
| `dphone page` | Shows the state of multicast paging.  `start <group>` pages a group, and `stop` ends a page. |

To view a phone's screen from the workstation:

```sh
ssh root@192.0.2.20 dphone screenshot /tmp/screen.ppm
scp root@192.0.2.20:/tmp/screen.ppm .
```

## Hardware and subsystems

These commands are remote.  In safe mode, only the network is started, so `dphone net` works and the other commands report that the subsystem is unavailable.

| Command | Description |
| --- | --- |
| `dphone net` | Shows the interfaces, the addresses, and the network task's current activity.  `bridge on` and `bridge off` toggle the bridge to the PC port.  `plain` applies the simplest configuration (DHCP, no VLAN, PC port down), and `configured` reapplies the configuration from the settings.  `sync-time` synchronizes the clock with the time server. |
| `dphone audio` | Shows the audio engine, the codecs, the levels, and the audio state.  `path`, `tone`, `ring`, `volume`, and `mute` control the audio manually. |
| `dphone dsp` | Shows the state of the voice DSP.  `selftest` runs the checks that need no hardware, and `restart` reloads the DSP firmware.  `job`, `bands`, and `canceller` are experiments for work on the DSP interface. |
| `dphone hal` | Shows the model, the devices found, and the devices missing.  `lamp`, `backlight`, `contrast`, and `pattern` set the lamps and the displays.  These subcommands work only before the user interface has started, e.g., in a main role started without the user interface. |
| `dphone ehs` | Shows the state of the EHS headset adapter, and whether the serial console has the port. |
| `dphone expansion` | Shows the expansion modules. |

## New commands

A remote command is a `clap` type that a subsystem defines.  The subsystem registers a handler for the type at startup.  To add the command to the command tree, and therefore to `dphone --help`, list the type in the `Remote` enumeration in `crates/dphone/src/cli.rs`.  The comments at the top of that file describe the design.
