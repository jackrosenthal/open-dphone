# `config`

Read-only access to the app's provisioned configuration, the phone's accounts, and some of the phone's settings.

The phone's configuration, which the PBX provisions, can contain an element for an app:

```xml
<appconfig>
  <application id="weather" units="metric">
    <account id="101" username="101" password="..." appserver="dasp://pbx/json" />
    <setting id="city" value="Huntsville" />
  </application>
</appconfig>
```

The `id` of `<application>` is the `name` in the app's manifest.

Credentials stay with the phone.  The phone adds them to the app's PBX requests, and no function in this interface returns them.

Every running app that uses `config` receives [`Event::NewConfig`](app-events.md#newconfig) when the phone applies a new configuration.

## `app_config`

```rust
pub fn app_config() -> String
```

Returns the app's `<application>` element as XML text, or `""` if the configuration has no element for the app.  The SDK's [`xml::parse`](helpers.md#xml) can read the text.

The text includes every attribute and child element except credentials: the `password` and `secret` attributes of `<application>` and of each `<account>`, and any child `<setting>` whose `id` is `password` or `secret`.  (`app_setting` withholds the same values.)  For the example above, the `<account>` in the text has `id`, `username`, and `appserver`.

## `app_setting`

```rust
pub fn app_setting(name: &str) -> Option<String>
```

Returns one value from the app's configuration.  The lookup uses the first of these rules that applies:

1. `password` and `secret` always return `None`.
2. `account_id` returns the `id` attribute of the app's `<account>`, if there is one.
3. The attribute of `<application>` whose name is `name`.
4. The `value` attribute of the child `<setting>` whose `id` is `name`.

Returns `None` if nothing matches or the app has no `<application>` element.  In the example above, `app_setting("units")` is `Some("metric")`, `app_setting("city")` is `Some("Huntsville")`, and `app_setting("account_id")` is `Some("101")`.

## `setting`

```rust
pub fn setting(id: &str) -> Option<String>
```

Returns one of the phone's settings.  Only the identifiers in this table are available.  Any other identifier returns `None`, and so does a setting that the configuration doesn't set.

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

Values are returned exactly as written in the configuration.

## `accounts`

```rust
pub fn accounts() -> Vec<AccountInfo>
```

Returns the phone's SIP accounts, including whether each is currently registered.  See [`AccountInfo`](types.md#calls) for the fields.

## `model`

```rust
pub fn model() -> String
```

Returns the phone's model: `D40`, `D45`, `D50`, `D60`, `D62`, `D65`, or `D70`.

## `firmware_version`

```rust
pub fn firmware_version() -> String
```

Returns the firmware version, with underscores as separators: `3_0_0`, for example.

## `mac_address`

```rust
pub fn mac_address() -> String
```

Returns the phone's MAC address as 12 lowercase hexadecimal digits with no separators: `000fd3a1b2c3`, for example.

## `contact_lists`

```rust
pub fn contact_lists() -> Vec<(String, String)>
```

Returns the contact list files provisioned to the phone, as pairs of file name and XML text.  Files that aren't valid UTF-8 are omitted.
