# Dev Guide

The Dev Guide is for developers who change the Open DPhone firmware and have a phone on the desk.  The pages cover building the firmware from source, getting a build onto a phone, and looking inside a phone that runs the build.

A developer who writes apps for the phones needs the [App SDK](../app-sdk/index.md) instead, and an administrator who installs the Asterisk module needs [Open DPMA](../open-dpma/index.md).

## The pages

| Page | Read the page to |
| --- | --- |
| [Building from source](building.md) | Find the parts of the repository, build a firmware image for a model, and run the tests and checks. |
| [Running on a workstation](simulator.md) | Run the firmware's main program and the web interface without a phone. |
| [SSH and the serial console](debug-access.md) | Get a shell on a phone, over the network or over a cable on the EHS port. |
| [Trying a build without installing](ramboot.md) | Run a changed binary, or a whole image with a new kernel, on a phone while the flash stays untouched. |
| [Installing a firmware image](installing.md) | Write an image to a firmware bank, follow the trial boot, and return to the previous image or to stock firmware. |
| [The `dphone` command](dphone-command.md) | Look up what the command-line tool on the phone can show and do. |
| [Logs and diagnostics](diagnostics.md) | Read the logs, the crash records, and the state of a running phone. |
| [When a phone does not start](recovery.md) | Learn what the firmware does when a part fails, and what can and cannot be recovered without a JTAG adapter. |

## How the firmware is put together

The other pages use the following names.

- **`dphone`** is the one program of the firmware, at `/sbin/init` on the phone.  The program takes one of several roles, depending on how the program is started.
- **The init role** is `dphone` running as process 1.  The init role mounts the file systems, loads the kernel modules, feeds the hardware watchdog, and starts and supervises the main role.
- **The main role** is `dphone main`: the phone application, with the user interface, SIP, audio, the DPMA client, the app runtime, the web interface, and the SSH server.
- **The early-boot stage** (`dphone-early`) is a small program built into the kernel image.  The stage runs before the root file system is mounted and counts the boots of a newly installed image.
- **The firmware banks** are two sets of flash partitions, bank A (`k0` and `root0`) and bank B (`k1` and `root1`).  Each bank holds a kernel and a root file system.  The `ctrl` partition records which bank the boot loader starts.
- **nvdata** is the flash partition mounted at `/nvdata`, which holds everything that survives an update: the settings, the SSH keys, the installed apps, and the phone's identity.  Both banks, and stock firmware, use the same nvdata.
- **An EFF image** is a firmware image file (`.eff`) for one model, in the format of stock firmware images.

## A working loop

Each step below costs more time than the step before and proves more, so a change usually moves down the list.

1. Run the tests and the simulator on the workstation ([Building from source](building.md), [Running on a workstation](simulator.md)).
2. Push the changed `dphone` binary to a phone with `cargo xtask deploy` ([Trying a build without installing](ramboot.md#push-a-changed-binary)).  A reboot brings the installed binary back.
3. Start the whole image from RAM with `dphone ramboot` ([Trying a build without installing](ramboot.md#start-an-image-from-ram)).  This step matters most for a kernel change, because a kernel that hangs in flash cannot be undone from the phone.
4. Install the image with `dphone update install` ([Installing a firmware image](installing.md)).
