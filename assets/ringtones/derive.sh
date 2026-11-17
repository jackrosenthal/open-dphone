#!/bin/sh
# Derives the ringtones from the Android Open Source Project's files.
# Usage: derive.sh UPSTREAM_DIR OUT_DIR, with ffmpeg on the PATH.
# UPSTREAM_DIR is the upstream directory data/sounds named in README.md,
# or a copy of the files used here at the same paths, unmodified.
# Each ringtone is brought to 16 kHz mono, cut to the sound of one pass (the
# silence around it is the pause the phone leaves between passes),
# brought to a level like the others', and stored as 8-bit mu-law.  The
# cut is in samples at 16 kHz: from the first kept to the first not kept,
# or to the end of the file.
set -eu
up=$1
out=$2
ring() { # file, source, gain (dB), from, to (samples; the end if none)
  ffmpeg -v error -y -i "$up/$2" \
    -af "aresample=16000,aformat=channel_layouts=mono,asetpts=N/SR/TB,atrim=start_sample=$4${5:+:end_sample=$5},volume=$3dB" \
    -ac 1 -ar 16000 -map_metadata -1 -fflags +bitexact -flags:a +bitexact \
    -c:a pcm_mulaw "$out/$1.wav"
}
ring flutey-phone       Ring_Synth_04.ogg                       2    480  27840
ring acheron            ringtones/ogg/Acheron.ogg            -3.5      0 113280
ring andromeda          ringtones/ogg/Andromeda.ogg            -4      0  26400
ring aquila             ringtones/ogg/Aquila.ogg             -3.5      0  20000
ring argo-navis         ringtones/ogg/ArgoNavis.ogg           1.5      0 192000
ring atria              ringtones/material/ogg/Atria.ogg       -8      0
ring bell-phone         Ring_Classic_02.ogg                   5.5      0  58240
ring bootes             ringtones/ogg/Bootes.ogg              0.5      0
ring callisto           ringtones/material/ogg/Callisto.ogg    -5      0
ring callisto-classic   ringtones/ogg/Callisto.ogg           -3.5      0 113792
ring canis-major        ringtones/ogg/CanisMajor.ogg          1.5      0
ring carina             ringtones/ogg/Carina.ogg               -1      0  24000
ring cassiopeia         ringtones/CASSIOPEIA.ogg             -3.5    640  52480
ring centaurus          ringtones/ogg/Centaurus.ogg            -4      0  59360
ring chimey-phone       Ring_Synth_02.ogg                       4    480  42720
ring classic-1          Ring_Classic_01.ogg                     6      0  57120
ring classic-3          Ring_Classic_03.ogg                     2      0  28000
ring classic-4          Ring_Classic_04.ogg                    11      0  31520
ring classic-5          Ring_Classic_05.ogg                  -7.5      0  14256
ring cygnus             ringtones/ogg/Cygnus.ogg             -1.5      0  52160
ring digital-1          Ring_Digital_01.ogg                  -1.5      0  25440
ring digital-3          Ring_Digital_03.ogg                   0.5      0  20816
ring digital-4          Ring_Digital_04.ogg                  -0.5    480  26400
ring digital-5          Ring_Digital_05.ogg                   0.5      0  65440
ring digital-phone      Ring_Digital_02.ogg                     1      0  19360
ring dione              ringtones/material/ogg/Dione.ogg       -5      0 182080
ring dione-classic      ringtones/ogg/Dione.ogg                 0      0 205120
ring draco              ringtones/ogg/Draco.ogg              -2.5      0  29600
ring eridani            ringtones/ogg/Eridani.ogg             3.5      0  64000
ring free-flight        ringtones/FreeFlight.ogg              0.5      0
ring ganymede           ringtones/material/ogg/Ganymede.ogg  -4.5      0  78720
ring ganymede-classic   ringtones/ogg/Ganymede.ogg              3      0 146240
ring girtab             ringtones/ogg/Girtab.ogg             -4.5      0  40320
ring hydra              ringtones/ogg/Hydra.ogg              -4.5      0  47200
ring iridium            ringtones/ogg/Iridium.ogg            -3.5      0  44800
ring kuma               ringtones/ogg/Kuma.ogg                  0      0
ring luna               ringtones/material/ogg/Luna.ogg      -5.5      0  52800
ring luna-classic       ringtones/ogg/Luna.ogg               -1.5      0
ring lyra               ringtones/ogg/Lyra.ogg               -8.5      0  47840
ring lyra-classic       ringtones/Lyra.ogg                      4      0  82400
ring machina            ringtones/ogg/Machina.ogg              -9      0  65920
ring nasqueron          ringtones/ogg/Nasqueron.ogg            -5      0 136000
ring oberon             ringtones/material/ogg/Oberon.ogg      -5      0 254880
ring oberon-classic     ringtones/ogg/Oberon.ogg             -3.5      0 108960
ring orion              ringtones/ogg/Orion.ogg                -3      0
ring pegasus            ringtones/ogg/Pegasus.ogg            -1.5      0
ring perseus            ringtones/ogg/Perseus.ogg              -3      0
ring phobos             ringtones/material/ogg/Phobos.ogg    -3.5      0  34880
ring phobos-classic     ringtones/ogg/Phobos.ogg               -3      0  57824
ring pyxis              ringtones/material/ogg/Pyxis.ogg     -3.5      0
ring rasalas            ringtones/ogg/Rasalas.ogg              -2      0 404640
ring rigel              ringtones/ogg/Rigel.ogg              -2.5      0  50080
ring ringtone-18        ringtones/ringtone18.ogg               -1      0 209760
ring ringtone-19        ringtones/ringtone19.ogg               -3      0 103520
ring ringtone-20        ringtones/ringtone20.ogg               -5      0  62880
ring ringtone-21        ringtones/ringtone21.ogg            -10.5      0 176960
ring ringtone-23        ringtones/ringtone23.ogg             -2.5      0 164000
ring ringtone-24        ringtones/ringtone24.ogg               -5      0 237760
ring ringtone-25        ringtones/ringtone25.ogg             -4.5      0 170400
ring ringtone-26        ringtones/ringtone26.ogg             -3.5      0  88800
ring ringtone-28        ringtones/ringtone28.ogg             -8.5      0 202240
ring ringtone-29        ringtones/ringtone29.ogg               -5      0 201920
ring ringtone-30        ringtones/ringtone30.ogg               -4      0 158720
ring ringtone-31        ringtones/ringtone31.ogg             -4.5      0  98400
ring ringtone-32        ringtones/ringtone32.ogg             -0.5      0 151840
ring ringtone-33        ringtones/ringtone33.ogg            -10.5      0  63520
ring ringtone-34        ringtones/ringtone34.ogg             -0.5      0  53120
ring ringtone-35        ringtones/ringtone35.ogg             -9.5      0 186240
ring ringtone-36        ringtones/ringtone36.ogg               -6      0
ring ringtone-37        ringtones/ringtone37.ogg               -4      0 159840
ring ringtone-38        ringtones/ringtone38.ogg               -2      0 143360
ring scarabaeus         ringtones/ogg/Scarabaeus.ogg           -5   1760 140960
ring sceptrum           ringtones/ogg/Sceptrum.ogg             -4      0 475200
ring sceptrum-classic   ringtones/Sceptrum.ogg                 -3    480 349600
ring sedna              ringtones/material/ogg/Sedna.ogg       -4      0 107840
ring solarium           ringtones/ogg/Solarium.ogg             -4      0 129760
ring solarium-classic   ringtones/Solarium.ogg                 -5      0 109120
ring synth-1            Ring_Synth_01.ogg                     0.5      0  26560
ring synth-3            Ring_Synth_03.ogg                    -0.5      0  30720
ring synth-5            Ring_Synth_05.ogg                     3.5      0  49760
ring testudo            ringtones/ogg/Testudo.ogg             -11      0 115520
ring titania            ringtones/material/ogg/Titania.ogg     -7      0
ring triton             ringtones/material/ogg/Triton.ogg      -6      0 226560
ring triton-classic     ringtones/ogg/Triton.ogg               -3      0
ring umbriel            ringtones/material/ogg/Umbriel.ogg     -5      0 113600
ring umbriel-classic    ringtones/ogg/Umbriel.ogg              -3      0
ring ursa-minor         ringtones/ogg/UrsaMinor.ogg            -4      0 204000
ring ursa-minor-classic ringtones/URSAMINOR.ogg                -5      0
ring vespa              ringtones/Vespa.ogg                    -2      0  75840
ring zeta               ringtones/ogg/Zeta.ogg               -7.5      0  86560
