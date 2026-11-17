# Alarm tones

The sounds an alarm of the alarm clock can ring with.  The image carries the `.wav` files, `NOTICE`, and `LICENSE-Apache-2.0.txt` in `/usr/share/dphone/alarm-tones/`; `crates/dphone/src/mainrole/subsystems/alarmtones.rs` names the tones and the pause between two passes of each.

All six are derived from the alarm sounds of the Android Open Source Project, repository <https://android.googlesource.com/platform/frameworks/base>, directory `data/sounds`, at commit `1cdfff555f4a21f71ccc978290e2e212e2f8b168`.  Copyright The Android Open Source Project; Apache License, Version 2.0 (`LICENSE-Apache-2.0.txt`).  The build file of that directory, `data/sounds/Android.bp`, names the license for the directory's files (`default_applicable_licenses: ["Android-Apache-2.0"]`).  `NOTICE` carries the copyright notice, the statement of the changes, and the part of the upstream `NOTICE` file that applies; the license requires that it and the license text go wherever the tones go.

| Tone | File | Upstream file (git blob) | Upstream title | Seconds kept | Gain | Pause |
| --- | --- | --- | --- | --- | --- | --- |
| Beep | `beep.wav` | `Alarm_Beep_03.ogg` (`9d800377621c51700d0e01454860381ef04c3b20`) | Beep-Beep-Beep Alarm | 0.6 | +0.5 dB | 600 ms |
| Buzzer | `buzzer.wav` | `Alarm_Buzzer.ogg` (`4234f37a44f03895a58f65029693b63dd3b43367`) | Buzzer Alarm | 1.0 | +1.5 dB | 100 ms |
| Classic | `classic.wav` | `Alarm_Classic.ogg` (`695434f9d9d2aa8c27d01e5e625ec1b722ae1005`) | Cesium | 1.0 | +5.5 dB | 1070 ms |
| Osmium | `osmium.wav` | `alarms/ogg/Osmium.ogg` (`4c760809ccad1781e3d26aa48db9d4561be3853b`) | Osmium | 1.6 | -0.5 dB | 850 ms |
| Rooster | `rooster.wav` | `Alarm_Rooster_02.ogg` (`d20bd1dba800241ac8a2dbd737cddb47c8c0cde0`) | Rooster Alarm | 1.6725 (all) | -1.5 dB | 2000 ms |
| Argon | `argon.wav` | `alarms/ogg/Argon.ogg` (`35addf57853e757825310a908b62993f3575ed90`) | Argon | 3.5, faded out over the last 60 ms | +8 dB | None |

`derive.sh` holds the commands that made each file from the upstream file (ffmpeg 9.0.2): to 16 kHz mono, cut after the last sound of one period, changed in level so that the tones are about equally loud, and stored as 8-bit mu-law, which costs 16 KB per second.  The silence that follows each sound upstream is not stored; the phone leaves it as the pause between two passes.  Argon is the first of the upstream file's two phrases.

A raw file is at `https://android.googlesource.com/platform/frameworks/base/+/1cdfff555f4a21f71ccc978290e2e212e2f8b168/data/sounds/<file>?format=TEXT`, in Base64.
