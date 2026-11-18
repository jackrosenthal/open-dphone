---
title: "Dev Guide"
---

## Architecture

The firmware's single program, `dphone`, is installed as `/sbin/init`.  How the program is started selects the role.

| Component | Description |
| --- | --- |
| Init role | `dphone` as process 1.  Mounts the file systems, loads the kernel modules, feeds the hardware watchdog, and starts and supervises the main role. |
| Main role | `dphone main`: the user interface, SIP, audio, the DPMA client, the app runtime, the web interface, and the SSH server. |
| Early-boot stage | `opendeskphone-early`, built into the kernel image.  Runs before the root file system is mounted, and counts the boots of a newly installed image. |
| nvdata | The flash partition at `/nvdata` that survives updates: the settings, the SSH keys, the installed apps, and the MAC address.  Shared by both firmware banks and by stock firmware. |
| EFF image | A firmware image file (`.eff`) for one model, in the stock firmware's image format. |
