//! Modal soundboard, two-way coupled to the strings.
//!
//! The modes (frequency, damping, shape at the bridge points, radiation to
//! the microphones) come from `data/board.bin`, the Rayleigh-Ritz solution of
//! a grand-piano shaped orthotropic plate with bridges computed by
//! `src/bin/gen_board.rs`.
//!
//! Every mode is a complex one-pole resonator `s[n] = p s[n-1] + x[n]`; the
//! velocity seen at a bridge point is
//!
//! ```text
//!   v_z[n] = sum_m win_mz ( Re(p s_m[n-1]) + x_m[n] / 2 )  +  G_loc F_z[n]
//!          = h_z[n] + (Y_inst F[n])_z
//! ```
//!
//! i.e. the driving-point admittance is `sum (dt/2M) Re-part of
//! (1 + p z^-1) / (1 - p z^-1)`, which is positive real: the board is a
//! passive load for the strings whatever the mode set, so the coupled
//! instrument cannot become unstable. `h` (the part that depends on the past
//! only) is published after every step; the engine solves the instantaneous
//! part together with the string junctions.
//!
//! The sound is the pressure radiated by the plate (Rayleigh integral of the
//! mode shape, evaluated per mode at the mode's own frequency, which yields
//! one complex weight per mode and receiver): two close microphones, and two
//! distant directions that excite the room.

use crate::design::NZONES;
use crate::simd::F4;
use std::f64::consts::PI;

/// Receivers: two microphones and four room feeds.
pub const NOUT: usize = 6;
/// SIMD lane count of the mode loops.
const W: usize = 4;
/// Local (non modal) bridge conductance [s/kg]: felt/wood losses at the bridge.
pub const G_LOCAL: f64 = 0.6e-4;
/// Largest conductance a single mode may present at a bridge point at its
/// resonance [s/kg] (= 250 kg/s, the order of the lowest impedances measured
/// on grand piano bridges). The plate solution leaves a few low modes much
/// more mobile than that at the ends of the bridges (140 Hz at the bass
/// bridge: 0.021 s/kg, a modal mass of 1 kg). With the dampers up, all
/// strings together load the bridge more than such a mode resists: most of
/// what a strike puts into it ends up in the strings tuned near it, which
/// then drone on for seconds whatever key was played, louder with every
/// repetition. Modes above the limit are coupled to the bridge more weakly
/// (same shape, radiation and decay).
pub const G_CAP: f64 = 4.0e-3;

static TABLE: &[u8] = include_bytes!("../data/board.bin");

/// What the first loop over the modes touches: resonator and bridge
/// points of four modes.
#[derive(Clone, Copy)]
#[repr(C, align(16))]
struct Blk {
    pr: F4,
    pi: F4,
    qr: F4,
    qi: F4,
    wk: F4,
    win: [F4; NZONES],
}

/// What the second loop touches: the radiation weights of the receivers,
/// (on Re q, on Im q) of the state after the step.
#[derive(Clone, Copy)]
#[repr(C, align(16))]
struct RadBlk {
    w: [F4; 2 * NOUT],
}

/// One mode as the analysis functions see it (values as the loops hold them).
#[derive(Clone, Copy)]
struct Mode {
    pr: f32,
    pi: f32,
    win: [f32; NZONES],
    /// Radiation weights (on Re s, on Im s) of the receivers.
    rad: [f32; 2 * NOUT],
}

pub struct Soundboard {
    /// Number of modes.
    pub n: usize,
    blocks: Vec<Blk>,
    rads: Vec<RadBlk>,
    modes: Vec<Mode>,
    pub freqs: Vec<f64>,
    /// Velocity at the bridge points due to the past [m/s].
    pub h: [f64; NZONES],
    /// Instantaneous admittance matrix [s/kg] (includes `G_LOCAL`).
    pub y_inst: [[f64; NZONES]; NZONES],
    peak: f32,
    count: u32,
    pub quiet: bool,
}

impl Soundboard {
    pub fn new(fs: f64) -> Soundboard {
        let word = |i: usize| f32::from_le_bytes([TABLE[4 * i], TABLE[4 * i + 1], TABLE[4 * i + 2], TABLE[4 * i + 3]]) as f64;
        let total = u32::from_le_bytes([TABLE[0], TABLE[1], TABLE[2], TABLE[3]]) as usize;
        let rec = u32::from_le_bytes([TABLE[4], TABLE[5], TABLE[6], TABLE[7]]) as usize;
        assert_eq!(rec, 2 + NZONES + 1 + 2 * NOUT);
        // modes are sorted by frequency
        let n = (0..total).take_while(|&i| word(2 + i * rec) < 0.42 * fs).count();
        let z4 = F4::zero();
        let zero = Blk { pr: z4, pi: z4, qr: z4, qi: z4, wk: z4, win: [z4; NZONES] };
        let mut blocks = vec![zero; n.div_ceil(W)];
        let mut rads = vec![RadBlk { w: [z4; 2 * NOUT] }; n.div_ceil(W)];
        let mut modes = Vec::with_capacity(n);
        let wscale = (1.0 / fs).sqrt();
        // the table's radiation weights are normalised for 48 kHz
        let rscale = (48000.0 / fs).sqrt();
        let mut y_inst = [[0.0; NZONES]; NZONES];
        let mut freqs = vec![];
        for i in 0..n {
            let r = |k: usize| word(2 + i * rec + k);
            let (f, alpha) = (r(0), r(1));
            // conductance of this mode alone at its resonance, at its most mobile bridge point
            let g_res = (0..NZONES).map(|z| r(2 + z) * r(2 + z) / (2.0 * alpha)).fold(0.0, f64::max);
            let bscale = if g_res > G_CAP { (G_CAP / g_res).sqrt() } else { 1.0 };
            let rad = (-alpha / fs).exp();
            let th = 2.0 * PI * f / fs;
            let b = &mut blocks[i / W];
            let j = i % W;
            let mut m = Mode { pr: (rad * th.cos()) as f32, pi: (rad * th.sin()) as f32, win: [0.0; NZONES], rad: [0.0; 2 * NOUT] };
            b.pr.set(j, m.pr);
            b.pi.set(j, m.pi);
            for z in 0..NZONES {
                m.win[z] = (r(2 + z) * bscale * wscale) as f32;
                b.win[z].set(j, m.win[z]);
            }
            for z in 0..NZONES {
                for z2 in 0..NZONES {
                    y_inst[z][z2] += 0.5 * (m.win[z] as f64) * (m.win[z2] as f64);
                }
            }
            b.wk.set(j, (r(2 + NZONES) * wscale) as f32);
            let (pr, pi) = (m.pr as f64, m.pi as f64);
            let p2 = pr * pr + pi * pi;
            for c in 0..NOUT {
                // out = Re(W s) = wr Re(s) - wi Im(s)
                let (w0, w1) = (r(3 + NZONES + 2 * c) * rscale, -r(4 + NZONES + 2 * c) * rscale);
                m.rad[2 * c] = w0 as f32;
                m.rad[2 * c + 1] = w1 as f32;
                // The loop reads the state after the step, q = p s:
                // s = q conj(p) / |p|^2.
                rads[i / W].w[2 * c].set(j, ((w0 * pr - w1 * pi) / p2) as f32);
                rads[i / W].w[2 * c + 1].set(j, ((w0 * pi + w1 * pr) / p2) as f32);
            }
            modes.push(m);
            freqs.push(f);
        }
        for z in 0..NZONES {
            y_inst[z][z] += G_LOCAL;
        }
        Soundboard { n, blocks, rads, modes, freqs, h: [0.0; NZONES], y_inst, peak: 0.0, count: 0, quiet: true }
    }

    /// (1 / (1 - p e^-jw), 1 / (1 - conj(p) e^-jw)) for mode `i`.
    fn poles(&self, i: usize, w: f64) -> ((f64, f64), (f64, f64)) {
        let (pr, pi) = (self.modes[i].pr as f64, self.modes[i].pi as f64);
        let (c, s) = (w.cos(), w.sin());
        let inv = |pr: f64, pi: f64| {
            // 1 - (pr + j pi)(c - j s)
            let (dr, di) = (1.0 - pr * c - pi * s, pr * s - pi * c);
            let d = dr * dr + di * di;
            (dr / d, -di / d)
        };
        (inv(pr, pi), inv(pr, -pi))
    }

    /// Magnitude of the transfer bridge force at `zone` -> (L, R) microphone
    /// (`far == false`) or -> the two room feeds (`far == true`).
    pub fn response(&self, zone: usize, w: f64, far: bool) -> (f64, f64) {
        let o = if far { 4 } else { 0 };
        let (mut l, mut r) = ((0.0, 0.0), (0.0, 0.0));
        for i in 0..self.n {
            let m = &self.modes[i];
            let (a, c) = self.poles(i, w);
            // Re(s) -> (a + c) / 2, Im(s) -> (a - c) / 2j
            let (er, ei) = (0.5 * (a.0 + c.0), 0.5 * (a.1 + c.1));
            let (or, oi) = (0.5 * (a.1 - c.1), -0.5 * (a.0 - c.0));
            let g = m.win[zone] as f64;
            l.0 += g * (m.rad[o] as f64 * er + m.rad[o + 1] as f64 * or);
            l.1 += g * (m.rad[o] as f64 * ei + m.rad[o + 1] as f64 * oi);
            r.0 += g * (m.rad[o + 2] as f64 * er + m.rad[o + 3] as f64 * or);
            r.1 += g * (m.rad[o + 2] as f64 * ei + m.rad[o + 3] as f64 * oi);
        }
        ((l.0 * l.0 + l.1 * l.1).sqrt(), (r.0 * r.0 + r.1 * r.1).sqrt())
    }

    /// Power transfer (mean of |L|^2 and |R|^2) from a force at the bridge
    /// point `sc` (weighted zones) to the microphones, at frequency `f` [Hz].
    pub fn transfer_power(&self, sc: &[f64; NZONES], f: f64, fs: f64) -> f64 {
        let w = 2.0 * PI * f / fs;
        let (mut l, mut r) = ((0.0, 0.0), (0.0, 0.0));
        for i in 0..self.n {
            let m = &self.modes[i];
            let (a, c) = self.poles(i, w);
            let (er, ei) = (0.5 * (a.0 + c.0), 0.5 * (a.1 + c.1));
            let (or, oi) = (0.5 * (a.1 - c.1), -0.5 * (a.0 - c.0));
            let mut g = 0.0;
            for z in 0..NZONES {
                g += sc[z] * m.win[z] as f64;
            }
            l.0 += g * (m.rad[0] as f64 * er + m.rad[1] as f64 * or);
            l.1 += g * (m.rad[0] as f64 * ei + m.rad[1] as f64 * oi);
            r.0 += g * (m.rad[2] as f64 * er + m.rad[3] as f64 * or);
            r.1 += g * (m.rad[2] as f64 * ei + m.rad[3] as f64 * oi);
        }
        0.5 * (l.0 * l.0 + l.1 * l.1 + r.0 * r.0 + r.1 * r.1)
    }

    /// Complex admittance [s/kg] seen by a key whose bridge point is the
    /// weighted combination `sc` of the zones, at frequency `f` [Hz].
    pub fn admittance(&self, sc: &[f64; NZONES], f: f64, fs: f64) -> (f64, f64) {
        let w = 2.0 * PI * f / fs;
        let (mut re, mut im) = (0.0, 0.0);
        for i in 0..self.n {
            let mut g = 0.0;
            for z in 0..NZONES {
                g += sc[z] * self.modes[i].win[z] as f64;
            }
            let (a, c) = self.poles(i, w);
            re += g * g * (0.5 * (a.0 + c.0) - 0.5);
            im += g * g * 0.5 * (a.1 + c.1);
        }
        (re + G_LOCAL * sc.iter().map(|s| s * s).sum::<f64>(), im)
    }

    /// Conductance averaged (log-uniformly) over `f_lo..f_hi`.
    pub fn mean_conductance(&self, sc: &[f64; NZONES], f_lo: f64, f_hi: f64, fs: f64) -> f64 {
        let k = 48;
        (0..k).map(|i| self.admittance(sc, f_lo * (f_hi / f_lo).powf((i as f64 + 0.5) / k as f64), fs).0).sum::<f64>() / k as f64
    }

    pub fn reset(&mut self) {
        for b in self.blocks.iter_mut() {
            b.qr = F4::zero();
            b.qi = F4::zero();
        }
        self.h = [0.0; NZONES];
        self.peak = 0.0;
        self.count = 0;
        self.quiet = true;
    }

    /// Resonators and bridge points for one sample (`fk`: with the key bed
    /// shock).
    #[inline(always)]
    fn resonate(&mut self, f: &[f32; NZONES], fk: Option<f32>) {
        let fv: [F4; NZONES] = std::array::from_fn(|z| F4::splat(f[z]));
        let fkv = F4::splat(fk.unwrap_or(0.0));
        // tiny offset keeps the states out of the denormal range
        let bias = F4::splat(1.0e-18);
        let mut ah = [F4::zero(); NZONES];
        for b in self.blocks.iter_mut() {
            let mut sr = ((bias + b.qr) + (b.win[0] * fv[0] + b.win[1] * fv[1])) + ((b.win[2] * fv[2] + b.win[3] * fv[3]) + (b.win[4] * fv[4] + b.win[5] * fv[5]));
            if fk.is_some() {
                sr += b.wk * fkv;
            }
            let si = b.qi;
            let qr = b.pr * sr - b.pi * si;
            b.qr = qr;
            b.qi = b.pi * sr + b.pr * si;
            for z in 0..NZONES {
                ah[z] += b.win[z] * qr;
            }
        }
        for z in 0..NZONES {
            self.h[z] = ah[z].sum() as f64;
        }
    }

    /// Advance one sample. `f`: force on each bridge point [N], `fk`: key bed
    /// shock [N]. Returns [mic L, mic R, room feeds...] and refreshes `h`.
    #[inline]
    pub fn step(&mut self, f: &[f32; NZONES], fk: f32) -> [f32; NOUT] {
        if fk != 0.0 {
            self.resonate(f, Some(fk));
        } else {
            self.resonate(f, None);
        }
        // radiation
        let mut acc = [F4::zero(); NOUT];
        for (b, r) in self.blocks.iter().zip(self.rads.iter()) {
            let (qr, qi) = (b.qr, b.qi);
            for c in 0..NOUT {
                acc[c] += r.w[2 * c] * qr + r.w[2 * c + 1] * qi;
            }
        }
        let out: [f32; NOUT] = std::array::from_fn(|c| acc[c].sum());
        self.peak = self.peak.max(out[0].abs()).max(out[1].abs());
        self.count += 1;
        if self.count >= 1024 {
            self.quiet = self.peak < 1.0e-7;
            self.peak = 0.0;
            self.count = 0;
        }
        out
    }
}
