//! The instrument: 88 keys, sustain pedal, soundboard, room.
//!
//! Strings and soundboard are solved together, sample by sample. With the
//! bridge-point velocities `v` (one per zone), the waves `a` arriving from
//! the strings and the board's admittance split into its instantaneous part
//! `Y` and its history `h`:
//!
//! ```text
//!   F = 2 S - K v          force of the strings on the bridge (S = sum c Z w a)
//!   v = h + Y F            soundboard
//!   =>  v = (I + Y K)^-1 (h + 2 Y S)
//! ```
//!
//! Decay rates, two-stage decay, beating and sympathetic resonance all follow
//! from this coupling; none of them is imposed separately.
//!
//! The work is done in chunks of up to 32 samples. Within a chunk the
//! strings run ahead of the bridge as far as their length allows (see
//! `bank.rs`), the bridge and the soundboard are solved sample by sample,
//! and everything behind them (timbre correction, mixing, room) is computed
//! for the whole chunk at once.

use crate::bank::{Acc, Bank, CHUNK, KPB};
use crate::design::*;
use crate::leveling::Leveling;
use crate::room::{Room, NF};
use crate::soundboard::{Soundboard, NOUT};
use crate::voice::{Voice, LEVER_STEP};

pub const MAX_BLOCK: usize = 4096;
const _: () = assert!(CHUNK == LEVER_STEP && NKEYS % KPB == 0);
/// Fraction of the tension change that acts on the board (down-bearing
/// angle and rocking of the bridge).
const LONG_COUPLING: f64 = 0.06;
/// Key bed shock per unit hammer velocity [N s/m], as transmitted to the board.
const KNOCK_FORCE: f64 = 0.8;

/// Corner of the unbaffled-board roll-off [Hz]: close microphones / room feeds.
const DIP_HZ_MIC: f64 = 55.0;
const DIP_HZ_FAR: f64 = 100.0;

/// Fraction of the bridge-induced pitch shift that the tuning removes.
const TUNE_COMP: f64 = 0.75;
/// Largest pitch pull by the bridge that a key may have [cents].
const MAX_PULL: f64 = 3.0;
/// Largest decay rate of a fundamental relative to the key's nominal rate.
const MAX_F1_RATE: f64 = 1.3;

/// The room is fed mostly with the strings' own tone and only in part with
/// the board's radiation in the distant directions. That radiation has the
/// same gaps and peaks from key to key as the sound at the microphones,
/// but at other keys: fed with it alone, the room is as loud as the note
/// itself on single keys, which after a short key press sounds as if the
/// pedal were down. (Levels chosen to keep the average room level.)
const ROOM_BOARD: f64 = 0.4;
const ROOM_STRINGS: f64 = 0.9;

/// At most this many keys compute their longitudinal force at once.
const MAX_LONG: usize = 16;

const WAKE_PER_CHUNK: usize = 4;

/// Lever of strings that only ring along (dampers up, not struck since they
/// last came to rest), relative to the lever they have once their key is
/// struck. All strings of a bridge region share two bridge points here, so
/// every string feels the full motion that any other one causes; on the
/// instrument each string has its own place on the bridge. At 1 the strings
/// next to a repeated key (and those tuned to a soundboard resonance) pick
/// up enough of every strike to be heard as a second pitch; see
/// `docs/experiments/10-pedal-repetition.md` (`repeat scan`).
const SYMPATHETIC_LEVER: f64 = 0.35;

/// After a change of the reference pitch one silent key is retuned per this
/// many samples (so that the work does not show as a load spike).
const RETUNE_EVERY: usize = 128;

/// Fastest hammer the levelling table may ask for [m/s].
pub const MAX_HAMMER_V: f64 = 16.0;

type Mat = [[f64; NZONES]; NZONES];

pub struct Engine {
    pub fs: f64,
    pub voices: Vec<Voice>,
    /// The strings of the keys, four keys per bank.
    banks: Vec<Bank>,
    active: Vec<usize>,
    pub board: Soundboard,
    room: Room,
    pub out_l: Vec<f32>,
    pub out_r: Vec<f32>,
    pedal: bool,
    /// Master output gain.
    pub gain: f64,
    /// How much of the soundboard motion the strings feel (1 = physical).
    pub sympathetic: f64,
    /// With the pedal down, let all 88 keys' strings resonate.
    pub pedal_resonance: bool,
    /// Level of the room (reflections + reverberation), 1 = as computed for the room.
    pub room_amount: f64,
    /// Level of the key bed shock (1 = nominal).
    pub knock: f64,
    /// Level of the longitudinal string force (1 = nominal).
    pub longitudinal: f64,
    idle: bool,
    dirty: bool,
    /// Voices waiting to join the computation (a few per chunk, so that
    /// waking all strings at once does not produce a load spike).
    pending: Vec<usize>,
    k_mat: Mat,
    p_mat: Mat,
    q_mat: Mat,
    knock_buf: Vec<f32>,
    knock_pos: usize,
    knock_left: usize,
    /// Per-key loudness levelling table and its switch.
    leveling_table: Leveling,
    pub leveling: bool,
    max_ap: usize,
    max_bq: usize,
    /// Hammer hardness factor (kept to restore it on a rebuilt key).
    hardness: f64,
    /// Keys whose strings are still tuned to the previous reference pitch.
    stale: Vec<bool>,
    stale_left: usize,
    /// Samples rendered since the last key was retuned.
    retune_clock: usize,
    /// Hammer velocity of the latest strike [m/s] (diagnostics).
    pub last_hammer_velocity: f64,
    /// Level of the strings' direct radiation (1 = as voiced, see `voicing.rs`).
    pub direct: f64,
    /// What the room is fed with: the board's distant radiation and the
    /// strings' own tone (see `ROOM_BOARD`).
    pub room_modal: f64,
    pub room_tone: f64,
    /// Level of the soundboard's sound at the microphones (1; diagnostics).
    pub board_out: f64,
    /// Level of the per-key timbre correction (1 = as in the table, see
    /// `morph.rs`; 0 = the model alone).
    pub morph: f64,
    /// Low-frequency roll-off of the radiation, per receiver (see `process`).
    dip_c: [f32; NOUT],
    dip_s: [f32; NOUT],
    // work space of a chunk
    acc: Acc,
    /// Bridge velocities [zone][sample].
    vz: [[f32; CHUNK]; NZONES],
    /// Soundboard radiation [receiver][sample].
    rad: [[f32; CHUNK]; NOUT],
}

#[inline]
fn soft_clip(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.8 {
        x
    } else {
        x.signum() * (0.8 + 0.2 * ((a - 0.8) / 0.2).tanh())
    }
}

fn invert(mut a: Mat) -> Mat {
    let mut inv = [[0.0; NZONES]; NZONES];
    for i in 0..NZONES {
        inv[i][i] = 1.0;
    }
    for c in 0..NZONES {
        let mut p = c;
        for r in c + 1..NZONES {
            if a[r][c].abs() > a[p][c].abs() {
                p = r;
            }
        }
        a.swap(c, p);
        inv.swap(c, p);
        let d = 1.0 / a[c][c];
        for j in 0..NZONES {
            a[c][j] *= d;
            inv[c][j] *= d;
        }
        for r in 0..NZONES {
            if r != c {
                let f = a[r][c];
                if f != 0.0 {
                    for j in 0..NZONES {
                        a[r][j] -= f * a[c][j];
                        inv[r][j] -= f * inv[c][j];
                    }
                }
            }
        }
    }
    inv
}

impl Engine {
    pub fn new(fs: f64) -> Engine {
        Self::with_options(fs, MAX_AP, MAX_BQ)
    }

    pub fn with_options(fs: f64, max_ap: usize, max_bq: usize) -> Engine {
        let board = Soundboard::new(fs);
        let voices: Vec<Voice> =
            (0..NKEYS).map(|i| Self::build_voice(&board, fs, MIDI_LO + i, max_ap, max_bq, crate::sustain::correction(MIDI_LO + i))).collect();
        // room in the delay lines for the lowest reference pitch
        let period = |i: usize| (1.06 * fs / (voices[i].key.f1 * A4_MIN / reference_pitch())).ceil() as usize;
        let mut banks: Vec<Bank> = (0..NKEYS / KPB)
            .map(|b| Bank::new(fs, std::array::from_fn(|k| period(b * KPB + k)), std::array::from_fn(|k| voices[b * KPB + k].n_strings())))
            .collect();
        for (b, bank) in banks.iter_mut().enumerate() {
            bank.set_zones(voices[b * KPB..(b + 1) * KPB].iter().map(|v| v.zone).min().unwrap());
        }
        for (i, v) in voices.iter().enumerate() {
            banks[i / KPB].install(i % KPB, v, false);
        }
        let klen = ((0.02 * fs) as usize).next_power_of_two();
        Engine {
            fs,
            voices,
            banks,
            active: Vec::with_capacity(NKEYS),
            board,
            room: Room::new(fs),
            out_l: vec![0.0; MAX_BLOCK],
            out_r: vec![0.0; MAX_BLOCK],
            pedal: false,
            gain: 0.013,
            sympathetic: 1.0,
            pedal_resonance: true,
            room_amount: 0.30,
            knock: 1.0,
            longitudinal: 1.0,
            idle: true,
            dirty: true,
            pending: Vec::with_capacity(NKEYS),
            k_mat: [[0.0; NZONES]; NZONES],
            p_mat: [[0.0; NZONES]; NZONES],
            q_mat: [[0.0; NZONES]; NZONES],
            knock_buf: vec![0.0; klen],
            knock_pos: 0,
            knock_left: 0,
            leveling_table: Leveling::new(fs),
            leveling: true,
            max_ap,
            max_bq,
            hardness: 1.0,
            stale: vec![false; NKEYS],
            stale_left: 0,
            retune_clock: 0,
            last_hammer_velocity: 0.0,
            direct: 1.0,
            room_modal: ROOM_BOARD,
            room_tone: ROOM_STRINGS,
            board_out: 1.0,
            morph: 1.0,
            dip_c: std::array::from_fn(|c| {
                let fc = if c < 2 { DIP_HZ_MIC } else { DIP_HZ_FAR };
                (1.0 - (-2.0 * std::f64::consts::PI * fc / fs).exp()) as f32
            }),
            dip_s: [0.0; NOUT],
            acc: Acc::new(),
            vz: [[0.0; CHUNK]; NZONES],
            rad: [[0.0; CHUNK]; NOUT],
        }
    }

    /// One key's strings and their lever on the bridge. `sustain`: the
    /// key's (prompt, after) multipliers on its nominal decay rates, see
    /// `sustain.rs`.
    fn build_voice(board: &Soundboard, fs: f64, midi: usize, max_ap: usize, max_bq: usize, sustain: (f64, f64)) -> Voice {
        // Lever ratio between the key's strings and the board, chosen so
        // that the in-phase motion of the unison loses energy to the board at
        // the key's nominal rate on average. The actual rate of each partial
        // follows the board's admittance at that frequency.
        let mut k = key_params(midi);
        // (the mistuning of the unison stays that of the nominal rates)
        let extra = sustain.0 * (k.rate_prompt - k.rate_after);
        k.rate_after *= sustain.1;
        k.rate_prompt = k.rate_after + extra;
        let (zone, zf) = bridge_zone(k.midi);
        let mut sc = [0.0; NZONES];
        sc[zone] = 1.0 - zf;
        sc[zone + 1] = zf;
        let f_lo = (0.8 * k.f1).min(5000.0);
        let f_hi = (6.0 * k.f1).min(9000.0).max(1.5 * f_lo);
        // (No correction here for a fundamental that sits on a board
        // resonance: such a note speaks loudly and dies quickly. What that
        // does to the length of the note is evened out by `sustain`.)
        let g = board.mean_conductance(&sc, f_lo, f_hi, fs);
        let kappa = k.z * k.n_struck as f64;
        let mut c = (extra / (8.686 * 2.0 * kappa * g * k.f1)).sqrt().clamp(0.15, 1.5);
        // The bridge is not rigid: the unison's in-phase mode sees the
        // reflection (1 - a) / (1 + a), a = kappa c^2 Y(f1), whose phase
        // detunes it by 275.5 arg(..) cents (-551 kappa c^2 Im(Y) to
        // first order). Near a board resonance that pull gets large;
        // there the lever is shortened until it is at most
        // `MAX_PULL` cents (the note still speaks louder and shorter than
        // its neighbours, but its pitch does not sag during the decay).
        let (re, im) = board.admittance(&sc, k.f1, fs);
        let pull = |c: f64| {
            let (ar, ai) = (kappa * c * c * re, kappa * c * c * im);
            // arg((1 - a) / (1 + a))
            275.5 * ((-ai).atan2(1.0 - ar) - ai.atan2(1.0 + ar))
        };
        // Likewise the fundamental may lose energy to the board at most
        // `MAX_F1_RATE` times faster than the key's nominal rate: beyond
        // that the string and the board resonance merge into one mode.
        let f1_rate = |c: f64| 8.686 * 2.0 * kappa * c * c * re * k.f1;
        let mut shift = pull(c);
        for _ in 0..60 {
            if shift.abs() <= MAX_PULL && f1_rate(c) <= MAX_F1_RATE * extra {
                break;
            }
            c *= 0.97;
            shift = pull(c);
        }
        let mut v = Voice::new(fs, k, max_ap, max_bq, -TUNE_COMP * shift);
        v.direct_g = crate::voicing::direct_gain(midi) * direct_share(midi);
        v.set_morph(fs, &crate::morph::gains(midi, fs));
        v.set_coupling(c);
        v.set_sympathetic_lever(SYMPATHETIC_LEVER);
        v
    }

    /// Rebuild one key with other sustain multipliers than the table's
    /// (for `gen_sustain`). Silences the instrument; the hammer hardness of
    /// the key goes back to nominal.
    pub fn set_key_sustain(&mut self, midi: usize, sustain: (f64, f64)) {
        self.all_off();
        let i = midi - MIDI_LO;
        self.voices[i] = Self::build_voice(&self.board, self.fs, midi, self.max_ap, self.max_bq, sustain);
        self.banks[i / KPB].install(i % KPB, &self.voices[i], false);
    }

    /// Direct radiation gain of one key other than the table's (for
    /// `gen_voicing`).
    pub fn set_key_direct(&mut self, midi: usize, g: f64) {
        self.voices[midi - MIDI_LO].direct_g = g;
    }

    /// Timbre correction gains of one key other than the table's (for
    /// `gen_morph`).
    pub fn set_key_morph(&mut self, midi: usize, gains: &[[f64; 4]]) {
        self.voices[midi - MIDI_LO].set_morph(self.fs, gains);
    }

    /// Retune the instrument to A4 = `hz`. The strings of a key are
    /// redesigned (a few ms of work each), so this is spread out: one silent
    /// key per `RETUNE_EVERY` samples in `process`, and a key that is struck
    /// before its turn is retuned on the spot. Strings that are sounding keep
    /// their pitch until they have died away or are struck again.
    pub fn set_reference_pitch(&mut self, hz: f64) {
        let before = reference_pitch();
        set_reference_pitch(hz);
        if reference_pitch() != before {
            self.stale.fill(true);
            self.stale_left = NKEYS;
        }
    }

    /// Rebuild the strings of one key for the current reference pitch.
    fn retune_key(&mut self, idx: usize) {
        let midi = MIDI_LO + idx;
        let mut v = Self::build_voice(&self.board, self.fs, midi, self.max_ap, self.max_bq, crate::sustain::correction(midi));
        let old = &self.voices[idx];
        v.set_hardness(self.hardness);
        v.held = old.held;
        v.keep_alive = old.keep_alive;
        v.active = old.active;
        if old.held || self.pedal {
            v.lift_damper();
        }
        v.is_light = old.is_light;
        self.banks[idx / KPB].install(idx % KPB, &v, v.is_light);
        self.voices[idx] = v;
        self.stale[idx] = false;
        self.stale_left -= 1;
        self.dirty = true;
    }

    fn activate(&mut self, idx: usize) {
        if !self.active.contains(&idx) {
            self.voices[idx].active = true;
            self.banks[idx / KPB].set_active(idx % KPB, true);
            self.active.push(idx);
            self.dirty = true;
        }
        self.idle = false;
        self.board.quiet = false;
    }

    /// Which strings the bank computes for the key: the full design (the
    /// key is struck) or the cheaper one of strings that only ring along.
    /// Changing it brings the strings to rest.
    fn set_design(&mut self, idx: usize, light: bool) {
        let v = &mut self.voices[idx];
        if v.is_light != light {
            v.is_light = light;
            self.banks[idx / KPB].install(idx % KPB, v, light);
        }
    }

    /// Activate later, spread over the coming chunks.
    fn activate_soon(&mut self, idx: usize) {
        if !self.active.contains(&idx) && !self.pending.contains(&idx) {
            self.pending.push(idx);
        }
        self.idle = false;
        self.board.quiet = false;
    }

    /// Bridge loading by the active strings changed: refresh the solver.
    fn refresh(&mut self) {
        let mut k = [[0.0; NZONES]; NZONES];
        for &i in &self.active {
            let v = &self.voices[i];
            for a in 0..2 {
                for b in 0..2 {
                    k[v.zone + a][v.zone + b] += v.k_bridge * v.sc[a] * v.sc[b];
                }
            }
        }
        let y = &self.board.y_inst;
        let mut m = [[0.0; NZONES]; NZONES];
        for i in 0..NZONES {
            for j in 0..NZONES {
                m[i][j] = if i == j { 1.0 } else { 0.0 };
                for l in 0..NZONES {
                    m[i][j] += y[i][l] * k[l][j];
                }
            }
        }
        let p = invert(m);
        let mut q = [[0.0; NZONES]; NZONES];
        for i in 0..NZONES {
            for j in 0..NZONES {
                for l in 0..NZONES {
                    q[i][j] += 2.0 * p[i][l] * y[l][j];
                }
            }
        }
        self.k_mat = k;
        self.p_mat = p;
        self.q_mat = q;
        self.dirty = false;
    }

    /// Key bed / key frame shock: a short force pulse into the case.
    fn add_knock(&mut self, amp: f64, secs: f64) {
        let n = ((secs * self.fs) as usize).clamp(4, self.knock_buf.len() - 1);
        let mask = self.knock_buf.len() - 1;
        for i in 0..n {
            let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * (i as f64 + 0.5) / n as f64).cos();
            self.knock_buf[(self.knock_pos + i) & mask] += (amp * w) as f32;
        }
        self.knock_left = self.knock_left.max(n);
        self.idle = false;
        self.board.quiet = false;
    }

    /// `vel` in 0..1.
    pub fn note_on(&mut self, midi: usize, vel: f64) {
        if !(MIDI_LO..MIDI_LO + NKEYS).contains(&midi) {
            return;
        }
        // The levelling table sits between the player and the model: it
        // only rescales the requested hammer velocity.
        let scale = if self.leveling { self.leveling_table.scale(midi, vel) } else { 1.0 };
        // The key bed shock goes with how fast the key was pressed, not
        // with what the levelling makes of it for this key's hammer.
        self.strike_key(midi, (hammer_velocity(vel) * scale).min(MAX_HAMMER_V), hammer_velocity(vel));
    }

    /// Strike `midi` with hammer velocity `v0` [m/s].
    pub fn strike(&mut self, midi: usize, v0: f64) {
        self.strike_key(midi, v0, v0);
    }

    /// Strike `midi` with hammer velocity `v0` [m/s]; `v_key`: nominal
    /// hammer velocity of the key press (key bed shock).
    pub fn strike_key(&mut self, midi: usize, v0: f64, v_key: f64) {
        if !(MIDI_LO..MIDI_LO + NKEYS).contains(&midi) {
            return;
        }
        let idx = midi - MIDI_LO;
        self.last_hammer_velocity = v0;
        if self.stale[idx] {
            self.retune_key(idx);
        }
        // (strings that were ringing along start anew with the full design;
        // what they held is far below what the hammer puts into them)
        self.set_design(idx, false);
        self.voices[idx].strike(v0);
        self.activate(idx);
        // the strike may have changed the key's lever (`SYMPATHETIC_LEVER`)
        self.dirty = true;
        for i in UNDAMPED_LO - MIDI_LO..NKEYS {
            self.activate_soon(i);
        }
        if self.knock > 0.0 {
            // Softer towards the top: against the short, thin notes above
            // C6 the same shock is heard as a hard click.
            let treble = 1.0 - 0.5 * ((midi as f64 - 84.0) / 24.0).clamp(0.0, 1.0);
            self.add_knock(self.knock * KNOCK_FORCE * treble * v_key, 0.003);
        }
    }

    pub fn note_off(&mut self, midi: usize) {
        if !(MIDI_LO..MIDI_LO + NKEYS).contains(&midi) {
            return;
        }
        let v = &mut self.voices[midi - MIDI_LO];
        let was_held = v.held;
        v.held = false;
        if !self.pedal {
            v.drop_damper();
        }
        if was_held && self.knock > 0.0 {
            // key return and damper landing
            self.add_knock(-0.3 * self.knock * KNOCK_FORCE, 0.005);
        }
    }

    pub fn set_pedal(&mut self, down: bool) {
        if down == self.pedal {
            return;
        }
        self.pedal = down;
        if down {
            for i in 0..NKEYS {
                self.voices[i].lift_damper();
                if self.pedal_resonance {
                    self.voices[i].keep_alive = true;
                    self.activate_soon(i);
                }
            }
        } else {
            for v in self.voices.iter_mut() {
                v.keep_alive = false;
                if !v.held {
                    v.drop_damper();
                }
            }
        }
    }

    pub fn set_hardness(&mut self, h: f64) {
        self.hardness = h;
        for v in self.voices.iter_mut() {
            v.set_hardness(h);
        }
    }

    pub fn all_off(&mut self) {
        self.pedal = false;
        for (i, v) in self.voices.iter_mut().enumerate() {
            v.keep_alive = false;
            v.held = false;
            v.drop_damper();
            v.reset();
            let b = &mut self.banks[i / KPB];
            b.sync(i % KPB, v);
            b.clear_key(i % KPB);
            b.set_active(i % KPB, false);
        }
        self.active.clear();
        self.pending.clear();
        self.dirty = true;
        self.board.reset();
        self.room.reset();
        self.knock_buf.fill(0.0);
        self.knock_left = 0;
        // a stale filter state here would be heard as a thump on the next note
        self.dip_s = [0.0; NOUT];
        self.idle = true;
    }

    /// Per bank of four keys: (keys sounding, first-order / second-order
    /// dispersion sections computed, samples it runs ahead of the bridge).
    pub fn bank_stats(&self) -> Vec<(usize, usize, usize, usize)> {
        self.banks.iter().map(|b| (b.n_active, b.sections().0, b.sections().1, b.blk)).collect()
    }

    pub fn active_voices(&self) -> usize {
        self.active.len()
    }

    pub fn active_strings(&self) -> usize {
        self.active.iter().map(|&i| self.voices[i].n_strings()).sum()
    }

    /// Render `n` (<= MAX_BLOCK) samples into `out_l` / `out_r`.
    pub fn process(&mut self, n: usize) {
        let n = n.min(MAX_BLOCK);
        if self.stale_left > 0 {
            self.retune_clock += n;
            if self.retune_clock >= RETUNE_EVERY {
                self.retune_clock = 0;
                let next = (0..NKEYS).find(|&i| self.stale[i] && !self.voices[i].active && !self.pending.contains(&i));
                if let Some(i) = next {
                    self.retune_key(i);
                }
            }
        }
        let mut off = 0;
        while off < n {
            let len = CHUNK.min(n - off);
            if self.idle {
                self.out_l[off..off + len].fill(0.0);
                self.out_r[off..off + len].fill(0.0);
            } else {
                self.chunk(off, len);
            }
            off += len;
        }
    }

    /// `len` (<= CHUNK) samples into `out_l` / `out_r` from `off`.
    fn chunk(&mut self, off: usize, len: usize) {
        for _ in 0..WAKE_PER_CHUNK {
            match self.pending.pop() {
                Some(i) => {
                    // strings at rest that are woken to ring along
                    if !self.voices[i].active {
                        self.set_design(i, true);
                    }
                    self.activate(i)
                }
                None => break,
            }
        }
        for &idx in &self.active {
            if self.voices[idx].update_lever() {
                self.dirty = true;
            }
        }
        if self.dirty {
            self.refresh();
        }
        let symp = self.sympathetic.clamp(0.0, 1.0);
        let long_gain = LONG_COUPLING * self.longitudinal;
        // (next[b]: the sample from which bank `b` has to be computed next)
        let mut next = [usize::MAX; NKEYS / KPB];
        for (b, at) in self.banks.iter_mut().zip(next.iter_mut()) {
            b.long_on = [false; KPB];
            b.a_done = 0;
            b.b_done = 0;
            if b.n_active > 0 {
                *at = 0;
            }
        }
        let mut n_long = 0;
        for &idx in &self.active {
            let v = &self.voices[idx];
            let b = &mut self.banks[idx / KPB];
            b.sync(idx % KPB, v);
            if v.longitudinal_on() && long_gain > 0.0 && n_long < MAX_LONG {
                n_long += 1;
                b.long_on[idx % KPB] = true;
            }
        }
        self.acc.clear(len);
        let kmask = self.knock_buf.len() - 1;

        // strings, hammers, bridge and soundboard
        for t in 0..len {
            for bi in 0..next.len() {
                if next[bi] == t {
                    let b = &mut self.banks[bi];
                    let vs = &mut self.voices[bi * KPB..(bi + 1) * KPB];
                    b.fixup(vs, &self.vz);
                    b.phase_a(t, b.blk.min(len - t), vs, long_gain, &mut self.acc);
                    next[bi] = b.a_done;
                }
            }
            // bridge velocities and forces
            let s: [f64; NZONES] = std::array::from_fn(|z| self.acc.s[t][z].sum() as f64);
            let h = &self.board.h;
            let mut vz = [0.0; NZONES];
            for i in 0..NZONES {
                let mut a = 0.0;
                for j in 0..NZONES {
                    a += self.p_mat[i][j] * symp * h[j] + self.q_mat[i][j] * s[j];
                }
                vz[i] = a;
            }
            let mut f = [0.0f32; NZONES];
            for i in 0..NZONES {
                let mut a = 2.0 * s[i] + self.acc.fl[t][i];
                for j in 0..NZONES {
                    a -= self.k_mat[i][j] * vz[j];
                }
                f[i] = a as f32;
                self.vz[i][t] = vz[i] as f32;
            }
            let mut fk = 0.0;
            if self.knock_left > 0 {
                fk = self.knock_buf[self.knock_pos];
                self.knock_buf[self.knock_pos] = 0.0;
                self.knock_left -= 1;
            }
            self.knock_pos = (self.knock_pos + 1) & kmask;
            let o = self.board.step(&f, fk);
            for c in 0..NOUT {
                self.rad[c][t] = o[c];
            }
        }
        for (bi, b) in self.banks.iter_mut().enumerate() {
            if b.n_active > 0 {
                b.fixup(&self.voices[bi * KPB..(bi + 1) * KPB], &self.vz);
            }
        }

        // per key: energy, the per-key correction of the partial levels (`morph.rs`)
        let (mut ml, mut mr) = ([0.0f32; CHUNK], [0.0f32; CHUNK]);
        let morph = self.morph as f32;
        for &idx in &self.active {
            let (b, k) = (&mut self.banks[idx / KPB], idx % KPB);
            let v = &mut self.voices[idx];
            v.account(b.take_energy(k) as f64, len, b.xk[len - 1][k] as f64);
            if v.morph_fade > 0.0 {
                let mut x = [0.0f32; CHUNK];
                for t in 0..len {
                    x[t] = b.xk[t][k];
                }
                v.run_morph(&x[..len], morph, &mut ml[..len], &mut mr[..len]);
            }
        }

        // The board is not baffled: at low frequencies the sound from
        // its two faces cancels progressively (first-order high-pass).
        // Far away that happens below ~100 Hz; the close microphones
        // sit in the near field of the upper face, where the path
        // around the rim is much longer than the direct one, and
        // keep the fundamentals of the bass.
        for c in 0..NOUT {
            let (g, mut st) = (self.dip_c[c], self.dip_s[c]);
            for o in self.rad[c][..len].iter_mut() {
                st += g * (*o - st);
                *o -= st;
            }
            self.dip_s[c] = st;
        }
        // The room: fed with the board's distant radiation and the strings' tone.
        let (rm, rs) = (self.room_modal as f32, self.room_tone as f32);
        let mut feed = [[0.0f32; CHUNK]; NF];
        for d in 0..NF {
            let sign = if d % 2 == 0 { rs } else { -rs };
            for t in 0..len {
                feed[d][t] = rm * self.rad[2 + d][t] + sign * self.acc.out[t][2].sum();
            }
        }
        let (mut wl, mut wr) = ([0.0f32; CHUNK], [0.0f32; CHUNK]);
        self.room.process(&feed, len, &mut wl, &mut wr);
        // The tone of the strings as radiated next to the bridge fills in
        // where the soundboard's modes happen to cancel at the microphones
        // (`voicing.rs`).
        let (bo, direct, room_amount, gain) = (self.board_out as f32, self.direct as f32, self.room_amount as f32, self.gain as f32);
        for t in 0..len {
            let l = bo * self.rad[0][t] + direct * self.acc.out[t][0].sum() + ml[t];
            let r = bo * self.rad[1][t] + direct * self.acc.out[t][1].sum() + mr[t];
            self.out_l[off + t] = soft_clip((l + room_amount * wl[t]) * gain);
            self.out_r[off + t] = soft_clip((r + room_amount * wr[t]) * gain);
        }

        // retire voices whose strings have come to rest
        let mut i = 0;
        while i < self.active.len() {
            let idx = self.active[i];
            let v = &mut self.voices[idx];
            if v.is_silent() && !v.keep_alive && !v.held {
                v.reset();
                let b = &mut self.banks[idx / KPB];
                b.clear_key(idx % KPB);
                b.set_active(idx % KPB, false);
                self.active.swap_remove(i);
                self.dirty = true;
                continue;
            }
            i += 1;
        }
        if self.active.is_empty() && self.pending.is_empty() && self.knock_left == 0 && self.board.quiet && self.room.quiet {
            self.board.reset();
            self.room.reset();
            self.dip_s = [0.0; NOUT];
                self.idle = true;
        }
    }
}
