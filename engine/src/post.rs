//! Optional output stage, independent of the piano model: it only sees the
//! finished stereo signal. Remove the two calls in `wasm.rs` (and this file)
//! and the instrument is unchanged.
//!
//! `BassBoost` lifts the low range for small loudspeakers:
//!
//! * a low-shelf filter (up to +10 dB below ~150 Hz, half of that at 280 Hz, at amount 1), which is a
//!   linear tone control and adds no distortion;
//! * a 35 Hz high-pass, so that the boost is not spent on frequencies such
//!   loudspeakers cannot reproduce anyway;
//! * a level trim (-3.5 dB at amount 1) that gives back most of the headroom
//!   the boost uses;
//! * a peak limiter (gain riding, no look-ahead, hence no added latency) that
//!   turns the level down smoothly on the rare peaks that would still clip.
//!
//! At amount 0 the signal passes through untouched.

use std::f64::consts::PI;

const SHELF_HZ: f64 = 280.0;
const MAX_GAIN_DB: f64 = 10.0;
const TRIM_DB: f64 = -3.5;
const HP_HZ: f64 = 35.0;
/// The limiter keeps peaks at or below this level.
const CEILING: f64 = 0.89;
const RELEASE_S: f64 = 0.4;

#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    s: [[f64; 2]; 2],
}

impl Biquad {
    #[inline]
    fn run(&mut self, ch: usize, x: f64) -> f64 {
        let y = self.b[0] * x + self.s[ch][0];
        self.s[ch][0] = self.b[1] * x - self.a[0] * y + self.s[ch][1];
        self.s[ch][1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

pub struct BassBoost {
    fs: f64,
    amount: f64,
    shelf: Biquad,
    hp: Biquad,
    trim: f64,
    gain: f64,
    release: f64,
    /// Lowest limiter gain since the last call of `take_reduction`.
    min_gain: f64,
}

impl BassBoost {
    pub fn new(fs: f64) -> BassBoost {
        let w = 2.0 * PI * HP_HZ / fs;
        let (c, alpha) = (w.cos(), w.sin() / (2.0 * std::f64::consts::FRAC_1_SQRT_2));
        let a0 = 1.0 + alpha;
        let hp = Biquad { b: [(1.0 + c) / 2.0 / a0, -(1.0 + c) / a0, (1.0 + c) / 2.0 / a0], a: [-2.0 * c / a0, (1.0 - alpha) / a0], s: [[0.0; 2]; 2] };
        let mut b = BassBoost { fs, amount: 0.0, shelf: Biquad::default(), hp, trim: 1.0, gain: 1.0, release: 1.0 - (-1.0 / (RELEASE_S * fs)).exp(), min_gain: 1.0 };
        b.set_amount(0.0);
        b
    }

    /// `amount`: 0 (off) .. 1 (strongest).
    pub fn set_amount(&mut self, amount: f64) {
        let amount = amount.clamp(0.0, 1.0);
        self.amount = amount;
        let a = 10f64.powf(MAX_GAIN_DB * amount / 40.0);
        let w = 2.0 * PI * SHELF_HZ / self.fs;
        let (c, s) = (w.cos(), w.sin());
        let slope = 0.8;
        let alpha = s / 2.0 * ((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0).sqrt();
        let k = 2.0 * a.sqrt() * alpha;
        let a0 = (a + 1.0) + (a - 1.0) * c + k;
        self.shelf.b = [a * ((a + 1.0) - (a - 1.0) * c + k) / a0, 2.0 * a * ((a - 1.0) - (a + 1.0) * c) / a0, a * ((a + 1.0) - (a - 1.0) * c - k) / a0];
        self.shelf.a = [-2.0 * ((a - 1.0) + (a + 1.0) * c) / a0, ((a + 1.0) + (a - 1.0) * c - k) / a0];
        self.trim = 10f64.powf(TRIM_DB * amount / 20.0);
    }

    pub fn amount(&self) -> f64 {
        self.amount
    }

    /// Largest gain reduction applied by the limiter since the last call [dB, <= 0].
    pub fn take_reduction(&mut self) -> f64 {
        let db = 20.0 * self.min_gain.log10();
        self.min_gain = 1.0;
        db
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        if self.amount == 0.0 {
            return;
        }
        for i in 0..l.len().min(r.len()) {
            let mut x = [l[i] as f64, r[i] as f64];
            for ch in 0..2 {
                // 1e-25: keeps the filter states out of the denormal range
                x[ch] = self.trim * self.shelf.run(ch, self.hp.run(ch, x[ch] + 1.0e-25));
            }
            let peak = x[0].abs().max(x[1].abs());
            let target = if peak > CEILING { CEILING / peak } else { 1.0 };
            if target < self.gain {
                self.gain = target; // never lets a sample through above the ceiling
            } else {
                self.gain += (target - self.gain) * self.release;
            }
            self.min_gain = self.min_gain.min(self.gain);
            l[i] = (x[0] * self.gain) as f32;
            r[i] = (x[1] * self.gain) as f32;
        }
    }
}
