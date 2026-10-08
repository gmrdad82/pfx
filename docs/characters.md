# Characters

A character is a kinematic capsule a game drives each tick with a move and a jump: it walks up slopes and steps, keeps to the ground, rides what it stands on, pushes the bodies it walks into, and jumps with coyote time and a jump buffer. A 2D character is the same controller held in a plane, for a platformer, with a variable jump, wall slide and wall jump. Both run on the one rigid world (`pfx_physics::rigid`, on Rapier); there is no second physics engine.

The controller is generic: no game's names or values. Its tests and fixtures are the engine's neutral shapes (below).

## In a game

A character belongs to a scene object, through the object's `[object.character]` (below). `World::character(object)` spawns the controller there the first time it is called and returns the same one after:

```rust
fn tick(&mut self, world: &mut World, _tick: &Tick, input: &Input) {
    let player = world.object("player").unwrap();
    let x = f32::from(u8::from(input.action("right").held)) - f32::from(u8::from(input.action("left").held));
    world.character(player)?.drive([x, 0.0, 0.0], input.action("jump").held);
}
```

- `drive(direction, jump)` sets this tick's intent: `direction` is a world-space move whose length (at most 1) scales the character's `speed`; the part along "up" is dropped, and in 2D the part across the plane too. `jump` is whether the jump is held; a press is its rising edge.
- The session steps every character just before the rigid world steps (`docs/play.md`, "Bodies and sound per tick"), and then moves its object so the object's origin sits at the character's feet. The capsule is the object's own: an object with a character takes no `[object.body]`.
- `World::character_of(object)` reads one without spawning, `characters()` lists them, `remove_character(object)` drops one, and `set_character_desc(object, desc)` changes any field at run time (a game's move speed from a tunable, say), and gives an object with no table a character the game describes itself. `character_desc(object)` reads the description a character has or will spawn with.
- `Character` reads back `feet()`, `center()`, `velocity()` (its own, without what carries it), `up()`, `grounded()`, `sliding()` (on a slope steeper than `max_climb`), `on_wall()` (the wall's normal, this tick), `ground()` (the body it stands on, when that body moves) and `ticks()`.

## The table

The keys are pfx-scene's `[object.character]` (format 1, pinned at `d9a9f1a`; its `docs/format.md` owns them). pfx-load reads the table into `Scene::characters` with pfx-scene's defaults, and the radius and height from the object's mesh bounds times its scale:

```toml
[[object]]
name = "player"
mesh = "player"
at = [0.0, 0.0, 0.0]

[object.character]
kind = "2d"
jump_speed = 6.3
variable_jump = 0.5
wall_slide = 2.0
wall_jump = [5.0, 7.0]
layer = "player"
```

| key | default | what it does |
|---|---|---|
| `kind` | `"3d"` | `"2d"` holds the character in a plane |
| `radius` | the larger of the X and Z halves of the mesh bounds times the scale | the capsule's radius |
| `height` | the Y size of those bounds | the capsule's whole height, caps included; at least twice the radius |
| `max_climb` | `45` | degrees: the steepest slope it walks up; on a steeper one it slides down and cannot jump |
| `max_step` | `0.3` | the highest step it walks onto; below `height` |
| `snap` | `0.2` | how far the ground may drop under it, over a bump, a step down or a slope's top, and it still keeps to it; `0` is off |
| `coyote` | `6` | ticks after it leaves the ground in which a jump still takes off (0 to 1000) |
| `jump_buffer` | `6` | ticks before it lands in which a jump pressed early is kept (0 to 1000) |
| `jump_speed` | `5` | its upward speed at take-off; `0` never jumps |
| `plane` | `"xy"` | 2D only: `"xy"` moves along X, `"yz"` along Z, both with Y up, through its starting position |
| `variable_jump` | `0` | 2D only: the share of its upward speed it loses when the jump is let go while it rises |
| `wall_slide` | none | 2D only: the fastest it slides down a wall it presses into |
| `wall_jump` | none | 2D only: `[away, up]`, the speeds of a jump off a wall it touches |
| `layer` | none | its collision layer, a name from the project's `[layers]` |

pfx-scene refuses a bad table with its findings (an unknown key, a value out of range, a 2D key on a 3D character, a character with a body). pfx-load also refuses a character whose mesh gives it no size, or a height less than twice its radius, by object. A layer is the same bit a body on that layer takes ([play.md](play.md#layers)): its member bit and the mask of the layers it lists; a character with no `layer` is on every layer and meets everything.

The rest of the description is the engine's and has no key; a game sets it with `set_character_desc`:

| field | default | what it does |
|---|---|---|
| `speed` | `5` | the horizontal speed of a full `drive` |
| `acceleration`, `air_acceleration` | `60`, `20` | how fast the horizontal speed reaches the wish, on the ground and in the air (m/s²); none while sliding |
| `step_width` | `0.1` | how much of a step's top must be free to step onto it |
| `skin` | `0.02` | the gap kept between the capsule and what it touches |
| `gravity_scale`, `max_fall` | `1`, `50` | its share of gravity and its fastest fall |
| `mass`, `push` | `80`, on | the impulses it gives the dynamic bodies it walks into |
| `ride` | on | whether it rides moving ground |
| `layers` | from `layer`, or every layer | the collision groups its capsule and its casts use |

## How it moves

Each tick, in order:

1. **Gravity** is the world's gravity plus every gravity source at the capsule's centre (`World::gravity_at`), times `gravity_scale`; "up" is against it, so a character walks a planet or a gravity zone and its capsule turns to stand up in it. In 2D, up stays in the plane.
2. **Intent.** The horizontal velocity moves toward `direction × speed` at the ground or air acceleration. A buffered press jumps when the character stands, or stood within `coyote` ticks and has not jumped since; otherwise, with `wall_jump`, off a wall it touched within `coyote` ticks. Letting go of a rising jump takes `variable_jump` of its speed once. Gravity then pulls; `wall_slide` caps the fall against a wall; `max_fall` caps any fall.
3. **The move** goes through Rapier's `KinematicCharacterController`: first a zero move that pushes the capsule out of anything it overlaps, then the tick's move with slopes, autostep (onto any body, a loose crate included) and ground snap. Rapier's own platform carry lags a tick, so the controller hides the bodies' velocities from it and carries the character itself.
4. **Riding.** On a kinematic body, the character moves with the body's motion of this very step (the pose the world will move it to, from its `move_to` target or its drive), so it stands still on a lift or a carousel, up or down; on a dynamic body, with its velocity at the feet. The carry is cast against everything but the ground it rides.
5. **Pushing.** The bodies the move touched take Rapier's character impulses for `mass`.
6. **Ground and walls.** A ray down from the centre reads the floor's slope: steeper than `max_climb` is a slide, flatter is standing. Rising is never standing. A wall is a hit within about 17° of vertical; airborne and pushing toward one, a short cast finds it too. Velocity into a wall is dropped.
7. The kinematic body is moved to the result through the world's `move_to`, with the 2D plane's locked axes (`Locks::PLANE_XY`, or X locked for `yz`) holding it exactly.

## Determinism

The controller reads no clock and no randomness, walks nothing in hash order, and uses `pfx_core::sim`'s own cosine. The same scene, recording and edits give the same positions bit for bit, natively and under Wine:

- `pfx-play` turns on pfx-physics's `enhanced-determinism`, so Rapier and parry take their cross-platform float paths; a game that links pfx-play gets them too.
- `rigid::character_tests::the_same_inputs_give_the_same_positions_bit_for_bit` plays two characters over a course with a lift and loose crates for 720 ticks, twice, and with `enhanced-determinism` pins the result's fingerprint.
- `tests::characters::a_recorded_run_gives_every_pose_bit_for_bit_on_every_platform` (pfx-play) records six seconds of input in each fixture, replays it twice, and pins the fingerprint of every body's pose and every character's feet and velocity. `bin/windows-check test` runs both under Wine against the same numbers.

## Live edits

During play the inspector shows an object's character keys under `character.*`, and an edit reaches the running character at once through `session.edit(Edit::scene(Target::Object(name), &["character", key], value))`. A new size rebuilds the capsule with the feet kept; `kind` cannot change while the character lives. `layer` is not live. The pending patch holds the edit; Keep writes it into the scene through `SceneEdit` (`[object.character]`, comments and spacing kept) as one undo step with the play's other edits, and an edit while stopped writes it the same way.

## Fixtures and tests

`crates/play/tests/characters/` holds two neutral levels built from the engine's block:

- `platformer/`: a 2D level with a 0.25 step, a 20° ramp, a ledge to jump to, a tall wall for wall slide and wall jump, and a lift (a kinematic body the test game moves).
- `room/`: an FPS room with four walls, three 0.2 stairs, a ramp and a crate to push.

- `crates/physics/src/rigid/character_tests.rs` (CPU): falling and standing, walking to speed, slopes walked and refused, a slide down a steep slope, steps climbed and refused, ground snap on a small drop and its absence, riding a moved and a driven platform (and not, with `ride` off), coyote time, the jump buffer, the jump's height against the closed form and a variable jump, the 2D plane held bit for bit, wall slide and wall jump, pushing a crate, a gravity zone turning the character, determinism, refusals and a live resize.
- `crates/play/src/tests/characters.rs` (CPU): each fixture played by a test game through the action map (the step, the ramp, the ledge, the wall, the lift; the stairs, the walls, the crate), record and replay, the scene's keys, defaults from the mesh bounds, layers and a too-flat refusal, and live edits.
- `crates/editor/src/tests/characters.rs` (CPU): the inspector's rows from `[object.character]`, an edit while stopped, and a play edit kept, both written into the scene.
- `crates/play/tests/characters.rs` (GPU, ignored): plays the platformer for four seconds of recorded input, drawing every frame offscreen, and checks the final positions against the same recording played headless, bit for bit, and against where the level puts them.

```
cargo test -q -p pfx-physics --lib character_tests
cargo test -q -p pfx-play --lib tests::characters
cargo test -q -p pfx-editor --lib tests::characters
cargo test -q -p pfx-play --test characters --no-run
pgpu run --class clip --as pfx -- cargo test -q -p pfx-play --test characters -- --ignored --test-threads=1
```
