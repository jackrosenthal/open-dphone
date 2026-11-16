# SSH and serial console

Open DPhone provides a root shell over SSH and on a serial console on the EHS port.  Both are off by default.

- **SSH** is the everyday tool for pushing builds, installing images, and reading logs.
- **The serial console** shows kernel messages and keeps working when the network or the main role is down.  Use the console for kernel work and for phones that fail to come up.

The shell is [brush](https://github.com/reubeno/brush), a Bash-compatible shell.  The uutils versions of the common utilities, `sed`, and the [`dphone` command](dphone-command.md) are available.

## SSH

The SSH server accepts public keys only, and the login name is `root`.  Before you can log in, the SSH setting must be on, and the phone must have at least one public key.  You can configure both from the phone's web interface, from the PBX, or from a shell.

### Web interface

On the **System** page, the **SSH** card has the **Enabled** switch and the list of keys.  Paste a public key (the contents of a `.pub` file, e.g., `~/.ssh/id_ed25519.pub`) into the box, or choose the file, and click **Add Key**.  The switch takes effect immediately.

Keys added in the web interface are stored in `/nvdata/root/.ssh/authorized_keys`, the same file that the stock firmware's SSH server reads.

A phone provisioned by a DPMA server serves the web interface only when the phone's section of `res_digium_phone.conf` has `web_ui_enabled=yes`.

### PBX configuration

With [Open DPMA](../open-dpma/index.md) on the PBX, the phone's section of `res_digium_phone.conf` can turn SSH on and supply keys.  This approach works well for development phones that are reset often.

```ini
[phone101]
type=phone
dphone_ssh_enabled=yes
dphone_ssh_authorized_key=ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa alice@laptop
```

Repeat `dphone_ssh_authorized_key` to add more keys.  The original `res_digium_phone` does not support these options.  See [Open DPhone options](../open-dpma/open-dphone-options.md#dphone_ssh_enabled) for details.

Provisioned keys are accepted in addition to the keys added on the phone, and are removed from the phone when the PBX stops supplying them.

The PBX's `dphone_ssh_enabled` overrides the SSH setting on the phone.  The switch in the web interface is then disabled and labeled as configured via DPMA.

### Settings file

From a shell that is already open, e.g., on the serial console, turn SSH on by editing `/nvdata/dphone_settings.txtpb`, the file where the web interface stores the setting.  The file has one field per line.  Set the field, then tell the main role to reload the configuration.

If the file has no `ssh_enabled` line yet:

```sh
echo 'ssh_enabled: true' >> /nvdata/dphone_settings.txtpb
dphone config reload
```

If the file already has the line, change the value on that line instead of adding a second line.  The field for the serial console is `serial_console: true`.

### Login

```sh
ssh root@192.0.2.20
```

Replace `192.0.2.20` with the phone's address.  The server also runs single commands and serves SFTP, so `scp` and `sftp` work:

```sh
ssh root@192.0.2.20 dphone version
scp root@192.0.2.20:/tmp/screen.ppm .
```

The phone generates an Ed25519 host key on first use and stores the key in nvdata, so the host key stays the same across firmware updates.  The server accepts at most six connections at a time.

Turning SSH off closes the listening socket but leaves open sessions running, so turning SSH off from an SSH session does not cut the session off.

## Serial console

The phone's serial port is the EHS port, the connector for an electronic hook switch (EHS) headset adapter.  A headset adapter and a console cannot share the port, so a setting selects one or the other.  With the default setting, Headset, the phone writes nothing to the port: no kernel messages, no shell, and no log output.

### Console mode

On the **System** page of the web interface, set **EHS Port** to **Serial Console (115200 8N1)**.  The change takes effect immediately.  From the PBX, set [`dphone_serial_console`](../open-dpma/open-dphone-options.md#dphone_serial_console) in the phone's section:

```ini
dphone_serial_console=yes
```

In console mode:

- the kernel writes console messages to the port,
- the main role's log is copied to the port, and
- a root shell runs on the port, with no login prompt.

!!! warning "Console mode gives a root shell to anyone with physical access"

    Switch the setting back to Headset before a phone goes back into service.

The console also takes over the port when the main role fails, whatever the setting says.  The init role gives the port to the console until a working main role applies the setting again.  A phone whose main role cannot start is therefore reachable with nothing but a cable.

### Wiring

Every model has the same EHS header: two rows of three positions on a 1 mm pitch.  One position has no pin and serves as the key.  The diagram shows the header as seen when attaching the cable, with the key at the top right.

![The EHS header: top row pin 1 ground, pin 2 phone TX, key; bottom row pin 4 3.3 V, pin 5 phone RX, pin 6 ground](../assets/dev-guide/ehs-header.svg){ width="420" }

| Pin | Function | USB serial cable |
| --- | --- | --- |
| 1 | Ground | Ground wire (black) |
| 2 | Phone TX (output from the phone) | Receive wire (white) |
| 3 | Key, no pin | |
| 4 | 3.3 V supply from the phone | Leave unconnected |
| 5 | Phone RX (input to the phone) | Transmit wire (green) |
| 6 | Ground | Alternative to pin 1 for the ground wire |

Use a USB serial cable with 3.3 V logic levels.  The wire colors in the table are typical for USB serial cables with four loose wires, but colors vary between manufacturers, so check the cable's documentation.

!!! danger "Do not connect the cable's power wire"

    Pin 4 carries the phone's 3.3 V supply.  Leave the cable's power wire (red, 5 V from the USB port) unconnected.  Three wires are enough: ground, the phone's TX, and the phone's RX.

### Terminal settings

The port runs at 115200 bits per second with 8 data bits, no parity, and 1 stop bit (8N1), and no flow control.  Any terminal program works.  For example, with the cable at `/dev/ttyUSB0`:

```sh
picocom -b 115200 /dev/ttyUSB0
```

Press ++enter++ to get a prompt.

### Early kernel messages

The firmware applies the console setting, so with the setting alone, kernel messages appear on the port only after the main role has started.  The kernel parameter `amba_pl011.console_output=1` makes the kernel write to the port from the start of the boot.

- **For an image booted from RAM**, pass the parameter with [`--append`](test-builds.md#options):

    ```sh
    ssh root@192.0.2.20 dphone ramboot --append amba_pl011.console_output=1 - < result
    ```

- **For an image that will be installed**, add the parameter to the `bootargs` property in `kernel/files/arch/arm/boot/dts/broadcom/bcmring-digium-common.dtsi` and build the image.  The boot loader's part of the kernel command line cannot be changed.
- **On a running phone**, write `1` to `/sys/module/amba_pl011/parameters/console_output` to turn the kernel's console output on.
