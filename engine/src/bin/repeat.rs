//! Repeated strikes of one key with the sustain pedal down.
//!
//!   repeat scan [vel] [strikes] [fs]        all 88 keys x {125, 180, 250} ms: table of the
//!                                           components that stand out against a single strike
//!   repeat key <midi> [ms] [vel] [strikes]  peak list of one key
//!   repeat wav <out.wav> <midi> [ms] [vel] [strikes]
//!
//! For every key the spectrum after the last of `strikes` strikes is compared
//! with the spectrum at the same delay after a single strike (pedal down in
//! both cases). A component that the repetition makes much stronger relative
//! to the note itself, and that is not one of the key's own partials, is what
//! is heard as "another pitch ringing along".

use piano_engine::design::*;
use piano_engine::engine::Engine;
use piano_engine::wav::write_wav;
use std::f64::consts::PI;

fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
        }
        len <<= 1;
    }
}

const NWIN: usize = 32768;

/// Hann-windowed magnitude spectrum [dB] of x[start..start + NWIN], zero padded x2.
fn spectrum_db(x: &[f64], start: usize) -> Vec<f64> {
    let nfft = NWIN * 2;
    let mut re = vec![0.0; nfft];
    let mut im = vec![0.0; nfft];
    for i in 0..NWIN {
        let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / NWIN as f64).cos();
        re[i] = x.get(start + i).copied().unwrap_or(0.0) * w;
    }
    fft(&mut re, &mut im);
    (0..nfft / 2).map(|i| 10.0 * ((re[i] * re[i] + im[i] * im[i]) * (4.0 / NWIN as f64).powi(2) + 1e-30).log10()).collect()
}

/// Rough equal-loudness weighting (A-weighting) [dB].
fn a_weight(f: f64) -> f64 {
    let f2 = f * f;
    let ra = 12194.0f64.powi(2) * f2 * f2 / ((f2 + 20.6f64.powi(2)) * ((f2 + 107.7f64.powi(2)) * (f2 + 737.9f64.powi(2))).sqrt() * (f2 + 12194.0f64.powi(2)));
    20.0 * ra.log10() + 2.0
}

fn note_name(f: f64) -> String {
    let m = 69.0 + 12.0 * (f / 440.0).log2();
    let n = m.round() as i32;
    let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}{:+.0}", names[n.rem_euclid(12) as usize], n / 12 - 1, 100.0 * (m - n as f64))
}

/// Mono output of `strikes` strikes `ms` apart, pedal down; the analysis
/// window starts `LEAD` seconds after the last strike.
const LEAD: f64 = 0.20;

fn run(e: &mut Engine, midi: usize, ms: f64, vel: f64, strikes: usize, pedal: bool) -> (Vec<f64>, Vec<f64>, usize) {
    e.all_off();
    let env = |k: &str, d: f64| std::env::var(k).ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(d);
    e.pedal_resonance = env("RES", 1.0) > 0.0;
    e.knock = env("KNOCK", 1.0);
    e.longitudinal = env("LONG", 1.0);
    e.sympathetic = env("SYMP", 1.0);
    // HOLD=37,49: instead of the pedal, only these keys' dampers are up (silently held keys)
    if let Ok(h) = std::env::var("HOLD") {
        for m in h.split(',').filter_map(|x| x.parse::<usize>().ok()) {
            e.strike(m, 1e-9);
        }
    } else {
        e.set_pedal(pedal && env("PEDAL", 1.0) > 0.0);
    }
    let fs = e.fs;
    // PRE=60,62: these keys are played first (pedal down) and left to die away for PREWAIT s
    if let Ok(h) = std::env::var("PRE") {
        for m in h.split(',').filter_map(|x| x.parse::<usize>().ok()) {
            e.note_on(m, vel);
        }
        let mut left = (env("PREWAIT", 12.0) * fs) as usize;
        let mut released = false;
        while left > 0 {
            let len = 32.min(left);
            e.process(len);
            left -= len;
            if !released {
                released = true;
                for m in h.split(',').filter_map(|x| x.parse::<usize>().ok()) {
                    e.note_off(m);
                }
            }
        }
    }
    // A player does not repeat a key with sample accuracy: an exactly periodic
    // train would comb the spectrum at multiples of the repetition rate and
    // single out whatever rings there. JITTER = relative timing deviation.
    let jit = env("JITTER", 0.08);
    let at: Vec<usize> = (0..strikes).map(|i| ((i as f64 + if i > 0 { jit * (2.0 * hash01(977 * midi as u32 + 31 * i as u32 + ms as u32) - 1.0) } else { 0.0 }) * ms * 1e-3 * fs) as usize).collect();
    let last = at[strikes - 1];
    let start = last + (LEAD * fs) as usize;
    let n = start + NWIN + 64 + (env("TAIL", 0.0) * fs) as usize;
    let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut next = 0;
    let mut done = 0;
    while done < n {
        while next < strikes && at[next] <= done {
            if next > 0 {
                e.note_off(midi);
            }
            e.note_on(midi, vel);
            next += 1;
        }
        let len = 32.min(n - done);
        e.process(len);
        l.extend(e.out_l[..len].iter().map(|&v| v as f64));
        r.extend(e.out_r[..len].iter().map(|&v| v as f64));
        done += len;
    }
    (l, r, start)
}

#[derive(Clone, Debug)]
struct Peak {
    f: f64,
    /// Level relative to the strongest (A-weighted) component of the note [dB].
    rel: f64,
    /// Same component of the key played alone (one strike, no pedal), relative to its strongest one [dB].
    rel_single: f64,
    /// Distance to the nearest own partial [cents], and its number.
    cents: f64,
    n: usize,
    /// The key played alone (once, no pedal) has a clear component here.
    in_note: bool,
}

struct Analysis {
    peaks: Vec<Peak>,
    /// Level of the strongest component [dB].
    top: f64,
    top_single: f64,
    /// A-weighted power of the foreign components relative to the key's own [dB].
    foreign_db: f64,
}

fn analyse(e: &mut Engine, midi: usize, ms: f64, vel: f64, strikes: usize) -> Analysis {
    let fs = e.fs;
    let k = key_params(midi);
    let (l, _, s) = run(e, midi, ms, vel, strikes, true);
    let rep = spectrum_db(&l, s);
    // reference: the key alone, struck once, no pedal (the note itself)
    let (l1, _, s1) = run(e, midi, ms, vel, 1, false);
    let sin = spectrum_db(&l1, s1);
    let hz = fs / (2 * rep.len()) as f64;
    let lo = (25.0 / hz) as usize;
    let hi = ((12000.0 / hz) as usize).min(rep.len() - 2);
    let wt = |sp: &[f64], i: usize| sp[i] + a_weight(i as f64 * hz);
    let top = (lo..hi).map(|i| wt(&rep, i)).fold(-300.0, f64::max);
    let top_single = (lo..hi).map(|i| wt(&sin, i)).fold(-300.0, f64::max);
    let f0 = k.f1 / (1.0 + k.b_inh).sqrt();
    let mut peaks = vec![];
    for i in lo..hi {
        let v = wt(&rep, i);
        if v < top - 50.0 || rep[i] < rep[i - 1] || rep[i] < rep[i + 1] {
            continue;
        }
        // local maximum over +-4 Hz
        let w = (4.0 / hz) as usize;
        if (i.saturating_sub(w)..=(i + w).min(hi)).any(|j| rep[j] > rep[i]) {
            continue;
        }
        let f = i as f64 * hz;
        // the note alone: strongest value within +-1.5 Hz, and whether it has a peak there
        let w = (1.5 / hz) as usize;
        let (a, b) = (i.saturating_sub(w).max(1), (i + w).min(hi));
        let rs = (a..=b).map(|j| wt(&sin, j)).fold(-300.0, f64::max);
        let in_note = (a..=b).any(|j| sin[j] > sin[j - 1] && sin[j] >= sin[j + 1]) && rs - top_single > -40.0;
        // nearest own partial
        let mut best = (f64::INFINITY, 0);
        let n0 = (f / f0).round().max(1.0) as usize;
        for n in n0.saturating_sub(3).max(1)..=n0 + 1 {
            let c = 1200.0 * (f / (n as f64 * f0 * (1.0 + k.b_inh * (n * n) as f64).sqrt())).log2();
            if c.abs() < best.0.abs() {
                best = (c, n);
            }
        }
        peaks.push(Peak { f, rel: v - top, rel_single: rs - top_single, cents: best.0, n: best.1, in_note });
    }
    let pw = |own_: bool| peaks.iter().filter(|p| own(p) == own_).map(|p| 10f64.powf(p.rel / 10.0)).sum::<f64>();
    let foreign_db = 10.0 * (pw(false) / pw(true).max(1e-12) + 1e-12).log10();
    Analysis { peaks, top, top_single, foreign_db }
}

/// A component "of the note": close to one of the key's own partials.
fn own(p: &Peak) -> bool {
    // close to the ideal stiff-string series (the realised partials drift
    // from it at high n), or a component of the note played alone
    p.cents.abs() < 12.0 + 0.6 * p.n as f64 || p.in_note
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("scan");
    let num = |i: usize, d: f64| args.get(i).and_then(|s| s.parse::<f64>().ok()).unwrap_or(d);
    match cmd {
        "key" => {
            let midi = num(2, 60.0) as usize;
            let (ms, vel, strikes) = (num(3, 125.0), num(4, 0.7), num(5, 8.0) as usize);
            let mut e = Engine::new(48000.0);
            let a = analyse(&mut e, midi, ms, vel, strikes);
            println!("key {midi}  f1 {:.2} Hz  top {:.1} dB (single {:.1} dB)", key_params(midi).f1, a.top, a.top_single);
            println!("    f[Hz]  note      rel[dB] single[dB] excess  partial cents");
            let mut ps = a.peaks.clone();
            ps.sort_by(|a, b| b.rel.partial_cmp(&a.rel).unwrap());
            for p in ps.iter().take(40) {
                println!(
                    "{:9.1}  {:8} {:7.1} {:9.1} {:7.1}  {:4} {:+7.1} {}",
                    p.f,
                    note_name(p.f),
                    p.rel,
                    p.rel_single,
                    p.rel - p.rel_single,
                    p.n,
                    p.cents,
                    if own(p) { "" } else { "  <-- foreign" }
                );
            }
        }
        "couple" => {
            // |a| = Z n c^2 |Y(f)|: how strongly each key's unison loads the bridge at f
            let fs = 48000.0;
            let e = Engine::new(fs);
            let fq: Vec<f64> = (2..args.len()).filter_map(|i| args[i].parse().ok()).collect();
            for v in e.voices.iter() {
                let mut sc = [0.0; NZONES];
                sc[v.zone] = v.sc[0];
                sc[v.zone + 1] = v.sc[1];
                let c = v.sc[0] + v.sc[1];
                let kap = v.key.z * v.key.n_wg as f64;
                print!("{:3} f1 {:7.1} Z {:5.2} c {:5.2} |", v.key.midi, v.key.f1, v.key.z, c);
                for &f in &fq {
                    let (re, im) = e.board.admittance(&sc, f, fs);
                    print!(" {:7.4} {:+7.4} |", kap * re, kap * im);
                }
                let (re, im) = e.board.admittance(&sc, v.key.f1, fs);
                println!(" own f1: {:7.4} {:+7.4}", kap * re, kap * im);
            }
        }
        "ymap" => {
            // Peaks of the driving-point conductance of each bridge zone [1e-3 s/kg]
            let fs = 48000.0;
            let e = Engine::new(fs);
            for z in 0..NZONES {
                let mut sc = [0.0; NZONES];
                sc[z] = 1.0;
                let g: Vec<(f64, f64)> = (0..2400).map(|i| 30.0 * (4000.0f64 / 30.0).powf(i as f64 / 2400.0)).map(|f| (f, e.board.admittance(&sc, f, fs).0)).collect();
                let mean = |lo: f64, hi: f64| {
                    let v: Vec<f64> = g.iter().filter(|x| x.0 >= lo && x.0 < hi).map(|x| x.1).collect();
                    v.iter().sum::<f64>() / v.len().max(1) as f64
                };
                println!("zone {z}: mean G [1e-3 s/kg] 30-100 {:.2} 100-200 {:.2} 200-400 {:.2} 400-800 {:.2} 800-1600 {:.2} 1600-4000 {:.2}", 1e3 * mean(30.0, 100.0), 1e3 * mean(100.0, 200.0), 1e3 * mean(200.0, 400.0), 1e3 * mean(400.0, 800.0), 1e3 * mean(800.0, 1600.0), 1e3 * mean(1600.0, 4000.0));
                let mut pk: Vec<(f64, f64)> = (1..g.len() - 1).filter(|&i| g[i].1 > g[i - 1].1 && g[i].1 > g[i + 1].1).map(|i| g[i]).collect();
                pk.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                println!("   peaks: {}", pk.iter().take(12).map(|p| format!("{:.0}Hz {:.2}", p.0, 1e3 * p.1)).collect::<Vec<_>>().join(", "));
            }
        }
        "hires" => {
            // repeat hires <midi> <ms> <vel> <strikes> <lo> <hi>: long-window peak list (TAIL=secs)
            let midi = num(2, 60.0) as usize;
            let (ms, vel, strikes, lo, hi) = (num(3, 125.0), num(4, 0.7), num(5, 1.0) as usize, num(6, 100.0), num(7, 200.0));
            let fs = 48000.0;
            let mut e = Engine::new(fs);
            let (l, _, start) = run(&mut e, midi, ms, vel, strikes, true);
            let start = start + (0.3 * fs) as usize;
            let n = (l.len() - start).min(1 << 18);
            let nfft = (4 * n).next_power_of_two();
            let (mut re, mut im) = (vec![0.0; nfft], vec![0.0; nfft]);
            for i in 0..n {
                re[i] = l[start + i] * (0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos());
            }
            fft(&mut re, &mut im);
            let hz = fs / nfft as f64;
            let db: Vec<f64> = (0..nfft / 2).map(|i| 10.0 * ((re[i] * re[i] + im[i] * im[i]) * (4.0 / n as f64).powi(2) + 1e-30).log10()).collect();
            let (a, b) = ((lo / hz) as usize, (hi / hz) as usize);
            let mut pk: Vec<usize> = (a.max(1)..b).filter(|&i| db[i] > db[i - 1] && db[i] >= db[i + 1]).collect();
            pk.sort_by(|&x, &y| db[y].partial_cmp(&db[x]).unwrap());
            pk.truncate(16);
            pk.sort();
            println!("key {midi}: window {:.2} s from {:.2} s after the last strike", n as f64 / fs, LEAD + 0.3);
            for i in pk {
                println!("{:9.2} Hz {:7.1} dB  {}", i as f64 * hz, db[i], note_name(i as f64 * hz));
            }
        }
        "wav" => {
            let out = args.get(2).cloned().unwrap_or_else(|| "repeat.wav".into());
            let midi = num(3, 60.0) as usize;
            let (ms, vel, strikes) = (num(4, 125.0), num(5, 0.7), num(6, 8.0) as usize);
            let mut e = Engine::new(48000.0);
            let (l, r, _) = run(&mut e, midi, ms, vel, strikes, true);
            let (l, r): (Vec<f32>, Vec<f32>) = (l.iter().map(|&v| v as f32).collect(), r.iter().map(|&v| v as f32).collect());
            write_wav(&out, 48000, &l, &r).unwrap();
            println!("wrote {out}");
        }
        _ => {
            let (vel, strikes, fs) = (num(2, 0.7), num(3, 8.0) as usize, num(4, 48000.0));
            let intervals = [125.0, 180.0, 250.0];
            let nthreads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(12);
            let mut handles = vec![];
            for t in 0..nthreads {
                handles.push(std::thread::spawn(move || {
                    let mut e = Engine::new(fs);
                    let mut rows = vec![];
                    for midi in (MIDI_LO..MIDI_LO + NKEYS).filter(|m| m % nthreads == t) {
                        for &ms in &intervals {
                            let a = analyse(&mut e, midi, ms, vel, strikes);
                            let loud = a.peaks.iter().filter(|p| !own(p)).max_by(|a, b| a.rel.partial_cmp(&b.rel).unwrap()).cloned();
                            rows.push((midi, ms, a.foreign_db, loud));
                        }
                    }
                    rows
                }));
            }
            let mut rows: Vec<_> = handles.into_iter().flat_map(|h| h.join().unwrap()).collect();
            rows.sort_by(|a, b| (a.0, a.1 as i64).cmp(&(b.0, b.1 as i64)));
            println!("vel {vel}  strikes {strikes}  fs {fs}");
            println!("foreign = A-weighted power of components that are not the key's own, relative to its own, {LEAD} s after the last strike");
            println!("key | per interval: foreign[dB] (loudest foreign: Hz, note, dB rel. strongest own)");
            let mut worst = vec![];
            for c in rows.chunks(intervals.len()) {
                print!("{:3} |", c[0].0);
                for (_, _, fdb, loud) in c {
                    match loud {
                        Some(p) => print!(" {:6.1} ({:7.1} {:8} {:6.1}) |", fdb, p.f, note_name(p.f), p.rel),
                        None => print!(" {:6.1} ({:>23}) |", fdb, "-"),
                    }
                }
                println!();
                worst.push((c.iter().map(|r| r.2).fold(-200.0, f64::max), c[0].0));
            }
            let band = |lo: usize, hi: usize| {
                let v: Vec<f64> = worst.iter().filter(|w| w.1 >= lo && w.1 <= hi).map(|w| w.0).collect();
                (v.iter().sum::<f64>() / v.len() as f64, v.iter().cloned().fold(-200.0, f64::max))
            };
            for (lo, hi) in [(21, 32), (33, 46), (47, 60), (61, 74), (75, 89), (90, 108)] {
                let (m, x) = band(lo, hi);
                println!("keys {lo:3}-{hi:3}: worst-interval foreign  mean {m:6.1} dB  max {x:6.1} dB");
            }
            worst.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
            println!("worst keys: {}", worst.iter().take(16).map(|w| format!("{}:{:.1}", w.1, w.0)).collect::<Vec<_>>().join(" "));
        }
    }
}
