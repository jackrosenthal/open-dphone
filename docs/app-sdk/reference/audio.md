# `audio`

Sound playback.

The phone plays one sound at a time across all apps, through the same path as a ringtone.

## `play`

```rust
pub fn play(sound: &[u8], start: u32) -> Result<(), Error>
```

Plays a sound once, starting `start` seconds in.

```rust
audio::play(include_bytes!("chime.wav"), 0)?;
```

`sound` is the contents of a sound file, which an app usually embeds with `include_bytes!`:

- A WAVE file (data that starts with `RIFF`) can contain 8-bit or 16-bit PCM, mu-law, or A-law, at any sample rate.  For a file with several channels, the phone plays the first.
- Anything else is treated as headerless 16 kHz mono signed 16-bit little-endian samples (the `.sln16` format).

A sound can be at most 8 MiB and at most five minutes long.

If a sound is already playing, whether started by this app or another, the new sound replaces that sound, and the app that started the old sound receives `Event::PlaybackStopped`.

When the new sound is over, the app receives [`Event::PlaybackStopped(position)`](app-events.md#playbackstopped).  The sound also stops when the instance is discarded.

Returns `Err(Error::Failed(text))` if the sound doesn't start: the data can't be decoded, the sound is too large or too long, the phone is ringing, or the phone has no audio.  The phone decodes the sound before answering, and `play` waits up to 5 seconds.  If no answer comes in that time, `play` returns `Failed`, but the sound may still start afterward.

## `stop`

```rust
pub fn stop()
```

Stops the sound that the app started with `play`, if that sound is still playing.  The app then receives `Event::PlaybackStopped`.  If the app has no sound playing, `stop` does nothing: another app's sound, or a ringtone preview, keeps playing.
