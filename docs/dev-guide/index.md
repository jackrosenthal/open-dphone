# Dev Guide

## Architecture

The firmware's single program, `dphone`, is installed at `/sbin/init` on the phone and takes on a different role depending on how the program is started.

| Component | Description |
| --- | --- |
| Init role | `dphone` running as process 1.  The init role mounts the file systems, loads the kernel modules, feeds the hardware watchdog, and starts and supervises the main role. |
| Main role | `dphone main`, the phone application: the user interface, SIP, audio, the DPMA client, the app runtime, the web interface, and the SSH server. |
| Early-boot stage | `dphone-early`, a small program built into the kernel image.  The stage runs before the root file system is mounted and counts the boots of a newly installed image. |
| nvdata | The flash partition mounted at `/nvdata`, which holds everything that survives an update: the settings, the SSH keys, the installed apps, and the phone's MAC address.  Both firmware banks share nvdata, and so does stock firmware. |
| EFF image | A firmware image file (`.eff`) for one model, in the stock firmware image format. |
