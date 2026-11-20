//! Per-key timbre morphing: the keys between the reference keys take their
//! tone from them.
//!
//! The parameters in `design.rs` vary smoothly over the keyboard, yet the
//! keys do not sound alike: how strongly each partial of a key reaches the
//! microphones depends on where it falls between the soundboard's
//! resonances, and that differs from key to key by many dB. On the keys in
//! `ANCHORS` the result was judged (by ear) to be right. Every other key is
//! made a blend of the reference keys around it:
//!
//! * its strings radiate directly like theirs (blend of their gains in
//!   `voicing.rs`; beyond the outermost reference keys: like that one),
//! * each of its partials reaches each microphone at the blend of the levels
//!   the same partial has on the reference keys (in dB, linear in the key
//!   number). Below the lowest and above the highest reference key the
//!   trend between the last two reference keys is continued.
//! * The blow of its hammer stands against its tone as on the reference
//!   keys: the correction is made for the hammer velocity at which that is
//!   the case (the level of the tone being fixed by the previous point, the
//!   hammer only moves the blow).
//!
//! The loudness levelling of such a key is then solved with this correction
//! in place (`gen_leveling`, run once more after `gen_morph`), so that it
//! is as loud as every other key at every key velocity.
//!
//! The partial levels are set by a small bank of resonators per key, one per
//! partial, driven by the key's bridge wave and mixed into the microphone
//! signals with a complex gain per partial and channel (`data/morph.bin`,
//! produced by `src/bin/gen_morph.rs`, the last of the generated tables).
//! The gain is aligned with the phase the partial already has at that
//! microphone, so it acts as a plain level change of the partial; the
//! soundboard model itself and the reference keys are left as they are.
//!
//! The treble (from C6) is voiced with the same resonators: there the
//! partials above the fundamental are put at fixed levels below it on
//! every key, the reference keys included (`voice_treble`).

use crate::design::{partial_freq, KeyParams, MIDI_LO, NKEYS};
use crate::simd::F4;
use std::f64::consts::PI;

static TABLE: &[u8] = include_bytes!("../data/morph.bin");

/// The reference keys.
pub const ANCHORS: [usize; 41] = [
    26, 31, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 62, 64, 65, 67, 72, 74, 76, 77, 84,
    93, 98, 103,
    // (sentinel: never matches a key)
    usize::MAX,
];

/// Partials per key at most, and the highest partial frequency corrected [Hz].
pub const MAX_PARTIALS: usize = 48;
pub const MAX_HZ: f64 = 10000.0;
/// Largest steepness of a continued trend [dB per key].
pub const MAX_TREND: f64 = 1.5;

/// Voicing of the treble: from `TREBLE_FULL` (C6) up, partials 2 to 4 of
/// every key, the reference keys included, stand at these levels below the
/// fundamental [dB] (the fundamental as the two microphones hold it
/// together). Between `TREBLE_LO` and `TREBLE_FULL` the levels of the
/// blend go over into these.
///
/// Up there a blow lasts about as long as a period of the note, and how
/// much of the second and third partial a hammer leaves on the string
/// swings by 30 dB from one key to the next (whatever the hammer; in the
/// model as on an instrument that has not been voiced). Left alone, one
/// key is glassy and the next a bare sine; with soft, light hammers all
/// are sines, which is even but dull. These levels are the middle of what
/// the keys do with harder hammers: every key gets the same brilliance.
pub const TREBLE_LO: usize = 78;
pub const TREBLE_FULL: usize = 84;
pub const TREBLE_PARTIALS: [f64; 3] = [-21.0, -36.0, -48.0];

/// Share of the treble voicing in the tone of `midi` (0..1).
pub fn treble_weight(midi: usize) -> f64 {
    ((midi as f64 - TREBLE_LO as f64) / (TREBLE_FULL - TREBLE_LO) as f64).clamp(0.0, 1.0)
}

/// Apply the treble voicing to the target levels [dB] of the partials of
/// `midi` at the two microphones (`target[0]`: the fundamental).
pub fn voice_treble(midi: usize, target: &mut [Option<[f64; 2]>]) {
    let w = treble_weight(midi);
    let Some(p1) = target.first().copied().flatten() else { return };
    if w == 0.0 {
        return;
    }
    let p1 = 10.0 * (0.5 * (10f64.powf(p1[0] / 10.0) + 10f64.powf(p1[1] / 10.0))).log10();
    for (t, off) in target.iter_mut().skip(1).zip(TREBLE_PARTIALS) {
        if let Some(t) = t {
            for c in t.iter_mut() {
                *c = (1.0 - w) * *c + w * (p1 + off);
            }
        }
    }
}

/// True for the keys that get a correction of their partial levels: all
/// but the reference keys below the treble voicing.
pub fn is_corrected(midi: usize) -> bool {
    !is_anchor(midi) || treble_weight(midi) > 0.0
}

pub fn is_anchor(midi: usize) -> bool {
    ANCHORS.contains(&midi)
}

/// How a key is made of two reference keys: `a + t (b - a)`, with `a`
/// below `b`. `t` lies in 0..1 between them and outside where a trend is
/// continued (`t < 0` below the lowest reference key, `t > 1` above the
/// highest). None for the reference keys themselves.
pub fn blend(midi: usize) -> Option<(usize, usize, f64)> {
    if is_anchor(midi) {
        return None;
    }
    let n = ANCHORS.len() - 1;
    let i = ANCHORS[..n].iter().position(|&a| a > midi).unwrap_or(n);
    let (a, b) = if i == 0 {
        (ANCHORS[0], ANCHORS[1])
    } else if i == n {
        (ANCHORS[n - 2], ANCHORS[n - 1])
    } else {
        (ANCHORS[i - 1], ANCHORS[i])
    };
    Some((a, b, (midi as f64 - a as f64) / (b as f64 - a as f64)))
}

/// Blend of two values given in dB (or any other quantity that is blended
/// linearly). Between the reference keys: interpolation. Outside: the
/// trend, at most `max_trend` per key.
pub fn mix(midi: usize, (a, b, t): (usize, usize, f64), va: f64, vb: f64, max_trend: f64) -> f64 {
    if (0.0..=1.0).contains(&t) {
        return va + t * (vb - va);
    }
    let slope = ((vb - va) / (b as f64 - a as f64)).clamp(-max_trend, max_trend);
    let near = if t < 0.0 { (a, va) } else { (b, vb) };
    near.1 + slope * (midi as f64 - near.0 as f64)
}

/// Frequencies of the partials of a key that are corrected.
pub fn partials(k: &KeyParams) -> Vec<f64> {
    (1..=MAX_PARTIALS).map(|n| partial_freq(k.f1, k.b_inh, n)).take_while(|&f| f < MAX_HZ).collect()
}

/// Bandwidth of the resonators of a key [Hz]: narrow against the distance
/// between partials, yet wide enough to follow the attack.
pub fn bandwidth(k: &KeyParams) -> f64 {
    (0.2 * k.f1).max(10.0)
}

/// One partial: a complex one-pole resonator with unit gain at its centre.
#[derive(Clone)]
pub struct Res {
    pub pr: f64,
    pub pi: f64,
    pub k: f64,
    pub sr: f64,
    pub si: f64,
    /// Gains on (Re s, Im s) for the left and for the right microphone.
    pub g: [f64; 4],
}

impl Res {
    pub fn new(fs: f64, f: f64, bw: f64, g: [f64; 4]) -> Res {
        let r = (-PI * bw / fs).exp();
        let th = 2.0 * PI * f / fs;
        Res { pr: r * th.cos(), pi: r * th.sin(), k: 1.0 - r, sr: 0.0, si: 0.0, g }
    }

    #[inline]
    pub fn step(&mut self, x: f64) {
        let sr = self.pr * self.sr - self.pi * self.si + self.k * x;
        self.si = self.pi * self.sr + self.pr * self.si;
        self.sr = sr;
    }

    /// Transfer functions x -> Re s and x -> Im s at normalised frequency `w`.
    pub fn response(&self, w: f64) -> ((f64, f64), (f64, f64)) {
        let (c, s) = (w.cos(), w.sin());
        let inv = |pr: f64, pi: f64| {
            // k / (1 - (pr + j pi)(c - j s))
            let (dr, di) = (1.0 - pr * c - pi * s, pr * s - pi * c);
            let d = dr * dr + di * di;
            (self.k * dr / d, -self.k * di / d)
        };
        let (a, b) = (inv(self.pr, self.pi), inv(self.pr, -self.pi));
        // Re s = (s + conj s) / 2, Im s = (s - conj s) / 2j
        ((0.5 * (a.0 + b.0), 0.5 * (a.1 + b.1)), (0.5 * (a.1 - b.1), -0.5 * (a.0 - b.0)))
    }
}

/// Per-partial gains of `midi` from the table measured at the sample rate
/// closest to `fs`: one `[f64; 4]` per partial (empty for the reference
/// keys below the treble voicing).
pub fn gains(midi: usize, fs: f64) -> Vec<[f64; 4]> {
    let u = |p: usize| u32::from_le_bytes([TABLE[p], TABLE[p + 1], TABLE[p + 2], TABLE[p + 3]]) as usize;
    let f = |p: usize| f32::from_le_bytes([TABLE[p], TABLE[p + 1], TABLE[p + 2], TABLE[p + 3]]) as f64;
    if TABLE.len() < 4 {
        return vec![];
    }
    // per rate: the rate, then per key the number of partials and their gains
    let mut best: Option<(f64, Vec<[f64; 4]>)> = None;
    let mut p = 4;
    for _ in 0..u(0) {
        if p + 4 > TABLE.len() {
            break;
        }
        let d = (f(p) / fs).ln().abs();
        p += 4;
        let mut found = vec![];
        for key in MIDI_LO..MIDI_LO + NKEYS {
            if p + 4 > TABLE.len() {
                return vec![];
            }
            let n = u(p);
            p += 4;
            if p + 16 * n > TABLE.len() {
                return vec![];
            }
            if key == midi {
                found = (0..n).map(|i| std::array::from_fn(|c| f(p + 16 * i + 4 * c))).collect();
            }
            p += 16 * n;
        }
        if best.as_ref().map(|b| d < b.0).unwrap_or(true) {
            best = Some((d, found));
        }
    }
    best.map(|b| b.1).unwrap_or_default()
}

/// The resonator bank of a key with the given gains.
pub fn bank(fs: f64, k: &KeyParams, gains: &[[f64; 4]]) -> Vec<Res> {
    let bw = bandwidth(k);
    partials(k).iter().zip(gains).filter(|(&f, _)| f < 0.4 * fs).map(|(&f, &g)| Res::new(fs, f, bw, g)).collect()
}

/// Four resonators.
#[derive(Clone, Copy)]
#[repr(C, align(16))]
struct MBlk {
    pr: F4,
    pi: F4,
    k: F4,
    sr: F4,
    si: F4,
    g: [F4; 4],
}

/// The resonators of a key as they run in the audio loop (single
/// precision, four at a time).
#[derive(Default)]
pub struct MorphBank {
    blocks: Vec<MBlk>,
}

impl MorphBank {
    pub fn new(res: &[Res]) -> MorphBank {
        let z = F4::zero();
        let mut blocks = vec![MBlk { pr: z, pi: z, k: z, sr: z, si: z, g: [z; 4] }; res.len().div_ceil(4)];
        for (i, r) in res.iter().enumerate() {
            let (b, j) = (&mut blocks[i / 4], i % 4);
            b.pr.set(j, r.pr as f32);
            b.pi.set(j, r.pi as f32);
            b.k.set(j, r.k as f32);
            for c in 0..4 {
                b.g[c].set(j, r.g[c] as f32);
            }
        }
        MorphBank { blocks }
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn clear(&mut self) {
        for b in self.blocks.iter_mut() {
            b.sr = F4::zero();
            b.si = F4::zero();
        }
    }

    /// Run the resonators over the bridge waves `x` and add what they give
    /// the two microphones, times `gain`, to `ml` / `mr`.
    pub fn process(&mut self, x: &[f32], gain: f32, ml: &mut [f32], mr: &mut [f32]) {
        for ((&x, ml), mr) in x.iter().zip(ml.iter_mut()).zip(mr.iter_mut()) {
            let xv = F4::splat(x);
            let (mut l, mut r) = (F4::zero(), F4::zero());
            for b in self.blocks.iter_mut() {
                let sr = b.pr * b.sr - b.pi * b.si + b.k * xv;
                let si = b.pi * b.sr + b.pr * b.si;
                b.sr = sr;
                b.si = si;
                l += b.g[0] * sr + b.g[1] * si;
                r += b.g[2] * sr + b.g[3] * si;
            }
            *ml += gain * l.sum();
            *mr += gain * r.sum();
        }
    }
}
