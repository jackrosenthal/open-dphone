# Open DPMA

Open DPMA is an open-source Asterisk module, `res_open_dpma.so`, that replaces the original `res_digium_phone` along with the two PJSIP helper modules that ship with the original.  Open DPMA reads the same configuration file, `res_digium_phone.conf`, and registers the same CLI commands and AMI actions as the original module.  Sessions that the original module saved in the Asterisk database are restored, so phones stay connected when you switch modules.

Open DPMA works with every D-series phone, regardless of firmware.

## Differences from the original module

### Unsupported features

- **`chan_sip`.**  Open DPMA supports PJSIP only.  Lines are not read from `sip.conf` or from `hassip` users in `users.conf`.
- **Licensing.**  Open DPMA has no license check, so the CLI command `digium_phones license status` and the AMI action `DPMALicenseStatus` do not exist.

### Changed behavior

Configuration:

- A misspelled `config_auth` or `userlist_auth` value produces a warning.  The original module silently ignores the value.
- `enable_check_sync` turns off again when the line is removed from the configuration file.  With the original module, the option stays on until the file says `enable_check_sync=no`.
- `network_vlan_qos` works in a network section.
- A network section without `registration_port` sends the phone the default port for the transport: 5061 for TLS, and 5060 otherwise.  The original module sends an empty port.
- Options in a `type=line` section keep overriding the PJSIP values after the PJSIP endpoint changes.  With the original module, an endpoint change replaces or clears the section's `context`, `callerid`, `mailbox`, `secret`, and `authname` until the next reload.
- A phone section without `active_locale` sends no locale to the phone.  The original module sends an empty locale, which stock firmware cannot load.
- Text values containing `"`, `<`, or `&` are escaped in the phone's configuration.  The original module writes such values unescaped, which produces a document that phones cannot parse.

Security:

- A phone file request with a `..` component in the file name is refused.  The original module opens such paths, which exposes every file that the Asterisk process can read.
- Configuration tokens contain values that only the server can compute, and a phone section that has been issued such a token accepts no other kind.  The original module accepts any token that begins with the section's name and the MAC address last seen for the section, regardless of `config_auth`.  Phones holding a token from the original module keep working.  See [Configuration authentication](administration.md#configuration-authentication).
- A configuration token request with a wrong PIN is recorded as an [authentication incident](administration.md#authentication-incidents).  The original module records only wrong PINs in user list requests.

Provisioning:

- The server sends each phone the MD5 digest of the phone's configuration, so a phone downloads the configuration only after a change.  With the original module, a phone downloads the configuration at every start.
- A firmware entry includes a checksum of the firmware image when the image file is on the server.  See [Firmware](administration.md#firmware).
- Service discovery does not require `avahi-daemon`.  The module answers mDNS queries directly, and the service's host name is `<hostname>-dpma.local`.

Reliability:

- Several requests and events that crash Asterisk with the original module are handled correctly, e.g., a PJSIP AOR change while a line has no PJSIP endpoint, a voicemail request that names no folder, and a voicemail request that asks for zero messages per page.
- A phone request that was forwarded to an AMI client and never answered is dropped after 300 seconds.  The original module keeps such requests until the module is unloaded.
- A phone's request to start recording a call reports success.  The original module starts the recording but reports an error to the phone.

### Additions

- The [Open DPhone options](open-dphone-options.md) and [app data](administration.md#app-data), for phones running Open DPhone.
- The `[general]` options [`appserver_certificate` and `phone_ca_file`](configuration.md#general).
