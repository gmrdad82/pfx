# `pfx_core::words`: a seeded text generator

NPC names, item and quest titles and barks for any game. A grammar is plain data a game fills (pools of entries, name to list), and `sim::Rng` drives every choice, so a seed names the same cast on every platform and the stream is frozen by a test.

The syntax follows Tracery's grammar (Apache-2.0; read, not copied): an entry is text with `{slot}` references to other pools, which may nest. Prior art checked: the `tracery` crate takes `rand` and hash maps (output not stable across platforms), and the name generators on crates.io are Markov chains over `rand`; none replays, so this is a small grammar over `sim::Rng`, with no dependency.

## Example

```rust
use pfx_core::sim::Rng;
use pfx_core::words::{PoolSpec, Words};

let words = Words::new(
    &[
        PoolSpec::new("first", ["Ada", "Bram", "Cleo", "Dov"]),
        PoolSpec::new("last", ["Stone", "Vale", "Marsh"]),
        PoolSpec::weighted("title", [("the Elder", 1), ("the Bold", 3)]),
        PoolSpec::new("name", ["{first} {last}", "{first} {last} {title}"]).avoiding(["Dov Vale"]),
        PoolSpec::new("bark", ["Stay back, {name}!", "{name|a|cap}? Never heard of one."]),
    ],
    ["Ada Stone"],
)?;
let rng = Rng::new(seed);
let one = words.generate("name", &rng, 0)?;
let cast = words.cast("name", &rng, 12)?;
```

## Templates

- `{pool}` expands that pool; entries may themselves hold slots, nested as deep as the pools go.
- `{pool|mod|mod}` applies modifiers left to right: `cap` (first letter), `title` (each word, after a space or hyphen), `upper`, `lower`, `a` (prepends "a" or "an": vowels, with `hour`/`honest`/`heir` taking "an" and `uni`/`use`/`one` taking "a").
- `{$name}` is a context slot: the game supplies its text at draw time with `generate_with(pool, rng, request, &[("company", "Acme"), ("contact", "Dov")])`. Modifiers apply (`{$company|upper}`); bindings, `once`, `sticky` and `hide` are refused on it. A context value draws nothing from the seed, so it shifts no other result. A draw that meets a context slot with no value given errors with `WordsError::MissingContext { name }` (`generate`, `cast` and `redraw` give none).
- `{{` and `}}` are a literal `{` and `}`.
- Weights: `PoolSpec::weighted` gives each entry a `u32` weight; 0 removes it.
- The same slot twice in one entry draws twice, independently.

## One person, several forms

`{boss=person}` draws the pool `person`, remembers it as `boss` and prints it. Later in the same entry `{boss}` prints the same text again, and `{boss.first}` prints the part that came from the `first` slot of the entry that was drawn, so a full name and a first name are one draw. `{boss=person|hide}` binds without printing. A form is any pool named by a direct slot of the bound pool's entries; at load every entry of the bound pool must hold that slot (`MissingForm` names the pool, the entry and the binding). A binding lasts for the entry that makes it, and a bound name shadows a pool of the same name from there on. Modifiers on a recall apply as usual (`{boss.first|upper}`); `once`, `sticky` and `hide` are refused there (`Misplaced`).

## Fixed for the run, fresh on every use

- `{theme|once}` draws that pool from the seed alone, not the request: every use in a run prints the same thing, and everything nested inside it is fixed too. A different seed picks differently.
- `{ladder|sticky}` fixes only which entry of `ladder` is taken (say the guild branch or the army branch) and draws what is inside it fresh on every use, so one run climbs one theme's titles.
- Both work with a binding (`{t=theme|once}`). They are refused together on one slot.

## Avoid

- `PoolSpec::avoiding([...])`: this pool never produces these texts (matched ignoring case, after expansion). A plain entry on the list is dropped at load; a template that expands to one is redrawn.
- A pool that builds the composite holds composite avoids: `PoolSpec::new("person", ["{first} {last}"]).avoiding(["Ada Stone"])` never returns that pair, whether drawn by `generate`, nested in another entry or bound with `{p=person}`.
- The second argument to `Words::new` lists whole results never returned by `generate` or `cast`, from any pool.
- Redraws stop after 32 attempts per level with `WordsError::Exhausted`.

## No repeats

`cast(pool, rng, n)` returns `n` distinct results, in draw order. `WordsError::NotEnoughDistinct { pool, wanted, got }` when the pool cannot: at once when `n` exceeds `capacity(pool)` (an upper bound: entries summed, slots multiplied), after a bounded number of redraws when duplicate entries or modifiers make the real count smaller.

`cast_distinct(pool, rng, n, &["first", "last"])` also keeps each listed pool's results out of two members: no repeated first name and no repeated last name across the cast, not just no repeated full name. It returns a `Cast` (the members' text plus the component text that was drawn, as it was printed, plain data a game can save; distinctness compares the components ignoring case). It errors with `NotEnoughDistinct` when `n` exceeds the capacity of the pool or of a listed pool.

## Re-drawing one member

`redraw(&mut cast, rng, index, counter)` replaces member `index` in place and returns its new text. The draw is keyed from the seed and `counter` (say a hire count) alone, so it does not depend on how many other draws happened or on any stream; the same seed, cast and counter give the same member. The new member differs from every current member, the one it replaces included, and shares no listed component with the others.

## Determinism

- `rng` is never advanced. Each pool visit forks it by a label made from the request number, the pool's name hash, the slot's position in its entry and the redraw attempt. Adding, removing or reordering other pools never shifts a result; changing a pool's own entries does shift that pool's.
- `generate(pool, rng, request)`: the request number names the result, so `request` 7 is the same text whenever it is asked for.
- Maps are `BTreeMap`; the only hash set (`cast`'s membership test) is never iterated. No floats, no clock.

## Load-time checks

`Words::new` returns `WordsError` naming the pool, the entry and the slot for: a duplicate pool, an empty pool, a malformed template (with the byte offset), a slot naming no pool, an unknown modifier, a pool that cannot produce anything after its avoid list and zero weights, and recursion. Recursion is any cycle among pools (`a -> b -> c -> a`), even one a weighted entry would usually escape: a grammar that can recurse is refused, not capped.

## Cost

Release build on the dev machine (Ryzen 9 9950X), `cargo test --release -p pfx-core --lib words::tests::bench -- --ignored --nocapture`: 20 pools, 2,000 entries, most entries with two or three slots nested up to four deep.

| work | time |
| --- | --- |
| 10,000 `generate` draws | 3.3 ms (about 330 ns each) |
| `cast` of 10,000 distinct names | 4.6 ms |
