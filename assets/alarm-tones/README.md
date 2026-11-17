# Alarm tones

The sounds an alarm of the alarm clock can ring with.  The image carries the `.wav` files, `NOTICE`, and `LICENSE-Apache-2.0.txt` in `/usr/share/dphone/alarm-tones/`; `crates/dphone/src/mainrole/subsystems/alarmtones.rs` names the tones and the pause between two passes of each.

All 39 are derived from the alarm sounds of the Android Open Source Project, repository <https://android.googlesource.com/platform/frameworks/base>, directory `data/sounds`, at commit `1cdfff555f4a21f71ccc978290e2e212e2f8b168`: every file in `data/sounds/alarms` and every `Alarm_*` file in `data/sounds`, 84 files that hold 39 distinct sounds.  Copyright The Android Open Source Project; Apache License, Version 2.0 (`LICENSE-Apache-2.0.txt`).  `NOTICE` carries the copyright notice, the statement of the changes, and the part of the upstream `NOTICE` file that applies; the license requires that it and the license text go wherever the tones go.

## License

The build file of the directory, `data/sounds/Android.bp`, names the license for it: `default_applicable_licenses: ["Android-Apache-2.0"]`.  No directory below `data/sounds` has a build file, a license file, or a `METADATA` file of its own, so the declaration is the one for every file used here.  It takes effect through the directory's build modules, and its module `frameworks_alarm_sounds` installs 18 of the 84 files.  Of the 39 sources, 11 are among those 18, and 4 more (Fermium, Hassium, Nobelium, and Plutonium) decode to the same samples as files among them.  The other 24 are in no module: Rooster 1, Copernicium, Curium, the three JP tones, and all 18 tones from `alarms/material/ogg`.  For these, the directory's declaration and the repository's `MODULE_LICENSE_APACHE2` file are the only license statements, and both name the Apache License 2.0.  No statement in the repository puts any of the 84 files under another license.

## Names and order

A tone has the title that Android gives the sound: the file's `TITLE` tag without the word "Alarm", or the file name where the file has no tag.  Android gives some titles to two or three different sounds:

- The sounds of `alarms/material/ogg`, which took the place of the older ones in Android 5.0, keep the plain title.
- An older sound with the same title takes the other title that Android gives it: Fermium (the old Argon), Hassium (Helium), Neptunium (Carbon), Nobelium (Neon), Plutonium (Krypton), Cesium (Oxygen), and Promethium (Platinum; the file `Promethium.ogg`, whose tag says "Platinum").
- The three sounds of `alarms/ogg-jp` add "JP", from the name of that directory.  The older Osmium, which has no other title, adds "Classic".  The two roosters, both "Rooster Alarm", take the numbers of their files.

The list has the default, Beep-Beep-Beep, first, and the others in alphabetical order.

## The tones

| Tone | File | Upstream file in `data/sounds` (git blob) | Upstream title | Seconds kept | Gain | Pause |
| --- | --- | --- | --- | --- | --- | --- |
| Beep-Beep-Beep | `beep-beep-beep.wav` | `Alarm_Beep_03.ogg` (`9d800377621c51700d0e01454860381ef04c3b20`) | Beep-Beep-Beep Alarm | 0 to 0.561 (first of 2 equal passes) | +0.5 dB | 625 ms |
| Argon | `argon.wav` | `alarms/material/ogg/Argon.ogg` (`87ecfebbe907b3383dc1f60ef49adcaa6a8ad911`) | Argon | 0.03 to 19 | -5 dB | 422 ms |
| Argon JP | `argon-jp.wav` | `alarms/ogg-jp/Argon.ogg` (`2ff6600f7b90a0c1b519181ab9a5f18e6692b902`) | Argon | 0 to 5.23 | -5.5 dB | 770 ms |
| Awaken | `awaken.wav` | `alarms/material/ogg/Awaken_OG7_1ch_48k.ogg` (`47b1d346a682d9e7e154acb9d75408e78c7525b3`) | Awaken | 0.24 to 23.02 | 0 dB | 1032 ms |
| Barium | `barium.wav` | `alarms/ogg/Barium.ogg` (`9a920224cace5deb9eec308c3e50f99c7aacda0e`) | Barium | 0.975 to the end (second of 2 equal passes) | -0.5 dB | none |
| BeeBeep | `beebeep.wav` | `Alarm_Beep_02.ogg` (`43ea34e1de4ceda6b534cbf9c916a3e68fba7eae`) | BeeBeep Alarm | 0 to 0.21 | -1 dB | 38 ms |
| Bounce | `bounce.wav` | `alarms/material/ogg/Bounce_OG7_1ch_48k.ogg` (`3d1f8ea2a8dbe519c79d15e108211aebf9beb85f`) | Bounce | 0 to 14.36 | +0.5 dB | 78 ms |
| Buzzer | `buzzer.wav` | `Alarm_Buzzer.ogg` (`4234f37a44f03895a58f65029693b63dd3b43367`) | Buzzer Alarm | 0 to 1 | +1.5 dB | 103 ms |
| Carbon | `carbon.wav` | `alarms/material/ogg/Carbon.ogg` (`0d96853fcf87949cb63948ddd3ceb8e0bed69554`) | Carbon | 0 to 3.02 | -4 dB | 344 ms |
| Carbon JP | `carbon-jp.wav` | `alarms/ogg-jp/Carbon.ogg` (`c994f286e0815165f7097098aed94719fc9ef74e`) | Carbon | 0 to 2.02 | -4.5 dB | 1180 ms |
| Cesium | `cesium.wav` | `Alarm_Classic.ogg` (`695434f9d9d2aa8c27d01e5e625ec1b722ae1005`) | Cesium | 0 to 0.98 | +2.5 dB | 1090 ms |
| Copernicium | `copernicium.wav` | `alarms/ogg/Copernicium.ogg` (`65d28670d84188d627ce2890146a24cdf4db6f7e`) | none | 0 to 7.77 | -6 dB | 1030 ms |
| Curium | `curium.wav` | `alarms/ogg/Curium.ogg` (`125a2366eb0b6ca519947c6641335f4b6e1d28fa`) | none | 0.06 to 7.72 | -10.5 dB | 740 ms |
| Drip | `drip.wav` | `alarms/material/ogg/Drip_OG7_1ch_48k.ogg` (`de3eec256c7bf37d9bafdc2440581ddabf6591b0`) | Drip | 0 to 2.291 (first of 2 equal passes) | +2.5 dB | 1709 ms |
| Fermium | `fermium.wav` | `alarms/ogg/Fermium.ogg` (`d8f6124364ae81e5a57372a34a26c4b5fc2540cb`) | Fermium | 0 to the end | +8.5 dB | none |
| Gallop | `gallop.wav` | `alarms/material/ogg/Gallop_OG7_1ch_48k.ogg` (`98549b37d38b992c9bb36b714250f68cca7bf55f`) | Gallop | 3.6 to 7.08 (second of 3 equal passes) | -0.5 dB | 120 ms |
| Hassium | `hassium.wav` | `alarms/ogg/Hassium.ogg` (`793c2697eac043fc7c5b8f9f3e4b8c9eea75d169`) | Hassium | 0 to 7.98 | -2 dB | 854 ms |
| Helium | `helium.wav` | `alarms/material/ogg/Helium.ogg` (`d381448e3d95e875d1fd9b4e21bfb8faa4e806e0`) | Helium | 0 to the end | -7 dB | none |
| Helium JP | `helium-jp.wav` | `alarms/ogg-jp/Helium.ogg` (`94f13ce0ddb73774a42694f7277c890493e6bd02`) | Helium | 0 to 8.01 | -8 dB | 790 ms |
| Krypton | `krypton.wav` | `alarms/material/ogg/Krypton.ogg` (`4529aa8c9b4fe2e303f609a11f7bad7e8a05934d`) | Krypton | 0 to the end | 0 dB | none |
| Neon | `neon.wav` | `alarms/material/ogg/Neon.ogg` (`61582d5853a16bb2816f0ca08c82ca04e65d1750`) | Neon | 0 to the end | -4.5 dB | none |
| Neptunium | `neptunium.wav` | `alarms/ogg/Neptunium.ogg` (`d99f13344f30835a94aa2ae2128beaf8708503b4`) | Neptunium | 0 to the end | -1.5 dB | none |
| Nobelium | `nobelium.wav` | `alarms/ogg/Nobelium.ogg` (`1f94d1e9dfe790a1c5fb2b74bc2134bc1443650e`) | Nobelium | 0 to 19.62 | -4 dB | 780 ms |
| Nudge | `nudge.wav` | `alarms/material/ogg/Nudge_OG7_1ch_48k.ogg` (`7a3d7ed474c640cfd008ca6448fe8319908c91a4`) | Nudge | 0 to 5.1 | -3 dB | 1284 ms |
| Orbit | `orbit.wav` | `alarms/material/ogg/Orbit_OG7_1ch_48k.ogg` (`52e3918326edf8357ca1bfef6cb1f15cb0ea5ab9`) | Orbit | 0 to 10.3 | -3 dB | 659 ms |
| Osmium | `osmium.wav` | `alarms/material/ogg/Osmium.ogg` (`245c4cebc2abc65bddae9a7ebc1f747b2eeabbdd`) | Osmium | 0 to 5.24 | -5 dB | 1077 ms |
| Osmium Classic | `osmium-classic.wav` | `alarms/ogg/Osmium.ogg` (`4c760809ccad1781e3d26aa48db9d4561be3853b`) | Osmium | 0 to 1.71 | -0.5 dB | 736 ms |
| Oxygen | `oxygen.wav` | `alarms/material/ogg/Oxygen.ogg` (`0b489bde7f14f3ac2bcf1d66477ea1b970234b06`) | Oxygen | 0 to 10.36 | -2.5 dB | 374 ms |
| Piezo | `piezo.wav` | `Alarm_Beep_01.ogg` (`b7de5af7813c632c8a20c2a453df0cdd39b77258`) | Piezo Alarm | 0.03 to 0.41 (first of 2 equal passes) | +1 dB | 671 ms |
| Platinum | `platinum.wav` | `alarms/material/ogg/Platinum.ogg` (`053b008b551347e5a6c9a6a209f6510f84cf1ee8`) | Platinum | 0 to 23.67 | +0.5 dB | 52 ms |
| Plutonium | `plutonium.wav` | `alarms/ogg/Plutonium.ogg` (`5e9a75093c6d5b0c00929167a5710af408f0eec8`) | Plutonium | 0 to 3.41 | -0.5 dB | 1553 ms |
| Promethium | `promethium.wav` | `alarms/ogg/Promethium.ogg` (`d5f0893e4cf742e1dff72ab4a46c2419f71b1868`) | Platinum | 0 to 6.65 | -0.5 dB | 64 ms |
| Rise | `rise.wav` | `alarms/material/ogg/Rise_OG7_1ch_48k.ogg` (`a2d00cc8ddbc453a85af932f0ecf4c77fa4a9704`) | Rise | 0 to 4.23 (first of 2 equal passes) | +10 dB | 201 ms |
| Rooster 1 | `rooster-1.wav` | `Alarm_Rooster_01.ogg` (`36b57b717b0d263efb1251444837f976d3bf6ee8`) | Rooster Alarm | 0 to 1.6 | -1 dB | 2000 ms |
| Rooster 2 | `rooster-2.wav` | `Alarm_Rooster_02.ogg` (`d20bd1dba800241ac8a2dbd737cddb47c8c0cde0`) | Rooster Alarm | 0 to the end | -1.5 dB | 2000 ms |
| Scandium | `scandium.wav` | `alarms/ogg/Scandium.ogg` (`86ffbf89d0379c970a70bd5ae531582f53fc449e`) | Scandium | 0 to 2.47 | -2.5 dB | 1468 ms |
| Sway | `sway.wav` | `alarms/material/ogg/Sway_OG7_1ch_48k.ogg` (`273728a6e4170bd97315f630b0366bcd86a2ac0f`) | Sway | 0 to the end | 0 dB | none |
| Timer | `timer.wav` | `alarms/material/ogg/Timer.ogg` (`97519aa971b0c23722b4eee7413dc68b73ae33bf`) | none | 0.03 to 2.35 | -6.5 dB | 1317 ms |
| Wag | `wag.wav` | `alarms/material/ogg/Wag_OG7_1ch_48k.ogg` (`f61d5b9b75f30b1cefef7aac8b4d546bf28fd168`) | Wag | 0 to 3.48 | -2 dB | 1771 ms |

`derive.sh` holds the commands that made each file from the upstream file (ffmpeg 9.0.2): to 16 kHz mono, cut to the sound of one pass, changed in level so that the tones are about equally loud, and stored as 8-bit mu-law, which costs 16 KB per second.  A pass ends where the sound falls 50 dB below its loudest 10 ms.  The silence before and after the sound upstream is not stored; the phone leaves it as the pause between two passes, so that a tone repeats at the period of its upstream file.  Where the upstream file holds two or three equal passes, one is kept.  A tone without a pause runs into its next pass as its upstream file does when it loops.  The roosters' pause is a choice: upstream, the crow repeats at once.  The level brings the samples above a twentieth of the peak to an RMS of 5,500 (-15.5 dBFS), or less where the peak would pass 26,000.

The 39 files are 4,601,326 bytes; in the image's compressed root file system, with `NOTICE` and the license text, they take about 3.4 MB.

A raw file is at `https://android.googlesource.com/platform/frameworks/base/+/1cdfff555f4a21f71ccc978290e2e212e2f8b168/data/sounds/<file>?format=TEXT`, in Base64.

## Files not used

The other 45 files hold sounds already in the table, each compared with the file used by its samples decoded at 16 kHz.  "Same samples" means that the two decode to identical samples; "same recording" means a waveform correlation above 0.999, a separate encoding of the same audio.

| Sound | Files not used |
| --- | --- |
| Cesium | `alarms/Cesium.ogg` and `alarms/ogg/Cesium.ogg` (one blob), `alarms/ogg/Oxygen.ogg` and `alarms/ogg-jp/Oxygen.ogg` (one blob): same samples.  `alarms/ogg/Departure.ogg` (titled "Blue Alarm"), and `Alarm_Classic.wav`, `alarms/wav/Cesium.wav`, and `alarms/wav/Oxygen.wav` (one blob): same recording. |
| Fermium | `alarms/ogg/Argon.ogg`: same samples.  `alarms/wav/Argon.wav` and `alarms/wav/Fermium.wav` (one blob): same recording. |
| Barium | `alarms/ogg/Analysis.ogg` (titled "Violet Alarm") and `alarms/wav/Barium.wav`: same recording. |
| Neptunium | `alarms/ogg/Carbon.ogg`: same samples.  `alarms/wav/Carbon.wav` and `alarms/wav/Neptunium.wav` (one blob): same recording. |
| Copernicium | `alarms/wav/Copernicium.wav`: same recording. |
| Curium | `alarms/wav/Curium.wav`: same recording. |
| Hassium | `alarms/ogg/Helium.ogg`: same samples.  `alarms/wav/Hassium.wav` and `alarms/wav/Helium.wav` (one blob): same recording. |
| Plutonium | `alarms/ogg/Krypton.ogg` and `alarms/ogg-jp/Krypton.ogg` (one blob): same samples.  `alarms/ogg/FireDrill.ogg` (titled "White Alarm"), and `alarms/wav/Krypton.wav` and `alarms/wav/Plutonium.wav` (one blob): same recording. |
| Nobelium | `alarms/ogg/Neon.ogg` and `alarms/ogg-jp/Neon.ogg` (one blob): same samples.  `alarms/wav/Neon.wav` and `alarms/wav/Nobelium.wav` (one blob): same recording. |
| Osmium Classic | `alarms/wav/Osmium.wav`: same recording. |
| Promethium | `alarms/ogg/Platinum.ogg` (the same blob).  `alarms/wav/Platinum.wav` and `alarms/wav/Promethium.wav` (one blob): same recording. |
| Scandium | `alarms/ogg/Timing.ogg` (titled "Green Alarm") and `alarms/wav/Scandium.wav`: same recording. |
| The nine sounds of `alarms/material/ogg` with a 48 kHz version | `Argon_48k.ogg`, `Carbon_48k.ogg`, `Helium_48k.ogg`, `Krypton_48k.ogg`, `Neon_48k.ogg`, `Osmium_48k.ogg`, `Oxygen_48k.ogg`, `Platinum_48k.ogg`, and `Timer_48k.ogg`: the same notes at the same times (their spectrograms correlate at 0.84 to 0.99; Platinum's 48 kHz version starts 0.35 s later), mastered separately at 48 kHz.  Android installs one or the other under the same name. |
