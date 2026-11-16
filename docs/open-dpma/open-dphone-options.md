# Options for Open DPhone

Open DPMA adds options to the phone sections (`type=phone`) of `res_digium_phone.conf` for the features that only Open DPhone has.  The names of the options begin with `dphone_`.

## What the options have in common

- **Which firmware honors the options.**  A phone with Open DPhone acts on the options.  A phone with stock firmware ignores the settings, so the same phone section serves a phone before and after a change of firmware.  The original module does not know the options and reports each as invalid.
- **An absent option sends nothing.**  The phone then uses the value set on the phone, or else the firmware's default, which each entry below states.
- **The server's value holds on the phone.**  The module sends the on/off options, the softkeys, the disabled apps, the theme, the palette file, and the static address as frozen settings (`conflict="freeze"`).  The phone uses the server's value, whatever was set on the phone before, and the phone's web interface shows the setting as a disabled control with the note "Configured via DPMA".  A user who can reach the phone's settings therefore cannot change the value, e.g., cannot switch SSH on against `dphone_ssh_enabled=no`.  When the option is removed from the configuration file, the value that was set on the phone applies again.
- **A value that the module cannot read is skipped.**  The module logs `Invalid option '<name>' at line <n> is not supported by phone module.` at the level `DPMA`, and sends the phone nothing for the option.
- **`digium_phones show phone <name>`** prints the values that the module holds for a phone section.

A template sets options for many phones at once:

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

## Summary

| Option | Value | Without the option |
| --- | --- | --- |
| [`dphone_ssh_enabled`](#dphone_ssh_enabled) | On/off | SSH server off |
| [`dphone_ssh_authorized_key`](#dphone_ssh_authorized_key) | An SSH public key; repeatable | No keys from the server |
| [`dphone_serial_console`](#dphone_serial_console) | On/off | Serial console off |
| [`dphone_metrics_enabled`](#dphone_metrics_enabled) | On/off | `/metrics` served |
| [`dphone_static_address`](#dphone_static_address) | `<address>/<prefix length>` | DHCP |
| [`dphone_static_gateway`](#dphone_static_gateway) | IPv4 address | No gateway |
| [`dphone_static_dns`](#dphone_static_dns) | IPv4 addresses, comma-separated; repeatable | No DNS servers |
| [`dphone_idle_softkeys`](#dphone_idle_softkeys) | Softkeys, comma-separated; repeatable | `call_log,contacts,status,menu,apps,dnd,forward` |
| [`dphone_disabled_apps`](#dphone_disabled_apps) | App names, comma-separated; repeatable | Every built-in app offered |
| [`dphone_theme`](#dphone_theme) | Name of a built-in theme | The theme chosen on the phone |
| [`dphone_palette_file`](#dphone_palette_file) | File name or URL | The phone's palette |
| [`dphone_ignore_provisioned_firmware`](#dphone_ignore_provisioned_firmware) | On/off | Firmware entries followed |
| [`dphone_app_data_tenant`](#dphone_app_data_tenant) | A tenant key | The store of the phones without a key |

## Remote access

### `dphone_ssh_enabled` { #dphone_ssh_enabled }

Whether the phone's SSH server runs.

- **Value:** on/off.
- **Without the option:** the SSH server does not run.

The SSH server gives a root shell on the phone and accepts public keys only.  Nobody can log in until the phone has at least one key: a [`dphone_ssh_authorized_key`](#dphone_ssh_authorized_key), or a key added on the phone.

### `dphone_ssh_authorized_key` { #dphone_ssh_authorized_key }

An SSH public key that may log in to the phone.

- **Value:** one public key in the form of a line of `authorized_keys`: `<type> <base64 key> [comment]`, e.g., `ssh-ed25519 AAAA... alice@laptop`.  The type begins with `ssh-`, `ecdsa-sha2-`, or `sk-`.  A line that begins with `authorized_keys` options, e.g., `command="..."`, is refused, as is a line of more than 8192 bytes.
- **Repeatable:** each occurrence adds a key; a key written twice is sent once.
- **Without the option:** the server sends no keys.

The phone accepts the server's keys in addition to the keys that were added on the phone.  After a key is removed from the phone section, the phone drops the key when the phone next fetches the configuration.  The keys have no effect while the SSH server does not run.

### `dphone_serial_console` { #dphone_serial_console }

Whether the phone's serial port, on the EHS connector, is a console.

- **Value:** on/off.
- **Without the option:** no serial console.

The console and an electronic hook switch headset adapter use the same serial port.  A phone with the console on cannot use an EHS adapter, and the `ehs` option has no effect.  Switch the console on for a phone on the bench, not for a phone whose user has an EHS headset.

### `dphone_metrics_enabled` { #dphone_metrics_enabled }

Whether the phone's web server answers `GET /metrics`, the endpoint that a Prometheus server scrapes.

- **Value:** on/off.
- **Without the option:** the endpoint is served.

The endpoint needs no login, and does not depend on `web_ui_enabled`.  Set `dphone_metrics_enabled=no` where the phones' network is not trusted to read the metrics.

## Network address

The three options give a phone a static IPv4 address in place of DHCP.  The module sends the gateway and the DNS servers only together with `dphone_static_address`.  The server's address replaces a static address that was set on the phone, so check the address, the prefix length, and the gateway before reconfiguring the phone: a phone with a wrong address does not reach the server for a corrected configuration.

### `dphone_static_address` { #dphone_static_address }

The phone's IPv4 address and prefix length.

- **Value:** `<address>/<prefix length>`, e.g., `192.0.2.20/24`.  The prefix length is 1 to 30.  The address has to be a host's address: not the network's address or the broadcast address of the prefix, and not a loopback, multicast, or reserved address.
- **Without the option:** the phone uses DHCP.

### `dphone_static_gateway` { #dphone_static_gateway }

The default gateway that goes with the static address.

- **Value:** an IPv4 address, e.g., `192.0.2.1`.
- **Without the option:** the phone has no default gateway.

### `dphone_static_dns` { #dphone_static_dns }

The DNS servers that go with the static address.

- **Value:** one or more IPv4 addresses, separated by commas, e.g., `192.0.2.53,198.51.100.53`.  When one address of a line is invalid, the module skips the whole line.
- **Repeatable:** each occurrence adds servers; an address written twice is sent once.
- **Without the option:** the phone has no DNS server.  A phone without a DNS server needs addresses, not host names, in `registration_address` and in the URL prefixes.

## The phone's screens

### `dphone_idle_softkeys` { #dphone_idle_softkeys }

The softkeys of the idle screen, in order from the left.

- **Value:** softkeys, separated by commas.  Each is one of the functions below, or `app:<name>` for a softkey that opens the app of that name.
- **Repeatable:** each occurrence adds softkeys at the end; a softkey written twice appears once.  When one softkey of a line is unknown, the module skips the whole line.
- **Without the option:** `call_log,contacts,status,menu,apps,dnd,forward`, and `theme` after them on the color models D60, D62, and D65.

| Function | Softkey |
| --- | --- |
| `call_log` | The call log; shown as missed calls while there are unseen missed calls. |
| `contacts` | The Contacts app. |
| `status` | Information about the phone. |
| `menu` | The phone's menu. |
| `apps` | The list of apps. |
| `dnd` | Switches Do Not Disturb on and off. |
| `forward` | The call forwarding menu. |
| `redial` | Redial. |
| `theme` | The theme selector.  Only the color models D60, D62, and D65 have themes; the other models leave the softkey out. |

For example, `dphone_idle_softkeys=call_log,contacts,app:parking,menu`.

The options that shape the idle screen of stock firmware (`show_call_log`, `can_forward_calls`, `show_call_parking`, and `idlescreen_softkey` of a custom application) do not change the idle softkeys of Open DPhone.  Only `dphone_idle_softkeys` sets the idle softkeys of an Open DPhone phone from the server.

### `dphone_disabled_apps` { #dphone_disabled_apps }

The built-in apps that the phone does not offer.  A disabled app is not in the phone's list of apps and cannot be opened.

- **Value:** app names, separated by commas, e.g., `calendar,parking`.  A name has letters, digits, `_`, `-`, and `.`.  When one name of a line is invalid, the module skips the whole line.
- **Repeatable:** each occurrence adds names.
- **Without the option:** every built-in app is offered.

The built-in apps are `alarms`, `boxpusher`, `calendar`, `contacts`, `desksparrow`, `diagnostics`, `hotdesking`, `lightup`, `parking`, `queues`, `status`, and `voicemail`.  Disable the apps for services that the server does not have, e.g., `queues` on a server without `app_queue`, and the games where games are not wanted.

### `dphone_theme` { #dphone_theme }

The built-in theme of the phone: a wallpaper together with the colors in which the phone draws every screen.

- **Value:** the name of a theme, e.g., `dusk`: 1 to 32 letters, digits, `_`, and `-`.  The phone's theme selector and the phone's web interface list the themes.  A phone shows the default theme for a name that the firmware does not have.
- **Without the option:** the theme that the user chose on the phone, else the firmware's default theme.
- **Models:** the color models D60, D62, and D65.  The other models have no themes.

A phone section that also has `wallpaper_file` shows that picture, not the theme.

### `dphone_palette_file` { #dphone_palette_file }

The palette file for the wallpaper picture of `wallpaper_file`: the colors in which the phone draws every screen while that picture is the wallpaper.

- **Value:** a file name or a URL, e.g., `lobby.txtpb`.  The module finds the file as `wallpaper_file` is found: a file in `file_directory` is fetched from the module, any other name from the network's `file_url_prefix`, and a full URL is used as written.
- **Without the option:** the phone draws the picture with the phone's palette.
- **Models:** the color models D60, D62, and D65.  The other models have no wallpaper.

The phone fetches the palette file only when the section also has `wallpaper_file`.  The file is a `dphone.palette.v1` message in protobuf text format, which `proto/dphone/palette/v1/palette.proto` in the repository describes, and is at most 16 KiB.

The server's wallpaper picture and palette file replace a theme that the user chose on the phone, as the server's values of the other options of this page do.

## Firmware

### `dphone_ignore_provisioned_firmware` { #dphone_ignore_provisioned_firmware }

Whether the phone ignores the firmware entries of the configuration.

- **Value:** on/off.
- **Without the option:** the phone follows the firmware entries, and installs the image of the phone's entry when that image differs from the running one.

With the option on, the phone keeps the running firmware, whatever `firmware` sections or firmware package the server has.  Use the option for a phone that has to stay on a chosen build, e.g., a development phone on a server that offers every other phone a release.  See [Firmware](administration.md#firmware).

## App data

### `dphone_app_data_tenant` { #dphone_app_data_tenant }

The tenant whose app data the phone reads and writes.

- **Value:** a key of 1 to 32 characters: letters, digits, `.`, `_`, and `-`.
- **Without the option:** the phone shares the store of all phones without a key.

Phones with the same key share one store of app data, and no phone reads or writes the store of another key.  The module never sends the key to a phone, and no request of a phone can name a key: the key comes from the phone section alone.

The module does not treat an invalid value as "no key".  The module logs the option as invalid and refuses every app data request of the phone until the value is corrected, so that a typing mistake never puts a tenant's phone into the shared store.

[App data](administration.md#app-data) describes the store, and the practice for servers with more than one tenant.
