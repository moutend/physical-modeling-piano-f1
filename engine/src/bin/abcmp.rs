//! Compare two renderings (WAV files of `render`), e.g. of the engine
//! before and after a change: level of the difference signal, and
//! third-octave band level differences over time windows.
//!
//!   abcmp ref.wav new.wav                      summary per window (0.05, 0.25, 1, 2, 4 ... s)
//!   abcmp ref.wav new.wav x                    same, with the difference of every band
//!   abcmp ref.wav new.wav <from> <to> <step>   windows of <step> seconds; per band:
//!                                              level in ref (re the window's total) / difference
//!
//! Only bands within 60 dB of a window's total count for "max band dev".
use std::f64::consts::PI;

fn read(path: &str) -> (Vec<f64>, Vec<f64>, f64) {
    let b = std::fs::read(path).expect("cannot read");
    let u16 = |p: usize| u16::from_le_bytes([b[p], b[p + 1]]) as usize;
    let u32 = |p: usize| u32::from_le_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]]) as usize;
    let mut p = 12;
    let (mut fmt, mut ch, mut fs, mut bits) = (1, 2, 48000.0, 16);
    let (mut l, mut r) = (vec![], vec![]);
    while p + 8 <= b.len() {
        let (id, len) = (&b[p..p + 4], u32(p + 4));
        if id == b"fmt " {
            fmt = u16(p + 8);
            ch = u16(p + 10);
            fs = u32(p + 12) as f64;
            bits = u16(p + 22);
        } else if id == b"data" {
            let bytes = bits / 8;
            let n = len.min(b.len() - p - 8) / (bytes * ch);
            for i in 0..n {
                let s = |c: usize| {
                    let q = p + 8 + (i * ch + c) * bytes;
                    match (fmt, bits) {
                        (3, 32) => f32::from_le_bytes([b[q], b[q + 1], b[q + 2], b[q + 3]]) as f64,
                        (1, 16) => i16::from_le_bytes([b[q], b[q + 1]]) as f64 / 32768.0,
                        (1, 24) => (i32::from_le_bytes([0, b[q], b[q + 1], b[q + 2]]) >> 8) as f64 / 8388608.0,
                        _ => panic!("unsupported format"),
                    }
                };
                l.push(s(0));
                r.push(s(ch - 1));
            }
        }
        p += 8 + len + (len & 1);
    }
    (l, r, fs)
}

fn db(x: f64) -> f64 {
    10.0 * (x + 1e-30).log10()
}

fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f64;
        for s in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (c, sn) = ((ang * k as f64).cos(), (ang * k as f64).sin());
                let (a, b) = (s + k, s + k + len / 2);
                let (tr, ti) = (re[b] * c - im[b] * sn, re[b] * sn + im[b] * c);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// Power per third-octave band (25 Hz .. 20 kHz) of x[a..b].
fn bands(x: &[f64], fs: f64) -> Vec<f64> {
    let n = x.len().next_power_of_two();
    let (mut re, mut im) = (vec![0.0; n], vec![0.0; n]);
    for (i, &v) in x.iter().enumerate() {
        re[i] = v * (0.5 - 0.5 * (2.0 * PI * i as f64 / x.len() as f64).cos());
    }
    fft(&mut re, &mut im);
    let mut out = vec![0.0; 30];
    for k in 1..n / 2 {
        let f = k as f64 * fs / n as f64;
        let b = (3.0 * (f / 25.0).log2() + 0.5).floor();
        if b >= 0.0 && (b as usize) < 30 {
            out[b as usize] += re[k] * re[k] + im[k] * im[k];
        }
    }
    out
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (rl, rr, fs) = read(&a[1]);
    let (nl, nr, _) = read(&a[2]);
    let n = rl.len().min(nl.len());
    let pw = |x: &[f64]| x.iter().map(|v| v * v).sum::<f64>() / x.len().max(1) as f64;
    let total = pw(&rl[..n]) + pw(&rr[..n]);
    let dl: Vec<f64> = (0..n).map(|i| nl[i] - rl[i]).collect();
    let dr: Vec<f64> = (0..n).map(|i| nr[i] - rr[i]).collect();
    println!("level: ref {:.2} dB, new {:.2} dB ({:+.3} dB); difference signal {:.1} dB rel. ref", db(total), db(pw(&nl[..n]) + pw(&nr[..n])), db(pw(&nl[..n]) + pw(&nr[..n])) - db(total), db(pw(&dl) + pw(&dr)) - db(total));
    // windows
    let secs = n as f64 / fs;
    let mut edges = vec![0.0, 0.05, 0.25, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 1e9];
    edges.retain(|&e| e < secs);
    edges.push(secs);
    // abcmp ref new <from> <to> <step>: windows of <step> seconds, with all bands
    if let (Some(t0), Some(t1), Some(dt)) = (a.get(3).and_then(|s| s.parse::<f64>().ok()), a.get(4).and_then(|s| s.parse::<f64>().ok()), a.get(5).and_then(|s| s.parse::<f64>().ok())) {
        edges = (0..).map(|i| t0 + i as f64 * dt).take_while(|&t| t < t1.min(secs)).collect();
        edges.push(t1.min(secs));
    }
    let mut worst = (0.0f64, 0.0, 0.0);
    for w in edges.windows(2) {
        let (i0, i1) = ((w[0] * fs) as usize, ((w[1] * fs) as usize).min(n));
        if i1 <= i0 + 64 {
            continue;
        }
        let p_ref = pw(&rl[i0..i1]) + pw(&rr[i0..i1]);
        let p_new = pw(&nl[i0..i1]) + pw(&nr[i0..i1]);
        let p_d = pw(&dl[i0..i1]) + pw(&dr[i0..i1]);
        // band levels (mid signal and side signal)
        let mid = |l: &[f64], r: &[f64]| -> Vec<f64> { (i0..i1).map(|i| 0.5 * (l[i] + r[i])).collect() };
        let side = |l: &[f64], r: &[f64]| -> Vec<f64> { (i0..i1).map(|i| 0.5 * (l[i] - r[i])).collect() };
        let (bm_r, bm_n) = (bands(&mid(&rl, &rr), fs), bands(&mid(&nl, &nr), fs));
        let (bs_r, bs_n) = (bands(&side(&rl, &rr), fs), bands(&side(&nl, &nr), fs));
        let tot: f64 = bm_r.iter().sum::<f64>() + bs_r.iter().sum::<f64>();
        let mut line = String::new();
        let mut wmax = 0.0f64;
        for b in 0..30 {
            // only bands that matter (within 60 dB of the window's total)
            let (pr, pn) = (bm_r[b] + bs_r[b], bm_n[b] + bs_n[b]);
            let d = db(pn) - db(pr);
            if pr > 1e-6 * tot {
                wmax = wmax.max(d.abs());
                if d.abs() > worst.0 {
                    worst = (d.abs(), w[0], 25.0 * 2f64.powf(b as f64 / 3.0));
                }
            }
            if a.len() > 5 {
                line += &format!("{:.0}/{:+.1} ", db(pr / tot), d);
            } else if pr > 1e-6 * tot {
                line += &format!("{:+.1} ", d);
            } else {
                line += "  .  ";
            }
        }
        println!("{:6.2}-{:6.2}s  ref {:6.1} dB  level {:+.3} dB  diff {:6.1} dB  max band dev {:.2} dB", w[0], w[1], db(p_ref), db(p_new) - db(p_ref), db(p_d) - db(p_ref), wmax);
        if a.len() > 3 {
            println!("    {line}");
        }
    }
    println!("worst third-octave deviation: {:.2} dB (window from {:.2} s, band {:.0} Hz)", worst.0, worst.1, worst.2);
}
