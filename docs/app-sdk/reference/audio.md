# `audio`

Playing a sound.  Capability `audio`.

The phone plays one sound at a time, for all apps together, the way the phone plays a ringtone.

## `play`

```rust
pub fn play(sound: &[u8], start: u32) -> Result<(), Error>
```

Plays a sound once, beginning `start` seconds into the sound.  `sound` is the content of a sound file, which an app usually holds with `include_bytes!`:

- A WAVE file (the content starts with `RIFF`): 8-bit or 16-bit PCM, mu-law, or A-law, at any sample rate; of several channels, the first is played.
- Anything else is taken as headerless 16 kHz mono signed 16-bit little-endian samples (the format `.sln16`).

A sound is at most 8 MiB and at most five minutes long.  A sound that is already playing, of this app or another, is replaced, and the app that started that sound receives `Event::PlaybackStopped`.

[`Event::PlaybackStopped(position)`](app-events.md#playbackstopped) follows when the sound is over.  The sound also stops when the instance is dropped.

`Err(Error::Failed(text))` when the sound does not start: the content cannot be decoded, the sound is too large or too long, the phone is ringing, or the phone has no audio.  The phone decodes before answering and `play` waits up to 5 seconds; a `Failed` for a missing answer in that time does not rule out that the sound starts afterward.

```rust
audio::play(include_bytes!("chime.wav"), 0)?;
```

## `stop`

```rust
pub fn stop()
```

Stops the sound that the app started with `play`, when that sound is still playing; the app then receives `Event::PlaybackStopped`.  When the app has no sound playing, nothing happens: another app's sound and the preview of a ringtone play on.
