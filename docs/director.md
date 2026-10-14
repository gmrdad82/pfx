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

- **`Schedule`**: `tick_rate` (ticks per second, 60 by default), the overall `cap`, `allow_overlap`, the `tables`, the `phases`, `lulls`, `cycles` and `spot_sets`. `lulls` are quiet windows `Window { start, end }` in absolute ticks in which nothing spawns and no spawn charge builds; they may overlap and come in any order.
- **`Cycle { start, every, length, end }`**: a quiet window that repeats. `Cycle::new(start, every, length)` is quiet for `length` ticks out of every `every`, from tick `start` on (`start` is its phase: ticks `start + k * every .. start + k * every + length`), and `.until(end)` stops it at tick `end` (open-ended by default). One cycle stands in for any number of evenly spaced windows and is checked by one modulo.
- **Quiet ticks cost nothing between boundaries.** The director merges the windows into a sorted list once at load; at a tick where quiet may change it finds the window with a binary search, works out each cycle by modulo, and records the next tick at which any of them starts or ends. Every tick before that reuses the answer, so 180 windows cost what none do.
- **`Phase { start, end, table, tag_filter, rate, group, speed, after_spawns, on_clear }`**: active for ticks `start..end` (the director counts ticks from 0, one per `tick` call). `table` is the index of the table it draws from. `tag_filter` is a bit mask: with 0 every entry qualifies, otherwise only entries sharing a tag bit.
- **`Ramp { from, to, shape }`**: a value over the phase's length, in 16.16 fixed point (`ONE` = 65,536). `Ramp::flat(v)`, `Ramp::linear(a, b)`, `Ramp::stepped(a, b, steps)` (`steps` of at least 2: the value holds for each equal slice of the phase and reaches `b` in the last one). The math is i128 integer, no floats.
  - `rate`: spawns per minute. The director adds the rate to a charge counter each tick and fires one order whenever it holds a full `60 * tick_rate` of them, so `0.5` a minute is exact, and a rate above one per tick fires several orders in a tick (at most `MAX_ORDERS_PER_TICK`, 16).
  - `rate_jitter` (0 by default, no jitter): a 16.16 fraction below `ONE`. With it set, the charge each order needs is the plain amount scaled by a factor drawn uniformly from `1 - jitter ..= 1 + jitter` (in 1/65,536 steps), drawn again after every order from the `JITTERS` stream, so the gaps between orders vary while their average stays the rate's. `ONE / 2` at one spawn a second gives gaps of 30 to 90 ticks at 60 ticks a second. With 0 the director draws nothing and the stream is never touched.
  - `group`: members per order, the fixed-point value floored, at least 1.
  - `speed`: a multiplier the game applies as it likes, handed back in each order.
- **`Lull { min, max }`**: a quiet span in ticks, drawn from the timing stream between `min` and `max` inclusive (`Lull::fixed(n)` for no jitter). `after_spawns` starts one once a phase has spawned that many entities; `on_clear` starts one when everything the director spawned is dead after the phase has spawned something. A lulled phase builds no charge, so it never bursts when the lull ends.
- **`Table { entries, cap }`** and **`Entry { id, weight, tags, cost }`**: the id is the game's own, unique across all tables; the weight is relative; `tags` is the bit mask a phase filters on; `cost` is what one alive member counts against a cap (1 is a head count, 0 is free, a boss may cost 10). `cap` bounds the table's alive cost, `Schedule::cap` bounds all of it; `UNCAPPED` is the default.
- **`Entry::from_tick(t)`**: the entry's earliest tick. Before tick `t` it is never picked, whatever the phase says, so a kind can arrive partway through a phase (a tougher enemy at minute three) beside the phase windows. A table whose every entry starts at or after a phase's end fails the load check for that phase.
- **`Entry::at_spots(set)`**: the kind spawns only at the spots listed in `Schedule::spot_sets[set]`, a list of indices into the `Spots` passed to `tick` (points first, then regions, the same numbering as `SpawnOrder::spot`). The director draws one of the listed spots that exist in this tick's `Spots` uniformly from the `POSITIONS` stream (an index listed twice is twice as likely); when none of them exists, the order has `position` `[0; 3]` and `spot` `NO_SPOT`. An entry without a set draws from all the spots.
- **`Entry::in_line(Line)`**: the kind spawns as a group placed in a line. `Line::new(min, max, direction, gap)` draws the member count from `min..=max` (at least 1; the phase's group ramp does not apply to a line kind) and places member `i` at the spot plus `direction * gap * i` on each axis: `direction` is a small integer vector (`[1, 0, 0]` along x, `[0, -1, 0]` down a 2D field, `[1, 0, 1]` a 3D diagonal that steps `gap` on both axes) and `gap` the distance between neighbours in the game's units. `.anchored(Anchor)` says which member sits on the picked spot: `Start` (the line runs away from it), `Centre` (the line is centred on it) or `End` (the line runs up to it). `.spread([x, y, z])` jitters each member by a uniform amount of at most that on each axis (put it on the axes across the line for a ragged file; 0 draws nothing). Motion along the way, such as zigzags and dives, stays in the game.

## What comes out

`tick(&Spots, &mut Vec<SpawnOrder>)` clears the list and fills it with `SpawnOrder { entry, count, speed, position, spot }`:

- `entry`, `count`: what and how many. A plain kind is one order with its `count`; a line kind is one order per member with `count` 1, each at its own `position`. `group` numbers the spawn (consecutive from 0, saved with the state), `member` is the member's index and `members` the size of the group, so a game can keep a line together (a formation, a shared leader, a bonus for clearing all of it). The director counts them alive at once, so the game does not report spawns, only `died(entry, count)` (it returns how many it removed, never more than are alive, 0 for an unknown id).
- `speed`: the speed ramp's value at that tick.
- `position`, `spot`: `Spots { points, regions }` are the game's spawn points (`[i32; 3]`) and boxes (`Region { min, max }`, half-open). The director picks one of them uniformly (or one of the entry's set, `Entry::at_spots`) and, for a region, a uniform point in it. `spot` indexes the points then the regions. With no spots, `position` is `[0; 3]` and `spot` is `NO_SPOT`. A 2D game uses `z = 0` (`Region::flat` makes a flat box) and reads the first two axes.
- **`Director::set_weight(id, weight)`** changes an entry's weight from the next tick on and returns the old one (`None` for an unknown id); `weight(id)` reads it (0 for an unknown id). A weight of 0 takes the kind out of the picks; when every entry a phase may pick has weight 0, its charge waits as it does at the cap. The schedule the director was built from, its load checks and its fingerprint keep the weights it was built with; the live weights are saved with the state (below), so a game no longer rebuilds the director to change them.
- The pick only considers entries that have reached their `from` tick, pass the tag filter, have weight above zero and whose cost for the smallest group they can spawn (a line's `min`, otherwise 1) fits what the table and overall caps have left. A plain group is cut to fit the room; a line is never cut below its minimum and is cut above it only when the room is short. A line's members all count toward `after_spawns`. When nothing fits, the charge waits (held at one order) and the spawn fires on the first tick after a death makes room.

## Streams

`Rng::new(seed)` is forked six times by label: `TIMING` (lull lengths), `PICKS` (which entry), `POSITIONS` (which spot, from all spots or an entry's set, and where in a region), `FORMS` (how many members a line has), `SPREADS` (each member's jitter) and `JITTERS` (the rate jitter). A jittered rate changes when orders fire, never which entries they pick. Adding a lull range never shifts a pick, a different number of spawn points never changes which entries come out, and adding spread to a line never changes how many members it has; tests guard all three.

## Load-time checks

`Director::new` (and `Schedule::check`) return a `DirectorError` whose message names the phase, table or id:

- a zero tick rate, phases with no tables, a table with no weight above zero, a duplicate entry id, a phase that ends at or before its start, a phase naming a table that does not exist, a quiet window that ends at or before its start;
- overlapping phases, unless `allow_overlap` is set (overlapping phases each keep their own charge and lull and fire in phase order);
- a phase that schedules spawns (a rate above zero anywhere on its ramp) while its table's cap or the overall cap is zero, or while no entry can ever spawn (all filtered out, or each costing more than a cap);
- a ramp with a negative value, a stepped ramp with fewer than two steps, a lull with `min > max`, a line with no members, `min > max` or a negative spread (`BadLine`);
- a cycle with a zero period, a length of zero or of at least its period, or an end at or before its start (`BadCycle`), a rate jitter below 0 or of at least `ONE` (`BadJitter`), an entry naming a spot set that does not exist (`UnknownSpots`) and an empty spot set (`EmptySpots`).

## Save and restore

`save()` returns bytes and `restore(&bytes)` loads them into a director built from the same schedule: the tick, each phase's charge, spawn count and lull, every entry's alive count, the next group number and the five streams. It refuses another length, version or schedule (a hash of all its data is in the header) with `Length`, `Version` or `Mismatch`, and leaves the director untouched when it does. Alive costs are recomputed from the counts, so a restored director continues exactly as the saved one would have.

There are two layouts, and the version in the header says which:

- **Version 2** (`DIRECTOR_PLAIN_VERSION`) is the layout above. `save()` writes it whenever every weight is the one the schedule was built with and no phase jitters its rate, so a director that uses none of the newer options saves the same bytes it always did, and cycles and spot sets alone do not change the layout (they are schedule data in the fingerprint, not state). Restoring a version 2 save puts the weights back to the schedule's.
- **Version 3** (`DIRECTOR_VERSION`) appends every entry's live weight (u32, in table then entry order) and, when a phase jitters, each phase's drawn charge and the `JITTERS` stream. `save()` writes it once a weight differs from the schedule's or a phase jitters.

The fingerprint hashes the schedule as it was built, weights included; cycles, spot sets, entry sets and jitters enter the hash only when used, so a schedule without them keeps its old fingerprint. To load a save, build the director from the original schedule and `restore`: the saved weights come back with the state, and a replay that records `set_weight` calls beside `died` calls plays back the same bytes.

## Cost

Release build on a Ryzen 9 9950X, one `tick` with 20 phases all active, a table of 200 entries and 500 alive (the overall cap):

| case | per tick |
| --- | --- |
| every phase fires each tick (about 11 orders a tick, every pick scanning 200 entries, the game reporting the deaths) | 4 to 8 µs |
| the cap reached, all 20 phases waiting for room | 3 to 5 µs |

The spread is run to run on a loaded machine. A pick scans the table twice (total weight, then the draw), so its cost is the table's size.

Quiet windows, the same machine and build, one `tick` with 4 phases, a table of 20 entries and a cap of 40, a death every third tick, the mean of 108,000 ticks:

| case | per tick |
| --- | --- |
| no windows | 37 to 59 ns |
| 180 windows, each 60 ticks every 600 | 34 to 54 ns (a scan of every window each tick was 170 to 250 ns) |
| one `Cycle::new(300, 600, 60)`, the same quiet ticks | 35 to 54 ns |

`tests_director.rs` holds both benches (`--ignored`, release).

`tests_director.rs` pins a 600-tick run of a two-phase schedule with a table cap, an overall cap, a stepped group ramp, lulls and a quiet window by digest, the bytes of its version 2 save, and a second digest of the same schedule with rate jitter, spot sets and a cycle, so any change to the stream or the save shows.
