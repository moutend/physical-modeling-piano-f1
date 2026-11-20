//! Builds `data/leveling.bin`: for every key and several key velocities, the
//! hammer velocity multiplier that makes the key as loud as the bass
//! register (the median loudness of C2..C3 as levelled by the table in
//! use; perceived loudness, see `loudness.rs`). The keys up to `BASS_HI`
//! keep their entries. Run again whenever the model changes:
//!
//!   cargo run --release --bin gen_leveling [out.bin] [fs]
//!
//! `gen_morph` uses this table, and this table has to be made with the
//! result of `gen_morph`: run `gen_leveling`, `gen_morph`, `gen_leveling`.
//!
//! With `fs`, only that sample rate is measured (quick check while tuning
//! the model; the shipped table holds all of `RATES`).
//!
//! The model is used as it is, with the default room and key noise, and the
//! levelling switched off.

use piano_engine::design::*;
use piano_engine::engine::{Engine, MAX_HAMMER_V};
use piano_engine::leveling::Leveling;
use piano_engine::loudness::{note_loudness, sone_to_phon};

const RATES: [f64; 4] = [44100.0, 48000.0, 88200.0, 96000.0];
// denser at the soft end, where loudness changes fastest with the force
const LEVELS: [f64; 9] = [0.06, 0.12, 0.18, 0.25, 0.33, 0.43, 0.56, 0.75, 0.97];
/// The reference loudness is that of the bass register as it is levelled by
/// the table in use: the median over these keys (C2..C3), each struck with
/// its hammer velocity from that table. Their entries are kept as they are
/// (`BASS_HI`), and every key above is made as loud as they are, whatever
/// has changed in its model.
const REF_KEYS: std::ops::RangeInclusive<usize> = 36..=BASS_HI;
const SECS: f64 = 1.0;
/// Residual above which a cell is considered unsolved [phon].
const INHERIT_PHON: f64 = 2.0;

fn measure(e: &mut Engine, midi: usize, v0: f64) -> f64 {
    e.all_off();
    e.strike(midi, v0);
    let n = (SECS * e.fs) as usize;
    let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
    while l.len() < n {
        let len = 128.min(n - l.len());
        e.process(len);
        l.extend_from_slice(&e.out_l[..len]);
        r.extend_from_slice(&e.out_r[..len]);
    }
    note_loudness(&l, &r, e.fs)
}

/// (scale, residual error in phon)
fn solve(e: &mut Engine, midi: usize, vel: f64, target: f64) -> (f64, f64) {
    let v_nom = hammer_velocity(vel);
    let (lo, hi) = (0.03 / v_nom, MAX_HAMMER_V / v_nom);
    let mut s: f64 = 1.0;
    let mut prev: Option<(f64, f64)> = None;
    let mut best = (1.0, f64::INFINITY);
    for _ in 0..8 {
        let l = measure(e, midi, v_nom * s);
        let err = sone_to_phon(l) - sone_to_phon(target);
        if err.abs() < best.1.abs() {
            best = (s, err);
        }
        if err.abs() < 0.05 {
            break;
        }
        // loudness ~ s^p locally
        let p = match prev {
            Some((ps, pl)) if (s / ps).ln().abs() > 1e-6 => ((l / pl).ln() / (s / ps).ln()).clamp(0.3, 2.5),
            _ => 0.9,
        };
        prev = Some((s, l));
        let next = (s * (target / l).powf(1.0 / p)).clamp(lo, hi);
        if (next / s).ln().abs() < 1e-4 {
            break; // pinned at a limit
        }
        s = next;
    }
    best
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "data/leveling.bin".into());
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(NKEYS);
    let mut bytes: Vec<u8> = vec![];
    let rates: Vec<f64> = match std::env::args().nth(2).and_then(|s| s.parse::<f64>().ok()) {
        Some(fs) => vec![fs],
        None => RATES.to_vec(),
    };
    bytes.extend_from_slice(&(rates.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(LEVELS.len() as u32).to_le_bytes());
    for l in LEVELS {
        bytes.extend_from_slice(&(l as f32).to_le_bytes());
    }
    for fs in rates {
        let t0 = std::time::Instant::now();
        let mut e = Engine::new(fs);
        e.leveling = false;
        let kept = Leveling::new(fs);
        let targets: Vec<f64> = LEVELS
            .iter()
            .enumerate()
            .map(|(j, &v)| {
                let mut l: Vec<f64> = REF_KEYS.map(|m| measure(&mut e, m, (hammer_velocity(v) * kept.entry(m, j)).min(MAX_HAMMER_V))).collect();
                l.sort_by(|a, b| a.partial_cmp(b).unwrap());
                l[l.len() / 2]
            })
            .collect();
        let mut table = vec![(1.0, 0.0); NKEYS * LEVELS.len()];
        std::thread::scope(|sc| {
            for (t, chunk) in table.chunks_mut(NKEYS.div_ceil(threads) * LEVELS.len()).enumerate() {
                let (targets, kept) = (&targets, &kept);
                sc.spawn(move || {
                    let mut e = Engine::new(fs);
                    e.leveling = false;
                    for (i, cell) in chunk.iter_mut().enumerate() {
                        let key = t * NKEYS.div_ceil(threads) + i / LEVELS.len();
                        let j = i % LEVELS.len();
                        let midi = MIDI_LO + key;
                        *cell = if midi <= BASS_HI { (kept.entry(midi, j), 0.0) } else { solve(&mut e, midi, LEVELS[j], targets[j]) };
                    }
                });
            }
        });
        // Near the threshold of hearing (softest strokes of the treble) the
        // loudness hardly depends on the hammer any more and the solution
        // is erratic: such a cell takes over the scale of the next stronger
        // stroke of its key.
        let mut inherited = 0;
        for key in 0..NKEYS {
            for j in (0..LEVELS.len() - 1).rev() {
                if table[key * LEVELS.len() + j].1.abs() > INHERIT_PHON {
                    table[key * LEVELS.len() + j].0 = table[key * LEVELS.len() + j + 1].0;
                    inherited += 1;
                }
            }
        }
        eprintln!("fs {fs}: {inherited} cells inherit the scale of the next stronger stroke");
        let db = |s: f64| 20.0 * s.log10();
        let (mn, mx) = table.iter().fold((f64::MAX, f64::MIN), |a, c| (a.0.min(db(c.0)), a.1.max(db(c.0))));
        let worst = table.iter().map(|c| c.1.abs()).fold(0.0, f64::max);
        let off = table.iter().filter(|c| c.1.abs() > 0.5).count();
        eprintln!(
            "fs {fs}: reference loudness {:?} phon; hammer velocity scale {mn:+.1} .. {mx:+.1} dB; residual max {worst:.2} phon ({off} cells > 0.5 phon); {:.0} s",
            targets.iter().map(|&t| (sone_to_phon(t) * 10.0).round() / 10.0).collect::<Vec<_>>(),
            t0.elapsed().as_secs_f64()
        );
        // the cells that could not be levelled (hammer velocity limits)
        let mut bad: Vec<(usize, f64)> = table.iter().map(|c| c.1).enumerate().filter(|c| c.1.abs() > 0.5).collect();
        bad.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap());
        for (i, r) in bad.iter().take(12) {
            eprintln!("    key {} at velocity {}: {r:+.1} phon", MIDI_LO + i / LEVELS.len(), LEVELS[i % LEVELS.len()]);
        }
        bytes.extend_from_slice(&(fs as f32).to_le_bytes());
        for c in &table {
            bytes.extend_from_slice(&(c.0 as f32).to_le_bytes());
        }
    }
    std::fs::write(&out, &bytes).unwrap();
    eprintln!("wrote {out}: {} bytes", bytes.len());
}
