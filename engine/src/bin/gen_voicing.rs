//! Builds `data/voicing.bin`: for every key, the gain of the direct
//! radiation of its strings (see `voicing.rs`). Run again whenever the model
//! changes, after `gen_sustain` and before `gen_leveling`:
//!
//!   cargo run --release --bin gen_voicing [out.bin] [fs]
//!
//! Every key is struck with the same medium stroke, without room and key
//! noise and with the levelling off. What is measured is how well the
//! soundboard carries the key's tone to the microphones: the level of the
//! note relative to that of its strings' own signal. Over the keyboard
//! that follows a smooth trend with a scatter of several dB around it. A
//! key below the trend gets the smallest direct gain that lifts it onto
//! it (at most `MAX_GAIN`); the keys above keep the board's sound alone.
//! From C6 upwards the target rises by up to `TREBLE_DB`: a stronger tone
//! there means a slower hammer after the levelling, hence a softer blow,
//! which is heard as a sharp click in that register.
//!
//! A second pass, with room and key noise, looks at the blow itself
//! (`attack_measure`): where it still stands out against the tone by more
//! than `BLOW_DB`, the gain is raised until it does not.
//!
//! A third pass, above the bass register, raises the gain of the notes that
//! reach the two microphones in opposite phase (`MIN_CORR`).
//!
//! The gains found are those that would fill a key in completely. Above
//! the bass register the engine takes only a share of them
//! (`design::direct_share`), which the first two passes do not see: there
//! the levels they report as "after" are those of the full gain.

use piano_engine::design::*;
use piano_engine::engine::Engine;
use piano_engine::voicing::attack_measure;

const VEL: f64 = 0.6;
/// Keys below the trend by more than this are filled in [dB].
const MARGIN_DB: f64 = 0.0;
/// Lift of the target at C8, linear in the key number from C6 [dB].
const TREBLE_DB: f64 = 3.0;
const MAX_GAIN: f64 = 4.0;
/// Largest blow / tone left on a key [dB], and how much lower at C8
/// (linear in the key number from C6).
const BLOW_DB: f64 = -8.0;
const BLOW_TREBLE_DB: f64 = 1.0;

/// Smallest correlation of the two microphone signals over the tone of a
/// note (50..300 ms) above the bass register. Seen from the two
/// microphones the soundboard radiates some notes in opposite phase; such
/// a note has no place between the loudspeakers and all but vanishes when
/// the channels are added. The direct radiation is the same in both.
const MIN_CORR: f64 = 0.15;

/// Correlation of the two channels over 50..300 ms of `midi` as played
/// with direct gain `g` (with the share the key takes of it).
fn corr(e: &mut Engine, midi: usize, g: f64) -> f64 {
    e.all_off();
    e.set_key_direct(midi, g * direct_share(midi));
    e.strike(midi, hammer_velocity(VEL));
    let n = (0.3 * e.fs) as usize;
    let (mut ll, mut rr, mut lr) = (0.0, 0.0, 0.0);
    let mut done = 0;
    while done < n {
        let len = 128.min(n - done);
        e.process(len);
        if done >= (0.05 * e.fs) as usize {
            for i in 0..len {
                let (l, r) = (e.out_l[i] as f64, e.out_r[i] as f64);
                ll += l * l;
                rr += r * r;
                lr += l * r;
            }
        }
        done += len;
    }
    lr / (ll * rr + 1e-60).sqrt()
}

/// Tone level [dB] (rms of 50..300 ms, both channels) of `midi` with direct
/// gain `g`; `board`: with the soundboard's sound.
fn tone(e: &mut Engine, midi: usize, g: f64, board: bool) -> f64 {
    e.all_off();
    e.set_key_direct(midi, g);
    e.board_out = if board { 1.0 } else { 0.0 };
    e.strike(midi, hammer_velocity(VEL));
    let n = (0.3 * e.fs) as usize;
    let mut acc = 0.0;
    let mut done = 0;
    while done < n {
        let len = 128.min(n - done);
        e.process(len);
        if done >= (0.05 * e.fs) as usize {
            acc += (0..len).map(|i| (e.out_l[i] as f64).powi(2) + (e.out_r[i] as f64).powi(2)).sum::<f64>();
        }
        done += len;
    }
    e.board_out = 1.0;
    10.0 * (acc + 1e-30).log10()
}

/// Blow / tone [dB] (`attack_measure`) of `midi` with direct gain `g`.
fn blow(e: &mut Engine, midi: usize, g: f64) -> f64 {
    e.all_off();
    e.set_key_direct(midi, g);
    e.strike(midi, hammer_velocity(VEL));
    let n = (0.5 * e.fs) as usize;
    let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
    while l.len() < n {
        let len = 128.min(n - l.len());
        e.process(len);
        l.extend(e.out_l[..len].iter().map(|&v| v as f64));
        r.extend(e.out_r[..len].iter().map(|&v| v as f64));
    }
    attack_measure(&l, &r, e.fs, midi).blow[3]
}

/// Running median (window of 2 h + 1 keys) followed by a moving average.
fn trend(x: &[f64], h: usize) -> Vec<f64> {
    let n = x.len();
    let med: Vec<f64> = (0..n)
        .map(|i| {
            let mut w: Vec<f64> = x[i.saturating_sub(h)..(i + h + 1).min(n)].to_vec();
            w.sort_by(|a, b| a.partial_cmp(b).unwrap());
            w[w.len() / 2]
        })
        .collect();
    (0..n).map(|i| { let w = &med[i.saturating_sub(h)..(i + h + 1).min(n)]; w.iter().sum::<f64>() / w.len() as f64 }).collect()
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "data/voicing.bin".into());
    let fs = std::env::args().nth(2).and_then(|s| s.parse::<f64>().ok()).unwrap_or(48000.0);
    let mut e = Engine::new(fs);
    e.leveling = false;
    e.morph = 0.0;
    // the notes themselves: no room, no key noise
    let (room, knock) = (e.room_amount, e.knock);
    e.room_amount = 0.0;
    e.knock = 0.0;
    // How well the board carries each key's tone to the microphones: the
    // level it gives, relative to that of the strings' own signal.
    let eff: Vec<f64> = (MIDI_LO..MIDI_LO + NKEYS).map(|m| tone(&mut e, m, 0.0, true) - tone(&mut e, m, 1.0, false)).collect();
    let tr = trend(&eff, 5);
    println!("| midi | board re strings [dB] | target [dB] | gain | after [dB] |");
    println!("|---|---|---|---|---|");
    let mut gains = vec![0.0f64; NKEYS];
    let (mut before, mut after) = (vec![], vec![]);
    for (i, midi) in (MIDI_LO..MIDI_LO + NKEYS).enumerate() {
        let t = tr[i] + TREBLE_DB * ((midi as f64 - 84.0) / 24.0).clamp(0.0, 1.0);
        let strings = tone(&mut e, midi, 1.0, false);
        let mut g = 0.0;
        let mut got = eff[i];
        if eff[i] < t - MARGIN_DB {
            // The two paths add with whatever phase they have, so the level
            // need not rise monotonically at first: scan upwards, then
            // bisect between the last gain that missed and the first hit.
            let (mut lo, mut hi) = (0.0, MAX_GAIN);
            let mut x: f64 = 0.125;
            let mut hit = false;
            while x <= MAX_GAIN * 1.0001 {
                if tone(&mut e, midi, x, true) - strings >= t {
                    hi = x;
                    hit = true;
                    break;
                }
                lo = x;
                x *= 2f64.sqrt();
            }
            if hit {
                for _ in 0..6 {
                    let mid = 0.5 * (lo + hi);
                    if tone(&mut e, midi, mid, true) - strings >= t {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
            }
            g = hi;
            got = tone(&mut e, midi, g, true) - strings;
        }
        e.set_key_direct(midi, g);
        gains[i] = g;
        before.push(eff[i] - t);
        after.push(got - t);
        println!("| {midi} | {:.1} | {t:.1} | {g:.2} | {got:.1} |", eff[i]);
    }
    // Second pass, as played (room, key noise): a key on which the blow
    // still stands out against the tone gets more, until the ratio is at
    // `BLOW_DB` (the blow itself differs from key to key as well).
    e.room_amount = room;
    e.knock = knock;
    println!();
    println!("| midi | blow / tone [dB] | after [dB] | gain |");
    println!("|---|---|---|---|");
    let (mut b_before, mut b_after) = (vec![], vec![]);
    for (i, midi) in (MIDI_LO..MIDI_LO + NKEYS).enumerate() {
        let t = BLOW_DB - BLOW_TREBLE_DB * ((midi as f64 - 84.0) / 24.0).clamp(0.0, 1.0);
        let r0 = blow(&mut e, midi, gains[i]);
        let mut r = r0;
        if r0 > t {
            let (mut lo, mut hi) = (gains[i], MAX_GAIN);
            let mut x = (gains[i] * 2f64.sqrt()).max(0.125);
            let mut hit = false;
            while x <= MAX_GAIN * 1.0001 {
                if blow(&mut e, midi, x) <= t {
                    hi = x;
                    hit = true;
                    break;
                }
                lo = x;
                x *= 2f64.sqrt();
            }
            if hit {
                for _ in 0..6 {
                    let mid = 0.5 * (lo + hi);
                    if blow(&mut e, midi, mid) <= t {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
            }
            gains[i] = hi;
            r = blow(&mut e, midi, hi);
        }
        e.set_key_direct(midi, gains[i]);
        b_before.push(r0);
        b_after.push(r);
        println!("| {midi} | {r0:.1} | {r:.1} | {:.2} |", gains[i]);
    }
    // Third pass, above the bass register: a note that the board radiates
    // in opposite phase to the two microphones gets more, until the
    // channels go together (`MIN_CORR`).
    println!();
    println!("| midi | L/R correlation | after | gain |");
    println!("|---|---|---|---|");
    let mut turned = 0;
    for (i, midi) in (MIDI_LO..MIDI_LO + NKEYS).enumerate().filter(|&(_, m)| m > BASS_HI) {
        let c0 = corr(&mut e, midi, gains[i]);
        let mut c = c0;
        if c0 < MIN_CORR {
            let (mut lo, mut hi) = (gains[i], MAX_GAIN);
            let mut x = (gains[i] * 2f64.sqrt()).max(0.125);
            let mut hit = false;
            while x <= MAX_GAIN * 1.0001 {
                if corr(&mut e, midi, x) >= MIN_CORR {
                    hi = x;
                    hit = true;
                    break;
                }
                lo = x;
                x *= 2f64.sqrt();
            }
            if hit {
                for _ in 0..6 {
                    let mid = 0.5 * (lo + hi);
                    if corr(&mut e, midi, mid) >= MIN_CORR {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
            }
            // (not within reach: the key stays as it is)
            if hit {
                gains[i] = hi;
                c = corr(&mut e, midi, hi);
            }
            turned += 1;
        }
        e.set_key_direct(midi, gains[i]);
        println!("| {midi} | {c0:.2} | {c:.2} | {:.2} |", gains[i]);
    }
    eprintln!("fs {fs}: {turned} keys above {BASS_HI} radiated in opposite phase (L/R correlation below {MIN_CORR})");
    let scatter = |v: &[f64]| ((1..v.len() - 1).map(|i| (v[i] - 0.5 * (v[i - 1] + v[i + 1])).powi(2)).sum::<f64>() / (v.len() - 2) as f64).sqrt();
    eprintln!(
        "fs {fs}: blow / tone: strongest {:.1} -> {:.1} dB, scatter between neighbours {:.1} -> {:.1} dB",
        b_before.iter().cloned().fold(f64::MIN, f64::max),
        b_after.iter().cloned().fold(f64::MIN, f64::max),
        scatter(&b_before),
        scatter(&b_after)
    );
    let rms = |v: &[f64]| (v.iter().map(|x| x * x).sum::<f64>() / v.len() as f64).sqrt();
    eprintln!(
        "fs {fs}: tone against the target: rms {:.1} -> {:.1} dB, weakest {:+.1} -> {:+.1} dB; {} keys with direct radiation, largest gain {:.2}",
        rms(&before),
        rms(&after),
        before.iter().cloned().fold(f64::MAX, f64::min),
        after.iter().cloned().fold(f64::MAX, f64::min),
        gains.iter().filter(|&&g| g > 0.0).count(),
        gains.iter().cloned().fold(0.0, f64::max)
    );
    // The keys up to `BASS_HI` keep the entries of the table in use
    // (judged by ear with those; their model has not changed). The table
    // holds the full gains; above the bass register a key radiates only
    // `direct_share` of its gain (`Engine::build_voice`).
    let mut bytes: Vec<u8> = vec![];
    for (i, g) in gains.iter().enumerate() {
        let kept = piano_engine::voicing::own_gain(MIDI_LO + i);
        let g = if MIDI_LO + i <= BASS_HI { &kept } else { g };
        bytes.extend_from_slice(&(*g as f32).to_le_bytes());
    }
    std::fs::write(&out, &bytes).unwrap();
    eprintln!("wrote {out}: {} bytes", bytes.len());
}
