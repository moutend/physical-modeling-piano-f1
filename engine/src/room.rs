//! The room the piano stands in: early reflections from the image sources of
//! a shoebox room, followed by a feedback delay network for the diffuse
//! field. Nothing here is recorded; the direct sound bypasses this module, so
//! it adds no latency.

use crate::bank::CHUNK;
use crate::simd::axpy;

const C_AIR: f64 = 343.0;
/// Room size [m] and amplitude reflection coefficient of the surfaces.
const ROOM: (f64, f64, f64) = (9.6, 7.1, 3.9);
const BETA: f64 = 0.82;
/// Source (centre of the soundboard) and the two microphones.
const SRC: (f64, f64, f64) = (4.1, 3.3, 1.0);
const MICS: [(f64, f64, f64); 2] = [(3.73, 3.2, 1.75), (4.47, 3.2, 1.75)];
/// Reverberation time at low / high (4 kHz) frequencies [s].
const T60_LO: f64 = 1.0;
const T60_HI: f64 = 0.6;
/// Diffuse-field energy relative to the direct sound at full amount [dB].
const LATE_DB: f64 = -5.0;
const NL: usize = 8;
/// Number of feeds (directions in which the piano's radiation is known).
pub const NF: usize = 4;
const LINE_MS: [f64; NL] = [29.7, 34.3, 39.1, 43.9, 50.3, 56.9, 63.7, 71.3];

pub struct Room {
    /// One delay line per radiation direction of the piano.
    bufs: [Vec<f32>; NF],
    mask: usize,
    wpos: usize,
    /// (delay, gain, direction) of each reflection, per microphone.
    taps: [Vec<(usize, f32, usize)>; 2],
    er_c: f32,
    er_s: [f32; 2],
    pre: usize,
    lines: [Vec<f32>; NL],
    pos: [usize; NL],
    g: [f32; NL],
    b: [f32; NL],
    lp: [f32; NL],
    late_gain: f32,
    peak: f32,
    count: u32,
    pub quiet: bool,
}

impl Room {
    pub fn new(fs: f64) -> Room {
        let mut r = Room::build(fs);
        // Calibrate the diffuse level from the network's own impulse response.
        let mut e = 0.0f64;
        let n = (2.5 * T60_LO * fs) as usize;
        for i in 0..n {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let (l, rr) = r.late([x; NF]);
            e += 0.5 * (l as f64 * l as f64 + rr as f64 * rr as f64);
        }
        r.reset();
        // the test impulse was coherent in all feeds; real feeds add in power
        r.late_gain = (10f64.powf(LATE_DB / 10.0) / e).sqrt() as f32;
        r
    }

    fn build(fs: f64) -> Room {
        // image sources up to second order
        let mut taps: [Vec<(usize, f32, usize)>; 2] = [vec![], vec![]];
        for (c, mic) in MICS.iter().enumerate() {
            let d0 = dist(SRC, *mic);
            for ix in -2i32..=2 {
                for iy in -2i32..=2 {
                    for iz in -2i32..=2 {
                        let order = ix.abs() + iy.abs() + iz.abs();
                        if order == 0 || order > 2 {
                            continue;
                        }
                        let img = (image(SRC.0, ROOM.0, ix), image(SRC.1, ROOM.1, iy), image(SRC.2, ROOM.2, iz));
                        let d = dist(img, *mic);
                        let gain = d0 / d * BETA.powi(order);
                        let delay = ((d - d0) / C_AIR * fs).round() as usize;
                        // each image sees the piano from another side
                        let dir = ((ix * 7 + iy * 3 + iz * 5).rem_euclid(NF as i32)) as usize;
                        taps[c].push((delay.max(1), gain as f32, dir));
                    }
                }
            }
        }
        let max_delay = taps.iter().flatten().map(|t| t.0).max().unwrap();
        // (a chunk is written before its reflections are read)
        let size = (max_delay + CHUNK + 2).next_power_of_two();
        let mut lines: [Vec<f32>; NL] = Default::default();
        let (mut g, mut b) = ([0.0; NL], [0.0; NL]);
        let wc = 2.0 * std::f64::consts::PI * 4000.0 / fs;
        for i in 0..NL {
            let len = ((LINE_MS[i] * 1e-3 * fs).round() as usize) | 1;
            lines[i] = vec![0.0; len];
            let g_lo = 10f64.powf(-3.0 * len as f64 / (fs * T60_LO));
            let g_hi = 10f64.powf(-3.0 * len as f64 / (fs * T60_HI));
            // one-pole lowpass whose gain at 4 kHz is g_hi / g_lo
            let rho2 = (g_hi / g_lo).powi(2);
            let k = 1.0 - rho2 * wc.cos();
            let bb = (k - (k * k - (1.0 - rho2).powi(2)).max(0.0).sqrt()) / (1.0 - rho2);
            g[i] = g_lo as f32;
            b[i] = bb as f32;
        }
        Room {
            bufs: std::array::from_fn(|_| vec![0.0; size]),
            mask: size - 1,
            wpos: 0,
            taps,
            er_c: (-2.0 * std::f64::consts::PI * 7000.0 / fs).exp() as f32,
            er_s: [0.0; 2],
            pre: ((0.012 * fs) as usize).min(size - 1),
            lines,
            pos: [0; NL],
            g,
            b,
            lp: [0.0; NL],
            late_gain: 1.0,
            peak: 0.0,
            count: 0,
            quiet: true,
        }
    }

    pub fn reset(&mut self) {
        for b in self.bufs.iter_mut() {
            b.fill(0.0);
        }
        for l in self.lines.iter_mut() {
            l.fill(0.0);
        }
        self.lp = [0.0; NL];
        self.er_s = [0.0; 2];
        self.peak = 0.0;
        self.count = 0;
        self.quiet = true;
    }

    /// Feedback delay network (Hadamard mixing, frequency dependent decay).
    #[inline]
    fn late(&mut self, x: [f32; NF]) -> (f32, f32) {
        let mut u = [0.0f32; NL];
        for i in 0..NL {
            let y = self.lines[i][self.pos[i]];
            self.lp[i] = (1.0 - self.b[i]) * y + self.b[i] * self.lp[i] + 1.0e-20;
            u[i] = self.g[i] * self.lp[i];
        }
        let (l, r) = (u[0] - u[2] + u[4] - u[6], u[1] - u[3] + u[5] - u[7]);
        // 8x8 Hadamard transform
        let mut h = 1;
        while h < NL {
            let mut i = 0;
            while i < NL {
                for j in i..i + h {
                    let (a, c) = (u[j], u[j + h]);
                    u[j] = a + c;
                    u[j + h] = a - c;
                }
                i += 2 * h;
            }
            h *= 2;
        }
        const S: f32 = 0.353_553_4; // 1 / sqrt(8)
        const SIGN: [f32; NL] = [1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0];
        for i in 0..NL {
            self.lines[i][self.pos[i]] = S * u[i] + SIGN[i] * x[i % NF];
            self.pos[i] += 1;
            if self.pos[i] == self.lines[i].len() {
                self.pos[i] = 0;
            }
        }
        (l, r)
    }

    /// `x`: `n` samples of the piano's sound in `NF` directions. Writes the
    /// room's contribution (reflections + reverberation) to the two
    /// microphone signals at full amount to `wl` / `wr`.
    pub fn process(&mut self, x: &[[f32; CHUNK]; NF], n: usize, wl: &mut [f32; CHUNK], wr: &mut [f32; CHUNK]) {
        let (w, size) = (self.wpos, self.mask + 1);
        let first = n.min(size - w);
        for d in 0..NF {
            self.bufs[d][w..w + first].copy_from_slice(&x[d][..first]);
            self.bufs[d][..n - first].copy_from_slice(&x[d][first..n]);
        }
        // early reflections
        let mut er = [[0.0f32; CHUNK]; 2];
        for c in 0..2 {
            for &(d, g, dir) in self.taps[c].iter() {
                let buf = &self.bufs[dir];
                let start = (w + size - d) & self.mask;
                let first = n.min(size - start);
                axpy(&mut er[c][..first], g, &buf[start..start + first]);
                axpy(&mut er[c][first..n], g, &buf[..n - first]);
            }
            let mut s = self.er_s[c];
            for y in er[c][..n].iter_mut() {
                s = *y + self.er_c * (s - *y);
                *y = s;
            }
            self.er_s[c] = s;
        }
        // reverberation
        let mut peak = self.peak;
        for t in 0..n {
            let p = (w + t + size - self.pre) & self.mask;
            let (ll, lr) = self.late(std::array::from_fn(|d| self.bufs[d][p]));
            wl[t] = er[0][t] + self.late_gain * ll;
            wr[t] = er[1][t] + self.late_gain * lr;
            peak = peak.max(wl[t].abs()).max(wr[t].abs());
        }
        self.wpos = (w + n) & self.mask;
        self.peak = peak;
        self.count += n as u32;
        if self.count >= 1024 {
            self.quiet = self.peak < 1.0e-7;
            self.peak = 0.0;
            self.count = 0;
        }
    }
}

fn dist(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)).sqrt()
}

/// Coordinate of the image source of order `i` along one axis of length `len`.
fn image(x: f64, len: f64, i: i32) -> f64 {
    if i % 2 == 0 {
        i as f64 * len + x
    } else {
        (i + 1) as f64 * len - x
    }
}
