#!/bin/sh
# Regenerate the measurement logs in docs/experiments/ from the current engine.
set -e
cd "$(dirname "$0")/.."
(cd engine && cargo build --release 2>/dev/null)
# engine/data/board.bin (soundboard modes) is regenerated with:
#   (cd engine && cargo run --release --bin gen_board)
./build.sh >/dev/null 2>&1
A=engine/target/release/analyze
OUT=docs/experiments
mkdir -p "$OUT" docs/images

{
  echo "# Filter design per key (fs = 48 kHz)"
  echo
  echo '`analyze design` — every third key. "weighted fit err" is the RMS partial'
  echo "frequency error of the dispersion filter (partials above 3 kHz weighted down)."
  echo
  $A design
  echo
  echo "## Partial frequency error of the dispersion filter"
  echo
  echo '`analyze dispersion` — error in cents against f_n = n f0 sqrt(1 + B n^2).'
  echo '"1st-order only" is the first design tried (identical first-order allpasses),'
  echo '"with 2nd-order" adds the Abel-Smith style biquad cascade used in the final model.'
  echo
  $A dispersion | tail -n +2
} > "$OUT/01-filter-design.md"

{
  echo "# Per-key measurements of the rendered sound (fs = 48 kHz, velocity 0.6)"
  echo
  echo '`analyze keys` — measured from the stereo output (after the soundboard).'
  echo "B is fitted to the measured partial frequencies. Decay rates are those of"
  echo "the first partial; unison beating makes them fluctuate from key to key."
  echo
  $A keys | tail -n +2
  echo
  echo "## Partials of C2, C4 and C6"
  echo
  for m in 36 60 84; do $A note $m 0.6 | head -24; echo; done
  echo "## Velocity response"
  echo
  for m in 33 60 84; do $A velocity $m; echo; done
  echo "## Decay envelope of C4 (first partial, second partial, broadband RMS)"
  echo
  echo '```'
  $A env 60 0.6 12 | awk 'NR == 1 || NR % 2 == 0'
  echo '```'
} > "$OUT/02-tone-measurements.md"

{
  echo "# Soundboard, bridge coupling, sympathetic resonance and stability"
  echo
  echo "## Radiated response of the soundboard (to the microphones)"
  echo
  $A board
  echo
  echo "## Radiated response towards the room"
  echo
  $A board 48000 far | tail -n +4
  echo
  echo "## Bridge admittance seen by the keys"
  echo
  echo "G = conductance (real part of the admittance) after the key's lever ratio."
  echo
  $A admittance
  echo
  echo "## Decay of each partial (C2, C4, C6)"
  echo
  for m in 36 60 84; do $A decay $m; echo; done
  echo "## Sympathetic resonance"
  echo
  echo "C4 struck staccato; the level that remains 0.5-1.5 s after its damper has fallen"
  echo "comes only from the other strings (and the room)."
  echo
  $A sympathetic
  echo
  echo "## Stability: all 88 keys fff with the pedal down"
  echo
  for fs in 44100 48000 96000; do echo "fs = $fs"; echo; $A stability $fs | awk 'NR <= 3 || NR % 5 == 0'; echo; done
} > "$OUT/03-soundboard-resonance-stability.md"

{
  echo "# Ingredients of the tone and overall spectrum"
  echo
  echo "## Level of key noise, longitudinal string force and room"
  echo
  echo "Each column compares a render with the ingredient on against one with it off."
  echo
  $A parts
  echo
  echo "## Long-term average spectrum of a short piece (chords with pedal + melody)"
  echo
  for v in 0.3 0.55 0.9; do $A ltas $v; echo; done
} > "$OUT/07-ingredients-spectrum.md"

{
  echo "# Per-key loudness levelling and bass emphasis"
  echo
  echo "## Loudness of every key relative to C4"
  echo
  echo "Loudness level (phon) from third-octave band levels, ISO 226:2003 equal-loudness"
  echo "contours and Stevens' band summation; full scale = 100 dB SPL. The levelling"
  echo "table rescales the hammer velocity per key and key velocity."
  echo
  $A loudness 48000 0.1,0.2,0.3,0.45,0.6,0.75,0.9,1.0
  echo
  $A loudness 96000 0.1,0.2,0.3,0.45,0.6,0.75,0.9,1.0 full
  echo
  echo "## Bass emphasis (output stage, src/post.rs)"
  echo
  $A bass 48000
} > "$OUT/08-leveling-bass.md"

{
  echo "# CPU cost"
  echo
  echo "Machine: $(sysctl -n machdep.cpu.brand_string), $(rustc --version), node $(node --version)"
  echo
  echo "## Native build (engine/src/bin/bench.rs)"
  echo
  for fs in 48000 96000; do echo "fs = $fs Hz"; echo; engine/target/release/bench $fs; echo; done
  echo "The second column is what an audio thread sees: it renders one quantum per"
  echo "period and sleeps in between, and such a thread is run at low clock rates or"
  echo "on efficiency cores. Confined to the efficiency cores (\`taskpolicy -b\`), as a"
  echo "stand-in for a slow machine:"
  echo
  taskpolicy -b engine/target/release/bench 48000 | tail -n +3
  echo
  echo "## WASM build under V8 (tools/bench-wasm.mjs)"
  echo
  echo '"quantum max" includes scheduling noise of the benchmark machine; "quantum p99"'
  echo "is the relevant figure against the render-quantum budget. The quanta are"
  echo "rendered in a row here (see above for what that leaves out)."
  echo
  for fs in 48000 96000; do echo "fs = $fs Hz"; echo; node tools/bench-wasm.mjs $fs; echo; done
} > "$OUT/04-cpu.md"

{
  echo "# Algorithmic latency (engine only)"
  echo
  echo '`analyze latency` — time from note-on to the first output sample exceeding 1 % of'
  echo "the note's peak (key noise off). The bridge junction is evaluated at the striking"
  echo "point, so the hammer-to-bridge travel time does not delay the sound."
  echo
  $A latency
} > "$OUT/05-latency-engine.md"

W=$(mktemp -d)
engine/target/release/render demo "$W/demo.wav" >/dev/null
sox "$W/demo.wav" -n remix 1 trim 0 14 rate 16k spectrogram -x 1200 -y 513 -z 90 \
  -t "A0 A1 A2 A3 C4 A4 A5 A6 A7 C8 (velocity 0.6)" -o docs/images/spectrogram-notes.png 2>/dev/null
sox "$W/demo.wav" -n remix 1 trim 14 5.2 rate 16k spectrogram -x 1200 -y 513 -z 90 \
  -t "C4 at velocity 0.15 / 0.35 / 0.55 / 0.75 / 0.95" -o docs/images/spectrogram-dynamics.png 2>/dev/null
rm -rf "$W"
echo "experiments written to $OUT"
