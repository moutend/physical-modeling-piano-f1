//! Builds `data/sustain.bin`: for every key, the multipliers on its decay
//! rates (see `sustain.rs`) that put the length of its note on a smooth
//! curve over the keyboard. Run again whenever the model changes, before
//! `gen_leveling`:
//!
//!   cargo run --release --bin gen_sustain [out.bin] [fs]
//!
//! The length of a note is measured as the times after which its level has
//! fallen by 20 dB and by 40 dB (`sustain_times`), key held, at one medium
//! stroke. The targets are fixed curves over the key number (`target`):
//! the trend from bass to treble and the typical shape of the decay of the
//! model, without the scatter from key to key.
//!
//! The keys are measured as they are played: with the default room and key
//! noise and with the other tables in use (direct radiation, levelling,
//! timbre correction), all of which shift the balance of the partials and
//! with it the length of the note. Those tables in turn are made after
//! this one: after a larger change of the model, run the whole sequence of
//! `build.sh` twice.

use piano_engine::design::*;
use piano_engine::engine::Engine;
use piano_engine::sustain::{sustain_times, REF_KEY};

const VEL: f64 = 0.6;
const MAX_SECS: f64 = 30.0;
/// Range of the multipliers.
const LIMIT: f64 = 4.0;
/// Both times within this (relative): solved.
const TOL: f64 = 0.03;
/// Notes rendered per key at most.
const BUDGET: usize = 40;

/// ln of the -20 dB and -40 dB times of `midi` with multipliers `s`.
fn measure(e: &mut Engine, midi: usize, s: (f64, f64), secs: f64) -> [f64; 2] {
    e.set_key_sustain(midi, s);
    e.note_on(midi, VEL);
    let n = (secs * e.fs) as usize;
    let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
    while l.len() < n {
        let len = 128.min(n - l.len());
        e.process(len);
        l.extend_from_slice(&e.out_l[..len]);
        r.extend_from_slice(&e.out_r[..len]);
    }
    let (_, t20, t40) = sustain_times(&l, &r, e.fs);
    // not reached: at least the length of the signal
    [t20.unwrap_or(secs).max(0.01).ln(), t40.unwrap_or(secs).max(0.01).ln()]
}

/// The curves the bass register was judged with: ln of the -20 dB and
/// -40 dB times [s] as cubics in (key - C4) / 24. (Fits to the model
/// without correction at that time, shifted to pass through its C4 at
/// -40 dB.)
const CURVES: [[f64; 4]; 2] = [[0.252805, -0.908384, -0.046337, -0.002395], [1.254832, -1.146995, -0.075679, 0.047136]];
/// From C4 up the notes last this many times longer than on those curves
/// (both times; in between the factor grows from 1 at `BASS_HI`): the
/// length the notes had before the decay rates were fitted to the
/// reference recording, which was judged the more beautiful one.
const SING: f64 = 1.5;

/// Target of `midi`: ln of its -20 dB and -40 dB times.
fn target(midi: usize) -> [f64; 2] {
    let x = (midi as f64 - REF_KEY as f64) / 24.0;
    let sing = interp(midi as f64, &[(BASS_HI as f64, 1.0), (REF_KEY as f64, SING)], true).ln();
    std::array::from_fn(|d| CURVES[d][0] + x * (CURVES[d][1] + x * (CURVES[d][2] + x * CURVES[d][3])) + sing)
}

/// Multipliers that bring the key to `target` (ln of the two times), and
/// the remaining error (ln of measured / target).
fn solve(e: &mut Engine, midi: usize, target: [f64; 2]) -> ((f64, f64), [f64; 2]) {
    let secs = (1.6 * target[1].exp() + 0.5).min(MAX_SECS);
    let lim = LIMIT.ln();
    let norm = |v: [f64; 2]| v[0].abs().max(v[1].abs());
    let left = std::cell::Cell::new(BUDGET);
    let mut probe = |x: [f64; 2]| {
        left.set(left.get() - 1);
        let m = measure(e, midi, (x[0].exp(), x[1].exp()), secs);
        [m[0] - target[0], m[1] - target[1]]
    };
    let mut best = ([0.0f64; 2], probe([0.0; 2]));
    // Newton steps with a fixed estimate of d ln(time) / d ln(multiplier):
    // the lever mostly sets the early decay, the internal losses the late one.
    const J: [[f64; 2]; 2] = [[-0.6, -0.3], [-0.3, -0.6]];
    let det = J[0][0] * J[1][1] - J[0][1] * J[1][0];
    let mut cap = 0.5;
    for _ in 0..8 {
        let (x, f) = best;
        if norm(f) < TOL {
            break;
        }
        let dx = [-(J[1][1] * f[0] - J[0][1] * f[1]) / det, -(-J[1][0] * f[0] + J[0][0] * f[1]) / det];
        let nx: [f64; 2] = std::array::from_fn(|i| (x[i] + dx[i].clamp(-cap, cap)).clamp(-lim, lim));
        if (nx[0] - x[0]).abs() + (nx[1] - x[1]).abs() < 1e-4 {
            break; // pinned at the limits
        }
        let nf = probe(nx);
        if norm(nf) < norm(f) {
            best = (nx, nf);
        } else {
            cap *= 0.5;
        }
    }
    // The times jump where a beat crosses a threshold, which a Newton step
    // cannot deal with: finish with a pattern search around the best point.
    let mut step = 0.2;
    while norm(best.1) >= TOL && step > 0.02 && left.get() >= 4 {
        let mut moved = false;
        for d in [[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]] {
            let nx: [f64; 2] = std::array::from_fn(|i| (best.0[i] + step * d[i]).clamp(-lim, lim));
            if nx == best.0 {
                continue;
            }
            let nf = probe(nx);
            if norm(nf) < norm(best.1) {
                best = (nx, nf);
                moved = true;
            }
        }
        if !moved {
            step *= 0.5;
        }
    }
    ((best.0[0].exp(), best.0[1].exp()), best.1)
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "data/sustain.bin".into());
    let fs = std::env::args().nth(2).and_then(|s| s.parse::<f64>().ok()).unwrap_or(48000.0);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(NKEYS);
    let per = NKEYS.div_ceil(threads);
    let t0 = std::time::Instant::now();

    // the uncorrected model
    let mut base = vec![[0.0; 2]; NKEYS];
    std::thread::scope(|sc| {
        for (t, chunk) in base.chunks_mut(per).enumerate() {
            sc.spawn(move || {
                let mut e = Engine::new(fs);
                for (i, cell) in chunk.iter_mut().enumerate() {
                    *cell = measure(&mut e, MIDI_LO + t * per + i, (1.0, 1.0), MAX_SECS);
                }
            });
        }
    });
    let targets: Vec<[f64; 2]> = (MIDI_LO..MIDI_LO + NKEYS).map(target).collect();

    let mut table = vec![((1.0, 1.0), [0.0; 2]); NKEYS];
    std::thread::scope(|sc| {
        for (t, chunk) in table.chunks_mut(per).enumerate() {
            let targets = &targets;
            sc.spawn(move || {
                let mut e = Engine::new(fs);
                for (i, cell) in chunk.iter_mut().enumerate() {
                    let key = t * per + i;
                    if MIDI_LO + key > BASS_HI {
                        *cell = solve(&mut e, MIDI_LO + key, targets[key]);
                    }
                }
            });
        }
    });

    println!("| midi | target -20 dB [s] | target -40 dB [s] | before [%] | before [%] | after [%] | after [%] | prompt x | after x |");
    println!("|---|---|---|---|---|---|---|---|---|");
    let pc = |v: f64| 100.0 * (v.exp() - 1.0);
    for (i, (s, r)) in table.iter().enumerate() {
        let (t, b) = (targets[i], base[i]);
        println!(
            "| {} | {:.2} | {:.2} | {:+.0} | {:+.0} | {:+.0} | {:+.0} | {:.2} | {:.2} |",
            MIDI_LO + i, t[0].exp(), t[1].exp(), pc(b[0] - t[0]), pc(b[1] - t[1]), pc(r[0]), pc(r[1]), s.0, s.1
        );
    }
    let solved = BASS_HI + 1 - MIDI_LO..NKEYS;
    let rms = |f: &dyn Fn(usize) -> f64| (solved.clone().map(|i| f(i).powi(2)).sum::<f64>() / solved.len() as f64).sqrt();
    eprintln!(
        "fs {fs}: keys above {BASS_HI}: deviation from the targets (rms of ln): -20 dB {:.3} -> {:.3}, -40 dB {:.3} -> {:.3}; {} keys beyond {TOL}; {:.0} s",
        rms(&|i| base[i][0] - targets[i][0]),
        rms(&|i| table[i].1[0]),
        rms(&|i| base[i][1] - targets[i][1]),
        rms(&|i| table[i].1[1]),
        table.iter().filter(|c| c.1[0].abs().max(c.1[1].abs()) > TOL).count(),
        t0.elapsed().as_secs_f64()
    );
    // The keys up to `BASS_HI` keep the entries of the table in use: they
    // were judged by ear with those, and their model has not changed since
    // (the solution scatters from run to run where a beat crosses a
    // threshold).
    let mut bytes: Vec<u8> = vec![];
    for (i, (s, _)) in table.iter().enumerate() {
        let kept = piano_engine::sustain::correction(MIDI_LO + i);
        let s = if MIDI_LO + i <= BASS_HI { &kept } else { s };
        bytes.extend_from_slice(&(s.0 as f32).to_le_bytes());
        bytes.extend_from_slice(&(s.1 as f32).to_le_bytes());
    }
    std::fs::write(&out, &bytes).unwrap();
    eprintln!("wrote {out}: {} bytes", bytes.len());
}
