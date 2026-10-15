# `pfx_core::sim` journal: a compact input log and headless verify

A run that replays from its seed (a leaderboard entry, a daily run, a 1v1, a racing ghost, a kill-cam) only needs its seed and the player's input on every tick. The journal stores exactly that, a few bytes a tick, in a format any program can read, and `verify` replays it headless and proves the run by its state digests.

pfx-play's `Recording` stays the engine's own log of a session. The journal is the small format a game ships, uploads and checks.

## Use

The game declares its per-tick input as a fixed list of fields, each a bit flag or an integer of a declared width:

```rust
use pfx_core::sim::journal::{Digest, Field, Journal, JournalWriter, Schema};

const SIM_VERSION: u32 = 4;

let schema = Schema::new(&[
    Field::Signed(16),
    Field::Signed(16),
    Field::Flag,
    Field::Flag,
])?;

let mut writer = JournalWriter::new(&schema, seed, SIM_VERSION, 600);
loop {
    let input = [aim_x, aim_y, fire as i64, jump as i64];
    game.step(&input);
    writer.push(&input, || game.digest())?;
}
let bytes = writer.finish();
```

`push` takes one tick's values, in schema order, after the game stepped them. The digest closure runs only on a checkpoint tick (every 600 here; 0 means no checkpoints), so a game computes its state digest ten times a minute, not sixty.

To check a run, a server or a client opens the bytes and replays them through the same sim:

```rust
let journal = Journal::open(&bytes)?;
let mut game = Game::new(journal.header().seed);
let last = journal.verify(&schema, SIM_VERSION, |input| {
    game.step(input);
    game.digest()
})?;
```

`verify` refuses a journal from another sim version, another rng stream version or another input schema before it steps anything, replays every tick, and stops with `JournalError::Checkpoint { tick, found, expected }` at the first checkpoint whose digest differs, so a forged or diverged run fails within one checkpoint interval. It returns the digest of the last tick (`None` for a journal with no ticks); the game compares it, or the state it reached, with the claimed result. `verify_sparse(schema, sim_version, |input, wanted| ...)` replays the same way but tells the step when its digest is read, on checkpoint ticks and the last tick only; on other ticks the value it returns is ignored, so a game with a costly digest computes it only there.

`JournalWriter::bytes()` (or `bytes_into(&mut buf)`, reusing the buffer) returns the journal so far without consuming the writer, the same bytes `finish` would return at that tick, so a game can flush a running journal every second and keep recording. Most of `bytes()` is the checksum over the whole file; `snapshot()` (or `snapshot_into(buf)`, reusing a buffer) copies the same bytes without it, a few microseconds at 25 minutes against about 200 µs, and hands back an `Unsealed` that a worker thread turns into the file with `seal()`, byte for byte what `bytes()` returns at that tick.

A running journal needs no copy of the writer to flush: `snapshot_into` borrows it, so the game thread keeps one buffer, takes a snapshot every second or so, and hands the `Unsealed` to a worker that seals it and replaces the file (write a temporary file, then rename it over the old one, so a crash leaves the last whole journal). The file cannot be appended to instead: the tick count, the checkpoint count and the stream length lead it, the checkpoint table follows the stream, and the checksum runs over every byte from the first, so each flush is a new file of the whole run, about 270 KB at 25 minutes (below).

`Digest` is FNV-1a 64 over little-endian bytes, the hash the format itself uses. A game builds its state digest with it from integers and from `sim`'s bit-exact outputs:

```rust
let mut digest = Digest::new();
digest.u64(self.tick).i64(self.score);
for body in &self.bodies {
    digest.i64(body.x).i64(body.y);
}
digest.value()
```

To read the inputs without a sim (a ghost, a kill-cam that starts mid-run, a tool):

```rust
let mut reader = journal.seek(tick)?;
while let Some(input) = reader.read()? {
    ghost.step(input);
}
```

`seek` starts at the last checkpoint at or before the tick and reads forward from there, so it never decodes more than one checkpoint interval it does not need. `journal.reader()` reads from the first tick.

## Fields

| field | values |
| --- | --- |
| `Flag` | 0 or 1 |
| `Unsigned(bits)`, 1 to 63 bits | `0 ..= 2^bits - 1` |
| `Signed(bits)`, 1 to 64 bits | `-2^(bits-1) ..= 2^(bits-1) - 1` |

A schema takes 1 to 63 fields. Values travel as `i64`. `push` refuses a value outside its field (`JournalError::Input { tick, field }`) or a wrong count (`InputLength`) and writes nothing for that tick. Analogue input is quantised by the game to integers in its own units (a stick at 1/127, a pointer in sub-pixels), which is what its sim steps on anyway. The width does not change the bytes a tick costs: only the change from the previous tick is written. It bounds what a reader accepts.

## Errors, never panics

`Journal::open` and `Reader::read` return `JournalError` for any input, whatever bytes arrive: too short (`Truncated`), not a journal (`Magic`), another format version (`Format`), a checksum that does not match (`Checksum`), or bytes that pass the checksum and still break the rules below (`Corrupt { at }`, the byte offset in the file). The tests cut a journal at every length, flip every byte, and overwrite every byte with several values behind a fixed checksum, and read and verify whatever opens.

## The bytes

All integers are little-endian. A file is a header, the field codes, the tick stream, the checkpoint table and a checksum:

| offset | size | field |
| --- | --- | --- |
| 0 | 4 | magic, the ASCII bytes `PFXJ` |
| 4 | 2 | `u16` format version, `1` |
| 6 | 2 | `u16` field count `F`, 1 to 63 |
| 8 | 8 | `u64` seed |
| 16 | 4 | `u32` the game's sim version |
| 20 | 4 | `u32` `sim::STREAM_VERSION` of the build that wrote it |
| 24 | 8 | `u64` schema hash |
| 32 | 8 | `u64` tick count `T` |
| 40 | 4 | `u32` checkpoint interval `N`, 0 for none |
| 44 | 4 | `u32` checkpoint count `C`, `T / N` (0 when `N` is 0) |
| 48 | 8 | `u64` tick stream length `L` |
| 56 | `F` | one code byte per field |
| 56 + F | `L` | the tick stream |
| 56 + F + L | 24 `C` | the checkpoint table |
| end - 8 | 8 | `u64` checksum |

The file is exactly `56 + F + L + 24 C + 8` bytes; a shorter one is truncated and a longer one corrupt.

**Field codes.** The top two bits are the kind, the low six the width minus one: `0x00` a flag, `0x40 | (bits - 1)` unsigned, `0x80 | (bits - 1)` signed. Any other byte (`0xc0` and up, a flag with width bits set, an unsigned width of 64) is corrupt.

**Hashes.** Both are FNV-1a 64: start at `0xcbf29ce484222325`, and for each byte `h = (h ^ byte) * 0x100000001b3` modulo 2^64. The schema hash runs over the field count as a `u16` followed by the `F` code bytes; a reader recomputes it and treats a mismatch as corrupt. The checksum runs over every byte before it.

**Varints.** Unsigned LEB128, as protobuf writes them: seven bits a byte, low group first, the top bit set on every byte but the last. At most ten bytes, the tenth no more than `0x01`, and no trailing zero group (a last byte of `0x00` after another byte): every value has one encoding. Signed deltas are zigzag-mapped first, `(d << 1) ^ (d >> 63)`, so 0, -1, 1, -2 become 0, 1, 2, 3.

**The tick stream.** Each tick is coded against the previous tick's values, all zeros before the first tick and again after every checkpoint. The stream is a sequence of records, each starting with a varint `h`:

- `h` even, a run: the next `(h >> 1) + 1` ticks repeat the previous values unchanged.
- `h` odd, a change: `m = h >> 1` is a mask of the fields that changed, bit `i` for field `i`. `m` is never zero and has no bit at or above `F`. Then, for each set bit from the lowest up, an integer field carries one varint, the zigzagged delta `new - old` in wrapping 64-bit arithmetic, never zero; a flag carries nothing and toggles. Each new value lies inside its field.

A run never spans a checkpoint boundary or the end. With checkpoints, tick `k N` (the first tick after checkpoint `k`) starts a new record exactly at that checkpoint's offset, coded against zeros. After `T` ticks the stream ends exactly at `L`.

So an unchanged tick inside a run costs nothing extra (a run of up to 64 ticks is one byte), a button press or release is one byte, and a pointer moving under 64 units a tick on both axes is three: the mask byte and one byte per axis.

**Checkpoints.** Entry `k` (from 0) is 24 bytes: `u64` tick `(k + 1) N`, the number of ticks applied; `u64` the game's state digest after that tick; `u64` the byte offset into the tick stream where the next tick's record starts (`L` for a checkpoint at `T`). Offsets never decrease.

**An example.** Fields `Unsigned(12)`, `Signed(8)`, `Flag`, seed `0x0102030405060708`, sim version 9, a checkpoint every 4 ticks, and seven ticks `[0,0,0]` three times, `[100,-3,1]`, `[101,-3,1]`, `[101,-3,0]`, `[101,-3,0]`, written with digest `0xd1d10003` at tick 4:

```
5046584a 0100 0300 0807060504030201 09000000 01000000   magic, format 1, 3 fields, seed, sim 9, stream 1
805330000586075a 0700000000000000                       schema hash, 7 ticks
04000000 01000000 0b00000000000000                      every 4, 1 checkpoint, 11 stream bytes
4b 87 00                                                Unsigned(12), Signed(8), Flag
04                                                      run of 3 unchanged ticks
0f c801 05                                              all three change: +100, -3, flag toggles
0f ca01 05                                              after the checkpoint, against zeros: +101, -3, toggle
09                                                      only the flag changes
00                                                      run of 1
0400000000000000 0300d1d100000000 0500000000000000      checkpoint: tick 4, digest, offset 5
74d8cfcc438c5867                                        checksum
```

`tests_journal.rs` pins these bytes; the hashes were checked against an independent FNV-1a.

**Versions.** The format version changes only with the bytes above; a reader refuses any other. The sim version is the game's own integer: it bumps it when a change to its sim would replay old inputs differently, and `verify` refuses another one. The stream version is `sim::STREAM_VERSION` (see `docs/sim.md`), refused when it is not the build's.

## Cost

Release build on a Ryzen 9 9950X: 25 minutes at 60 ticks (90,000 ticks) of a pointer that moves every tick (a random walk of up to 24 units a tick on a 3840×2160 field, bouncing at the edges) and four buttons that each toggle about once every 90 ticks, with a checkpoint every 600 ticks: 270,065 bytes, 3.00 bytes a tick; writing takes 0.015 µs a tick and opening (checksum included) and reading 0.013 µs a tick. An idle stretch costs one byte per 64 ticks.

## Prior art

- The varint and zigzag are protobuf's wire encoding (LEB128, and zigzag for signed values), the few lines every Rust varint crate (`integer-encoding`, `leb128`, both MIT or Apache-2.0) also carries; nothing else from them is needed, so pfx-core takes no dependency.
- Doom's `.lmp` demos store four fixed bytes per player per tic (forward, side, turn, buttons) after a header with the version, skill and map: the shape of a seed-plus-input log, without delta coding. Quake's `.dem` stores the server's messages, far bigger, and is not an input log.
- GGPO and rollback write-ups keep a fixed-size input per frame and compare it with the previous one to predict; the journal does the same comparison to choose between a run and a change record.
- The change mask plus per-field deltas follow the delta-compressed snapshots of Quake 3's network protocol, applied to the input alone.
