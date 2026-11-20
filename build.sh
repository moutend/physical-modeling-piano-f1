#!/bin/sh
# Build the WASM module used by the web app.
set -e
cd "$(dirname "$0")/engine"
# Generated tables (run in engine/, in this order, whenever the model changes;
# twice after a larger change, since gen_sustain measures the keys as they are
# played with the other tables. The keys up to C3 keep their entries.):
#   cargo run --release --bin gen_board      -> data/board.bin    (soundboard modes)
#   cargo run --release --bin gen_sustain    -> data/sustain.bin  (per-key sustain correction)
#   cargo run --release --bin gen_voicing    -> data/voicing.bin  (per-key direct radiation: even blow / tone)
#   cargo run --release --bin gen_leveling   -> data/leveling.bin (per-key loudness levelling)
#   cargo run --release --bin gen_morph      -> data/morph.bin    (keys between the reference keys: blend of their tone)
#   cargo run --release --bin gen_leveling   (again: levels the keys that gen_morph changed)
RUSTFLAGS="-C target-feature=+simd128" cargo build --release --target wasm32-unknown-unknown --lib
cp target/wasm32-unknown-unknown/release/piano_engine.wasm ../web/piano.wasm
ls -l ../web/piano.wasm
