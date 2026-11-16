# Administration

## Authentication

### Configuration authentication

A phone's configuration contains the SIP password for every line in the phone section, along with the section's PIN.  With the default, `config_auth=disabled`, the server hands a phone section's configuration to any phone that completes the handshake and names the section.  Choose another value on any production server:

| Site | Setting | Result |
| --- | --- | --- |
| Each phone belongs to one desk. | `config_auth=mac`, with `mac` in every phone section | A phone gets only the section that matches the phone's MAC address, and nobody types a PIN. |
| Each phone belongs to one desk, but MAC addresses are not trusted. | `config_auth=mac_pin` or `mac_globalpin` | The user must also know a PIN. |
| Users move between phones. | `config_auth=pin`, with `pin` in every phone section | The PIN identifies the user at any phone. |

After a change, check `Config Authentication` in the output of `digium_phones show settings`.  The module does not apply a misspelled value, so `disabled` stays in effect, and the log has a warning that names the value.

#### Configuration tokens

A phone that passes the `config_auth` check receives a configuration token and presents the token in place of the PIN from then on, e.g., at every start.  Each token contains two values that the server computes from the phone section's name, the phone's MAC address, and a key that only the server has.  Knowing a section's name and a phone's MAC address is therefore not enough to forge a token.

The server accepts a token without those values in two cases:

- **The phone section has never been issued a token with the values.**  The section's phone got a token from the original module, or from an early version of Open DPMA that issued the same kind of token.  For such a section, the server behaves like the original module and accepts any token that begins with the section's name and the MAC address last seen for the section.  The exception ends the next time a phone requests a token for the section, e.g., when a user logs in on a phone, or after a factory reset.  `database show OpenDPMATokens` lists the sections that are past this stage as `issued/<section>`.
- **The Asterisk database was lost.**  Until a phone requests a token for a section, the server behaves like the original module and accepts any token that begins with the section's name.

These exceptions exist because stock firmware does not request a new token after a token is refused.  The phone shows an error message and waits for someone to press **Retry**.

### User list

With `userlist_auth=disabled`, any phone that completes the handshake can request the list of all phone sections, including each section's name and extension.  With `userlist_auth=globalpin`, the phone must send `globalpin` or a `group_pin`, and a `group_pin` reveals only the sections with the same `group_pin`.

Set `userlist_auth=globalpin` unless everyone on the phones' network is allowed to see the list of users.  On a shared server, give each tenant a `group_pin`.

### File directory

A phone that has completed the handshake can request any file under `file_directory` by name.  The request needs no configuration token and is not subject to `config_auth`.  Subdirectories are included, and names that escape the directory with `..` are refused.

Use the directory only for files that every phone may read.  A `config_file` document must be in the directory, so any phone can request the document.

### Authentication incidents

The module records the following failures by source address:

- a handshake with a certificate that fails verification,
- a user list request with a wrong PIN, and
- a configuration token request with a wrong PIN.  The reason is `invalid pin` for a phone section's `pin`, and `invalid config pass` for `globalpin` or a `group_pin`.

A record expires 600 seconds after the last failure.  To review the records:

- Each failure is logged at the `DPMA` level as `Auth failed for ip '<address>' mac '<MAC>' reason '<reason>' count=<n>`.  Starting with an address's fifth failure, the line ends in `ALERT`.
- The AMI action `DPMAShowIncidents` lists the records, and `DPMARemoveIncident` with the `IPAddress` header removes one.

!!! note "The module does not block addresses"

    The module keeps answering an address even when the list shows `Blocking: Yes`.  To block an address, feed the log line or the AMI list to the server's firewall.

### Web interface, SSH, and metrics

- `web_ui_enabled` is off by default.  When you turn the web interface on, set `login_password` in the same phone section so that the phone does not keep a default password.
- On Open DPhone, the `/metrics` endpoint requires no login and is on by default, and SSH is off by default.  See [`dphone_metrics_enabled`](open-dphone-options.md#dphone_metrics_enabled) and [`dphone_ssh_enabled`](open-dphone-options.md#dphone_ssh_enabled).
- On Open DPhone, a `dphone_*` option set on the server cannot be changed from the phone's web interface.  Without the option, the user decides.  To keep users from turning SSH on, set `dphone_ssh_enabled=no` in the phone section, or leave `web_ui_enabled` off.

## Configuration changes

`module reload res_open_dpma.so` makes the module reread `res_digium_phone.conf`.  A reload notifies running phones in only three cases: the phone section's lines changed, the voicemail password behind `pin=voicemail` changed, or the phone section was removed.  For any other change, reconfigure the phones:

| Interface | Command or action | Effect |
| --- | --- | --- |
| CLI | `digium_phones reconfigure phone <name>` | Tells one phone to fetch the configuration. |
| CLI | `digium_phones reconfigure all` | Tells every phone to fetch the configuration. |
| AMI | `DigiumPhoneReconfigure` with the `PhoneName` header | Same as the first command.  Requires the `message` authority. |
| AMI | `DigiumPhoneReconfigureAll` | Same as the second command.  Requires the `message` authority. |

Changes in `pjsip.conf` also require a reconfigure.  The module picks up a changed endpoint, `auth` section, or AOR immediately, but does not notify the phone.  After you change a line's SIP password, reconfigure the phone, or the phone keeps registering with the old password.

Reconfiguring a single phone fails with `Phone ... has not contacted the server` when the server does not know the phone's address.  That phone gets the new configuration at the next start.

Reconfiguring every phone is cheap.  The server sends each phone the MD5 digest of the phone's configuration, and a phone downloads the configuration only when the digest differs from the digest of the configuration that the phone already has.

## Firmware

### Update criteria

A phone acts on the first firmware entry for the phone's model.  The entries come from the `firmware` sections listed in the phone section or, for a phone section without `firmware`, from the [firmware package](configuration.md#firmware-package).

| Phone firmware | The phone installs the image when |
| --- | --- |
| Stock firmware | The entry's `version` text differs from the version that the phone is running. |
| Open DPhone | The entry's image is not the image that the phone is running.  The phone compares the checksums and sizes of the image's kernel and root file system, and treats `version` as a label only. |

An Open DPhone phone gets the image's checksum from the entry when the module supplies one.  Otherwise, the phone reads the first few kilobytes of the image from the URL.  The module supplies the checksum when the image file is on the server: in `file_directory` for a `firmware` section, or in the package directory for the firmware package.  Keep the images there, under the file names used in the entries, so that a phone already running the image can tell without a download.

The module does not serve the images.  A web server must serve the directory at the URL given by the network's `file_url_prefix`, and entries are sent only to networks that have a `file_url_prefix`.

### Migration between stock firmware and Open DPhone { #firmware-migration }

Stock firmware and Open DPhone can each install the other's images, so a firmware entry moves phones in either direction.

To move a server's D65 phones to Open DPhone, add an entry for the Open DPhone image from the release.  Image files are named `<version>_<model>_firmware.eff`.

```ini title="res_digium_phone.conf"
[fw-d65]
type=firmware
model=D65
version=3_0_0
file=3_0_0_D65_firmware.eff

[open-dphone](!)
type=phone
firmware=fw-d65
```

A phone running stock firmware installs the image because the version text differs.  To move the phones back, change `version` and `file` to match the stock image.  A phone running Open DPhone then installs the stock image because the image differs from the running one.  In both directions, [reconfigure](#configuration-changes) the phones afterward, or wait for each phone to restart.

Recommendations:

- **Use one firmware section per model.**  A phone rejects an image built for another model, and considers only the entries for the phone's model.
- **Start with one phone.**  Assign the new firmware section to a single phone section first.  Once that phone comes up, add the firmware section to the template for the rest.
- **Use separate templates for a mixed fleet.**  Every phone in a section follows the section's entries, regardless of the firmware that the phone is running.  Phones that stay on stock firmware need a template whose `firmware` names the stock image, or no entry for the model at all.
- **Watch the firmware package.**  The package applies to every phone section without a `firmware` option, so a new image in the package directory reaches all of those phones.
- **Exempt individual phones with [`dphone_ignore_provisioned_firmware`](open-dphone-options.md#dphone_ignore_provisioned_firmware).**  An Open DPhone phone with the option keeps the running firmware.  Stock firmware does not recognize the option.

An Open DPhone phone remembers an image that failed to install, or that was installed but rolled back, and does not try the same image again until the entry names a different image.  If a phone is still running the previous firmware after a firmware change, such an image may be the reason.  See [Firmware updates](../dev-guide/firmware-updates.md#dphone-update-status) in the Dev Guide to inspect the phone's firmware banks.

## App data

Apps on Open DPhone phones can store data on the server, such as a game's best times, which then appear on every phone, or a user's progress, which survives a factory reset.  App data requires no configuration: the module answers the phones' app data requests and stores the data in the Asterisk database, in the `DPhoneAppData` family.  Stock firmware and the original module do not support app data.

Each app has shared keys, which all phones in a store can access, and per-user keys.  The user is the phone section that the phone is provisioned as.

The firmware stores the user's theme the same way, as the per-user key `theme` under the reserved app name `dphone`.

| Limit | Value |
| --- | --- |
| Shared keys per app | 1024 |
| Keys per user per app | 64 |
| Key length | 64 characters |
| Value size | 4096 bytes |

### Multi-tenant servers

Without [`dphone_app_data_tenant`](open-dphone-options.md#dphone_app_data_tenant), all phones on the server share one store, and one tenant's phones see the names and scores that another tenant's users wrote.  On a multi-tenant server, set a key in each tenant's template:

```ini title="res_digium_phone.conf"
[north-office](!)
type=phone
dphone_app_data_tenant=north-office

[phone101](north-office)
line=101
```

- Give every phone section a key.  A section without the option uses the store shared by all phones without a key.
- Check each new section with `digium_phones show phone <name>`.  An invalid key leaves the phone without app data, and the log reports the option as invalid.
- A key is 1 to 32 characters long and may contain lowercase letters, digits, `.`, and `-`.  Because `database deltree` matches case-insensitively and treats `_` as a single-character wildcard, keys cannot contain uppercase letters or `_`.  Deleting one tenant's data therefore never deletes another tenant's.
- Avoid renaming tenant keys and phone sections.  Data stays under the key and section name that were in effect when the data was written.  A phone with a new key sees a different store, and a renamed phone section loses access to the user's keys.

Two other lists cross tenant boundaries on a shared server, regardless of tenant keys:

- The user list, unless `userlist_auth=globalpin` and a `group_pin` per tenant restrict the list.
- The module's contact list of internal extensions, which includes every phone section that has a line.

### Database layout

Entries in the `DPhoneAppData` family have the following keys:

| Key | Contents |
| --- | --- |
| `@<tenant>/<app>/s/<key>` | An app's shared entry.  A bare `@` with no tenant is the store for phones without a key. |
| `@<tenant>/<app>/u/<phone section>/<key>` | One user's entry. |
| `watch/<phone section>` | The apps whose changes the phone is notified about. |
| `seq` | The most recent version number that the module assigned. |

In a phone section's name, every character other than a letter, a digit, `.`, or `-` is written as `=` followed by two hexadecimal digits.  Each value is a JSON object with the version, the time of writing, the author, and the data in Base64.

To list all app data:

```text
*CLI> database show DPhoneAppData
```

To delete one app's data for one tenant:

```text
*CLI> database deltree DPhoneAppData @north-office/lightup
```

To delete one user's data for one app, in the store for phones without a key:

```text
*CLI> database deltree DPhoneAppData @/lightup/u/phone101
```

To delete all of one tenant's data:

```text
*CLI> database deltree DPhoneAppData @north-office
```

Keep the following in mind when deleting data:

- Removing a phone section from the configuration file does not delete the section's app data.  Run `database deltree` when a user leaves.
- Within a tenant, `database deltree` on an app name that contains `_` also matches app names of the same length that differ only at that position, e.g., `@north-office/word_game` also matches `@north-office/word.game`.
- Do not delete `seq`.  The entry is the counter for the version numbers that the module assigns to entries.

## Monitoring

| Interface | Command or action | Output |
| --- | --- | --- |
| CLI | `digium_phones show sessions`{ .nowrap } | Each session: the age, the seconds since the last message, the phone's URI, whether the phone is authenticated and configured, and the MAC address. |
| CLI | `digium_phones show tokens`{ .nowrap } | Each phone section that a phone has claimed: the MAC address, the URI, and the session. |
| CLI | `digium_phones show phone <name>`{ .nowrap } | The options that the module holds for a phone section. |
| CLI | `digium_phones show status <name>`{ .nowrap } | The presence states that the phone offers, with the current state marked. |
| AMI | `DPMAShowPhones` | One `DPMAShowPhonesList` event per session, with the fields of `show sessions`, the user agent (the phone's model and firmware version), and the phone section.  Requires the `call` authority. |

Poll `DPMAShowPhones` from a monitoring system.  A phone section that should have a phone but has no session with `Configured: 1` means that the phone is off, unreachable, or refused.  The user agent in the same event shows which phones are running which firmware version, which is useful during a firmware rollout.

## AMI message forwarding

The AMI actions `DPMAMessageRegister`, `DPMAMessageResponse`, and `DPMAMessageSend` let an AMI client answer phone app requests that the module does not handle.  A request that a client registered for but does not answer within 300 seconds is dropped.

## Backups

The module stores state in four families of the Asterisk database:

| Family | Contents | If lost |
| --- | --- | --- |
| `DPhoneAppData` | The app data. | The data is gone.  No other copy exists. |
| `DigiumPhoneSessions` | The phones' sessions, each with a session password. | Each phone starts a new session with a handshake. |
| `DigiumPhoneUsers` | For each phone section: the MAC address, the URI, and the session of the phone that claimed the section. | The server cannot send a reconfigure to a phone until the phone contacts the server again. |
| `OpenDPMATokens` | The key for [configuration tokens](#configuration-tokens), and the phone sections that have been issued a token made with the key. | The phones keep working.  For each phone section, the server accepts a token without the key's values until a phone requests a token for the section. |

- **Back up the Asterisk database to preserve `DPhoneAppData`.**  The module rebuilds the other three families as phones contact the server.
- **Treat database backups as secrets.**  The session passwords in `DigiumPhoneSessions` are encrypted with a passphrase that is the same in every copy of Open DPMA and of the original module, so the encryption only prevents casual inspection.  The key in `OpenDPMATokens` can mint a configuration token for any phone section.
- **Back up the module's files with the rest of the Asterisk configuration.**  These are `res_digium_phone.conf`, the contents of `file_directory`, and the files named by `appserver_certificate` and `phone_ca_file`.  The configuration file contains PINs and the passwords for external lines.

## Upgrades

Open DPMA and the Open DPhone firmware are released together under one version number.  Upgrading the module does not change the phones' firmware; phones change firmware only through [firmware entries](#firmware).

1. Download the module from the new release and verify the checksum, as described in [Download](installation.md#download).  Keep the file for the running version.
2. Replace `res_open_dpma.so` in the module directory.
3. Restart Asterisk.  Neither `module reload` nor `module unload` followed by `module load` picks up the new file, because the module's code stays in the Asterisk process until the process exits.
4. Check `digium_phones show version`, and check that phones have sessions, as described in [Verification](installation.md#verification).

Sessions are restored from the Asterisk database after the restart, so phones keep working without a new handshake.  To roll back, restore the old file and restart Asterisk again.

When you upgrade Asterisk to a new major version, install the module built for that version at the same time.  Each module is built against the headers of one Asterisk major version.
