//! Builds `data/morph.bin`: for every key that is not a reference key, the
//! gains that put each of its partials at the blend of the levels the same
//! partial has on the reference keys around it (see `morph.rs`). Run again
//! whenever the model or another table changes, after `gen_leveling`:
//!
//!   cargo run --release --bin gen_morph [out.bin] [fs]
//!   cargo run --release --bin gen_morph check [fs]     the table in use: partial levels as played
//!
//! (`gen_leveling` has to be run before, for the reference keys, and once
//! more afterwards, see there.)
//!
//! Every reference key is played with the same medium stroke as it is heard
//! (with the levelling and the room at its default level, as the reference
//! keys were judged; without the key noise, which is no part of the tone),
//! a key in between with the blend of their hammer velocities. The level of a partial is
//! its energy at a microphone over the first part of the note. For a key
//! in between, the bridge wave of the key is related to what arrives at
//! each microphone, partial by partial (least squares over the bins around
//! the partial): that gives the gain that would double the partial there;
//! scaled, it moves the partial to its target. A few rounds of measuring
//! and rescaling take care of what the resonators of neighbouring partials
//! add to each other.
//!
//! From C6 up the partials above the fundamental are not blended but put
//! at fixed levels below it (`morph::voice_treble`), on the reference keys
//! as well: those get gains for these partials only.
//!
//! With the tone fixed like that, the hammer velocity of the key only
//! decides how much of the blow is heard next to it (`attack_measure`, as
//! played: with room and key noise). It is trimmed until blow / tone is
//! the blend of the reference keys' as well, and the gains are solved again.
//! The trim is not stored: the levelling made afterwards finds the hammer
//! velocity at which the key, with these gains, is as loud as the others,
//! which is about the same one, the blend of equally loud tones being
//! about as loud.

use piano_engine::design::*;
use piano_engine::engine::Engine;
use piano_engine::leveling::Leveling;
use piano_engine::loudness::fft;
use piano_engine::morph::*;
use piano_engine::voicing::attack_measure;
use std::f64::consts::PI;

const VEL: f64 = 0.6;
/// Range of the level change of a partial [dB]: up, down.
const MAX_UP_DB: f64 = 26.0;
const MAX_DOWN_DB: f64 = 20.0;
/// Partials that are (and are meant to be) weaker than the strongest one by
/// more than this are left alone [dB].
const RANGE_DB: f64 = 50.0;
/// The partials that make the tone, for the summary of `check`: those
/// within this of the strongest one [dB].
const MAIN_DB: f64 = 30.0;
/// A partial is only lowered if at least this share of its band at the
/// microphone is explained by the key's bridge wave. (Below that, the
/// soundboard carries next to nothing of the partial to that microphone;
/// it can still be raised, and then is the resonator's signal itself.)
const MIN_FIT: f64 = 0.5;
const ROUNDS: usize = 4;
/// Range of the hammer trim [dB], and how close blow / tone has to come [dB].
const TRIM_DB: (f64, f64) = (-9.0, 3.0);
const BLOW_TOL: f64 = 0.5;
const TRIM_ROUNDS: usize = 4;

/// Length of the part of the note that is compared [s].
fn window_secs(midi: usize) -> f64 {
    interp(midi as f64, &[(21.0, 2.0), (60.0, 1.0), (84.0, 0.5), (108.0, 0.25)], true)
}

struct Take {
    l: Vec<f64>,
    r: Vec<f64>,
    /// The key's bridge wave.
    x: Vec<f64>,
}

/// Hammer velocity [m/s] of the medium stroke on `midi`: as levelled on the
/// reference keys, the blend of those in between (beyond the outermost
/// reference key: like that one).
fn stroke(lv: &Leveling, midi: usize) -> f64 {
    let db = |m: usize| 20.0 * lv.scale(m, VEL).log10();
    hammer_velocity(VEL) * 10f64.powf(blend(midi).map(|bl| mix(midi, bl, db(bl.0), db(bl.1), 0.0)).unwrap_or(db(midi)) / 20.0)
}

/// The note, struck with `trim` times its medium stroke; `noise`: with the
/// key noise.
fn render(e: &mut Engine, lv: &Leveling, midi: usize, trim: f64, gains: &[[f64; 4]], secs: f64, noise: bool) -> Take {
    let knock = e.knock;
    if !noise {
        e.knock = 0.0;
    }
    e.all_off();
    e.set_key_morph(midi, gains);
    e.strike_key(midi, trim * stroke(lv, midi), hammer_velocity(VEL));
    // (the key noise is set off by the strike)
    e.knock = knock;
    let n = (secs * e.fs) as usize;
    let mut t = Take { l: Vec::with_capacity(n), r: Vec::with_capacity(n), x: Vec::with_capacity(n) };
    while t.l.len() < n {
        // sample by sample: the bridge wave is only kept for the current one
        e.process(1);
        t.l.push(e.out_l[0] as f64);
        t.r.push(e.out_r[0] as f64);
        t.x.push(e.voices[midi - MIDI_LO].bridge);
    }
    t
}

/// Spectrum of the first `secs` of `x` (flat, then fading out over the
/// second half): (re, im).
fn spectrum(x: &[f64], fs: f64, secs: f64) -> (Vec<f64>, Vec<f64>) {
    let len = ((secs * fs) as usize).min(x.len());
    let n = len.next_power_of_two();
    let (mut re, mut im) = (vec![0.0; n], vec![0.0; n]);
    for i in 0..len {
        let u = i as f64 / len as f64;
        re[i] = x[i] * if u < 0.5 { 1.0 } else { (PI * (u - 0.5)).cos().powi(2) };
    }
    fft(&mut re, &mut im);
    (re, im)
}

/// Bins that belong to the partial at `f`.
fn band(f: f64, f1: f64, fs: f64, n: usize) -> std::ops::RangeInclusive<usize> {
    let df = fs / n as f64;
    let h = (0.3 * f1).min((0.03 * f).max(3.0 * df));
    (((f - h) / df).floor().max(1.0) as usize)..=(((f + h) / df).ceil() as usize).min(n / 2 - 1)
}

/// Level [dB] of every partial of the key at the two microphones.
fn levels(t: &Take, k: &KeyParams, fs: f64, secs: f64) -> Vec<[f64; 2]> {
    let (sl, sr) = (spectrum(&t.l, fs, secs), spectrum(&t.r, fs, secs));
    let n = sl.0.len();
    partials(k)
        .iter()
        .map(|&f| {
            let e = |s: &(Vec<f64>, Vec<f64>)| 10.0 * (band(f, k.f1, fs, n).map(|q| s.0[q] * s.0[q] + s.1[q] * s.1[q]).sum::<f64>() + 1e-30).log10();
            [e(&sl), e(&sr)]
        })
        .collect()
}

/// Per partial and microphone: the gains on (Re s, Im s) of the partial's
/// resonator that reproduce what the microphone holds around that partial,
/// and the share of the energy there that they explain.
fn unit_gains(t: &Take, k: &KeyParams, fs: f64, secs: f64) -> Vec<[([f64; 2], f64); 2]> {
    let sx = spectrum(&t.x, fs, secs);
    let sy = [spectrum(&t.l, fs, secs), spectrum(&t.r, fs, secs)];
    let n = sx.0.len();
    let bw = bandwidth(k);
    partials(k)
        .iter()
        .map(|&f| {
            let res = Res::new(fs, f, bw, [0.0; 4]);
            std::array::from_fn(|c| {
                // least squares: gr P + gi Q = Y over the band, P and Q the
                // spectra of Re s and Im s
                let (mut pp, mut pq, mut qq, mut py, mut qy, mut yy) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
                for q in band(f, k.f1, fs, n) {
                    let (a, b) = res.response(2.0 * PI * q as f64 / n as f64);
                    let x = (sx.0[q], sx.1[q]);
                    let p = (a.0 * x.0 - a.1 * x.1, a.0 * x.1 + a.1 * x.0);
                    let qv = (b.0 * x.0 - b.1 * x.1, b.0 * x.1 + b.1 * x.0);
                    let y = (sy[c].0[q], sy[c].1[q]);
                    pp += p.0 * p.0 + p.1 * p.1;
                    pq += p.0 * qv.0 + p.1 * qv.1;
                    qq += qv.0 * qv.0 + qv.1 * qv.1;
                    py += p.0 * y.0 + p.1 * y.1;
                    qy += qv.0 * y.0 + qv.1 * y.1;
                    yy += y.0 * y.0 + y.1 * y.1;
                }
                let det = pp * qq - pq * pq;
                if det <= 1e-12 * pp * qq || yy <= 0.0 {
                    return ([0.0; 2], 0.0);
                }
                let g = [(py * qq - qy * pq) / det, (qy * pp - py * pq) / det];
                // explained energy: g . (py, qy)
                let fit = ((g[0] * py + g[1] * qy) / yy).clamp(0.0, 1.0);
                if fit >= MIN_FIT {
                    return (g, fit);
                }
                // not related: the same direction, as strong as what is there
                let own = g[0] * g[0] * pp + 2.0 * g[0] * g[1] * pq + g[1] * g[1] * qq;
                let sc = if own > 0.0 { (yy / own).sqrt() } else { 0.0 };
                ([sc * g[0], sc * g[1]], fit)
            })
        })
        .collect()
}

/// Blow / tone [dB] of the key as played.
fn blow(e: &mut Engine, lv: &Leveling, midi: usize, trim: f64, gains: &[[f64; 4]]) -> f64 {
    let t = render(e, lv, midi, trim, gains, 0.5, true);
    attack_measure(&t.l, &t.r, e.fs, midi).blow[3]
}

struct Row {
    midi: usize,
    trim: f64,
    gains: Vec<[f64; 4]>,
    /// rms deviation from the target over the partials [dB]: before, after.
    dev: [f64; 2],
    /// Largest deviation of a partial after.
    worst: f64,
    /// Number of partials: corrected, all.
    count: [usize; 2],
    /// Largest level changes applied [dB]: down, up.
    reach: [f64; 2],
    /// Blow / tone [dB]: target, before, after.
    blow: [f64; 3],
}

/// The gains that bring the partials of `midi`, struck with hammer trim
/// `trim`, to `target`.
fn solve(e: &mut Engine, lv: &Leveling, midi: usize, trim: f64, target: &[Option<[f64; 2]>], debug: Option<(&[[f64; 2]], &[[f64; 2]])>) -> Row {
    let fs = e.fs;
    let master = e.gain;
    let k = key_params(midi);
    let secs = window_secs(midi);
    let np = target.len();
    let base = render(e, lv, midi, trim, &[], secs + 0.05, false);
    let l0 = levels(&base, &k, fs, secs);
    let unit = unit_gains(&base, &k, fs, secs);
    let lim = (10f64.powf(-MAX_DOWN_DB / 20.0), 10f64.powf(MAX_UP_DB / 20.0));
    let top = target.iter().flatten().flatten().cloned().fold(f64::MIN, f64::max);
    let heard = |j: usize, c: usize| target[j].map(|t| t[c].max(l0[j][c]) > top - RANGE_DB).unwrap_or(false);
    let related = |j: usize, c: usize| unit[j][c].1 >= MIN_FIT;
    let dev = |l: &[[f64; 2]]| {
        let (mut acc, mut cnt, mut worst) = (0.0, 0.0, 0.0f64);
        for j in 0..np {
            for c in 0..2 {
                if let (Some(t), true) = (target[j], heard(j, c)) {
                    acc += (l[j][c] - t[c]).powi(2);
                    cnt += 1.0;
                    worst = worst.max((l[j][c] - t[c]).abs());
                }
            }
        }
        ((acc / f64::max(cnt, 1.0)).sqrt(), worst)
    };
    let to_gains = |fac: &[[f64; 2]]| -> Vec<[f64; 4]> {
        (0..np)
            .map(|j| {
                // (the correction is added ahead of the master gain)
                let g = |c: usize, i: usize| (fac[j][c] - 1.0) * unit[j][c].0[i] / master;
                [g(0, 0), g(0, 1), g(1, 0), g(1, 1)]
            })
            .collect()
    };
    // amplitude factor of every partial and channel (1: untouched)
    let mut fac: Vec<[f64; 2]> = vec![[1.0; 2]; np];
    let mut cur = l0.clone();
    let mut best = (dev(&l0), fac.clone());
    for _ in 0..ROUNDS {
        for j in 0..np {
            for c in 0..2 {
                if heard(j, c) {
                    let f = (fac[j][c] * 10f64.powf((target[j].unwrap()[c] - cur[j][c]) / 20.0)).clamp(lim.0, lim.1);
                    fac[j][c] = if related(j, c) { f } else { f.max(1.0) };
                }
            }
        }
        cur = levels(&render(e, lv, midi, trim, &to_gains(&fac), secs + 0.05, false), &k, fs, secs);
        let d = dev(&cur);
        if d.0 < best.0 .0 {
            best = (d, fac.clone());
        }
    }
    let fac = best.1;
    let gains = to_gains(&fac);
    if let Some((la, lb)) = debug {
        let fin = levels(&render(e, lv, midi, trim, &gains, secs + 0.05, false), &k, fs, secs);
        let mut s = format!("key {midi}, trim {:+.1} dB: partial | lower ref L R | upper ref L R | target L R | before L R | after L R | fit L R\n", 20.0 * trim.log10());
        for j in 0..np {
            let p = |v: Option<[f64; 2]>| v.map(|v| format!("{:6.1} {:6.1}", v[0], v[1])).unwrap_or("     -      -".into());
            s += &format!(
                "  {:2} | {} | {} | {} | {} | {} | {:.2} {:.2}\n",
                j + 1, p(la.get(j).copied()), p(lb.get(j).copied()), p(target[j]), p(Some(l0[j])), p(Some(fin[j])), unit[j][0].1, unit[j][1].1
            );
        }
        eprint!("{s}");
    }
    let db = |f: &f64| 20.0 * f.log10();
    Row {
        midi,
        trim,
        dev: [dev(&l0).0, best.0 .0],
        worst: best.0 .1,
        count: [fac.iter().flatten().filter(|&&f| f != 1.0).count().div_ceil(2), np],
        reach: [fac.iter().flatten().map(db).fold(0.0, f64::min), fac.iter().flatten().map(db).fold(0.0, f64::max)],
        blow: [0.0; 3],
        gains,
    }
}

/// The keys as they are played now (with the table in use and their own
/// levelling): how far the partials are from the blend of the reference keys.
fn check(fs: f64) {
    let mut e = Engine::new(fs);
    e.knock = 0.0;
    let mut take = |midi: usize, secs: f64| {
        e.all_off();
        e.note_on(midi, VEL);
        let n = (secs * fs) as usize;
        let mut t = Take { l: Vec::with_capacity(n), r: Vec::with_capacity(n), x: vec![] };
        while t.l.len() < n {
            let len = 64.min(n - t.l.len());
            e.process(len);
            t.l.extend(e.out_l[..len].iter().map(|&v| v as f64));
            t.r.extend(e.out_r[..len].iter().map(|&v| v as f64));
        }
        t
    };
    println!("| midi | blend of | partials within {MAIN_DB} dB of the strongest | off the target [dB rms] | worst [dB] | within {RANGE_DB} dB | off the target [dB rms] | worst [dB] |");
    println!("|---|---|---|---|---|---|---|---|");
    let mut all = [vec![], vec![]];
    for midi in (MIDI_LO..MIDI_LO + NKEYS).filter(|&m| !is_anchor(m)) {
        let secs = window_secs(midi);
        let bl = blend(midi).unwrap();
        let l = levels(&take(midi, secs + 0.05), &key_params(midi), fs, secs);
        // (the reference keys of the treble are voiced themselves: what
        // is blended is their tone below the voiced partials, which are
        // set by `voice_treble` on this key as on them)
        let la = levels(&take(bl.0, secs + 0.05), &key_params(bl.0), fs, secs);
        let lb = levels(&take(bl.1, secs + 0.05), &key_params(bl.1), fs, secs);
        let target = targets(midi, &la, &lb, l.len());
        let top = target.iter().flatten().flatten().cloned().fold(f64::MIN, f64::max);
        if std::env::var("MORPH_DEBUG").map(|v| v.split(',').any(|m| m.parse() == Ok(midi))).unwrap_or(false) {
            for j in 0..l.len() {
                eprintln!("  {midi} partial {:2}: target {:?}, level {:6.1} {:6.1}", j + 1, target[j].map(|t| [(t[0] * 10.0).round() / 10.0, (t[1] * 10.0).round() / 10.0]), l[j][0], l[j][1]);
            }
        }
        let mut cells = vec![];
        for (q, range) in [MAIN_DB, RANGE_DB].into_iter().enumerate() {
            let d: Vec<f64> = (0..l.len())
                .flat_map(|j| (0..2).map(move |c| (j, c)))
                .filter_map(|(j, c)| target[j].filter(|t| t[c].max(l[j][c]) > top - range).map(|t| l[j][c] - t[c]))
                .collect();
            let rms = (d.iter().map(|x| x * x).sum::<f64>() / d.len().max(1) as f64).sqrt();
            cells.push(format!("{} | {rms:.1} | {:.1}", d.len().div_ceil(2), d.iter().fold(0.0f64, |a, x| a.max(x.abs()))));
            all[q].push(rms);
        }
        println!("| {midi} | {} .. {} | {} |", bl.0, bl.1, cells.join(" | "));
    }
    println!();
    let rms = |v: &[f64]| (v.iter().map(|x| x * x).sum::<f64>() / v.len() as f64).sqrt();
    println!("rms over the keys: {:.1} dB (partials within {MAIN_DB} dB of the strongest), {:.1} dB (within {RANGE_DB} dB)", rms(&all[0]), rms(&all[1]));
}

/// Target level of every partial of `midi` at the two microphones, from
/// the levels `la`, `lb` of the partials of its two reference keys; None
/// where a reference key has no such partial. For a reference key in the
/// treble: from its own levels `la`. The treble voicing is applied.
fn targets(midi: usize, la: &[[f64; 2]], lb: &[[f64; 2]], np: usize) -> Vec<Option<[f64; 2]>> {
    let Some(bl) = blend(midi) else {
        // a reference key: its own levels (`la`), voiced
        let mut t: Vec<Option<[f64; 2]>> = (0..np).map(|j| la.get(j).copied()).collect();
        voice_treble(midi, &mut t);
        return t;
    };
    let both = la.len().min(lb.len());
    let mut t: Vec<Option<[f64; 2]>> = (0..np)
        .map(|j| {
            if j >= both {
                return None;
            }
            Some(std::array::from_fn(|c| {
                if (0.0..=1.0).contains(&bl.2) {
                    mix(midi, bl, la[j][c], lb[j][c], MAX_TREND)
                } else {
                    // the trend, evened out over the neighbouring
                    // partials where there are many
                    let h = (both / 8).min(2);
                    let w = j.saturating_sub(h)..(j + h + 1).min(both);
                    let d = w.clone().map(|i| lb[i][c] - la[i][c]).sum::<f64>() / w.len() as f64;
                    if bl.2 < 0.0 {
                        mix(midi, bl, la[j][c], la[j][c] + d, MAX_TREND)
                    } else {
                        mix(midi, bl, lb[j][c] - d, lb[j][c], MAX_TREND)
                    }
                }
            }))
        })
        .collect();
    voice_treble(midi, &mut t);
    t
}

/// Sample rates the table is made for (those of the levelling table).
const RATES: [f64; 4] = [44100.0, 48000.0, 88200.0, 96000.0];

fn main() {
    if std::env::args().nth(1).as_deref() == Some("check") {
        return check(std::env::args().nth(2).and_then(|s| s.parse::<f64>().ok()).unwrap_or(48000.0));
    }
    let out = std::env::args().nth(1).unwrap_or_else(|| "data/morph.bin".into());
    let rates: Vec<f64> = match std::env::args().nth(2).and_then(|s| s.parse::<f64>().ok()) {
        Some(fs) => vec![fs],
        None => RATES.to_vec(),
    };
    let mut bytes: Vec<u8> = (rates.len() as u32).to_le_bytes().to_vec();
    for fs in rates {
        let rows = solve_all(fs);
        bytes.extend_from_slice(&(fs as f32).to_le_bytes());
        for midi in MIDI_LO..MIDI_LO + NKEYS {
            // The keys up to `BASS_HI` keep the entries of the table in
            // use (judged by ear with those; their model and their
            // reference keys have not changed).
            let kept = piano_engine::morph::gains(midi, fs);
            let g: &[[f64; 4]] = if midi <= BASS_HI { &kept } else { rows.iter().find(|r| r.midi == midi).map(|r| r.gains.as_slice()).unwrap_or(&[]) };
            bytes.extend_from_slice(&(g.len() as u32).to_le_bytes());
            for v in g.iter().flatten() {
                bytes.extend_from_slice(&(*v as f32).to_le_bytes());
            }
        }
    }
    std::fs::write(&out, &bytes).unwrap();
    eprintln!("wrote {out}: {} bytes", bytes.len());
}

/// The table of one sample rate.
fn solve_all(fs: f64) -> Vec<Row> {
    let t0 = std::time::Instant::now();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let lv = &Leveling::new(fs);

    // the reference keys: the note itself, as long as the longest window
    // that uses it, and blow / tone
    let anchors: Vec<usize> = (MIDI_LO..MIDI_LO + NKEYS).filter(|&m| is_anchor(m)).collect();
    let mut takes: Vec<Option<(Take, f64)>> = (0..NKEYS).map(|_| None).collect();
    std::thread::scope(|sc| {
        let per = anchors.len().div_ceil(threads);
        let hs: Vec<_> = anchors
            .chunks(per)
            .map(|chunk| {
                sc.spawn(move || {
                    let mut e = Engine::new(fs);
                    chunk.iter().map(|&m| (m, render(&mut e, lv, m, 1.0, &[], window_secs(MIDI_LO) + 0.05, false), blow(&mut e, lv, m, 1.0, &[]))).collect::<Vec<_>>()
                })
            })
            .collect();
        for h in hs {
            for (m, t, b) in h.join().unwrap() {
                takes[m - MIDI_LO] = Some((t, b));
            }
        }
    });
    let takes = &takes;
    let debug: Vec<usize> = std::env::var("MORPH_DEBUG").map(|v| v.split(',').filter_map(|m| m.parse().ok()).collect()).unwrap_or_default();
    let debug = &debug;

    let gaps: Vec<usize> = (MIDI_LO..MIDI_LO + NKEYS).filter(|&m| is_corrected(m)).collect();
    let mut rows: Vec<Row> = vec![];
    std::thread::scope(|sc| {
        let per = gaps.len().div_ceil(threads);
        let hs: Vec<_> = gaps
            .chunks(per)
            .map(|chunk| {
                sc.spawn(move || {
                    let mut e = Engine::new(fs);
                    chunk
                        .iter()
                        .map(|&midi| {
                            let k = key_params(midi);
                            let secs = window_secs(midi);
                            let Some(bl) = blend(midi) else {
                                // a reference key of the treble: its own
                                // tone and blow, the upper partials voiced
                                let own = takes[midi - MIDI_LO].as_ref().unwrap();
                                let l = levels(&own.0, &k, fs, secs);
                                let target = targets(midi, &l, &l, partials(&k).len());
                                let dbg = debug.contains(&midi).then_some((l.as_slice(), l.as_slice()));
                                let mut row = solve(&mut e, lv, midi, 1.0, &target, dbg);
                                let b = blow(&mut e, lv, midi, 1.0, &row.gains);
                                row.blow = [own.1, own.1, b];
                                return row;
                            };
                            let (ta, tb) = (takes[bl.0 - MIDI_LO].as_ref().unwrap(), takes[bl.1 - MIDI_LO].as_ref().unwrap());
                            let la = levels(&ta.0, &key_params(bl.0), fs, secs);
                            let lb = levels(&tb.0, &key_params(bl.1), fs, secs);
                            let target = targets(midi, &la, &lb, partials(&k).len());
                            // blow / tone: beyond the outermost reference key, like that one
                            let blow_target = mix(midi, bl, ta.1, tb.1, 0.0);
                            let dbg = debug.contains(&midi).then_some((la.as_slice(), lb.as_slice()));
                            let mut trim_db: f64 = 0.0;
                            let mut row = solve(&mut e, lv, midi, 1.0, &target, None);
                            let first = blow(&mut e, lv, midi, 1.0, &row.gains);
                            let mut got = first;
                            for _ in 0..TRIM_ROUNDS {
                                if (got - blow_target).abs() < BLOW_TOL {
                                    break;
                                }
                                // (the tone stays, the blow follows the hammer)
                                let next = (trim_db - (got - blow_target)).clamp(TRIM_DB.0, TRIM_DB.1);
                                if (next - trim_db).abs() < 0.05 {
                                    break;
                                }
                                let trim = 10f64.powf(next / 20.0);
                                let r = solve(&mut e, lv, midi, trim, &target, None);
                                let b = blow(&mut e, lv, midi, trim, &r.gains);
                                if (b - blow_target).abs() >= (got - blow_target).abs() {
                                    break;
                                }
                                (trim_db, row, got) = (next, r, b);
                            }
                            if dbg.is_some() {
                                solve(&mut e, lv, midi, row.trim, &target, dbg);
                            }
                            row.blow = [blow_target, first, got];
                            row
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for h in hs {
            rows.extend(h.join().unwrap());
        }
    });

    println!("fs = {fs}");
    println!("| midi | blend of | partials corrected / all | off the target before [dB rms] | after [dB rms] | worst partial after [dB] | level changes [dB] | blow / tone: target [dB] | untrimmed | after | hammer trim [dB] |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    for r in &rows {
        let bl = blend(r.midi).unwrap_or((r.midi, r.midi, 0.0));
        println!(
            "| {} | {} .. {} ({:+.2}) | {} / {} | {:.1} | {:.1} | {:.1} | {:+.1} .. {:+.1} | {:.1} | {:.1} | {:.1} | {:+.1} |",
            r.midi, bl.0, bl.1, bl.2, r.count[0], r.count[1], r.dev[0], r.dev[1], r.worst, r.reach[0], r.reach[1], r.blow[0], r.blow[1], r.blow[2], 20.0 * r.trim.log10()
        );
    }
    let rms = |f: &dyn Fn(&Row) -> f64| (rows.iter().map(|r| f(r).powi(2)).sum::<f64>() / rows.len() as f64).sqrt();
    eprintln!(
        "fs {fs}: {} keys; partial levels off the blend of the reference keys: {:.1} -> {:.1} dB rms; blow / tone off it: {:.1} -> {:.1} dB rms; {:.0} s",
        rows.len(), rms(&|r| r.dev[0]), rms(&|r| r.dev[1]), rms(&|r| r.blow[1] - r.blow[0]), rms(&|r| r.blow[2] - r.blow[0]), t0.elapsed().as_secs_f64()
    );

    rows
}
