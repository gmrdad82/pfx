# `pfx_core::numbers`: number text for games

Damage numbers, scores and idle-game counters: `Format` turns an integer (`i8` to `i128`, `u8` to `u128`, `isize`, `usize`) or a float (`f32`, `f64`) into text, the same text for the same value on every platform. It is presentation, not simulation, but nothing in it depends on the platform: no float arithmetic, no libm, no locale. A number becomes decimal digits first (an integer exactly, a float as its shortest round-trip digits, the ones Rust's `{:e}` prints), and rounding, grouping and the style work on those digits.

```rust
use pfx_core::numbers::{Format, Precision, Style};

let f = Format {
    style: Style::Short,
    precision: Precision::Decimals(2),
    trim_zeros: true,
    ..Format::default()
};
assert_eq!(f.format(1_500).unwrap(), "1.5K");
assert_eq!(f.format(4_000_000u64).unwrap(), "4M");

let mut line = String::new();
Format::default().write(&mut line, 1234567).unwrap();
assert_eq!(line, "1,234,567.00");
```

`write(&mut impl fmt::Write, n)` allocates nothing (digits live on the stack); `format(n)` returns a `String`. Both return `Result<_, NumberError>`: it fails only for the `Error` non-finite policy or a writer that refuses.

## The fields

| field | default | meaning |
| --- | --- | --- |
| `style` | `Plain` | what a number at or past `from_exp` becomes: `Plain`, `Scientific`, `Engineering` or `Short` |
| `from_exp` | `0` | the decimal exponent (the digit count minus one) from which the style applies; below it the number is plain. `Short` never starts below 3. `from_exp: 12` keeps `999,999,999,999` grouped; `4` starts `Short` at 10,000 |
| `sci_from` | `None` | for `Short`: the exponent from which it becomes scientific instead (`Some(12)`: `999B`, then `1.2e12`) |
| `precision` | `Decimals(2)` | for plain numbers: `Decimals(d)` fixed places after the point; `Significant(s)` that many digits (0 counts as 1); `AtLeast(s)` that many digits but never fewer integer digits (`s` = 2 gives `1.2`, `12`, `123`) |
| `big_precision` | `None` | the same choice for the numbers past `from_exp` (scientific, engineering, short); `None` reuses `precision`. This is how plain stays exact (`Decimals(0)`) while `1.0e12` has two significant digits |
| `rounding` | `HalfAway` | `HalfAway` (2.5 is 3, -2.5 is -3), `HalfEven` (2.5 is 2, 3.5 is 4), `Ceil` (toward +infinity: 0.0001 is 1 with no decimals, so a live count never reads 0; -2.9 is -2) or `Truncate` (toward zero, never carries: 999,999 is `999K`, not `1.0M`) |
| `trim_zeros` | `false` | drop trailing zeros of the fraction, and the mark when nothing is left |
| `group_sep`, `group_size` | `Some(',')`, `3` | digit grouping of the plain integer part; `None` or size 0 turns it off. `Some(' ')` gives `1 234 567` |
| `decimal_mark` | `'.'` | the mark before the fraction |
| `suffixes` | `Named(SHORT_NAMES)` | for `Short`: `Named(&[...])` starts at 10^3 (the default list is `K M B T`), `Alpha` is `K M B T` then `aa`, `ab` .. `zz`, `aaa` .. |
| `non_finite`, `nan_text`, `inf_text` | `Text`, `"NaN"`, `"inf"` | see below |

Styles, all on the rounded digits:

- `Scientific`: `1.23e15`, mantissa 1 to 9.99, exponent as is.
- `Engineering`: `12.3e15`, exponent a multiple of 3, mantissa 1 to 999.
- `Short`: `1.23K`, `4.5M`, `1.00aa`. When the list ends (or `Named` is short), the number falls back to scientific with the same precision (`2.50e9`).
- Rounding can carry into the next step and the style follows: 999.96 with one decimal is `1.0K` in `Short`, 999999.999 is `1.00e6` when scientific starts at 6.
- Zero is always plain. A negative number that rounds to zero loses its sign (`-0.001` is `0.00`), and `-0.0` prints as `0.00`.

## Two styles a game asks for

```rust
let long = Format {
    style: Style::Scientific,
    from_exp: 12,
    precision: Precision::Decimals(0),
    big_precision: Some(Precision::Significant(2)),
    rounding: Rounding::Truncate,
    ..Format::default()
};
// 999,999,999,999 then 1.0e12

let card = Format {
    style: Style::Short,
    from_exp: 4,
    sci_from: Some(12),
    precision: Precision::Decimals(0),
    big_precision: Some(Precision::AtLeast(2)),
    rounding: Rounding::Truncate,
    ..Format::default()
};
// 9,999  12K  3.4M  999B  1.2e12  (trim_zeros turns 3.0M into 3M)
```

## Rounding

Integers are exact, all 39 digits of `u128::MAX` included. A float is rounded as its shortest round-trip decimal, not as its exact binary value, so `2.675` with two decimals is `2.68` (half away) and `2.665` is `2.66` (half even), the way a player reads them. `f32` uses `f32`'s own shortest digits (`0.1f32` is `0.1`, not `0.10000000149`). Asking for more places than the digits have pads zeros.

## NaN and infinity

- `OneText`: every non-finite value prints `nan_text`, never `inf`.
- `Text`: `nan_text` for NaN, `inf_text` for infinity with a `-` in front for negative (`"-∞"`). No rounding or grouping.
- `Clamp`: infinity becomes the largest finite value of its type (`f64::MAX`, `f32::MAX`) with its sign, NaN becomes 0.
- `Error`: `NumberError::NotANumber` or `NumberError::Infinite`, so a game that forbids showing them catches it.

## Cost

Bench `cargo test --release -p pfx-core --lib numbers::tests::bench_styles -- --ignored --nocapture`: 100,000 integers (1 to 63 bits) and 100,000 floats (1e-6 to 1e24), 4 significant digits, trailing zeros trimmed, into a reused `String`. Measured on a machine loaded far past its cores (load average about 85 on 32 threads), so these are upper bounds:

| style | ns per call |
| --- | --- |
| plain | 217 |
| scientific | 206 |
| engineering | 262 |
| short | 234 |

An earlier run of the same code on a quieter machine gave 123 to 160 ns for the three big styles and 360 for plain (grouped output is the longest).
