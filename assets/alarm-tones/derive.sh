#!/bin/sh
# Derives the alarm tones from the Android Open Source Project's files.
# Usage: derive.sh UPSTREAM_DIR OUT_DIR, with ffmpeg on the PATH.
# UPSTREAM_DIR holds the .ogg files named in README.md, unmodified.
# Each tone is brought to 16 kHz mono, cut after its last sound (the
# silence that follows is the pause the phone leaves between passes),
# brought to a level like the others', and stored as 8-bit mu-law.
set -eu
up=$1
out=$2
tone() { # name, source file, seconds kept, gain, extra filter
  ffmpeg -v error -y -i "$up/$2" -ac 1 -ar 16000 \
    -af "atrim=0:$3,volume=$4$5" \
    -map_metadata -1 -fflags +bitexact -flags:a +bitexact \
    -c:a pcm_mulaw "$out/$1.wav"
}
tone beep    Alarm_Beep_03.ogg    0.6    0.5dB ""
tone buzzer  Alarm_Buzzer.ogg     1.0    1.5dB ""
tone classic Alarm_Classic.ogg    1.0    5.5dB ""
tone osmium  Osmium.ogg           1.6   -0.5dB ""
tone rooster Alarm_Rooster_02.ogg 1.6725 -1.5dB ""
# The first of Argon's two phrases, faded out over its last 60 ms.
tone argon   Argon.ogg            3.5    8dB   ",afade=t=out:st=3.44:d=0.06"
