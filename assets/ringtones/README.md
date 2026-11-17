# Ringtones

The phone's built-in ringtones.  The image carries the `.wav` files, `NOTICE`, and `LICENSE-Apache-2.0.txt` in `/usr/share/dphone/ringtones/`; `crates/dphone/src/mainrole/subsystems/ringtones.rs` names the ringtones and the pause between two passes of each, and carries a copy of the default ringtone, which rings when a ringtone's file cannot be played.

All 90 are derived from the ringtones of the Android Open Source Project, repository <https://android.googlesource.com/platform/frameworks/base>, directory `data/sounds`, at commit `1cdfff555f4a21f71ccc978290e2e212e2f8b168`: every file in `data/sounds/ringtones` and every `Ring_*` file in `data/sounds`, 150 files that hold 90 distinct sounds.  Copyright The Android Open Source Project; Apache License, Version 2.0 (`LICENSE-Apache-2.0.txt`).  `NOTICE` carries the copyright notice, the statement of the changes, and the part of the upstream `NOTICE` file that applies; the license requires that it and the license text go wherever the ringtones go.

Android's ringtone build module, `frameworks_ringtones_sounds` in `data/sounds/Android.bp`, also installs 61 files of `data/sounds/newwavelabs`, ringtones that the G1's audio package (`OriginalAudio.mk`) and `AudioPackage2.mk` to `AudioPackage4.mk` install.  They are not among these.

## License

The build file of the directory, `data/sounds/Android.bp`, names the license for it: `default_applicable_licenses: ["Android-Apache-2.0"]`, in place of the repository's `frameworks_base_license`.  No directory below `data/sounds` has a build file, a license file, or a `METADATA` file of its own, so the declaration is the one for every file used here.  It takes effect through the directory's build modules, and its module `frameworks_ringtones_sounds` installs 41 of the 150 files.  Of the 90 sources, 31 are among those 41, and 4 more (Boötes, Dione Classic, Eridani, and Testudo) decode to the same samples as files among them.  The other 55 are in no module: the 11 untitled `Ring_*` files, the 19 `ringtoneNN` files, all 12 ringtones of `ringtones/material/ogg`, and Acheron, Callisto Classic, Ganymede Classic, Iridium, Luna Classic, Lyra, Nasqueron, Oberon Classic, Phobos Classic, Sceptrum Classic, Solarium Classic, Triton Classic, and Umbriel Classic.  For these, the directory's declaration and the repository's `MODULE_LICENSE_APACHE2` file are the license statements, and both name the Apache License 2.0.  The repository's `METADATA` file classifies the repository as a whole (`license_type: RECIPROCAL`) and names no license or file.  No statement in the repository puts any of the 150 files under another license.  Three files carry a copyright tag, `(c) Copyright 2009 Android Open Source Project`: `ringtones/Carina.ogg`, `ringtones/FreeFlight.ogg`, and `ringtones/FreeFlight.wav`.

## The default

Flutey Phone (`Ring_Synth_04.ogg`) is the default: the ringtone a phone rings with when nothing else is chosen, and for a name that is no ringtone.  It is the default of the Android Open Source Project's own builds: `target/product/full_base.mk` of <https://android.googlesource.com/platform/build> sets `ro.config.ringtone?=Ring_Synth_04.ogg` among the "Additional settings used in all AOSP builds" (at commit `045a3d6a3e359633a14853a5a5e1e4f2a11cbdae`).  It is also short: 1.7 s of sound and a 2 s pause, 27,418 bytes, so the program carries a copy of it at little cost.

## Names and order

A ringtone has the title that Android gives the sound: the file's `TITLE` tag, or, where the file has no tag, the file name without `Ring_` and with the number written plainly (`Ring_Classic_01.ogg` is Classic 1, `ringtone18.ogg` is Ringtone 18, and `ringtones/ogg/Lyra.ogg` is Lyra).  A sound in files of two titles has the title of the file that Android installs: Girtab and Zeta (`ringtones/ogg/Girtab.ogg` and `ringtones/ogg/Zeta.ogg`, which Android 4.0 to 4.3 install), not Cassiopeia and Vespa (`ringtones/ogg/Cassiopeia.ogg` and `ringtones/ogg/Vespa.ogg`, which no release installs).  Android gives some titles to two sounds, and the newer sound keeps the title:

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
| Bell Phone | `bell-phone.wav` | `Ring_Classic_02.ogg` (`aeb2d5b21ed6b20b512996e5c1c6a474f3cc6a57`) | Bell Phone | 0 to 3.64 | +5.5 dB | 1454 ms |
| Boötes | `bootes.wav` | `ringtones/ogg/Bootes.ogg` (`46342940ff086d7093ebc304ccd5290be2443058`) | Boötes | 0 to the end | +0.5 dB | none |
| Callisto | `callisto.wav` | `ringtones/material/ogg/Callisto.ogg` (`dd9bc5b26ad2e566422a305a2909a07791e475ea`) | Callisto | 0 to the end | -5 dB | none |
| Callisto Classic | `callisto-classic.wav` | `ringtones/ogg/Callisto.ogg` (`9c982d80752b18334980a4715c11ef8810214fe2`) | Callisto | 0 to 7.112 (first of 2 equal passes) | -3.5 dB | none |
| Canis Major | `canis-major.wav` | `ringtones/ogg/CanisMajor.ogg` (`0e575e06b4dd0b647ff79cbe98a56ff4fd69b7af`) | Canis Major | 0 to the end | +1.5 dB | none |
| Carina | `carina.wav` | `ringtones/ogg/Carina.ogg` (`e8a66b463b41ac327856177a507fbead0d25d4fb`) | Carina | 0 to 1.5 | -1 dB | 1501 ms |
| Cassiopeia | `cassiopeia.wav` | `ringtones/CASSIOPEIA.ogg` (`c4a7948364b2e01dc6fe71fcc00cba69d66ab385`) | Cassiopeia | 0.04 to 3.28 | -3.5 dB | 760 ms |
| Centaurus | `centaurus.wav` | `ringtones/ogg/Centaurus.ogg` (`79a393c3b3295bf9294ba9be55443677d4493d47`) | Centaurus | 0 to 3.71 | -4 dB | 290 ms |
| Chimey Phone | `chimey-phone.wav` | `Ring_Synth_02.ogg` (`4fc192080e7df0f5a03221e0086ccf9aaa6b5055`) | Chimey Phone | 0.03 to 2.67 | +4 dB | 1514 ms |
| Classic 1 | `classic-1.wav` | `Ring_Classic_01.ogg` (`b5e6077cc8fcb3245d57567e7ce37f520aac7d36`) | none | 0 to 3.57 | +6 dB | 662 ms |
| Classic 3 | `classic-3.wav` | `Ring_Classic_03.ogg` (`2b14eb4b84952b1030238c86b2eba8576a8be244`) | none | 0 to 1.75 | +2 dB | 1437 ms |
| Classic 4 | `classic-4.wav` | `Ring_Classic_04.ogg` (`7474ac542796cd1bb744b31216d90ffa40811392`) | none | 0 to 1.97 | +11 dB | 1374 ms |
| Classic 5 | `classic-5.wav` | `Ring_Classic_05.ogg` (`b9ef5b10c92e27d390eb1c364c2b97d7590fcf26`) | none | 0 to 0.891 | -7.5 dB | 2061 ms |
| Cygnus | `cygnus.wav` | `ringtones/ogg/Cygnus.ogg` (`80317ef18caeef149603b57cc2df109b18925266`) | Cygnus | 0 to 3.26 | -1.5 dB | 740 ms |
| Digital 1 | `digital-1.wav` | `Ring_Digital_01.ogg` (`39d2bb86e5915db39a0f52bc0d1710c2ae6cd253`) | none | 0 to 1.59 | -1.5 dB | 2381 ms |
| Digital 3 | `digital-3.wav` | `Ring_Digital_03.ogg` (`2b70ee886d7778f4893099a84fd7774eba6f450f`) | none | 0 to 1.301 | +0.5 dB | 2461 ms |
| Digital 4 | `digital-4.wav` | `Ring_Digital_04.ogg` (`d2ca48680ffcb32bf53d5e947cdc6e2d6ce192d3`) | none | 0.03 to 1.65 | -0.5 dB | 2481 ms |
| Digital 5 | `digital-5.wav` | `Ring_Digital_05.ogg` (`efde4978978742a1614e44511bc6bb0a86732779`) | none | 0 to 4.09 | +0.5 dB | 37 ms |
| Digital Phone | `digital-phone.wav` | `Ring_Digital_02.ogg` (`f823739047def0c2757d9ab5f8cdadfc39482cc2`) | Digital Phone | 0 to 1.21 | +1 dB | 2499 ms |
| Dione | `dione.wav` | `ringtones/material/ogg/Dione.ogg` (`cf7d1cb90417bc0b696d545e591fe8c8c46999c8`) | Dione | 0 to 11.38 | -5 dB | 620 ms |
| Dione Classic | `dione-classic.wav` | `ringtones/ogg/Dione.ogg` (`a378520f588f3faab1e08f40043beaaeaade9bdc`) | Dione | 0 to 12.82 | 0 dB | 680 ms |
| Draco | `draco.wav` | `ringtones/ogg/Draco.ogg` (`d09b8917766a6d993f713de888d4568b313db6bc`) | Draco | 0 to 1.85 | -2.5 dB | 2150 ms |
| Eridani | `eridani.wav` | `ringtones/ogg/Eridani.ogg` (`8d8a1f0ec516b02602cbcb902fca5019557733f4`) | Eridani | 0 to 4 | +3.5 dB | 1000 ms |
| Free Flight | `free-flight.wav` | `ringtones/FreeFlight.ogg` (`76dfabe619edc6394104b2f13d2d6102c3b42576`) | Free Flight | 0 to the end | +0.5 dB | none |
| Ganymede | `ganymede.wav` | `ringtones/material/ogg/Ganymede.ogg` (`e7ff9f4189125d2dec59c071a72ed239956442de`) | Ganymede | 0 to 4.92 | -4.5 dB | 61 ms |
| Ganymede Classic | `ganymede-classic.wav` | `ringtones/ogg/Ganymede.ogg` (`01ff573aef5453508046c87c280cef0dee619dae`) | Ganymede | 0 to 9.14 | +3 dB | 259 ms |
| Girtab | `girtab.wav` | `ringtones/ogg/Girtab.ogg` (`f6b3a35a38597365f55c380e3e2e02cd9241aee2`) | Girtab | 0 to 2.52 | -4.5 dB | 930 ms |
| Hydra | `hydra.wav` | `ringtones/ogg/Hydra.ogg` (`935c12f40c51259fbaa6728bc31f1d8ea6fc0a2d`) | Hydra | 0 to 2.95 | -4.5 dB | 2050 ms |
| Iridium | `iridium.wav` | `ringtones/ogg/Iridium.ogg` (`3de61fb95f28260dfbf755bff1bcfded4f37f171`) | Iridium | 0 to 2.8 | -3.5 dB | 500 ms |
| Kuma | `kuma.wav` | `ringtones/ogg/Kuma.ogg` (`780d5d45c3158c92585aaae8be221ea079f579f4`) | Kuma | 0 to the end | 0 dB | none |
| Luna | `luna.wav` | `ringtones/material/ogg/Luna.ogg` (`8aeb3a036597a9024f5721d62497f2887ba4725d`) | Luna | 0 to 3.3 | -5.5 dB | 1556 ms |
| Luna Classic | `luna-classic.wav` | `ringtones/ogg/Luna.ogg` (`87a2463902ea0b7cc8709d57e0fcb359284ad4dd`) | Luna | 0 to the end | -1.5 dB | none |
| Lyra | `lyra.wav` | `ringtones/ogg/Lyra.ogg` (`d170bc07202340c51e98eb5022e07750afb24e6a`) | none | 0 to 2.99 | -8.5 dB | 1210 ms |
| Lyra Classic | `lyra-classic.wav` | `ringtones/Lyra.ogg` (`696f278b62dea6402619a8137c27af17fa413a1f`) | Lyra | 0 to 5.15 | +4 dB | 526 ms |
| Machina | `machina.wav` | `ringtones/ogg/Machina.ogg` (`33388e6ed18292a038dd45d83ed3423c9eef3815`) | Machina | 0 to 4.12 | -9 dB | 1005 ms |
| Nasqueron | `nasqueron.wav` | `ringtones/ogg/Nasqueron.ogg` (`ae1a7258529a7e57955164663f214eba7528033f`) | Nasqueron | 0 to 8.5 | -5 dB | 1100 ms |
| Oberon | `oberon.wav` | `ringtones/material/ogg/Oberon.ogg` (`145f4746ed27be7d97ec4e5a5d8c6054aa748dd9`) | Oberon | 0 to 15.93 | -5 dB | 70 ms |
| Oberon Classic | `oberon-classic.wav` | `ringtones/ogg/Oberon.ogg` (`124af10019d903e723c97de3b2fa3bb9d138428d`) | Oberon | 0 to 6.81 | -3.5 dB | 107 ms |
| Orion | `orion.wav` | `ringtones/ogg/Orion.ogg` (`0323e4d811a94479c94152155ff1b1422cf77bc2`) | Orion | 0 to the end | -3 dB | none |
| Pegasus | `pegasus.wav` | `ringtones/ogg/Pegasus.ogg` (`d1bf4b01d64ec539e6af29de3049f733ddb65b55`) | Pegasus | 0 to the end | -1.5 dB | none |
| Perseus | `perseus.wav` | `ringtones/ogg/Perseus.ogg` (`48348e5bc97a9d176cb0212fbf78c30afa417d3f`) | Perseus | 0 to the end | -3 dB | none |
| Phobos | `phobos.wav` | `ringtones/material/ogg/Phobos.ogg` (`5e312d20cbc6f8aeebf195fae485c606218d8c50`) | Phobos | 0 to 2.18 | -3.5 dB | 1845 ms |
| Phobos Classic | `phobos-classic.wav` | `ringtones/ogg/Phobos.ogg` (`768ca9498f41a43cdb82ea05c3e054e05fbe5052`) | Phobos | 0 to 3.614 (first of 2 equal passes) | -3 dB | none |
| Pyxis | `pyxis.wav` | `ringtones/material/ogg/Pyxis.ogg` (`4786b18a193591872ca784774cbf42256b3f4587`) | Pyxis | 0 to the end | -3.5 dB | none |
| Rasalas | `rasalas.wav` | `ringtones/ogg/Rasalas.ogg` (`7e62f17eef4fe82b403ae6c68c32010af2717222`) | Rasalas | 0 to 25.29 | -2 dB | 235 ms |
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
| Scarabaeus | `scarabaeus.wav` | `ringtones/ogg/Scarabaeus.ogg` (`3dcd0d7e8c274d69b54e38283d752d9bb59c12f8`) | Scarabaeus | 0.11 to 8.81 | -5 dB | 531 ms |
| Sceptrum | `sceptrum.wav` | `ringtones/ogg/Sceptrum.ogg` (`f2d3ed445c21eace9d3e6b09473e92ffe8991f0f`) | Sceptrum | 0 to 29.7 | -4 dB | 300 ms |
| Sceptrum Classic | `sceptrum-classic.wav` | `ringtones/Sceptrum.ogg` (`fc50aef575e4043d076baa8b706bffe55b6f6635`) | Sceptrum | 0.03 to 21.85 | -3 dB | 708 ms |
| Sedna | `sedna.wav` | `ringtones/material/ogg/Sedna.ogg` (`ff889e60882d4c21553408620bf019abbb779cc5`) | Sedna | 0 to 6.74 | -4 dB | 117 ms |
| Solarium | `solarium.wav` | `ringtones/ogg/Solarium.ogg` (`8c9d3073b8ce1557ed1c328d3f314b2b6737cf4a`) | Solarium | 0 to 8.11 | -4 dB | 804 ms |
| Solarium Classic | `solarium-classic.wav` | `ringtones/Solarium.ogg` (`d27f141dd9304a44accfa2742e30c40a16ef9dd2`) | Solarium | 0 to 6.82 | -5 dB | 1075 ms |
| Synth 1 | `synth-1.wav` | `Ring_Synth_01.ogg` (`ec2e628528dd31370bb169b1b32186c7d9807c6f`) | none | 0 to 1.66 | +0.5 dB | 1292 ms |
| Synth 3 | `synth-3.wav` | `Ring_Synth_03.ogg` (`bb45ecb5e7546b978c950a9a480541ab9f07895d`) | none | 0 to 1.92 | -0.5 dB | 2155 ms |
| Synth 5 | `synth-5.wav` | `Ring_Synth_05.ogg` (`674434ea22a68b993c462652000a44377a7451b5`) | none | 0 to 3.11 | +3.5 dB | 51 ms |
| Testudo | `testudo.wav` | `ringtones/ogg/Testudo.ogg` (`42db3a59a923c6d6b071a52dc5b873ef38749cab`) | Testudo | 0 to 7.22 | -11 dB | 416 ms |
| Titania | `titania.wav` | `ringtones/material/ogg/Titania.ogg` (`87a05eef57e01ef9816bb8aa652dd1eb4eef191d`) | Titania | 0 to the end | -7 dB | none |
| Triton | `triton.wav` | `ringtones/material/ogg/Triton.ogg` (`b5893bde3eb0e0a7eef71a9d540d0c014ad45841`) | Triton | 0 to 14.16 | -6 dB | 533 ms |
| Triton Classic | `triton-classic.wav` | `ringtones/ogg/Triton.ogg` (`542ae1624e00bdd6184149331fe9994fa0035cdc`) | Triton | 0 to the end | -3 dB | none |
| Umbriel | `umbriel.wav` | `ringtones/material/ogg/Umbriel.ogg` (`193e566f8cc5db98cc421fbbf361cab044beda2a`) | Umbriel | 0 to 7.1 | -5 dB | 1117 ms |
| Umbriel Classic | `umbriel-classic.wav` | `ringtones/ogg/Umbriel.ogg` (`b21a12e0778a834e492afc8c20b18654a36e5998`) | Umbriel | 0 to the end | -3 dB | none |
| Ursa Minor | `ursa-minor.wav` | `ringtones/ogg/UrsaMinor.ogg` (`30d2cd1d1763b3ff4e5143542cc08f9f906168bd`) | UrsaMinor | 0 to 12.75 | -4 dB | 50 ms |
| Ursa Minor Classic | `ursa-minor-classic.wav` | `ringtones/URSAMINOR.ogg` (`c0010e821fe9cadd027511794b542bab305fa816`) | Ursa Minor | 0 to the end | -5 dB | none |
| Vespa | `vespa.wav` | `ringtones/Vespa.ogg` (`4423bbb6a5acff01bc7ac4eafccd018421d54800`) | Vespa | 0 to 4.74 | -2 dB | 2117 ms |
| Zeta | `zeta.wav` | `ringtones/ogg/Zeta.ogg` (`ef64d2feda7e7f484f6d00c355a7c11e6aa5a343`) | Zeta | 0 to 5.41 | -7.5 dB | 1190 ms |

`derive.sh` holds the commands that made each file from the upstream file (ffmpeg 9.0.2), the same as for the alarm tones (`assets/alarm-tones/README.md`): to 16 kHz mono, cut to the sound of one pass, changed in level so that the ringtones are about equally loud, and stored as 8-bit mu-law, which costs 16 KB per second.  A pass ends where the sound falls 50 dB below its loudest 10 ms.  The silence before and after the sound upstream is not stored; the phone leaves it as the pause between two passes, so that a ringtone repeats at the period of its upstream file, which Android loops (`ANDROID_LOOP`).  Where the upstream file holds two equal passes (Callisto Classic and Phobos Classic), one is kept.  A ringtone without a pause runs into its next pass as its upstream file does when it loops.  The level brings the samples above a twentieth of the peak to an RMS of 5,500 (-15.5 dBFS), or less where the peak would pass 26,000.  Stereo files are mixed to mono.

The 90 files are 10,763,484 bytes; in the image's compressed root file system, with `NOTICE` and the license text, they take about 8.3 MB.

A raw file is at `https://android.googlesource.com/platform/frameworks/base/+/1cdfff555f4a21f71ccc978290e2e212e2f8b168/data/sounds/<file>?format=TEXT`, in Base64.

## Files not used

The other 60 files hold sounds already in the table, each compared with the file used by its samples decoded at 16 kHz mono.  "Same samples" means that the two decode to identical samples.  "Same recording" means a waveform correlation of 0.99 or more: another encoding of the same audio, at most at another level, sample rate, or number of channels.  "Another mix" means the same notes at the same times, with a waveform correlation from 0.40 to 0.96, log spectrograms that correlate at 0.84 or more, and pitch-class profiles that correlate at 0.71 or more: a separate mix or master of the same recording.  The distinct sounds that share a title measure at most 0.13 and 0.39 on the first and last of these.

| Ringtone | Files not used |
| --- | --- |
| Andromeda, Aquila, Boötes, Canis Major, Centaurus, Cygnus, Draco, Eridani, Hydra, Machina, Orion, Pegasus, Scarabaeus, and Testudo | `ringtones/ANDROMEDA.ogg`, `ringtones/Aquila.ogg`, `ringtones/BOOTES.ogg`, `ringtones/CANISMAJOR.ogg`, `ringtones/Centaurus.ogg`, `ringtones/Cygnus.ogg`, `ringtones/Draco.ogg`, `ringtones/Eridani.ogg`, `ringtones/hydra.ogg`, `ringtones/Machina.ogg`, `ringtones/Orion.ogg`, `ringtones/Pegasus.ogg`, `ringtones/Scarabaeus.ogg`, and `ringtones/Testudo.ogg`: same samples as the file of the same name in `ringtones/ogg`. |
| Acheron | `ringtones/ringtone27.ogg`: same samples. |
| Argo Navis | `ringtones/ArgoNavis.ogg`: another mix (waveform 0.83). |
| Carina | `ringtones/Carina.ogg`: same samples.  `ringtones/Carina.wav` and `ringtones/wav/Carina.wav` (one blob): same recording, without the 1.5 s of silence after it. |
| Digital Phone | `ringtones/Pyxis.ogg`, `ringtones/ogg/Pyxis.ogg`, and `ringtones/ogg/Sedna.ogg` and `ringtones/ogg/Sedna_48k.ogg` (one blob): same samples as one another, and the same recording as `Ring_Digital_02.ogg`. |
| Dione Classic | `ringtones/ogg/Atria.ogg`: same samples.  `ringtones/ogg/Dione_48k.ogg`: the same blob. |
| Free Flight | `ringtones/FreeFlight.wav`: same recording. |
| Girtab | `ringtones/ogg/Cassiopeia.ogg`: same samples.  `ringtones/wav/Cassiopeia.wav`: same recording. |
| Lyra, Sceptrum, Solarium, and Ursa Minor | `ringtones/wav/Lyra.wav`, `ringtones/wav/Sceptrum.wav`, `ringtones/wav/Solarium.wav`, and `ringtones/wav/UrsaMinor.wav`: same recording. |
| Nasqueron | `ringtones/ringtone22.ogg`: another mix (waveform 0.40). |
| Perseus | `ringtones/PERSEUS.ogg`: another mix (waveform 0.96). |
| Rigel | `ringtones/Rigel.ogg`: same recording. |
| Titania | `ringtones/ogg/Titania.ogg` (Android 4.4's Titania) and `ringtones/ogg/Titania_48k.ogg`, and `ringtones/ogg/Themos.ogg` (Android 4.0 to 4.3's Themos) and `ringtones/wav/Themos.wav`: another mix (waveform 0.53 to 0.61). |
| Zeta | `ringtones/ogg/Vespa.ogg`: same samples.  `ringtones/wav/Vespa.wav`: same recording. |
| The ringtones of `ringtones/material/ogg` and Android 4.4's with a 48 kHz version | `X_48k.ogg` beside each of the 12 files of `ringtones/material/ogg` and beside `ringtones/ogg/X.ogg` for Callisto, Ganymede, Luna, Oberon, Phobos, Triton, and Umbriel: same recording, or another mix for Atria, Luna, Titania, and Triton of `ringtones/material/ogg` and Oberon and Umbriel of `ringtones/ogg` (waveform 0.63 to 0.93).  Android installs one or the other under the same name (`AudioPackage12_48.mk` and `AudioPackage13_48.mk`). |
