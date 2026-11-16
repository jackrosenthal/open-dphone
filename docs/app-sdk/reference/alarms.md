# `alarms`

The phone's alarm clock.  Capability `alarms`.

The phone keeps the alarms and rings them whether or not any app runs: when an alarm is due, the phone sounds the alarm's ringtone and shows the alarm with Snooze and Dismiss over whatever is on screen.  This interface lets an app show the alarms and change them.  The web interface changes the same alarms.  Every running app that uses `alarms` receives [`Event::AlarmsChanged`](app-events.md#alarmschanged) after each change.

```rust
pub struct Alarm {
    pub id: u32,
    pub hour: u8,
    pub minute: u8,
    pub days: u8,
    pub tone: String,
    pub enabled: bool,
}
```

| Field | Content |
| --- | --- |
| `id` | Identifies the alarm; never 0 for an alarm that exists. |
| `hour`, `minute` | The time of day in the phone's time zone: 0 to 23 and 0 to 59. |
| `days` | The days of the week the alarm rings on: bit 0 is Sunday and bit 6 is Saturday.  0 is a one-time alarm, which rings the next time the clock shows the time and is then switched off. |
| `tone` | The ringtone, one of `tones()`. |
| `enabled` | An alarm that is switched off does not ring. |

The functions that return `Result` wait up to 1.5 seconds for the alarm clock, which writes the alarms to the flash before answering.  A refusal for a bad argument is `Error::Invalid(text)`; anything else, also a missing answer, is `Error::Failed(text)`.

## `list`

```rust
pub fn list() -> Vec<Alarm>
```

The alarms, in the order in which they were added.

## `set`

```rust
pub fn set(a: &Alarm) -> Result<u32, Error>
```

Adds an alarm when `a.id` is 0, or changes the alarm with that `id`.  Returns the alarm's `id`.

- An empty `tone` means the phone's first ringtone.  A name is matched without regard to letter case.
- A changed alarm counts from now: a time earlier today is not due until tomorrow.  If the alarm was ringing or snoozed, that ends.
- Setting an alarm to the values the alarm already has changes nothing.

| `Invalid` text | Cause |
| --- | --- |
| `the time must be between 00:00 and 23:59` | `hour` or `minute` is out of range. |
| `the days are not days of the week` | `days` is above 127. |
| `the phone has no ringtone named "..."` | `tone` is not one of `tones()`. |
| `the phone keeps at most 16 alarms` | `a.id` is 0 and 16 alarms exist. |
| `no such alarm` | `a.id` is not 0 and names no alarm. |

```rust
// Every Monday at 8:00, with the phone's first ringtone.
let id = alarms::set(&Alarm {
    id: 0,
    hour: 8,
    minute: 0,
    days: 1 << 1,
    tone: String::new(),
    enabled: true,
})?;
```

## `remove`

```rust
pub fn remove(id: u32) -> Result<(), Error>
```

Removes an alarm.  `Invalid("no such alarm")` when no alarm has the `id`.

## `tones`

```rust
pub fn tones() -> Vec<String>
```

The names of the phone's ringtones, which are the tones an alarm can ring with.

## `preview`

```rust
pub fn preview(tone: Option<&str>)
```

`Some(name)` sounds a ringtone once, for choosing one; `None` stops the sound.  A name that is not a ringtone is not reported.  `None` stops whatever sound is playing, also a sound that an app started with [`audio::play`](audio.md#play).

## `next`

```rust
pub fn next() -> Option<(u32, u64)>
```

The next time an alarm rings: the alarm's `id` and the time in seconds since the Unix epoch.  A snoozed alarm counts.  `None` when no alarm is due to ring.

## `snoozed`

```rust
pub fn snoozed() -> Option<(u32, u64)>
```

The snoozed alarm, if there is one: the alarm's `id` and the time at which the alarm rings again.

## `dismiss`

```rust
pub fn dismiss() -> Result<(), Error>
```

Ends the alarm that is ringing or snoozed.  `Invalid("no alarm is ringing or snoozed")` when there is none.

## `icon`

```rust
pub fn icon() -> bool
```

Whether the idle screen shows an icon while an alarm is upcoming, which means that an alarm rings within 24 hours or is snoozed.

## `set_icon`

```rust
pub fn set_icon(on: bool) -> Result<(), Error>
```

WIT name: `set-icon`.

Turns the idle screen's alarm icon on or off for the phone.
