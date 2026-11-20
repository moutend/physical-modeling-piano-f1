# CPU cost

Machine: Apple M5 Max, rustc 1.95.0 (59807616e 2026-04-14), node v26.13.1

## Native build (engine/src/bin/bench.rs)

fs = 48000 Hz

init: 218.2 ms, soundboard modes: 465

| scenario | voices | strings | CPU (1 core), quanta in a row | CPU, one quantum per period |
|---|---|---|---|---|
| 1 note (C4) | 1 | 3 | 2.2% | 9.3% |
| 10-note chord | 10 | 28 | 4.7% | 17.1% |
| 10-note chord + pedal resonance | 88 | 240 | 5.8% | 23.0% |
| all 88 keys | 88 | 240 | 8.4% | 36.1% |
| all 88 keys + pedal | 88 | 240 | 8.4% | 36.0% |

fs = 96000 Hz

init: 213.0 ms, soundboard modes: 465

| scenario | voices | strings | CPU (1 core), quanta in a row | CPU, one quantum per period |
|---|---|---|---|---|
| 1 note (C4) | 1 | 3 | 4.2% | 18.6% |
| 10-note chord | 10 | 28 | 8.0% | 31.5% |
| 10-note chord + pedal resonance | 88 | 240 | 10.1% | 42.1% |
| all 88 keys | 88 | 240 | 14.6% | 58.8% |
| all 88 keys + pedal | 88 | 240 | 14.6% | 58.0% |

The second column is what an audio thread sees: it renders one quantum per
period and sleeps in between, and such a thread is run at low clock rates or
on efficiency cores. Confined to the efficiency cores (`taskpolicy -b`), as a
stand-in for a slow machine:

| scenario | voices | strings | CPU (1 core), quanta in a row | CPU, one quantum per period |
|---|---|---|---|---|
| 1 note (C4) | 1 | 3 | 9.8% | 9.7% |
| 10-note chord | 10 | 28 | 17.7% | 17.9% |
| 10-note chord + pedal resonance | 88 | 240 | 24.1% | 24.4% |
| all 88 keys | 88 | 240 | 38.6% | 38.1% |
| all 88 keys + pedal | 88 | 240 | 33.4% | 32.5% |

## WASM build under V8 (tools/bench-wasm.mjs)

"quantum max" includes scheduling noise of the benchmark machine; "quantum p99"
is the relevant figure against the render-quantum budget. The quanta are
rendered in a row here (see above for what that leaves out).

fs = 48000 Hz

init: 233.5 ms, wasm 263720 bytes
warm-up: 91.3 ms

| scenario | voices | strings | CPU (1 core) | quantum p99 | quantum max | budget |
|---|---|---|---|---|---|---|
| 1 note (C4) | 1 | 3 | 2.3% | 0.073 ms | 0.126 ms | 2.67 ms |
| 10-note chord | 10 | 28 | 4.5% | 0.142 ms | 0.206 ms | 2.67 ms |
| 10-note chord + pedal resonance | 88 | 240 | 5.8% | 0.179 ms | 0.215 ms | 2.67 ms |
| all 88 keys | 88 | 240 | 9.1% | 0.268 ms | 0.683 ms | 2.67 ms |
| all 88 keys + pedal | 88 | 240 | 9.1% | 0.269 ms | 0.634 ms | 2.67 ms |

| song (first 90 s) | most voices | CPU (1 core) | quantum p99 | quantum max | budget |
|---|---|---|---|---|---|
| debussy_Ste_Bergamesq_Clair.mid | 39 | 4.1% | 0.157 ms | 0.216 ms | 2.67 ms |
| chp-10-01.mid | 88 | 7.2% | 0.218 ms | 0.248 ms | 2.67 ms |
| pathetique-1.mid | 55 | 4.4% | 0.158 ms | 0.210 ms | 2.67 ms |
| KV331_3_RondoAllaTurca.mid | 43 | 4.1% | 0.139 ms | 0.816 ms | 2.67 ms |
| fur_Elise_WoO59.mid | 38 | 4.2% | 0.137 ms | 0.229 ms | 2.67 ms |

fs = 96000 Hz

init: 216.4 ms, wasm 263720 bytes
warm-up: 71.3 ms

| scenario | voices | strings | CPU (1 core) | quantum p99 | quantum max | budget |
|---|---|---|---|---|---|---|
| 1 note (C4) | 1 | 3 | 4.6% | 0.069 ms | 0.228 ms | 1.33 ms |
| 10-note chord | 10 | 28 | 8.5% | 0.131 ms | 0.215 ms | 1.33 ms |
| 10-note chord + pedal resonance | 88 | 240 | 10.8% | 0.166 ms | 0.229 ms | 1.33 ms |
| all 88 keys | 88 | 240 | 16.3% | 0.242 ms | 0.737 ms | 1.33 ms |
| all 88 keys + pedal | 88 | 240 | 16.3% | 0.241 ms | 0.715 ms | 1.33 ms |

| song (first 90 s) | most voices | CPU (1 core) | quantum p99 | quantum max | budget |
|---|---|---|---|---|---|
| debussy_Ste_Bergamesq_Clair.mid | 39 | 8.2% | 0.153 ms | 0.232 ms | 1.33 ms |
| chp-10-01.mid | 88 | 13.3% | 0.203 ms | 0.263 ms | 1.33 ms |
| pathetique-1.mid | 55 | 8.7% | 0.152 ms | 0.221 ms | 1.33 ms |
| KV331_3_RondoAllaTurca.mid | 43 | 8.2% | 0.137 ms | 0.185 ms | 1.33 ms |
| fur_Elise_WoO59.mid | 38 | 8.5% | 0.137 ms | 0.222 ms | 1.33 ms |
