//! Physically modelled piano.
//!
//! * `design`     – per-key physical parameters, loss / dispersion / tuning filter design
//! * `voice`      – one key: string design, nonlinear hammer, longitudinal force
//! * `bank`       – the waveguide strings of four keys at a time (the inner loop)
//! * `soundboard` – modal plate, two-way coupled to the strings, radiating to two microphones
//! * `room`       – early reflections and reverberation of the room
//! * `sustain`    – per-key lookup table applied to the decay rates (even sustain)
//! * `voicing`    – per-key lookup table: direct radiation of the strings (even blow / tone)
//! * `leveling`   – per-key loudness lookup table applied to the key velocity
//! * `morph`      – the keys between the reference keys: blends of their neighbours' tone
//! * `post`       – optional output stage (bass emphasis), independent of the model
//! * `simd`       – four-lane vector type of the inner loops
//! * `engine`     – 88 keys, pedal, mixing
//! * `wasm`       – C ABI used by the AudioWorklet

pub mod bank;
pub mod design;
pub mod engine;
pub mod leveling;
pub mod loudness;
pub mod morph;
pub mod post;
pub mod room;
pub mod simd;
pub mod soundboard;
pub mod sustain;
pub mod voice;
pub mod voicing;
pub mod wav;

#[cfg(target_arch = "wasm32")]
mod wasm;
