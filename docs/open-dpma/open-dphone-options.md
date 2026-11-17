# Open DPhone options

Open DPMA adds options to phone sections (`type=phone`) of `res_digium_phone.conf` for features that only Open DPhone has.  The option names all begin with `dphone_`.

Phones running stock firmware ignore these options, so one phone section works for a phone both before and after a firmware change.  The original module does not recognize the options and reports each one as invalid.

For example, a template for phones running Open DPhone:

```ini title="res_digium_phone.conf"
[open-dphone](!)
type=phone
dphone_ssh_enabled=yes
dphone_ssh_authorized_key=ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa alice@laptop
dphone_disabled_apps=boxpusher,desksparrow,lightup
dphone_app_data_tenant=north-office

[phone101](open-dphone)
mac=000fd3000001
line=101
```

To see the values that the module holds for a phone section, run `digium_phones show phone <name>`.

## Precedence

**Server values override phone settings.**  The module sends the on/off options, the softkeys, the disabled apps, the theme, the palette file, and the static address as frozen settings.  The phone uses the server's value regardless of what was set on the phone, and the phone's web interface shows the setting as a disabled control labeled "Configured via DPMA".  Removing the option from the configuration file restores the value that was set on the phone.

**Omitted options send nothing.**  The phone uses the value set on the phone, or else the firmware default listed for each option below.

**Invalid values are skipped.**  The module logs `Invalid option '<name>' at line <n> is not supported by phone module.` at the `DPMA` level and sends the phone nothing for that option.

## Remote access

### `dphone_ssh_enabled` { #dphone_ssh_enabled }

Enables or disables the phone's SSH server.

- **Value:** on/off.
- **Default:** the SSH server is off.

The SSH server provides a root shell and accepts public keys only.  Nobody can log in until the phone has at least one key, either from [`dphone_ssh_authorized_key`](#dphone_ssh_authorized_key) or added on the phone.

### `dphone_ssh_authorized_key` { #dphone_ssh_authorized_key }

Authorizes an SSH public key to log in to the phone.

- **Value:** one public key in `authorized_keys` format: `<type> <base64 key> [comment]`, e.g., `ssh-ed25519 AAAA... alice@laptop`.  The type must begin with `ssh-`, `ecdsa-sha2-`, or `sk-`.  Lines that begin with `authorized_keys` options, e.g., `command="..."`, are rejected, as are lines longer than 8192 bytes.
- **Repeatable:** each occurrence adds a key.  Duplicate keys are sent once.
- **Default:** the server sends no keys.

The phone accepts the server's keys in addition to keys added on the phone.  When you remove a key from the phone section, the phone drops the key at the next configuration fetch.

### `dphone_serial_console` { #dphone_serial_console }

Turns the phone's serial port, on the EHS connector, into a console.

- **Value:** on/off.
- **Default:** no serial console.

The console and an electronic hook switch (EHS) headset adapter share one serial port.  With the console on, the phone cannot use an EHS adapter, and the `ehs` option has no effect.

### `dphone_metrics_enabled` { #dphone_metrics_enabled }

Enables or disables the phone's `GET /metrics` endpoint, which a Prometheus server can scrape.

- **Value:** on/off.
- **Default:** the endpoint is served.

The endpoint requires no login and is independent of `web_ui_enabled`.  Set `dphone_metrics_enabled=no` if the metrics should not be readable from the phones' network.

## Network address

The module sends `dphone_static_gateway` and `dphone_static_dns` only when `dphone_static_address` is also set.

!!! warning "Check the address before reconfiguring the phone"

    The server's address replaces any static address that was set on the phone.  A phone with a wrong address, prefix length, or gateway cannot reach the server to fetch a corrected configuration.

### `dphone_static_address` { #dphone_static_address }

The phone's IPv4 address and prefix length.

- **Value:** `<address>/<prefix length>`, e.g., `192.0.2.20/24`.  The prefix length is 1 to 30.  The address must be a host address: not the network or broadcast address of the prefix, and not a loopback, multicast, or reserved address.
- **Default:** the phone uses DHCP.

### `dphone_static_gateway` { #dphone_static_gateway }

The default gateway for the static address.

- **Value:** an IPv4 address, e.g., `192.0.2.1`.
- **Default:** the phone has no default gateway.

### `dphone_static_dns` { #dphone_static_dns }

The DNS servers for the static address.

- **Value:** one or more IPv4 addresses, separated by commas, e.g., `192.0.2.53,198.51.100.53`.  If any address on a line is invalid, the module skips the whole line.
- **Repeatable:** each occurrence adds servers.  Duplicate addresses are sent once.
- **Default:** the phone has no DNS server.  Without a DNS server, `registration_address` and the URL prefixes must use addresses, not host names.

## Display and apps

### `dphone_idle_softkeys` { #dphone_idle_softkeys }

The idle screen softkeys, in order from the left.

- **Value:** softkeys, separated by commas.  Each is one of the functions below, or `app:<name>` for a softkey that opens the named app.  If any softkey on a line is unknown, the module skips the whole line.
- **Repeatable:** each occurrence appends softkeys.  A softkey listed twice appears once.
- **Default:** `call_log,contacts,status,menu,apps,dnd,forward`, followed by `theme` on the color models (D60, D62, and D65).

| Function | Softkey |
| --- | --- |
| `call_log` | The call log.  The softkey shows missed calls while there are unseen missed calls. |
| `contacts` | The Contacts app. |
| `status` | Information about the phone. |
| `menu` | The phone's menu. |
| `apps` | The list of apps. |
| `dnd` | Toggles Do Not Disturb. |
| `forward` | The call forwarding menu. |
| `redial` | Redial. |
| `theme` | The theme selector, on the color models only. |

For example:

```ini
dphone_idle_softkeys=call_log,contacts,app:parking,menu
```

The options that shape the idle screen on stock firmware (`show_call_log`, `can_forward_calls`, `show_call_parking`, and a custom application's `idlescreen_softkey`) have no effect on Open DPhone's idle softkeys.

### `dphone_disabled_apps` { #dphone_disabled_apps }

Hides built-in apps.  A disabled app does not appear in the phone's list of apps and cannot be opened.

- **Value:** app names, separated by commas, e.g., `calendar,parking`.  Names may contain letters, digits, `_`, `-`, and `.`.  If any name on a line is invalid, the module skips the whole line.
- **Repeatable:** each occurrence adds names.
- **Default:** all built-in apps are available.

The built-in apps are `alarms`, `boxpusher`, `calendar`, `contacts`, `desksparrow`, `diagnostics`, `hotdesking`, `lightup`, `parking`, `queues`, `status`, and `voicemail`.  Consider disabling apps for services that the server does not provide, e.g., `queues` on a server without `app_queue`.

### `dphone_theme` { #dphone_theme }

The phone's built-in theme.  A theme is a wallpaper paired with the color palette for every screen.

- **Value:** a theme name, e.g., `dusk`: 1 to 32 letters, digits, `_`, and `-`.  The phone's theme selector and web interface list the themes.  If the firmware has no theme by that name, the phone shows the default theme.
- **Default:** the theme that the user chose on the phone, or else the firmware's default theme, Tides.
- **Models:** the color models (D60, D62, and D65).

If the phone section also sets `wallpaper_file`, the phone shows that picture instead of the theme.

### `dphone_palette_file` { #dphone_palette_file }

The palette file for the `wallpaper_file` picture.  The palette sets the colors of every screen while that picture is the wallpaper.

- **Value:** a file name or a URL, e.g., `lobby.txtpb`.  The file is located the same way as `wallpaper_file`: a file in `file_directory` is fetched from the module, any other name is fetched from the network's `file_url_prefix`, and a full URL is used as written.
- **Default:** the phone draws the picture with the phone's current palette.
- **Models:** the color models (D60, D62, and D65).

The phone fetches the palette file only when the section also sets `wallpaper_file`.  The file is a `dphone.palette.v1` message in protobuf text format, at most 16 KiB in size.  The format is defined in `proto/dphone/palette/v1/palette.proto` in the repository.

## Firmware

### `dphone_ignore_provisioned_firmware` { #dphone_ignore_provisioned_firmware }

Makes the phone ignore the firmware entries in the configuration.

- **Value:** on/off.
- **Default:** the phone follows the firmware entries, and installs the image from the matching entry when that image differs from the running firmware.

Use the option for a phone that must stay on a particular build, e.g., a development phone on a server that offers a release to every other phone.  See [Firmware](administration.md#firmware).

## App data

### `dphone_app_data_tenant` { #dphone_app_data_tenant }

The tenant whose app data the phone reads and writes.

- **Value:** a key of 1 to 32 characters: lowercase letters, digits, `.`, and `-`.  An uppercase letter or a `_` makes the value invalid.
- **Default:** the phone uses the store shared by all phones without a key.

Phones with the same key share one app data store, and no phone can read or write the store of another key.  The module never sends the key to a phone, and a phone's request cannot name a key.

An invalid value is not treated as "no key."  The module logs the option as invalid and refuses all of the phone's app data requests until the value is fixed, so a typo never puts a tenant's phone into the shared store.

See [App data](administration.md#app-data) for details about the store and recommendations for multi-tenant servers.
