//! Offline renderer.
//!
//!   render demo out.wav [fs]                 short demo (scale, chords, pedal)
//!   render note out.wav <midi> [vel] [secs] [fs]
//!   render midi out.wav <file.mid> [fs]      a standard MIDI file (for comparisons with recordings)

use piano_engine::engine::Engine;
use piano_engine::wav::write_wav;

struct Ev {
    t: f64,
    kind: u8, // 0 on, 1 off, 2 pedal down, 3 pedal up
    midi: usize,
    vel: f64,
}

fn play(e: &mut Engine, mut evs: Vec<Ev>, total: f64) -> (Vec<f32>, Vec<f32>) {
    evs.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
    let n = (total * e.fs) as usize;
    let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let (mut done, mut next) = (0, 0);
    while done < n {
        while next < evs.len() && evs[next].t * e.fs <= done as f64 {
            let ev = &evs[next];
            match ev.kind {
                0 => e.note_on(ev.midi, ev.vel),
                1 => e.note_off(ev.midi),
                2 => e.set_pedal(true),
                _ => e.set_pedal(false),
            }
            next += 1;
        }
        let len = 64.min(n - done);
        e.process(len);
        l.extend_from_slice(&e.out_l[..len]);
        r.extend_from_slice(&e.out_r[..len]);
        done += len;
    }
    (l, r)
}

fn note(evs: &mut Vec<Ev>, t: f64, dur: f64, midi: usize, vel: f64) {
    evs.push(Ev { t, kind: 0, midi, vel });
    evs.push(Ev { t: t + dur, kind: 1, midi, vel: 0.0 });
}

/// Minimal standard MIDI file reader: note on/off and sustain pedal of all
/// channels, with the tempo map applied. Returns the events and the duration.
fn read_midi(path: &str) -> (Vec<Ev>, f64) {
    let b = std::fs::read(path).expect("cannot read MIDI file");
    assert!(b.len() > 14 && &b[..4] == b"MThd", "not a MIDI file");
    let be = |p: usize, n: usize| b[p..p + n].iter().fold(0usize, |a, &x| a * 256 + x as usize);
    let div = be(12, 2) as f64;
    let mut p = 8 + be(4, 4);
    let mut raw: Vec<(usize, usize, u8, u8, u8)> = vec![];
    let mut tempos: Vec<(usize, f64)> = vec![];
    while p + 8 <= b.len() {
        let end = (p + 8 + be(p + 4, 4)).min(b.len());
        let is_track = &b[p..p + 4] == b"MTrk";
        p += 8;
        if is_track {
            let (mut tick, mut status) = (0usize, 0u8);
            let var = |p: &mut usize| {
                let mut v = 0usize;
                while *p < end {
                    let c = b[*p];
                    *p += 1;
                    v = v * 128 + (c & 0x7f) as usize;
                    if c < 0x80 {
                        break;
                    }
                }
                v
            };
            while p < end {
                tick += var(&mut p);
                if p >= end {
                    break;
                }
                if b[p] >= 0x80 {
                    status = b[p];
                    p += 1;
                }
                if status == 0xff {
                    let kind = b[p];
                    p += 1;
                    let len = var(&mut p);
                    if kind == 0x51 && len == 3 {
                        tempos.push((tick, be(p, 3) as f64));
                    }
                    p += len;
                } else if status == 0xf0 || status == 0xf7 {
                    let len = var(&mut p);
                    p += len;
                } else {
                    let n = if matches!(status & 0xf0, 0xc0 | 0xd0) { 1 } else { 2 };
                    if p + n > end {
                        break;
                    }
                    raw.push((tick, raw.len(), status, b[p], if n == 2 { b[p + 1] } else { 0 }));
                    p += n;
                }
            }
        }
        p = end;
    }
    raw.sort();
    tempos.sort_by(|a, b| a.0.cmp(&b.0));
    let secs = |tick: usize| {
        let (mut t, mut last, mut us) = (0.0, 0usize, 500000.0);
        for &(tk, u) in &tempos {
            if tk >= tick {
                break;
            }
            t += (tk - last) as f64 * us / 1e6 / div;
            last = tk;
            us = u;
        }
        t + (tick - last) as f64 * us / 1e6 / div
    };
    let mut evs = vec![];
    let mut end_t: f64 = 0.0;
    for &(tick, _, st, d1, d2) in &raw {
        let t = secs(tick);
        end_t = end_t.max(t);
        let midi = d1 as usize;
        match st & 0xf0 {
            0x90 if d2 > 0 => evs.push(Ev { t, kind: 0, midi, vel: d2 as f64 / 127.0 }),
            0x80 | 0x90 => evs.push(Ev { t, kind: 1, midi, vel: 0.0 }),
            0xb0 if d1 == 64 => evs.push(Ev { t, kind: if d2 >= 64 { 2 } else { 3 }, midi: 0, vel: 0.0 }),
            _ => {}
        }
    }
    (evs, end_t)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("demo");
    let out = args.get(2).cloned().unwrap_or_else(|| "out.wav".into());
    let num = |i: usize, d: f64| args.get(i).and_then(|s| s.parse::<f64>().ok()).unwrap_or(d);
    match cmd {
        "note" => {
            let fs = num(6, 48000.0);
            let mut e = Engine::new(fs);
            let secs = num(5, 4.0);
            let mut evs = vec![];
            note(&mut evs, 0.0, secs - 1.0, num(3, 60.0) as usize, num(4, 0.6));
            let (l, r) = play(&mut e, evs, secs);
            write_wav(&out, fs as u32, &l, &r).unwrap();
        }
        "midi" => {
            let fs = num(4, 48000.0);
            let mut e = Engine::new(fs);
            let (evs, dur) = read_midi(&args[3]);
            let (l, r) = play(&mut e, evs, dur + 3.0);
            write_wav(&out, fs as u32, &l, &r).unwrap();
        }
        _ => {
            let fs = num(3, 48000.0);
            let mut e = Engine::new(fs);
            let mut evs = vec![];
            let mut t = 0.2;
            // 1. single notes across the range
            for &m in &[21, 33, 45, 57, 60, 69, 81, 93, 105, 108] {
                note(&mut evs, t, 1.2, m, 0.6);
                t += 1.4;
            }
            // 2. dynamics on C4
            for &v in &[0.15, 0.35, 0.55, 0.75, 0.95] {
                note(&mut evs, t, 0.9, 60, v);
                t += 1.0;
            }
            // 3. C major scale, two octaves
            for &m in &[60, 62, 64, 65, 67, 69, 71, 72, 74, 76, 77, 79, 81, 83, 84] {
                note(&mut evs, t, 0.28, m, 0.55);
                t += 0.25;
            }
            t += 0.8;
            // 4. chords with pedal
            evs.push(Ev { t, kind: 2, midi: 0, vel: 0.0 });
            for chord in [&[36usize, 48, 55, 60, 64, 67][..], &[41, 53, 57, 60, 65, 69], &[43, 55, 59, 62, 67, 71], &[36, 48, 52, 55, 60, 72]] {
                for (i, &m) in chord.iter().enumerate() {
                    note(&mut evs, t + 0.012 * i as f64, 1.5, m, 0.6);
                }
                t += 1.8;
                evs.push(Ev { t: t - 0.15, kind: 3, midi: 0, vel: 0.0 });
                evs.push(Ev { t: t - 0.02, kind: 2, midi: 0, vel: 0.0 });
            }
            t += 2.5;
            evs.push(Ev { t, kind: 3, midi: 0, vel: 0.0 });
            let (l, r) = play(&mut e, evs, t + 1.5);
            write_wav(&out, fs as u32, &l, &r).unwrap();
        }
    }
    println!("wrote {out}");
}
