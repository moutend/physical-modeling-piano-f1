# Physical Modeling Piano F1

An 88-key piano instrument that computes physical models of the strings, bridge, soundboard, sound radiation into the air, and more in real time, together with its web app.

## Quick Start

Visit [piano.yoshiyukikoyanagi.com](https://piano.yoshiyukikoyanagi.com) to try the piano right away.

To try it in your local development environment, open a terminal and run the following command.

```bash
node tools/serve.mjs 8080
```

- After running the command, open `http://localhost:8080` in your browser.
- The latest version of Node.js is required.

## How to Use the UI

After opening the page, press the "Start Playing" button first. The piano is then ready to make sound.

### Playing with a PC Keyboard

- The `Z X C …` row and the `Q W E …` row: play the keys. The `Q W E …` row sounds one octave higher than the `Z X C …` row.
- `↑` `↓`: shift the octave.
- `←` `→`: adjust how hard the keys are struck (Velocity).
- `Shift`: holds the sustain pedal down while pressed.
- `Esc`: stops all sounding notes.

### Playing a MIDI File

- Choose a standard MIDI file (`.mid`) with the "Open MIDI File" button to play it back.
  - The keyboard shortcut is `Command+U` on macOS and `Ctrl+U` elsewhere.
- `Space`: toggles between play and pause for the MIDI file.

## About This Project

This project was created through conversation with Anthropic's Fable 5.2. The physical model was built from the knowledge Fable 5.2 has. I only gave instructions and did not implement anything by hand.

Note that this project was not made by reverse engineering a well-known commercial piano instrument (such as Synthogy's Ivory) to reproduce its behavior. Nor is it a wholesale copy of an existing open source implementation.

### Contributions and Forks

This is strictly a hobby project, so I do not accept PRs. If you find something to improve, feel free to fork this repository and release it as your own new piano instrument. No credit is required. That said, I would be glad if you mentioned that the project originated with [@moutend](https://github.com/moutend).

### Privacy and Security

This web app runs on the client side and does not send visitors' personal information to any external server. It also does not load dynamically generated JavaScript from an external CDN.

I therefore believe its security is sound at this time. However, I accept no responsibility even if a security flaw is discovered.

## Miscellaneous

<details>
<summary>Documentation and commands for developers</summary>

- The work log, model description, and measurement results are in [docs/report.md](docs/report.md) (written in Japanese).
- The measurement logs are in [docs/experiments/](docs/experiments/) (written in Japanese).
- Render a MIDI file to WAV (for comparison with recordings): `cd engine && cargo run --release --bin render -- midi out.wav ../midi/fur_Elise_WoO59.mid`
- Test repeated strikes of the same note with the pedal held down (all 88 keys): `cd engine && cargo run --release --bin repeat -- scan`
- Rebuild the engine: `./build.sh` (requires Rust and the `wasm32-unknown-unknown` target)
- Measure the CPU cost: `cd engine && cargo run --release --bin bench` (native), `node tools/bench-wasm.mjs` (WASM; also times the demo songs). To measure on efficiency cores as a stand-in for a slow machine, prefix the command with `taskpolicy -b` on macOS.
- Compare renderings made before and after a change to the engine: `cd engine && cargo run --release --bin abcmp -- before.wav after.wav` (level of the difference signal, and third-octave band level differences per time window)
- Recompute the soundboard modes: `cd engine && cargo run --release --bin gen_board`
- Recompute the loudness leveling table (required after changing the model): `cd engine && cargo run --release --bin gen_leveling`
- After changing the model, recompute the full set of correction tables in the order given at the top of `build.sh` (`gen_sustain` → `gen_voicing` → `gen_leveling` → `gen_morph` → `gen_leveling`; run the sequence twice after a larger change). The entries for keys C3 and below are kept as they are and are not regenerated.
- Recompute the table that interpolates the tone of the reference keys to the other keys: `cd engine && cargo run --release --bin gen_morph`, then run `gen_leveling` once more (the reference keys are listed in `ANCHORS` in `engine/src/morph.rs`; check the result with `cargo run --release --bin analyze -- morph`).

</details>

## License

MIT

**Additional Note**

As the owner of this repository, I understand this implementation to be something Fable 5.2 devised and implemented on its own, based on the physical characteristics of a piano. However, if it contains anything that infringes the license of existing software, please contact me at `Yoshiyuki Koyanagi <moutend@gmail.com>`.
