#!/bin/sh
# Derives the alarm tones from the Android Open Source Project's files.
# Usage: derive.sh UPSTREAM_DIR OUT_DIR, with ffmpeg on the PATH.
# UPSTREAM_DIR is the upstream directory data/sounds named in README.md,
# or a copy of the files used here at the same paths, unmodified.
# Each tone is brought to 16 kHz mono, cut to the sound of one pass (the
# silence around it is the pause the phone leaves between passes),
# brought to a level like the others', and stored as 8-bit mu-law.  The
# cut is in samples at 16 kHz: from the first kept to the first not kept,
# or to the end of the file.
set -eu
up=$1
out=$2
tone() { # file, source, gain (dB), from, to (samples; the end if none)
  ffmpeg -v error -y -i "$up/$2" \
    -af "aresample=16000,aformat=channel_layouts=mono,asetpts=N/SR/TB,atrim=start_sample=$4${5:+:end_sample=$5},volume=$3dB" \
    -ac 1 -ar 16000 -map_metadata -1 -fflags +bitexact -flags:a +bitexact \
    -c:a pcm_mulaw "$out/$1.wav"
}
tone beep-beep-beep Alarm_Beep_03.ogg                            0.5      0   8976
tone argon          alarms/material/ogg/Argon.ogg                 -5    480 304000
tone argon-jp       alarms/ogg-jp/Argon.ogg                     -5.5      0  83680
tone awaken         alarms/material/ogg/Awaken_OG7_1ch_48k.ogg     0   3840 368320
tone barium         alarms/ogg/Barium.ogg                       -0.5  15600
tone beebeep        Alarm_Beep_02.ogg                             -1      0   3360
tone bounce         alarms/material/ogg/Bounce_OG7_1ch_48k.ogg   0.5      0 229760
tone buzzer         Alarm_Buzzer.ogg                             1.5      0  16000
tone carbon         alarms/material/ogg/Carbon.ogg                -4      0  48320
tone carbon-jp      alarms/ogg-jp/Carbon.ogg                    -4.5      0  32320
tone cesium         Alarm_Classic.ogg                            2.5      0  15680
tone copernicium    alarms/ogg/Copernicium.ogg                    -6      0 124320
tone curium         alarms/ogg/Curium.ogg                      -10.5    960 123520
tone drip           alarms/material/ogg/Drip_OG7_1ch_48k.ogg     2.5      0  36656
tone fermium        alarms/ogg/Fermium.ogg                       8.5      0
tone gallop         alarms/material/ogg/Gallop_OG7_1ch_48k.ogg  -0.5  57600 113280
tone hassium        alarms/ogg/Hassium.ogg                        -2      0 127680
tone helium         alarms/material/ogg/Helium.ogg                -7      0
tone helium-jp      alarms/ogg-jp/Helium.ogg                      -8      0 128160
tone krypton        alarms/material/ogg/Krypton.ogg                0      0
tone neon           alarms/material/ogg/Neon.ogg                -4.5      0
tone neptunium      alarms/ogg/Neptunium.ogg                    -1.5      0
tone nobelium       alarms/ogg/Nobelium.ogg                       -4      0 313920
tone nudge          alarms/material/ogg/Nudge_OG7_1ch_48k.ogg     -3      0  81600
tone orbit          alarms/material/ogg/Orbit_OG7_1ch_48k.ogg     -3      0 164800
tone osmium         alarms/material/ogg/Osmium.ogg                -5      0  83840
tone osmium-classic alarms/ogg/Osmium.ogg                       -0.5      0  27360
tone oxygen         alarms/material/ogg/Oxygen.ogg              -2.5      0 165760
tone piezo          Alarm_Beep_01.ogg                              1    480   6560
tone platinum       alarms/material/ogg/Platinum.ogg             0.5      0 378720
tone plutonium      alarms/ogg/Plutonium.ogg                    -0.5      0  54560
tone promethium     alarms/ogg/Promethium.ogg                   -0.5      0 106400
tone rise           alarms/material/ogg/Rise_OG7_1ch_48k.ogg      10      0  67680
tone rooster-1      Alarm_Rooster_01.ogg                          -1      0  25600
tone rooster-2      Alarm_Rooster_02.ogg                        -1.5      0
tone scandium       alarms/ogg/Scandium.ogg                     -2.5      0  39520
tone sway           alarms/material/ogg/Sway_OG7_1ch_48k.ogg       0      0
tone timer          alarms/material/ogg/Timer.ogg               -6.5    480  37600
tone wag            alarms/material/ogg/Wag_OG7_1ch_48k.ogg       -2      0  55680
