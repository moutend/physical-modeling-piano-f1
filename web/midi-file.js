// Standard MIDI File (.mid) reader. No dependencies; knows nothing about the
// piano engine. All channels are merged into one piano part (the percussion
// channel is skipped).

export const EV_NOTE_ON = 0, EV_NOTE_OFF = 1, EV_PEDAL = 2;

/**
 * @param {ArrayBuffer} buffer  contents of a .mid file
 * @returns {{ events: {t: number, type: number, a: number, b: number}[], duration: number, notes: number, name: string, bpm: number | null }}
 *   `t` in seconds from the start; note on: a = key, b = velocity 0..1;
 *   note off: a = key; pedal: a = 1 (down) or 0 (up).
 *   `bpm` is the tempo recorded in the file (the one in effect at the first
 *   note), or null when the file has no tempo event.
 */
export function parseMidi(buffer) {
  const d = new DataView(buffer);
  const bytes = new Uint8Array(buffer);
  const tag = (p) => String.fromCharCode(...bytes.subarray(p, p + 4));
  // Some files are wrapped in a RIFF container (.rmi): find the header chunk.
  let p = 0;
  if (bytes.length >= 4 && tag(0) === 'RIFF') {
    while (p + 4 <= bytes.length && tag(p) !== 'MThd') p++;
  }
  if (bytes.length < p + 14 || tag(p) !== 'MThd') throw new Error('not a MIDI file');
  const headerLen = d.getUint32(p + 4);
  const format = d.getUint16(p + 8);
  const division = d.getUint16(p + 12);
  if (format > 2) throw new Error(`unsupported MIDI format (format ${format})`);
  p += 8 + headerLen;

  // Pass 1: every track to a list of { tick, order, ... }.
  const raw = [];     // channel events
  const tempos = [];  // { tick, usPerQuarter }
  let name = '';
  let order = 0;
  let trackOffset = 0; // format 2: the tracks follow one another
  while (p + 8 <= bytes.length) {
    const id = tag(p);
    const len = d.getUint32(p + 4);
    const end = Math.min(bytes.length, p + 8 + len);
    p += 8;
    if (id !== 'MTrk') {
      p = end;
      continue;
    }
    let tick = trackOffset;
    let status = 0;
    const varLen = () => {
      let v = 0;
      for (let i = 0; i < 4 && p < end; i++) {
        const b = bytes[p++];
        v = v * 128 + (b & 0x7f);
        if (b < 0x80) break;
      }
      return v;
    };
    while (p < end) {
      tick += varLen();
      if (p >= end) break;
      let b = bytes[p];
      if (b >= 0x80) {
        p++;
        if (b < 0xf0) status = b;
      } else {
        if (!status) throw new Error('the MIDI data is corrupted');
        b = status; // running status
      }
      if (b === 0xff) {
        const type = bytes[p++];
        const n = varLen();
        if (type === 0x51 && n === 3) {
          tempos.push({ tick, order: order++, us: (bytes[p] << 16) | (bytes[p + 1] << 8) | bytes[p + 2] });
        } else if (type === 0x03 && !name && n > 0) {
          try {
            name = new TextDecoder().decode(bytes.subarray(p, Math.min(end, p + n)));
          } catch { /* not text */ }
        }
        p += n;
        if (type === 0x2f) break; // end of track
      } else if (b === 0xf0 || b === 0xf7) {
        p += varLen(); // system exclusive
      } else if (b >= 0xf0) {
        // system common / real time (should not occur in a file)
        p += b === 0xf2 ? 2 : b === 0xf3 ? 1 : 0;
      } else {
        const cmd = b & 0xf0;
        const ch = b & 0x0f;
        const d1 = bytes[p++];
        const d2 = cmd === 0xc0 || cmd === 0xd0 ? 0 : bytes[p++];
        if (ch === 9) continue; // percussion
        if (cmd === 0x90 && d2 > 0) raw.push({ tick, order: order++, type: EV_NOTE_ON, a: d1, b: d2 / 127, ch });
        else if (cmd === 0x80 || cmd === 0x90) raw.push({ tick, order: order++, type: EV_NOTE_OFF, a: d1, b: 0, ch });
        else if (cmd === 0xb0 && d1 === 64) raw.push({ tick, order: order++, type: EV_PEDAL, a: d2 >= 64 ? 1 : 0, b: 0, ch });
      }
    }
    if (format === 2) trackOffset = tick;
    p = end;
  }

  // Pass 2: ticks to seconds.
  const byTick = (x, y) => x.tick - y.tick || x.order - y.order;
  raw.sort(byTick);
  tempos.sort(byTick);
  let toSeconds;
  let bpm = null;
  if (!(division & 0x8000) && tempos.length) {
    const firstNote = raw.find((e) => e.type === EV_NOTE_ON)?.tick ?? 0;
    let us = tempos[0].us;
    for (const t of tempos) if (t.tick <= firstNote) us = t.us;
    if (us > 0) bpm = 60e6 / us;
  }
  if (division & 0x8000) {
    // SMPTE time code: frames per second and ticks per frame, no tempo map
    const fps = 256 - (division >> 8);
    const perSecond = (fps === 29 ? 29.97 : fps) * (division & 0xff);
    toSeconds = (tick) => tick / perSecond;
  } else {
    if (!division) throw new Error('the MIDI data is corrupted');
    let ti = 0, lastTick = 0, lastSec = 0, us = 500000;
    toSeconds = (tick) => {
      // called with non-decreasing ticks
      while (ti < tempos.length && tempos[ti].tick <= tick) {
        lastSec += ((tempos[ti].tick - lastTick) * us) / division / 1e6;
        lastTick = tempos[ti].tick;
        us = tempos[ti].us;
        ti++;
      }
      return lastSec + ((tick - lastTick) * us) / division / 1e6;
    };
  }

  // Pass 3: merge the channels into one keyboard. A key or the pedal goes up
  // only when every channel that holds it has let go.
  const events = [];
  const held = new Map();   // key -> number of note-ons not yet released
  const pedals = new Set(); // channels holding the pedal
  let notes = 0;
  let t0 = -1;
  for (const e of raw) {
    let t = toSeconds(e.tick);
    if (e.type === EV_NOTE_ON) {
      if (t0 < 0) t0 = Math.max(0, t - 0.25); // skip a long silent lead-in
      held.set(e.a, (held.get(e.a) ?? 0) + 1);
      notes++;
    } else if (e.type === EV_NOTE_OFF) {
      const n = held.get(e.a) ?? 0;
      if (n === 0) continue;
      held.set(e.a, n - 1);
      if (n > 1) continue;
    } else {
      const was = pedals.size > 0;
      if (e.a) pedals.add(e.ch);
      else pedals.delete(e.ch);
      if (was === pedals.size > 0) continue;
    }
    t = Math.max(0, t - Math.max(0, t0));
    events.push({ t, type: e.type, a: e.a, b: e.b });
  }
  if (!notes) throw new Error('the file contains no notes');
  // Whatever the file left hanging is released at the end.
  let last = events[events.length - 1].t;
  for (const [key, n] of held) if (n > 0) events.push({ t: last, type: EV_NOTE_OFF, a: key, b: 0 });
  if (pedals.size) events.push({ t: last, type: EV_PEDAL, a: 0, b: 0 });
  return { events, duration: last, notes, name, bpm };
}
