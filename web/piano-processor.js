// AudioWorklet side: hosts the WASM physical model.
//
// Messages are plain arrays to keep allocation (and GC on the audio thread) minimal:
//   [0, midi, velocity, id]  note on   (id > 0: reply with the frame at which it starts)
//   [1, midi]                note off
//   [2, down]                sustain pedal
//   [3, paramId, value]      engine parameter (see piano_set_param in engine/src/wasm.rs)
//   [4]                      all notes off
//   [5, enabled]             periodic stats
//   [6, events, delay]       play a song: Float64Array of (frame, type, a, b) per event,
//                            frames counted from `delay` frames after now
//   [7]                      stop the song
//
// A song is sequenced here, on the audio clock, so its timing does not depend
// on main-thread timers. It only calls the same note/pedal entry points of the
// engine as live playing does.

const NOTE_ON = 0, NOTE_OFF = 1, PEDAL = 2, PARAM = 3, ALL_OFF = 4, STATS = 5, SONG = 6, SONG_STOP = 7;
// Song events are aligned to this many frames (0.3 ms) to bound the number of
// partial blocks rendered per quantum.
const SONG_GRID = 16;

class PianoProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    // (compiled on the main thread, see main.js)
    const instance = new WebAssembly.Instance(options.processorOptions.wasmModule, {});
    this.x = instance.exports;
    this.x.piano_init(sampleRate);
    this.bindViews();
    // Warm-up: exercise every code path (hammer contact, all strings,
    // soundboard) so the JIT has produced optimised code before the first real
    // key press. It is spread over the first quanta (output muted) instead of
    // blocking the constructor, which would cause output underruns.
    this.warm = 160;
    this.x.piano_pedal(1);
    for (let m = 21; m <= 108; m++) this.x.piano_note_on(m, 0.5 + 0.005 * m);

    this.acks = [];          // note-on ids waiting for their start frame
    this.song = null;        // { ev, i, start }
    this.songKeys = new Uint8Array(128); // keys held down by the song
    this.songPedal = false;
    this.livePedal = false;
    this.stats = false;
    this.statsCountdown = 0;
    this.peak = 0;
    // Render time: a high resolution clock where the browser offers one in
    // this scope. Date.now() has a resolution of 1 ms, about the length of
    // a quantum: summed over the many quanta of a report it still gives the
    // average load, but not the longest quantum.
    this.fine = typeof performance !== 'undefined' && typeof performance.now === 'function';
    this.now = this.fine ? () => performance.now() : () => Date.now();
    this.busy = 0;           // accumulated render time [ms]
    this.busyMax = 0;
    this.frames = 0;

    this.port.onmessage = (e) => {
      const m = e.data;
      if (this.warm && (m[0] === NOTE_ON || m[0] === PEDAL || m[0] === SONG)) this.endWarmUp();
      switch (m[0]) {
        case NOTE_ON:
          this.x.piano_note_on(m[1], m[2]);
          if (m[3]) this.acks.push(m[3]);
          break;
        case NOTE_OFF: this.x.piano_note_off(m[1]); break;
        case PEDAL:
          this.livePedal = !!m[1];
          this.x.piano_pedal(this.livePedal || this.songPedal ? 1 : 0);
          break;
        case PARAM: this.x.piano_set_param(m[1], m[2]); break;
        case ALL_OFF:
          this.stopSong();
          this.livePedal = false;
          this.x.piano_all_off();
          break;
        case SONG: {
          this.stopSong();
          const start = Math.ceil((currentFrame + m[2]) / SONG_GRID) * SONG_GRID;
          this.song = { ev: m[1], i: 0, start };
          this.port.postMessage({ type: 'songStart', frame: start });
          break;
        }
        case SONG_STOP: this.stopSong(); break;
        case STATS: this.stats = !!m[1]; break;
      }
    };
    this.port.postMessage({ type: 'ready', sampleRate });
  }

  endWarmUp() {
    this.warm = 0;
    this.x.piano_all_off();
  }

  // Lets go of everything the song holds; live notes are not touched.
  stopSong() {
    this.song = null;
    for (let k = 0; k < 128; k++) {
      if (this.songKeys[k]) {
        this.songKeys[k] = 0;
        this.x.piano_note_off(k);
      }
    }
    if (this.songPedal) {
      this.songPedal = false;
      this.x.piano_pedal(this.livePedal ? 1 : 0);
    }
  }

  // Dispatches the song events due at `frame`; returns the number of frames
  // until the next one (Infinity when the song is over).
  runSong(frame) {
    const { ev, start } = this.song;
    let i = this.song.i;
    for (; i < ev.length; i += 4) {
      const at = start + Math.round(ev[i] / SONG_GRID) * SONG_GRID;
      if (at > frame) {
        this.song.i = i;
        return at - frame;
      }
      const a = ev[i + 2];
      switch (ev[i + 1]) {
        case NOTE_ON:
          this.songKeys[a] = 1;
          this.x.piano_note_on(a, ev[i + 3]);
          break;
        case NOTE_OFF:
          this.songKeys[a] = 0;
          this.x.piano_note_off(a);
          break;
        case PEDAL:
          this.songPedal = !!a;
          this.x.piano_pedal(this.livePedal || this.songPedal ? 1 : 0);
          break;
      }
    }
    this.stopSong();
    this.port.postMessage({ type: 'songEnd' });
    return Infinity;
  }

  // Views on the engine's output. One per segment length that occurs (a
  // quantum, or a part of one up to a song event): `subarray` in process()
  // would allocate on every call, and garbage collection on the audio
  // thread is heard as a dropout on a slow machine.
  bindViews() {
    this.buf = this.x.memory.buffer;
    this.views = new Map();
  }

  view(seg) {
    let v = this.views.get(seg);
    if (!v) {
      v = [new Float32Array(this.buf, this.x.piano_out_l(), seg), new Float32Array(this.buf, this.x.piano_out_r(), seg)];
      this.views.set(seg, v);
    }
    return v;
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    const n = out[0].length;
    if (this.warm) {
      this.x.piano_process(n);
      this.x.piano_process(n);
      if (--this.warm === 0) this.endWarmUp();
      return true; // outputs stay silent
    }
    if (this.acks.length) {
      // Notes received since the previous quantum start sounding at this frame.
      this.port.postMessage({ type: 'ack', ids: this.acks, frame: currentFrame });
      this.acks = [];
    }
    const t0 = this.stats ? this.now() : 0;
    // Render up to each song event, so that it starts on its own frame.
    for (let pos = 0; pos < n;) {
      let seg = n - pos;
      if (this.song) seg = Math.min(seg, this.runSong(currentFrame + pos));
      this.x.piano_process(seg);
      if (this.buf !== this.x.memory.buffer) this.bindViews(); // memory grew
      const v = this.view(seg);
      out[0].set(v[0], pos);
      if (out.length > 1) out[1].set(v[1], pos);
      pos += seg;
    }
    if (this.stats) {
      const dt = this.now() - t0;
      this.busy += dt;
      if (dt > this.busyMax) this.busyMax = dt;
      this.frames += n;
      const l = out[0];
      let pk = this.peak;
      for (let i = 0; i < n; i += 4) {
        const a = l[i] < 0 ? -l[i] : l[i];
        if (a > pk) pk = a;
      }
      this.peak = pk;
      this.statsCountdown -= n;
      if (this.statsCountdown <= 0) {
        this.statsCountdown = sampleRate / 5;
        this.port.postMessage({
          type: 'stats',
          voices: this.x.piano_active_voices(),
          strings: this.x.piano_active_strings(),
          peak: this.peak,
          load: this.busy / (1000 * this.frames / sampleRate),
          worst: this.fine ? this.busyMax / (1000 * n / sampleRate) : -1,
        });
        this.peak = 0;
        this.busy = 0;
        this.busyMax = 0;
        this.frames = 0;
      }
    }
    return true;
  }
}

registerProcessor('piano-processor', PianoProcessor);
