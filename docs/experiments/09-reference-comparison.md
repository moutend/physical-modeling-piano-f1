# Comparison with the reference recording (fs = 48 kHz)

Reference: `wave/music2.wav` (the performance of `midi/music2.mid`; all notes at
MIDI velocity 80, no pedal, 2421 notes, keys 31-98). The engine renders the
same file (`render midi`), and both signals go through the same analysis.
"ref" is the recording, "syn" the engine. Levels are relative to each
signal's own total power, spectra are the power mean of L and R.

Alignment: the recording starts 0.345 s after the file start, tempo ratio
1.000. `midi/music1.mid` is byte-identical to `music2.mid` and does not match
`wave/music1.wav` (262.5 s), which was only used for its long-term spectrum
(within about 3 dB of `music2.wav` in most bands up to 4 kHz).

Adjacent keys of the recording share partial levels to 0.1-0.3 dB (69-72,
77-79, 80-82, 45-46, 47-48, ...), and the lead-in is digital silence
(-99 dBFS): the recording most likely comes from a sampled instrument whose
samples each cover a few keys. Targets were therefore taken per register, not
per key.

## Before (commit f024ac1)

```
LTAS syn-ref (dB, each rel. to own total):
   62    79   100   125   158   200   251   317   400   503   634   800  1007  1269  1600  2015  2539  3200  4031  5079  6400  8063 10159 12800
 -1.3  -5.7  -6.8   0.6  -1.2  -1.1  -0.6   0.2  -0.8  -2.2  -0.1   0.9   3.0   2.9   8.2   3.8   5.2   1.5   2.9   7.1  14.1   7.3  13.5  -3.8

onset partial level syn-ref (dB) by keys x partial no.   [+ = synth too strong]
keys         1      2      3      4    5-6    7-9  10-13  14-19  20-30
31-42     -6.1   -4.1   -0.3   -1.3   -1.3    7.8    4.8    3.7    3.7
43-48     -8.4   -3.3    2.2    0.2    1.2    1.3    4.9    7.2    8.3
49-54     -0.5   -2.2   -6.4    0.7    1.7    1.9    7.5   10.0   11.7
55-60     -0.7   -1.7   -6.6   -0.4    2.0   -1.7    4.3    6.6   11.3
61-66     -3.9  -13.1    5.2    0.8    7.2   -0.0    0.7    7.2    4.1
67-72     -4.3   -4.1    7.5    8.9    9.7   -0.9    4.2    0.3   14.9
73-78     -2.7   -0.4    6.8    9.7    2.8    2.5    6.2   -2.0      .
79-84      1.1    4.6      .      .   -6.0    0.2    6.4      .      .
85-98      0.8      .    1.3   24.9   12.1   14.1      .      .      .
by freq 60:-7.9 125:-2.3 250:-2.0 500:1.1 1000:4.5 2000:6.4 4000:6.0

onset-locked band envelopes (dB re total) ref / syn
                 -50    -5     0     2     4     6    10    20    30    50    80   120   180
60-125   ref   -11.3 -10.5 -11.0 -10.9 -11.6 -10.3  -9.9  -8.9  -8.1  -7.9  -9.8 -11.4 -11.0
         syn   -15.8 -16.9 -16.0 -15.0 -14.6 -15.3 -14.8 -14.2 -14.2 -15.5 -15.5 -16.2 -16.1
125-250  ref    -8.5  -9.3  -8.8  -8.0  -7.2  -6.2  -5.1  -4.4  -4.4  -5.2  -6.4  -7.5  -8.3
         syn    -7.3  -8.1  -7.7  -6.8  -6.5  -6.7  -7.6  -6.2  -6.0  -6.5  -7.1  -6.7  -7.9
250-500  ref    -8.6  -9.1  -8.6  -7.2  -7.2  -6.8  -5.5  -4.7  -4.5  -5.3  -5.9  -7.1  -8.9
         syn    -8.0  -8.7  -8.5  -7.9  -7.6  -6.5  -5.9  -6.3  -6.5  -6.8  -7.1  -7.4  -7.8
500-1k   ref    -6.7  -8.7  -7.8  -6.6  -5.1  -4.8  -3.2  -2.7  -2.9  -3.9  -4.6  -6.0  -7.8
         syn    -5.6  -6.4  -6.2  -5.7  -5.1  -3.8  -3.4  -3.6  -3.7  -4.2  -4.4  -5.7  -6.2
1k-2k    ref   -14.5 -16.0 -12.4 -10.1  -9.7 -10.3  -9.7 -10.2 -10.2 -11.2 -13.0 -14.0 -15.5
         syn    -8.7 -10.1 -10.3  -8.3  -6.7  -6.1  -5.7  -6.3  -6.7  -7.1  -7.6  -8.9  -9.7
2k-4k    ref   -26.5 -27.3 -24.6 -23.0 -22.0 -21.5 -20.9 -20.9 -21.5 -22.3 -23.9 -25.2 -27.3
         syn   -22.3 -24.0 -24.1 -18.5 -16.6 -16.5 -16.4 -17.4 -17.5 -19.1 -19.4 -21.0 -23.5
4k-8k    ref   -45.4 -48.0 -41.4 -39.1 -37.6 -37.3 -37.0 -38.0 -39.4 -41.6 -44.0 -44.5 -47.2
         syn   -39.2 -39.4 -39.3 -29.2 -27.3 -28.9 -29.0 -31.7 -33.5 -35.3 -37.0 -39.1 -40.9
8k-16k   ref   -76.1 -75.3 -63.4 -61.9 -61.3 -61.8 -62.6 -63.8 -64.0 -68.8 -70.7 -74.5 -76.0
         syn   -68.8 -75.4 -73.4 -49.6 -48.5 -51.9 -52.9 -57.0 -58.2 -60.5 -63.6 -67.2 -73.5

tail after final key-up: ref  -6.3 -13.6 -19.8 -26.1 -37.8 -55.2 -71.8 -81.6
                         syn  -5.2 -17.9 -20.9 -25.9 -36.6 -50.5 -61.5 -65.6
held chord (85.74 s):    ref   2.8   0.7  -1.5  -3.7  -5.7  -7.4  -8.6  -9.8
                         syn   2.6   3.1   2.0   1.4   0.3  -0.7  -1.7  -2.7
```

## After

```
LTAS syn-ref (dB, each rel. to own total):
   62    79   100   125   158   200   251   317   400   503   634   800  1007  1269  1600  2015  2539  3200  4031  5079  6400  8063 10159 12800
  7.0   0.2  -2.1   2.9   4.0   0.8  -0.4  -0.3  -1.8  -5.3  -3.7  -1.3   1.7   2.0   4.3   0.3   4.4   0.4   0.3   3.3  12.0   8.6   4.9  -1.1

onset partial level syn-ref (dB) by keys x partial no.   [+ = synth too strong]
keys         1      2      3      4    5-6    7-9  10-13  14-19  20-30
31-42     -0.7    1.5    2.7   -3.7   -4.2   10.1   -0.3    1.8   -6.0
43-48     -1.1   -0.3    1.0    3.4   -0.0    5.1    0.7    0.1    0.5
49-54      6.0   -9.7   -7.8   -1.5    3.0   -0.8   -3.6   -4.4   -5.5
55-60      3.7   -6.2   -3.0    2.7    0.5   -7.3   -3.8   -4.3   -8.2
61-66     -2.8  -12.5   -7.1    2.9    2.9   -4.7   -2.3   -7.7  -14.8
67-72     -6.8   -3.8    6.6    4.2    7.2   -9.8   -8.1  -13.2  -16.6
73-78     -5.3    8.3    8.5   11.3   -6.0    4.1   -0.2  -21.0      .
79-84     -3.5   -0.7      .      .   -6.4  -18.0  -15.7      .      .
85-98     -1.8      .   -0.3   20.6    4.1  -11.2      .      .      .
by freq 60:-2.0 125:1.1 250:-1.9 500:-1.5 1000:-0.3 2000:-3.9 4000:-7.1

initial decay dB/s ref|syn (first 100 ms; in brackets notes>=0.38s over 280 ms)
31-42    -49| -13 [ -37|  -2]  -17| -18 [ nan| nan]   -8|  -4 [  -9| -17]   -8|  -2 [  -6| -20]
43-48    -43|  -7 [ -47| -21]  -10| -11 [ -10| -10]  -22|  -0 [ -10|  -6]   -6|  -5 [ nan| nan]
49-54    -16| -23 [ nan| nan]  -19|  45 [ nan| nan]  -26|  -3 [ nan| nan]  -16|  -5 [ nan| nan]
55-60     -3| -22 [ nan| nan]  -17| -60 [ nan| nan]  -31| -13 [ nan| nan]  -32| -37 [ nan| nan]
61-66    -31| -23 [ -28| -16]  -21|   7 [ nan| nan]  -27| -54 [ nan| nan]   -9| -67 [ nan| nan]
67-72    -38| -29 [ -35| -26]  -14| -21 [ -27| -39]   -7| -21 [ nan| nan]  -44| -66 [ nan| nan]
73-78    -25| -17 [ -23| -14]  -44| -42 [ nan| nan]  -61| -22 [ nan| nan]  -43| -84 [ -53| -85]
79-84    -51| -19 [ -34| -34]  -50| -50 [ nan| nan]  -80| -45 [ nan| nan]  -75| -88 [ -49| -76]
85-98    -50| -46 [ nan| nan]  nan| nan [ nan| nan] -152| -84 [ nan| nan]  -85|-116 [ nan| nan]
by freq: 60: -17|-6 125: -16|-12 250: -23|-2 500: -14|-14 1000: -18|-14 2000: -24|-58 4000: -56|-88

release drop (dB, level 30..115 ms after key-up rel. to last 85 ms held) ref|syn, partials 1-4
31-42: -7|-7  43-48: -6|-7  49-54: -6|-10  55-60: -6|-5  61-66: -7|-8  67-72: -9|-12  73-78: -7|-8  79-84: -10|-10  85-98: nan|nan

onset-locked band envelopes (dB re total) ref / syn
                 -50    -5     0     2     4     6    10    20    30    50    80   120   180
60-125   ref   -11.3 -10.5 -11.0 -10.9 -11.6 -10.3  -9.9  -8.9  -8.1  -7.9  -9.8 -11.4 -11.0
         syn   -11.8 -11.8  -9.6 -10.7  -8.4  -8.1  -7.7  -8.8  -7.7  -8.9 -10.8 -10.9 -12.7
125-250  ref    -8.5  -9.3  -8.8  -8.0  -7.2  -6.2  -5.1  -4.4  -4.4  -5.2  -6.4  -7.5  -8.3
         syn    -5.4  -6.4  -6.3  -5.8  -4.9  -3.6  -4.2  -2.7  -2.3  -3.0  -4.6  -4.1  -6.6
250-500  ref    -8.6  -9.1  -8.6  -7.2  -7.2  -6.8  -5.5  -4.7  -4.5  -5.3  -5.9  -7.1  -8.9
         syn    -9.2  -9.8  -9.4  -9.6  -8.4  -7.5  -6.9  -6.2  -6.5  -7.0  -8.1  -8.3  -9.2
500-1k   ref    -6.7  -8.7  -7.8  -6.6  -5.1  -4.8  -3.2  -2.7  -2.9  -3.9  -4.6  -6.0  -7.8
         syn    -8.6  -9.8  -9.6  -9.4  -7.0  -6.4  -6.0  -6.1  -6.8  -7.2  -7.7  -8.2  -8.6
1k-2k    ref   -14.5 -16.0 -12.4 -10.1  -9.7 -10.3  -9.7 -10.2 -10.2 -11.2 -13.0 -14.0 -15.5
         syn   -11.9 -13.2 -13.7 -12.8  -7.6  -7.5  -6.1  -7.2  -7.7  -8.3  -9.5 -11.0 -12.7
2k-4k    ref   -26.5 -27.3 -24.6 -23.0 -22.0 -21.5 -20.9 -20.9 -21.5 -22.3 -23.9 -25.2 -27.3
         syn   -25.9 -28.1 -28.1 -24.8 -19.6 -19.1 -18.4 -19.1 -19.6 -20.3 -22.2 -24.7 -28.2
4k-8k    ref   -45.4 -48.0 -41.4 -39.1 -37.6 -37.3 -37.0 -38.0 -39.4 -41.6 -44.0 -44.5 -47.2
         syn   -41.2 -46.0 -45.9 -38.2 -30.4 -30.6 -31.5 -34.4 -37.3 -38.3 -39.8 -41.6 -46.0
8k-16k   ref   -76.1 -75.3 -63.4 -61.9 -61.3 -61.8 -62.6 -63.8 -64.0 -68.8 -70.7 -74.5 -76.0
         syn   -75.1 -74.6 -76.0 -61.4 -54.9 -56.4 -58.1 -61.4 -62.3 -65.7 -66.5 -68.8 -74.6

tail after final key-up: ref  -6.3 -13.6 -19.8 -26.1 -37.8 -55.2 -71.8 -81.6
                         syn  -0.3  -9.2 -14.0 -19.7 -33.1 -50.2 -67.7 -79.0
held chord (85.74 s):    ref   2.8   0.7  -1.5  -3.7  -5.7  -7.4  -8.6  -9.8
                         syn   4.9   2.9   1.9  -0.2  -2.5  -4.6  -6.7  -8.8
```

Reading the tables

* LTAS: third-octave long-term spectrum, engine minus recording.
* onset partial level: level of each note's partials in the 10-95 ms after its
  onset (median over the notes of a key range whose partial is not masked by
  another note), engine minus recording. The partial search follows each
  model's own inharmonicity ("before" uses the old coefficients).
* initial decay: slope of those partials over the first 100 ms (in brackets:
  over 280 ms, notes of 0.38 s or longer). Few notes are long, so these
  medians are noisy.
* onset-locked band envelopes: band power around the note onsets (ms),
  averaged over 1051 onsets.
* tail: broadband level from 50 ms before to 1.4 s after the last key-up;
  held chord: level through a 0.7 s stretch without new onsets.

## Tuning and inharmonicity measured on the recording

| key | B measured | B model before | B model after | pitch vs ET, recording [cent] |
|---|---|---|---|---|
| C2 (36) | 1.03e-4 | 1.49e-4 | 9.7e-5 | -10.4 |
| F#2 (42) | 7.8e-5 | 1.25e-4 | 8.8e-5 | -4.3 |
| C3 (48) | 1.11e-4 | 1.33e-4 | 1.09e-4 | -5.3 |
| F#3 (54) | 1.71e-4 | 1.74e-4 | 1.63e-4 | -4.5 |
| C4 (60) | 2.92e-4 | 2.59e-4 | 2.64e-4 | -1.0 |
| A4 (69) | 6.40e-4 | 5.19e-4 | 5.70e-4 | +0.3 |
| A5 (81) | 1.73e-3 | 1.37e-3 | 1.61e-3 | +6.1 |
| F#6 (90) | 3.01e-3 | 2.86e-3 | 3.53e-3 | +14.8 |

The same measurement applied to the engine's own output reads B about 10 %
high around C4, so the model values were set slightly below the raw numbers.

## Checks after the change

* All 88 keys fff with the pedal down: monotonic decay at 44.1 / 48 / 96 kHz
  (-6 dB at 0 s, -52 dB at 20 s, -114 dB at 58 s).
* Fundamental of every third key, measured from the output: within 1.6 cent
  of the design target.
