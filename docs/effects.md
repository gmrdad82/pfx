# Effects on the flat pass

Games want punch on a fixed board: camera shake, motion blur on fast things, bursts of shapes, shockwave rings, a screen flash and a hit-stop. They also want two accessibility settings: a game speed that slows everything that keeps time, and a reduced-motion setting that turns the punch down or off. This page describes all of it.

The engine is game-agnostic. Every amount, curve, colour, seed and duration comes from the game; the engine ships the mechanisms and two hard rules (the flash-rate cap, and that pacing never changes the simulation).

| Piece | Where |
|---|---|
| The game clock, hit-stop, fixed-step counters | `pfx_core::clock` |
| Intensity curves, reduced motion, trauma shake, the flash gate | `pfx_core::fx` |
| Bursts, rings, the flash, shake on the board, motion blur | `pfx_live::flat::effects` and `Draw::fx` |
| Rigid-body shards | `pfx_live::flat::effects::shards` |

## One game clock

`Clock::new(tick_rate)` is the one place a game's time comes from. The game feeds it the real seconds of each frame, which it measures itself; the clock never reads the wall clock.

```rust
let mut clock = Clock::new(60.0)?;
clock.set_speed(settings.game_speed);
loop {
    let tick = clock.advance(real_seconds);
    for _ in 0..tick.ticks {
        sim.step(inputs.at(clock_tick));
    }
    tween.step(tick.dt);
    effects.step(&tick);
}
```

`advance(real)` returns a `Tick`:

| Field | Meaning |
|---|---|
| `ticks` | how many of the game's fixed sim ticks fall due this frame |
| `dt` | game seconds this frame: real × speed × the hit-stop dip, 0 while paused |
| `paced` | presentation seconds: real × speed, without the dip, 0 while paused |
| `real` | the real seconds given |
| `scale` | the time scale at the end of the frame |
| `alpha` | how far game time is toward the next tick (0..1), for drawing between ticks |

**What it scales.**
- **The scale** is the game's speed (`set_speed`, 0.05 to 4) times any hit-stop dip, or 0 while paused (`pause`, `resume`).
- **Sim ticks** are whole fixed steps of `1 / tick_rate` game seconds. Game time accumulates in `f64`, and a tick is due each time it passes a multiple of the tick length.
- **Easings and tweens** step by `tick.dt`, so at 75% a one-second tween takes 4/3 real seconds.
- **Effect lifetimes** (bursts, rings, shards) age by `tick.dt`. Shake and the flash run on `tick.paced`: they slow with the game speed but keep moving through a hit-stop, so a freeze frame still shakes.
- **Rigid-body steps.** `Steps::new(world.rate(), &clock)` counts a world's fixed steps; `steps.due(&clock)` is how many `World::step` calls are due this frame. At 75% a 60 Hz world takes 45 steps a real second.
- **Sound.** `clock.sound_rate(tie)` is a rate for the mixer: 1 at `tie` 0, the clock's scale at `tie` 1, clamped to the mixer's 1/16..8. A game that wants its sound slowed with the game applies it with `Mixer::set_channel_pitch` (or `glide_channel_pitch`) on each channel it chooses. Pausing is the game's own call to the mixer.

**Pacing never changes the run.** Speed, hit-stop and reduced motion change only how many ticks run per real second and how presentation moves. Every tick is the same `1 / tick_rate` step, so the number of ticks in a run and the inputs each tick sees are the game's alone: a game that records its inputs by tick replays them exactly at any speed. `speed_and_hit_stops_change_pacing_never_the_ticks_or_their_inputs` runs 1,200 ticks at 100% and at 75% with three hit-stops, and checks the same ticks, the same input per tick and the same final state.

### Hit-stop

`clock.hit_stop(HitStop { floor, hold, recover, ease })` dips the scale to `floor` (0 is a full stop) for `hold` real seconds, then eases it back to 1 over `recover` real seconds along `ease` (`QuadOut` by default). The dip runs on real time, and stops while paused. A new hit-stop replaces one in progress.

The game time a frame gets through the recovery is the integral of the eased scale over the frame (Simpson's rule over 16 slices), so the length of a dip does not depend on the frame rate. A quadratic ease is integrated exactly. `Effects::hit_stop(&mut clock, stop)` applies reduced motion first.

### Saving and resuming

A game resumes a quit run at the exact tick it was saved, on any machine. `clock.save()` gives 56 bytes and `Clock::restore(&bytes)` reads them back; the restored clock advances exactly as the saved one would. `CLOCK_VERSION` is 1.

| bytes | field |
|---|---|
| 0..4 | `u32` version (`1`) |
| 4..12 | `f64` tick rate |
| 12..16 | `f32` speed |
| 16 | pause flag, 0 or 1 |
| 17 | dip flag, 0 or 1 |
| 18 | the dip's easing, its index in `Ease::ALL` |
| 19 | zero |
| 20..28 | `f64` game time |
| 28..36 | `u64` ticks run |
| 36..40, 40..44, 44..48 | `f32` dip floor, hold and recover (1, 0, 0 without a dip) |
| 48..56 | `f64` real seconds into the dip |

All little-endian. `restore` refuses, with `ClockError`, a length other than 56 (`Length`), another version (`Version`), and anything out of range (`Invalid`): a tick rate that is not finite and positive, a speed outside 0.05..=4, a flag other than 0 or 1, an unknown easing, nonzero padding, a negative or non-finite game time, or a dip whose floor is outside 0..1, whose times are negative, or which has already ended. `the_saved_bytes_are_pinned` holds the layout.

Only the clock is saved. Presentation state, the physics world, bursts, rings, shake and tweens, is rebuilt by the game on resume. `Steps::new(rate, &clock)` starts from the clock's game time, so a rebuilt world does not try to catch up from zero.

## Intensity

`effects.intensity` is one game-driven value, 0 to 1. Each effect reads it through a `Response { gain, rate }` the game sets in `Responses { shake, burst, ring, flash, blur }`:

- `Curve { low, high, ease }` maps intensity to a value: `low + (high − low) × ease(intensity)`. `Curve::ONE` is a constant 1, and `Response::STEADY` is gain 1 and rate 1, the default for every effect.
- **Shake:** gain scales the offset, rate the noise frequency. The noise phase accumulates, so a rising rate quickens the shake without a jump.
- **Bursts:** gain scales the particle count, rate the spawn speed.
- **Rings:** gain scales the alpha, rate shortens the life.
- **Flash:** gain scales the alpha, rate shortens the attack and release.
- **Blur:** gain scales the shutter (`Effects::motion`).

A game that wants a building alert sets rising curves (a rate from 1 to 4, a gain from 0.5 to 2) and raises the intensity: shake quickens and grows, bursts get denser and faster, flashes sharpen, until the screen feels like a high-pitched alarm. The flash-rate cap still holds.

## Camera shake

`ShakeTuning { translation, roll, frequency, decay, seed }` is the game's:
- `translation` is the largest offset in layout units, per axis, at full trauma;
- `roll` the largest turn in radians;
- `frequency` the noise rate in hertz;
- `decay` trauma lost per second;
- `seed` the noise seed.

`effects.shake(amount)` adds trauma, which stacks and is clamped to 0..1, and decays linearly. The offset is trauma² × gain × reduced motion × seeded smooth noise, separately in x, y and roll. The noise is 1D gradient noise with a quintic fade, hashed with `sim::mix64` from the seed, in [−1, 1] and continuous.

It is deterministic from the seed and the sequence of calls and game time: two runs with the same seed and the same `shake` calls at the same ticks give the same offsets bit for bit, so pfx shots repeat. `shake_repeats_from_its_seed_and_differs_across_seeds` checks the bits.

**On the board.** The camera stays fixed at rest; shake is applied at render only. `effects.compose(&draws, light)` returns the frame's draw list and light. With shake, every game draw, particle and ring is moved by `camera()`, the translation and roll about the layout's centre. The light's shadow direction turns with the roll, so shadows stay where they were relative to their casters. The flash is not shaken. The game's own draws are copied, never changed; with no trauma the list is the game's, bit for bit. A HUD the game wants steady is drawn outside `compose`.

## Motion blur

Blur is opt-in per draw: `Draw::blur([dx, dy])` is the shape's displacement in layout units over the shutter, centred on where it is drawn. `effects.motion(velocity, shutter)` gives it from a velocity in layout units per second, with the blur response's gain. Reduced motion scales every draw's blur in `compose`, and at 0 removes it.

Each pixel's coverage is the integral of the moving shape over the pixel and the shutter, computed analytically. No frame is accumulated and nothing is drawn twice.

- Along the motion, the pixel's footprint is the 1-pixel box convolved with the motion segment, a trapezoid. The coverage is that trapezoid's weight over the parts of the line through the pixel that lie inside the shape.
- For rect, circle and ring fills, and solid strokes on rects and circles, those parts come in closed form. A rounded rect is the union of two boxes and four corner discs, so its chord is the hull of a slab test per box and a quadratic per disc. A stroke is the shape grown by half the width less the shape shrunk by it (the SDF's own level sets), and a ring is its outer disc less its inner one.
- Arcs, icons, dashed strokes and strokes on rings are stepped along their signed distance field instead. A step outside or inside is the distance itself, which can never jump an edge, and each crossing is placed by linear interpolation of the distance, which is exact for a straight edge. A step is at least 0.35 pixel, or 1/40 of the trapezoid's length, and there are at most 48. A dash is the intersection of the stroke's band with the dash's own field (`max(band, dash)`), so dashes blur with the stroke. `the_closed_form_agrees_with_the_march` checks the two ways against each other within 0.01 of full coverage.
- The trapezoid's weight has a closed form (a piecewise quadratic), so the integral is exact along the line.
- Across the motion, four lines at ±1/8 and ±3/8 of a pixel are averaged.

It works for rects, circles, rings, arcs and icons, for fills, strokes and dashed strokes. Rotation within the frame is not blurred: motion is a straight line. A pattern and a vertical gradient are taken at the pixel. A blurred draw drops its material dials while it moves and draws flat; at the speeds blur is for, a bevel or gloss can't be seen. Its shadow stays sharp, which the shadow's own σ already softens. A displacement under a quarter pixel packs the still instance, byte for byte.

**Accuracy.** `blur_is_exact_along_the_motion_of_a_straight_edge` compares a moving rect with the analytic time integral of its pixel overlap, at displacements of 0.6, 3, 30 and 90 pixels: within 0.002 of full coverage. `blur_matches_the_integral_of_moving_shapes` compares six shapes with a brute-force integral (12 × 12 samples a pixel, 96 in time):

| Shape | Motion (px) | mean error | worst |
|---|---|---|---|
| circle | (18, 12) | 0.0003 | 0.0145 |
| turned rounded rect | (−20, 9) | 0.0001 | 0.0044 |
| ring | (0, 24) | 0.0002 | 0.0021 |
| rect outline | (16, 16) | 0.0002 | 0.0039 |
| dashed outline | (14, 0) | 0.0001 | 0.0051 |
| check (line icon) | (26, −14) | 0.0001 | 0.0072 |
| pointer (filled icon) | (30, 20) | 0.0003 | 0.0095 |

The brute force itself is good to about 1/(12² × 96) per sample. The CPU twin runs the same code, and the GPU matches it to within 1/255 at 1× and 2× (`the_gpu_blurs_and_blends_like_its_twin`).

**Picking.** A blurred draw writes its id where its still footprint, at the middle of the shutter, reaches one half: a pick finds a moving thing where it is, and costs what a still draw costs.

## Bursts

A `Burst` is a recipe the game registers once, with `effects.recipe(burst) -> BurstId`:

| Field | Meaning |
|---|---|
| `shape`, `fill`, `stroke` | any flat shape: circles, rounded rects, rings, arcs, icons, or a text index for glyphs |
| `count` | particles per burst, times the burst gain |
| `speed`, `direction`, `spread` | spawn speed range (layout units per second, times the burst rate), the mean direction and the full cone in radians |
| `gravity`, `drag` | acceleration in layout units per second², and drag per second (`v *= exp(−drag dt)`) |
| `spin` | spin range in radians per second, either way |
| `life`, `size` | life range in game seconds, starting scale range |
| `scale`, `alpha` | `Curve`s over the particle's life, 0 to 1 |
| `colour` | an optional `Fade { from, to, ease }` over life, which replaces a solid fill |
| `elevation`, `shadow` | painter's order and the shadow, as any draw |
| `shutter` | seconds of motion blur along its velocity, 0 for none |
| `additive` | adds its light instead of covering |

`effects.burst(id, at)` and `burst_toward(id, at, direction)` spawn; each returns how many it spawned. Spawns come from the effects' seeded `Rng`, so the same calls give the same bursts.

**The pool.** `EffectsDesc::particles` (8,192 by default) is the drawn cap, allocated when the effects are made. A spawn past it is dropped and counted in `dropped()`. Dead particles leave by swap-remove. After the first `compose` sizes the frame list, nothing allocates: `the_pool_does_not_allocate_after_warm_up_and_the_cap_holds` spawns 400 particles a frame against a cap of 5,000 for 640 frames (over 40,000 particles at roughly 7,000 a second) and checks every buffer's capacity and address. The flat pass's own instance buffers grow by powers of two and are rewritten in place, so thousands of instances a second cost a buffer write.

Particles, rings, shards and the flash are `unpicked`: they write no id, so a card under a burst can still be picked.

### Shards that collide

Debris that must hit things comes from the rigid-body world. `Shards::new(cap, seed)` keeps up to `cap` shards. `shards.spawn(&mut world, &Shatter, at)` adds dynamic rounded boxes in the world's z = 0 plane, with `Locks::PLANE_XY` (moving in x and y, turning about z), in layout units. Their size range, speed, cone, spin, life, fade, density, restitution, friction and collision layers come from the `Shatter`. The game builds the world in layout units (gravity `[0, g, 0]` falls down the board) and adds its own colliders.

Each frame the game steps the world `steps.due(&clock)` times, then `shards.step(&mut world, tick.dt)` ages them and removes each expired shard's body, and `shards.draws(&world, &mut out)` appends their draws, fading over the last `fade` seconds. `shards_collide_follow_the_clock_and_leave_with_their_bodies` drops twelve shards on a floor at 75% speed: 90 world steps in two real seconds, all of them resting above the floor, all bodies gone at the end.

## Shockwave rings and the screen flash

`effects.ring(Ring { centre, radius, width, colour, alpha, life, ease, elevation, additive })` expands a ring from `radius[0]` to `radius[1]` while its width goes from `width[0]` to `width[1]`, along `ease`, with `alpha` over its life. `EffectsDesc::rings` caps them (64 by default).

`effects.flash(Flash { colour, alpha, attack, release, ease, additive })` is a full-layout pulse drawn above everything (`FLASH_ELEVATION`). It rises to `alpha` over `attack` along `ease` and falls back over `release`, on presentation time. A scrim pulse darkens (`additive: false`); an additive one adds its colour to what is beneath.

**Additive draws.** `Draw::additive()` composites as premultiplied colour with zero alpha, so it adds light (`src + dst`) and leaves the coverage beneath it as it was. Any draw can use it.

**The flash-rate cap.** WCAG 2.3.1 allows no more than three flashes in any one-second period. The engine holds this whatever the game asks: `FlashGate` admits a flash only if fewer than three started in the last real second, in real time, not game time, since a slowed game must not flash faster than the eye allows. `flash` returns false when it is refused, and the gate is not configurable.

## Reduced motion

`Comfort { motion, glitch_floor }` is one per-game accessibility setting. `motion` runs from 0 (none) to 1 (all), and `effects.comfort` holds it:

| Effect | At `motion` m |
|---|---|
| shake | offset × m |
| motion blur | every draw's blur × m |
| flashes | alpha × m; at 0 a flash is refused |
| hit-stop | dip depth × m: the floor becomes 1 − (1 − floor) × m |
| the tape glitch | strength × (glitch_floor + (1 − glitch_floor) × m), through `effects.tape(tape)` |

The default is `Comfort::FULL`: m = 1 and a glitch floor of 0.5, so at m = 0 the glitch runs at half strength rather than vanishing, since a game may tell part of its story with it. The game sets both, and the default setting it ships is its own choice. `reduced_motion_at_zero_removes_shake_blur_flashes_and_hit_stop` checks all of it. Bursts and rings are not motion of the view, so they stay; a game that wants fewer turns its burst gain down.

## The flat pass, unchanged without effects

`Draw::fx` is a `Fx { blur, additive, unpicked }`, all zero by default. A draw without effects packs the same instance as before (240 bytes since per-corner radii), and the flat pipeline's shader (`shader::flat_source()`) does not contain the effect code. The effect paths live in `shader::FX_WGSL`, reached only from the second pipeline that lit draws already use, so no binding, attribute or inter-stage component is added and the WebGPU floor is unchanged. Blurred, additive and unpicked instances set flags 11, 15 and 16 of `info.x`, and a blurred one carries its local displacement in the material slot it no longer uses.

`frames_without_effects_are_byte_identical` composes the neutral board with effects made but idle and checks the draws, and the CPU frame's bits, against the board alone. On the GPU, `a_frame_without_effects_keeps_its_bytes_after_effects_ran` renders the board before any effect, after a frame with a burst, shake and flash, and on a fresh pass: the colour and id bytes are identical.

## Cost

GPU time per pass on this machine (RX 9060 XT, RADV), medians of 80 frames after a warm-up round, from `a_burst_heavy_frame_fits_at_4k_and_on_a_deck_sized_target`. The scene is the neutral board (without its group layers) plus 500 checks flying up the lanes. The effects variant adds 5,000 live particles in three recipes (discs, blurred rounded rects, additive rings), shake with roll, and blur on all 500 checks.

| Size | Variant | flat | flat ids | total |
|---|---|---|---|---|
| 3840×2160 | board and 500 still checks | 0.98 ms | 0.37 ms | **1.34 ms** |
| 3840×2160 | blur on 500 checks | 2.43 ms | 0.39 ms | 2.82 ms |
| 3840×2160 | 5,000 particles | 1.16 ms | 0.39 ms | 1.55 ms |
| 3840×2160 | 5,000 particles, a third blurred | 1.76 ms | 0.39 ms | 2.15 ms |
| 3840×2160 | 5,000 particles, shake, blur on 500 checks | 4.68 ms | 0.60 ms | **5.28 ms** |
| 1280×800 | board and 500 still checks | 0.13 ms | 0.05 ms | **0.18 ms** |
| 1280×800 | blur on 500 checks | 0.37 ms | 0.05 ms | 0.43 ms |
| 1280×800 | 5,000 particles | 0.18 ms | 0.05 ms | 0.24 ms |
| 1280×800 | 5,000 particles, a third blurred | 0.29 ms | 0.05 ms | 0.35 ms |
| 1280×800 | 5,000 particles, shake, blur on 500 checks | 0.55 ms | 0.06 ms | **0.61 ms** |

The scene draws 5,761 instances. The GPU was shared with other jobs while this ran, so the full 4K row is above the sum of its parts (about 3.3 ms in an earlier run). Blurring a check costs most: an icon stroke is stepped along its field on four lines. Particles, whose fills blur in closed form, cost little more blurred than still. Unpicked instances never reach the ids pass, and a blurred draw's id costs what a still draw's does.

**The Deck floor at 1280×800.** On the flat pass's own estimate (a Deck-class GPU at 8× to 16× this machine's time, see [flat.md](flat.md)), the full effects frame is 4.8 to 9.7 ms. That fits 60 fps (16.7 ms) and 90 fps (11.1 ms). The bench fails if 16× the 1280×800 total passes 16.7 ms.

## Not covered

- Rotation is not blurred, and a blurred draw shows its flat paint, not its material dials.
- Shadows of moving draws are not blurred.
- A master playback rate inside the mixer: the clock gives the rate, and the game applies it per channel.
- A real measurement on a Deck-class device.
