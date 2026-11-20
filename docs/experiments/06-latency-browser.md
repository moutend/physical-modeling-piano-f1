# Latency measured in the browser

Environment: Apple M5 Max, macOS (Darwin 25.6), Chromium 156 (Claude desktop
in-app browser), system default output device (reports 96 kHz), 2026-11-15.

Method (implemented in `web/main.js`):

* every note-on message carries an id; the AudioWorklet replies with the frame
  number of the render quantum in which the note starts sounding;
* `AudioContext.getOutputTimestamp()` maps that frame to the `performance.now()`
  time at which it leaves the output device;
* "key event -> speaker" = that time minus the event timestamp;
* "round trip" = time from `postMessage` in the handler until the worklet's reply
  is received on the main thread (an upper bound for handler -> render start).

Notes were triggered from script (`performance.now()` as the event time), 40-50
notes per configuration, so the figures exclude the keyboard's own scan / USB
delay and the OS input stack. `getOutputTimestamp().performanceTime` has 1 ms
resolution in this browser.

## latencyHint and sample rate

| latencyHint | sample rate | render quantum | baseLatency | outputLatency | render -> speaker (measured) | round trip median / p95 | key event -> speaker median / p95 | underruns |
|---|---|---|---|---|---|---|---|---|
| playback | 96 kHz (device) | 1.33 ms | 21.33 ms | 40 ms | 40.3 ms | 14.2 / 20.2 ms | 54.8 / 60.8 ms | 0 |
| balanced | 96 kHz (device) | 1.33 ms | 10.00 ms | 32 ms | 33.3 ms | 4.4 / 10.6 ms | 34.6 / 40.5 ms | 0 |
| interactive | 96 kHz (device) | 1.33 ms | 2.67 ms | 24 ms | 22.8 ms | – | – | – |
| 0 (app default) | 96 kHz (device) | 1.33 ms | 2.67 ms | 24 ms | 22.8 ms | 1.4 / 2.5 ms | 23.3 / 24.5 ms | 0 |
| 0 | 48 kHz | 2.67 ms | 2.67 ms | 24 ms | 22.1 ms | 1.2 / 2.6 ms | 23.2 / 24.5 ms | 0 |
| 0 | 44.1 kHz (resampled) | 2.90 ms | 2.90 ms | 24 ms | 24.5 ms | 1.4 / 3.0 ms | 25.0 / 26.7 ms | 0 |

(`interactive` was only probed with a silent test context: it yields the same
buffer sizes as `0` in this browser.)

Observations:

* Of the ~23 ms, about 22-23 ms is the output path after rendering (browser
  output buffer + device), which the page cannot shorten further; the part the
  app controls (handler -> start of rendering) is 1-3 ms, i.e. at most about one
  render quantum plus thread hand-over.
* `latencyHint` matters most: `playback` costs +31 ms and `balanced` +11 ms
  compared with `interactive` / `0`.
* Running at a rate other than the device rate adds a resampler (44.1 kHz:
  +1.7 ms here), so the app uses the device rate by default.

## Start-up underruns and first-note stalls

| situation | before | after |
|---|---|---|
| output underruns while the engine initialises (0.2 s of filter design on the audio thread) | 106-107 underrun events, 0.28 s | 0 (context suspended until the worklet reports ready) |
| longest 128-frame block of the very first note (WASM, V8, 48 kHz; JIT tier-up) | 15.2 ms (budget 2.67 ms) | 0.12-0.17 ms (warm-up pass during the first 160 muted quanta) |
| longest block when all 88 keys are struck in the same quantum (WASM, 96 kHz, budget 1.33 ms) | 2.1-2.5 ms at 48 kHz, 3 underrun events at 96 kHz in the browser | 0.75 ms, 0 underruns (felt lookup table + warm-started Newton solver) |

## Stress test in the browser

All 88 keys struck simultaneously with the pedal down at 96 kHz (240 strings
sounding): 0 underrun events over 4 s (`AudioContext.playbackStats`).
