# Ringtones

The phone's built-in ringtones.  The image carries the `.wav` files, `NOTICE`, and `LICENSE-Apache-2.0.txt` in `/usr/share/dphone/ringtones/`; `crates/dphone/src/mainrole/subsystems/ringtones.rs` names the ringtones and the pause between two passes of each, and carries a copy of the default ringtone, which rings when a ringtone's file cannot be played.

All 149 are derived from the ringtones of the Android Open Source Project, repository <https://android.googlesource.com/platform/frameworks/base>, directory `data/sounds`, at commit `1cdfff555f4a21f71ccc978290e2e212e2f8b168`: every file in `data/sounds/ringtones`, every `Ring_*` file in `data/sounds`, and the 61 files of `data/sounds/newwavelabs` that Android installs as ringtones, 211 files that hold 149 distinct sounds.  Copyright The Android Open Source Project; Apache License, Version 2.0 (`LICENSE-Apache-2.0.txt`).  `NOTICE` carries the copyright notice, the statement of the changes, and the part of the upstream `NOTICE` file that applies; the license requires that it and the license text go wherever the ringtones go.

The 61 of `data/sounds/newwavelabs` are the ones that Android's ringtone build module, `frameworks_ringtones_sounds` in `data/sounds/Android.bp`, installs, and that the G1's audio package (`OriginalAudio.mk`) and `AudioPackage2.mk` to `AudioPackage4.mk` install as ringtones.  The directory's other 50 files are not used: 37 PCM masters (`.wav`), 7 notification sounds, and 6 Ogg files that nothing installs (`CousinJones.ogg`, `Gotcha.ogg`, `Jump_Up.ogg`, `Lectro_Beat.ogg`, `Miami_Twice.ogg`, and `NoiseyDing.ogg`).

## License

The build file of the directory, `data/sounds/Android.bp`, names the license for it: `default_applicable_licenses: ["Android-Apache-2.0"]`, in place of the repository's `frameworks_base_license`.  No directory below `data/sounds` has a build file, a license file, or a `METADATA` file of its own, so the declaration is the one for every file used here.  It takes effect through the directory's build modules, and its module `frameworks_ringtones_sounds` installs 102 of the 211 files, all 61 of `newwavelabs` among them.  Of the 149 sources, 90 are among those 102, and 4 more (Boötes, Dione Classic, Eridani, and Testudo) decode to the same samples as files among them.  The other 55 are in no module: the 11 untitled `Ring_*` files, the 19 `ringtoneNN` files, all 12 ringtones of `ringtones/material/ogg`, and Acheron, Callisto Classic, Ganymede Classic, Iridium, Luna Classic, Lyra, Nasqueron, Oberon Classic, Phobos Classic, Sceptrum Classic, Solarium Classic, Triton Classic, and Umbriel Classic.  For these, the directory's declaration and the repository's `MODULE_LICENSE_APACHE2` file are the license statements, and both name the Apache License 2.0.  The repository's `METADATA` file classifies the repository as a whole (`license_type: RECIPROCAL`) and names no license or file.  No statement in the repository puts any of the 211 files under another license.  Sixteen files carry a copyright tag, `(c) Copyright 2009 Android Open Source Project` or `Copyright (c) 2009 Android Open Source Project`: `ringtones/Carina.ogg`, `ringtones/FreeFlight.ogg`, `ringtones/FreeFlight.wav`, and 13 files of `newwavelabs`.  The artist tags (pdx, Dr. Mad, Ali Spagnola, and Dave Sparks) carry no license.

## The default

Flutey Phone (`Ring_Synth_04.ogg`) is the default: the ringtone a phone rings with when nothing else is chosen, and for a name that is no ringtone.  It is the default of the Android Open Source Project's own builds: `target/product/full_base.mk` of <https://android.googlesource.com/platform/build> sets `ro.config.ringtone?=Ring_Synth_04.ogg` among the "Additional settings used in all AOSP builds" (at commit `045a3d6a3e359633a14853a5a5e1e4f2a11cbdae`).  It is also short: 1.7 s of sound and a 2 s pause, 27,418 bytes, so the program carries a copy of it at little cost.

## Names and order

A ringtone has the title that Android gives the sound: the file's `TITLE` tag, or, where the file has no tag, the file name without `Ring_` and with the number written plainly (`Ring_Classic_01.ogg` is Classic 1, `ringtone18.ogg` is Ringtone 18, and `ringtones/ogg/Lyra.ogg` is Lyra).  A sound in files of two titles has the title of the file that Android installs, or of the later release where Android installs both: Girtab and Zeta (`ringtones/ogg/Girtab.ogg` and `ringtones/ogg/Zeta.ogg`, which Android 4.0 to 4.3 install), not Cassiopeia and Vespa (`ringtones/ogg/Cassiopeia.ogg` and `ringtones/ogg/Vespa.ogg`, which no release installs); and Carina and Eridani, the titles from Android 2.3, not Ding and Mildly Alarming (`newwavelabs/Ding.ogg` and `newwavelabs/MildlyAlarming.ogg`, which earlier packages install).  The tags of `newwavelabs` differ from some file names: `CurveBall.ogg` is Curve Ball Blend, `MidEvilJaunt.ogg` Medieval Jaunt, and `Noises1.ogg` to `Noises3.ogg` Noisey One, Terrible Twos, and Thriller Three.  Android gives some titles to two sounds, and the newer sound keeps the title:

- The sounds of `ringtones/material/ogg`, which took the place of Android 4.4's sounds of the same titles in Android 5.0, keep the plain title: Atria, Callisto, Dione, Ganymede, Luna, Oberon, Phobos, Pyxis, Sedna, Titania, Triton, and Umbriel.
- An older sound takes another title that Android gives it: Digital Phone, the title of `Ring_Digital_02.ogg` (Android 1), for the recording that Android 2.3 to 4.3 install as Pyxis and Android 4.4 as Sedna.
- An older sound without another title adds "Classic": Android 4.4's Callisto, Dione (Atria in Android 4.2 and 4.3), Ganymede, Luna, Oberon, Phobos, Triton, and Umbriel, and Android 2.3's Lyra, Sceptrum, Solarium, and Ursa Minor (in `ringtones`), whose titles newer sounds of `ringtones/ogg` have.  No release installs the newer Lyra, and the newer Ursa Minor is tagged "UrsaMinor".

The list has the default, Flutey Phone, first, and the others in alphabetical order.

## The ringtones

| Ringtone | File | Upstream file in `data/sounds` (git blob) | Upstream title | Seconds kept | Gain | Pause |
| --- | --- | --- | --- | --- | --- | --- |
| Flutey Phone | `flutey-phone.wav` | `Ring_Synth_04.ogg` (`ec2d9ac01faf57cbec8405dc1e98c37b92ebebc8`) | Flutey Phone | 0.03 to 1.74 | +2 dB | 1973 ms |
| Acheron | `acheron.wav` | `ringtones/ogg/Acheron.ogg` (`67d73884648eada23729c84fc7fb43a4fb3e7f4a`) | Acheron | 0 to 7.08 | -3.5 dB | 634 ms |
| Andromeda | `andromeda.wav` | `ringtones/ogg/Andromeda.ogg` (`26c230e134ee0d7bdd8e42b9abd2867412cb885c`) | Andromeda | 0 to 1.65 | -4 dB | 1350 ms |
| Aquila | `aquila.wav` | `ringtones/ogg/Aquila.ogg` (`9eab4ffb4766a8c871da15221682c5be3975618a`) | Aquila | 0 to 1.25 | -3.5 dB | 1750 ms |
| Argo Navis | `argo-navis.wav` | `ringtones/ogg/ArgoNavis.ogg` (`d25b5e89c74af78225ec5c927adc8f821923e091`) | Argo Navis | 0 to 12 | +1.5 dB | 1328 ms |
| Atria | `atria.wav` | `ringtones/material/ogg/Atria.ogg` (`173f73d201769dcaf9ae8fc26c206a577669c62c`) | Atria | 0 to the end | -8 dB | none |
| Backroad | `backroad.wav` | `newwavelabs/Backroad.ogg` (`e1fb8064c15ca6d2c0e958781c78fbeac6ae46f4`) | Backroad | 0 to the end | -3.5 dB | none |
| Beat Plucker | `beat-plucker.wav` | `newwavelabs/BeatPlucker.ogg` (`f1bcef0c5d2655891312c4ef7cf7fb4453f1fb35`) | Beat Plucker | 0 to the end | -3 dB | none |
| Bell Phone | `bell-phone.wav` | `Ring_Classic_02.ogg` (`aeb2d5b21ed6b20b512996e5c1c6a474f3cc6a57`) | Bell Phone | 0 to 3.64 | +5.5 dB | 1454 ms |
| Bentley Dubs | `bentley-dubs.wav` | `newwavelabs/BentleyDubs.ogg` (`44c0631858757eee94d35980ba7ff00bd3ebb187`) | Bentley Dubs | 0 to 4.4 | -5 dB | 44 ms |
| Big Easy | `big-easy.wav` | `newwavelabs/Big_Easy.ogg` (`277dd96c67619c3a8bdebbb7712f7358af60f088`) | Big Easy | 0 to the end | -2 dB | none |
| Bird Loop | `bird-loop.wav` | `newwavelabs/BirdLoop.ogg` (`b484582f51045c815300f2f38015e02fe5f6ea6b`) | Bird Loop | 0 to the end | -1 dB | none |
| Bollywood | `bollywood.wav` | `newwavelabs/Bollywood.ogg` (`b393dd4f6d3aaf5c8d5c2d0e464170fce22fa547`) | Bollywood | 0 to the end | -3 dB | none |
| Boötes | `bootes.wav` | `ringtones/ogg/Bootes.ogg` (`46342940ff086d7093ebc304ccd5290be2443058`) | Boötes | 0 to the end | +0.5 dB | none |
| Bus' a Move | `bus-a-move.wav` | `newwavelabs/BussaMove.ogg` (`7c9f7b5e9441aa13bf87ef40e201840b2f3e0a23`) | Bus' a Move | 0 to the end | -6 dB | none |
| Cairo | `cairo.wav` | `newwavelabs/Cairo.ogg` (`d90d3b42130dc98315d4598bbf00908553e9a177`) | Cairo | 0 to the end | -1 dB | none |
| Callisto | `callisto.wav` | `ringtones/material/ogg/Callisto.ogg` (`dd9bc5b26ad2e566422a305a2909a07791e475ea`) | Callisto | 0 to the end | -5 dB | none |
| Callisto Classic | `callisto-classic.wav` | `ringtones/ogg/Callisto.ogg` (`9c982d80752b18334980a4715c11ef8810214fe2`) | Callisto | 0 to 7.112 (first of 2 equal passes) | -3.5 dB | none |
| Calypso Steel | `calypso-steel.wav` | `newwavelabs/Calypso_Steel.ogg` (`8e07da130244bb28d03b8b6c1c3ddc890f8ec3ee`) | Calypso Steel | 0 to the end | +0.5 dB | none |
| Canis Major | `canis-major.wav` | `ringtones/ogg/CanisMajor.ogg` (`0e575e06b4dd0b647ff79cbe98a56ff4fd69b7af`) | Canis Major | 0 to the end | +1.5 dB | none |
| Caribbean Ice | `caribbean-ice.wav` | `newwavelabs/CaribbeanIce.ogg` (`75251236eeba59a12cc4e9801948cc24876aa262`) | Caribbean Ice | 0 to the end | -1.5 dB | none |
| Carina | `carina.wav` | `ringtones/ogg/Carina.ogg` (`e8a66b463b41ac327856177a507fbead0d25d4fb`) | Carina | 0 to 1.5 | -1 dB | 1501 ms |
| Cassiopeia | `cassiopeia.wav` | `ringtones/CASSIOPEIA.ogg` (`c4a7948364b2e01dc6fe71fcc00cba69d66ab385`) | Cassiopeia | 0.04 to 3.28 | -3.5 dB | 760 ms |
| Centaurus | `centaurus.wav` | `ringtones/ogg/Centaurus.ogg` (`79a393c3b3295bf9294ba9be55443677d4493d47`) | Centaurus | 0 to 3.71 | -4 dB | 290 ms |
| Champagne Edition | `champagne-edition.wav` | `newwavelabs/Champagne_Edition.ogg` (`0ba8be5e6a0f8e4a2b51df4992fa8188f818b835`) | Champagne Edition | 0 to the end | -2 dB | none |
| Chimey Phone | `chimey-phone.wav` | `Ring_Synth_02.ogg` (`4fc192080e7df0f5a03221e0086ccf9aaa6b5055`) | Chimey Phone | 0.03 to 2.67 | +4 dB | 1514 ms |
| Classic 1 | `classic-1.wav` | `Ring_Classic_01.ogg` (`b5e6077cc8fcb3245d57567e7ce37f520aac7d36`) | none | 0 to 3.57 | +6 dB | 662 ms |
| Classic 3 | `classic-3.wav` | `Ring_Classic_03.ogg` (`2b14eb4b84952b1030238c86b2eba8576a8be244`) | none | 0 to 1.75 | +2 dB | 1437 ms |
| Classic 4 | `classic-4.wav` | `Ring_Classic_04.ogg` (`7474ac542796cd1bb744b31216d90ffa40811392`) | none | 0 to 1.97 | +11 dB | 1374 ms |
| Classic 5 | `classic-5.wav` | `Ring_Classic_05.ogg` (`b9ef5b10c92e27d390eb1c364c2b97d7590fcf26`) | none | 0 to 0.891 | -7.5 dB | 2061 ms |
| Club Cubano | `club-cubano.wav` | `newwavelabs/Club_Cubano.ogg` (`5d09db1e56d647bc493b04c4736f7ab32c822626`) | Club Cubano | 0 to the end | -4.5 dB | none |
| Crayon Rock | `crayon-rock.wav` | `newwavelabs/CrayonRock.ogg` (`6827aa5ba7e6bef90a58f86d61373d7fdcefda26`) | Crayon Rock | 0 to the end | -5 dB | none |
| Crazy Dream | `crazy-dream.wav` | `newwavelabs/CrazyDream.ogg` (`5fa47b917b025f2e4be4622f5d0d3996958672a1`) | Crazy Dream | 0 to the end | -8.5 dB | none |
| Curve Ball Blend | `curve-ball-blend.wav` | `newwavelabs/CurveBall.ogg` (`733a004d62547d710459b2cff899a8f20e95f771`) | Curve Ball Blend | 0 to the end | -6 dB | none |
| Cygnus | `cygnus.wav` | `ringtones/ogg/Cygnus.ogg` (`80317ef18caeef149603b57cc2df109b18925266`) | Cygnus | 0 to 3.26 | -1.5 dB | 740 ms |
| Dancin' Fool | `dancin-fool.wav` | `newwavelabs/DancinFool.ogg` (`39e3b8ab9a6119f432306af7a11c0c6fa4600809`) | Dancin' Fool | 0 to the end | -3 dB | none |
| Digital 1 | `digital-1.wav` | `Ring_Digital_01.ogg` (`39d2bb86e5915db39a0f52bc0d1710c2ae6cd253`) | none | 0 to 1.59 | -1.5 dB | 2381 ms |
| Digital 3 | `digital-3.wav` | `Ring_Digital_03.ogg` (`2b70ee886d7778f4893099a84fd7774eba6f450f`) | none | 0 to 1.301 | +0.5 dB | 2461 ms |
| Digital 4 | `digital-4.wav` | `Ring_Digital_04.ogg` (`d2ca48680ffcb32bf53d5e947cdc6e2d6ce192d3`) | none | 0.03 to 1.65 | -0.5 dB | 2481 ms |
| Digital 5 | `digital-5.wav` | `Ring_Digital_05.ogg` (`efde4978978742a1614e44511bc6bb0a86732779`) | none | 0 to 4.09 | +0.5 dB | 37 ms |
| Digital Phone | `digital-phone.wav` | `Ring_Digital_02.ogg` (`f823739047def0c2757d9ab5f8cdadfc39482cc2`) | Digital Phone | 0 to 1.21 | +1 dB | 2499 ms |
| Dione | `dione.wav` | `ringtones/material/ogg/Dione.ogg` (`cf7d1cb90417bc0b696d545e591fe8c8c46999c8`) | Dione | 0 to 11.38 | -5 dB | 620 ms |
| Dione Classic | `dione-classic.wav` | `ringtones/ogg/Dione.ogg` (`a378520f588f3faab1e08f40043beaaeaade9bdc`) | Dione | 0 to 12.82 | 0 dB | 680 ms |
| Don' Mess Wiv It | `don-mess-wiv-it.wav` | `newwavelabs/DonMessWivIt.ogg` (`5b14ba5dc05c33832a57f5f674805240e3f23628`) | Don' Mess Wiv It | 0 to the end | -4.5 dB | none |
| Draco | `draco.wav` | `ringtones/ogg/Draco.ogg` (`d09b8917766a6d993f713de888d4568b313db6bc`) | Draco | 0 to 1.85 | -2.5 dB | 2150 ms |
| Dream Theme | `dream-theme.wav` | `newwavelabs/DreamTheme.ogg` (`e602a38f46d2531da605753a3f8295d0c00d0487`) | Dream Theme | 0 to the end | +2 dB | none |
| Eastern Sky | `eastern-sky.wav` | `newwavelabs/Eastern_Sky.ogg` (`c0cf74c438e3ed88cbb7aff57972d6db411d71ea`) | Eastern Sky | 0 to the end | -3.5 dB | none |
| Enter the Nexus | `enter-the-nexus.wav` | `newwavelabs/Enter_the_Nexus.ogg` (`ba304541d4e2870bdaf360c399fb5ac72ee7f87b`) | Enter the Nexus | 0 to the end | -2 dB | none |
| Eridani | `eridani.wav` | `ringtones/ogg/Eridani.ogg` (`8d8a1f0ec516b02602cbcb902fca5019557733f4`) | Eridani | 0 to 4 | +3.5 dB | 1000 ms |
| Ether Shake | `ether-shake.wav` | `newwavelabs/EtherShake.ogg` (`178aa21fc1bd4c61672a80f413b14f7c581e4d88`) | Ether Shake | 0 to the end | 0 dB | none |
| Free Flight | `free-flight.wav` | `ringtones/FreeFlight.ogg` (`76dfabe619edc6394104b2f13d2d6102c3b42576`) | Free Flight | 0 to the end | +0.5 dB | none |
| Friendly Ghost | `friendly-ghost.wav` | `newwavelabs/FriendlyGhost.ogg` (`d99388097a9338364f2dd4b1555fdaf19a7082b6`) | Friendly Ghost | 0 to the end | -5.5 dB | none |
| Funk Y'all | `funk-yall.wav` | `newwavelabs/Funk_Yall.ogg` (`5b98643cec9e8d76e73c294d6dc804267a98d0a6`) | Funk Y'all | 0 to the end | -3.5 dB | none |
| Game Over Guitar | `game-over-guitar.wav` | `newwavelabs/GameOverGuitar.ogg` (`2adb0a87ace860406a1d6d13ef45011845570289`) | Game Over Guitar | 0 to the end | 0 dB | none |
| Ganymede | `ganymede.wav` | `ringtones/material/ogg/Ganymede.ogg` (`e7ff9f4189125d2dec59c071a72ed239956442de`) | Ganymede | 0 to 4.92 | -4.5 dB | 61 ms |
| Ganymede Classic | `ganymede-classic.wav` | `ringtones/ogg/Ganymede.ogg` (`01ff573aef5453508046c87c280cef0dee619dae`) | Ganymede | 0 to 9.14 | +3 dB | 259 ms |
| Gimme Mo' Town | `gimme-mo-town.wav` | `newwavelabs/Gimme_Mo_Town.ogg` (`a1969358fd6ddc093fc9ab877337798107847bb5`) | Gimme Mo' Town | 0 to the end | -2.5 dB | none |
| Girtab | `girtab.wav` | `ringtones/ogg/Girtab.ogg` (`f6b3a35a38597365f55c380e3e2e02cd9241aee2`) | Girtab | 0 to 2.52 | -4.5 dB | 930 ms |
| Glacial Groove | `glacial-groove.wav` | `newwavelabs/Glacial_Groove.ogg` (`a0e83e60cc3e2bcd416622574585197c93d0cc49`) | Glacial Groove | 0 to the end | -2 dB | none |
| Growl | `growl.wav` | `newwavelabs/Growl.ogg` (`c54bde0d2182440cf2893dfb52d39ecd775aedfe`) | Growl | 0 to the end | -8.5 dB | none |
| Halfway Home | `halfway-home.wav` | `newwavelabs/HalfwayHome.ogg` (`4ece7cab10d58439653636385084a1084f186563`) | Halfway Home | 0 to the end | -1.5 dB | none |
| Hydra | `hydra.wav` | `ringtones/ogg/Hydra.ogg` (`935c12f40c51259fbaa6728bc31f1d8ea6fc0a2d`) | Hydra | 0 to 2.95 | -4.5 dB | 2050 ms |
| Insert Coin | `insert-coin.wav` | `newwavelabs/InsertCoin.ogg` (`8737da5584744666e596f19c3b43d5ea9e5e97b7`) | Insert Coin | 0 to 1.75 | -2.5 dB | 250 ms |
| Iridium | `iridium.wav` | `ringtones/ogg/Iridium.ogg` (`3de61fb95f28260dfbf755bff1bcfded4f37f171`) | Iridium | 0 to 2.8 | -3.5 dB | 500 ms |
| Kuma | `kuma.wav` | `ringtones/ogg/Kuma.ogg` (`780d5d45c3158c92585aaae8be221ea079f579f4`) | Kuma | 0 to the end | 0 dB | none |
| Loopy Lounge | `loopy-lounge.wav` | `newwavelabs/LoopyLounge.ogg` (`6d168cd049f1befde231a476f0dfe76ded782d66`) | Loopy Lounge | 0 to the end | -0.5 dB | none |
| Love Flute | `love-flute.wav` | `newwavelabs/LoveFlute.ogg` (`16cf413fe84583de76258737d76fc884b5bd94e6`) | Love Flute | 0 to the end | -6 dB | none |
| Luna | `luna.wav` | `ringtones/material/ogg/Luna.ogg` (`8aeb3a036597a9024f5721d62497f2887ba4725d`) | Luna | 0 to 3.3 | -5.5 dB | 1556 ms |
| Luna Classic | `luna-classic.wav` | `ringtones/ogg/Luna.ogg` (`87a2463902ea0b7cc8709d57e0fcb359284ad4dd`) | Luna | 0 to the end | -1.5 dB | none |
| Lyra | `lyra.wav` | `ringtones/ogg/Lyra.ogg` (`d170bc07202340c51e98eb5022e07750afb24e6a`) | none | 0 to 2.99 | -8.5 dB | 1210 ms |
| Lyra Classic | `lyra-classic.wav` | `ringtones/Lyra.ogg` (`696f278b62dea6402619a8137c27af17fa413a1f`) | Lyra | 0 to 5.15 | +4 dB | 526 ms |
| Machina | `machina.wav` | `ringtones/ogg/Machina.ogg` (`33388e6ed18292a038dd45d83ed3423c9eef3815`) | Machina | 0 to 4.12 | -9 dB | 1005 ms |
| Medieval Jaunt | `medieval-jaunt.wav` | `newwavelabs/MidEvilJaunt.ogg` (`66d7fad4018840240322cf7195aa27c15180900f`) | Medieval Jaunt | 0 to the end | -3.5 dB | none |
| Nairobi | `nairobi.wav` | `newwavelabs/Nairobi.ogg` (`21929c91051d91bb366f8b64b6d457b528bfc067`) | Nairobi | 0 to the end | +4 dB | none |
| Nasqueron | `nasqueron.wav` | `ringtones/ogg/Nasqueron.ogg` (`ae1a7258529a7e57955164663f214eba7528033f`) | Nasqueron | 0 to 8.5 | -5 dB | 1100 ms |
| Nassau | `nassau.wav` | `newwavelabs/Nassau.ogg` (`6615b736280b37f3e7443c90a264647857d1fb52`) | Nassau | 0 to the end | -3.5 dB | none |
| New Player | `new-player.wav` | `newwavelabs/NewPlayer.ogg` (`c2df5d8a0f00c4b4405cec61b86a5d3f5967462c`) | New Player | 0 to 1.65 | +1.5 dB | 350 ms |
| No Limits | `no-limits.wav` | `newwavelabs/No_Limits.ogg` (`346f2fe7c3e1a278f5cf1aceca73727338deafdc`) | No Limits | 0 to the end | -2 dB | none |
| Noisey One | `noisey-one.wav` | `newwavelabs/Noises1.ogg` (`829b4fac0a0115030bbd7ce2a4e6df7e25761b50`) | Noisey One | 0 to the end | +1.5 dB | none |
| Oberon | `oberon.wav` | `ringtones/material/ogg/Oberon.ogg` (`145f4746ed27be7d97ec4e5a5d8c6054aa748dd9`) | Oberon | 0 to 15.93 | -5 dB | 70 ms |
| Oberon Classic | `oberon-classic.wav` | `ringtones/ogg/Oberon.ogg` (`124af10019d903e723c97de3b2fa3bb9d138428d`) | Oberon | 0 to 6.81 | -3.5 dB | 107 ms |
| Organ Dub | `organ-dub.wav` | `newwavelabs/OrganDub.ogg` (`c0a22f43d50da8d77c6a418938a188a91c5938cb`) | Organ Dub | 0 to the end | -2 dB | none |
| Orion | `orion.wav` | `ringtones/ogg/Orion.ogg` (`0323e4d811a94479c94152155ff1b1422cf77bc2`) | Orion | 0 to the end | -3 dB | none |
| Paradise Island | `paradise-island.wav` | `newwavelabs/Paradise_Island.ogg` (`386bbd3f81b64a72f4d6767eeca672e32aa9d184`) | Paradise Island | 0 to the end | -2 dB | none |
| Pegasus | `pegasus.wav` | `ringtones/ogg/Pegasus.ogg` (`d1bf4b01d64ec539e6af29de3049f733ddb65b55`) | Pegasus | 0 to the end | -1.5 dB | none |
| Perseus | `perseus.wav` | `ringtones/ogg/Perseus.ogg` (`48348e5bc97a9d176cb0212fbf78c30afa417d3f`) | Perseus | 0 to the end | -3 dB | none |
| Phobos | `phobos.wav` | `ringtones/material/ogg/Phobos.ogg` (`5e312d20cbc6f8aeebf195fae485c606218d8c50`) | Phobos | 0 to 2.18 | -3.5 dB | 1845 ms |
| Phobos Classic | `phobos-classic.wav` | `ringtones/ogg/Phobos.ogg` (`768ca9498f41a43cdb82ea05c3e054e05fbe5052`) | Phobos | 0 to 3.614 (first of 2 equal passes) | -3 dB | none |
| Playa | `playa.wav` | `newwavelabs/Playa.ogg` (`747b6995fb28dd842bb7a8c528f0303552a7d721`) | Playa | 0 to the end | -4.5 dB | none |
| Pyxis | `pyxis.wav` | `ringtones/material/ogg/Pyxis.ogg` (`4786b18a193591872ca784774cbf42256b3f4587`) | Pyxis | 0 to the end | -3.5 dB | none |
| Rasalas | `rasalas.wav` | `ringtones/ogg/Rasalas.ogg` (`7e62f17eef4fe82b403ae6c68c32010af2717222`) | Rasalas | 0 to 25.29 | -2 dB | 235 ms |
| Revelation | `revelation.wav` | `newwavelabs/Revelation.ogg` (`4a240e98b5572cf9a324da021dc28ff7538f8638`) | Revelation | 0 to the end | -1 dB | none |
| Rigel | `rigel.wav` | `ringtones/ogg/Rigel.ogg` (`de3d92f57b0a85f45f120fb4e686cc5d72388af7`) | Rigel | 0 to 3.13 | -2.5 dB | 304 ms |
| Ringtone 18 | `ringtone-18.wav` | `ringtones/ringtone18.ogg` (`00cd28f4b0e5f48d7b2076ff14a26b0ece63537b`) | none | 0 to 13.11 | -1 dB | 557 ms |
| Ringtone 19 | `ringtone-19.wav` | `ringtones/ringtone19.ogg` (`02acaa78a26a0d993acd92d1ade1111dd4cadbbe`) | none | 0 to 6.47 | -3 dB | 1130 ms |
| Ringtone 20 | `ringtone-20.wav` | `ringtones/ringtone20.ogg` (`e6d1051edb5e4411ad8eb6aaa401ba7e9a496ba1`) | none | 0 to 3.93 | -5 dB | 1170 ms |
| Ringtone 21 | `ringtone-21.wav` | `ringtones/ringtone21.ogg` (`f8ef5414121a155b74a051e18e287f3e8f0743a2`) | none | 0 to 11.06 | -10.5 dB | 940 ms |
| Ringtone 23 | `ringtone-23.wav` | `ringtones/ringtone23.ogg` (`3b4f000b25b297461f05eb3c2cc6796f3c57c494`) | none | 0 to 10.25 | -2.5 dB | 1750 ms |
| Ringtone 24 | `ringtone-24.wav` | `ringtones/ringtone24.ogg` (`e1c5c3b31d46dd0004c42270f6c5dd4ff84287cc`) | none | 0 to 14.86 | -5 dB | 1375 ms |
| Ringtone 25 | `ringtone-25.wav` | `ringtones/ringtone25.ogg` (`0f66463e31d7c648aad3832df0ffd75233550182`) | none | 0 to 10.65 | -4.5 dB | 889 ms |
| Ringtone 26 | `ringtone-26.wav` | `ringtones/ringtone26.ogg` (`8cd4648f55f9e5d5f877a7dcfc73f30ecc6969ef`) | none | 0 to 5.55 | -3.5 dB | 879 ms |
| Ringtone 28 | `ringtone-28.wav` | `ringtones/ringtone28.ogg` (`6139667c572de1d28fce20aa5d96f5e5fc0b9688`) | none | 0 to 12.64 | -8.5 dB | 693 ms |
| Ringtone 29 | `ringtone-29.wav` | `ringtones/ringtone29.ogg` (`042c82a48f089fa090003229029e19696c194b24`) | none | 0 to 12.62 | -5 dB | 1780 ms |
| Ringtone 30 | `ringtone-30.wav` | `ringtones/ringtone30.ogg` (`eec8ebc0f0db076c67dbbda4fb451fef51bcebbc`) | none | 0 to 9.92 | -4 dB | 1157 ms |
| Ringtone 31 | `ringtone-31.wav` | `ringtones/ringtone31.ogg` (`0140a806677cdeb5cd5adb6b3db1ec2bbacb6f0f`) | none | 0 to 6.15 | -4.5 dB | 773 ms |
| Ringtone 32 | `ringtone-32.wav` | `ringtones/ringtone32.ogg` (`4e12f5a8f78107566c8354aede426173006c948a`) | none | 0 to 9.49 | -0.5 dB | 664 ms |
| Ringtone 33 | `ringtone-33.wav` | `ringtones/ringtone33.ogg` (`1342066dcee464e9f01cd6f881d0355a81a6a0e1`) | none | 0 to 3.97 | -10.5 dB | 645 ms |
| Ringtone 34 | `ringtone-34.wav` | `ringtones/ringtone34.ogg` (`a16737b1abc8b7369aae3135ada0c33153fab7dd`) | none | 0 to 3.32 | -0.5 dB | 834 ms |
| Ringtone 35 | `ringtone-35.wav` | `ringtones/ringtone35.ogg` (`3407857a57440da9281ab74c1229cf62458ca696`) | none | 0 to 11.64 | -9.5 dB | 992 ms |
| Ringtone 36 | `ringtone-36.wav` | `ringtones/ringtone36.ogg` (`bbd526340fa29a9d4c4a7d9f16c0fdea9a30da8f`) | none | 0 to the end | -6 dB | none |
| Ringtone 37 | `ringtone-37.wav` | `ringtones/ringtone37.ogg` (`ae7b57ee3e0f1b577a7225e569359caccbda83df`) | none | 0 to 9.99 | -4 dB | 697 ms |
| Ringtone 38 | `ringtone-38.wav` | `ringtones/ringtone38.ogg` (`fae20cde3bdb949d23ca17d6fe32158a2d919c94`) | none | 0 to 8.96 | -2 dB | 940 ms |
| Road Trip | `road-trip.wav` | `newwavelabs/Road_Trip.ogg` (`cd14ef13e145e087479ee6e782e93ff1cb0c58d1`) | Road Trip | 0 to the end | -4 dB | none |
| Romancing The Tone | `romancing-the-tone.wav` | `newwavelabs/RomancingTheTone.ogg` (`ebe54e555c966b68a629dca439a341290882f046`) | Romancing The Tone | 0 to the end | -1.5 dB | none |
| Safari | `safari.wav` | `newwavelabs/Safari.ogg` (`fa2d4ed84525fe5fd78700e0ca8d6f8608082432`) | Safari | 0 to the end | -3 dB | none |
| Savannah | `savannah.wav` | `newwavelabs/Savannah.ogg` (`779f3ddc8792b8aaa2cfbdf7abc9ec484b9ec1be`) | Savannah | 0 to the end | -2 dB | none |
| Scarabaeus | `scarabaeus.wav` | `ringtones/ogg/Scarabaeus.ogg` (`3dcd0d7e8c274d69b54e38283d752d9bb59c12f8`) | Scarabaeus | 0.11 to 8.81 | -5 dB | 531 ms |
| Sceptrum | `sceptrum.wav` | `ringtones/ogg/Sceptrum.ogg` (`f2d3ed445c21eace9d3e6b09473e92ffe8991f0f`) | Sceptrum | 0 to 29.7 | -4 dB | 300 ms |
| Sceptrum Classic | `sceptrum-classic.wav` | `ringtones/Sceptrum.ogg` (`fc50aef575e4043d076baa8b706bffe55b6f6635`) | Sceptrum | 0.03 to 21.85 | -3 dB | 708 ms |
| Sedna | `sedna.wav` | `ringtones/material/ogg/Sedna.ogg` (`ff889e60882d4c21553408620bf019abbb779cc5`) | Sedna | 0 to 6.74 | -4 dB | 117 ms |
| Seville | `seville.wav` | `newwavelabs/Seville.ogg` (`bea926a53969fdd65026df0e3514391e5eab53aa`) | Seville | 0 to the end | -2 dB | none |
| She's All That | `shes-all-that.wav` | `newwavelabs/Shes_All_That.ogg` (`86afbb3983bf5f1d6a455dc029ce373c3a5a0869`) | She's All That | 0 to the end | -2.5 dB | none |
| Silky Way | `silky-way.wav` | `newwavelabs/SilkyWay.ogg` (`e5a38ec05c0f0e56aee11ec762b950069e56ec7e`) | Silky Way | 0 to the end | -1.5 dB | none |
| Sitar Versus Sitar | `sitar-versus-sitar.wav` | `newwavelabs/SitarVsSitar.ogg` (`cfe9c8043153a08536600773a878fa9edabcf34c`) | Sitar Versus Sitar | 0 to the end | +0.5 dB | none |
| Solarium | `solarium.wav` | `ringtones/ogg/Solarium.ogg` (`8c9d3073b8ce1557ed1c328d3f314b2b6737cf4a`) | Solarium | 0 to 8.11 | -4 dB | 804 ms |
| Solarium Classic | `solarium-classic.wav` | `ringtones/Solarium.ogg` (`d27f141dd9304a44accfa2742e30c40a16ef9dd2`) | Solarium | 0 to 6.82 | -5 dB | 1075 ms |
| Springy Jalopy | `springy-jalopy.wav` | `newwavelabs/SpringyJalopy.ogg` (`b837affb75b509801bb91bb8e95cc80b69a50372`) | Springy Jalopy | 0 to the end | +7 dB | none |
| Steppin' Out | `steppin-out.wav` | `newwavelabs/Steppin_Out.ogg` (`5ee5fd18e44d97f7f4ec8daa2bdd09f7b146a445`) | Steppin' Out | 0 to the end | -2 dB | none |
| Synth 1 | `synth-1.wav` | `Ring_Synth_01.ogg` (`ec2e628528dd31370bb169b1b32186c7d9807c6f`) | none | 0 to 1.66 | +0.5 dB | 1292 ms |
| Synth 3 | `synth-3.wav` | `Ring_Synth_03.ogg` (`bb45ecb5e7546b978c950a9a480541ab9f07895d`) | none | 0 to 1.92 | -0.5 dB | 2155 ms |
| Synth 5 | `synth-5.wav` | `Ring_Synth_05.ogg` (`674434ea22a68b993c462652000a44377a7451b5`) | none | 0 to 3.11 | +3.5 dB | 51 ms |
| Terminated | `terminated.wav` | `newwavelabs/Terminated.ogg` (`b98005397015bf2075d92648c06e82864479563f`) | Terminated | 0 to the end | -1 dB | none |
| Terrible Twos | `terrible-twos.wav` | `newwavelabs/Noises2.ogg` (`69a3d7296124a3c3811c726bf6ee1ec1db1dd2f9`) | Terrible Twos | 0 to the end | -2 dB | none |
| Testudo | `testudo.wav` | `ringtones/ogg/Testudo.ogg` (`42db3a59a923c6d6b071a52dc5b873ef38749cab`) | Testudo | 0 to 7.22 | -11 dB | 416 ms |
| Third Eye | `third-eye.wav` | `newwavelabs/Third_Eye.ogg` (`dd5a9fdffeac43dfef07782930af2a774458a004`) | Third Eye | 0 to the end | +3 dB | none |
| Thriller Three | `thriller-three.wav` | `newwavelabs/Noises3.ogg` (`3607c2a1d53b10a1f25d784e98f2e962b727adb3`) | Thriller Three | 0 to the end | +1 dB | none |
| Thunderfoot | `thunderfoot.wav` | `newwavelabs/Thunderfoot.ogg` (`5f907a80570f4df4f8b610880448ceda4043f3ea`) | Thunderfoot | 0 to the end | -3 dB | none |
| Titania | `titania.wav` | `ringtones/material/ogg/Titania.ogg` (`87a05eef57e01ef9816bb8aa652dd1eb4eef191d`) | Titania | 0 to the end | -7 dB | none |
| Triton | `triton.wav` | `ringtones/material/ogg/Triton.ogg` (`b5893bde3eb0e0a7eef71a9d540d0c014ad45841`) | Triton | 0 to 14.16 | -6 dB | 533 ms |
| Triton Classic | `triton-classic.wav` | `ringtones/ogg/Triton.ogg` (`542ae1624e00bdd6184149331fe9994fa0035cdc`) | Triton | 0 to the end | -3 dB | none |
| Twirl Away | `twirl-away.wav` | `newwavelabs/TwirlAway.ogg` (`eeff7f0767a2ac7ffe3e84b23b72ac2ce4744714`) | Twirl Away | 0 to the end | -1.5 dB | none |
| Umbriel | `umbriel.wav` | `ringtones/material/ogg/Umbriel.ogg` (`193e566f8cc5db98cc421fbbf361cab044beda2a`) | Umbriel | 0 to 7.1 | -5 dB | 1117 ms |
| Umbriel Classic | `umbriel-classic.wav` | `ringtones/ogg/Umbriel.ogg` (`b21a12e0778a834e492afc8c20b18654a36e5998`) | Umbriel | 0 to the end | -3 dB | none |
| Ursa Minor | `ursa-minor.wav` | `ringtones/ogg/UrsaMinor.ogg` (`30d2cd1d1763b3ff4e5143542cc08f9f906168bd`) | UrsaMinor | 0 to 12.75 | -4 dB | 50 ms |
| Ursa Minor Classic | `ursa-minor-classic.wav` | `ringtones/URSAMINOR.ogg` (`c0010e821fe9cadd027511794b542bab305fa816`) | Ursa Minor | 0 to the end | -5 dB | none |
| Very Alarmed | `very-alarmed.wav` | `newwavelabs/VeryAlarmed.ogg` (`bfb63f48c7c35ca4131280555f494553ee789ca3`) | Very Alarmed | 0 to the end | -2 dB | none |
| Vespa | `vespa.wav` | `ringtones/Vespa.ogg` (`4423bbb6a5acff01bc7ac4eafccd018421d54800`) | Vespa | 0 to 4.74 | -2 dB | 2117 ms |
| World | `world.wav` | `newwavelabs/World.ogg` (`e226166f3c0e76d0080d602f4f863aefe4dae0b2`) | World | 0 to the end | -2 dB | none |
| Zeta | `zeta.wav` | `ringtones/ogg/Zeta.ogg` (`ef64d2feda7e7f484f6d00c355a7c11e6aa5a343`) | Zeta | 0 to 5.41 | -7.5 dB | 1190 ms |

`derive.sh` holds the commands that made each file from the upstream file (ffmpeg 9.0.2), the same as for the alarm tones (`assets/alarm-tones/README.md`): to 16 kHz mono, cut to the sound of one pass, changed in level so that the ringtones are about equally loud, and stored as 8-bit mu-law, which costs 16 KB per second.  A pass ends where the sound falls 50 dB below its loudest 10 ms.  The silence before and after the sound upstream is not stored; the phone leaves it as the pause between two passes, so that a ringtone repeats at the period of its upstream file looped.  Where the upstream file holds two equal passes (Callisto Classic and Phobos Classic), one is kept.  A ringtone without a pause runs into its next pass as its upstream file does when it loops.  So Love Flute and Revelation click where they loop: their upstream files end far from where they begin.  The level brings the samples above a twentieth of the peak to an RMS of 5,500 (-15.5 dBFS), or less where the peak would pass 26,000.  Stereo files are mixed to mono.

The 149 files are 15,775,320 bytes; in the image's compressed root file system, with `NOTICE` and the license text, they take about 12.9 MB.

A raw file is at `https://android.googlesource.com/platform/frameworks/base/+/1cdfff555f4a21f71ccc978290e2e212e2f8b168/data/sounds/<file>?format=TEXT`, in Base64.

## Files not used

The other 62 files hold sounds already in the table, each compared with the file used by its samples decoded at 16 kHz mono.  "Same samples" means that the two decode to identical samples.  "Same recording" means a waveform correlation of 0.99 or more: another encoding of the same audio, at most at another level, sample rate, or number of channels.  "Another mix" means the same notes at the same times, with a waveform correlation from 0.40 to 0.96, log spectrograms that correlate at 0.84 or more, and pitch-class profiles that correlate at 0.71 or more: a separate mix or master of the same recording.  The distinct sounds that share a title measure at most 0.13 and 0.39 on the first and last of these.

| Ringtone | Files not used |
| --- | --- |
| Andromeda, Aquila, Boötes, Canis Major, Centaurus, Cygnus, Draco, Eridani, Hydra, Machina, Orion, Pegasus, Scarabaeus, and Testudo | `ringtones/ANDROMEDA.ogg`, `ringtones/Aquila.ogg`, `ringtones/BOOTES.ogg`, `ringtones/CANISMAJOR.ogg`, `ringtones/Centaurus.ogg`, `ringtones/Cygnus.ogg`, `ringtones/Draco.ogg`, `ringtones/Eridani.ogg`, `ringtones/hydra.ogg`, `ringtones/Machina.ogg`, `ringtones/Orion.ogg`, `ringtones/Pegasus.ogg`, `ringtones/Scarabaeus.ogg`, and `ringtones/Testudo.ogg`: same samples as the file of the same name in `ringtones/ogg`. |
| Acheron | `ringtones/ringtone27.ogg`: same samples. |
| Argo Navis | `ringtones/ArgoNavis.ogg`: another mix (waveform 0.83). |
| Carina | `ringtones/Carina.ogg`: same samples.  `ringtones/Carina.wav` and `ringtones/wav/Carina.wav` (one blob), and `newwavelabs/Ding.ogg`: same recording, without the 1.5 s of silence after it.  (Noisey One, `newwavelabs/Noises1.ogg`, begins with the same 1.5 s and goes on for 2.5 s more, which no other file holds.) |
| Digital Phone | `ringtones/Pyxis.ogg`, `ringtones/ogg/Pyxis.ogg`, and `ringtones/ogg/Sedna.ogg` and `ringtones/ogg/Sedna_48k.ogg` (one blob): same samples as one another, and the same recording as `Ring_Digital_02.ogg`. |
| Dione Classic | `ringtones/ogg/Atria.ogg`: same samples.  `ringtones/ogg/Dione_48k.ogg`: the same blob. |
| Eridani | `newwavelabs/MildlyAlarming.ogg`: same recording, without the last second of silence. |
| Free Flight | `ringtones/FreeFlight.wav`: same recording. |
| Girtab | `ringtones/ogg/Cassiopeia.ogg`: same samples.  `ringtones/wav/Cassiopeia.wav`: same recording. |
| Lyra, Sceptrum, Solarium, and Ursa Minor | `ringtones/wav/Lyra.wav`, `ringtones/wav/Sceptrum.wav`, `ringtones/wav/Solarium.wav`, and `ringtones/wav/UrsaMinor.wav`: same recording. |
| Nasqueron | `ringtones/ringtone22.ogg`: another mix (waveform 0.40). |
| Perseus | `ringtones/PERSEUS.ogg`: another mix (waveform 0.96). |
| Rigel | `ringtones/Rigel.ogg`: same recording. |
| Titania | `ringtones/ogg/Titania.ogg` (Android 4.4's Titania) and `ringtones/ogg/Titania_48k.ogg`, and `ringtones/ogg/Themos.ogg` (Android 4.0 to 4.3's Themos) and `ringtones/wav/Themos.wav`: another mix (waveform 0.53 to 0.61). |
| Zeta | `ringtones/ogg/Vespa.ogg`: same samples.  `ringtones/wav/Vespa.wav`: same recording. |
| The ringtones of `ringtones/material/ogg` and Android 4.4's with a 48 kHz version | `X_48k.ogg` beside each of the 12 files of `ringtones/material/ogg` and beside `ringtones/ogg/X.ogg` for Callisto, Ganymede, Luna, Oberon, Phobos, Triton, and Umbriel: same recording, or another mix for Atria, Luna, Titania, and Triton of `ringtones/material/ogg` and Oberon and Umbriel of `ringtones/ogg` (waveform 0.63 to 0.93).  Android installs one or the other under the same name (`AudioPackage12_48.mk` and `AudioPackage13_48.mk`). |
