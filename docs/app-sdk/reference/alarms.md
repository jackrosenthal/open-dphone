# `alarms`

The phone's alarm clock.

The phone stores the alarms and rings them whether or not any app is running.  When an alarm is due, the phone plays the alarm's tone and shows the alarm, with Snooze and Dismiss, on top of whatever is on screen.  This interface lets an app display and change the alarms.  The web interface edits the same alarms.  Every running app that uses `alarms` receives [`Event::AlarmsChanged`](app-events.md#alarmschanged) after each change.

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
| `id` | The alarm's ID.  Never 0 for an existing alarm. |
| `hour`, `minute` | The time of day in the phone's time zone: 0 to 23, and 0 to 59. |
| `days` | The days of the week on which the alarm rings, as a bit mask: bit 0 is Sunday, and bit 6 is Saturday.  0 makes a one-time alarm, which rings the next time the clock reaches the alarm's time and is then disabled. |
| `tone` | The alarm tone, one of the names that `tones()` returns. |
| `enabled` | Whether the alarm is on.  A disabled alarm doesn't ring. |

The functions that return `Result` wait up to 1.5 seconds for the alarm clock, which writes the alarms to flash before answering.  A bad argument returns `Error::Invalid(text)`.  A failure, including no answer in time, returns `Error::Failed(text)`.  When no answer comes in time, the alarm clock may still carry out the request.

## `list`

```rust
pub fn list() -> Vec<Alarm>
```

Returns the alarms, in the order they were added.

## `set`

```rust
pub fn set(a: &Alarm) -> Result<u32, Error>
```

Adds an alarm if `a.id` is 0, or updates the alarm with that `id`.  Returns the alarm's `id`.

```rust
// Every Monday at 8:00, with the phone's first alarm tone.
let id = alarms::set(&Alarm {
    id: 0,
    hour: 8,
    minute: 0,
    days: 1 << 1,
    tone: String::new(),
    enabled: true,
})?;
```

- An empty `tone` selects the phone's first alarm tone.  Tone names are matched without regard to case.
- An updated alarm starts over from the current time: if the new time has already passed today, the alarm isn't due until tomorrow.  If the alarm was ringing or snoozed, that ends.
- Setting an alarm to the values that the alarm already has changes nothing.

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

Removes an alarm.  Returns `Invalid("no such alarm")` if no alarm has that `id`.

## `tones`

```rust
pub fn tones() -> Vec<String>
```

Returns the names of the alarm tones.  The alarm clock has a separate set of tones; the phone's ringtones are not available to alarms.  The first name is the default tone.

## `preview`

```rust
pub fn preview(tone: Option<&str>)
```

Plays or stops a preview of an alarm tone, for use while the user picks a tone.

`Some(name)` plays the alarm tone for up to four seconds, replacing any sound that is playing.  A name that isn't an alarm tone plays the default tone.

`None` stops a preview that the app started, if the preview is still within those four seconds.  At any other time, `None` does nothing, so a sound that another app started keeps playing.

## `next`

```rust
pub fn next() -> Option<(u32, u64)>
```

Returns the next time that an alarm will ring, as the alarm's `id` and the time in seconds since the Unix epoch.  A snoozed alarm counts.  Returns `None` if no alarm is due to ring.

## `snoozed`

```rust
pub fn snoozed() -> Option<(u32, u64)>
```

Returns the snoozed alarm, if there is one, as the alarm's `id` and the time when the alarm will ring again.

## `dismiss`

```rust
pub fn dismiss() -> Result<(), Error>
```

Dismisses the alarm that is ringing or snoozed.  Returns `Invalid("no alarm is ringing or snoozed")` if there is none.
