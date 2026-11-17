# `config`

Read-only access to the app's provisioned configuration, the phone's accounts, and some of the phone's settings.

The phone's configuration can hold an element for the app, whose `id` is the app's manifest `name`:

```xml
<appconfig>
  <application id="weather" units="metric">
    <account id="101" username="101" password="..." appserver="dasp://pbx/json" />
    <setting id="city" value="Huntsville" />
  </application>
</appconfig>
```

The phone adds the credentials to the app's PBX requests; no function returns them.  Every running app that uses `config` receives [`Event::NewConfig`](app-events.md#newconfig) when the phone applies a new configuration.

## `app_config`

```rust
pub fn app_config() -> String
```

The app's `<application>` element as XML text, or `""` if the configuration has none.  [`xml::parse`](helpers.md#xml) reads the text.  The text omits the credentials: the `password` and `secret` attributes of `<application>` and of each `<account>`, and each child `<setting>` whose `id` is `password` or `secret`.

## `app_setting`

```rust
pub fn app_setting(name: &str) -> Option<String>
```

One value of the app's configuration, by the first rule that applies:

1. `password` and `secret` return `None`.
2. `account_id` returns the `id` attribute of the app's `<account>`, if any.
3. The attribute of `<application>` named `name`.
4. The `value` attribute of the child `<setting>` whose `id` is `name`.

`None` if nothing matches or the app has no `<application>`.  In the example, `app_setting("units")` is `Some("metric")`, `app_setting("city")` is `Some("Huntsville")`, and `app_setting("account_id")` is `Some("101")`.

## `setting`

```rust
pub fn setting(id: &str) -> Option<String>
```

One of the phone's settings, exactly as written in the configuration.  Only these identifiers are available; others, and settings that the configuration doesn't set, return `None`.

| `id` | Setting in the phone's configuration |
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

## `accounts`

```rust
pub fn accounts() -> Vec<AccountInfo>
```

The phone's SIP accounts ([`AccountInfo`](types.md#calls)).

## `model`

```rust
pub fn model() -> String
```

`D40`, `D45`, `D50`, `D60`, `D62`, `D65`, or `D70`.

## `firmware_version`

```rust
pub fn firmware_version() -> String
```

The firmware version, with underscores as separators, e.g., `3_0_0`.

## `mac_address`

```rust
pub fn mac_address() -> String
```

The MAC address as 12 lowercase hexadecimal digits without separators, e.g., `000fd3a1b2c3`.

## `contact_lists`

```rust
pub fn contact_lists() -> Vec<(String, String)>
```

The provisioned contact list files, as pairs of file name and XML text.  Files that aren't valid UTF-8 are omitted.
