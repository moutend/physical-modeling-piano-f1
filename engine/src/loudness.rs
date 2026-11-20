//! Perceived loudness of a rendered note, for the per-key levelling table.
//!
//! Third-octave band levels are converted to loudness level (phon) with the
//! ISO 226:2003 equal-loudness contours, then to sones and summed with the
//! band-summation rule of Stevens' method (ISO 532 method A). This is an
//! offline measurement tool; nothing here runs in the audio loop.

use std::f64::consts::PI;

/// Sound pressure level of a full-scale (RMS 1.0) signal [dB SPL]: a
/// mezzo-forte note at about -30 dBFS RMS is then heard at about 70 dB.
pub const CAL_DB: f64 = 100.0;

const F: [f64; 29] = [
    20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0, 500.0, 630.0, 800.0, 1000.0, 1250.0, 1600.0,
    2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0, 8000.0, 10000.0, 12500.0,
];
const AF: [f64; 29] = [
    0.532, 0.506, 0.480, 0.455, 0.432, 0.409, 0.387, 0.367, 0.349, 0.330, 0.315, 0.301, 0.288, 0.276, 0.267, 0.259, 0.253, 0.250, 0.246, 0.244,
    0.243, 0.243, 0.243, 0.242, 0.242, 0.245, 0.254, 0.271, 0.301,
];
const LU: [f64; 29] = [
    -31.6, -27.2, -23.0, -19.1, -15.9, -13.0, -10.3, -8.1, -6.2, -4.5, -3.1, -2.0, -1.1, -0.4, 0.0, 0.3, 0.5, 0.0, -2.7, -4.1, -1.0, 1.7, 2.5, 1.2,
    -2.1, -7.1, -11.2, -10.7, -3.1,
];
const TF: [f64; 29] = [
    78.5, 68.7, 59.5, 51.1, 44.0, 37.5, 31.5, 26.5, 22.1, 17.9, 14.4, 11.4, 8.6, 6.2, 4.4, 3.0, 2.2, 2.4, 3.5, 1.7, -1.3, -4.2, -6.0, -5.4, -1.5,
    6.0, 12.6, 13.9, 12.3,
];

/// Loudness level [phon] of a band at centre frequency index `i` with sound
/// pressure level `lp` [dB] (inverse of the ISO 226 contour formula).
pub fn phon(i: usize, lp: f64) -> f64 {
    let b = (0.4 * 10f64.powf((lp + LU[i]) / 10.0 - 9.0)).powf(AF[i]) - (0.4 * 10f64.powf((TF[i] + LU[i]) / 10.0 - 9.0)).powf(AF[i]) + 0.005135;
    if b <= 0.0 {
        0.0
    } else {
        (40.0 * b.log10() + 94.0).max(0.0)
    }
}

fn sone(p: f64) -> f64 {
    if p >= 40.0 {
        2f64.powf((p - 40.0) / 10.0)
    } else {
        (p / 40.0).powf(2.642)
    }
}

pub fn sone_to_phon(s: f64) -> f64 {
    if s >= 1.0 {
        40.0 + 10.0 * s.log2()
    } else {
        40.0 * s.max(1e-9).powf(1.0 / 2.642)
    }
}

pub fn fft(re: &mut [f64], im: &mut [f64]) {
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

/// Loudness [sone] of a note: the loudest ~0.2 s of the stereo signal.
pub fn note_loudness(l: &[f32], r: &[f32], fs: f64) -> f64 {
    let n = if fs > 64000.0 { 8192 } else { 4096 };
    let hop = n / 2;
    let win: Vec<f64> = (0..n).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos()).collect();
    let wsum2: f64 = win.iter().map(|w| w * w).sum();
    let hz = fs / n as f64;
    // third-octave bands 25 Hz .. 12.5 kHz as bin ranges
    let bands: Vec<(usize, usize, usize)> = (1..29)
        .map(|i| {
            // The bands meet halfway (geometrically) between the nominal
            // centre frequencies. Those are rounded values: bands of exactly
            // a third of an octave around them leave gaps (3536..3564 Hz
            // holds the fundamental of A7, which then counted for nothing)
            // and overlap elsewhere.
            let lo = ((F[i - 1] * F[i]).sqrt() / hz).ceil() as usize;
            let hi = if i + 1 < F.len() { ((F[i] * F[i + 1]).sqrt() / hz).ceil() as usize } else { (F[i] * 2f64.powf(1.0 / 6.0) / hz).ceil() as usize }.min(n / 2);
            (i, lo.max(1), hi)
        })
        .collect();
    let mut frames: Vec<f64> = vec![];
    let (mut re, mut im) = (vec![0.0; n], vec![0.0; n]);
    let mut pw = vec![0.0; n / 2 + 1];
    let mut s0 = 0;
    while s0 + n <= l.len() {
        pw.fill(0.0);
        for ch in [l, r] {
            for i in 0..n {
                re[i] = ch[s0 + i] as f64 * win[i];
                im[i] = 0.0;
            }
            fft(&mut re, &mut im);
            for k in 0..=n / 2 {
                // mean square per bin, averaged over the two ears
                pw[k] += (re[k] * re[k] + im[k] * im[k]) / (n as f64 * wsum2);
            }
        }
        let (mut sum, mut max) = (0.0, 0.0f64);
        for &(i, lo, hi) in &bands {
            let ms: f64 = pw[lo..hi.max(lo)].iter().sum();
            if ms <= 0.0 {
                continue;
            }
            let s = sone(phon(i, 10.0 * ms.log10() + CAL_DB));
            sum += s;
            max = max.max(s);
        }
        frames.push(max + 0.15 * (sum - max));
        s0 += hop;
    }
    // loudest run of three overlapping frames
    frames.windows(3).map(|w| (w[0] + w[1] + w[2]) / 3.0).fold(0.0, f64::max)
}
