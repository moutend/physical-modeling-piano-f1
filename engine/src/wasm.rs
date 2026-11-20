//! Minimal C ABI for the AudioWorklet (no wasm-bindgen needed).

use crate::engine::Engine;
use crate::post::BassBoost;
use core::ptr::null_mut;

static mut ENGINE: *mut Engine = null_mut();
/// Output stage after the model (see `post.rs`); not part of the engine.
static mut BASS: *mut BassBoost = null_mut();

#[inline]
fn eng() -> &'static mut Engine {
    unsafe { &mut *ENGINE }
}

#[no_mangle]
pub extern "C" fn piano_init(sample_rate: f32) {
    unsafe {
        if !ENGINE.is_null() {
            drop(Box::from_raw(ENGINE));
        }
        ENGINE = Box::into_raw(Box::new(Engine::new(sample_rate as f64)));
        if !BASS.is_null() {
            drop(Box::from_raw(BASS));
        }
        BASS = Box::into_raw(Box::new(BassBoost::new(sample_rate as f64)));
    }
}

#[no_mangle]
pub extern "C" fn piano_note_on(midi: u32, velocity: f32) {
    eng().note_on(midi as usize, velocity as f64);
}

#[no_mangle]
pub extern "C" fn piano_note_off(midi: u32) {
    eng().note_off(midi as usize);
}

#[no_mangle]
pub extern "C" fn piano_pedal(down: u32) {
    eng().set_pedal(down != 0);
}

#[no_mangle]
pub extern "C" fn piano_all_off() {
    eng().all_off();
}

/// 0: master gain, 1: sympathetic amount (0..1), 2: unused (was: pedal
/// resonance on/off; it is always on now), 3: room amount, 4: hammer
/// hardness, 5: key noise amount,
/// 6: longitudinal string force amount, 7: per-key loudness levelling (0/1),
/// 8: bass emphasis of the output stage (0..1), 9: reference pitch of A4 [Hz].
#[no_mangle]
pub extern "C" fn piano_set_param(id: u32, value: f32) {
    let e = eng();
    match id {
        0 => e.gain = value as f64,
        1 => e.sympathetic = value as f64,
        3 => e.room_amount = value as f64,
        4 => e.set_hardness(value as f64),
        5 => e.knock = value as f64,
        6 => e.longitudinal = value as f64,
        7 => e.leveling = value != 0.0,
        8 => unsafe { (*BASS).set_amount(value as f64) },
        9 => e.set_reference_pitch(value as f64),
        _ => {}
    }
}

#[no_mangle]
pub extern "C" fn piano_process(n: u32) {
    let e = eng();
    e.process(n as usize);
    let n = (n as usize).min(e.out_l.len());
    unsafe { (*BASS).process(&mut e.out_l[..n], &mut e.out_r[..n]) };
}

#[no_mangle]
pub extern "C" fn piano_out_l() -> *const f32 {
    eng().out_l.as_ptr()
}

#[no_mangle]
pub extern "C" fn piano_out_r() -> *const f32 {
    eng().out_r.as_ptr()
}

#[no_mangle]
pub extern "C" fn piano_active_voices() -> u32 {
    eng().active_voices() as u32
}

#[no_mangle]
pub extern "C" fn piano_active_strings() -> u32 {
    eng().active_strings() as u32
}

/// Largest gain reduction of the output limiter since the last call [dB].
#[no_mangle]
pub extern "C" fn piano_limiter_db() -> f32 {
    unsafe { (*BASS).take_reduction() as f32 }
}
