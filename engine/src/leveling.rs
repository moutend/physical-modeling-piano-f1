//! Per-key loudness levelling: a lookup table in front of the engine.
//!
//! The physical model is left as it is; this table only rescales the hammer
//! velocity that a key press asks for, so that every key is heard as loud as
//! the bass register (median of C2..C3, whose entries are kept as they were
//! judged) played with the same force.
//! `data/leveling.bin` is produced by
//! `src/bin/gen_leveling.rs`, which measures each key's loudness (ISO 226
//! equal-loudness contours, see `loudness.rs`) and solves for the scale.
//! The table must be regenerated whenever the model changes.

use crate::design::{MIDI_LO, NKEYS};

static TABLE: &[u8] = include_bytes!("../data/leveling.bin");

pub struct Leveling {
    /// Key velocities (0..1) at which the table was measured.
    levels: Vec<f64>,
    /// scale[key][level]: hammer velocity multiplier.
    scale: Vec<f64>,
}

impl Leveling {
    /// Load the table measured at the sample rate closest to `fs`.
    pub fn new(fs: f64) -> Leveling {
        let u = |i: usize| u32::from_le_bytes([TABLE[4 * i], TABLE[4 * i + 1], TABLE[4 * i + 2], TABLE[4 * i + 3]]) as usize;
        let f = |i: usize| f32::from_le_bytes([TABLE[4 * i], TABLE[4 * i + 1], TABLE[4 * i + 2], TABLE[4 * i + 3]]) as f64;
        let (n_rates, n_levels) = (u(0), u(1));
        let levels: Vec<f64> = (0..n_levels).map(|j| f(2 + j)).collect();
        let block = 1 + NKEYS * n_levels;
        let base = 2 + n_levels;
        let mut best: Option<usize> = None;
        for r in 0..n_rates {
            let d = (f(base + r * block) / fs).ln().abs();
            if best.map(|b| d < (f(base + b * block) / fs).ln().abs()).unwrap_or(true) {
                best = Some(r);
            }
        }
        let scale = match best {
            Some(r) => (0..NKEYS * n_levels).map(|i| f(base + r * block + 1 + i)).collect(),
            None => vec![1.0; NKEYS * n_levels],
        };
        Leveling { levels, scale }
    }

    /// The table's entry for `midi` at its `j`-th key velocity (for
    /// `gen_leveling`, which writes kept entries back unchanged).
    pub fn entry(&self, midi: usize, j: usize) -> f64 {
        self.scale[(midi - MIDI_LO) * self.levels.len() + j]
    }

    /// Hammer velocity multiplier for `midi` struck with key velocity `vel` (0..1).
    pub fn scale(&self, midi: usize, vel: f64) -> f64 {
        let n = self.levels.len();
        if n == 0 || !(MIDI_LO..MIDI_LO + NKEYS).contains(&midi) {
            return 1.0;
        }
        let row = &self.scale[(midi - MIDI_LO) * n..(midi - MIDI_LO + 1) * n];
        if vel <= self.levels[0] {
            return row[0];
        }
        for j in 1..n {
            if vel <= self.levels[j] {
                let t = (vel - self.levels[j - 1]) / (self.levels[j] - self.levels[j - 1]);
                // interpolate the scale in decibels
                return row[j - 1] * (row[j] / row[j - 1]).powf(t);
            }
        }
        row[n - 1]
    }
}
