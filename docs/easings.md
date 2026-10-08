# Easings and tweens

`pfx_core::ease` holds the standard curves and `pfx_core::tween` drives a value along one. Time always comes from the caller; nothing reads a clock, so the same steps give the same bits.

## The curves

Every curve is `fn(f32) -> f32`, maps 0 to 0 and 1 to 1 exactly, and clamps its input to [0, 1]. The `Ease` enum names each one, and `Ease::at(t)` calls it.

| Family | In | Out | In-out | Shape |
|---|---|---|---|---|
| linear | `Linear` | | | straight |
| sine | `SineIn` | `SineOut` | `SineInOut` | gentle |
| quad, cubic, quart, quint | `QuadIn` … | `QuadOut` … | `QuadInOut` … | power 2, 3, 4, 5 |
| expo | `ExpoIn` | `ExpoOut` | `ExpoInOut` | exponential |
| circ | `CircIn` | `CircOut` | `CircInOut` | quarter circle |
| back | `BackIn` | `BackOut` | `BackInOut` | overshoots by about 10% |
| elastic | `ElasticIn` | `ElasticOut` | `ElasticInOut` | rings past the ends, up to about 37% |
| bounce | `BounceIn` | `BounceOut` | `BounceInOut` | stays in [0, 1] |

- The free functions have the same names in snake case (`sine_in`, `quart_in_out`, `bounce_out`, …). `smoothstep`, `smootherstep` and the older `quad_*` and `cubic_*` are re-exported from `motion` with their results unchanged.
- `Ease::ALL` lists all 31, `Ease::name()` and `Ease::from_name()` map to and from the snake-case name (`"elastic_in_out"`), `Ease::monotonic()` says whether a curve never steps back (not back, elastic or bounce), and `Ease::symmetric()` whether an in-out curve is point-symmetric about its middle.
- Expo and elastic jump a thousandth at their ends, as the standard formulas do; the endpoints are exact.

## Tweens

```rust
use pfx_core::ease::Ease;
use pfx_core::tween::{Repeat, Sequence, Tween};

let mut slide = Tween::new([0.0, 0.0, 0.0], [4.0, 1.0, 0.0], 0.5)
    .with_ease(Ease::BackOut)
    .with_delay(0.2)
    .with_repeat(Repeat::Count(3))
    .with_yoyo();
slide.step(dt);
let at = slide.value();
```

`Tween<T: Sample>` works on `f32` and `[f32; 3]`. It has `from`, `to`, `duration`, `ease`, `delay`, `repeat` and `yoyo`.

- `Repeat::Count(n)` plays n times (0 counts as 1) and `Repeat::Forever` never ends. The delay comes once, before the first play. With `yoyo`, every second play runs backward.
- `step(dt)` advances by the caller's time, ignoring a `dt` that is not finite and positive. `value()` reads the current value, `finished()` says it is done, and `length()` is `delay + count * duration`, or `None` for forever.
- `at(seconds)` reads any moment without moving. `seek(seconds)` jumps, `reset()` goes back to the start, and `time()` is how far it has run.
- Time accumulates in `f64` and snaps to the end within a microsecond, so 60 steps of 1/60 give exactly `at(1.0)`, bit for bit, and so does any other frame rate that adds up to the duration.
- Before the delay ends a tween holds `from`; after it ends it holds the last play's end (`to`, or `from` after an even number of yoyo plays).

`Sequence<T>` chains tweens end to start: `Sequence::new().then(a).then(b)`. It has the same `step`, `value`, `at`, `seek`, `reset`, `finished` and `length`, plus `current()` for the index of the running tween. Each tween keeps its own delay, repeats and yoyo. A forever tween holds the sequence there, so what follows it never runs. An empty sequence rests at the type's default.
