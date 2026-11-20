//! The strings of four neighbouring keys, computed together.
//!
//! The string model is that of `voice.rs` (two delay lines meeting at the
//! striking point, dispersion allpasses, loss filter, tuning allpass). What
//! this module adds is only how it is laid out and scheduled:
//!
//! * The state of the 12 strings of a bank (3 per key; lane = string * 4 +
//!   key) sits in three vectors of four single-precision values (string
//!   `s` of the four keys), so that every filter step is three SIMD
//!   operations for all of them, instead of 12 scalar double-precision ones
//!   scattered over the heap.
//! * A bank runs several samples in a row (`phase_a`) before the bridge is
//!   solved for them. That is exact: what a string sends to the bridge at
//!   sample `n` depends on the bridge motion only up to sample `n - nr`
//!   (`nr`: its round trip striking point -> bridge -> striking point), so a
//!   bank can run ahead by the shortest `nr` of its strings (`blk`). The
//!   wave reflected at the bridge is first stored as if the bridge stood
//!   still; `fixup` adds the motion of the bridge once it is known.
//!
//! The delay lines of the four keys of a string vector share one region of
//! the bank's buffer, one row of four values per sample: a sample is
//! written as one vector, and each key reads its own row further back. The
//! lines are windows that slide through the region (write at row `w`, read
//! at `w - d`); at the end of the region the window is moved back to its
//! start. Within a run of samples no index wraps, so the loops need no
//! per-sample bookkeeping.
//!
//! Keys that are not sounding keep an all-zero state, which costs nothing
//! but the arithmetic of their lanes while a neighbour in the bank sounds.
//! (Giving every struck key a vector of its own instead, lane = string, was
//! tried: it costs less for a single chord, but 30 to 40 % more on real
//! pieces, where most banks hold several sounding keys.)

use crate::design::*;
use crate::simd::F4;
use crate::voice::Voice;

/// Keys per bank.
pub const KPB: usize = 4;
/// Lanes (strings) per bank.
pub const L: usize = KPB * MAX_STRINGS;
/// Longest run of samples the engine processes in one piece.
pub const CHUNK: usize = 32;

/// Room behind a delay line's window before it has to be moved back (at
/// least; a long line gets its own length).
const SLACK: usize = 4 * CHUNK;

/// Added to every wave reflected at the bridge: keeps the single-precision
/// state of a string that is left to ring for minutes out of the denormal
/// range (which is very slow on x86). 320 dB below a soft note.
const BIAS: f32 = 1.0e-16;

/// One value per string: `[string][key]`.
type V = [F4; MAX_STRINGS];
/// One value per key.
type K = F4;

/// Sums of a chunk that the banks add to (per sample). Each bank adds one
/// lane per key; the sum over the four lanes is what counts.
pub struct Acc {
    /// Waves arriving at the bridge points: `sum c Z w a` [N].
    pub s: [[F4; NZONES]; CHUNK],
    /// Longitudinal string force on the bridge points [N].
    pub fl: [[f64; NZONES]; CHUNK],
    /// Direct radiation of the strings (left, right) and the strings' tone
    /// as fed to the room.
    pub out: [[F4; 3]; CHUNK],
}

impl Acc {
    pub fn new() -> Acc {
        Acc { s: [[F4::zero(); NZONES]; CHUNK], fl: [[0.0; NZONES]; CHUNK], out: [[F4::zero(); 3]; CHUNK] }
    }

    pub fn clear(&mut self, len: usize) {
        self.s[..len].fill([F4::zero(); NZONES]);
        self.fl[..len].fill([0.0; NZONES]);
        self.out[..len].fill([F4::zero(); 3]);
    }
}

/// Bridge zones a bank's keys may be coupled to.
const ZSPAN: usize = 4;

/// What one string needs in the audio loop (designed in `Voice::new`).
#[derive(Clone, Debug, Default)]
pub struct StringDesign {
    /// Round trips striking point -> agraffe / -> bridge [samples].
    pub nl: usize,
    pub nr: usize,
    /// `m_ap` identical first-order allpasses with coefficient `a_ap`.
    pub m_ap: usize,
    pub a_ap: f64,
    /// Second-order allpass sections (b0, b1).
    pub bq: Vec<[f64; 2]>,
    /// One-pole loss filter: gain (times 1 - pole) and pole.
    pub loss_g: f64,
    pub loss_p: f64,
    /// Tuning allpass.
    pub eta: f64,
    /// Projection of the bridge velocity on this string / polarisation.
    pub w: f64,
}

#[repr(C, align(64))]
pub struct Bank {
    // dispersion: first-order sections, then second-order ones; lanes with
    // fewer sections hold the identity (a = 1 resp. b0 = 1, b1 = 0)
    nap: usize,
    nbq: usize,
    ap_a: [V; MAX_AP],
    ap_s: [V; MAX_AP],
    bq0: [V; MAX_BQ],
    bq1: [V; MAX_BQ],
    bs1: [V; MAX_BQ],
    bs2: [V; MAX_BQ],
    loss_g: V,
    loss_p: V,
    loss_s: V,
    eta: V,
    frac_s: V,
    w: V,
    /// `-w` per lane.
    wn: [f32; L],
    // per key: damper, impedance, band-pass of the direct radiation
    dg: K,
    dg_tgt: K,
    dg_coef: K,
    z: K,
    hp_c: K,
    hp_s: K,
    lp_c: K,
    lp_s: K,
    /// Left / right gain of the direct radiation.
    gl: K,
    gr: K,
    e_acc: K,
    /// Coupling of the keys to the bridge zones `zb .. zb + ZSPAN`, and the
    /// values it was made from.
    zb: usize,
    wz: [K; ZSPAN],
    sc: [[f64; 2]; KPB],
    /// Delay lines: per string vector the agraffe side (`2 s`) and the
    /// bridge side (`2 s + 1`) of its four keys.
    lines: [Lines; 2 * MAX_STRINGS],
    /// Samples computed so far (modulo 2^32).
    pos: u32,
    /// Samples that still fit into every region before a window has to be
    /// moved back (a lower bound).
    room: u32,
    buf: Vec<f32>,
    /// Strings in use per key, and string vectors in use.
    n_wg: [usize; KPB],
    ns: usize,
    /// Per key: dispersion sections and shortest bridge-side round trip of
    /// the strings as installed.
    kap: [usize; KPB],
    kbq: [usize; KPB],
    kblk: [usize; KPB],
    /// Samples the bank may run ahead of the bridge.
    pub blk: usize,
    pub active: [bool; KPB],
    pub n_active: usize,
    /// Keys whose longitudinal force is computed in this chunk.
    pub long_on: [bool; KPB],
    /// Bridge wave `Z sum(w a)` of each key, per sample of the chunk.
    pub xk: [[f32; KPB]; CHUNK],
    /// Samples of the chunk done by `phase_a` / by `fixup`.
    pub a_done: usize,
    pub b_done: usize,
    /// Work space of `fixup`: bridge velocity at each key.
    vb: [[f32; CHUNK]; KPB],
}

/// The delay lines of the four keys of one string vector on one side of
/// the striking point: rows of four values in the bank's buffer.
#[derive(Clone, Copy, Default)]
struct Lines {
    /// First value of the region, and its size in rows.
    beg: u32,
    cap: u32,
    /// Delay of each key [rows].
    del: [u32; KPB],
    /// Row to be written next, counted from the bank's `pos` (modulo 2^32).
    w: u32,
    /// Where key `k` reads, in values relative to the row being written:
    /// `k - 4 del[k]`.
    off: [i32; KPB],
}

impl Lines {
    fn set_delay(&mut self, k: usize, d: usize) {
        assert!(d >= 1 && d + SLACK <= self.cap as usize, "delay line too short");
        self.del[k] = d as u32;
        self.off[k] = k as i32 - (KPB * d) as i32;
    }

    /// Value index of row `w + pos`.
    #[inline(always)]
    fn row(&self, pos: u32) -> usize {
        self.beg as usize + KPB * self.w.wrapping_add(pos) as usize
    }
}

/// The contents of one delay line, oldest sample first.
pub struct Line<'a> {
    buf: &'a [f32],
    first: usize,
    pub len: usize,
}

impl Line<'_> {
    #[inline(always)]
    pub fn get(&self, i: usize) -> f32 {
        debug_assert!(i < self.len);
        self.buf[self.first + KPB * i]
    }
}

impl Bank {
    /// A bank for keys whose strings have at most the given round trip
    /// lengths [samples] (`periods[key]`, 0: no key).
    pub fn new(fs: f64, periods: [usize; KPB], n_wg: [usize; KPB]) -> Bank {
        let ns = n_wg.iter().copied().max().unwrap_or(0);
        let longest = periods.iter().copied().max().unwrap_or(0);
        let mut lines = [Lines::default(); 2 * MAX_STRINGS];
        let mut at = 0u32;
        for (g, ln) in lines.iter_mut().enumerate() {
            // (the striking point is at most 1/8 of the string from the agraffe)
            let d = if g / 2 >= ns { 1 } else if g % 2 == 0 { longest / 4 + 8 } else { longest + 8 };
            ln.beg = at;
            ln.cap = (d + d.max(SLACK)) as u32;
            at += KPB as u32 * ln.cap;
            for k in 0..KPB {
                ln.set_delay(k, 1);
            }
            ln.w = 1;
        }
        let one = [F4::splat(1.0); MAX_STRINGS];
        let zero = [F4::zero(); MAX_STRINGS];
        Bank {
            nap: 0,
            nbq: 0,
            ap_a: [one; MAX_AP],
            ap_s: [zero; MAX_AP],
            bq0: [one; MAX_BQ],
            bq1: [zero; MAX_BQ],
            bs1: [zero; MAX_BQ],
            bs2: [zero; MAX_BQ],
            loss_g: zero,
            loss_p: zero,
            loss_s: zero,
            eta: zero,
            frac_s: zero,
            w: zero,
            wn: [0.0; L],
            dg: F4::splat(1.0),
            dg_tgt: F4::splat(1.0),
            dg_coef: F4::splat((1.0 - (-1.0 / (0.004 * fs)).exp()) as f32),
            z: F4::zero(),
            hp_c: F4::zero(),
            hp_s: F4::zero(),
            lp_c: F4::zero(),
            lp_s: F4::zero(),
            gl: F4::zero(),
            gr: F4::zero(),
            e_acc: F4::zero(),
            zb: 0,
            wz: [F4::zero(); ZSPAN],
            sc: [[0.0; 2]; KPB],
            lines,
            pos: 0,
            room: 0,
            buf: vec![0.0; at as usize],
            n_wg,
            ns,
            kap: [0; KPB],
            kbq: [0; KPB],
            kblk: [CHUNK; KPB],
            blk: CHUNK,
            active: [false; KPB],
            n_active: 0,
            long_on: [false; KPB],
            xk: [[0.0; KPB]; CHUNK],
            a_done: 0,
            b_done: 0,
            vb: [[0.0; CHUNK]; KPB],
        }
    }

    /// Put the strings of `v` into slot `k` (silent, damper as `v` has it):
    /// its full design, or the one for strings that only ring along.
    pub fn install(&mut self, k: usize, v: &Voice, light: bool) {
        let strings = if light { &v.light } else { &v.strings };
        assert_eq!(strings.len(), self.n_wg[k]);
        let (mut nap, mut nbq, mut blk) = (0, 0, CHUNK);
        for (s, d) in strings.iter().enumerate() {
            assert!(d.m_ap <= MAX_AP && d.bq.len() <= MAX_BQ);
            self.lines[2 * s].set_delay(k, d.nl);
            self.lines[2 * s + 1].set_delay(k, d.nr);
            for i in 0..MAX_AP {
                self.ap_a[i][s].set(k, if i < d.m_ap { d.a_ap as f32 } else { 1.0 });
            }
            for i in 0..MAX_BQ {
                let q = d.bq.get(i).copied().unwrap_or([1.0, 0.0]);
                self.bq0[i][s].set(k, q[0] as f32);
                self.bq1[i][s].set(k, q[1] as f32);
            }
            self.loss_g[s].set(k, d.loss_g as f32);
            self.loss_p[s].set(k, d.loss_p as f32);
            self.eta[s].set(k, d.eta as f32);
            self.w[s].set(k, d.w as f32);
            self.wn[s * KPB + k] = -d.w as f32;
            nap = nap.max(d.m_ap);
            nbq = nbq.max(d.bq.len());
            blk = blk.min(d.nr);
        }
        self.z.set(k, v.z as f32);
        self.hp_c.set(k, v.direct_hp_c as f32);
        self.lp_c.set(k, v.direct_lp_c as f32);
        self.sync(k, v);
        self.clear_key(k);
        // section counts and look-ahead of the bank as a whole
        (self.kap[k], self.kbq[k], self.kblk[k]) = (nap, nbq, blk);
        self.nap = self.kap.iter().copied().max().unwrap();
        self.nbq = self.kbq.iter().copied().max().unwrap();
        self.blk = self.kblk.iter().copied().min().unwrap().max(1);
    }

    /// Take over what the engine may have changed on the key since the last
    /// chunk: damper position, level of the direct radiation.
    #[inline]
    pub fn sync(&mut self, k: usize, v: &Voice) {
        self.dg_tgt.set(k, v.damp_target as f32);
        self.gl.set(k, (v.pan[0] * v.direct_g) as f32);
        self.gr.set(k, (v.pan[1] * v.direct_g) as f32);
        if self.sc[k] != v.sc {
            self.sc[k] = v.sc;
            let at = v.zone - self.zb;
            assert!(at + 1 < ZSPAN);
            for j in 0..ZSPAN {
                self.wz[j].set(k, if j == at { v.sc[0] as f32 } else if j == at + 1 { v.sc[1] as f32 } else { 0.0 });
            }
        }
    }

    /// Tell the bank which bridge zones its keys sit on (once, before the
    /// keys are installed).
    pub fn set_zones(&mut self, lowest: usize) {
        self.zb = lowest.min(NZONES - ZSPAN);
    }

    /// Bring the strings of slot `k` to rest.
    pub fn clear_key(&mut self, k: usize) {
        for ln in &self.lines[..2 * self.ns] {
            // (the rows the keys still read; the ones ahead are written first)
            let at = ln.w.wrapping_add(self.pos) as usize;
            let back = at.min(ln.del.iter().copied().max().unwrap() as usize);
            for row in at - back..at {
                self.buf[ln.beg as usize + KPB * row + k] = 0.0;
            }
        }
        for s in 0..MAX_STRINGS {
            for i in 0..MAX_AP {
                self.ap_s[i][s].set(k, 0.0);
            }
            for i in 0..MAX_BQ {
                self.bs1[i][s].set(k, 0.0);
                self.bs2[i][s].set(k, 0.0);
            }
            self.loss_s[s].set(k, 0.0);
            self.frac_s[s].set(k, 0.0);
        }
        self.hp_s.set(k, 0.0);
        self.lp_s.set(k, 0.0);
        self.e_acc.set(k, 0.0);
        self.dg.set(k, self.dg_tgt.get(k));
        for x in self.xk.iter_mut() {
            x[k] = 0.0;
        }
        // (the windows may have to move before the next sample)
        self.room = 0;
    }

    /// Dispersion sections computed per string (first-order, second-order).
    pub fn sections(&self) -> (usize, usize) {
        (self.nap, self.nbq)
    }

    pub fn set_active(&mut self, k: usize, on: bool) {
        if self.active[k] != on {
            self.active[k] = on;
            self.n_active = self.active.iter().filter(|&&a| a).count();
        }
    }

    /// Sum of the squared bridge waves of key `k` since the last call.
    #[inline]
    pub fn take_energy(&mut self, k: usize) -> f32 {
        let e = self.e_acc.get(k);
        self.e_acc.set(k, 0.0);
        e
    }

    /// Make room for `n` more samples in every delay line: a window that
    /// has reached the end of its region (or does not reach back far enough
    /// in it) is moved to where it has both.
    #[cold]
    fn make_room(&mut self, n: u32) {
        let pos = self.pos;
        let mut room = u32::MAX;
        for ln in self.lines[..2 * self.ns].iter_mut() {
            let longest = ln.del.iter().copied().max().unwrap();
            let mut at = ln.w.wrapping_add(pos);
            if at + n > ln.cap || at < longest {
                // keep what the keys still read: the last `longest` rows
                let keep = longest.min(at);
                let from = ln.beg as usize + KPB * (at - keep) as usize;
                let to = ln.beg as usize + KPB * (longest - keep) as usize;
                self.buf.copy_within(from..from + KPB * keep as usize, to);
                // (a key whose line has just become longer finds silence further back)
                self.buf[ln.beg as usize..to].fill(0.0);
                at = longest;
                ln.w = at.wrapping_sub(pos);
            }
            room = room.min(ln.cap - at);
        }
        self.room = room;
    }

    /// Samples `t0 .. t0 + n` of the chunk (`n <= blk`): string loops,
    /// hammers, and what the keys send to the bridge and radiate directly.
    pub fn phase_a(&mut self, t0: usize, n: usize, voices: &mut [Voice], long_gain: f64, acc: &mut Acc) {
        debug_assert!(n <= self.blk && t0 + n <= CHUNK && voices.len() == KPB);
        if self.room < n as u32 {
            self.make_room(n as u32);
        }
        self.room -= n as u32;
        if self.ns == 2 {
            self.run::<2>(t0, n, voices, long_gain, acc);
        } else {
            self.run::<3>(t0, n, voices, long_gain, acc);
        }
        self.pos = self.pos.wrapping_add(n as u32);
        self.b_done = t0;
        self.a_done = t0 + n;
    }

    /// `phase_a` for a bank with `NS` strings per key.
    #[inline(always)]
    fn run<const NS: usize>(&mut self, t0: usize, n: usize, voices: &mut [Voice], long_gain: f64, acc: &mut Acc) {
        let (nap, nbq) = (self.nap, self.nbq);
        let mut hammers = voices.iter().any(|v| v.hammer.active);
        let longs = self.long_on.iter().any(|&b| b);
        // value index of the row written in the first sample, per line group
        let rows: [usize; 6] = std::array::from_fn(|g| self.lines[g].row(self.pos));
        for i in 0..n {
            let t = t0 + i;
            let buf = self.buf.as_mut_ptr();
            // waves arriving at the striking point from the bridge / agraffe side
            let mut x = [F4::zero(); NS];
            let mut a = [F4::zero(); NS];
            for s in 0..NS {
                let (ll, lr) = (&self.lines[2 * s], &self.lines[2 * s + 1]);
                // SAFETY: `make_room` keeps the rows `w - del .. w + n` of
                // every line group inside its region of `buf`.
                unsafe {
                    let (pl, pr) = (buf.add(rows[2 * s] + KPB * i) as *const f32, buf.add(rows[2 * s + 1] + KPB * i) as *const f32);
                    x[s] = F4::gather(std::array::from_fn(|k| pr.offset(lr.off[k] as isize)));
                    a[s] = -F4::gather(std::array::from_fn(|k| pl.offset(ll.off[k] as isize)));
                }
            }
            // dispersion
            for i in 0..nap {
                let c = &self.ap_a[i];
                let st = &mut self.ap_s[i];
                for s in 0..NS {
                    let y = c[s] * x[s] + st[s];
                    st[s] = x[s] - c[s] * y;
                    x[s] = y;
                }
            }
            for i in 0..nbq {
                let (b0, b1) = (&self.bq0[i], &self.bq1[i]);
                let (s1, s2) = (&mut self.bs1[i], &mut self.bs2[i]);
                for s in 0..NS {
                    let y = b0[s] * x[s] + s1[s];
                    s1[s] = b1[s] * (x[s] - y) + s2[s];
                    s2[s] = x[s] - b0[s] * y;
                    x[s] = y;
                }
            }
            // loss, tuning, reflection at the bridge end, damper
            self.dg += (self.dg_tgt - self.dg) * self.dg_coef;
            let mut b = [F4::zero(); NS];
            for s in 0..NS {
                let ls = self.loss_g[s] * x[s] + self.loss_p[s] * self.loss_s[s];
                self.loss_s[s] = ls;
                let y = self.eta[s] * ls + self.frac_s[s];
                self.frac_s[s] = ls - self.eta[s] * y;
                b[s] = -(y * self.dg);
            }
            // hammers
            let mut f = [F4::zero(); NS];
            if hammers {
                let (mut aa, mut ba, mut fa) = ([0.0f32; L], [0.0f32; L], [0.0f32; L]);
                for s in 0..NS {
                    aa[s * KPB..s * KPB + KPB].copy_from_slice(&a[s].to_array());
                    ba[s * KPB..s * KPB + KPB].copy_from_slice(&b[s].to_array());
                }
                hammers = false;
                for (k, v) in voices.iter_mut().enumerate() {
                    if v.hammer.active {
                        v.hammer_step(k, &aa, &ba, &mut fa);
                        hammers = true;
                    }
                }
                for s in 0..NS {
                    f[s] = F4::from_slice(&fa[s * KPB..]);
                }
            }
            let (mut xk, mut en) = (F4::zero(), F4::zero());
            for s in 0..NS {
                let tb = a[s] + f[s];
                // SAFETY: as above
                unsafe {
                    (b[s] + f[s]).write_ptr(buf.add(rows[2 * s] + KPB * i));
                    // (the bridge motion is added by `fixup`)
                    tb.write_ptr(buf.add(rows[2 * s + 1] + KPB * i));
                }
                // per key: bridge wave, energy
                xk += self.w[s] * tb;
                en += tb * tb;
            }
            let xk = xk * self.z;
            self.e_acc += en;
            // direct radiation
            self.hp_s += self.hp_c * (xk - self.hp_s);
            self.lp_s += self.lp_c * (xk - self.hp_s - self.lp_s);
            let out = &mut acc.out[t];
            out[0] += self.gl * self.lp_s;
            out[1] += self.gr * self.lp_s;
            out[2] += self.lp_s;
            // (keys at rest send nothing: their waves are zero)
            let sz = &mut acc.s[t][self.zb..self.zb + ZSPAN];
            for j in 0..ZSPAN {
                sz[j] += self.wz[j] * xk;
            }
            self.xk[t] = xk.to_array();
            if longs {
                for (k, v) in voices.iter_mut().enumerate() {
                    if self.long_on[k] {
                        // (string 0 of the key, after this sample was written)
                        let line = |g: usize| {
                            let len = self.lines[g].del[k] as usize;
                            Line { buf: &self.buf, first: rows[g] + KPB * (i + 1) - KPB * len + k, len }
                        };
                        let fl = long_gain * v.step_longitudinal(&line(0), &line(1));
                        acc.fl[t][v.zone] += v.sc[0] * fl;
                        acc.fl[t][v.zone + 1] += v.sc[1] * fl;
                    }
                }
            }
        }
    }

    /// Add the motion of the bridge (`vz[zone][t]`, [m/s]) to the waves that
    /// `phase_a` reflected there in samples `b_done .. a_done`.
    pub fn fixup(&mut self, voices: &[Voice], vz: &[[f32; CHUNK]; NZONES]) {
        let (t0, n) = (self.b_done, self.a_done - self.b_done);
        self.b_done = self.a_done;
        if n == 0 {
            return;
        }
        // bridge velocity at each key (those at rest: whatever, times zero)
        for (k, v) in voices.iter().enumerate() {
            if self.active[k] {
                let (c0, c1) = (v.sc[0] as f32, v.sc[1] as f32);
                let (v0, v1) = (&vz[v.zone][t0..t0 + n], &vz[v.zone + 1][t0..t0 + n]);
                for (y, (&a, &b)) in self.vb[k][..n].iter_mut().zip(v0.iter().zip(v1)) {
                    *y = c0 * a + c1 * b;
                }
            }
        }
        let on = F4::from_array(std::array::from_fn(|k| if self.active[k] { 1.0 } else { 0.0 }));
        let bias = F4::splat(BIAS) * on;
        let wn: [F4; MAX_STRINGS] = std::array::from_fn(|s| F4::from_slice(&self.wn[s * KPB..]) * on);
        // the rows written in these `n` samples, per string vector
        let ends: [usize; MAX_STRINGS] = std::array::from_fn(|s| self.lines[2 * s + 1].row(self.pos) - KPB * n);
        for i in 0..n {
            let u = F4::new(self.vb[0][i], self.vb[1][i], self.vb[2][i], self.vb[3][i]);
            for s in 0..self.ns {
                let row = &mut self.buf[ends[s] + KPB * i..];
                (F4::from_slice(row) + (bias + wn[s] * u)).write_to(row);
            }
        }
    }
}
