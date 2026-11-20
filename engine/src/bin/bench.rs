//! CPU cost of the native build (the WASM build is measured by tools/bench-wasm.mjs).
//!
//!   bench [fs] [longitudinal amount]
//!
//! Every scenario is timed twice: with the 128-sample quanta rendered in a
//! row, and rendered one per period with the thread asleep in between, which
//! is what an audio thread does. The second figure is the one that decides
//! whether a machine keeps up: a thread that works for a fraction of every
//! period is run at low clock rates or on efficiency cores (on an Apple M5
//! it comes out about four times the first figure, the same as when the
//! whole process is confined to the efficiency cores with `taskpolicy -b`).

use piano_engine::design::*;
use piano_engine::engine::Engine;
use std::time::{Duration, Instant};

fn run(e: &mut Engine, secs: f64, paced: bool) -> f64 {
    let blocks = (secs * e.fs / 128.0) as usize;
    let period = Duration::from_secs_f64(128.0 / e.fs);
    let mut busy = Duration::ZERO;
    let start = Instant::now();
    for i in 0..blocks {
        if paced {
            let due = start + period * i as u32;
            let now = Instant::now();
            if due > now {
                std::thread::sleep(due - now);
            }
        }
        let t = Instant::now();
        e.process(128);
        busy += t.elapsed();
    }
    busy.as_secs_f64() / (blocks as f64 * 128.0 / e.fs)
}

fn main() {
    let fs: f64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(48000.0);
    let t = Instant::now();
    let mut e = Engine::new(fs);
    e.longitudinal = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    println!("init: {:.1} ms, soundboard modes: {}", t.elapsed().as_secs_f64() * 1e3, e.board.n);
    println!();
    println!("| scenario | voices | strings | CPU (1 core), quanta in a row | CPU, one quantum per period |");
    println!("|---|---|---|---|---|");
    let scenarios: [(&str, Vec<usize>, bool); 5] = [
        ("1 note (C4)", vec![60], false),
        ("10-note chord", vec![36, 43, 48, 52, 55, 60, 64, 67, 72, 76], false),
        ("10-note chord + pedal resonance", vec![36, 43, 48, 52, 55, 60, 64, 67, 72, 76], true),
        ("all 88 keys", (MIDI_LO..MIDI_LO + NKEYS).collect(), false),
        ("all 88 keys + pedal", (MIDI_LO..MIDI_LO + NKEYS).collect(), true),
    ];
    for (name, notes, pedal) in scenarios {
        let mut load = [0.0; 2];
        for (i, paced) in [false, true].into_iter().enumerate() {
            e.all_off();
            e.set_pedal(pedal);
            for &m in &notes {
                e.note_on(m, 0.8);
            }
            // (with the pedal down the strings join in over the first 15 ms)
            run(&mut e, 0.1, false);
            load[i] = run(&mut e, 3.0, paced);
        }
        println!("| {} | {} | {} | {:.1}% | {:.1}% |", name, e.active_voices(), e.active_strings(), load[0] * 100.0, load[1] * 100.0);
    }
}
