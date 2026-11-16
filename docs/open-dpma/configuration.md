# Configuration

Open DPMA is configured in `res_digium_phone.conf`, in Asterisk's configuration directory: the file that the original `res_digium_phone` reads.  This page is the reference of every section and option that Open DPMA accepts, for the administrator who writes or reviews that file.  The options that only phones with Open DPhone honor are on a separate page, [Options for Open DPhone](open-dphone-options.md).

## How the module reads the file

The file has a `[general]` section, and any number of other sections.  Each other section has a `type` option that says what the section describes; the section's name is the name of the thing described.

| `type` | Describes | Referred to by |
| --- | --- | --- |
| [`phone`](#phone) | One phone, or one user who can log in to a phone. | — |
| [`line`](#line) | A line on this Asterisk server.  The section's name is the name of a PJSIP endpoint. | `line` of a phone |
| [`external_line`](#line) | A line on another SIP server. | `external_line` of a phone |
| [`network`](#network) | A network that a phone can be on, and the server addresses to use there. | `network` of a phone |
| [`firmware`](#firmware) | A firmware image for one phone model. | `firmware` of a phone |
| [`application`](#application) | Settings of a phone app: queues, status, parking, voicemail, or a custom app. | `application` of a phone |
| [`ringtone`](#ringtone) | A ringtone file. | `ringtone` of a phone, `ringtone` of an alert |
| [`alert`](#alert) | How the phone rings for calls with a given `Alert-Info` header. | `alert` of a phone |
| [`multicastpage`](#multicastpage) | A multicast paging address that a phone listens to. | `multicastpage` of a phone |
| [`translation`](#translation) | Names for voicemail folders. | `translation` of a voicemail application |

Rules that apply to the whole file:

- Option names and the values of `type` are matched without regard to case.
- Asterisk's templates work, e.g., `[sales](!)` and `[phone101](sales)`.
- An on/off option takes any value that Asterisk reads as true (`yes`, `true`, `on`, `1`) or false.
- An option that the module does not know, or a value that the module cannot read, is reported in the log at the level `DPMA` and skipped; the rest of the section still applies.  A section without `type`, or with an unknown `type`, is reported and skipped.
- Most text values are put into the phone's configuration, an XML document, as written.  A value that contains `"`, `<`, `>`, or `&` damages the document.  The names and credentials of lines are the exception and are escaped.

The module reads the file when Asterisk starts and at `module reload res_open_dpma.so`.  A reload does not send the new configuration to phones that are running, except to a phone whose lines changed; see [Pushing a changed configuration](administration.md#pushing-a-changed-configuration).

## A minimal configuration

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

The phone section names the line `101`, which is the PJSIP endpoint `101` of `pjsip.conf`; the module takes the line's SIP credentials from that endpoint.  `registration_address` is the address at which phones reach the server, and gives every phone a network.  A phone without a network gets a configuration without SIP accounts.

## `[general]` { #general }

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `config_auth` | `disabled`, `mac`, `pin`, `globalpin`, `mac_pin`, or `mac_globalpin` | `disabled` | What a phone has to prove before the server hands out a phone section's configuration.  See [the table below](#config_auth). |
| `userlist_auth` | `disabled` or `globalpin` | `disabled` | With `globalpin`, a phone has to send the global PIN to get the list of all phones.  A phone that sends a phone section's `group_pin` instead gets the list of the phone sections that have that `group_pin`.  A request that matches no phone section is refused and recorded as an authentication incident. |
| `globalpin` | Text | None | The global PIN that `config_auth` and `userlist_auth` refer to. |
| `registration_address` | Host name or address | None | The address at which phones reach this server.  When set, every phone gets a network named after the phone section, with the alias `Default Network`, that matches every address (`0.0.0.0/0`) and comes after the phone's `network` options. |
| `registration_port` | Port | `5060` | The SIP port of that network. |
| `firmware_url_prefix` | URL | None | The `file_url_prefix` of that network: the URL under which a web server serves the files of `file_directory`.  A `/` is added at the end when missing.  The option alone, without `registration_address`, also creates the network. |
| `file_directory` | Directory | `digium_phones` in Asterisk's `astvarlibdir`, e.g., `/var/lib/asterisk/digium_phones` | The directory of the files that the configuration names: contact lists, logos, wallpapers, palette files, display rules, firmware images, and `config_file` files.  Phones request contact lists from this directory through the module. |
| `firmware_package_directory` | Directory | None | A directory with a firmware package: the firmware images and the file `digium_phones_firmware.conf` that lists them.  See [Firmware package](#firmware-package). |
| `service_discovery_enabled` | On/off | `yes` | Whether the module announces the server by mDNS, as the service `_digiumproxy._udp`, so that phones on the same network segment find the server without being told an address. |
| `service_name` | Text of at most 63 bytes, valid UTF-8 | `Digium Phone Config Server` | The name of the announced service, which a phone shows in the list of servers.  A longer name, or a name that is not UTF-8, is not announced. |
| `mdns_address` | Host name or address | The value of `registration_address` | The server address in the announcement.  Without an address, the announcement carries no SIP URL. |
| `mdns_port` | Port | The value of `registration_port`, else `5060` | The port in the announcement. |
| `mdns_transport` | Text, e.g., `tcp` | None | Added to the announced SIP URL as `;transport=<value>`. |
| `server_uuid` | Text | None | An identifier of this server, sent to the phone with every account of an internal line and of an app. |
| `pjsip_message_context` | Dialplan context name | `dpma_pjsip_message_context` | The dialplan context that the module creates to route the phones' messages.  Change the name only when another context has the default name. |
| `transaction_initial_threads` | Number, at least 1 | `1` | The number of threads that handle phone requests at the start.  A value above `transaction_max_threads` is lowered to that value. |
| `transaction_max_threads` | Number, at least 1 | `4` | The most threads that handle phone requests at the same time. |
| `message_delay` | Milliseconds | `5` | The pause between the SIP messages of one reply that is longer than one message. |
| `appserver_certificate` | Path of a PEM file | A certificate built into the module | The server certificate that the module sends to a phone in the handshake.  Only Open DPMA has this option. |
| `phone_ca_file` | Path of a PEM file with one or more CA certificates | The CA certificates built into the module | The certificate authorities against which the module verifies the certificate that a phone presents in the handshake.  Only Open DPMA has this option. |

The certificates that are built in are those of the original module, which the phones of the D-series are made for.  When a file named by `appserver_certificate` or `phone_ca_file` cannot be read at start, the module does not load; at a reload, the previous certificates stay in use.

### `config_auth` { #config_auth }

| Value | The phone presents | The server checks |
| --- | --- | --- |
| `disabled` | The user chosen from the user list. | Nothing. |
| `mac` | The phone's MAC address.  The phone shows no user list. | A phone section has that `mac`. |
| `pin` | The chosen user and a PIN. | The PIN is the phone section's `pin`. |
| `globalpin` | The chosen user and a PIN. | The PIN is `globalpin`, or the phone section's `group_pin`. |
| `mac_pin` | The MAC address, the chosen user, and a PIN. | The phone section has that `mac`, and the PIN is the section's `pin`. |
| `mac_globalpin` | The MAC address, the chosen user, and a PIN. | The phone section has that `mac`, and the PIN is `globalpin` or the section's `group_pin`. |

!!! warning "A misspelled value leaves authentication disabled"

    The module ignores a `config_auth` or `userlist_auth` value that is not in the lists above, without a log message.  After every change, check the lines `Config Authentication` and `Userlist Authentication` of `digium_phones show settings`.  [Authentication](administration.md#authentication) says which value to choose.

## `type=phone` { #phone }

A phone section describes one phone.  With a `config_auth` that has no `mac`, a phone section is rather a user: any phone can take the section's configuration by choosing the section in the user list.

Options that can be repeated are marked "repeatable"; each occurrence adds to a list.

### Identity and access

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `mac` | MAC address, with or without colons | None | The phone that the section is for.  Used by the `config_auth` values with `mac`. |
| `pin` | Text, or `voicemail` | None | The section's PIN, for the `config_auth` values with `pin`.  The PIN is also the password of the phone's app account, which the phone's apps send with every request.  With `voicemail`, the PIN is the voicemail password of the primary line's mailbox in `voicemail.conf`, and a change of that password reconfigures the phone. |
| `group_pin` | Text | None | A PIN that stands in for `globalpin` for this section, so that a group of sections can have a PIN of the group's.  See `config_auth` and `userlist_auth`. |
| `full_name` | Text | The section's name | The name in the user list that phones show, in the module's contact list of internal extensions, and as the author of app data. |
| `web_ui_enabled` | On/off | `no` | Whether the phone's web interface can be used. |
| `login_password` | Text | Not sent | The password of the phone's web interface. |
| `lock_preferences` | On/off | `no` | Whether the user is kept from changing the phone's preferences.  The display, sound, answering, localization, and network selection menus are hidden, and the values of this section override the values set on the phone. |
| `logout_enabled` | On/off | `no` | Whether the phone has the Log Out app. |
| `config_file` | File name in `file_directory` | None | A complete phone configuration document to send in place of the one that the module builds.  With this option, no other option of the section shapes the configuration. |

### Lines, networks, and other sections

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `line` | Name of a PJSIP endpoint | None | Repeatable.  A line on this server.  The first `line` or `external_line` of the section is the primary line: the line of the phone's voicemail, parking, and status, and the extension in the user list.  A phone has at most 12 lines; more are dropped.  A [`type=line`](#line) section of the same name adds settings. |
| `external_line` | Name of a `type=external_line` section | None | Repeatable.  A line on another SIP server. |
| `network` | Name of a `type=network` section | None | Repeatable.  A network that the phone can be on.  The phone gets each account once for every network, and chooses among the networks by `cidr`. |
| `firmware` | Name of a `type=firmware` section | None | Repeatable.  A firmware image that the phone is to run.  See [Firmware](administration.md#firmware). |
| `application` | Name of a `type=application` section | None | Repeatable.  Settings of one of the phone's apps. |
| `alert` | Name of a `type=alert` section | None | Repeatable. |
| `multicastpage` | Name of a `type=multicastpage` section | None | Repeatable. |
| `ringtone` | Name of a `type=ringtone` section, or a built-in ringtone | None | Repeatable.  A ringtone file to load onto the phone.  The name of a built-in ringtone instead selects that ringtone as the active one, unless `active_ringtone` came before.  The built-in names are `Alarm`, `Chimes`, `Digium`, `GuitarStrum`, `Jingle`, `Office`, `Office2`, `RotaryPhone`, `SteelDrum`, `Techno`, `Theme`, `Tweedle`, `Twinkle`, and `Vibe`. |
| `active_ringtone` | Name of a ringtone | `Digium` | The ringtone of normal calls. |
| `codecs` | Comma-separated codec names | Not sent | The codecs of the phone, in order of preference.  A `!` before a name disables that codec.  The names are `PCMU`, `PCMA`, `G722`, `G7221`, `G726-32`, `opus`, `G729`, `iLBC`, `L16`, and `L16-256`.  An unknown name ends the list at that point. |

### Contacts and busy lamp fields

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `contact` | File name or URL | An empty list | Repeatable.  A contact list.  A file that is in `file_directory` is fetched from the module; any other name is fetched from the network's `file_url_prefix`, and a full URL is used as written. |
| `blf_contact_group` | Name of a contact group | Not sent | The contact group that the phone uses for busy lamp field keys. |
| `contacts_display_rules` | File name in `file_directory` | None | An XML file of display rules that the module puts into the settings of the phone's Contacts app. |
| `contacts_max_subscriptions` | Number | Not sent | The most contacts whose state the phone subscribes to. |
| `blf_items` | File name or URL | None | A file that assigns contacts to line keys, fetched from the network's `file_url_prefix`. |
| `blf_unused_linekeys` | On/off | `no` | Whether unused line keys show contacts. |
| `blf_page_return_timeout` | Number | `0` | The time after which the line keys return to the first page. |
| `name_format` | Text | Not sent | How the phone orders the parts of a contact's name. |
| `use_local_storage` | On/off | `no` | Whether the Contacts app keeps contacts on the phone. |

### Call features

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `send_to_vm` | On/off | `yes` | Whether the phone has the softkey that sends an incoming call to voicemail, and can transfer to voicemail. |
| `can_forward_calls` | On/off | `yes` | Whether the phone has call forwarding. |
| `record_own_calls` | On/off | `no` | Whether the phone has the softkey that records the call. |
| `show_call_log` | On/off | `yes` | Whether the phone has the call log. |
| `show_call_parking` | On/off | `yes` | Whether the phone has the Parking app. |
| `parking_exten` | Extension | None | The extension that a call is transferred to in order to park the call.  With a value, the phone has the Park softkey. |
| `parking_transfer_type` | `blind` or `attended` | Not sent | How the phone transfers a call to `parking_exten`. |
| `accept_local_calls` | Text | Not sent | Which hosts the phone accepts calls from. |
| `call_waiting_tone` | On/off | `yes` | Whether a second call sounds a tone during a call. |
| `headset_answer` | On/off | `no` | Whether a call is answered on the headset. |
| `ring_headset_only` | On/off | `no` | Whether the phone rings on the headset alone. |
| `reset_call_volume` | On/off | `no` | Whether the call volume returns to the default after each call. |
| `ehs` | Text | Not sent | The kind of electronic hook switch headset adapter. |
| `enable_check_sync` | On/off | `no` | Whether the phone acts on a SIP `check-sync` notification.  Once on, the option stays on until the file says `enable_check_sync=no` or Asterisk restarts; removing the line is not enough. |
| `transport_tls_allowed` | On/off | Not sent | Whether the phone may use TLS for SIP. |
| `allow_insecure_ssl` | On/off | `no` | Whether the phone accepts a TLS certificate that the phone cannot verify. |
| `handshake_timeout` | Seconds | `946080000` (30 years) | The lifetime of the phone's handshake with the server. |

### Display and sound

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `timezone` | Time zone name, e.g., `America/Denver` | Not sent | The phone's time zone. |
| `ntp_resync` | Seconds | Not sent | The interval at which the phone asks the NTP server for the time. |
| `active_locale` | Locale name, e.g., `en_US` | Not sent | The phone's language. |
| `brightness` | Number | Not sent | The brightness of the display. |
| `contrast` | Number | Not sent | The contrast of the display. |
| `dim_backlight` | On/off | `yes` | Whether the backlight dims when the phone is idle.  A `backlight_timeout` of `0` turns dimming off. |
| `backlight_timeout` | Seconds | Not sent | The idle time before the backlight dims. |
| `backlight_dim_level` | Number | Not sent | The brightness of the dimmed backlight. |
| `default_fontsize` | Number | Not sent | The font size of the display. |
| `small_clock` | On/off | `no` | Whether the idle screen has the small clock. |
| `idle_company_text` | Text | Not sent | A text for the idle screen. |
| `display_mc_notification` | On/off | `yes` | Whether the phone shows a notice for missed calls. |
| `ringer_volume` | Number | Not sent | The volume of the ringer. |
| `speaker_volume`, `handset_volume`, `headset_volume` | Number | Not sent | The call volume of each output. |
| `handset_sidetone_db` | Number of decibels | `-25` | The sidetone of the handset.  `0` leaves the phone's value. |
| `headset_sidetone_db` | Number of decibels | `-15` | The sidetone of the headset.  `0` leaves the phone's value. |
| `<model>_logo_file` | File name or URL | None | A logo picture for the phones of one model, e.g., `d70_logo_file`.  A file in `file_directory` is fetched from the module; any other name is fetched from the network's `file_url_prefix`. |
| `wallpaper_file` | File name or URL | None | A wallpaper picture, found as a logo file is. |
| `expansion_enabled` | On/off | `no` | Whether the phone uses an expansion module. |
| `expansion_page_indicators_enabled` | On/off | `no` | Whether the expansion module shows page indicators. |

With `lock_preferences=no`, the display and sound values are the phone's starting values, and a value that the user sets on the phone replaces the value of the section.

### Network and logging

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `ntp_server`, `syslog_server`, `syslog_port`, `syslog_level` | As in a [network section](#network) | None | Set the option on the phone's default network, the network that `registration_address` of `[general]` creates.  When `[general]` has no `registration_address`, the option creates that network without a server address. |
| `lan_port_mode`, `pc_port_mode` | Text | Not sent | The speed and duplex of the phone's LAN port and PC port. |
| `8021x_method` | Text | Empty | The 802.1X authentication method. |
| `8021x_identity`, `8021x_anonymous_identity`, `8021x_password` | Text | Empty | The 802.1X credentials. |
| `8021x_client_cert_url`, `8021x_root_cert_url` | URL | Empty | Where the phone fetches the 802.1X client certificate and root certificate. |
| `8021x_client_cert_value`, `8021x_root_cert_value` | File name | The file name at the end of the URL | The name under which the phone keeps each certificate. |
| `8021x_passthrough` | Text | Empty | Whether the phone passes 802.1X frames to the PC port. |
| `8021x_eapol_on_disconnect` | Text | `0` | Whether the phone sends an EAPOL logoff when the device on the PC port disconnects. |
| `8021x_debug` | Text | Not sent | 802.1X logging on the phone. |
| `openvpn_enable` | On/off | `no` | Whether the phone connects to an OpenVPN server. |
| `openvpn_config_file_url`, `openvpn_root_cert_url`, `openvpn_client_cert_url`, `openvpn_client_key_url` | URL | Empty | Where the phone fetches the OpenVPN configuration, root certificate, client certificate, and client key. |
| `openvpn_config_file_value`, `openvpn_root_cert_value`, `openvpn_client_cert_value`, `openvpn_client_key_value` | File name | The file name at the end of the URL | The name under which the phone keeps each file. |
| `openvpn_logging` | Text | Not sent | OpenVPN logging on the phone. |

The module sends the values marked "Text" to the phone unchanged; the phone's firmware defines which values have a meaning.

A phone section also takes the `dphone_*` options, described in [Options for Open DPhone](open-dphone-options.md).

## `type=line` and `type=external_line` { #line }

A line that a phone names with `line` is a PJSIP endpoint of this server.  The module takes the following from `pjsip.conf` and follows later changes there:

| Line setting | Source in `pjsip.conf` |
| --- | --- |
| Context and caller ID | `context` and `callerid` of the endpoint. |
| User name and password | `username` and `password` of the first `auth` section of the endpoint. |
| Mailbox | `mailboxes` of the endpoint, else of the endpoint's first AOR that has the option.  The first mailbox counts. |

A user of `users.conf` with `hasvoicemail` also gives the line of the user's name a mailbox, as with the original module.

A `type=line` section with the endpoint's name is optional and sets what PJSIP does not carry.  A `type=external_line` section is required for each external line, and carries all of the line's settings.  Both types take the same options:

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `exten` (or `extension`) | Extension | The line's name | The number that others dial to reach the line, shown in the user list and in the module's contact list of internal extensions. |
| `line_label` | Text | The line's name | The label of the line key. |
| `callerid` | Caller ID | From PJSIP | The caller ID that the phone shows for the line. |
| `first_name`, `last_name`, `title`, `location` | Text | None | The line's user, in the phone's account and in the module's contact list of internal extensions. |
| `digit_map` | Digit map | None | The dial plan with which the phone decides that a dialed number is complete. |
| `mailbox` | `mailbox@context` | From PJSIP or `users.conf` | The line's voicemail box.  Without a context, `default` is used; of a list, the first mailbox counts. |
| `voicemail_uri` | SIP URI or number | None | A number that the phone calls for voicemail, in place of the phone's Voicemail app. |
| `context` | Dialplan context | `default`, or from PJSIP | The context in which the module creates the line's hint when `subscribecontext` is not set. |
| `subscribecontext` | Dialplan context | The value of `context` | The context in which the module creates the hint `auto_hint_<line>`, which other phones subscribe to for the line's state. |
| `transport` | `udp`, `tcp`, or `tls` | The network's `transport` | The SIP transport to the primary server. |
| `media_encryption` | Text, e.g., `sdes` | Not sent | Media encryption with the primary server. |
| `secondary_server_transport` | `udp`, `tcp`, or `tls` | The network's `alternate_transport` | The SIP transport to the alternate server. |
| `secondary_media_encryption` | Text | Not sent | Media encryption with the alternate server. |
| `reregistration_timeout` | Seconds | Not sent | The lifetime of the phone's registration. |
| `registration_retry_interval` | Seconds | Not sent | The pause before the phone repeats a failed registration. |
| `registration_max_retries` | Number | Not sent | The most times the phone repeats a failed registration. |
| `register` | On/off | `yes` | Whether the phone registers the line. |
| `host` | `dynamic`, or a host | `dynamic` | Any value other than `dynamic` keeps the phone from registering the line. |
| `outboundproxy_address` | Host name or address | None | An outbound proxy for the line. |
| `outboundproxy_port` | Port | `5060` | The port of the outbound proxy. |
| `plar_number` | Number | None | A number that the phone dials as soon as the line goes off hook. |
| `secret` | Text | From PJSIP | The SIP password.  Set for an external line. |
| `authname` | Text | From PJSIP; for an external line `userid`, else the line's name | The SIP authentication user name. |
| `userid` | Text | The line's name | External line: the SIP user name. |
| `server_address` | Host name or address | None | External line: the SIP server. |
| `server_port` | Port | `5060` | External line: the port of the SIP server. |
| `secondary_server_address` | Host name or address | None | External line: an alternate SIP server. |
| `secondary_server_port` | Port | `5060` | External line: the port of the alternate server. |
| `tech` | Text | `PJSIP` for a line with a PJSIP endpoint | The channel technology in the line's hint.  The module sets the value. |
| `pjsip_auth` | Name of an `auth` section | The endpoint's first `auth` section | The `auth` section whose changes the line follows.  The module sets the value. |

For an internal line, the server address, port, and alternate server come from the phone's [networks](#network), not from the line.

## `type=network` { #network }

A network section says which server addresses a phone uses while the phone's address is inside `cidr`.  A phone that moves between networks, e.g., between an office and a VPN, has one section for each.

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `cidr` | Network in CIDR form, e.g., `10.10.0.0/16` | None | The addresses for which the phone uses this network. |
| `alias` | Text | None | The network's name on the phone. |
| `registration_address` | Host name or address | None | The address of this server as seen from the network. |
| `registration_port` | Port | None | The SIP port of this server.  A network section has no default port: set the option. |
| `transport` | `udp`, `tcp`, or `tls` | None | The SIP transport, for lines without `transport`. |
| `alternate_registration_address` | Host name or address | None | A second server that the phone registers to when the first does not answer. |
| `alternate_registration_port` | Port | None | The SIP port of the second server. |
| `alternate_transport` | `udp`, `tcp`, or `tls` | None | The SIP transport to the second server. |
| `file_url_prefix` | URL | None | The URL under which a web server serves the files of `file_directory` to this network.  A `/` is added at the end when missing.  Ringtones, custom apps, `blf_items`, and firmware are offered only with this option. |
| `public_firmware_url_prefix` | URL | `http://dphone.dl.digium.com/firmware/asterisk/` | The URL under which the images of firmware sections with `public_firmware=yes` are found. |
| `ntp_server` | Host name or address | Not sent | The NTP server of the phone. |
| `syslog_server` | Host name or address | None | The syslog server of the phone.  The phone logs to the server only when `syslog_level` is also set. |
| `syslog_port` | Port | Not sent | The port of the syslog server. |
| `syslog_level` | Text, e.g., `error` | Not sent | The lowest level of the messages that the phone logs. |
| `network_vlan_discovery_mode` | Text, e.g., `MANUAL` | Not sent | How the phone finds the VLAN.  The three VLAN options below are sent only with this option. |
| `network_vlan_id` | VLAN number | Not sent | The VLAN of the phone.  Sent when above `0`. |
| `sip_qos` | `0` to `7` | `0` | The 802.1p priority of SIP packets on the VLAN. |
| `rtp_qos` | `0` to `7` | `0` | The 802.1p priority of RTP packets on the VLAN. |
| `network_vlan_qos` | `0` to `7` | — | An old name of `rtp_qos`.  The option has no effect in a network section; use `rtp_qos`. |
| `pc_vlan_id` | VLAN number | `0` | The VLAN of the phone's PC port. |
| `pc_qos` | Number | `0` | The 802.1p priority of the PC port's packets. |
| `sip_dscp` | Number | `0` | The DSCP value of SIP packets. |
| `rtp_dscp` | Number | `0` | The DSCP value of RTP packets. |
| `udp_ka_interval` | Seconds | `0` | The interval of the keep-alive packets that the phone sends over UDP. |

## `type=firmware` { #firmware }

A firmware section names one firmware image.  A phone section lists the images for the phone with `firmware`, and each phone acts on the first entry for the phone's model.  [Firmware](administration.md#firmware) describes how phones decide to install an image.

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `model` | Model name, e.g., `D65` | None | The phone model that the image is for.  The value is put in upper case. |
| `version` | Version text, e.g., `3_0_0` | None | The version of the image.  Stock firmware installs the image when this text differs from the version that the phone runs. |
| `file` | File name or URL | None | The image.  A file name is fetched from the network's `file_url_prefix`; a full URL is used as written.  When the file is in `file_directory`, the entry also carries the image's checksum. |
| `public_firmware` | On/off | `no` | Whether the phone may also fetch the image from the network's `public_firmware_url_prefix`, as `<version>/<version>_<model>_firmware.eff`. |

An entry is sent only for a network that has a `file_url_prefix`, and only when the section has both `version` and `file`.

### Firmware package

A phone section without any `firmware` option is offered the images of the firmware package instead.  The package is the directory named by `firmware_package_directory` of `[general]`, with the images and a file `digium_phones_firmware.conf`.  In that file, each section's name is a phone model:

```ini title="digium_phones_firmware.conf"
[D65]
version=3_0_0
file=3_0_0_D65_firmware.eff
```

| Option | Value | Effect |
| --- | --- | --- |
| `version` | Version text | As `version` of a firmware section. |
| `file` | File name or URL | As `file` of a firmware section.  The checksum is taken from the file in the package directory. |

## `type=application` { #application }

An application section holds settings of one phone app.  The option `application` says which app; the other options depend on that value.

### `application=queue`

A queue section lets the phone's Queues app show one queue of `app_queue`, and log the user in and out.

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `queue` | Queue name | None | The queue. |
| `member` | On/off | `yes` | Whether the user is a member of the queue.  With `no`, the user only watches the queue. |
| `location` | Interface, e.g., `PJSIP/101` | None | The member interface with which the user is added to the queue and found among the members. |
| `membername` | Text | None | The member name with which the user is added. |
| `permission` (or `permissions`) | `status`, `overview`, or `details` | `status` | How much of the queue the phone shows: the user's status only, also the number of members, or also the members that are logged in and the callers that wait. |
| `login_exten` | `extension@context` | None | An extension that the module calls from the phone to log the user in, in place of adding the member directly. |
| `logout_exten` | `extension@context` | None | An extension that the module calls from the phone to log the user out, in place of removing the member directly. |

### `application=status`

A status section adds one presence state to the list that the phone's Status app offers.  A phone with at least one status section offers only the states of the phone's status sections.

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `status` | `available`, `unavailable`, `away`, `xa`, `chat`, or `dnd` | None | The presence state.  Required. |
| `substatus` | Text | None | Repeatable.  A sub-state that the user can choose with the state, e.g., `Out to lunch`. |
| `send486` | On/off | `yes` for `dnd` and `xa`, else `no` | Whether the phone rejects calls as busy while the user is in the state. |

### `application=parking`

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `parkinglot` | Comma-separated parking lot names | None | Repeatable.  The parking lots whose calls the phone's Parking app shows.  A phone without a parking section shows the calls of every lot. |

### `application=voicemail`

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `translation` | Name of a `type=translation` section | None | The names under which the phone shows the voicemail folders. |
| `require_password` | On/off | `no` | Whether the phone's Voicemail app asks for the voicemail password. |

### `application=custom`

A custom section loads an app package onto phones with stock firmware.  A phone with Open DPhone skips the package: the apps of Open DPhone are a different kind, described in the [App SDK](../app-sdk/index.md).

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `name` | Text | None | The app's identifier on the phone. |
| `filename` | File name or URL | None | The app package, fetched from the network's `file_url_prefix`. |
| `md5` (or `md5sum`) | MD5 digest | Not sent | The digest of the package. |
| `autostart` | On/off | `no` | Whether the app starts with the phone. |
| `idlescreen_softkey` | On/off | `no` | Whether the idle screen has a softkey for the app. |
| Any other option | Text | — | Passed to the app as a setting of that name. |

## `type=ringtone` { #ringtone }

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `filename` | File name or URL | None | The sound file, fetched from the network's `file_url_prefix`. |
| `alias` | Text | The section's name | The ringtone's name on the phone. |

## `type=alert` { #alert }

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `alert_info` | Text | None | The value of the `Alert-Info` header that selects the alert.  Required. |
| `ring_type` | Text | `normal` | How the phone alerts: the configuration always has the types `normal`, `ring-answer`, `visual`, and `answer`. |
| `ringtone` | Name of a ringtone | None | The ringtone of the alert. |

## `type=multicastpage` { #multicastpage }

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `address` | Multicast address | None | The address that the phone listens to. |
| `port` | Port | None | The port that the phone listens to. |
| `alias` | Text | The section's name | The page's name on the phone. |
| `priority` | Number | None | The priority among pages. |
| `interrupt` | `0` or `1` | `0` | Whether a page interrupts a call. |

## `type=translation` { #translation }

Each option of a translation section is the name of a voicemail folder, and the value is the name that the phone shows for the folder.

```ini
[de_DE]
type=translation
INBOX=Eingang
Work=Arbeit
```

## Checking what the module read

Each section type has a CLI command that prints what the module holds, which is the way to confirm a change:

| Command | Prints |
| --- | --- |
| `digium_phones show settings` | The `[general]` settings. |
| `digium_phones show phones`, `digium_phones show phone <name>` | The phone sections, and every option of one phone, including the `dphone_*` options. |
| `digium_phones show lines`, `digium_phones show line <name>` | The lines, with the values taken from PJSIP. |
| `digium_phones show networks`, `digium_phones show network <name>` | The networks, including each phone's default network. |
| `digium_phones show firmwares`, `digium_phones show firmware <name>` | The firmware package and the firmware sections. |
| `digium_phones show applications`, `digium_phones show application <name>` | The application sections. |
| `digium_phones show alerts`, `digium_phones show alert <name>` | The alert sections. |
| `digium_phones show multicastpages`, `digium_phones show multicastpage <name>` | The multicast page sections. |
| `digium_phones show translations`, `digium_phones show translation <name>` | The translation sections. |
