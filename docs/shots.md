# `pfx_core::sim::shots`: projectiles, emitters and followers

Pooled projectiles and the emitters that fire them, in 2D and 3D, for a shoot-'em-up, a twin-stick, an FPS's bullets or a tower defence. Everything is in integer game units (positions and velocities in `[i32; N]`, velocity per tick) or goes through `sim::math`, so it stays inside sim's promise (`docs/sim.md`): the same inputs give the same bytes on every platform.

## The pool

```rust
let mut shots = Shots::<2>::new(4096);
let id = shots.spawn(Shot { pos, vel, half: [4, 8], life: Some(240), pierce: 0, tag: owner });
for &(id, end) in shots.step(arena) {
    if end == End::Expired { pop(id) }
}
shots.boxes(&mut ids, &mut boxes);
let hits = overlaps.find(&boxes, &targets);
for strike in shots.resolve(&ids, hits, &targets, |shot, target| alive[target as usize]) {
    hurt(strike.shot, strike.target);
}
```

- `Shots<N>` (`N` = 2 or 3) holds a fixed number of slots, allocated once. `spawn` returns `None` when the pool is full. A `ShotId` is a slot index and a generation: `despawn`, `get` and `contains` refuse a handle whose shot is gone, even after the slot is reused.
- Free slots are reused last-freed first, and `iter`, `step` and `boxes` walk the slots in index order, so the order of shots depends only on what the game did, and replays.
- `step(bounds)` moves every live shot by its velocity once (saturating at the `i32` edge). `life: Some(n)` lets a shot make `n` moves; the step after its last move removes it as `End::Expired`. A shot whose swept box no longer overlaps `bounds` is removed as `End::Left` in the same step, so a shot on the edge still gets its last box tested. `life: None` lives until it leaves or is despawned. The returned list (also `ended()`) is in slot order.
- `Span<N>` is a half-open integer box like `Aabb`. `swept(id)` is the box a shot covered over its last step: its half-extents around every point from where it was to where it is, the union of its start and end boxes, as Box2D builds a moving body's AABB. A shot fresh from `spawn` has its own box. `boxes` fills `Vec<Aabb>` in 2D and `Vec<Aabb3>` in 3D (anything `From<Span<N>>`), with the matching ids, ready for `Overlaps::find` / `Overlaps3::find`.
- **Pierce:** `resolve(ids, hits, targets, accept)` applies `find`'s hits to the pool. Each shot takes its targets in the order its swept box reached them over the step (ties by target index), skips any `accept` refuses (a dead target, friendly fire, one this shot already struck), and strikes the rest: the first strike with no `pierce` left despawns the shot, each earlier one spends a pierce. It returns `Strike { shot, target, gone, pos }` (`pos` the shot's position at the strike, kept after a gone shot is freed) in shot order, then path order; the game resolves damage from it. A piercing shot overlapping a target over several ticks is offered it every tick, so a game that wants one strike per target remembers the pairs and refuses them in `accept`. `targets` is the same list given to `find` (`Aabb`, `Aabb3` or `Span<N>`).

## Emitters

```rust
let mut gun = Emitter::new(Fire {
    pattern: Pattern::Fan { count: 5, arc: 40_000 },
    rate: Rate { charge: 7, cost: 60 },
    burst: Some(Burst { count: 3, gap: 4, cooldown: 30 }),
    windup: 12,
    speed: 900,
    spread: 1_500,
});
out.clear();
gun.tick2(origin, Aim2::At { target, vel: target_vel }, held, &mut rng, &mut out);
for &vel in &out { shots.spawn(Shot { pos: origin, vel, half, life, tag }); }
```

- Angles are integer millidegrees, `FULL_TURN = 360_000`, so a game's data reads in degrees (`90_000` is a quarter turn) and fans split evenly. They become radians once, go through `sim::math::{sin, cos, atan2}`, and each velocity component is rounded to an integer once.
- **Rate:** each ticked step adds `charge`; each volley costs `cost`; the remainder carries. 7 charge against 60 cost fires exactly 7 volleys every 60 ticks, forever, and a charge above the cost fires several volleys in one tick. With the trigger released the charge only tops up to one volley, so the first shot after a pause is immediate.
- **Burst:** a paid volley becomes `count` volleys `gap` ticks apart (`gap: 0` fires them all in the tick), then the emitter rests `cooldown` ticks with its charge frozen.
- **Telegraph:** `windup: n` makes a paid volley wind up for `n` ticks before it fires; `phase()` reports `Phase::Winding { left, total }` for the game to draw, and `cancel()` drops it (the charge is spent). `Telegraph` on its own does the same for any attack: `start(n)`, then `tick()` is true on the `n`th tick.
- **Patterns** (BulletML's vocabulary, cut to integer data): `Single`; `Fan { count, arc }`, `count` shots evenly over `arc` centred on the aim; `Ring { count }`, a full turn starting on the aim; `Spiral { arms, step }`, `arms` evenly over a turn, turned by `step` more on each volley (`spin()` reads it); `Crown { count, tilt }`, in 3D a ring of `count` shots `tilt` off the aim axis, in 2D the cone's two edges.
- **Aim, 2D:** `Aim2::Angle(a)` (0 along +x, positive toward +y) or `Aim2::At { target, vel }`, which leads a constant-velocity target: `intercept(from, target, vel, speed)` solves `|D + V t| = s t` for the first positive `t` in ticks; with no solution (the target outruns the shot) it aims straight at the target. `vel: [0, 0]` is plain aiming.
- **Aim, 3D:** `Aim3::Basis { forward, up }` or `Aim3::At { target, vel, up }`. The basis is the aim axis, `side = forward × up` and `forward`'s own up; fans, rings and spirals lie in the forward–side plane (positive angles toward `side`), crowns roll around the aim axis. An `up` parallel to `forward` falls back to +z, +y, +x in that order.
- **Spread:** `spread` jitters each shot from the `Rng` the caller passes: in 2D a whole millidegree in `[-spread, spread]` (one draw), in 3D a direction uniform over the cone cap of half-angle `spread` around the shot (two draws). With `spread: 0` the rng is never touched. Fork it per emitter or per wave (`rng.fork(label)`) so emitters don't shift each other's streams.
- **Staggered guns:** `Emitter::with_charge(fire, charge)` starts with `charge` already banked, so guns sharing one `Fire` fire out of step: with cost 4, charges 0, 1, 2 and 3 put four guns on four different ticks.
- `tick(trigger)` alone returns the volleys due this tick and `volley2` / `volley3` make one; `tick2` / `tick3` are the two together.

## Followers and kinematic follow

```rust
follow(&mut wing, leader_pos);
let mut walker = Mover::new(pos, 40);
walker.slow(1, 2, 90);
walker.stun(30);
walker.step(goal);
```

- `toward(from, to, step)` moves at most `step` units in a straight line (Euclidean length, each component rounded once), and lands exactly on `to` when it is within reach. Generic over `N`.
- `Member { pos, offset, limit }` holds `offset` from its leader; `limit: None` snaps to the slot, `Some(l)` catches up at most `l` units a tick. `follow` steps a slice in order. `follow_scaled(members, leader, num, den)` and `Member::step_scaled` use `offset * num / den` (rounded to nearest, halves away from zero, so a mirrored formation stays mirrored) for a formation that tightens or spreads with a state, such as closing in while firing: ease `num` toward its target with `toward([num], [goal], step)` each tick. `slot_scaled` gives the same point for a gun's muzzle. To turn a formation with its leader, set the offsets from the leader's facing.
- `Mover { pos, top }` walks toward a target at `top` units a tick. `stun(n)` stops it for `n` steps; a second stun keeps the longer of the two. `slow(num, den, n)` keeps `num/den` of `top` (floored) for `n` steps; the same ratio again keeps the longer time, a different ratio is tracked on its own, and the strongest live slow is the one that applies. A stun beats every slow. Modifiers apply on the step they count down in.

## Saving

With pfx-core's `serde` feature, `Shots`, `Shot`, `ShotId`, `Span`, `Strike`, `End`, `Emitter` (with `Fire`, `Pattern`, `Rate`, `Burst`, `Phase`, `Aim2`, `Aim3`), `Telegraph`, `Mover`, `Slow` and `Member` derive `Serialize` and `Deserialize`. A pool saves only what it needs to come back: its capacity, its live slots (`index`, `shot`, `prev`), each slot's generation up to the last one that is not 0, and its free list as `fresh` (the length of the untouched run `capacity - 1, capacity - 2, …` it starts with) plus the rest in order. A dead slot's old shot is not saved, so a 4,096-slot pool with 205 live shots saves in 23 KB of JSON (22 µs with serde_json) instead of every slot. Loading refuses a save that lists a slot twice, misses one, reaches past the capacity or claims more than 1,048,576 slots (1 << 20), before it allocates the pool. The per-step outputs (`ended`, the last `resolve`'s strikes) and scratch buffers are not saved. A reloaded arena hands out the same ids and steps on the same bits as the one that was saved, and `Shots` compares equal (`PartialEq`) on that state: generations, free list and live slots, which tests pin.

## Cost

Release build, Ryzen 9 9950X, `bench_two_thousand_shots_and_fifty_emitters` under `pgpu run --class bench`: one tick of 50 emitters (aimed, leading a target), the pool step and every swept box, with about 2,000 live shots:

| | per tick |
| --- | --- |
| 2D, 4,096 slots, ~2,060 live | 22 µs |
| 3D, 4,096 slots, ~1,980 live, crowns with cone spread | 21 µs |

`bench_two_thousand_shots_against_four_hundred_targets`: 2,000 live shots in 4,096 slots against 400 targets, about 1,080 hits a tick, nearly all one target per shot: `step` and `boxes` 18 µs (24 before the walks stopped at the highest live slot), `resolve` 3.5 µs (15 before a shot with one candidate skipped the entry time and the sort), under a load average of about 22.

Those runs shared the machine with a load average of about 31 on 32 threads (at 94 they took 47 and 52 µs), so an idle machine should take less; rerun the bench before quoting it as a budget. The pool allocates nothing after `new`; `boxes` and the emitters' buffers allocate only while they grow.

`tests.rs` pins two 300-tick scenes (50 emitters of every pattern, bursts, wind-ups, spread, led aim) by a hash of every shot's slot, generation, position and velocity, so a change to any output bit fails it.
