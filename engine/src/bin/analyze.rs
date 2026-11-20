//! Objective checks of the model (no listening required).
//!
//!   analyze design [fs]            per-key filter design table
//!   analyze keys [fs] [vel]        per-key measurements (inharmonicity, decay, level, ...)
//!   analyze attack [fs] [vel] [knock] [room] [direct]   blow against tone, every key
//!   analyze tap [fs] [hold] [room]     how long every key is heard after a short key press
//!   analyze sustain [fs] [vel] [secs]  how long every key is heard (-20 / -40 dB)
//!   analyze morph [fs] [vel]       the keys between the reference keys against the blend of those
//!   analyze note <midi> [vel] [fs] partial table of one note
//!   analyze velocity <midi> [fs]   brightness / contact time vs velocity
//!   analyze latency [fs]           algorithmic onset delay per key, both tap modes
//!   analyze stability [fs]         all 88 keys + pedal, long run
//!   analyze sympathetic [fs]       sympathetic resonance check

use piano_engine::design::*;
use piano_engine::engine::Engine;
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

/// Hann-windowed magnitude spectrum of x[start..start+n], zero padded x4.
fn spectrum(x: &[f64], start: usize, n: usize) -> Vec<f64> {
    let nfft = (n * 4).next_power_of_two();
    let mut re = vec![0.0; nfft];
    let mut im = vec![0.0; nfft];
    for i in 0..n {
        let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos();
        re[i] = x.get(start + i).copied().unwrap_or(0.0) * w;
    }
    fft(&mut re, &mut im);
    (0..nfft / 2).map(|i| (re[i] * re[i] + im[i] * im[i]).sqrt() * 4.0 / n as f64).collect()
}

fn render(e: &mut Engine, secs: f64) -> (Vec<f64>, Vec<f64>) {
    let n = (secs * e.fs) as usize;
    let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut done = 0;
    while done < n {
        let len = 128.min(n - done);
        e.process(len);
        l.extend(e.out_l[..len].iter().map(|&v| v as f64));
        r.extend(e.out_r[..len].iter().map(|&v| v as f64));
        done += len;
    }
    (l, r)
}

/// Peak near `f` (searching +-`span` Hz): (frequency, magnitude).
fn find_peak(spec: &[f64], fs: f64, f: f64, span: f64) -> (f64, f64) {
    let hz = fs / (2.0 * spec.len() as f64);
    let lo = (((f - span) / hz).floor().max(1.0)) as usize;
    let hi = (((f + span) / hz).ceil() as usize).min(spec.len() - 2);
    let mut best = lo;
    for i in lo..=hi {
        if spec[i] > spec[best] {
            best = i;
        }
    }
    let (a, b, c) = (spec[best - 1].max(1e-300).ln(), spec[best].max(1e-300).ln(), spec[best + 1].max(1e-300).ln());
    let d = 0.5 * (a - c) / (a - 2.0 * b + c);
    let d = if d.is_finite() { d.clamp(-0.5, 0.5) } else { 0.0 };
    ((best as f64 + d) * hz, spec[best])
}

struct Partials {
    freqs: Vec<f64>,
    mags: Vec<f64>,
    b_fit: f64,
}

fn measure_partials(x: &[f64], fs: f64, f1: f64, b_guess: f64) -> Partials {
    let n = if f1 < 100.0 { 1 << 16 } else { 1 << 15 };
    let spec = spectrum(x, (0.12 * fs) as usize, n);
    let (mut freqs, mut mags) = (vec![], vec![]);
    let f0 = f1 / (1.0 + b_guess).sqrt();
    let mut ratio = 1.0; // running correction measured / predicted
    for k in 1..=60usize {
        let pred = k as f64 * f0 * (1.0 + b_guess * (k * k) as f64).sqrt() * ratio;
        if pred > 0.45 * fs || pred > 16000.0 {
            break;
        }
        let (f, m) = find_peak(&spec, fs, pred, (0.25 * f1).min(0.02 * pred + 3.0));
        ratio *= f / pred;
        freqs.push(f);
        mags.push(m);
    }
    // Linear regression of (f_n / n)^2 on n^2 over partials within 50 dB of the max.
    let mx = mags.iter().cloned().fold(0.0, f64::max);
    let (mut sx, mut sy, mut sxx, mut sxy, mut cnt) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, (&f, &m)) in freqs.iter().zip(&mags).enumerate() {
        if m < mx * 0.003 || i >= 24 {
            continue;
        }
        let n2 = ((i + 1) * (i + 1)) as f64;
        let y = (f / (i + 1) as f64).powi(2);
        sx += n2;
        sy += y;
        sxx += n2 * n2;
        sxy += n2 * y;
        cnt += 1.0;
    }
    let slope = (cnt * sxy - sx * sy) / (cnt * sxx - sx * sx);
    let icpt = (sy - slope * sx) / cnt;
    Partials { freqs, mags, b_fit: slope / icpt }
}

/// Amplitude envelope [dB] of the component at `f`, one value per `hop` samples.
fn partial_env(x: &[f64], fs: f64, f: f64, win: usize, hop: usize) -> Vec<f64> {
    let mut out = vec![];
    let mut s = 0;
    while s + win <= x.len() {
        let (mut re, mut im) = (0.0, 0.0);
        for i in 0..win {
            let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / win as f64).cos();
            let ph = 2.0 * PI * f * i as f64 / fs;
            re += x[s + i] * w * ph.cos();
            im -= x[s + i] * w * ph.sin();
        }
        out.push(20.0 * ((re * re + im * im).sqrt() * 4.0 / win as f64).max(1e-12).log10());
        s += hop;
    }
    out
}

/// Least-squares slope [dB/s] of the envelope between t0 and t1.
fn slope(env: &[f64], hop_s: f64, win_s: f64, t0: f64, t1: f64) -> f64 {
    let (mut sx, mut sy, mut sxx, mut sxy, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, &v) in env.iter().enumerate() {
        let t = i as f64 * hop_s + 0.5 * win_s;
        if t < t0 || t > t1 {
            continue;
        }
        sx += t;
        sy += v;
        sxx += t * t;
        sxy += t * v;
        n += 1.0;
    }
    (n * sxy - sx * sy) / (n * sxx - sx * sx)
}

fn centroid(x: &[f64], fs: f64, start: usize, n: usize) -> f64 {
    let spec = spectrum(x, start, n);
    let hz = fs / (2.0 * spec.len() as f64);
    let (mut a, mut b) = (0.0, 0.0);
    for (i, &m) in spec.iter().enumerate() {
        a += i as f64 * hz * m * m;
        b += m * m;
    }
    a / b
}

fn rms_db(x: &[f64]) -> f64 {
    10.0 * (x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64).max(1e-30).log10()
}

fn onset_ms(x: &[f64], fs: f64) -> f64 {
    let pk = x.iter().fold(0.0f64, |a, &v| a.max(v.abs()));
    let i = x.iter().position(|&v| v.abs() > 0.01 * pk).unwrap_or(0);
    1000.0 * i as f64 / fs
}

fn note_name(m: usize) -> String {
    const N: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", N[m % 12], m as i32 / 12 - 1)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("keys");
    let num = |i: usize, d: f64| args.get(i).and_then(|s| s.parse::<f64>().ok()).unwrap_or(d);
    match cmd {
        "design" => {
            let fs = num(2, 48000.0);
            println!("| key | midi | f1 [Hz] | B | waveguides | Z [kg/s] | 1st-order AP | 2nd-order AP | weighted fit err [cent] | nl | nr |");
            println!("|---|---|---|---|---|---|---|---|---|---|---|");
            for midi in (MIDI_LO..MIDI_LO + NKEYS).step_by(3) {
                let k = key_params(midi);
                let (_, p) = design_loss(fs, k.f1, k.rate_after / 8.686, k.b3);
                let (disp, err, _) = design_dispersion(fs, k.f1, k.b_inh, p, MAX_AP, MAX_BQ);
                let d = tune_loop(fs, k.f1, p, &disp, k.strike_ratio).unwrap();
                println!(
                    "| {} | {} | {:.2} | {:.2e} | {} | {:.2} | {} | {} | {:.2} | {} | {} |",
                    note_name(midi), midi, k.f1, k.b_inh, k.n_wg, k.z, disp.m_ap, disp.poles.len(), err, d.nl, d.nr
                );
            }
        }
        "dispersion" => {
            let fs = num(2, 48000.0);
            println!("partial frequency error [cents] of the dispersion design (weighted fit, fw = {} Hz)", FIT_FW);
            println!("| key | design | n=2 | n=4 | n=8 | n=12 | n=16 | n=24 | n=32 | n=48 | n=60 |");
            println!("|---|---|---|---|---|---|---|---|---|---|---|");
            for midi in [21usize, 24, 28, 33, 36, 40, 43, 48, 54, 60, 66, 72, 78, 84, 96] {
                let k = key_params(midi);
                let (_, p) = design_loss(fs, k.f1, k.rate_after / 8.686, k.b3);
                for (label, bq) in [("1st-order only", 0), ("with 2nd-order", MAX_BQ)] {
                    let (disp, _, np) = design_dispersion(fs, k.f1, k.b_inh, p, MAX_AP, bq);
                    let errs = partial_errors(fs, k.f1, k.b_inh, p, &disp, np).unwrap_or_default();
                    let cell = |n: usize| errs.get(n - 2).map(|e| format!("{:+.1}", e)).unwrap_or("-".into());
                    println!(
                        "| {} | {} ({}+{}) | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                        note_name(midi), label, disp.m_ap, disp.poles.len(),
                        cell(2), cell(4), cell(8), cell(12), cell(16), cell(24), cell(32), cell(48), cell(60)
                    );
                }
            }
        }
        "board" => {
            let fs = num(2, 48000.0);
            let e = Engine::new(fs);
            println!("soundboard: {} modes, lowest {:.1} Hz", e.board.n, e.board.freqs[0]);
            println!();
            let far = args.get(3).map(|s| s == "far").unwrap_or(false);
            println!("radiated response (bridge force at zone -> {}), 1/3-octave bands: mean level [dB] / std of level on a fine grid [dB]", if far { "room feeds" } else { "microphones" });
            println!();
            println!("| band [Hz] | zone 0 (bass bridge) | zone 1 | zone 2 (long bridge, tenor) | zone 3 | zone 4 | zone 5 (treble) |");
            println!("|---|---|---|---|---|---|---|");
            let mut fc = 31.5;
            while fc < 0.4 * fs {
                let mut row = format!("| {:.0} |", fc);
                for z in 0..NZONES {
                    let (mut p, mut dbs) = (0.0, vec![]);
                    for i in 0..60 {
                        let f = fc * 2f64.powf((i as f64 / 59.0 - 0.5) / 3.0);
                        let (l, r) = e.board.response(z, 2.0 * PI * f / fs, far);
                        let l = ((l * l + r * r) / 2.0).sqrt();
                        p += l * l / 60.0;
                        dbs.push(20.0 * l.log10());
                    }
                    let mean = dbs.iter().sum::<f64>() / 60.0;
                    let sd = (dbs.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / 60.0).sqrt();
                    row += &format!(" {:.1} / {:.1} |", 10.0 * p.log10(), sd);
                }
                println!("{row}");
                fc *= 2f64.powf(1.0 / 3.0) * if fc > 900.0 { 2f64.powf(1.0 / 3.0) } else { 1.0 };
            }
        }
        "env" => {
            // level of the first partial and broadband RMS over time
            let midi = num(2, 60.0) as usize;
            let vel = num(3, 0.6);
            let secs = num(4, 8.0);
            let fs = 48000.0;
            let k = key_params(midi);
            let mut e = Engine::new(fs);
            e.note_on(midi, vel);
            let (l, _) = render(&mut e, secs);
            let win = if midi < 48 { 16384 } else { 4096 };
            let env = partial_env(&l, fs, k.f1, win, 4800);
            let env2 = partial_env(&l, fs, partial_freq(k.f1, k.b_inh, 2), win, 4800);
            println!("{}: t [s], partial 1 [dB], partial 2 [dB], rms [dB]", note_name(midi));
            for (i, (a, b)) in env.iter().zip(&env2).enumerate() {
                if i % ((secs / 4.0) as usize).max(1) == 0 {
                    let s0 = i * 4800;
                    println!("{:.1}  {:6.1}  {:6.1}  {:6.1}", i as f64 * 0.1, a, b, rms_db(&l[s0..(s0 + 4800).min(l.len())]));
                }
            }
        }
        "keys" => {
            let fs = num(2, 48000.0);
            let vel = num(3, 0.6);
            println!("velocity = {vel}, fs = {fs}");
            println!("| key | f1 target | f1 meas | B target | B meas | decay 0.2-1.5s [dB/s] | decay late [dB/s] | contact [ms] | Fmax [N] | peak [dBFS] | rms 0.5s [dB] | centroid [Hz] | release T60 [s] |");
            println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|");
            let mut e = Engine::new(fs);
            // the string/board model itself: no room, no key noise
            e.room_amount = 0.0;
            e.knock = 0.0;
            for midi in (MIDI_LO..MIDI_LO + NKEYS).step_by(3) {
                e.all_off();
                let k = key_params(midi);
                e.note_on(midi, vel);
                let secs = if midi < 60 { 10.0 } else if midi < 84 { 6.0 } else { 3.0 };
                let (l, r) = render(&mut e, secs);
                let x: Vec<f64> = l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)).collect();
                let v = &e.voices[midi - MIDI_LO];
                let (contact, fmax) = (1000.0 * v.hammer.contact_samples as f64 / fs, v.hammer.peak_force);
                let p = measure_partials(&x, fs, k.f1, k.b_inh);
                let win = if midi < 48 { 16384 } else { 4096 };
                let hop = 1024;
                let env = partial_env(&x, fs, p.freqs[0], win, hop);
                let (hs, ws) = (hop as f64 / fs, win as f64 / fs);
                let early = slope(&env, hs, ws, 0.2, (0.25 * secs).min(1.5).max(0.6));
                let late = slope(&env, hs, ws, 0.6 * secs, 0.95 * secs);
                let peak = x.iter().fold(0.0f64, |a, &b| a.max(b.abs()));
                // release
                e.note_off(midi);
                let (l2, _) = render(&mut e, 1.5);
                let env2 = partial_env(&l2, fs, p.freqs[0], win.min(4096), 512);
                let rel = slope(&env2, 512.0 / fs, win.min(4096) as f64 / fs, 0.05, 0.35);
                println!(
                    "| {} | {:.2} | {:.2} | {:.2e} | {:.2e} | {:.1} | {:.1} | {:.2} | {:.1} | {:.1} | {:.1} | {:.0} | {} |",
                    note_name(midi), k.f1, p.freqs[0], k.b_inh, p.b_fit, early, late, contact, fmax,
                    20.0 * peak.log10(), rms_db(&x[..(0.5 * fs) as usize]),
                    centroid(&x, fs, (0.02 * fs) as usize, 8192),
                    if rel < -1.0 { format!("{:.2}", -60.0 / rel) } else { "-".into() }
                );
            }
        }
        "note" => {
            let midi = num(2, 60.0) as usize;
            let vel = num(3, 0.6);
            let fs = num(4, 48000.0);
            let k = key_params(midi);
            let mut e = Engine::new(fs);
            e.room_amount = 0.0;
            e.knock = 0.0;
            e.note_on(midi, vel);
            let (l, _) = render(&mut e, 4.0);
            let p = measure_partials(&l, fs, k.f1, k.b_inh);
            println!("{} vel {vel}: B target {:.3e}, measured {:.3e}", note_name(midi), k.b_inh, p.b_fit);
            println!("| n | f meas [Hz] | f ideal [Hz] | error [cent] | level [dB] |");
            println!("|---|---|---|---|---|");
            let f0 = k.f1 / (1.0 + k.b_inh).sqrt();
            let mx = p.mags.iter().cloned().fold(0.0, f64::max);
            for (i, (&f, &m)) in p.freqs.iter().zip(&p.mags).enumerate().take(30) {
                let n = (i + 1) as f64;
                let ideal = n * f0 * (1.0 + k.b_inh * n * n).sqrt();
                println!("| {} | {:.2} | {:.2} | {:+.1} | {:.1} |", i + 1, f, ideal, 1200.0 * (f / ideal).log2(), 20.0 * (m / mx).log10());
            }
        }
        "velocity" => {
            let midi = num(2, 60.0) as usize;
            let fs = num(3, 48000.0);
            println!("{}:", note_name(midi));
            println!("| velocity | hammer v [m/s] | contact [ms] | Fmax [N] | peak [dBFS] | centroid [Hz] |");
            println!("|---|---|---|---|---|---|");
            let mut e = Engine::new(fs);
            for &vel in &[0.1, 0.25, 0.4, 0.55, 0.7, 0.85, 1.0] {
                e.all_off();
                e.note_on(midi, vel);
                let (l, _) = render(&mut e, 1.0);
                let v = &e.voices[midi - MIDI_LO];
                let peak = l.iter().fold(0.0f64, |a, &b| a.max(b.abs()));
                println!(
                    "| {:.2} | {:.2} | {:.2} | {:.1} | {:.1} | {:.0} |",
                    vel, hammer_velocity(vel), 1000.0 * v.hammer.contact_samples as f64 / fs, v.hammer.peak_force,
                    20.0 * peak.log10(), centroid(&l, fs, (0.02 * fs) as usize, 8192)
                );
            }
        }
        "latency" => {
            let fs = num(2, 48000.0);
            println!("| key | onset [ms] |");
            println!("|---|---|");
            let mut e = Engine::new(fs);
            e.knock = 0.0;
            for midi in (MIDI_LO..MIDI_LO + NKEYS).step_by(6).chain([108]) {
                e.all_off();
                e.note_on(midi, 0.6);
                let (l, r) = render(&mut e, 0.3);
                let x: Vec<f64> = l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)).collect();
                println!("| {} | {:.2} |", note_name(midi), onset_ms(&x, fs));
            }
        }
        "admittance" => {
            // bridge admittance seen by a key and the resulting lever ratio
            let fs = num(2, 48000.0);
            let e = Engine::new(fs);
            println!("| key | lever c | mean G [1e-4 s/kg] | G at f1 | G at 2 f1 | G at 3 f1 | G at 4 f1 | nominal prompt rate [dB/s] | rate from G at f1 [dB/s] |");
            println!("|---|---|---|---|---|---|---|---|---|");
            for midi in (MIDI_LO..MIDI_LO + NKEYS).step_by(3) {
                let v = &e.voices[midi - MIDI_LO];
                let k = &v.key;
                let mut sc = [0.0; NZONES];
                sc[v.zone] = v.sc[0];
                sc[v.zone + 1] = v.sc[1];
                let g = |n: usize| e.board.admittance(&sc, partial_freq(k.f1, k.b_inh, n), fs).0;
                let c = v.sc[0] + v.sc[1];
                let rate = 8.686 * 2.0 * k.z * k.n_struck as f64 * g(1) * k.f1 + k.rate_after;
                let mean = e.board.mean_conductance(&sc, 0.8 * k.f1, (6.0 * k.f1).min(9000.0).max(1.2 * k.f1), fs);
                println!("| {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.1} | {:.1} |", note_name(midi), c, 1e4 * mean, 1e4 * g(1), 1e4 * g(2), 1e4 * g(3), 1e4 * g(4), k.rate_prompt, rate);
            }
        }
        "sustain" => {
            // How long each key is heard: time from the peak of the output
            // level (50 ms rms, both channels) until it has fallen for good
            // by 20 / 40 dB (`sustain_times`), key held. The model as the player hears it
            // (room, key noise, levelling).
            let fs = num(2, 48000.0);
            let vel = num(3, 0.6);
            let secs = num(4, 16.0);
            let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(NKEYS);
            let mut rows = vec![(0.0, 0.0, 0.0); NKEYS];
            let per = NKEYS.div_ceil(threads);
            std::thread::scope(|sc| {
                for (t, chunk) in rows.chunks_mut(per).enumerate() {
                    sc.spawn(move || {
                        let mut e = Engine::new(fs);
                        for (i, row) in chunk.iter_mut().enumerate() {
                            e.all_off();
                            e.note_on(MIDI_LO + t * per + i, vel);
                            let (l, r) = render(&mut e, secs);
                            let (l, r): (Vec<f32>, Vec<f32>) = (l.iter().map(|&v| v as f32).collect(), r.iter().map(|&v| v as f32).collect());
                            let (peak, t20, t40) = piano_engine::sustain::sustain_times(&l, &r, fs);
                            *row = (peak, t20.unwrap_or(f64::NAN), t40.unwrap_or(f64::NAN));
                        }
                    });
                }
            });
            println!("velocity = {vel}, fs = {fs}; nan: not reached within {secs} s");
            println!("| key | midi | peak [dBFS] | -20 dB after [s] | -40 dB after [s] |");
            println!("|---|---|---|---|---|");
            for (i, r) in rows.iter().enumerate() {
                println!("| {} | {} | {:.1} | {:.2} | {:.2} |", note_name(MIDI_LO + i), MIDI_LO + i, r.0, r.1, r.2);
            }
        }
        "morph" => {
            // The keys between the reference keys against the blend of
            // the reference keys around them (`morph.rs`), as played
            // (levelling, room, key noise): level of the tone, blow / tone,
            // the times after which the note has fallen by 20 / 40 dB, and
            // the loudness.
            use piano_engine::morph::{blend, mix};
            let fs = num(2, 48000.0);
            let vel = num(3, 0.6);
            let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(NKEYS);
            let per = NKEYS.div_ceil(threads);
            // tone [dB], blow / tone [dB], ln t20, ln t40, loudness [phon]
            let mut rows = vec![[0.0; 5]; NKEYS];
            std::thread::scope(|sc| {
                for (t, chunk) in rows.chunks_mut(per).enumerate() {
                    sc.spawn(move || {
                        let mut e = Engine::new(fs);
                        for (i, row) in chunk.iter_mut().enumerate() {
                            let midi = MIDI_LO + t * per + i;
                            e.all_off();
                            e.note_on(midi, vel);
                            let (l, r) = render(&mut e, 16.0);
                            let m = piano_engine::voicing::attack_measure(&l, &r, fs, midi);
                            let (l, r): (Vec<f32>, Vec<f32>) = (l.iter().map(|&v| v as f32).collect(), r.iter().map(|&v| v as f32).collect());
                            let (_, t20, t40) = piano_engine::sustain::sustain_times(&l, &r, fs);
                            let n = fs as usize;
                            let phon = piano_engine::loudness::sone_to_phon(piano_engine::loudness::note_loudness(&l[..n], &r[..n], fs));
                            *row = [m.tone, m.blow[3], t20.unwrap_or(f64::NAN).ln(), t40.unwrap_or(f64::NAN).ln(), phon];
                        }
                    });
                }
            });
            println!("velocity = {vel}, fs = {fs}; differences to the blend of the reference keys (times: ratio)");
            println!("| key | midi | blend of | tone [dB] | blow / tone [dB] | -20 dB after [s] | ratio | -40 dB after [s] | ratio | loudness [phon] |");
            println!("|---|---|---|---|---|---|---|---|---|---|");
            let mut acc = [0.0; 5];
            let mut cnt = [0.0; 5];
            for midi in MIDI_LO..MIDI_LO + NKEYS {
                let Some(bl) = blend(midi) else { continue };
                let r = rows[midi - MIDI_LO];
                // (beyond the outermost reference key: against that key)
                let d: [f64; 5] = std::array::from_fn(|q| r[q] - mix(midi, bl, rows[bl.0 - MIDI_LO][q], rows[bl.1 - MIDI_LO][q], 0.0));
                for q in 0..5 {
                    if d[q].is_finite() {
                        acc[q] += d[q] * d[q];
                        cnt[q] += 1.0;
                    }
                }
                println!(
                    "| {} | {} | {} .. {} | {:+.1} | {:+.1} | {:.2} | x{:.2} | {:.2} | x{:.2} | {:+.1} |",
                    note_name(midi), midi, bl.0, bl.1, d[0], d[1], r[2].exp(), d[2].exp(), r[3].exp(), d[3].exp(), d[4]
                );
            }
            let rms: [f64; 5] = std::array::from_fn(|q| (acc[q] / f64::max(cnt[q], 1.0)).sqrt());
            println!();
            println!(
                "rms over the keys: tone {:.1} dB, blow / tone {:.1} dB, -20 dB time {:.0} %, -40 dB time {:.0} %, loudness {:.1} phon",
                rms[0], rms[1], 100.0 * (rms[2].exp() - 1.0), 100.0 * (rms[3].exp() - 1.0), rms[4]
            );
        }
        "attack" => {
            // The blow against the tone, every key, as played (levelling,
            // room, key noise). The blow is what the note holds between its
            // partials: the energy of the first moments outside narrow bands
            // around the partial frequencies, split at 500 Hz and 2 kHz and
            // given relative to the energy of 25 ms of the tone (rms of
            // 50..300 ms).
            let fs = num(2, 48000.0);
            let vel = num(3, 0.6);
            let mut e = Engine::new(fs);
            // optional: knock and room levels (default: as played)
            e.knock = num(4, e.knock);
            e.room_amount = num(5, e.room_amount);
            e.direct = num(6, e.direct);
            e.set_hardness(num(7, 1.0));
            println!("velocity = {vel}, fs = {fs}");
            println!("| key | midi | hammer [m/s] | levelling [dB] | peak [dBFS] | tone rms [dB] | blow / tone [dB] | < 500 Hz | 0.5-2 kHz | > 2 kHz |");
            println!("|---|---|---|---|---|---|---|---|---|---|");
            for midi in MIDI_LO..MIDI_LO + NKEYS {
                e.all_off();
                e.note_on(midi, vel);
                let (l, r) = render(&mut e, 0.5);
                let m = piano_engine::voicing::attack_measure(&l, &r, fs, midi);
                println!(
                    "| {} | {} | {:.2} | {:+.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} |",
                    note_name(midi), midi, e.last_hammer_velocity, 20.0 * (e.last_hammer_velocity / hammer_velocity(vel)).log10(), m.peak, m.tone, m.blow[3], m.blow[0], m.blow[1], m.blow[2]
                );
            }
        }
        "tap" => {
            // A short key press, every key, as played: how long the note
            // is heard after it. Level [dB re the peak] 0.15 / 0.3 / 0.6 s
            // after the strike (50 ms rms), and the time to -40 dB.
            let fs = num(2, 48000.0);
            let hold = num(3, 0.04);
            let mut e = Engine::new(fs);
            e.room_amount = num(4, e.room_amount);
            e.room_modal = num(5, e.room_modal);
            e.room_tone = num(6, e.room_tone);
            println!("key pressed for {hold} s, fs = {fs}");
            println!("| key | midi | 0.15 s [dB] | 0.3 s [dB] | 0.6 s [dB] | -40 dB after [s] |");
            println!("|---|---|---|---|---|---|");
            for midi in MIDI_LO..MIDI_LO + NKEYS {
                e.all_off();
                e.note_on(midi, 0.6);
                let (mut l, mut r) = render(&mut e, hold);
                e.note_off(midi);
                let (l2, r2) = render(&mut e, 4.0);
                l.extend(l2);
                r.extend(r2);
                let (lf, rf): (Vec<f32>, Vec<f32>) = (l.iter().map(|&v| v as f32).collect(), r.iter().map(|&v| v as f32).collect());
                let (peak, _, t40) = piano_engine::sustain::sustain_times(&lf, &rf, fs);
                let lev = |t: f64| {
                    let a = (t * fs) as usize;
                    let b = a + (0.05 * fs) as usize;
                    10.0 * ((a..b).map(|i| l[i] * l[i] + r[i] * r[i]).sum::<f64>() / (2 * (b - a)) as f64 + 1e-30).log10() - peak
                };
                println!("| {} | {} | {:.1} | {:.1} | {:.1} | {:.2} |", note_name(midi), midi, lev(0.15), lev(0.3), lev(0.6), t40.unwrap_or(f64::NAN));
            }
        }
        "decay" => {
            // decay rate of each partial: early (0.1-1.0 s) and late
            let midi = num(2, 60.0) as usize;
            let vel = num(3, 0.6);
            let fs = 48000.0;
            let k = key_params(midi);
            let mut e = Engine::new(fs);
            e.room_amount = 0.0;
            e.knock = 0.0;
            e.direct = num(4, e.direct);
            e.note_on(midi, vel);
            let secs = if midi < 60 { 8.0 } else { 5.0 };
            let (l, r) = render(&mut e, secs);
            let x: Vec<f64> = l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)).collect();
            let p = measure_partials(&x, fs, k.f1, k.b_inh);
            let win = if midi < 48 { 8192 } else { 4096 };
            println!("{} vel {vel}: decay of each partial", note_name(midi));
            println!("| n | f [Hz] | level at 0.1 s [dB] | early decay [dB/s] | late decay [dB/s] | level swing (beats) [dB] |");
            println!("|---|---|---|---|---|---|");
            for (i, &f) in p.freqs.iter().enumerate().take(16) {
                let env = partial_env(&x, fs, f, win, 1024);
                let (hs, ws) = (1024.0 / fs, win as f64 / fs);
                let early = slope(&env, hs, ws, 0.1, 1.0);
                let late = slope(&env, hs, ws, 0.5 * secs, 0.95 * secs);
                // deviation from a straight line over the first 3 s
                let fit = slope(&env, hs, ws, 0.1, 3.0);
                let i0 = ((0.1 - 0.5 * ws).max(0.0) / hs) as usize;
                let i1 = (((3.0 - 0.5 * ws) / hs) as usize).min(env.len() - 1);
                let mean = env[i0..=i1].iter().sum::<f64>() / (i1 - i0 + 1) as f64;
                let tm = 0.5 * (i0 + i1) as f64 * hs;
                let swing = (i0..=i1).map(|j| (env[j] - (mean + fit * (j as f64 * hs - tm))).abs()).fold(0.0, f64::max);
                println!("| {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} |", i + 1, f, env[i0], early, late, swing);
            }
        }
        "parts" => {
            // level of each ingredient of a note, from renders with it on / off
            let fs = num(2, 48000.0);
            let band = |x: &[f64], lo: f64, hi: f64| -> f64 {
                let n = 1 << 15;
                let sp = spectrum(x, 0, n.min(x.len()));
                let hz = fs / (2.0 * sp.len() as f64);
                let e: f64 = sp.iter().enumerate().filter(|(i, _)| *i as f64 * hz >= lo && (*i as f64 * hz) < hi).map(|(_, m)| m * m).sum();
                10.0 * e.max(1e-30).log10()
            };
            println!("| key | vel | note peak [dBFS] | key noise peak re note [dB] | key noise 40-400 Hz re note [dB] | longitudinal re note, 0.2-1 kHz / 1-3 kHz / 3-8 kHz [dB] | room re direct (energy, first 1.5 s) [dB] |");
            println!("|---|---|---|---|---|---|---|");
            for &(midi, vel) in &[(24usize, 0.9), (33, 0.5), (33, 0.9), (48, 0.9), (60, 0.3), (60, 0.6), (60, 0.9), (84, 0.6), (96, 0.6), (105, 0.6)] {
                let run = |knock: f64, long: f64, room: f64| -> Vec<f64> {
                    let mut e = Engine::new(fs);
                    e.knock = knock;
                    e.longitudinal = long;
                    e.room_amount = room;
                    e.note_on(midi, vel);
                    let (l, r) = render(&mut e, 1.5);
                    l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)).collect()
                };
                let base = run(0.0, 0.0, 0.0);
                let diff = |a: &[f64]| -> Vec<f64> { a.iter().zip(&base).map(|(x, y)| x - y).collect() };
                let kn = diff(&run(1.0, 0.0, 0.0));
                let lo = diff(&run(0.0, 1.0, 0.0));
                let ro = diff(&run(0.0, 0.0, Engine::new(fs).room_amount));
                let pk = |x: &[f64]| 20.0 * x.iter().fold(0.0f64, |a, &b| a.max(b.abs())).max(1e-12).log10();
                println!(
                    "| {} | {} | {:.1} | {:.1} | {:.1} | {:.1} / {:.1} / {:.1} | {:.1} |",
                    note_name(midi), vel, pk(&base), pk(&kn) - pk(&base), band(&kn, 40.0, 400.0) - band(&base, 40.0, 400.0),
                    band(&lo, 200.0, 1000.0) - band(&base, 200.0, 1000.0), band(&lo, 1000.0, 3000.0) - band(&base, 1000.0, 3000.0),
                    band(&lo, 3000.0, 8000.0) - band(&base, 3000.0, 8000.0), rms_db(&ro) - rms_db(&base)
                );
            }
        }
        "ltas" => {
            // long-term average spectrum of a short piece (chords + melody, pedal)
            let fs = 48000.0;
            let vel = num(2, 0.55);
            let mut e = Engine::new(fs);
            let mut x: Vec<f64> = vec![];
            let chords: [&[usize]; 4] = [&[36, 48, 55, 60, 64, 67], &[41, 53, 57, 60, 65, 69], &[43, 55, 59, 62, 67, 71], &[36, 48, 52, 55, 60, 72]];
            let melody = [76usize, 79, 84, 83, 81, 77, 74, 72, 79, 86, 88, 84, 91, 96, 88, 84];
            for (ci, ch) in chords.iter().enumerate() {
                e.set_pedal(true);
                for &m in ch.iter() {
                    e.note_on(m, vel);
                }
                for j in 0..4 {
                    let m = melody[ci * 4 + j];
                    e.note_on(m, vel + 0.1);
                    x.extend(render(&mut e, 0.45).0);
                    e.note_off(m);
                }
                for &m in ch.iter() {
                    e.note_off(m);
                }
                e.set_pedal(false);
                x.extend(render(&mut e, 0.1).0);
            }
            let n = 8192;
            let mut pw = vec![0.0; n * 2];
            let mut cnt = 0.0;
            let mut s0 = 0;
            while s0 + n <= x.len() {
                let sp = spectrum(&x, s0, n);
                for (i, m) in sp.iter().enumerate() {
                    pw[i] += m * m;
                }
                cnt += 1.0;
                s0 += n / 2;
            }
            let hz = fs / (2.0 * pw.len() as f64);
            println!("velocity {vel}: 1/3-octave band levels re the strongest band");
            let mut rows = vec![];
            let mut fc = 50.0;
            while fc < 16000.0 {
                let (lo, hi) = (fc / 2f64.powf(1.0 / 6.0), fc * 2f64.powf(1.0 / 6.0));
                let p: f64 = pw.iter().enumerate().filter(|(i, _)| *i as f64 * hz >= lo && (*i as f64 * hz) < hi).map(|(_, v)| v / cnt).sum();
                rows.push((fc, 10.0 * p.max(1e-30).log10()));
                fc *= 2f64.powf(1.0 / 3.0);
            }
            let mx = rows.iter().map(|r| r.1).fold(f64::MIN, f64::max);
            println!("| band [Hz] | level [dB] |");
            println!("|---|---|");
            for (f, l) in rows {
                println!("| {:.0} | {:.1} |", f, l - mx);
            }
        }
        "loudness" => {
            // perceived loudness of every key, with and without the levelling table
            let fs = num(2, 48000.0);
            let vels: Vec<f64> = args.get(3).map(|s| s.split(',').filter_map(|v| v.parse().ok()).collect()).unwrap_or(vec![0.25, 0.45, 0.6, 0.85]);
            let full = args.get(4).map(|s| s == "full").unwrap_or(false);
            let mut e = Engine::new(fs);
            let note = |e: &mut Engine, midi: usize, vel: f64| -> f64 {
                e.all_off();
                e.note_on(midi, vel);
                let n = fs as usize;
                let (mut l, mut r) = (vec![], vec![]);
                while l.len() < n {
                    e.process(128);
                    l.extend_from_slice(&e.out_l[..128]);
                    r.extend_from_slice(&e.out_r[..128]);
                }
                piano_engine::loudness::sone_to_phon(piano_engine::loudness::note_loudness(&l, &r, fs))
            };
            println!("fs = {fs}: loudness level of each key relative to C4 at the same key velocity [phon]");
            println!();
            println!("| velocity | C4 [phon] | levelling off: min / max / std | levelling on: min / max / std | on: keys beyond +-1 phon |");
            println!("|---|---|---|---|---|");
            let mut rows: Vec<Vec<(f64, f64)>> = vec![];
            for &vel in &vels {
                let mut col = vec![];
                let mut stats = vec![];
                for on in [false, true] {
                    e.leveling = on;
                    let c4 = note(&mut e, 60, vel);
                    let d: Vec<f64> = (MIDI_LO..MIDI_LO + NKEYS).map(|m| note(&mut e, m, vel) - c4).collect();
                    let mean = d.iter().sum::<f64>() / d.len() as f64;
                    let sd = (d.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / d.len() as f64).sqrt();
                    stats.push((c4, d.iter().cloned().fold(f64::MAX, f64::min), d.iter().cloned().fold(f64::MIN, f64::max), sd, d.iter().filter(|x| x.abs() > 1.0).count()));
                    col.push(d);
                }
                println!(
                    "| {vel} | {:.1} | {:+.1} / {:+.1} / {:.1} | {:+.1} / {:+.1} / {:.2} | {} |",
                    stats[0].0, stats[0].1, stats[0].2, stats[0].3, stats[1].1, stats[1].2, stats[1].3, stats[1].4
                );
                rows.push(col[0].iter().cloned().zip(col[1].iter().cloned()).collect());
            }
            if full {
                println!();
                println!("| key | {} |", vels.iter().map(|v| format!("vel {v}: off / on")).collect::<Vec<_>>().join(" | "));
                println!("|---|{}", "---|".repeat(vels.len()));
                for k in 0..NKEYS {
                    println!("| {} | {} |", note_name(MIDI_LO + k), rows.iter().map(|r| format!("{:+.1} / {:+.1}", r[k].0, r[k].1)).collect::<Vec<_>>().join(" | "));
                }
            }
        }
        "bass" => {
            // output-stage bass emphasis: response, peak level and distortion
            use piano_engine::post::BassBoost;
            let fs = num(2, 48000.0);
            println!("fs = {fs}");
            println!();
            println!("frequency response [dB] (small signal)");
            println!();
            println!("| amount | 20 Hz | 30 Hz | 50 Hz | 80 Hz | 120 Hz | 180 Hz | 300 Hz | 500 Hz | 1 kHz | 5 kHz |");
            println!("|---|---|---|---|---|---|---|---|---|---|---|");
            for amount in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let mut row = format!("| {amount} |");
                for f in [20.0, 30.0, 50.0, 80.0, 120.0, 180.0, 300.0, 500.0, 1000.0, 5000.0] {
                    let mut b = BassBoost::new(fs);
                    b.set_amount(amount);
                    let n = (2.0 * fs) as usize;
                    let mut l: Vec<f32> = (0..n).map(|i| (0.01 * (2.0 * PI * f * i as f64 / fs).sin()) as f32).collect();
                    let mut r = l.clone();
                    b.process(&mut l, &mut r);
                    let ms: f64 = l[n / 2..].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / (n / 2) as f64;
                    row += &format!(" {:+.1} |", 10.0 * (ms / 0.00005).log10());
                }
                println!("{row}");
            }
            println!();
            println!("music: (a) chord C2+C3+G3+C4+E4+G4 at velocity 0.6 with pedal (ordinary playing), (b) bass chord A0+A1+E2+A2 fff with pedal, (c) all 88 keys fff with pedal (already clipping softly inside the engine)");
            println!();
            println!("| case | amount | peak in [dBFS] | peak out [dBFS] | samples >= 1.0 | max limiter reduction [dB] | level below 200 Hz re amount 0 [dB] | level 1-4 kHz re amount 0 [dB] | deviation from the purely linear filter, i.e. the limiter's level changes [dB re signal] |");
            println!("|---|---|---|---|---|---|---|---|---|");
            for (name, keys, vel) in [("a", vec![36usize, 48, 55, 60, 64, 67], 0.6), ("b", vec![21usize, 33, 40, 45], 1.0), ("c", (MIDI_LO..MIDI_LO + NKEYS).collect::<Vec<_>>(), 1.0)] {
                let mut e = Engine::new(fs);
                e.set_pedal(true);
                for &m in &keys {
                    e.note_on(m, vel);
                }
                let n = (3.0 * fs) as usize;
                let (mut l, mut r) = (vec![], vec![]);
                while l.len() < n {
                    e.process(128);
                    l.extend_from_slice(&e.out_l[..128]);
                    r.extend_from_slice(&e.out_r[..128]);
                }
                let pk = |x: &[f32]| 20.0 * x.iter().fold(0.0f32, |a, &b| a.max(b.abs())).max(1e-9).log10() as f64;
                let band = |x: &[f32], lo: f64, hi: f64| -> f64 {
                    let xd: Vec<f64> = x.iter().map(|v| *v as f64).collect();
                    let sp = spectrum(&xd, 0, 1 << 16);
                    let hz = fs / (2.0 * sp.len() as f64);
                    10.0 * sp.iter().enumerate().filter(|(i, _)| *i as f64 * hz >= lo && (*i as f64 * hz) < hi).map(|(_, m)| m * m).sum::<f64>().max(1e-30).log10()
                };
                for amount in [0.0, 0.5, 1.0] {
                    let mut b = BassBoost::new(fs);
                    b.set_amount(amount);
                    let (mut ol, mut or) = (l.clone(), r.clone());
                    b.process(&mut ol, &mut or);
                    let red = b.take_reduction();
                    // linear reference: same filter driven 60 dB lower, where the limiter never acts
                    let mut b2 = BassBoost::new(fs);
                    b2.set_amount(amount);
                    let (mut ll, mut lr): (Vec<f32>, Vec<f32>) = (l.iter().map(|v| v * 1e-3).collect(), r.iter().map(|v| v * 1e-3).collect());
                    b2.process(&mut ll, &mut lr);
                    let (mut err, mut sig) = (0.0f64, 0.0f64);
                    for i in 0..n {
                        let d = ol[i] as f64 - ll[i] as f64 * 1e3;
                        err += d * d;
                        sig += (ol[i] as f64).powi(2);
                    }
                    let clipped = ol.iter().chain(or.iter()).filter(|v| v.abs() >= 1.0).count();
                    println!(
                        "| {name} | {amount} | {:.1} | {:.1} | {clipped} | {:.1} | {:+.1} | {:+.1} | {} |",
                        pk(&l).max(pk(&r)), pk(&ol).max(pk(&or)), red, band(&ol, 20.0, 200.0) - band(&l, 20.0, 200.0), band(&ol, 1000.0, 4000.0) - band(&l, 1000.0, 4000.0),
                        if err == 0.0 { "none".to_string() } else { format!("{:.1}", 10.0 * (err / sig).log10()) }
                    );
                }
            }
        }
        "stability" => {
            let fs = num(2, 48000.0);
            let mut e = Engine::new(fs);
            e.set_pedal(true);
            for midi in MIDI_LO..MIDI_LO + NKEYS {
                e.note_on(midi, 1.0);
            }
            println!("all 88 keys fff with pedal");
            println!("| t [s] | rms [dB] | peak |");
            println!("|---|---|---|");
            for t in 0..30 {
                let (l, _) = render(&mut e, 2.0);
                let peak = l.iter().fold(0.0f64, |a, &b| a.max(b.abs()));
                println!("| {} | {:.1} | {:.3} |", t * 2, rms_db(&l), peak);
            }
            println!("active voices: {}", e.active_voices());
        }
        "sympathetic" => {
            let fs = num(2, 48000.0);
            // Strike C4 staccato with the pedal down vs up; compare the tail.
            println!("| condition | rms 0.5-1.5 s after release [dB] |");
            println!("|---|---|");
            let amt = num(3, 1.0);
            for (name, pedal, amount) in [("pedal up", false, amt), ("pedal down, resonance on", true, amt), ("pedal down, strings do not feel the board", true, 0.0)] {
                let mut e = Engine::new(fs);
                e.sympathetic = amount;
                e.set_pedal(pedal);
                // mute the struck key itself after release so only other strings remain
                e.note_on(60, 0.8);
                let _ = render(&mut e, 0.3);
                e.note_off(60);
                e.voices[60 - MIDI_LO].drop_damper();
                let (l, _) = render(&mut e, 1.5);
                println!("| {} | {:.1} |", name, rms_db(&l[(0.5 * fs) as usize..]));
            }
        }
        _ => eprintln!("unknown command"),
    }
}
