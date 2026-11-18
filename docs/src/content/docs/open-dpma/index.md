---
title: "Open DPMA"
---

Open DPMA (`res_open_dpma.so`) is an open-source Asterisk module that replaces the original `res_digium_phone` and the two PJSIP helper modules that ship with the original module.  Open DPMA reads `res_digium_phone.conf`, registers the original module's CLI commands and AMI actions, and restores the sessions that the original module saved in the Asterisk database, so phones stay connected across the switch.  Open DPMA works with every D-series phone, with either firmware.

## Differences from the original module

### Unsupported features

- **`chan_sip`.**  Open DPMA supports PJSIP only.  Lines are not read from `sip.conf` or from `hassip` users in `users.conf`.
- **Licensing.**  Open DPMA has no license check; the CLI command `digium_phones license status` and the AMI action `DPMALicenseStatus` do not exist.

### Changed behavior

| Case | Open DPMA | Original module |
| --- | --- | --- |
| A misspelled `config_auth` or `userlist_auth` value | Logs an error and refuses every configuration token request or user list request until the value is corrected.  See [`config_auth`](configuration.md#config_auth). | Ignores the value silently, so the default, `disabled`, applies. |
| `enable_check_sync` removed from the file | Turns the option off. | Keeps the option on until the file says `enable_check_sync=no`. |
| `network_vlan_qos` in a network section | Sets the RTP priority. | Has no effect. |
| A network section without `registration_port` | Sends the transport's default port: 5061 for TLS, 5060 otherwise. | Sends an empty port. |
| A PJSIP endpoint change | `type=line` options keep overriding the PJSIP values. | Replaces or clears the section's `context`, `callerid`, `mailbox`, `secret`, and `authname` until the next reload. |
| A phone section without `active_locale` | Sends no locale. | Sends an empty locale, which stock firmware cannot load. |
| Text with `"`, `<`, or `&` | Escapes the characters. | Writes the text unescaped, producing a document that phones cannot parse. |
| A phone file request with a `..` component | Refuses the request. | Opens the path, which exposes every file that the Asterisk process can read. |
| Configuration tokens | Contain values that only the server can compute.  A section issued such a token accepts no other kind.  Tokens from the original module keep working; see [Configuration authentication](administration.md#configuration-authentication). | Accepts any token that begins with the section's name and the MAC address last seen for the section, regardless of `config_auth`. |
| A configuration token request with a wrong PIN | Records an [authentication incident](administration.md#authentication-incidents). | Records only wrong PINs in user list requests. |
| Configuration downloads | Sends each phone the configuration's MD5 digest, so a phone downloads the configuration only after a change. | A phone downloads the configuration at every start. |
| Firmware entries | Include the image's checksum when the image file is on the server; see [Firmware](administration.md#firmware). | Have no checksum. |
| Service discovery | Answers mDNS queries without `avahi-daemon`, as `<hostname>-dpma.local`. | Requires `avahi-daemon`. |
| Several requests and events, e.g., a PJSIP AOR change while a line has no PJSIP endpoint, a voicemail request that names no folder, and a voicemail request for zero messages per page | Handles them. | Crashes Asterisk. |
| A phone request forwarded to an AMI client and never answered | Drops the request after 300 seconds. | Keeps the request until the module is unloaded. |
| A phone's request to start recording a call | Starts the recording and reports success. | Starts the recording and reports an error. |

### Additions

- The [Open Desk Phone options](opendeskphone-options.md) and [app data](administration.md#app-data), for phones running Open Desk Phone.
- The `[general]` options [`appserver_certificate` and `phone_ca_file`](configuration.md#general).
