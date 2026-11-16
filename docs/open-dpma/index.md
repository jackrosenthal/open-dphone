# Open DPMA

Open DPMA is an Asterisk module, `res_open_dpma.so`, that provisions Digium D-series phones and answers the requests of the phones' apps.  The module is the Open DPhone project's implementation, in Rust, of DPMA, the Digium Phone Module for Asterisk, and takes the place of the original `res_digium_phone` and the two PJSIP helper modules that come with the original.

This section is for the administrator of an Asterisk server that has D-series phones, with stock firmware or with Open DPhone.

| Page | Contents |
| --- | --- |
| [Installation](installation.md) | What the module needs, getting a build, replacing the original module, and confirming that phones are provisioned. |
| [Configuration](configuration.md) | Every section and option of `res_digium_phone.conf` that the module reads. |
| [Options for Open DPhone](open-dphone-options.md) | The `dphone_*` options, which only phones that run Open DPhone honor. |
| [Administration](administration.md) | Practices for authentication, firmware, app data, monitoring, backups, and upgrades. |

## Which phones work with the module

Open DPMA speaks the DPMA protocol to every D-series phone, whatever firmware the phone runs.

| Firmware on the phone | What Open DPMA provides |
| --- | --- |
| Stock firmware | What the original module provides.  A phone ignores the `dphone_*` settings, and stock firmware has no app data. |
| Open DPhone | The same, and also the [options for Open DPhone](open-dphone-options.md) and [app data](administration.md#app-data) kept on the server. |

A phone that runs Open DPhone also works with the original module, without the `dphone_*` options and without app data.

## Differences from the original module

Open DPMA reads the configuration file of the original module, `res_digium_phone.conf`, and registers the CLI commands and AMI actions of the original under the same names.  An administrator who moves a server from the original module is affected by the following differences.

**Not supported**

- `chan_sip`.  Open DPMA works with PJSIP only: lines are not read from `sip.conf` or from the `hassip` users of `users.conf`.
- Licensing.  Open DPMA has no license check, so the CLI command `digium_phones license status` and the AMI action `DPMALicenseStatus` do not exist.

**Changed**

- The module's name is `res_open_dpma`, e.g., in `module reload res_open_dpma.so` and in `modules.conf`.
- A phone file request whose file name has a `..` component is refused.  The original module opens such a name, which reaches every file that the Asterisk process can read.
- The server tells a phone the MD5 digest of the phone's configuration.  A phone therefore downloads the configuration only after a change; with the original module a phone downloads the configuration at every start.
- A phone section without `active_locale` sends no locale to the phone.  The original module sends an empty locale, which stock firmware cannot load.
- Service discovery does not use `avahi-daemon`.  The module answers mDNS queries directly, and the service's host name is `<hostname>-dpma.local`.
- A phone request that was forwarded to an AMI client and never answered is dropped after 300 seconds.  The original module keeps such requests until the module is unloaded.
- A firmware entry carries a checksum of the firmware image when the image file is on the server.  See [Firmware](administration.md#firmware).
- A session record in the Asterisk database that cannot be decrypted is deleted, and the phone starts a new session.
- Several requests and events that crash Asterisk with the original module are handled, e.g., a change of a PJSIP AOR while a line has no PJSIP endpoint, a voicemail request that names no folder, and a voicemail request for pages of zero messages.
- A phone's request to start recording a call reports success.  The original module starts the recording and reports an error to the phone.

**Added**

- The [options for Open DPhone](open-dphone-options.md).
- [App data](administration.md#app-data) for the apps of Open DPhone phones.
- The `[general]` options [`appserver_certificate` and `phone_ca_file`](configuration.md#general).

Open DPMA restores the sessions that the original module saved in the Asterisk database, so a phone keeps the session across the change of module.
