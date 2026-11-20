// CPU cost of the WASM build under Node (V8, same engine as Chrome).
//   node tools/bench-wasm.mjs [sampleRate] [piano.wasm] [seconds of each song]
//
// Two tables: held chords (the cost of a given number of sounding strings),
// and the demo songs played the way the AudioWorklet plays them (128-frame
// quanta, events on a 16-frame grid), which is what a listener's machine has
// to keep up with. "quantum p99 / max" against "budget" is what decides
// whether the output drops out; the average only says how warm the machine
// gets.
import { readFile } from 'node:fs/promises';
import { parseMidi, EV_NOTE_ON, EV_NOTE_OFF, EV_PEDAL } from '../web/midi-file.js';

const fs = Number(process.argv[2] ?? 48000);
const wasmPath = process.argv[3] ?? new URL('../web/piano.wasm', import.meta.url);
const songSecs = Number(process.argv[4] ?? 90);
const bytes = await readFile(wasmPath);
const t0 = performance.now();
const { instance } = await WebAssembly.instantiate(bytes, {});
const x = instance.exports;
x.piano_init(fs);
console.log(`init: ${(performance.now() - t0).toFixed(1)} ms, wasm ${bytes.length} bytes`);
// Same warm-up as the AudioWorklet (lets V8 tier up).
const tw = performance.now();
x.piano_pedal(1);
for (let m = 21; m <= 108; m++) x.piano_note_on(m, 0.5 + 0.005 * m);
for (let i = 0; i < 300; i++) x.piano_process(128);
x.piano_all_off();
console.log(`warm-up: ${(performance.now() - tw).toFixed(1)} ms`);

const budget = (128 / fs) * 1000;
const stats = (times, total) => {
  const sorted = [...times].sort((a, b) => a - b);
  const load = total / 1000 / ((times.length * 128) / fs);
  return `${(load * 100).toFixed(1)}% | ${sorted[Math.floor(times.length * 0.99)].toFixed(3)} ms | ${sorted[times.length - 1].toFixed(3)} ms | ${budget.toFixed(2)} ms`;
};

const chord = [36, 43, 48, 52, 55, 60, 64, 67, 72, 76];
const all = Array.from({ length: 88 }, (_, i) => 21 + i);
const scenarios = [
  ['1 note (C4)', [60], 0],
  ['10-note chord', chord, 0],
  ['10-note chord + pedal resonance', chord, 1],
  ['all 88 keys', all, 0],
  ['all 88 keys + pedal', all, 1],
];
console.log('\n| scenario | voices | strings | CPU (1 core) | quantum p99 | quantum max | budget |');
console.log('|---|---|---|---|---|---|---|');
for (const [name, notes, pedal] of scenarios) {
  x.piano_all_off();
  x.piano_pedal(pedal);
  for (const m of notes) x.piano_note_on(m, 0.8);
  const blocks = Math.floor((4 * fs) / 128);
  const times = new Float64Array(blocks);
  const t = performance.now();
  for (let i = 0; i < blocks; i++) {
    const b = performance.now();
    x.piano_process(128);
    times[i] = performance.now() - b;
  }
  console.log(`| ${name} | ${x.piano_active_voices()} | ${x.piano_active_strings()} | ${stats(times, performance.now() - t)} |`);
}

// The demo songs, sequenced as in web/piano-processor.js.
const GRID = 16;
const songs = ['debussy_Ste_Bergamesq_Clair.mid', 'chp-10-01.mid', 'pathetique-1.mid', 'KV331_3_RondoAllaTurca.mid', 'fur_Elise_WoO59.mid'];
console.log(`\n| song (first ${songSecs} s) | most voices | CPU (1 core) | quantum p99 | quantum max | budget |`);
console.log('|---|---|---|---|---|---|');
for (const file of songs) {
  let data;
  try {
    const buf = await readFile(new URL(`../midi/${file}`, import.meta.url));
    data = parseMidi(buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength));
  } catch {
    continue;
  }
  const ev = data.events.filter((e) => e.type > EV_NOTE_OFF || (e.a >= 21 && e.a <= 108)).map((e) => ({ ...e, frame: Math.round((e.t * fs) / GRID) * GRID }));
  x.piano_all_off();
  const blocks = Math.floor((Math.min(songSecs, data.duration + 2) * fs) / 128);
  const times = new Float64Array(blocks);
  let next = 0;
  let voices = 0;
  const t = performance.now();
  for (let i = 0; i < blocks; i++) {
    const b = performance.now();
    const start = i * 128;
    for (let pos = 0; pos < 128;) {
      while (next < ev.length && ev[next].frame <= start + pos) {
        const e = ev[next++];
        if (e.type === EV_NOTE_ON) x.piano_note_on(e.a, e.b);
        else if (e.type === EV_NOTE_OFF) x.piano_note_off(e.a);
        else if (e.type === EV_PEDAL) x.piano_pedal(e.a);
      }
      const seg = next < ev.length ? Math.min(128 - pos, ev[next].frame - (start + pos)) : 128 - pos;
      x.piano_process(seg);
      pos += seg;
    }
    times[i] = performance.now() - b;
    if (i % 16 === 0) voices = Math.max(voices, x.piano_active_voices());
  }
  console.log(`| ${file} | ${voices} | ${stats(times, performance.now() - t)} |`);
}
