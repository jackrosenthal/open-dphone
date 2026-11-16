# Administration

This page collects the practices for running Open DPMA on a production server, each with the behavior of the module that the practice rests on.  The reader is an administrator who has [installed](installation.md) the module and knows the [configuration file](configuration.md).

## Authentication

### Choose a `config_auth` other than `disabled`

A phone's configuration holds the SIP password of every line of the phone section, and the section's PIN.  With the default `config_auth=disabled`, the server hands a phone section's configuration to any phone that completes the handshake and names the section.

The configuration token that the server then issues is no secret either: the token is made of the phone section's name and the phone's MAC address, and the server accepts a token that begins with a section's name and the MAC address last seen for that section.  The checks of `config_auth` are therefore the control over who gets a configuration.

| Site | Setting | Reason |
| --- | --- | --- |
| Each phone belongs to one desk. | `config_auth=mac`, and `mac` in every phone section | A phone gets only the section with the phone's MAC address, and nobody types a PIN. |
| The same, where MAC addresses are not trusted. | `config_auth=mac_pin` or `mac_globalpin` | The user also has to know a PIN. |
| Users move between phones. | `config_auth=pin`, and `pin` in every phone section | A user proves who the user is; the phone does not matter. |

After a change, read `Config Authentication` in `digium_phones show settings`: the module ignores a value that is misspelled, without a message, and the default, `disabled`, applies.

### Protect the user list

With `userlist_auth=disabled`, a phone that completes the handshake can ask for the list of every phone section, with each section's name and extension.  With `userlist_auth=globalpin`, the phone has to send `globalpin`, or a `group_pin`, and a `group_pin` shows only the sections that have that `group_pin`.  Set `userlist_auth=globalpin` on a server where the list of users is not for everyone on the phones' network, and give each tenant of a shared server a `group_pin`.

### Keep only phone files in `file_directory`

A phone that has completed the handshake can request any file below `file_directory`, by name, without a configuration token and whatever `config_auth` says.  Subdirectories are included; a name that leaves the directory through `..` is refused.  Keep in the directory only what phones are meant to read, and keep in mind that a `config_file` document, which has to be in the directory, can be requested by every phone.

### Watch the authentication incidents

The module records, by source address, each handshake whose certificate fails verification, and each user list request with a wrong PIN.  A record expires 600 seconds after the last failure.  A wrong PIN in a request for a configuration token is refused, and is not recorded.

- Each failure is logged at the level `DPMA` as `Auth failed for ip '<address>' mac '<MAC>' reason '<reason>' count=<n>`.  From the fifth failure of an address, the line ends in `ALERT`.
- The AMI action `DPMAShowIncidents` lists the records, and `DPMARemoveIncident` with the header `IPAddress` removes one.

The module blocks nothing: an address with `Blocking: Yes` in the list is still answered.  To block an address, act on the log line or the AMI list with the server's firewall.

### Decide who may use each phone's interfaces

- `web_ui_enabled` is off by default.  When switching the web interface on, set `login_password` in the same phone section, so that the phone does not keep a default password.
- On Open DPhone, the `/metrics` endpoint needs no login and is on by default, and SSH is off by default.  See [`dphone_metrics_enabled`](open-dphone-options.md#dphone_metrics_enabled) and [`dphone_ssh_enabled`](open-dphone-options.md#dphone_ssh_enabled).
- A value that a user sets on an Open DPhone phone goes before the `dphone_*` option of the server.  A user who can log in to the phone's web interface can therefore switch SSH on.  Where that matters, leave `web_ui_enabled` off.

## Pushing a changed configuration

`module reload res_open_dpma.so` makes the module read `res_digium_phone.conf` again.  The reload alone tells a running phone about the change in three cases only: the lines of the phone section changed, the voicemail password behind `pin=voicemail` changed, or the phone section was removed.  For every other change, tell the phones:

| Interface | Command or action | Effect |
| --- | --- | --- |
| CLI | `digium_phones reconfigure phone <name>` | Tells one phone to fetch the configuration. |
| CLI | `digium_phones reconfigure all` | Tells every phone. |
| AMI | `DigiumPhoneReconfigure` with the header `PhoneName` | As the first command.  Needs the `message` authority. |
| AMI | `DigiumPhoneReconfigureAll` | As the second command.  Needs the `message` authority. |

The same holds for changes in `pjsip.conf`.  The module follows a changed endpoint, `auth` section, or AOR, and holds the new password or mailbox of the line at once, but does not tell the phone.  After changing a line's SIP password, reconfigure the phone, or the phone keeps registering with the old password.

A reconfigure of one phone fails with `Phone ... has not contacted the server` when the server does not know the phone's address.  Such a phone gets the new configuration at the phone's next start.

The server states the MD5 digest of a phone's configuration when the phone presents the configuration token, and a phone downloads the configuration only when the digest differs from that of the configuration that the phone has.

## Firmware

### How a phone decides to install

A phone acts on the first firmware entry for the phone's model: from the `firmware` sections that the phone section lists, or, for a phone section without `firmware`, from the [firmware package](configuration.md#firmware-package).

| Firmware on the phone | The phone installs the entry's image when |
| --- | --- |
| Stock firmware | The `version` text of the entry differs from the version that the phone runs. |
| Open DPhone | The image of the entry is not the image that the phone runs.  The phone compares the checksums and sizes of the image's kernel and root file system; `version` is a label only. |

An Open DPhone phone learns the image's checksum from the entry when the module supplied one, and otherwise by reading the first kilobytes of the image from the URL.  The module supplies the checksum when the image file is on the server: in `file_directory` for a `firmware` section, or in the package directory for the firmware package.  Keep the images there, under the names that the entries use, so that a phone that already runs the image settles the question without a download.

The module does not serve the images.  A web server has to serve the directory under the URL that `file_url_prefix` of the network gives, and an entry is sent only to a network that has a `file_url_prefix`.

### Moving phones between stock firmware and Open DPhone

Both firmwares install an image of either kind, so a firmware entry is the way to move a phone in both directions.

To move the D65 phones of a server to Open DPhone, name the Open DPhone image of the release, whose file name is `<version>_<model>_firmware.eff`:

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

A phone with stock firmware installs the image because the version text differs.  To move the phones back, change `version` and `file` to those of the stock image; an Open DPhone phone installs the stock image because the image differs from the running one.  In both directions, [reconfigure](#pushing-a-changed-configuration) the phones afterward, or wait for each phone's next start.

Practices that follow from the behavior above:

- **One firmware section for each model.**  A phone refuses an image that was made for another model, and looks only at entries for the phone's model.
- **Start with one phone.**  Give the new firmware section to one phone section first, and to the template of the others after that phone has come up.
- **Keep a fleet of two firmwares in two templates.**  Every phone of a section follows the section's entries, whatever firmware the phone runs.  Phones that stay on stock firmware need a template whose `firmware` names the stock image, or no entry for the model at all.
- **Mind the firmware package.**  The package applies to every phone section without a `firmware` option.  A new image in the package directory reaches all of those phones.
- **Exempt single phones with [`dphone_ignore_provisioned_firmware`](open-dphone-options.md#dphone_ignore_provisioned_firmware).**  An Open DPhone phone with the option keeps the running firmware.  Stock firmware does not know the option.

An Open DPhone phone remembers an image that failed to install, or that was installed and did not stay the running firmware, and does not try the same image again until the entry names another image.  A phone that keeps running the previous firmware after a firmware change may have such an image; the [Dev Guide](../dev-guide/index.md) covers looking at a phone.

## App data

The apps of Open DPhone phones keep data on the server, e.g., the best times of a game, which every phone then shows, or a user's progress, which returns after the phone is reset.  There is nothing to switch on: the module answers the phones' app data requests and keeps the data in the Asterisk database, in the family `DPhoneAppData`.  Stock firmware and the original module have no app data.  The [App SDK](../app-sdk/index.md) describes app data from the side of an app.

Each app has keys that all phones of a store share, and keys for each user.  The user is the phone section that the phone is provisioned as.

| Limit | Value |
| --- | --- |
| Shared keys of one app | 1024 |
| Keys of one user in one app | 64 |
| Length of a key | 64 characters |
| Size of a value | 4096 bytes |

### Give every phone of a multi-tenant server a tenant key

Without [`dphone_app_data_tenant`](open-dphone-options.md#dphone_app_data_tenant), all phones of the server share one store: the phones of one tenant see the names and scores that the users of another tenant wrote.  On a server with more than one tenant, set a key for each tenant in the tenant's template:

```ini title="res_digium_phone.conf"
[north-office](!)
type=phone
dphone_app_data_tenant=north-office

[phone101](north-office)
line=101
```

- Give a key to every phone section.  A section without the option is in the shared store of the phones without a key.
- Check a new section with `digium_phones show phone <name>`.  A value that is not a valid key leaves the phone without app data, and the log names the option as invalid.
- Choose keys that differ in more than the case of letters, and avoid `_` in keys.  The module keeps the stores of such keys apart, but `database deltree` matches without regard to case and reads `_` as a wildcard, so removing the data of `north_office` would also remove the data of `north-office`.
- Do not rename a tenant key or a phone section without need.  The data stays under the key and the section name that the data was written with: a phone with a new key sees the other store, and a renamed phone section has none of the user's keys.

Two other lists cross tenants on a shared server, whatever the tenant keys say: the user list, unless `userlist_auth=globalpin` and a `group_pin` for each tenant limit the list, and the module's contact list of internal extensions, which has every phone section with a line.

### Inspecting and removing app data

The entries of the family `DPhoneAppData` have the following keys:

| Key | Contents |
| --- | --- |
| `@<tenant>/<app>/s/<key>` | A shared entry of an app.  `@` alone, without a tenant, is the store of the phones without a key. |
| `@<tenant>/<app>/u/<phone section>/<key>` | An entry of one user. |
| `watch/<phone section>` | The apps whose changes the phone is told about. |
| `seq` | The version number that the module gave out last. |

In a phone section's name, every character other than a letter, a digit, `.`, and `-` is written as `=` and two hexadecimal digits.  A value is a JSON object with the version, the time of writing, the author, and the data in Base64.

```text
*CLI> database show DPhoneAppData
*CLI> database deltree DPhoneAppData @north-office/lightup
*CLI> database deltree DPhoneAppData @/lightup/u/phone101
*CLI> database deltree DPhoneAppData @north-office
```

The second command removes one app's data of one tenant, the third the data of one user of the phones without a key, and the fourth everything of one tenant.  Removing a phone section from the configuration file does not remove the section's app data; remove the data with `database deltree` when a user leaves.  Leave `seq` in place: the entry is the counter of the version numbers that the module gives to entries.

## Seeing the phones

| Interface | Command or action | Shows |
| --- | --- | --- |
| CLI | `digium_phones show sessions` | Each session: the age, the seconds since the last message, the phone's URI, whether the phone authenticated and was configured, and the MAC address. |
| CLI | `digium_phones show tokens` | Each phone section that a phone has taken: the MAC address, the URI, and the session. |
| CLI | `digium_phones show phone <name>` | The options that the module holds for a phone section. |
| CLI | `digium_phones show status <name>` | The presence states that the phone offers, with the current one marked. |
| AMI | `DPMAShowPhones` | One `DPMAShowPhonesList` event for each session, with the fields of `show sessions`, the user agent (the model and firmware version of the phone), and the phone section.  Needs the `call` authority. |

`DPMAShowPhones` is the action to poll from a monitoring system: a phone section that should have a phone and has no session with `Configured: 1` is a phone that is off, unreachable, or refused.  The user agent in the same event tells which phones run which firmware version, e.g., during a firmware change.

The module also has the AMI actions `DPMAMessageRegister`, `DPMAMessageResponse`, and `DPMAMessageSend`, with which an AMI client answers phone app requests that the module does not handle.  A request that such a client registered for and does not answer within 300 seconds is dropped.

## Backups

The module keeps state in three families of the Asterisk database:

| Family | Contents | When lost |
| --- | --- | --- |
| `DPhoneAppData` | The app data. | The data is gone; nothing else holds a copy. |
| `DigiumPhoneSessions` | The sessions of the phones, each with the session's password. | Each phone starts a new session with a handshake. |
| `DigiumPhoneUsers` | For each phone section: the MAC address, the URI, and the session of the phone that took the section. | The server cannot send a reconfigure to a phone until the phone contacts the server again. |

- Back up the Asterisk database for the sake of `DPhoneAppData`.  The other two families rebuild as the phones contact the server.
- Treat a copy of the database as a secret.  The session passwords in `DigiumPhoneSessions` are encrypted with a passphrase that is the same in every copy of the module and of the original module, which protects against nothing more than a casual look.
- Back up `res_digium_phone.conf`, the files of `file_directory`, and the files named by `appserver_certificate` and `phone_ca_file` with the rest of Asterisk's configuration.  The configuration file holds PINs and the passwords of external lines.

## Upgrades

Open DPMA and the Open DPhone firmware are released together and have one version number.  Upgrading the module does not change the firmware of the phones: a phone changes firmware only through [firmware entries](#firmware).

1. Download the module of the new release and verify the checksum, as in [Get a release build](installation.md#get-a-release-build).  Keep the file of the running version.
2. Replace `res_open_dpma.so` in the module directory.
3. Restart Asterisk.  `module reload` and `module unload` followed by `module load` do not bring in the new file: the module's code stays in the Asterisk process until the process ends.
4. Check `digium_phones show version`, and that phones have sessions, as in [Confirm that the module works](installation.md#confirm-that-the-module-works).

The sessions are restored from the Asterisk database after the restart, so the phones keep working without a new handshake.  To go back, put the kept file in place and restart Asterisk again.

When the Asterisk server is upgraded to another major version, install the module that was built for that version at the same time: a module is built against the headers of one major version of Asterisk.
