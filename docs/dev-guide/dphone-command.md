# The `dphone` command

`dphone`, the firmware's one program, is also the command-line tool on the phone.  Run the commands in a shell on the phone: over [SSH, on the serial console](debug-access.md), or on the **Shell** page of the web interface.

`dphone --help` prints the list of commands, and `dphone <command> --help` prints the arguments and options of one command.  The help comes from the binary, so the help always matches the running firmware.

## Two kinds of commands

The end of `dphone --help` names the commands that run in the calling process.  The difference matters when something is broken:

- **Commands that run in the calling process** need no main role, and work on a phone whose main role is down or in safe mode: the firmware update commands, `dphone ramboot`, and the system commands.
- **Commands that the main role answers** are sent over a local socket to the running main role, which does the work and returns the output.  When the main role is not running, such a command fails with a message that the main role cannot be reached.

Every command exits with status 0 on success.  A mistake in the command line exits with status 2 and prints the usage.

## Firmware

These run in the calling process.

| Command | Purpose |
| --- | --- |
| `dphone update status` | Shows both firmware banks, the `ctrl` record, and the bank that starts next. |
| `dphone update install <file, -, or URL>` | Installs an image into the bank that is not running, and selects that bank. |
| `dphone update switch-bank <a or b>` | Selects a bank for the next boot and writes no image. |
| `dphone update arm-trial` | Records a trial boot of the bank that is selected for the next boot. |
| `dphone update abandon` | Gives up an installed image that is not confirmed yet. |
| `dphone update write-userdata <image>` | Erases the unused `userdata` partition and writes a root file system image there, for starting a test kernel with a root file system outside both banks. |
| `dphone ramboot <image>` | Starts an image from RAM and installs nothing. |

[Installing a firmware image](installing.md) and [Trying a build without installing](ramboot.md) describe these commands.

## System commands

These run in the calling process.  The root file system has no separate programs for these tasks; `dphone` provides each, and the image links each name to `dphone`, so `dmesg` and `dphone dmesg` are the same command.

| Command | Purpose |
| --- | --- |
| `reboot`, `poweroff`, `halt` | Restart the phone, turn the phone off, or stop the system. |
| `dmesg` | Prints the kernel's log. |
| `ps` | Lists every process. |
| `mount`, `umount` | Mount a file system or list the mounts, and unmount. |
| `lsmod`, `modprobe`, `insmod`, `rmmod` | List, load, and unload kernel modules. |

## The main role

| Command | Purpose |
| --- | --- |
| `dphone main` | Runs the main role.  The init role starts the main role on a phone; a developer runs the command directly only for the [simulator](simulator.md).  `--safe` starts only SSH, the network, and the diagnostics, and `--verbose` logs at the debug level. |
| `dphone supervise` | Runs the init role's supervisor outside process 1.  The command exists for the tests. |

## Status and logs

The main role answers these.

| Command | Purpose |
| --- | --- |
| `dphone ping` | Checks that the main role answers. |
| `dphone version` | Shows the running version and the model. |
| `dphone health` | Shows the conditions behind the healthy report that confirms a trial boot.  The exit status is 0 when the main role reports healthy and 1 otherwise. |
| `dphone log` | Shows the last entries of the log.  `-n <count>` sets the number of entries, and `-f` keeps following. |
| `dphone crashlog` | Prints the records of the last kernel panic, which the main role saves in `/nvdata/open/crash/`. |
| `dphone state [topic]` | Dumps the state store, the shared state that every subsystem publishes: calls, accounts, network, audio, firmware, and more.  A topic limits the dump to the fields whose name contains the word, e.g., `dphone state audio`. |
| `dphone restart` | Restarts the main role only.  The kernel and the init role keep running. |

[Logs and diagnostics](diagnostics.md) shows how to use these commands.

## Configuration

The main role answers these.

| Command | Purpose |
| --- | --- |
| `dphone config list [text]` | Lists the settings of the merged configuration as `ID=VALUE`, limited to the IDs that contain the text.  Secrets are masked. |
| `dphone config get <id>` | Prints the value of one setting. |
| `dphone config set <id> <value>` | Sets a setting and applies the change. |
| `dphone config reload` | Merges the configuration files again and applies the result, e.g., after an edit to a file in nvdata. |

## Driving the phone

These commands stand in for a person at the phone, so that a test can be scripted over SSH.  The main role answers these.

| Command | Purpose |
| --- | --- |
| `dphone key <key>...` | Presses keys as if on the keypad, one after the other, e.g., `dphone key 1 2 3 sk1`.  `dphone key --help` lists the key names. |
| `dphone hook off` and `dphone hook on` | Lift and replace the handset. |
| `dphone call` | Lists the calls.  `dial <number>` calls a number, `answer` answers the first call, and `hangup` ends every call. |
| `dphone app` | Lists the installed apps.  `launch` starts an app or brings the app to the front, and `close` closes an app. |
| `dphone screenshot <file>` | Writes what is on a display to a picture file: PPM for a color display and PGM for a gray display.  `--screen side` chooses the second display of a D70.  A relative file name is relative to the working directory of the main role, so give an absolute path, e.g., `/tmp/screen.ppm`. |
| `dphone alarms` | Shows the alarms of the alarm clock.  `add`, `remove`, `snooze`, and `dismiss` act on alarms. |
| `dphone page` | Shows the state of multicast paging.  `start <group>` pages a group, and `stop` ends a page. |

To see the screen of a phone from the workstation:

```sh
ssh root@192.0.2.20 dphone screenshot /tmp/screen.ppm
scp root@192.0.2.20:/tmp/screen.ppm .
```

## Hardware and subsystems

The main role answers these.  In safe mode, only the network is started, so `dphone net` works and the other commands report being unavailable.

| Command | Purpose |
| --- | --- |
| `dphone net` | Shows the interfaces, the addresses, and what the network task is doing.  `bridge on` and `bridge off` switch the bridge to the PC port, `plain` applies the plainest arrangement (DHCP, no VLAN, PC port down), `configured` applies the arrangement of the settings again, and `sync-time` synchronizes the clock with the time server. |
| `dphone audio` | Shows the audio engine, the codecs, the levels, and the audio state.  `path`, `tone`, `ring`, `volume`, and `mute` drive the audio by hand. |
| `dphone dsp` | Shows the state of the voice DSP.  `selftest` runs the checks that need no hardware, `restart` loads the DSP firmware again, and `job`, `bands`, and `canceller` are experiments for work on the DSP interface. |
| `dphone hal` | Shows the model, the devices found, and the devices missing.  `lamp`, `backlight`, `contrast`, and `pattern` set the lamps and the displays, and act only before the user interface has started, e.g., in a main role started without the user interface. |
| `dphone ehs` | Shows the state of the EHS headset adapter, and whether the serial console has the port. |
| `dphone expansion` | Shows the expansion modules. |

## Adding a command

A command that the main role answers is a `clap` type that the subsystem defines and registers a handler for when the subsystem starts.  Listing the type in the `Remote` enumeration in `crates/dphone/src/cli.rs` puts the command in the tree, and with that in `dphone --help`.  The comments at the top of that file describe the design.
