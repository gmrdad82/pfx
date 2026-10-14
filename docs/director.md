# `pfx_core::sim::director`: what to spawn and when, from data

A deterministic spawn director for wave shooters, an FPS horde mode, or encounters in an adventure. It decides what to spawn, how many, how fast and where; the game spawns it and tells the director when something dies. It follows `sim`'s promise (docs/sim.md): integers only, `sim::Rng` forked by label, no wall clock, no hash-map order, so the same schedule, seed and calls give the same bytes on every platform.

The model is the one the public write-ups of Left 4 Dead's AI director describe: a pace set by phases, quiet windows after bursts or a cleared wave, and a budget of what may be alive at once. No code from any game was read.

## The data

All plain public types a game fills from its own TOML, JSON or code.

```rust
use pfx_core::sim::director::{Director, Entry, Lull, Phase, Ramp, Region, Schedule, SpawnLull, Spots, Table, ONE};

let mut table = Table::new(vec![
    Entry::new(1, 6).tagged(0b01),
    Entry::new(2, 3).tagged(0b01).costing(2),
    Entry::new(3, 1).tagged(0b10).costing(5),
]);
table.cap = 20;

let mut wave = Phase::new(0, 3_600, 0);
wave.rate = Ramp::linear(30 * ONE, 120 * ONE);
wave.group = Ramp::stepped(ONE, 4 * ONE, 4);
wave.speed = Ramp::linear(ONE, 3 * ONE / 2);
wave.after_spawns = Some(SpawnLull { spawns: 12, lull: Lull::between(120, 240) });
wave.on_clear = Some(Lull::fixed(180));

let schedule = Schedule { cap: 30, tables: vec![table], phases: vec![wave], ..Schedule::default() };
let mut director = Director::new(schedule, seed)?;

let regions = [Region::flat([0, 0], [1920, 1080])];
let mut orders = Vec::new();
director.tick(&Spots::regions(&regions), &mut orders);
for order in &orders {
    spawn(order.entry, order.count, order.speed, order.position);
}
director.died(entry, 1);
```

- **`Schedule`**: `tick_rate` (ticks per second, 60 by default), the overall `cap`, `allow_overlap`, the `tables`, the `phases` and `lulls`, quiet windows `Window { start, end }` in absolute ticks in which nothing spawns and no spawn charge builds.
- **`Phase { start, end, table, tag_filter, rate, group, speed, after_spawns, on_clear }`**: active for ticks `start..end` (the director counts ticks from 0, one per `tick` call). `table` is the index of the table it draws from. `tag_filter` is a bit mask: with 0 every entry qualifies, otherwise only entries sharing a tag bit.
- **`Ramp { from, to, shape }`**: a value over the phase's length, in 16.16 fixed point (`ONE` = 65,536). `Ramp::flat(v)`, `Ramp::linear(a, b)`, `Ramp::stepped(a, b, steps)` (`steps` of at least 2: the value holds for each equal slice of the phase and reaches `b` in the last one). The math is i128 integer, no floats.
  - `rate`: spawns per minute. The director adds the rate to a charge counter each tick and fires one order whenever it holds a full `60 * tick_rate` of them, so `0.5` a minute is exact, and a rate above one per tick fires several orders in a tick (at most `MAX_ORDERS_PER_TICK`, 16).
  - `group`: members per order, the fixed-point value floored, at least 1.
  - `speed`: a multiplier the game applies as it likes, handed back in each order.
- **`Lull { min, max }`**: a quiet span in ticks, drawn from the timing stream between `min` and `max` inclusive (`Lull::fixed(n)` for no jitter). `after_spawns` starts one once a phase has spawned that many entities; `on_clear` starts one when everything the director spawned is dead after the phase has spawned something. A lulled phase builds no charge, so it never bursts when the lull ends.
- **`Table { entries, cap }`** and **`Entry { id, weight, tags, cost }`**: the id is the game's own, unique across all tables; the weight is relative; `tags` is the bit mask a phase filters on; `cost` is what one alive member counts against a cap (1 is a head count, 0 is free, a boss may cost 10). `cap` bounds the table's alive cost, `Schedule::cap` bounds all of it; `UNCAPPED` is the default.

## What comes out

`tick(&Spots, &mut Vec<SpawnOrder>)` clears the list and fills it with `SpawnOrder { entry, count, speed, position, spot }`:

- `entry`, `count`: what and how many. The director counts them alive at once, so the game does not report spawns, only `died(entry, count)` (it returns how many it removed, never more than are alive, 0 for an unknown id).
- `speed`: the speed ramp's value at that tick.
- `position`, `spot`: `Spots { points, regions }` are the game's spawn points (`[i32; 3]`) and boxes (`Region { min, max }`, half-open). The director picks one of them uniformly and, for a region, a uniform point in it. `spot` indexes the points then the regions. With no spots, `position` is `[0; 3]` and `spot` is `NO_SPOT`. A 2D game uses `z = 0` (`Region::flat` makes a flat box) and reads the first two axes.
- The pick only considers entries that pass the tag filter, have weight above zero and whose cost fits what the table and overall caps have left. A group is cut to fit the room. When nothing fits, the charge waits (held at one order) and the spawn fires on the first tick after a death makes room.

## Streams

`Rng::new(seed)` is forked three times by label: `TIMING` (lull lengths), `PICKS` (which entry) and `POSITIONS` (which spot, where in a region). Adding a lull range never shifts a pick, and a different number of spawn points never changes which entries come out; tests guard both.

## Load-time checks

`Director::new` (and `Schedule::check`) return a `DirectorError` whose message names the phase, table or id:

- a zero tick rate, phases with no tables, a table with no weight above zero, a duplicate entry id, a phase that ends at or before its start, a phase naming a table that does not exist, a quiet window that ends at or before its start;
- overlapping phases, unless `allow_overlap` is set (overlapping phases each keep their own charge and lull and fire in phase order);
- a phase that schedules spawns (a rate above zero anywhere on its ramp) while its table's cap or the overall cap is zero, or while no entry can ever spawn (all filtered out, or each costing more than a cap);
- a ramp with a negative value, a stepped ramp with fewer than two steps, a lull with `min > max`.

## Save and restore

`save()` returns bytes and `restore(&bytes)` loads them into a director built from the same schedule: the tick, each phase's charge, spawn count and lull, every entry's alive count and the three streams. It refuses another length, version or schedule (a hash of all its data is in the header) with `Length`, `Version` or `Mismatch`, and leaves the director untouched when it does. Alive costs are recomputed from the counts, so a restored director continues exactly as the saved one would have.

## Cost

Release build on a Ryzen 9 9950X, one `tick` with 20 phases all active, a table of 200 entries and 500 alive (the overall cap):

| case | per tick |
| --- | --- |
| every phase fires each tick (about 11 orders a tick, every pick scanning 200 entries, the game reporting the deaths) | 4 to 8 µs |
| the cap reached, all 20 phases waiting for room | 3 to 5 µs |

The spread is run to run on a loaded machine. A pick scans the table twice (total weight, then the draw), so its cost is the table's size; `tests_director.rs` holds the bench (`--ignored`, release).

`tests_director.rs` pins a 600-tick run of a two-phase schedule with a table cap, an overall cap, a stepped group ramp, lulls and a quiet window by digest, so any change to the stream shows.
