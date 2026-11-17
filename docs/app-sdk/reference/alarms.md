# `alarms`

The phone stores the alarms and rings them whether or not any app runs.  The web interface edits the same alarms.  Every running app that uses `alarms` receives [`Event::AlarmsChanged`](app-events.md#alarmschanged) after each change.

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

| Field | Contents |
| --- | --- |
| `id` | Never 0 for an existing alarm. |
| `hour`, `minute` | The time of day in the phone's time zone: 0 to 23, and 0 to 59. |
| `days` | The days on which the alarm rings, as a bit mask: bit 0 is Sunday, and bit 6 is Saturday.  0 makes a one-time alarm, which rings the next time the clock reaches the alarm's time and is then disabled. |
| `tone` | One of the names that `tones()` returns. |

The functions that return `Result` wait up to 1.5 seconds for the alarm clock, which writes the alarms to flash before answering.  A bad argument returns `Error::Invalid(text)`.  A failure, including no answer in time, returns `Error::Failed(text)`; without an answer, the alarm clock may still carry out the request.

## `list`

```rust
pub fn list() -> Vec<Alarm>
```

The alarms, in the order added.

## `set`

```rust
pub fn set(a: &Alarm) -> Result<u32, Error>
```

Adds an alarm if `a.id` is 0, or else updates the alarm with that `id`.  Returns the alarm's `id`.

- An empty `tone` selects the phone's first alarm tone.  Tone names are matched without regard to case.
- An updated alarm starts over from the current time: a time already past today is due tomorrow.  Updating a ringing or snoozed alarm ends the ringing or the snooze.
- Setting the values that the alarm already has changes nothing.

| `Invalid` text | Cause |
| --- | --- |
| `the time must be between 00:00 and 23:59` | `hour` or `minute` is out of range. |
| `the days are not days of the week` | `days` is greater than 127. |
| `the phone has no alarm tone named "..."` | `tone` is not one of the names that `tones()` returns. |
| `the phone keeps at most 16 alarms` | `a.id` is 0, and 16 alarms already exist. |
| `no such alarm` | `a.id` is not 0, and no alarm has that `id`. |

## `remove`

```rust
pub fn remove(id: u32) -> Result<(), Error>
```

Returns `Invalid("no such alarm")` if no alarm has that `id`.

## `tones`

```rust
pub fn tones() -> Vec<String>
```

The names of the alarm tones; the first is the default.  Alarm tones are a separate set from the phone's ringtones.

## `preview`

```rust
pub fn preview(tone: Option<&str>)
```

- `Some(name)` plays the alarm tone for up to four seconds, replacing any playing sound.  A name that isn't an alarm tone plays the default tone.
- `None` stops the app's preview within those four seconds, and otherwise does nothing, so another app's sound keeps playing.

## `next`

```rust
pub fn next() -> Option<(u32, u64)>
```

The next alarm to ring, including a snoozed alarm, as the alarm's `id` and the time in seconds since the Unix epoch.  `None` if no alarm is due.

## `snoozed`

```rust
pub fn snoozed() -> Option<(u32, u64)>
```

The snoozed alarm, if any, as the alarm's `id` and the time of the next ring.

## `dismiss`

```rust
pub fn dismiss() -> Result<(), Error>
```

Dismisses the ringing or snoozed alarm.  Returns `Invalid("no alarm is ringing or snoozed")` if there is none.
