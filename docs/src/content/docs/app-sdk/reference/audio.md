---
title: "audio"
---

The phone plays one sound at a time across all apps, through the ringtone's path.

## `play`

```rust
pub fn play(sound: &[u8], start: u32) -> Result<(), Error>
```

Plays `sound` once, starting `start` seconds in.  `sound` is the contents of a sound file:

- A WAVE file (data starting with `RIFF`): 8-bit or 16-bit PCM, mu-law, or A-law, at any sample rate.  Of several channels, the first plays.
- Anything else: headerless 16 kHz mono signed 16-bit little-endian samples (`.sln16`).

A sound is at most 8 MiB and five minutes long.

A new sound replaces a playing sound from any app, and the app that started the old sound receives `Event::PlaybackStopped`.  When the new sound ends, the app receives [`Event::PlaybackStopped(position)`](app-events.md#playbackstopped).  The sound also stops when the instance is discarded.

Returns `Err(Error::Failed(text))` if the sound doesn't start: the data can't be decoded, the sound is too large or too long, the phone is ringing, or the phone has no audio.  `play` waits up to 5 seconds while the phone decodes the sound; after that, `play` returns `Failed`, and the sound may still start.

## `stop`

```rust
pub fn stop()
```

Stops the app's sound from `play`, if still playing; the app then receives `Event::PlaybackStopped`.  Otherwise does nothing: another app's sound, or a ringtone preview, keeps playing.
