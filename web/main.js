// Main thread: input handling, UI, latency measurement.

import { parseMidi, EV_NOTE_ON, EV_NOTE_OFF, EV_PEDAL } from './midi-file.js';
import { DEMO_SONGS } from './demo-songs.js';

const NOTE_ON = 0, NOTE_OFF = 1, PEDAL = 2, PARAM = 3, ALL_OFF = 4, STATS = 5, SONG = 6, SONG_STOP = 7;
const P_GAIN = 0, P_SYMP = 1, P_ROOM = 3, P_HARDNESS = 4, P_KNOCK = 5, P_LONG = 6, P_LEVELING = 7, P_BASS = 8, P_TUNING = 9;
// The engine costs in proportion to the sample rate, and above 48 kHz there
// is nothing to gain for the ear: a device that runs faster (96 or 192 kHz
// interfaces) is fed through the browser's resampler unless a rate is chosen.
const MAX_AUTO_RATE = 48000;
const MIDI_LO = 21, MIDI_HI = 108;
const A4_MIN = 400, A4_MAX = 480;

// Physical key (event.code) -> semitones above the base C. Layout independent.
const KEYMAP = {
  KeyZ: 0, KeyS: 1, KeyX: 2, KeyD: 3, KeyC: 4, KeyV: 5, KeyG: 6, KeyB: 7, KeyH: 8, KeyN: 9, KeyJ: 10, KeyM: 11,
  Comma: 12, KeyL: 13, Period: 14, Semicolon: 15, Slash: 16,
  KeyQ: 12, Digit2: 13, KeyW: 14, Digit3: 15, KeyE: 16, KeyR: 17, Digit5: 18, KeyT: 19, Digit6: 20, KeyY: 21,
  Digit7: 22, KeyU: 23, KeyI: 24, Digit9: 25, KeyO: 26, Digit0: 27, KeyP: 28, BracketLeft: 29, Equal: 30, BracketRight: 31,
};
const KEYCAP = {
  Comma: ',', Period: '.', Semicolon: ';', Slash: '/', BracketLeft: '[', BracketRight: ']', Equal: '=',
};
const NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
const noteName = (m) => NAMES[m % 12] + (Math.floor(m / 12) - 1);
const $ = (id) => document.getElementById(id);

const state = {
  ctx: null,
  node: null,
  base: 48,              // MIDI note of the Z key
  velocity: 0.6,
  held: new Map(),       // input source id -> midi
  sounding: new Map(),   // midi -> number of sources holding it
  pedal: false,
  pedalSources: new Set(), // 'ShiftLeft', 'ShiftRight', 'midi'
  nextId: 1,
  pending: new Map(),    // note id -> { tEvent, tHandler }
  lat: { input: [], sched: [], total: [] },
};

// --- audio ------------------------------------------------------------------

let wasmModule = null;

// The engine is compiled here, not in the AudioWorklet: see `warmUp`.
async function loadEngine() {
  if (wasmModule) return wasmModule;
  try {
    wasmModule = await WebAssembly.compileStreaming(fetch('piano.wasm'));
  } catch {
    // (a server that does not send the application/wasm type)
    wasmModule = await WebAssembly.compile(await (await fetch('piano.wasm')).arrayBuffer());
  }
  return wasmModule;
}

async function startAudio() {
  if (state.ctx) return;
  $('startBtn').disabled = true;
  $('startMsg').textContent = '';
  const hint = $('latencyHint').value;
  const sr = Number($('sampleRate').value);
  const opts = { latencyHint: hint === '0' ? 0 : hint };
  if (sr) opts.sampleRate = sr;
  let ctx = new AudioContext(opts);
  if (!sr && ctx.sampleRate > MAX_AUTO_RATE) {
    ctx.close();
    ctx = new AudioContext({ ...opts, sampleRate: MAX_AUTO_RATE });
  }
  state.ctx = ctx;
  dropouts.reset();
  try {
    // Keep the context suspended while the engine initialises (~0.2 s of
    // filter design on the audio thread) so the output does not underrun.
    await ctx.suspend();
    const [module] = await Promise.all([loadEngine(), ctx.audioWorklet.addModule('piano-processor.js')]);
    const node = new AudioWorkletNode(ctx, 'piano-processor', {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
      processorOptions: { wasmModule: module },
    });
    await new Promise((resolve) => {
      node.port.onmessage = (e) => {
        if (e.data.type === 'ready') resolve();
      };
    });
    node.port.onmessage = onWorkletMessage;
    node.connect(ctx.destination);
    state.node = node;
    await ctx.resume();
    pushSettings();
    send([STATS, 1]);
    if (!$('start').hidden) {
      // First start: replace the start screen with the main screen.
      $('start').hidden = true;
      $('app').hidden = false;
      labelKeys();
      $('songOpen').focus({ preventScroll: true });
    }
    updateLatencyInfo();
  } catch (err) {
    state.ctx = null;
    // Back to the start screen (also when a restart from the settings failed).
    if ($('advanced').open) $('advanced').close();
    $('app').hidden = true;
    $('start').hidden = false;
    $('startBtn').disabled = false;
    $('startMsg').textContent = 'Failed to initialize: ' + err.message;
    throw err;
  }
}

async function restartAudio() {
  if (!state.ctx) return;
  const old = state.ctx;
  state.ctx = null;
  state.node = null;
  state.held.clear();
  state.sounding.clear();
  state.pending.clear();
  songStopped();
  for (const k of Object.keys(state.lat)) state.lat[k].length = 0;
  document.querySelectorAll('.key.down').forEach((el) => el.classList.remove('down'));
  await old.close();
  await startAudio();
}

function send(msg) {
  state.node?.port.postMessage(msg);
}

// Master gain: slider 0.5 = engine default (0.013), +-20 dB range, times the
// level of the demo song that is loaded (see tools/gen-demo-songs.mjs).
function sendGain() {
  const demo = song.demo >= 0 ? DEMO_SONGS[song.demo].volume : 1;
  send([PARAM, P_GAIN, 0.013 * Math.pow(10, (Number($('volume').value) - 0.5) * 2) * demo]);
}

function pushSettings() {
  sendGain();
  send([PARAM, P_HARDNESS, Number($('hardness').value)]);
  send([PARAM, P_SYMP, Number($('sympathetic').value)]);
  send([PARAM, P_ROOM, Number($('room').value)]);
  send([PARAM, P_KNOCK, Number($('knock').value)]);
  send([PARAM, P_LONG, Number($('longi').value)]);
  send([PARAM, P_LEVELING, $('leveling').checked ? 1 : 0]);
  send([PARAM, P_BASS, Number($('bass').value)]);
  pushTuning();
}

// Reference pitch of A4. While a number is being typed it passes through
// values that are not meant ("4", "44"): only a value in range is applied.
function pushTuning() {
  const el = $('tuning');
  const hz = Number(el.value);
  const ok = el.value !== '' && hz >= A4_MIN && hz <= A4_MAX;
  el.setAttribute('aria-invalid', String(!ok));
  if (ok) send([PARAM, P_TUNING, hz]);
}

// --- notes ------------------------------------------------------------------

// `tEvent` is the DOM event timestamp (same clock as performance.now()).
function noteOn(source, midi, velocity, tEvent) {
  if (midi < MIDI_LO || midi > MIDI_HI || !state.node) return;
  // Send first, do bookkeeping and DOM work afterwards.
  const id = tEvent ? state.nextId++ : 0;
  state.node.port.postMessage([NOTE_ON, midi, velocity, id]);
  if (id) state.pending.set(id, { tEvent, tHandler: performance.now() });
  // A source can hold one note. If it is still registered (its release was
  // never delivered, or the key-down arrived twice), drop the old entry
  // first: otherwise the count below never returns to zero and that note
  // would ring on after every later release.
  const prev = state.held.get(source);
  if (prev !== undefined) {
    const n = (state.sounding.get(prev) ?? 1) - 1;
    if (n > 0) state.sounding.set(prev, n);
    else {
      state.sounding.delete(prev);
      if (prev !== midi) {
        send([NOTE_OFF, prev]);
        keyEls[prev - MIDI_LO].classList.remove('down');
      }
    }
  }
  state.held.set(source, midi);
  state.sounding.set(midi, (state.sounding.get(midi) ?? 0) + 1);
  keyEls[midi - MIDI_LO].classList.add('down');
}

function noteOff(source) {
  const midi = state.held.get(source);
  if (midi === undefined) return;
  state.held.delete(source);
  const n = (state.sounding.get(midi) ?? 1) - 1;
  if (n > 0) {
    state.sounding.set(midi, n);
    return;
  }
  state.sounding.delete(midi);
  send([NOTE_OFF, midi]);
  keyEls[midi - MIDI_LO].classList.remove('down');
}

// The pedal is down while any of its sources (Shift, MIDI) holds it.
function setPedalSource(source, down) {
  if (down) state.pedalSources.add(source);
  else state.pedalSources.delete(source);
  setPedal(state.pedalSources.size > 0);
}

function setPedal(down) {
  if (state.pedal === down) return;
  state.pedal = down;
  send([PEDAL, down ? 1 : 0]);
  $('pedalBtn').setAttribute('aria-pressed', String(down));
}

function releaseAll(hard) {
  for (const source of [...state.held.keys()]) noteOff(source);
  state.pedalSources.clear();
  setPedal(false);
  if (hard) {
    // nothing may stay registered after "all notes off"
    state.held.clear();
    state.sounding.clear();
    document.querySelectorAll('.key.down').forEach((el) => el.classList.remove('down'));
  }
  if (hard) {
    send([ALL_OFF]); // also ends a song in the worklet
    songStopped();
  }
}

function setBase(base) {
  state.base = Math.max(12, Math.min(84, base));
  $('octaveOut').textContent = noteName(state.base);
  $('octaveDown').disabled = state.base <= 12;
  $('octaveUp').disabled = state.base >= 84;
  labelKeys();
}

function setVelocity(v) {
  state.velocity = Math.max(0.05, Math.min(1, v));
  $('velocity').value = state.velocity;
  $('velocityOut').textContent = Math.round(state.velocity * 127);
}

// --- computer keyboard ------------------------------------------------------

addEventListener('keydown', (e) => {
  // Nothing but the start button works until the main screen is shown.
  if (!state.node) return;
  // Command+U (macOS) / Ctrl+U: choose a MIDI file.
  if (e.code === 'KeyU' && (IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey) && !e.altKey && !e.shiftKey) {
    e.preventDefault();
    if (!e.repeat) $('songFile').click();
    return;
  }
  if (e.ctrlKey || e.metaKey || e.altKey) return;
  // Typing a number (digits and "." are also piano keys) must reach the field.
  if (e.target instanceof HTMLInputElement && e.target.type === 'number') return;
  const semis = KEYMAP[e.code];
  if (semis !== undefined) {
    e.preventDefault();
    if (e.repeat) return;
    noteOn(e.code, state.base + semis, state.velocity, e.timeStamp);
    return;
  }
  const onRange = e.target instanceof HTMLInputElement && e.target.type === 'range';
  const onControl = onRange || e.target instanceof HTMLSelectElement || e.target instanceof HTMLButtonElement;
  switch (e.code) {
    case 'Space':
      // Space keeps its native meaning on a focused button, checkbox or select.
      if (e.target instanceof HTMLButtonElement || e.target instanceof HTMLSelectElement
        || (e.target instanceof HTMLInputElement && e.target.type === 'checkbox')) return;
      e.preventDefault(); // (no page scroll)
      if (!e.repeat && !$('advanced').open) toggleSong();
      break;
    case 'ShiftLeft':
    case 'ShiftRight':
      // Holding Shift sustains the notes played (it works as the pedal).
      if (!e.repeat) setPedalSource(e.code, true);
      break;
    case 'ArrowUp':
      if (onControl) return;
      e.preventDefault();
      setBase(state.base + 12);
      break;
    case 'ArrowDown':
      if (onControl) return;
      e.preventDefault();
      setBase(state.base - 12);
      break;
    case 'ArrowRight':
      if (onControl) return;
      e.preventDefault();
      setVelocity(state.velocity + 0.05);
      break;
    case 'ArrowLeft':
      if (onControl) return;
      e.preventDefault();
      setVelocity(state.velocity - 0.05);
      break;
    case 'Escape':
      if ($('advanced').open) return; // Esc only closes the dialog
      releaseAll(true);
      break;
  }
});

addEventListener('keyup', (e) => {
  if (KEYMAP[e.code] !== undefined) noteOff(e.code);
  else if (e.code === 'ShiftLeft' || e.code === 'ShiftRight') {
    // Some platforms report only one key-up when both Shift keys were down.
    if (e.shiftKey) setPedalSource(e.code, false);
    else {
      setPedalSource('ShiftLeft', false);
      setPedalSource('ShiftRight', false);
    }
  }
});

// Keys released while the window is unfocused never produce keyup.
addEventListener('blur', () => releaseAll(false));
document.addEventListener('visibilitychange', () => {
  if (document.hidden) releaseAll(false);
});

// --- on-screen keyboard -----------------------------------------------------

const keyEls = [];

// A mouse click or a tap is often over within a few tens of milliseconds,
// which is too short for the note to speak. Notes played with a pointer are
// therefore held for at least this long; the computer keyboard and MIDI input
// are passed through untouched.
const POINTER_MIN_HOLD_MS = 175;
// pointerId -> { source, t0 }. Every press gets a source of its own, so that a
// release that is still waiting cannot cut off the next note of that pointer.
const pointers = new Map();
let pointerSeq = 0;

function pointerOn(pointerId, midi, tEvent) {
  const source = `p${pointerId}:${pointerSeq++}`;
  pointers.set(pointerId, { source, t0: tEvent });
  noteOn(source, midi, state.velocity, tEvent);
}

function pointerOff(pointerId) {
  const press = pointers.get(pointerId);
  if (!press) return;
  pointers.delete(pointerId);
  const rest = POINTER_MIN_HOLD_MS - (performance.now() - press.t0);
  if (rest > 0) setTimeout(noteOff, rest, press.source);
  else noteOff(press.source);
}

function buildKeyboard() {
  const kb = $('keyboard');
  let lastWhite = null;
  for (let midi = MIDI_LO; midi <= MIDI_HI; midi++) {
    const el = document.createElement('div');
    const black = NAMES[midi % 12].includes('#');
    el.className = black ? 'key black' : 'key';
    el.dataset.midi = midi;
    el.innerHTML = '<span class="hintkey"></span><span class="name"></span>';
    if (!black && midi % 12 === 0) el.querySelector('.name').textContent = noteName(midi);
    if (black) lastWhite.appendChild(el);
    else {
      kb.appendChild(el);
      lastWhite = el;
    }
    keyEls.push(el);
  }
  const midiAt = (e) => {
    const el = document.elementFromPoint(e.clientX, e.clientY)?.closest?.('[data-midi]');
    return el ? Number(el.dataset.midi) : null;
  };
  kb.addEventListener('pointerdown', (e) => {
    const midi = midiAt(e);
    if (midi === null) return;
    e.preventDefault();
    kb.setPointerCapture(e.pointerId);
    pointerOff(e.pointerId); // a stale press of this pointer, if any
    pointerOn(e.pointerId, midi, e.timeStamp);
  });
  // Glissando: follow the pointer while it is down.
  kb.addEventListener('pointermove', (e) => {
    const press = pointers.get(e.pointerId);
    // (the note may have been taken away meanwhile: Esc, window blur)
    if (!press || !state.held.has(press.source)) return;
    const midi = midiAt(e);
    if (midi !== null && midi !== state.held.get(press.source)) {
      pointerOff(e.pointerId);
      pointerOn(e.pointerId, midi, e.timeStamp);
    }
  });
  const up = (e) => pointerOff(e.pointerId);
  kb.addEventListener('pointerup', up);
  kb.addEventListener('pointercancel', up);
  kb.addEventListener('contextmenu', (e) => e.preventDefault());
}

function labelKeys() {
  const byMidi = new Map();
  for (const [code, semis] of Object.entries(KEYMAP)) {
    const cap = KEYCAP[code] ?? code.replace(/^Key|^Digit/, '');
    const midi = state.base + semis;
    byMidi.set(midi, byMidi.has(midi) ? byMidi.get(midi) + cap : cap);
  }
  keyEls.forEach((el, i) => {
    const cap = byMidi.get(i + MIDI_LO);
    el.classList.toggle('mapped', !!cap);
    el.querySelector('.hintkey').textContent = cap ?? '';
  });
  // Keep the mapped range in view when the keyboard scrolls horizontally.
  const first = keyEls[Math.max(0, state.base - MIDI_LO)];
  first?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
}

// --- MIDI (optional) --------------------------------------------------------

async function enableMidi() {
  const btn = $('midiBtn');
  if (!navigator.requestMIDIAccess) {
    btn.textContent = 'Web MIDI is not supported in this browser';
    return;
  }
  try {
    const access = await navigator.requestMIDIAccess();
    const attach = () => {
      let n = 0;
      for (const input of access.inputs.values()) {
        input.onmidimessage = onMidi;
        n++;
      }
      btn.textContent = `MIDI input: ${n} device${n === 1 ? '' : 's'} connected`;
    };
    access.onstatechange = attach;
    attach();
  } catch {
    btn.textContent = 'MIDI access was not permitted';
  }
}

function onMidi(e) {
  const [status, d1, d2] = e.data;
  const cmd = status & 0xf0;
  if (cmd === 0x90 && d2 > 0) noteOn('m' + d1, d1, d2 / 127, e.timeStamp);
  else if (cmd === 0x80 || (cmd === 0x90 && d2 === 0)) noteOff('m' + d1);
  else if (cmd === 0xb0 && d1 === 64) setPedalSource('midi', d2 >= 64);
}

// --- MIDI file playback -------------------------------------------------------
//
// The file is parsed here and handed to the worklet as one list of events;
// the worklet sequences it on the audio clock. This side only mirrors the
// position for the display.

const IS_MAC = /mac|iphone|ipad/i.test(navigator.userAgentData?.platform ?? navigator.platform ?? '');
const song = {
  name: '',
  data: null,     // result of parseMidi(), keys limited to the 88 of the piano
  playing: false,
  paused: false,
  repeat: false,  // start again from the beginning when the end is reached
  demo: -1,       // index into DEMO_SONGS when the song is a demo, else -1
  offset: 0,      // song position [s] at which the current run started (or was paused)
  t0: null,       // AudioContext time of the run's start (null until the worklet confirms)
  ui: 0,          // next event to show on the keyboard
};

// Leaves the DOM alone when nothing changed (a rewrite could be re-announced).
const setText = (el, text) => {
  if (el.textContent !== text) el.textContent = text;
};
const clock = (s) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;

function songShow(pos) {
  const d = song.data;
  if (!d) return;
  // The live region holds only the file description; the running position is
  // in a separate, silent element so that it is not announced every second.
  setText($('songInfo'), `${song.name} — ${clock(d.duration)} (${d.notes} notes)`);
  setText($('songPos'), song.playing || song.paused ? `${clock(pos)} / ${clock(d.duration)}` : '');
  // The icon is aria-hidden: only the word is the button's accessible name.
  setText($('songPlay').querySelector('.icon'), song.playing ? '⏸' : '▶');
  setText($('songPlay').querySelector('.icon').nextSibling, song.playing ? ' Pause ' : ' Play ');
  setText($('songBpm'), d.bpm ? String(Math.round(d.bpm * 10) / 10) : '–');
  $('songPlay').disabled = false;
  $('songStop').disabled = !(song.playing || song.paused);
  demoShow();
}

// Parses a .mid file, limited to the 88 keys of the piano.
function readSong(buffer) {
  const data = parseMidi(buffer);
  data.events = data.events.filter((e) => e.type > EV_NOTE_OFF || (e.a >= MIDI_LO && e.a <= MIDI_HI));
  return data;
}

// There is one song at a time (a file the user chose or a demo): loading a
// new one stops whatever is playing and takes its place.
async function loadSong(name, read, demo = -1) {
  try {
    const data = await read();
    stopSong();
    song.data = data;
    song.name = name;
    song.demo = demo;
    sendGain();
    songShow(0);
    await playSong();
  } catch (err) {
    $('songPos').textContent = '';
    $('songInfo').textContent = `Cannot read ${name}: ${err.message}`;
  }
}

const openSong = (file) => loadSong(file.name, async () => readSong(await file.arrayBuffer()));

// --- demo songs ---------------------------------------------------------------
//
// The .mid files are embedded in demo-songs.js (base64), so that they play
// without choosing a file. A demo goes through the same player as a file.

const demoBtns = [];
const demoData = []; // parsed on first use

function buildDemos() {
  const list = $('demoList');
  DEMO_SONGS.forEach((s, i) => {
    const li = document.createElement('li');
    li.className = 'demo';
    li.innerHTML = `<button type="button"><span class="icon" aria-hidden="true">▶</span> <span class="word">Play</span></button>
      <div class="demo-text">
        <div class="demo-title"></div>
        <div class="demo-sub"><span class="composer"></span> / <a download>Download MIDI file</a></div>
      </div>`;
    li.querySelector('.demo-title').textContent = s.title;
    li.querySelector('.composer').textContent = s.composer;
    const a = li.querySelector('a');
    a.href = s.url;
    const btn = li.querySelector('button');
    btn.addEventListener('click', (e) => {
      if (e.detail > 0) btn.blur(); // keep the computer keyboard playing
      toggleDemo(i);
    });
    demoBtns.push(btn);
    list.appendChild(li);
  });
}

// Play / Stop of a demo. Stop while it plays; otherwise start it from the
// beginning (ending any other song first).
function toggleDemo(i) {
  if (song.demo === i && song.playing) {
    stopSong();
    return;
  }
  const s = DEMO_SONGS[i];
  loadSong(s.title, () => {
    demoData[i] ??= readSong(Uint8Array.from(atob(s.data), (c) => c.charCodeAt(0)).buffer);
    return demoData[i];
  }, i);
}

let demoShown = -1; // the demo whose button says "Stop"
function demoShow() {
  const now = song.playing ? song.demo : -1;
  if (now === demoShown) return;
  demoShown = now;
  demoBtns.forEach((btn, i) => {
    btn.querySelector('.icon').textContent = i === now ? '■' : '▶';
    btn.querySelector('.word').textContent = i === now ? 'Stop' : 'Play';
  });
}

// Starts (or, after a pause, resumes) the song at `from` seconds.
async function playSong(from = 0) {
  if (!song.data) return;
  if (!state.node) await startAudio();
  const fs = state.ctx.sampleRate;
  const ev = song.data.events;
  let first = 0;
  let pedal = 0; // pedal state the song had reached at `from`
  for (; first < ev.length && ev[first].t < from; first++) {
    if (ev[first].type === EV_PEDAL) pedal = ev[first].a;
  }
  const lead = pedal ? 1 : 0;
  const flat = new Float64Array((ev.length - first + lead) * 4);
  if (lead) flat.set([0, EV_PEDAL, 1, 0], 0);
  for (let i = first; i < ev.length; i++) {
    const e = ev[i];
    flat.set([Math.round((e.t - from) * fs), e.type, e.a, e.b], (i - first + lead) * 4);
  }
  songStopped();
  song.playing = true;
  song.offset = from;
  song.ui = first;
  send([SONG, flat, Math.round(0.1 * fs)]);
  songShow(from);
}

// Pause: the worklet lets go of the song's keys; the position is kept here and
// playback resumes from it (notes that were sounding are not struck again).
function pauseSong() {
  if (!song.playing) return;
  const run = song.t0 === null ? 0 : Math.max(0, state.ctx.currentTime - song.t0);
  send([SONG_STOP]);
  const at = Math.min(song.data.duration, song.offset + run);
  songStopped();
  song.paused = true;
  song.offset = at;
  songShow(at);
}

// Play / Pause (the button and the Space key).
function toggleSong() {
  if (!song.data) return;
  if (song.playing) pauseSong();
  else playSong(song.paused ? song.offset : 0);
}

function stopSong() {
  if (song.playing) send([SONG_STOP]);
  songStopped();
}

// The song is over (or was stopped): reset the display.
function songStopped() {
  song.playing = false;
  song.paused = false;
  song.offset = 0;
  song.t0 = null;
  song.ui = 0;
  document.querySelectorAll('.key.play').forEach((el) => el.classList.remove('play'));
  songShow(0);
}

// Shows the keys the song is pressing at the moment they become audible.
function songTick() {
  if (!song.playing || song.t0 === null || !state.ctx) return;
  const ctx = state.ctx;
  const pos = song.offset + ctx.currentTime - song.t0 - ctx.baseLatency - (ctx.outputLatency || 0);
  const ev = song.data.events;
  for (; song.ui < ev.length && ev[song.ui].t <= pos; song.ui++) {
    const e = ev[song.ui];
    if (e.type === EV_NOTE_ON) keyEls[e.a - MIDI_LO].classList.add('play');
    else if (e.type === EV_NOTE_OFF) keyEls[e.a - MIDI_LO].classList.remove('play');
  }
  songShow(Math.max(0, Math.min(song.data.duration, pos)));
}

// --- latency measurement ----------------------------------------------------

function onWorkletMessage(e) {
  const m = e.data;
  if (m.type === 'ack') {
    const tAck = performance.now();
    const ctx = state.ctx;
    if (!ctx) return;
    // getOutputTimestamp(): the sample at `contextTime` leaves the output
    // device at `performanceTime`. Extrapolate to the note's first frame.
    const ts = ctx.getOutputTimestamp();
    const tAudible = ts.performanceTime + (m.frame / ctx.sampleRate - ts.contextTime) * 1000;
    for (const id of m.ids) {
      const p = state.pending.get(id);
      if (!p) continue;
      state.pending.delete(id);
      record('input', p.tHandler - p.tEvent);
      record('sched', tAck - p.tHandler);
      if (ts.performanceTime > 0) record('total', tAudible - p.tEvent);
    }
  } else if (m.type === 'stats') {
    stats = m;
  } else if (m.type === 'songStart') {
    if (song.playing && state.ctx) song.t0 = m.frame / state.ctx.sampleRate;
  } else if (m.type === 'songEnd') {
    // (an end that crosses a Stop / Pause click must not start the song again)
    if (song.repeat && song.playing) playSong(0);
    else if (song.playing) songStopped();
  }
}

function record(kind, v) {
  const a = state.lat[kind];
  a.push(v);
  if (a.length > 50) a.shift();
}

function summary(a) {
  if (!a.length) return null;
  const s = [...a].sort((x, y) => x - y);
  return { n: s.length, median: s[Math.floor(s.length / 2)], p95: s[Math.min(s.length - 1, Math.floor(s.length * 0.95))] };
}

const fmt = (a) => {
  const s = summary(a);
  return s ? `${s.median.toFixed(1)} ms / ${s.p95.toFixed(1)} ms (n=${s.n})` : '–';
};

function latencyReport() {
  const ctx = state.ctx;
  if (!ctx) return null;
  const quantum = (128 / ctx.sampleRate) * 1000;
  const ps = ctx.playbackStats;
  // How far the render head is ahead of what is audible right now.
  const ts = ctx.getOutputTimestamp();
  const lag = ts.performanceTime > 0
    ? (ctx.currentTime - (ts.contextTime + (performance.now() - ts.performanceTime) / 1000)) * 1000
    : NaN;
  return {
    renderToAudibleMs: lag,
    sampleRate: ctx.sampleRate,
    quantumMs: quantum,
    latencyHint: $('latencyHint').value,
    baseLatencyMs: ctx.baseLatency * 1000,
    outputLatencyMs: (ctx.outputLatency ?? NaN) * 1000,
    input: summary(state.lat.input),
    sched: summary(state.lat.sched),
    total: summary(state.lat.total),
    playbackStats: ps ? { underrunEvents: ps.underrunEvents, underrunDuration: ps.underrunDuration, averageLatency: ps.averageLatency } : null,
  };
}
window.pianoLatencyReport = latencyReport; // for scripted measurements
window.pianoDebug = { state, song, noteOn, noteOff, setPedal: (down) => setPedalSource('debug', down) };

function updateLatencyInfo() {
  const r = latencyReport();
  if (!r) return;
  $('latSr').textContent = `${r.sampleRate} Hz / 128 frames = ${r.quantumMs.toFixed(2)} ms`;
  $('latBase').textContent = `${r.baseLatencyMs.toFixed(2)} ms`;
  $('latOut').textContent = Number.isFinite(r.outputLatencyMs) ? `${r.outputLatencyMs.toFixed(2)} ms` : 'unavailable';
  $('latLag').textContent = Number.isFinite(r.renderToAudibleMs) ? `approx. ${r.renderToAudibleMs.toFixed(0)} ms` : 'unavailable';
  $('latInput').textContent = fmt(state.lat.input);
  $('latSched').textContent = fmt(state.lat.sched);
  $('latTotal').textContent = fmt(state.lat.total);
  $('latUnderrun').textContent = r.playbackStats
    ? `${r.playbackStats.underrunEvents} (${(r.playbackStats.underrunDuration * 1000).toFixed(0)} ms)`
    : 'unavailable (playbackStats not supported)';
}

// --- dropouts -----------------------------------------------------------------
//
// When the audio thread cannot finish a render quantum in time, the output
// has a gap (heard as a click or a stutter). The page cannot repair that, but
// it can say so and name the remedy: a larger output buffer, which trades
// latency for slack. Called with every statistics report of the worklet
// (5 per second).

const dropouts = {
  t0: 0,        // performance.now() when counting started
  seen: [],     // { t, n }: underrun count of the last 10 s (where the browser reports it)
  hot: 0,       // consecutive reports with the render load near the limit
  reset() {
    this.t0 = performance.now();
    this.seen.length = 0;
    this.hot = 0;
    $('dropoutMsg').hidden = true;
  },
  check(load) {
    const t = performance.now();
    // (the first seconds hold the engine's warm-up, which is muted)
    if (t - this.t0 < 3000 || !$('dropoutMsg').hidden) return;
    const ps = state.ctx?.playbackStats;
    let bad = false;
    if (ps) {
      this.seen.push({ t, n: ps.underrunEvents });
      while (t - this.seen[0].t > 10000) this.seen.shift();
      bad = ps.underrunEvents - this.seen[0].n >= 3;
    } else {
      this.hot = load > 0.8 ? this.hot + 1 : 0;
      bad = this.hot >= 5;
    }
    if (bad) $('dropoutMsg').hidden = false;
  },
};

// --- UI ---------------------------------------------------------------------

let stats = null;

function uiTick() {
  if (stats) {
    $('voicesOut').textContent = `${stats.voices} keys / ${stats.strings} strings`;
    $('level').value = Math.min(1, stats.peak);
    $('loadBox').hidden = false;
    // (the longest quantum is only known where the worklet has a fine clock)
    $('loadOut').textContent = `${(stats.load * 100).toFixed(1)} %` + (stats.worst >= 0 ? ` (max ${(stats.worst * 100).toFixed(0)} %)` : '');
    dropouts.check(stats.load);
    stats = null;
    if ($('advanced').open) updateLatencyInfo();
  }
  songTick();
  requestAnimationFrame(uiTick);
}

function bindControls() {
  const show = () => {
    $('volumeOut').textContent = `${((Number($('volume').value) - 0.5) * 40).toFixed(0)} dB`;
    $('hardnessOut').textContent = `×${Number($('hardness').value).toFixed(2)}`;
    $('sympatheticOut').textContent = `×${Number($('sympathetic').value).toFixed(2)}`;
    $('roomOut').textContent = `${Math.round(Number($('room').value) * 100)} %`;
    $('knockOut').textContent = `×${Number($('knock').value).toFixed(2)}`;
    $('longiOut').textContent = `×${Number($('longi').value).toFixed(2)}`;
    $('bassOut').textContent = Number($('bass').value).toFixed(2);
  };
  for (const id of ['volume', 'hardness', 'sympathetic', 'room', 'knock', 'longi', 'leveling', 'bass']) {
    $(id).addEventListener('input', () => {
      show();
      pushSettings();
    });
  }
  $('velocity').addEventListener('input', (e) => setVelocity(Number(e.target.value)));
  $('latencyHint').addEventListener('change', restartAudio);
  $('sampleRate').addEventListener('change', restartAudio);
  $('startBtn').addEventListener('click', startAudio);
  for (const [id, step] of [['octaveDown', -12], ['octaveUp', 12]]) {
    $(id).addEventListener('click', (e) => {
      setBase(state.base + step);
      // After a mouse/touch click give the focus back to the page so that
      // the arrow keys and Space keep playing (keyboard activation keeps it).
      if (e.detail > 0) e.currentTarget.blur();
    });
  }
  // Retuning redesigns the strings of all keys: wait until the typing pauses.
  let tuningTimer = 0;
  $('tuning').addEventListener('input', () => {
    clearTimeout(tuningTimer);
    tuningTimer = setTimeout(pushTuning, 250);
  });
  // The pedal button latches: the pedal stays down until it is clicked again.
  $('pedalBtn').addEventListener('click', (e) => {
    setPedalSource('button', !state.pedalSources.has('button'));
    if (e.detail > 0) e.currentTarget.blur();
  });
  $('allOff').addEventListener('click', (e) => {
    releaseAll(true);
    if (e.detail > 0) e.currentTarget.blur();
  });
  $('midiBtn').addEventListener('click', enableMidi);
  $('advancedBtn').addEventListener('click', () => {
    updateLatencyInfo();
    $('advanced').showModal();
  });
  const shortcut = IS_MAC ? '⌘U' : 'Ctrl+U';
  $('songKey').textContent = shortcut;
  $('songOpen').setAttribute('aria-keyshortcuts', IS_MAC ? 'Meta+U' : 'Control+U');
  const afterClick = (e) => {
    if (e.detail > 0) e.currentTarget.blur(); // keep the computer keyboard playing
  };
  $('songOpen').addEventListener('click', (e) => {
    $('songFile').click();
    afterClick(e);
  });
  $('songFile').addEventListener('change', (e) => {
    const file = e.target.files[0];
    e.target.value = ''; // choosing the same file again must fire `change` too
    if (file) openSong(file);
  });
  $('songPlay').addEventListener('click', (e) => {
    afterClick(e);
    toggleSong();
  });
  $('songRepeat').addEventListener('click', (e) => {
    afterClick(e);
    song.repeat = !song.repeat;
    e.currentTarget.setAttribute('aria-pressed', String(song.repeat));
  });
  $('songStop').addEventListener('click', (e) => {
    afterClick(e);
    stopSong();
  });
  $('latCopy').addEventListener('click', async () => {
    await navigator.clipboard?.writeText(JSON.stringify(latencyReport(), null, 2));
    $('latCopy').textContent = 'Copied';
  });
  show();
}

buildKeyboard();
buildDemos();
bindControls();
setBase(48);
setVelocity(0.6);
requestAnimationFrame(uiTick);
