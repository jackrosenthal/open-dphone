# `config`

The app's provisioned configuration, the phone's accounts, and some settings of the phone.  Capability `config`.  Everything here is read-only.

The phone's configuration can hold an element for an app, which the PBX provisions:

```xml
<appconfig>
  <application id="weather" units="metric">
    <account id="101" username="101" password="..." appserver="dasp://pbx/json" />
    <setting id="city" value="Huntsville" />
  </application>
</appconfig>
```

The `id` of `<application>` is the `name` of the app's manifest.  Every running app that uses `config` receives [`Event::NewConfig`](app-events.md#newconfig) when the phone applies a new configuration.

## `app_config`

```rust
pub fn app_config() -> String
```

WIT name: `app-config`.

The app's `<application>` element as XML text, with every attribute and child element, or `""` when the configuration has none for the app.  The SDK's [`xml::parse`](helpers.md#xml) reads the text.  The text includes the `password` attribute of `<account>` as provisioned, although `app_setting` withholds that value.

## `app_setting`

```rust
pub fn app_setting(name: &str) -> Option<String>
```

WIT name: `app-setting`.

One value of the app's configuration, looked up in this order:

1. `password` and `secret` always return `None`: the phone adds the credentials to PBX requests and does not hand them out.
2. `account_id` returns the `id` attribute of the app's `<account>`, when there is an `<account>`.
3. The attribute `name` of `<application>`.
4. The `value` attribute of the child `<setting id="name">`.

`None` when nothing matches or the app has no `<application>`.  In the example above, `app_setting("units")` is `Some("metric")`, `app_setting("city")` is `Some("Huntsville")`, and `app_setting("account_id")` is `Some("101")`.

## `setting`

```rust
pub fn setting(id: &str) -> Option<String>
```

A setting of the phone.  Only these identifiers answer; any other returns `None`, as does a setting the configuration does not set:

| `id` | Setting of the phone's configuration |
| --- | --- |
| `locale` | `locale` |
| `timezone` | `time_zone` |
| `time_format` | `time_short_format` |
| `date_format` | `date_short_format` |
| `contacts_max_subscriptions` | `contacts_max_subscriptions` |
| `blf_contact_group` | `blf_contact_group` |
| `default_ringtone` | `default_ringtone` |
| `name_format` | `name_format` |
| `brightness` | `brightness` |

The values are the texts of the configuration, unchanged.

## `accounts`

```rust
pub fn accounts() -> Vec<AccountInfo>
```

The phone's SIP accounts, with whether each is registered now.  See [`AccountInfo`](types.md#calls).

## `model`

```rust
pub fn model() -> String
```

The phone's model: `D40`, `D45`, `D50`, `D60`, `D62`, `D65`, or `D70`.

## `firmware_version`

```rust
pub fn firmware_version() -> String
```

WIT name: `firmware-version`.

The version of the firmware with underscores, e.g., `3_0_0`.

## `mac_address`

```rust
pub fn mac_address() -> String
```

WIT name: `mac-address`.

The phone's MAC address as 12 lowercase hexadecimal digits without separators, e.g., `000fd3a1b2c3`.

## `contact_lists`

```rust
pub fn contact_lists() -> Vec<(String, String)>
```

WIT name: `contact-lists`.

The contact-list files provisioned to the phone: each file's name and XML text.  A file that is not UTF-8 is left out.
