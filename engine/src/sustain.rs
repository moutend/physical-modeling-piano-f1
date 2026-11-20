//! Per-key sustain correction: a lookup table applied to the decay rates.
//!
//! How long a key is heard follows from the coupled model (soundboard
//! resonances near its partials, the mistuning of its unison), and so it
//! varies from key to key much more than the nominal rates in `design.rs`
//! do: neighbours may differ by a factor of two. This table evens that out.
//! For every key it holds two multipliers,
//!
//! * `prompt`: on the rate at which the unison loses energy to the board
//!   (the lever between strings and bridge), and
//! * `after`: on the internal losses of the strings (the aftersound),
//!
//! chosen so that the times after which the note has fallen by 20 dB and by
//! 40 dB lie on smooth curves over the keyboard (`gen_sustain`: `target`).
//! `data/sustain.bin` is produced by `src/bin/gen_sustain.rs` and must be
//! regenerated whenever the model changes (before `gen_leveling`, which
//! depends on it).

use crate::design::{MIDI_LO, NKEYS};

static TABLE: &[u8] = include_bytes!("../data/sustain.bin");

/// The key the target curves of `gen_sustain` are centred on (C4).
pub const REF_KEY: usize = 60;

/// (prompt, after) multipliers of `midi`.
pub fn correction(midi: usize) -> (f64, f64) {
    let i = 2 * (midi - MIDI_LO);
    if TABLE.len() < 8 * NKEYS {
        return (1.0, 1.0);
    }
    let f = |i: usize| f32::from_le_bytes([TABLE[4 * i], TABLE[4 * i + 1], TABLE[4 * i + 2], TABLE[4 * i + 3]]) as f64;
    (f(i), f(i + 1))
}

/// Peak level [dB] of a note (10 ms steps of the 50 ms rms of both
/// channels) and the times [s] from that peak until the level has fallen
/// for good by 20 dB and by 40 dB (None: not within the signal).
pub fn sustain_times(l: &[f32], r: &[f32], fs: f64) -> (f64, Option<f64>, Option<f64>) {
    let hop = (0.01 * fs) as usize;
    let win = 5 * hop;
    if l.len() < win {
        return (f64::MIN, None, None);
    }
    let env: Vec<f64> = (0..(l.len() - win) / hop + 1)
        .map(|j| {
            let s: f64 = (j * hop..j * hop + win).map(|n| (l[n] as f64).powi(2) + (r[n] as f64).powi(2)).sum();
            10.0 * (s / (2 * win) as f64 + 1e-30).log10()
        })
        .collect();
    let (ip, peak) = env.iter().enumerate().fold((0, f64::MIN), |a, (j, &v)| if v > a.1 { (j, v) } else { a });
    let last = |drop: f64| {
        let th = peak - drop;
        let j = env.iter().rposition(|&v| v > th)?;
        if j + 1 >= env.len() {
            return None;
        }
        // the crossing lies between steps j and j + 1
        let frac = (env[j] - th) / (env[j] - env[j + 1]);
        Some((j as f64 + frac - ip as f64) * 0.01)
    };
    (peak, last(20.0), last(40.0))
}
