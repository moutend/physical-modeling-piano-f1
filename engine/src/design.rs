//! Per-key physical parameters and filter design.
//!
//! Everything here runs once at start-up; nothing in this module is called from
//! the audio loop.

use std::f64::consts::PI;
use std::sync::atomic::{AtomicU64, Ordering};

/// Reference pitch of A4 [Hz] the keys are tuned to (bits of an f64).
static A4_HZ: AtomicU64 = AtomicU64::new(0x407B_8000_0000_0000); // 440.0

/// Range of the reference pitch [Hz]; the tables of the model were measured
/// at 440 Hz and are used as they are within it.
pub const A4_MIN: f64 = 400.0;
pub const A4_MAX: f64 = 480.0;

pub fn reference_pitch() -> f64 {
    f64::from_bits(A4_HZ.load(Ordering::Relaxed))
}

/// Applies to every `key_params` call from now on (see
/// `Engine::set_reference_pitch` for an engine that is already built).
pub fn set_reference_pitch(hz: f64) {
    if hz.is_finite() {
        A4_HZ.store(hz.clamp(A4_MIN, A4_MAX).to_bits(), Ordering::Relaxed);
    }
}

pub const MIDI_LO: usize = 21;
pub const NKEYS: usize = 88;
pub const MAX_AP: usize = 16;
/// Budget of second-order dispersion sections per string (bass keys).
pub const MAX_BQ: usize = 20;
pub const NZONES: usize = 6;
pub const MAX_STRINGS: usize = 3;
/// Budget of dispersion sections of strings that only ring along (dampers
/// up, not struck): first-order / second-order.
///
/// Such a string is driven by the bridge at the partials of the keys that
/// are played, and answers where its own partials lie close to them. With
/// this budget its partials stay within a few cents of where they belong
/// up to about the 24th to 32nd (the full design of a bass string: up to
/// 10 kHz with up to 20 sections); above that, where the string hardly
/// answers, they do not. How loud and how long the strings ring along is
/// unchanged (on a piece played with the pedal, the third-octave band
/// levels stay within 0.2 dB); which partial of which string answers a
/// given note differs in detail. A struck key always has the full design.
pub const LIGHT_AP: usize = 8;
pub const LIGHT_BQ: usize = 8;
/// First key without a damper (its strings always ring along); past the
/// last key: none.
pub const UNDAMPED_LO: usize = MIDI_LO + NKEYS;

/// Piecewise interpolation over `(x, y)` anchors (linear, or linear in log(y)).
pub fn interp(x: f64, pts: &[(f64, f64)], log: bool) -> f64 {
    if x <= pts[0].0 {
        return pts[0].1;
    }
    for w in pts.windows(2) {
        if x <= w[1].0 {
            let t = (x - w[0].0) / (w[1].0 - w[0].0);
            return if log {
                (w[0].1.ln() + t * (w[1].1.ln() - w[0].1.ln())).exp()
            } else {
                w[0].1 + t * (w[1].1 - w[0].1)
            };
        }
    }
    pts[pts.len() - 1].1
}

/// Deterministic pseudo random number in [0, 1).
pub fn hash01(seed: u32) -> f64 {
    let mut x = seed.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x as f64) / 4294967296.0
}

/// Where the voicing below ends (C6): from here up both multipliers are 1.
pub const BRILLIANCE_HI: usize = 84;
/// Voicing of the keys below C6 for a more brilliant, longer singing tone:
/// multipliers on the felt stiffness of the hammer and on the frequency
/// dependent loss of the strings (`b3`), over the key number.
///
/// With the values fitted before, partials 7 and up of keys 49..84 were 5
/// to 20 dB weaker shortly after the strike than on the reference
/// recording and died away two to four times faster (C4 at velocity 0.6:
/// partials at 2.4..3.8 kHz at -40..-52 dB, falling by 27..48 dB/s). The
/// soundboard carries that range well; it was missing in the strings. A
/// harder felt puts it into the blow (C4: +8..+13 dB at 2.4..3.8 kHz, up to
/// +18 dB at 5 kHz, contact 2.35 -> 2.15 ms), less loss lets it ring (C4:
/// 16..30 dB/s at 3..4 kHz).
///
/// Above the bass register the felt follows how bright the instrument was
/// before the fit to the recording (2..8 kHz of the first 0.3 s of a note,
/// medians over half octaves): somewhat harder around C4, softer from C5
/// up, where the notes are struck faster since they ring longer and
/// radiate less directly (`direct_share`), which brightens them by itself.
pub const BRILLIANCE_FELT: [(f64, f64); 9] =
    [(21.0, 1.5), (36.0, 2.0), (48.0, 3.0), (54.0, 4.0), (60.0, 3.8), (66.0, 3.0), (72.0, 1.5), (78.0, 0.75), (BRILLIANCE_HI as f64, 1.0)];
pub const BRILLIANCE_LOSS: [(f64, f64); 4] = [(21.0, 0.6), (48.0, 0.45), (72.0, 0.45), (BRILLIANCE_HI as f64, 1.0)];

/// Highest key of the bass register (C3). Up to here the instrument was
/// judged right by ear as it is: the model of these keys and their entries
/// in the generated tables are left alone (the generators write the
/// entries of the tables in use back for them).
pub const BASS_HI: usize = 48;

/// Share of the direct radiation of the strings (`voicing.rs`) that a key
/// takes: all of it up to `BASS_HI` and from C6 up, `DIRECT_MID` from G3
/// to C5.
///
/// The direct path is a low-passed, nearly mono copy of the bridge wave.
/// At the gains that fill in a weak fundamental it is 6 to 9 dB louder
/// than what the soundboard radiates of the same note, and then the note
/// is mostly that copy: little above its third partial, the same in both
/// microphones (side 5 to 10 dB below mid, where the board alone gives 0).
/// In the middle of the keyboard the instrument without it was judged the
/// more brilliant one; a remainder is kept, so that a note whose
/// fundamental the board carries badly is not left thin. The treble keeps
/// all of it: its notes are little more than their fundamental, and what
/// the board makes of that differs too much from key to key.
pub const DIRECT_MID: f64 = 0.3;
pub fn direct_share(midi: usize) -> f64 {
    interp(midi as f64, &[(BASS_HI as f64, 1.0), (55.0, DIRECT_MID), (72.0, DIRECT_MID), (84.0, 1.0)], false)
}

#[derive(Clone, Debug)]
pub struct KeyParams {
    pub midi: usize,
    /// Frequency of the first partial (includes stretch tuning) [Hz].
    pub f1: f64,
    /// Inharmonicity coefficient B: f_n = n f0 sqrt(1 + B n^2).
    pub b_inh: f64,
    /// Number of waveguides (strings, or string + 2nd polarisation for mono-strung keys).
    pub n_wg: usize,
    /// Number of waveguides struck directly by the hammer.
    pub n_struck: usize,
    /// Per-waveguide detuning [cents].
    pub detune: [f64; MAX_STRINGS],
    /// Per-waveguide bridge coupling weight.
    pub coupling: [f64; MAX_STRINGS],
    /// Per-waveguide hammer stiffness factor (felt irregularity).
    pub strike: [f64; MAX_STRINGS],
    /// Fraction of the hammer force leaking into the non-struck polarisation.
    pub pol_leak: f64,
    /// Characteristic impedance of one string [kg/s].
    pub z: f64,
    pub hammer_mass: f64,
    pub hammer_k: f64,
    pub hammer_p: f64,
    /// Striking point as a fraction of the string length.
    pub strike_ratio: f64,
    /// Decay rate of the in-phase (bridge-coupled) mode [dB/s].
    pub rate_prompt: f64,
    /// Decay rate of the uncoupled modes (internal losses) [dB/s].
    pub rate_after: f64,
    /// Frequency dependent loss coefficient: sigma(f) = b1 + b3 (2 pi f)^2.
    pub b3: f64,
    /// T60 once the damper rests on the strings [s]; 0 = key has no damper.
    pub damper_t60: f64,
    /// Position along the bridge, 0 (bass) .. 1 (treble).
    pub bridge_pos: f64,
    /// Relative deviation of each waveguide's inharmonicity coefficient
    /// (string irregularities): breaks the regular beat pattern of a unison.
    pub b_var: [f64; MAX_STRINGS],
    /// Speaking length [m].
    pub length: f64,
    /// Longitudinal stiffness E A of the string (core) [N].
    pub ea: f64,
    /// Frequency of the first longitudinal mode [Hz].
    pub f_long: f64,
}

pub fn key_params(midi: usize) -> KeyParams {
    let m = midi as f64;
    // Railsback-like stretch (cents): a cubic, plus the deviation from it
    // measured on the reference instrument (flatter bass, sharper treble).
    let stretch = 2.5e-4 * (m - 66.0).powi(3)
        + interp(m, &[(21.0, -5.0), (48.0, -5.0), (54.0, -2.5), (60.0, 0.0), (69.0, 0.0), (75.0, 2.0), (81.0, 3.0), (90.0, 2.0), (108.0, 0.0)], false);
    let f1 = reference_pitch() * 2f64.powf((m - 69.0) / 12.0 + stretch / 1200.0);

    // Wound bass strings + plain steel strings, fitted to the partials of
    // the reference instrument (1.0e-4 at C2, 2.6e-4 at C4, 1.6e-3 at A5).
    let b_inh = (-8.0 - 0.11 * (m - 21.0)).exp() + (-9.3 + 0.087 * (m - 48.0)).exp();

    let length = interp(m, &[(21.0, 2.0), (36.0, 1.9), (60.0, 0.62), (96.0, 0.09), (108.0, 0.05)], true);
    let tension = interp(m, &[(21.0, 1100.0), (36.0, 850.0), (60.0, 680.0), (96.0, 750.0), (108.0, 750.0)], true);
    let z = tension / (2.0 * length * f1);

    // Mass of the hammer head. Chaigne & Askenfelt model one string of the
    // unison with (head mass / number of strings); `Voice` shares the whole
    // head between the strings it strikes.
    //
    // Above C6 the head gets light quickly, and the felt of E6..D7 is softer
    // than the trend of the keys below. With the heavier, harder hammers of
    // that trend the blow lasts 1.3 to 2 periods of the note: the
    // fundamental is partly cancelled (by 10 dB on single keys, A7 worst)
    // while partials 2..4 come out 10 to 20 dB stronger than on the
    // reference instrument, whose notes above E6 are nearly pure tones. The
    // weak notes then sound like the thump of the blow plus a thin, glassy
    // tone, and the levelling makes that worse by asking for a much faster
    // hammer. (Fitted to the onset spectra of the reference recording, keys
    // 86..98; the keys up to D6 already matched.)
    let hammer_mass =
        interp(m, &[(21.0, 0.0105), (36.0, 0.0100), (60.0, 0.0090), (84.0, 0.00747), (90.0, 0.0043), (96.0, 0.0038), (108.0, 0.0028)], true);
    let hammer_k = interp(
        m,
        &[(21.0, 3.2e7), (36.0, 6.4e7), (48.0, 2.7e8), (60.0, 0.9e9), (72.0, 6.0e9), (84.0, 4.0e10), (90.0, 2.0e10), (98.0, 7.4e10), (108.0, 2.0e12)],
        true,
    );
    // (the fit above, voiced harder below C6)
    let hammer_k = hammer_k * interp(m, &BRILLIANCE_FELT, true);
    let hammer_p = interp(m, &[(21.0, 2.2), (36.0, 2.3), (60.0, 2.5), (96.0, 3.0), (108.0, 3.0)], false);
    let strike_ratio = interp(m, &[(21.0, 0.125), (60.0, 0.12), (84.0, 0.10), (108.0, 0.07)], false);

    // Rate at which the unison loses energy to the board [dB/s]. Above the
    // bass register the notes ring about 1.5 times as long as the values
    // fitted to the reference recording gave (19 at C4, 25 at C5, 40 at
    // C6, 65 at C7, 100 at C8): the longer, singing decay was judged the
    // more beautiful one. (`sustain.rs` evens out what each key makes of it.)
    let rate_prompt = interp(
        m,
        &[(21.0, 4.0), (36.0, 7.0), (48.0, 12.0), (60.0, 12.7), (72.0, 16.7), (84.0, 26.7), (96.0, 43.0), (108.0, 67.0)],
        true,
    );
    let rate_after = 0.28 * rate_prompt;
    let b3 = interp(m, &[(21.0, 4.0e-9), (48.0, 7.0e-9), (72.0, 5.0e-9), (96.0, 2.5e-9), (108.0, 1.5e-9)], true);
    let b3 = b3 * interp(m, &BRILLIANCE_LOSS, true);
    // Every key has a damper. A piano has none above about G6 and lets
    // those strings ring; played from a computer keyboard that reads as
    // keys that ignore their release, so here a short key press is a short
    // note all the way up.
    let damper_t60 = if midi >= UNDAMPED_LO {
        0.0
    } else {
        interp(m, &[(21.0, 0.85), (48.0, 0.42), (72.0, 0.26), (91.0, 0.19), (108.0, 0.15)], true)
    };

    // Longitudinal properties. Wound strings: only the steel core carries the
    // tension change, the copper winding only adds mass.
    let mu = z / (2.0 * length * f1);
    let core = interp(m, &[(21.0, 0.22), (33.0, 0.35), (44.0, 0.6), (46.0, 1.0)], false);
    let ea = 2.0e11 * mu * core / 7850.0;
    let f_long = 5050.0 * core.sqrt() / (2.0 * length);

    let seed = (midi as u32) * 16;
    // Unison mistuning, chosen relative to the bridge coupling rate: below
    // sigma_c / (2 pi) the string modes lock (double decay, no beats), above
    // it they beat slowly (Weinreich 1977). Real unisons sit around that limit.
    let sigma_c = (rate_prompt - rate_after) / 8.686;
    let df = sigma_c / (2.0 * PI) * (0.25 + 0.6 * hash01(seed + 1));
    let d = 0.5 * 1731.234 * df / f1; // half spread [cents]
    let (n_wg, n_struck, detune, coupling, strike, pol_leak);
    if midi <= 32 {
        // One wound string: vertical + horizontal polarisation.
        n_wg = 2;
        n_struck = 1;
        detune = [0.0, 0.4 + 0.6 * hash01(seed + 2), 0.0];
        coupling = [1.0, 0.22, 0.0];
        strike = [1.0, 0.0, 0.0];
        pol_leak = 0.14;
    } else if midi <= 44 {
        n_wg = 2;
        n_struck = 2;
        detune = [-d, d, 0.0];
        coupling = [1.0, 1.0, 0.0];
        strike = [1.0, 0.92 + 0.12 * hash01(seed + 3), 0.0];
        pol_leak = 0.0;
    } else {
        n_wg = 3;
        n_struck = 3;
        detune = [-d, 0.25 * (hash01(seed + 4) - 0.5), d * (0.6 + 0.6 * hash01(seed + 5))];
        coupling = [1.0, 1.0, 1.0];
        strike = [1.0, 0.92 + 0.12 * hash01(seed + 3), 0.94 + 0.12 * hash01(seed + 6)];
        pol_leak = 0.0;
    }

    KeyParams {
        midi,
        f1,
        b_inh,
        n_wg,
        n_struck,
        detune,
        coupling,
        strike,
        pol_leak,
        z,
        hammer_mass,
        hammer_k,
        hammer_p,
        strike_ratio,
        rate_prompt,
        rate_after,
        b3,
        damper_t60,
        bridge_pos: (m - 21.0) / 87.0,
        b_var: [0.0, 0.012 + 0.03 * hash01(seed + 7), -0.012 - 0.03 * hash01(seed + 8)],
        length,
        ea,
        f_long,
    }
}

/// Where a key's strings meet the bridge: (zone, fraction towards zone + 1).
/// Zones 0-1 are the ends of the bass bridge, zones 2-5 lie along the long bridge.
pub fn bridge_zone(midi: usize) -> (usize, f64) {
    if midi <= BASS_BRIDGE_HI {
        (0, (midi - MIDI_LO) as f64 / (BASS_BRIDGE_HI - MIDI_LO) as f64)
    } else {
        let t = 3.0 * (midi - BASS_BRIDGE_HI - 1) as f64 / (MIDI_LO + NKEYS - 1 - BASS_BRIDGE_HI - 1) as f64;
        let z = (t.floor() as usize).min(2);
        (2 + z, t - z as f64)
    }
}

/// Highest key on the bass bridge.
pub const BASS_BRIDGE_HI: usize = 46;

/// MIDI-style velocity (0..1) to hammer velocity [m/s].
pub fn hammer_velocity(vel: f64) -> f64 {
    0.1 + 5.5 * vel.clamp(0.0, 1.0).powf(1.6)
}

// ---------------------------------------------------------------------------
// Filter phase helpers (phase *lag* in radians at normalised frequency w).

/// First-order allpass H(z) = (a + z^-1) / (1 + a z^-1).
#[inline]
pub fn ap_phase(a: f64, w: f64) -> f64 {
    w - 2.0 * (a * w.sin()).atan2(1.0 + a * w.cos())
}

/// One-pole lowpass H(z) = g (1 - p) / (1 - p z^-1).
#[inline]
pub fn loss_phase(p: f64, w: f64) -> f64 {
    (p * w.sin()).atan2(1.0 - p * w.cos())
}

/// One-pole loss filter matching sigma(f) = b1 + b3 (2 pi f)^2 per round trip.
/// Returns (g, p).
pub fn design_loss(fs: f64, f1: f64, b1: f64, b3: f64) -> (f64, f64) {
    let g = (-b1 / f1).exp();
    let q = b3 * fs * fs / f1;
    let p = if q < 1e-9 { 0.0 } else { ((4.0 * q + 1.0) - (8.0 * q + 1.0).sqrt()) / (4.0 * q) };
    (g, p.clamp(0.0, 0.95))
}

/// Dispersion (string stiffness) filter: a cascade of `m_ap` identical
/// first-order allpasses and/or second-order allpass sections.
#[derive(Clone, Debug, Default)]
pub struct Dispersion {
    pub m_ap: usize,
    pub a_ap: f64,
    /// Second-order sections as pole (radius, angle).
    pub poles: Vec<(f64, f64)>,
    /// (fc, beta) of the Abel-Smith design, if that is what `poles` holds.
    pub design: Option<(f64, f64)>,
}

impl Dispersion {
    pub fn none() -> Dispersion {
        Dispersion::default()
    }

    /// Phase lag at normalised frequency w.
    pub fn phase(&self, w: f64) -> f64 {
        let mut phi = self.m_ap as f64 * ap_phase(self.a_ap, w);
        for &(rho, th) in &self.poles {
            phi += 2.0 * w
                + 2.0 * (rho * (w - th).sin()).atan2(1.0 - rho * (w - th).cos())
                + 2.0 * (rho * (w + th).sin()).atan2(1.0 - rho * (w + th).cos());
        }
        phi
    }

    /// Relative cost (first-order sections = 1, biquads = 2.5).
    pub fn cost(&self) -> f64 {
        self.m_ap as f64 + 2.5 * self.poles.len() as f64
    }
}

#[derive(Clone, Debug, Default)]
pub struct LoopDesign {
    /// Round-trip delay hammer -> agraffe -> hammer [samples].
    pub nl: usize,
    /// Round-trip delay hammer -> bridge -> hammer (integer part) [samples].
    pub nr: usize,
    /// Fractional delay (tuning) allpass coefficient.
    pub eta: f64,
}

pub fn partial_freq(f1: f64, b: f64, n: usize) -> f64 {
    let f0 = f1 / (1.0 + b).sqrt();
    let nn = n as f64;
    nn * f0 * (1.0 + b * nn * nn).sqrt()
}

/// Solve the tuning for a given dispersion filter. Returns None if the loop
/// is too short to host the filters.
pub fn tune_loop(fs: f64, f1: f64, loss_p: f64, disp: &Dispersion, strike_ratio: f64) -> Option<LoopDesign> {
    let w1 = 2.0 * PI * f1 / fs;
    let d1 = (2.0 * PI - disp.phase(w1) - loss_phase(loss_p, w1)) / w1;
    let d_int = (d1 - 0.5).floor();
    let d = d1 - d_int;
    if d_int < 2.0 {
        return None;
    }
    let t = (w1 * (1.0 - d) / 2.0).tan();
    let eta = t / (w1.sin() - t * w1.cos());
    let d_int = d_int as usize;
    let nl = ((strike_ratio * fs / f1).round() as usize).clamp(1, d_int - 1);
    Some(LoopDesign { nl, nr: d_int - nl, eta })
}

/// Partial frequency errors [cents] of a loop design for partials 2..=k.
pub fn partial_errors(fs: f64, f1: f64, b: f64, loss_p: f64, disp: &Dispersion, k: usize) -> Option<Vec<f64>> {
    let des = tune_loop(fs, f1, loss_p, disp, 0.12)?;
    let d_int = (des.nl + des.nr) as f64;
    let mut out = Vec::with_capacity(k);
    for n in 2..=k {
        let w = 2.0 * PI * partial_freq(f1, b, n) / fs;
        let phi = w * d_int + ap_phase(des.eta, w) + disp.phase(w) + loss_phase(loss_p, w);
        let target = 2.0 * PI * n as f64;
        out.push(1731.234 * (phi - target) / target);
    }
    Some(out)
}

/// Partials above `FIT_FW` count progressively less in the fit.
pub const FIT_FW: f64 = 3000.0;

pub fn fit_error(fs: f64, f1: f64, b: f64, loss_p: f64, disp: &Dispersion, k: usize) -> f64 {
    let Some(errs) = partial_errors(fs, f1, b, loss_p, disp, k) else {
        return f64::INFINITY;
    };
    let (mut acc, mut wsum) = (0.0, 0.0);
    for (i, e) in errs.iter().enumerate() {
        let f = partial_freq(f1, b, i + 2);
        let w = 1.0 / (1.0 + (f / FIT_FW).powi(4));
        acc += w * e * e;
        wsum += w;
    }
    (acc / wsum).sqrt()
}

/// Number of partials considered by the fit.
pub fn fit_partials(fs: f64, f1: f64, b: f64) -> usize {
    let fmax = (0.45 * fs).min(12000.0);
    let mut k = 1;
    while k < 60 && partial_freq(f1, b, k + 1) < fmax {
        k += 1;
    }
    k
}

/// Cascades of M identical first-order allpasses (cheap; good for
/// mid/treble): the best one for each `max_ap` in `budgets`.
fn design_first_order(fs: f64, f1: f64, b: f64, loss_p: f64, budgets: &[usize], k: usize) -> Vec<Dispersion> {
    let cands = [1usize, 2, 3, 4, 6, 8, 10, 12, 14, 16];
    let most = budgets.iter().copied().max().unwrap_or(0);
    let mut results: Vec<(Dispersion, f64)> = vec![(Dispersion::none(), fit_error(fs, f1, b, loss_p, &Dispersion::none(), k))];
    for &m in cands.iter().filter(|&&m| m <= most) {
        let mut d = Dispersion { m_ap: m, a_ap: 0.0, poles: vec![], design: None };
        let mut best = (0.0, f64::INFINITY);
        for i in 0..98 {
            d.a_ap = -0.01 * i as f64;
            let e = fit_error(fs, f1, b, loss_p, &d, k);
            if e < best.1 {
                best = (d.a_ap, e);
            }
        }
        let centre = best.0;
        for i in 0..41 {
            d.a_ap = (centre - 0.01 + 0.0005 * i as f64).min(0.0);
            let e = fit_error(fs, f1, b, loss_p, &d, k);
            if e < best.1 {
                best = (d.a_ap, e);
            }
        }
        d.a_ap = best.0;
        results.push((d, best.1));
    }
    budgets
        .iter()
        .map(|&max_ap| {
            let within = || results.iter().filter(|r| r.0.m_ap <= max_ap);
            let min_err = within().map(|r| r.1).fold(f64::INFINITY, f64::min);
            let tol = (1.3 * min_err).max(1.0);
            within().find(|r| r.1 <= tol).unwrap().0.clone()
        })
        .collect()
}

/// Abel & Smith style design: the desired group delay is cut into bands of
/// 2 pi phase each, one pole pair per band. Accurate up to `fc`, constant
/// group delay above.
pub fn design_abel_smith(fs: f64, f1: f64, b: f64, fc: f64, beta: f64) -> Option<Dispersion> {
    let f0 = f1 / (1.0 + b).sqrt();
    // mode number (continuous) and its derivative w.r.t. x = f / f0
    let n_of = |x: f64| ((-1.0 + (1.0 + 4.0 * b * x * x).sqrt()) / (2.0 * b)).sqrt();
    let wc = 2.0 * PI * fc / fs;
    let xc = fc / f0;
    let nc = n_of(xc);
    let tau_c = (fs / f0) * xc / (nc * (1.0 + 4.0 * b * xc * xc).sqrt());
    let n_min = nc - tau_c * wc / (2.0 * PI);
    let n_sec = (n_min + 0.6).ceil().max(1.0);
    let d0 = (2.0 * PI * nc + tau_c * (PI - wc) - 2.0 * PI * n_sec) / PI;
    if d0 < 3.0 || tau_c - d0 <= 0.0 {
        return None;
    }
    let phi = |w: f64| {
        if w <= wc {
            2.0 * PI * n_of(w * fs / (2.0 * PI) / f0) - d0 * w
        } else {
            2.0 * PI * nc - d0 * wc + (tau_c - d0) * (w - wc)
        }
    };
    let solve = |target: f64| {
        let (mut lo, mut hi) = (0.0, PI);
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            if phi(mid) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    };
    let mut poles = vec![];
    let n_sec = n_sec as usize;
    let mut prev = 0.0;
    for k in 1..=n_sec {
        let edge = if k == n_sec { PI } else { solve(2.0 * PI * k as f64) };
        let centre = solve(2.0 * PI * (k as f64 - 0.5));
        let delta = 0.5 * (edge - prev);
        let eta = (1.0 - beta * delta.cos()) / (1.0 - beta);
        let rho = eta - (eta * eta - 1.0).sqrt();
        poles.push((rho, centre));
        prev = edge;
    }
    Some(Dispersion { m_ap: 0, a_ap: 0.0, poles, design: Some((fc, beta)) })
}

/// Choose the dispersion filter that best reproduces
/// f_n = n f0 sqrt(1 + B n^2) within the budget (`max_ap` first-order
/// sections, `max_bq` second-order sections).
pub fn design_dispersion(fs: f64, f1: f64, b: f64, loss_p: f64, max_ap: usize, max_bq: usize) -> (Dispersion, f64, usize) {
    let (mut d, k) = design_dispersions(fs, f1, b, loss_p, &[(max_ap, max_bq)]);
    let (disp, err) = d.remove(0);
    (disp, err, k)
}

/// `design_dispersion` for several budgets `(max_ap, max_bq)` at once (the
/// candidates are only evaluated once): (filter, fit error) per budget, and
/// the number of partials considered.
pub fn design_dispersions(fs: f64, f1: f64, b: f64, loss_p: f64, budgets: &[(usize, usize)]) -> (Vec<(Dispersion, f64)>, usize) {
    let k = fit_partials(fs, f1, b);
    if k < 2 {
        return (budgets.iter().map(|_| (Dispersion::none(), 0.0)).collect(), k);
    }
    let aps: Vec<usize> = budgets.iter().map(|b| b.0).collect();
    let mut best: Vec<(Dispersion, f64)> = design_first_order(fs, f1, b, loss_p, &aps, k)
        .into_iter()
        .map(|d| {
            let e = fit_error(fs, f1, b, loss_p, &d, k);
            (d, e)
        })
        .collect();
    let wanted: Vec<bool> = budgets.iter().zip(&best).map(|(bud, be)| bud.1 > 0 && be.1 > 1.5).collect();
    if wanted.iter().any(|&w| w) {
        // Largest accurate band that fits the section budget.
        let mut fc = 10000.0f64.min(0.4 * fs);
        while fc > 500.0 {
            for &beta in &[0.75, 0.85, 0.9] {
                if let Some(d) = design_abel_smith(fs, f1, b, fc, beta) {
                    let fits = |i: usize| wanted[i] && d.poles.len() <= budgets[i].1;
                    if (0..budgets.len()).any(fits) {
                        let e = fit_error(fs, f1, b, loss_p, &d, k);
                        for i in 0..budgets.len() {
                            if fits(i) && e < 0.8 * best[i].1 {
                                best[i] = (d.clone(), e);
                            }
                        }
                    }
                }
            }
            fc *= 0.92;
        }
    }
    (best, k)
}

/// Variant of a key's dispersion filter for a string whose inharmonicity
/// coefficient deviates by the relative amount `var`.
pub fn vary_dispersion(fs: f64, f1: f64, b: f64, base: &Dispersion, var: f64) -> Dispersion {
    if var == 0.0 {
        return base.clone();
    }
    if let Some((fc, beta)) = base.design {
        if let Some(d) = design_abel_smith(fs, f1, b * (1.0 + var), fc, beta) {
            if d.poles.len() == base.poles.len() {
                return d;
            }
        }
        return base.clone();
    }
    // First-order chain: the dispersion scales with the low-frequency delay
    // (1 + |a|) / (1 - |a|) of each section.
    let mut d = base.clone();
    let a = -d.a_ap;
    let k = (1.0 + a) / (1.0 - a) * (1.0 + var);
    d.a_ap = -((k - 1.0) / (k + 1.0)).clamp(0.0, 0.97);
    d
}
