//! One piano key: hammer + 2..3 coupled digital waveguide strings.
//!
//! This module designs the strings of a key and holds what is computed per
//! key in double precision (hammer contact, longitudinal force, the lever
//! on the bridge). The string loops themselves run in `bank.rs`, four keys
//! at a time.
//!
//! Each string is a pair of delay lines meeting at the striking point
//! (velocity waves):
//!
//! ```text
//!   agraffe  <--- line L (round trip, x -1) ---  [hammer]  --- line R (round trip) ---> bridge
//!                                                              dispersion allpasses
//!                                                              loss filter, tuning allpass
//! ```
//!
//! The strings of a key share a resistive bridge (Weinreich coupling), which
//! produces the double decay and beating of real unisons.

use crate::bank::{Line, StringDesign, KPB, L};
use crate::design::*;
use crate::morph::MorphBank;
use crate::simd::F4;
use std::f64::consts::PI;

pub struct Hammer {
    pub active: bool,
    y: f64,
    v: f64,
    t: u32,
    mass: f64,
    k: f64,
    p: f64,
    /// diagnostics
    pub contact_samples: u32,
    pub peak_force: f64,
    /// u^(p-1) lookup table.
    pow_tab: Vec<f64>,
}

/// Longitudinal string motion driven by the transverse vibration
/// (tension modulation): the source of the "phantom" partials at sums of
/// transverse partial frequencies and of the longitudinal mode ring.
struct Longitudinal {
    on: bool,
    stride: usize,
    /// (E A / 2 L) * (dx / c^2) per squared wave sum.
    coef: f64,
    /// cos(pi x / L) at the quadrature points (left / right of the hammer).
    cl: Vec<f32>,
    cr: Vec<f32>,
    /// First longitudinal mode (complex one-pole), 0 if out of band.
    mode_on: bool,
    pr: f64,
    pi: f64,
    sr: f64,
    si: f64,
    mode_gain: f64,
    hp_c: f64,
    hp_s: f64,
    /// Peak of the force over the current observation window.
    peak: f64,
    count: u32,
    /// The force is evaluated every `every` samples.
    every: u32,
    phase: u32,
    held: f64,
}

pub struct Voice {
    pub key: KeyParams,
    /// The strings as designed (they run in the key's `Bank`): for the key
    /// when it is struck, and with the cheaper dispersion filter for when
    /// its strings only ring along (see `LIGHT_BQ`).
    pub strings: Vec<StringDesign>,
    pub light: Vec<StringDesign>,
    /// Which of the two the bank holds at the moment.
    pub is_light: bool,
    n_struck: usize,
    pol_leak: f64,
    pub z: f64,
    inv2z: f64,
    /// Z * sum(w_i^2): what the strings add to the bridge impedance.
    pub k_bridge: f64,
    /// Coupling of this key to the bridge zones (lever ratio x interpolation).
    pub zone: usize,
    pub sc: [f64; 2],
    /// Left / right gains of the direct radiation (bass slightly left).
    pub pan: [f64; 2],
    /// Coupling of the struck key; `sc` is `symp_lever` times this until the
    /// key is struck (see `SYMPATHETIC_LEVER`).
    sc_full: [f64; 2],
    symp_lever: f64,
    /// Current lever relative to `sc_full` (1 = struck key).
    lever: f64,
    lever_coef: f64,
    struck: bool,
    /// String energy (mean of the squared bridge waves over the last 64 or
    /// more samples): the latest value and the largest since the last strike.
    e_last: f64,
    e_peak: f64,
    pub hammer: Hammer,
    /// Hammer contact state per string.
    k_scale: [f64; MAX_STRINGS],
    ys: [f64; MAX_STRINGS],
    xi_prev: [f64; MAX_STRINGS],
    f_prev: [f64; MAX_STRINGS],
    long: Longitudinal,
    /// Round-trip gain of the damper as it is asked for (1: lifted).
    pub damp_target: f64,
    damp_on: f64,
    pub held: bool,
    pub active: bool,
    pub keep_alive: bool,
    dt: f64,
    hyst: f64,
    e_sum: f64,
    e_cnt: usize,
    /// Consecutive samples below the silence threshold.
    quiet: usize,
    quiet_limit: usize,
    /// Direct radiation of the strings' tone (see `voicing.rs`): gain and
    /// the coefficients of its band-pass (below half the fundamental the
    /// bridge wave holds the thump of the blow, far above it the string
    /// is brighter than what the soundboard radiates).
    pub direct_g: f64,
    pub direct_hp_c: f64,
    pub direct_lp_c: f64,
    /// The key's bridge wave `Z sum(w_i a_i)` of the latest sample.
    pub bridge: f64,
    /// Timbre correction of the key (see `morph.rs`): one resonator per
    /// partial. Runs while the key sounds its own note (`morph_fade` > 0).
    morph: MorphBank,
    pub morph_fade: f64,
}

/// `Voice::update_lever` is called once per this many samples.
pub const LEVER_STEP: usize = 32;
/// A struck key counts as sounding its own note until its string energy has
/// fallen to this fraction of the largest value since the strike (-40 dB).
const STRUCK_RANGE: f64 = 1.0e-4;

/// Longitudinal force [N] below which the longitudinal model is switched off.
const LONG_FLOOR: f64 = 2.0e-3;
/// Strings with at least this many quadrature points evaluate the force
/// every other time (see `Voice::new`).
const LONG_SLOW_POINTS: usize = 40;

/// Felt compression table: POW_N steps up to POW_MAX metres.
const POW_N: usize = 4096;
const POW_MAX: f64 = 0.004;
const POW_SCALE: f64 = POW_N as f64 / POW_MAX;

/// Solve F = k (xi0 - c F)^p for F >= 0 (implicit hammer/string contact).
///
/// g(F) = F - k (xi0 - c F)^p is increasing and concave on [0, xi0 / c), so a
/// Newton step from any point lands on the left of the root and the
/// iteration then converges monotonically: any warm start is safe.
#[inline]
fn solve_force(k: f64, p: f64, xi0: f64, c: f64, f_init: f64, tab: &[f64]) -> f64 {
    if xi0 <= 0.0 {
        return 0.0;
    }
    let f_max = xi0 / c;
    let mut f = if f_init > 0.0 && f_init < 0.999 * f_max { f_init } else { 0.0 };
    for _ in 0..12 {
        let u = xi0 - c * f;
        // u^(p-1) from the table (linear interpolation); powf is the
        // dominant cost of a chord onset otherwise.
        let x = u * POW_SCALE;
        let up = if x >= 0.0 && x < POW_N as f64 {
            let i = x as usize;
            tab[i] + (x - i as f64) * (tab[i + 1] - tab[i])
        } else {
            u.powf(p - 1.0)
        };
        let g = f - k * up * u;
        let dg = 1.0 + k * p * c * up;
        let nf = (f - g / dg).max(0.0);
        if (nf - f).abs() <= 1e-6 * (1e-3 + nf) {
            return nf;
        }
        f = nf;
    }
    f
}

impl Voice {
    /// `tune_cents`: correction of the string tuning that compensates the
    /// pitch shift caused by the reactive part of the bridge admittance.
    pub fn new(fs: f64, key: KeyParams, max_ap: usize, max_bq: usize, tune_cents: f64) -> Voice {
        let b1 = key.rate_after / 8.686;
        let (loss_g, loss_p) = design_loss(fs, key.f1, b1, key.b3);
        let (designs, _k) = design_dispersions(fs, key.f1, key.b_inh, loss_p, &[(max_ap, max_bq), (max_ap.min(LIGHT_AP), max_bq.min(LIGHT_BQ))]);
        let sum_w2: f64 = key.coupling[..key.n_wg].iter().map(|c| c * c).sum();
        let design = |disp: &Dispersion| -> Vec<StringDesign> {
            (0..key.n_wg)
                .map(|i| {
                    let f = key.f1 * 2f64.powf((key.detune[i] + tune_cents) / 1200.0);
                    let di = vary_dispersion(fs, f, key.b_inh, disp, key.b_var[i]);
                    let (des, disp) = match tune_loop(fs, f, loss_p, &di, key.strike_ratio).or_else(|| tune_loop(fs, f, loss_p, disp, key.strike_ratio)) {
                        Some(d) => (d, if tune_loop(fs, f, loss_p, &di, key.strike_ratio).is_some() { di } else { disp.clone() }),
                        None => (tune_loop(fs, f, loss_p, &Dispersion::none(), key.strike_ratio).expect("string loop too short"), Dispersion::none()),
                    };
                    StringDesign {
                        nl: des.nl,
                        nr: des.nr,
                        m_ap: disp.m_ap,
                        a_ap: disp.a_ap,
                        bq: disp.poles.iter().map(|&(rho, th)| [rho * rho, -2.0 * rho * th.cos()]).collect(),
                        loss_g: loss_g * (1.0 - loss_p),
                        loss_p,
                        eta: des.eta,
                        w: key.coupling[i],
                    }
                })
                .collect()
        };
        let strings = design(&designs[0].0);
        // (the same filter, if the key's full design is within the budget)
        let light = if designs[1].0.cost() < designs[0].0.cost() { design(&designs[1].0) } else { strings.clone() };
        let damp_on = if key.damper_t60 > 0.0 { 10f64.powf(-3.0 / (key.damper_t60 * key.f1)) } else { 1.0 };
        let (zone, zf) = bridge_zone(key.midi);
        let quiet_limit = 2 * strings.iter().chain(&light).map(|w| w.nl + w.nr).max().unwrap() + 256;

        // Longitudinal model on the first string (the strings of a unison
        // move alike while the tension modulation is significant).
        let (hl, hr) = (strings[0].nl / 2, strings[0].nr / 2);
        // ~4 samples at 48 kHz between quadrature points, at most ~128 points
        let stride = ((4.0 * fs / 48000.0).round() as usize).max((hl + hr) / 128).max(1);
        let pts = |h: usize| if h > stride { (h - stride / 2).div_ceil(stride) } else { 0 };
        // The force is evaluated at 48 kHz, and at half of that on the long
        // strings (keys up to about D3), where it costs the most: their
        // partials end well below 6 kHz, so the force (whose components lie
        // at sums of two partial frequencies) is carried by 24 kHz, and the
        // soundboard, whose modes end at 10 kHz, does not pass on what
        // holding the value for two samples adds above 14 kHz.
        let slow = if pts(hl) + pts(hr) >= LONG_SLOW_POINTS { 2 } else { 1 };
        let every = slow * ((fs / 48000.0).round() as u32).max(1);
        let half = (hl + hr) as f64;
        let cl: Vec<f32> = (0..pts(hl)).map(|j| (PI * (hl as f64 - (stride / 2 + j * stride) as f64) / half).cos() as f32).collect();
        let cr: Vec<f32> = (0..pts(hr)).map(|j| (PI * (hl as f64 + (stride / 2 + j * stride) as f64) / half).cos() as f32).collect();
        let th = 2.0 * PI * key.f_long / fs;
        let fs_l = fs / every as f64;
        let th = th * fs / fs_l;
        // longitudinal modes are short lived (T60 ~ 0.07 s)
        let rl = (-100.0 / fs_l).exp();
        let (lpr, lpi) = (rl * th.cos(), rl * th.sin());
        let long = Longitudinal {
            on: false,
            stride,
            coef: key.ea * stride as f64 / (4.0 * key.length * key.length * key.f1 * fs) * key.n_struck as f64,
            mode_on: key.f_long < 0.2 * fs_l && key.f_long < 9000.0 && !cr.is_empty(),
            mode_gain: ((1.0 - lpr).powi(2) + lpi * lpi) / lpi,
            cl,
            cr,
            pr: lpr,
            pi: lpi,
            sr: 0.0,
            si: 0.0,
            hp_c: (-2.0 * PI * 20.0 / fs_l).exp(),
            hp_s: 0.0,
            peak: 0.0,
            count: 0,
            every,
            phase: 0,
            held: 0.0,
        };
        Voice {
            n_struck: key.n_struck,
            pol_leak: key.pol_leak,
            z: key.z,
            inv2z: 1.0 / (2.0 * key.z),
            k_bridge: key.z * sum_w2,
            zone,
            sc: [1.0 - zf, zf],
            pan: [(1.0 - 0.25 * (key.bridge_pos - 0.5)).sqrt(), (1.0 + 0.25 * (key.bridge_pos - 0.5)).sqrt()],
            sc_full: [1.0 - zf, zf],
            symp_lever: 1.0,
            lever: 1.0,
            lever_coef: 1.0 - (-(LEVER_STEP as f64) / (0.4 * fs)).exp(),
            struck: false,
            e_last: 0.0,
            e_peak: 0.0,
            hammer: Hammer {
                active: false,
                y: 0.0,
                v: 0.0,
                t: 0,
                mass: key.hammer_mass,
                k: key.hammer_k,
                p: key.hammer_p,
                contact_samples: 0,
                peak_force: 0.0,
                pow_tab: (0..=POW_N).map(|i| (i as f64 / POW_SCALE).powf(key.hammer_p - 1.0)).collect(),
            },
            k_scale: key.strike,
            ys: [0.0; MAX_STRINGS],
            xi_prev: [0.0; MAX_STRINGS],
            f_prev: [0.0; MAX_STRINGS],
            long,
            damp_target: damp_on,
            damp_on,
            held: false,
            active: false,
            keep_alive: false,
            dt: 1.0 / fs,
            hyst: 0.08,
            e_sum: 0.0,
            e_cnt: 0,
            quiet: 0,
            quiet_limit,
            direct_g: 0.0,
            direct_hp_c: 1.0 - (-2.0 * PI * 0.5 * key.f1 / fs).exp(),
            direct_lp_c: 1.0 - (-2.0 * PI * (2.5 * key.f1).clamp(400.0, 5000.0) / fs).exp(),
            bridge: 0.0,
            morph: MorphBank::default(),
            morph_fade: 0.0,
            strings,
            light,
            is_light: false,
            key,
        }
    }

    /// Per-partial gains of the timbre correction (`morph.rs`); empty: none.
    pub fn set_morph(&mut self, fs: f64, gains: &[[f64; 4]]) {
        self.morph = MorphBank::new(&crate::morph::bank(fs, &self.key, gains));
    }

    /// Scale the coupling to the bridge by the lever ratio `c` (the bridge
    /// is stiffer than the bare board; see `Engine::new`).
    pub fn set_coupling(&mut self, c: f64) {
        let s = self.sc_full[0] + self.sc_full[1];
        self.sc_full = [c * self.sc_full[0] / s, c * self.sc_full[1] / s];
        self.apply_lever();
    }

    /// Lever of the strings while they only ring along (not struck, or struck
    /// so long ago that their own note has died away), relative to the
    /// struck key's.
    pub fn set_sympathetic_lever(&mut self, l: f64) {
        self.symp_lever = l;
        if !self.struck {
            self.lever = l;
        }
        self.apply_lever();
    }

    fn apply_lever(&mut self) {
        self.sc = [self.lever * self.sc_full[0], self.lever * self.sc_full[1]];
    }

    /// Call every `LEVER_STEP` samples. Once the key has been released and
    /// its own note has fallen `STRUCK_RANGE` below its level after the
    /// strike, the strings (kept open by the pedal) go back, gradually, to
    /// the lever of strings that only ring along. Returns true if the
    /// coupling to the bridge changed.
    pub fn update_lever(&mut self) -> bool {
        if self.struck && !self.held && !self.hammer.active && self.e_last < STRUCK_RANGE * self.e_peak {
            self.struck = false;
        }
        let target = if self.struck { 1.0 } else { self.symp_lever };
        if self.lever == target {
            return false;
        }
        self.lever += (target - self.lever) * self.lever_coef;
        if (self.lever - target).abs() < 1.0e-3 {
            self.lever = target;
        }
        // the correction of the key's own note leaves along with its lever
        self.morph_fade = if self.symp_lever < 1.0 { ((self.lever - self.symp_lever) / (1.0 - self.symp_lever)).clamp(0.0, 1.0) } else { 0.0 };
        if self.morph_fade == 0.0 {
            self.morph.clear();
        }
        self.apply_lever();
        true
    }

    pub fn set_hardness(&mut self, h: f64) {
        self.hammer.k = self.key.hammer_k * h;
    }

    pub fn strike(&mut self, v0: f64) {
        self.hammer.active = true;
        self.hammer.y = 0.0;
        self.hammer.v = v0;
        self.hammer.t = 0;
        self.hammer.contact_samples = 0;
        self.hammer.peak_force = 0.0;
        self.ys = [0.0; MAX_STRINGS];
        self.xi_prev = [0.0; MAX_STRINGS];
        self.f_prev = [0.0; MAX_STRINGS];
        self.held = true;
        self.struck = true;
        self.lever = 1.0;
        self.morph_fade = 1.0;
        self.e_peak = 0.0;
        self.apply_lever();
        self.damp_target = 1.0;
        self.active = true;
        self.quiet = 0;
        self.long.on = self.key.midi <= 84 && self.long.coef > 0.0 && !self.long.cr.is_empty();
        self.long.peak = 0.0;
        self.long.count = 0;
    }

    pub fn lift_damper(&mut self) {
        self.damp_target = 1.0;
    }

    pub fn drop_damper(&mut self) {
        self.damp_target = self.damp_on;
    }

    /// Back to rest (the strings themselves: `Bank::clear_key`).
    pub fn reset(&mut self) {
        self.hammer.active = false;
        self.struck = false;
        self.lever = self.symp_lever;
        self.e_last = 0.0;
        self.e_peak = 0.0;
        self.apply_lever();
        self.long.on = false;
        self.long.sr = 0.0;
        self.long.si = 0.0;
        self.long.hp_s = 0.0;
        self.long.held = 0.0;
        self.active = false;
        self.quiet = 0;
        self.e_sum = 0.0;
        self.e_cnt = 0;
        self.bridge = 0.0;
        self.morph_fade = 0.0;
        self.morph.clear();
    }

    /// True once the strings have been silent for longer than two round trips.
    pub fn is_silent(&self) -> bool {
        !self.hammer.active && self.quiet > self.quiet_limit
    }

    pub fn n_strings(&self) -> usize {
        self.strings.len()
    }

    /// One sample of the hammer in slot `k` of its bank. `a`, `b`: the waves
    /// arriving at the striking point from the agraffe and from the bridge
    /// side; `f`: what the hammer adds to both [m/s], per lane.
    pub fn hammer_step(&mut self, k: usize, a: &[f32; L], b: &[f32; L], f: &mut [f32; L]) {
        let dt = self.dt;
        let c_f = dt * self.inv2z;
        let h = &mut self.hammer;
        h.y += h.v * dt;
        let mut f_tot = 0.0;
        let mut f_first = 0.0;
        let mut still_touching = false;
        for i in 0..self.strings.len() {
            let l = i * KPB + k;
            let fi;
            if i < self.n_struck {
                let v_free = a[l] as f64 + b[l] as f64;
                let xi0 = h.y - (self.ys[i] + dt * v_free);
                let hf = (1.0 + self.hyst * (xi0.max(0.0) - self.xi_prev[i]) / dt).clamp(0.5, 1.5);
                let force = solve_force(h.k * self.k_scale[i] * hf, h.p, xi0, c_f, self.f_prev[i], &h.pow_tab);
                self.f_prev[i] = force;
                fi = force * self.inv2z;
                self.ys[i] += dt * (v_free + fi);
                self.xi_prev[i] = (h.y - self.ys[i]).max(0.0);
                f_tot += force;
                if i == 0 {
                    f_first = fi;
                }
                if xi0 > -1.0e-4 {
                    still_touching = true;
                }
            } else {
                fi = self.pol_leak * f_first;
            }
            f[l] = fi as f32;
        }
        h.v -= dt * f_tot / h.mass;
        h.t += 1;
        if f_tot > 0.0 {
            h.contact_samples += 1;
            if f_tot > h.peak_force {
                h.peak_force = f_tot;
            }
        }
        if (!still_touching && h.v < 0.0) || h.t as f64 * dt > 0.05 {
            h.active = false;
        }
    }

    /// Book the string energy of the last `n` samples (sum of the squared
    /// bridge waves) and the latest bridge wave.
    pub fn account(&mut self, energy: f64, n: usize, bridge: f64) {
        self.bridge = bridge;
        self.e_sum += energy;
        self.e_cnt += n;
        if self.e_cnt >= 64 {
            let mean = self.e_sum / self.e_cnt as f64;
            if mean < 1e-13 * self.strings.len() as f64 {
                self.quiet += self.e_cnt;
            } else {
                self.quiet = 0;
            }
            self.e_last = mean;
            if mean > self.e_peak {
                self.e_peak = mean;
            }
            self.e_sum = 0.0;
            self.e_cnt = 0;
        }
    }

    /// The timbre correction for the bridge waves `x` of a chunk: added to
    /// the two microphone signals.
    #[inline]
    pub fn run_morph(&mut self, x: &[f32], gain: f32, ml: &mut [f32], mr: &mut [f32]) {
        if self.morph_fade > 0.0 && !self.morph.is_empty() {
            self.morph.process(x, gain * self.morph_fade as f32, ml, mr);
        }
    }

    #[inline]
    pub fn longitudinal_on(&self) -> bool {
        self.long.on
    }

    /// Longitudinal force at the bridge end [N] (tension change) from the
    /// contents of the two delay lines of the first string.
    pub fn step_longitudinal(&mut self, left: &Line, right: &Line) -> f64 {
        let lg = &mut self.long;
        lg.phase += 1;
        if lg.phase < lg.every {
            return lg.held;
        }
        lg.phase = 0;
        let st = lg.stride;
        let (mut t0, mut t1) = (0.0f32, 0.0f32);
        for (line, cw) in [(left, &lg.cl), (right, &lg.cr)] {
            let n = line.len;
            // outgoing wave written d+1 samples ago, returning wave n-d samples ago
            let d0 = st / 2;
            assert!(cw.is_empty() || d0 + (cw.len() - 1) * st < n);
            let (mut q0, mut q1) = (F4::zero(), F4::zero());
            let mut j = 0;
            while j + 4 <= cw.len() {
                let (ia, ib) = (n - 1 - d0 - j * st, d0 + j * st);
                let v = F4::new(line.get(ia), line.get(ia - st), line.get(ia - 2 * st), line.get(ia - 3 * st))
                    + F4::new(line.get(ib), line.get(ib + st), line.get(ib + 2 * st), line.get(ib + 3 * st));
                let e = v * v;
                q0 += e;
                q1 += F4::from_slice(&cw[j..]) * e;
                j += 4;
            }
            t0 += q0.sum();
            t1 += q1.sum();
            while j < cw.len() {
                let v = line.get(n - 1 - d0 - j * st) + line.get(d0 + j * st);
                let e = v * v;
                t0 += e;
                t1 += cw[j] * e;
                j += 1;
            }
        }
        let t0 = t0 as f64 * lg.coef;
        let t1 = t1 as f64 * lg.coef;
        let mut f = t0;
        if lg.mode_on {
            let (sr, si) = (lg.pr * lg.sr - lg.pi * lg.si + t1, lg.pi * lg.sr + lg.pr * lg.si);
            lg.sr = sr;
            lg.si = si;
            f += 2.0 * (lg.mode_gain * si - t1);
        }
        // remove the slowly varying mean tension
        lg.hp_s = f + lg.hp_c * (lg.hp_s - f);
        let out = f - lg.hp_s;
        // The force falls with the square of the string amplitude: stop
        // computing it once it has become negligible.
        lg.peak = lg.peak.max(out.abs());
        lg.count += 1;
        if lg.count >= 4096 {
            if lg.peak < LONG_FLOOR && !self.hammer.active {
                lg.on = false;
            }
            lg.peak = 0.0;
            lg.count = 0;
        }
        lg.held = out;
        out
    }
}
