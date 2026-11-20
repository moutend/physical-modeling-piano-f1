//! Per-key voicing: how much of the strings' tone is radiated directly.
//!
//! What a microphone hears of a note is the sum over the soundboard's modes,
//! and at single frequencies those cancel: seen from a microphone, the
//! fundamental of one key may sit 10 dB below that of its neighbours. The
//! blow of the hammer (broadband) does not share those gaps, so on such a
//! key the knock stands out against a thin tone, the levelling asks for a
//! faster hammer (more knock still), and the room, which is fed by other
//! directions of the board, is heard louder than the note itself.
//!
//! The strings' own signal at the bridge does not have these gaps. Each key
//! therefore also radiates a band-passed copy of its bridge wave directly
//! (`Voice::direct_out`, mixed into both microphones by `Engine::process`),
//! with a gain per key from `data/voicing.bin`: zero where the board serves
//! the key well, and otherwise just enough to bring the ratio of blow to
//! tone down to a smooth target over the keyboard. The table is produced by
//! `src/bin/gen_voicing.rs` (after `gen_sustain`, before `gen_leveling`)
//! and must be regenerated whenever the model changes.

use crate::design::{key_params, partial_freq, MIDI_LO, NKEYS};
use crate::loudness::fft;
use std::f64::consts::PI;

static TABLE: &[u8] = include_bytes!("../data/voicing.bin");

/// The entry of `midi` in the table: the gain found for the key itself.
pub fn own_gain(midi: usize) -> f64 {
    let i = midi - MIDI_LO;
    if TABLE.len() < 4 * NKEYS {
        return 0.0;
    }
    f32::from_le_bytes([TABLE[4 * i], TABLE[4 * i + 1], TABLE[4 * i + 2], TABLE[4 * i + 3]]) as f64
}

/// Direct radiation gain of `midi`. A key between the reference keys
/// radiates like those (the blend of their gains, see `morph.rs`; beyond
/// the outermost reference key: like that one), not with its own entry.
pub fn direct_gain(midi: usize) -> f64 {
    match crate::morph::blend(midi) {
        Some(bl) => crate::morph::mix(midi, bl, own_gain(bl.0), own_gain(bl.1), 0.0),
        None => own_gain(midi),
    }
}

pub struct Attack {
    /// Peak of the mono sum [dBFS].
    pub peak: f64,
    /// rms of 50..300 ms [dB].
    pub tone: f64,
    /// Blow energy re 25 ms of tone [dB]: < 500 Hz, 0.5..2 kHz, > 2 kHz, all.
    pub blow: [f64; 4],
}

/// The blow against the tone of a note (`l`, `r`: from its strike, at least
/// 0.5 s). The blow is what the note holds between its partials: the energy
/// of the first moments outside narrow bands around the partial
/// frequencies, given relative to the energy of 25 ms of the tone.
pub fn attack_measure(l: &[f64], r: &[f64], fs: f64, midi: usize) -> Attack {
    let k = key_params(midi);
    let peak = 20.0 * l.iter().zip(r).fold(0.0f64, |a, (x, y)| a.max(0.5 * (x + y).abs())).log10();
    let (a, b) = ((0.05 * fs) as usize, (0.3 * fs) as usize);
    let tone_p = (a..b).map(|i| 0.5 * (l[i] * l[i] + r[i] * r[i])).sum::<f64>() / (b - a) as f64;
    // Hann window whose first quarter lies before the strike (silence)
    let len = (((0.12f64).max(8.0 / k.f1) * fs) as usize).min(l.len() * 4 / 3);
    let n = len.next_power_of_two();
    let pre = len / 4;
    let df = fs / n as f64;
    let mut pw = vec![0.0; n / 2];
    for ch in [l, r] {
        let mut re = vec![0.0; n];
        let mut im = vec![0.0; n];
        for i in pre..len {
            re[i] = ch[i - pre] * (0.5 - 0.5 * (2.0 * PI * i as f64 / len as f64).cos());
        }
        fft(&mut re, &mut im);
        for (q, p) in pw.iter_mut().enumerate() {
            // energy per bin, mean of the two channels
            *p += (re[q] * re[q] + im[q] * im[q]) / n as f64;
        }
    }
    // bins that belong to a partial
    let mut tonal = vec![false; n / 2];
    let mut j = 1;
    loop {
        let f = partial_freq(k.f1, k.b_inh, j);
        if f > 12000.0 || j > 400 {
            break;
        }
        let h = (0.25 * k.f1).min((0.04 * f).max(4.0 * df));
        for q in ((f - h) / df).floor().max(0.0) as usize..=(((f + h) / df).ceil() as usize).min(n / 2 - 1) {
            tonal[q] = true;
        }
        j += 1;
    }
    let mut blow = [0.0; 4];
    for (bi, (lo, hi)) in [(60.0, 500.0), (500.0, 2000.0), (2000.0, 10000.0)].into_iter().enumerate() {
        let (q0, q1) = ((lo / df) as usize, ((hi / df) as usize).min(n / 2));
        let kept = (q0..q1).filter(|&q| !tonal[q]).count();
        if kept > 0 {
            let e: f64 = (q0..q1).filter(|&q| !tonal[q]).map(|q| pw[q]).sum();
            // make up for the excluded bins
            blow[bi] = e * (q1 - q0) as f64 / kept as f64;
        }
    }
    blow[3] = blow[0] + blow[1] + blow[2];
    let reference = tone_p * 0.025 * fs;
    Attack { peak, tone: 10.0 * tone_p.max(1e-30).log10(), blow: blow.map(|e| 10.0 * (e / reference).max(1e-12).log10()) }
}
