# Configuration reference

Open DPMA is configured in `res_digium_phone.conf` in the Asterisk configuration directory.

## File format

The file has a `[general]` section and any number of other sections.  Every other section has a `type` option, and the section's name identifies the phone, line, network, or other object that the section describes.

The following rules apply throughout the file:

- **On/off options.**  An on/off option accepts any value that Asterisk reads as true (`yes`, `true`, `on`, `1`) or false.
- **Errors.**  An unknown option or an unreadable value is logged at the `DPMA` level and skipped, and the rest of the section still applies.  A section with a missing or unknown `type` is logged and skipped.
- **Special characters.**  Text values end up in the phone's configuration, which is an XML document.  The module escapes `"`, `<`, and `&`, so you can write any text exactly as the phone should display the text.  An `&` that begins a reference, e.g., `&amp;` or `&#38;`, is left alone.  Values written with references for the original module, which escapes nothing, therefore display the same way.
- **Comments.**  A `;` begins a comment unless written as `\;`.

A reload (`module reload res_open_dpma.so`) does not send the new configuration to running phones, except for phones whose lines changed.  See [Configuration changes](administration.md#configuration-changes).

## Minimal example

```ini title="res_digium_phone.conf"
[general]
config_auth=mac
registration_address=192.0.2.10
registration_port=5060

[phone101]
type=phone
mac=000fd3000001
line=101
```

`registration_address` gives every phone a network.  A phone without a network gets a configuration with no SIP accounts.

## `[general]` { #general }

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `config_auth` | `disabled`, `mac`, `pin`, `globalpin`, `mac_pin`, or `mac_globalpin` | `disabled` | What a phone must prove before the server hands out a phone section's configuration.  See [`config_auth`](#config_auth). |
| `userlist_auth` | `disabled` or `globalpin` | `disabled` | With `globalpin`, a phone must send the global PIN to get the list of all phones.  A phone that sends a phone section's `group_pin` instead gets the list of phone sections with that `group_pin`.  A request that matches no phone section is refused and recorded as an authentication incident. |
| `globalpin` | Text | None | The global PIN used by `config_auth` and `userlist_auth`. |
| `registration_address` | Host name or address | None | The address where phones reach this server.  When set, every phone gets a default network, named after the phone section and with the alias `Default Network`.  The default network matches every address (`0.0.0.0/0`) and comes after the phone's `network` options. |
| `registration_port` | Port | `5060` | The SIP port for the default network. |
| `firmware_url_prefix` | URL | None | The `file_url_prefix` for the default network: the URL where a web server serves the files in `file_directory`.  A trailing `/` is added if missing.  This option also creates the default network when `registration_address` is not set. |
| `file_directory` | Directory | `digium_phones` in Asterisk's `astvarlibdir`, e.g., `/var/lib/asterisk/digium_phones` | The directory for files named in the configuration: contact lists, logos, wallpapers, palette files, display rules, firmware images, and `config_file` files.  Phones request contact lists from this directory through the module. |
| `firmware_package_directory` | Directory | None | A directory that contains a firmware package: firmware images, and a `digium_phones_firmware.conf` file that lists the images.  See [Firmware package](#firmware-package). |
| `service_discovery_enabled` | On/off | `yes` | Whether the module announces the server over mDNS as the service `_digiumproxy._udp`.  Phones on the same network segment can then find the server without being given an address. |
| `service_name` | UTF-8 text, at most 63 bytes | `Digium Phone Config Server` | The name of the announced service, which phones show in the list of servers.  A name that is too long or is not valid UTF-8 is not announced. |
| `mdns_address` | Host name or address | The value of `registration_address` | The server address in the announcement.  Without an address, the announcement has no SIP URL. |
| `mdns_port` | Port | The value of `registration_port`, or else `5060` | The port in the announcement. |
| `mdns_transport` | Text, e.g., `tcp` | None | Appended to the announced SIP URL as `;transport=<value>`. |
| `server_uuid` | Text | None | An identifier for this server, sent to the phone with every internal line account and app account. |
| `pjsip_message_context` | Dialplan context name | `dpma_pjsip_message_context` | The dialplan context that the module creates to route messages from phones.  Change the name only if another context already uses the default name. |
| `transaction_initial_threads` | Number, at least 1 | `1` | The initial number of threads that handle phone requests.  A value above `transaction_max_threads` is lowered to that value. |
| `transaction_max_threads` | Number, at least 1 | `4` | The maximum number of threads that handle phone requests. |
| `message_delay` | Milliseconds | `5` | The delay between SIP messages when a reply spans more than one message. |
| `appserver_certificate` | Path to a PEM file | A certificate built into the module | The server certificate that the module sends to phones in the handshake.  Open DPMA only. |
| `phone_ca_file` | Path to a PEM file with one or more CA certificates | The CA certificates built into the module | The certificate authorities used to verify the certificate that a phone presents in the handshake.  Open DPMA only. |

The built-in certificates are the same ones that the original module uses, which D-series phones are built to trust.  If a file named by `appserver_certificate` or `phone_ca_file` cannot be read at startup, the module does not load.  If the file cannot be read on a reload, the previous certificates stay in use.

### `config_auth` { #config_auth }

| Value | The phone presents | The server checks |
| --- | --- | --- |
| `disabled` | The user chosen from the user list. | Nothing. |
| `mac` | The phone's MAC address.  The phone shows no user list. | A phone section has that `mac`. |
| `pin` | The chosen user and a PIN. | The PIN matches the phone section's `pin`. |
| `globalpin` | The chosen user and a PIN. | The PIN matches `globalpin` or the phone section's `group_pin`. |
| `mac_pin` | The MAC address, the chosen user, and a PIN. | The phone section has that `mac`, and the PIN matches the section's `pin`. |
| `mac_globalpin` | The MAC address, the chosen user, and a PIN. | The phone section has that `mac`, and the PIN matches `globalpin` or the section's `group_pin`. |

!!! warning "A misspelled value leaves authentication disabled"

    The module does not apply a `config_auth` or `userlist_auth` value that is not listed above.  The default, `disabled`, stays in effect, and the module logs a warning that names the value and the line: `Invalid value '<value>' for config_auth at line <n> of res_digium_phone.conf is ignored`.  After every change, check the `Config Authentication` and `Userlist Authentication` lines in the output of `digium_phones show settings`.  See [Authentication](administration.md#authentication) for help choosing a value.

## `type=phone` { #phone }

A phone section describes one phone.  When `config_auth` is a value without `mac`, a phone section describes a user instead: any phone can take the section's configuration when the user picks the section from the phone's user list.

Options marked "Repeatable" can appear more than once, and each occurrence adds to a list.

A phone section also accepts the `dphone_*` options, described in [Open DPhone options](open-dphone-options.md).

### Identity and access

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `mac` | MAC address, with or without colons | None | The phone that the section is for.  Used by the `config_auth` values that include `mac`. |
| `pin` | Text, or `voicemail` | None | The section's PIN, used by the `config_auth` values that include `pin`.  The PIN is also the password for the phone's app account, which the phone's apps send with every request.  With `voicemail`, the PIN is the voicemail password for the primary line's mailbox in `voicemail.conf`, and changing that password reconfigures the phone. |
| `group_pin` | Text | None | A PIN shared by a group of phone sections, e.g., one tenant's sections.  The server accepts the PIN in place of `globalpin` for this section.  See [`config_auth`](#config_auth) and [`userlist_auth`](#general). |
| `full_name` | Text | The section's name | The name shown in the phones' user list and in the module's contact list of internal extensions, and recorded as the author of app data. |
| `web_ui_enabled` | On/off | `no` | Enables the phone's web interface. |
| `login_password` | Text | Not sent | The password for the phone's web interface. |
| `lock_preferences` | On/off | `no` | Prevents the user from changing the phone's preferences.  The display, sound, answering, localization, and network selection menus are hidden, and the section's values override values set on the phone. |
| `logout_enabled` | On/off | `no` | Adds the Log Out app to the phone. |
| `config_file` | File name in `file_directory`{ .nowrap } | None | A complete phone configuration document to send in place of the one that the module generates.  With this option, no other option in the section affects the configuration. |

### Lines and linked sections

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `line` | Name of a PJSIP endpoint | None | Repeatable.  A line on this server.  The section's first `line` or `external_line` is the primary line, which is used for the phone's voicemail, parking, and status, and supplies the extension in the user list.  A phone has at most 12 lines; extra lines are dropped.  A [`type=line`](#line) section with the same name adds settings. |
| `external_line` | Name of a `type=external_line` section | None | Repeatable.  A line on another SIP server. |
| `network` | Name of a `type=network` section | None | Repeatable.  A network that the phone can be on.  The phone gets each account once per network, and selects a network by `cidr`. |
| `firmware` | Name of a `type=firmware` section | None | Repeatable.  A firmware image for the phone to run.  See [Firmware](administration.md#firmware). |
| `application` | Name of a `type=application` section | None | Repeatable.  Settings for one of the phone's apps. |
| `alert` | Name of a `type=alert` section | None | Repeatable.  An alert for the phone. |
| `multicastpage` | Name of a `type=multicastpage` section | None | Repeatable.  A multicast page that the phone listens for. |
| `ringtone` | Name of a `type=ringtone` section, or a built-in ringtone | None | Repeatable.  A ringtone file to load onto the phone.  Naming a built-in ringtone instead makes that ringtone the active one, unless `active_ringtone` appears earlier.  The built-in ringtones are `Alarm`, `Chimes`, `Digium`, `GuitarStrum`, `Jingle`, `Office`, `Office2`, `RotaryPhone`, `SteelDrum`, `Techno`, `Theme`, `Tweedle`, `Twinkle`, and `Vibe`. |
| `active_ringtone` | Name of a ringtone | `Digium` | The ringtone for normal calls. |
| `codecs` | Comma-separated codec names | Not sent | The phone's codecs, in order of preference.  A `!` before a name disables that codec.  The names are `PCMU`, `PCMA`, `G722`, `G7221`, `G726-32`, `opus`, `G729`, `iLBC`, `L16`, and `L16-256`.  An unknown name ends the list at that point. |

### Contacts and busy lamp fields

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `contact` | File name or URL | An empty list | Repeatable.  A contact list.  A file in `file_directory` is fetched from the module, any other name is fetched from the network's `file_url_prefix`, and a full URL is used as written. |
| `blf_contact_group` | Name of a contact group | Not sent | The contact group that the phone uses for busy lamp field keys. |
| `contacts_display_rules` | File name in `file_directory`{ .nowrap } | None | An XML file of display rules, which the module includes in the settings for the phone's Contacts app. |
| `contacts_max_subscriptions` | Number | Not sent | The maximum number of contacts whose state the phone subscribes to. |
| `blf_items` | File name or URL | None | A file that assigns contacts to line keys, fetched from the network's `file_url_prefix`. |
| `blf_unused_linekeys` | On/off | `no` | Shows contacts on unused line keys. |
| `blf_page_return_timeout` | Number | `0` | The time after which the line keys return to the first page. |
| `name_format` | Text | Not sent | The order of the parts of a contact's name. |
| `use_local_storage` | On/off | `no` | Lets the Contacts app store contacts on the phone. |

### Call features

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `send_to_vm` | On/off | `yes` | Enables the softkey that sends an incoming call to voicemail, and transfers to voicemail. |
| `can_forward_calls` | On/off | `yes` | Enables call forwarding. |
| `record_own_calls` | On/off | `no` | Enables the softkey that records the call. |
| `show_call_log` | On/off | `yes` | Enables the call log. |
| `show_call_parking` | On/off | `yes` | Enables the Parking app. |
| `parking_exten` | Extension | None | The extension that a call is transferred to for parking.  Setting a value adds the Park softkey. |
| `parking_transfer_type` | `blind` or `attended` | Not sent | How the phone transfers a call to `parking_exten`. |
| `accept_local_calls` | Text | Not sent | The hosts that the phone accepts calls from. |
| `call_waiting_tone` | On/off | `yes` | Plays a tone when a second call arrives during a call. |
| `headset_answer` | On/off | `no` | Answers calls on the headset. |
| `ring_headset_only` | On/off | `no` | Rings on the headset only. |
| `reset_call_volume` | On/off | `no` | Resets the call volume to the default after each call. |
| `ehs` | Text | Not sent | The type of electronic hook switch (EHS) headset adapter. |
| `enable_check_sync` | On/off | `no` | Makes the phone act on SIP `check-sync` notifications. |
| `transport_tls_allowed` | On/off | Not sent | Allows the phone to use TLS for SIP. |
| `allow_insecure_ssl` | On/off | `no` | Makes the phone accept TLS certificates that the phone cannot verify. |
| `handshake_timeout` | Seconds | `946080000` (30 years) | The lifetime of the phone's handshake with the server. |

### Display and sound

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `timezone` | Time zone name, e.g., `America/Denver`{ .nowrap } | Not sent | The phone's time zone. |
| `ntp_resync` | Seconds | Not sent | How often the phone queries the NTP server. |
| `active_locale` | Locale name, e.g., `en_US` | Not sent | The phone's language. |
| `brightness` | Number | Not sent | The display brightness. |
| `contrast` | Number | Not sent | The display contrast. |
| `dim_backlight` | On/off | `yes` | Dims the backlight when the phone is idle.  A `backlight_timeout` of `0` turns dimming off. |
| `backlight_timeout` | Seconds | Not sent | The idle time before the backlight dims. |
| `backlight_dim_level` | Number | Not sent | The brightness of the dimmed backlight. |
| `default_fontsize` | Number | Not sent | The display font size. |
| `small_clock` | On/off | `no` | Shows the small clock on the idle screen. |
| `idle_company_text` | Text | Not sent | Text to show on the idle screen. |
| `display_mc_notification` | On/off | `yes` | Shows a notification for missed calls. |
| `ringer_volume` | Number | Not sent | The ringer volume. |
| `speaker_volume`, `handset_volume`, `headset_volume` | Number | Not sent | The call volume for each output. |
| `handset_sidetone_db` | Decibels | `-25` | The handset sidetone level.  `0` keeps the phone's value. |
| `headset_sidetone_db` | Decibels | `-15` | The headset sidetone level.  `0` keeps the phone's value. |
| `<model>_logo_file` | File name or URL | None | A logo image for one phone model, e.g., `d70_logo_file`.  A file in `file_directory` is fetched from the module, and any other name is fetched from the network's `file_url_prefix`. |
| `wallpaper_file` | File name or URL | None | A wallpaper image, located the same way as a logo file. |
| `expansion_enabled` | On/off | `no` | Enables an expansion module. |
| `expansion_page_indicators_enabled` | On/off | `no` | Shows page indicators on the expansion module. |

With `lock_preferences=no`, the display and sound values are only initial values, and a value that the user sets on the phone takes precedence.

### Network and logging

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `ntp_server`, `syslog_server`, `syslog_port`, `syslog_level` | Same as in a [network section](#network) | None | Sets the option on the phone's default network, which `registration_address` in `[general]` creates.  If `[general]` has no `registration_address`, the option creates the default network without a server address. |
| `lan_port_mode`, `pc_port_mode` | Text | Not sent | The speed and duplex of the phone's LAN port and PC port. |
| `8021x_method` | Text | Empty | The 802.1X authentication method. |
| `8021x_identity`, `8021x_anonymous_identity`, `8021x_password` | Text | Empty | The 802.1X credentials. |
| `8021x_client_cert_url`, `8021x_root_cert_url` | URL | Empty | Where the phone downloads the 802.1X client certificate and root certificate. |
| `8021x_client_cert_value`, `8021x_root_cert_value` | File name | The file name at the end of the URL | The file name under which the phone stores each certificate. |
| `8021x_passthrough` | Text | Empty | Whether the phone passes 802.1X frames through to the PC port. |
| `8021x_eapol_on_disconnect` | Text | `0` | Whether the phone sends an EAPOL logoff when the device on the PC port disconnects. |
| `8021x_debug` | Text | Not sent | 802.1X logging on the phone. |
| `openvpn_enable` | On/off | `no` | Connects the phone to an OpenVPN server. |
| `openvpn_config_file_url`, `openvpn_root_cert_url`, `openvpn_client_cert_url`, `openvpn_client_key_url` | URL | Empty | Where the phone downloads the OpenVPN configuration, root certificate, client certificate, and client key. |
| `openvpn_config_file_value`, `openvpn_root_cert_value`, `openvpn_client_cert_value`, `openvpn_client_key_value` | File name | The file name at the end of the URL | The file name under which the phone stores each file. |
| `openvpn_logging` | Text | Not sent | OpenVPN logging on the phone. |

The module passes values of type "Text" to the phone unchanged, and the phone's firmware determines which values are meaningful.

## `type=line` and `type=external_line` { #line }

A line that a phone references with `line` is a PJSIP endpoint on this server.  The module reads the following settings from `pjsip.conf` and tracks later changes there:

| Line setting | Source in `pjsip.conf` |
| --- | --- |
| Context and caller ID | The endpoint's `context` and `callerid`. |
| User name and password | The `username` and `password` of the endpoint's first `auth` section. |
| Mailbox | The endpoint's `mailboxes`, or else the `mailboxes` of the endpoint's first AOR that has the option.  Only the first mailbox is used. |

As with the original module, a `users.conf` user with `hasvoicemail` also gives a mailbox to the line with the same name as the user.

A `type=line` section named after the endpoint is optional, and sets what PJSIP does not provide.  Options in the section override the PJSIP values, even when the endpoint changes later.

A `type=external_line` section is required for each external line, and holds all of the line's settings.

Both types accept the same options:

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `exten` (or `extension`) | Extension | The line's name | The number that others dial to reach the line, shown in the user list and in the module's contact list of internal extensions. |
| `line_label` | Text | The line's name | The line key's label. |
| `callerid` | Caller ID | From PJSIP | The caller ID that the phone shows for the line. |
| `first_name`, `last_name`, `title`, `location` | Text | None | Details about the line's user, shown in the phone's account and in the module's contact list of internal extensions. |
| `digit_map` | Digit map | None | The dial plan that the phone uses to decide when a dialed number is complete. |
| `mailbox` | `mailbox@context`{ .nowrap } | From PJSIP or `users.conf` | The line's voicemail box.  The context defaults to `default`.  If the value is a list, only the first mailbox is used. |
| `voicemail_uri` | SIP URI or number | None | A number that the phone calls for voicemail, in place of the phone's Voicemail app. |
| `context` | Dialplan context | `default`, or from PJSIP | The context in which the module creates the line's hint when `subscribecontext` is not set. |
| `subscribecontext` | Dialplan context | The value of `context` | The context in which the module creates the hint `auto_hint_<line>`, which other phones subscribe to for the line's state. |
| `transport` | `udp`, `tcp`, or `tls` | The network's `transport` | The SIP transport to the primary server. |
| `media_encryption` | Text, e.g., `sdes` | Not sent | Media encryption with the primary server. |
| `secondary_server_transport` | `udp`, `tcp`, or `tls` | The network's `alternate_transport` | The SIP transport to the alternate server. |
| `secondary_media_encryption` | Text | Not sent | Media encryption with the alternate server. |
| `reregistration_timeout` | Seconds | Not sent | The lifetime of the phone's registration. |
| `registration_retry_interval` | Seconds | Not sent | The delay before the phone retries a failed registration. |
| `registration_max_retries` | Number | Not sent | The maximum number of times that the phone retries a failed registration. |
| `register` | On/off | `yes` | Whether the phone registers the line. |
| `host` | `dynamic`, or a host | `dynamic` | Any value other than `dynamic` prevents the phone from registering the line. |
| `outboundproxy_address` | Host name or address | None | An outbound proxy for the line. |
| `outboundproxy_port` | Port | `5060` | The outbound proxy's port. |
| `plar_number` | Number | None | A number that the phone dials as soon as the line goes off hook. |
| `secret` | Text | From PJSIP | The SIP password.  Set this option for an external line. |
| `authname` | Text | From PJSIP.  For an external line, `userid`, or else the line's name. | The SIP authentication user name. |
| `userid` | Text | The line's name | External lines only: the SIP user name. |
| `server_address` | Host name or address | None | External lines only: the SIP server. |
| `server_port` | Port | `5060` | External lines only: the SIP server's port. |
| `secondary_server_address` | Host name or address | None | External lines only: an alternate SIP server. |
| `secondary_server_port` | Port | `5060` | External lines only: the alternate server's port. |
| `tech` | Text | `PJSIP` for a line with a PJSIP endpoint | The channel technology in the line's hint.  The module sets the value; do not set this option in the file. |
| `pjsip_auth` | Name of an `auth` section | The endpoint's first `auth` section | The `auth` section whose changes the line tracks.  The module sets the value; do not set this option in the file. |

For an internal line, the server address, port, and alternate server come from the phone's [networks](#network), not from the line.

## `type=network` { #network }

A network section specifies the server addresses that a phone uses while the phone's address is within `cidr`.  A phone that moves between networks, e.g., between an office and a VPN, needs one section for each.

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `cidr` | Network in CIDR notation, e.g., `10.10.0.0/16` | None | The addresses for which the phone uses this network. |
| `alias` | Text | None | The network's name on the phone. |
| `registration_address` | Host name or address | None | This server's address as seen from the network. |
| `registration_port` | Port | `5061` with TLS, otherwise `5060` | This server's SIP port.  The default depends on the transport that the module sends for the line: the line's `transport`, or else the network's. |
| `transport` | `udp`, `tcp`, or `tls` | None | The SIP transport for lines without `transport`. |
| `alternate_registration_address` | Host name or address | None | A second server that the phone registers with when the first does not answer. |
| `alternate_registration_port` | Port | None | The second server's SIP port. |
| `alternate_transport` | `udp`, `tcp`, or `tls` | None | The SIP transport to the second server. |
| `file_url_prefix` | URL | None | The URL where a web server serves the files in `file_directory` to this network.  A trailing `/` is added if missing.  Ringtones, custom apps, `blf_items`, and firmware are offered only when this option is set. |
| `public_firmware_url_prefix` | URL | `http://dphone.dl.digium.com/firmware/asterisk/` | The URL where phones find the images of firmware sections with `public_firmware=yes`. |
| `ntp_server` | Host name or address | Not sent | The phone's NTP server. |
| `syslog_server` | Host name or address | None | The phone's syslog server.  The phone logs to the server only when `syslog_level` is also set. |
| `syslog_port` | Port | Not sent | The syslog server's port. |
| `syslog_level` | Text, e.g., `error` | Not sent | The lowest level of messages that the phone logs. |
| `network_vlan_discovery_mode` | Text, e.g., `MANUAL` | Not sent | How the phone finds the VLAN.  The three VLAN options below are sent only when this option is set. |
| `network_vlan_id` | VLAN number | Not sent | The phone's VLAN.  Sent when greater than `0`. |
| `sip_qos` | `0` to `7` | `0` | The 802.1p priority of SIP packets on the VLAN. |
| `rtp_qos` | `0` to `7` | `0` | The 802.1p priority of RTP packets on the VLAN. |
| `network_vlan_qos` | `0` to `7` | — | A deprecated name for `rtp_qos`; the module logs a deprecation message.  The option sets the RTP priority only if no `rtp_qos` appears earlier in the section. |
| `pc_vlan_id` | VLAN number | `0` | The VLAN for the phone's PC port. |
| `pc_qos` | Number | `0` | The 802.1p priority of the PC port's packets. |
| `sip_dscp` | Number | `0` | The DSCP value of SIP packets. |
| `rtp_dscp` | Number | `0` | The DSCP value of RTP packets. |
| `udp_ka_interval` | Seconds | `0` | The interval between the keep-alive packets that the phone sends over UDP. |

## `type=firmware` { #firmware }

A firmware section describes one firmware image.  A phone section lists images with `firmware`, and each phone acts on the first entry for the phone's model.  See [Firmware](administration.md#firmware) for how phones decide whether to install an image.

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `model` | Model name, e.g., `D65` | None | The phone model that the image is for.  The value is converted to uppercase. |
| `version` | Version text, e.g., `3_0_0` | None | The image's version.  Stock firmware installs the image when this text differs from the version that the phone is running. |
| `file` | File name or URL | None | The image.  A file name is fetched from the network's `file_url_prefix`, and a full URL is used as written.  When the file is in `file_directory`, the entry also includes the image's checksum. |
| `public_firmware` | On/off | `no` | Whether the phone may also fetch the image from the network's `public_firmware_url_prefix`, as `<version>/<version>_<model>_firmware.eff`. |

An entry is sent only for a network that has a `file_url_prefix`, and only when the section has both `version` and `file`.

### Firmware package

A phone section with no `firmware` option is offered the images in the firmware package instead.  The package is the directory named by `firmware_package_directory` in `[general]`, and contains the images and a `digium_phones_firmware.conf` file.  In that file, each section is named after a phone model:

```ini title="digium_phones_firmware.conf"
[D65]
version=3_0_0
file=3_0_0_D65_firmware.eff
```

| Option | Value | Description |
| --- | --- | --- |
| `version` | Version text | Same as `version` in a firmware section. |
| `file` | File name or URL | Same as `file` in a firmware section.  The checksum is computed from the file in the package directory. |

## `type=application` { #application }

An application section holds settings for one phone app.  The `application` option selects the app, and the other options depend on that value.

### `application=queue`

A queue section lets the phone's Queues app show one `app_queue` queue, and log the user in and out.

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `queue` | Queue name | None | The queue. |
| `member` | On/off | `yes` | Whether the user is a member of the queue.  With `no`, the user only monitors the queue. |
| `location` | Interface, e.g., `PJSIP/101` | None | The member interface used to add the user to the queue and to find the user among the members. |
| `membername` | Text | None | The member name used to add the user. |
| `permission` (or `permissions`) | `status`, `overview`, or `details` | `status` | How much of the queue the phone shows.  `status` shows only the user's status, `overview` adds the number of members, and `details` adds the logged-in members and the waiting callers. |
| `login_exten` | `extension@context`{ .nowrap } | None | An extension that the module calls from the phone to log the user in, in place of adding the member directly. |
| `logout_exten` | `extension@context`{ .nowrap } | None | An extension that the module calls from the phone to log the user out, in place of removing the member directly. |

### `application=status`

A status section adds one presence state to the list in the phone's Status app.  A phone with at least one status section offers only the states from those sections.

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `status` | `available`, `unavailable`, `away`, `xa`, `chat`, or `dnd` | None | The presence state.  Required. |
| `substatus` | Text | None | Repeatable.  A sub-state that the user can choose along with the state, e.g., `Out to lunch`. |
| `send486` | On/off | `yes` for `dnd` and `xa`, otherwise `no` | Whether the phone rejects calls as busy while the user is in the state. |

### `application=parking`

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `parkinglot` | Comma-separated parking lot names | None | Repeatable.  The parking lots whose calls the phone's Parking app shows.  A phone without a parking section shows the calls in every lot. |

### `application=voicemail`

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `translation` | Name of a `type=translation` section | None | The display names for the voicemail folders. |
| `require_password` | On/off | `no` | Whether the phone's Voicemail app asks for the voicemail password. |

### `application=custom`

A custom section loads an app package onto phones running stock firmware.  Phones running Open DPhone skip the package, because Open DPhone apps use a different format, described in the [App SDK](../app-sdk/index.md).

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `name` | Text | None | The app's identifier on the phone. |
| `filename` | File name or URL | None | The app package, fetched from the network's `file_url_prefix`. |
| `md5` (or `md5sum`) | MD5 digest | Not sent | The package's digest. |
| `autostart` | On/off | `no` | Starts the app when the phone starts. |
| `idlescreen_softkey` | On/off | `no` | Adds a softkey for the app to the idle screen. |
| Any other option | Text | — | Passed to the app as a setting with the same name. |

## `type=ringtone` { #ringtone }

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `filename` | File name or URL | None | The sound file, fetched from the network's `file_url_prefix`. |
| `alias` | Text | The section's name | The ringtone's name on the phone. |

## `type=alert` { #alert }

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `alert_info` | Text | None | The `Alert-Info` header value that selects the alert.  Required. |
| `ring_type` | `normal`, `ring-answer`, `visual`, or `answer` | `normal` | How the phone alerts the user. |
| `ringtone` | Name of a ringtone | None | The alert's ringtone. |

Every phone also gets four built-in alerts, for the `Alert-Info` values `normal`, `ring-answer`, `visual`, and `intercom`.  The corresponding ring types are `normal`, `ring-answer`, `visual`, and `answer`.

## `type=multicastpage` { #multicastpage }

| Option | Value | Default | Description |
| --- | --- | --- | --- |
| `address` | Multicast address | None | The address that the phone listens on. |
| `port` | Port | None | The port that the phone listens on. |
| `alias` | Text | The section's name | The page's name on the phone. |
| `priority` | Number | None | The page's priority relative to other pages. |
| `interrupt` | `0` or `1` | `0` | Whether a page interrupts a call. |

## `type=translation` { #translation }

In a translation section, each option is the name of a voicemail folder, and the value is the name that the phone displays for the folder.

```ini
[de_DE]
type=translation
INBOX=Eingang
Work=Arbeit
```

