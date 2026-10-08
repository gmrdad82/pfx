# Camera rigs

Beside the orbit `Director` ([camera.rs](../crates/core/src/camera.rs)), `pfx_core::rig` has rigs that follow a body or look through a character's eyes, and `pfx-play` lets a game drive the camera with them from data. They are generic: a rig knows a target's position, a look input and a probe of the world, never a game.

| Rig | Kind in data | What it does |
|---|---|---|
| `Follow` (plane) | `follow2d` | orthographic follow in x and y, a dead zone, look-ahead, damping, bounds the view never leaves, optional pixel snapping |
| `Follow` | `follow3d` | the same in 3D: the camera sits at a fixed arm from a focus that follows the target |
| `Person` | `first_person` | look from mouse or stick, sensitivity, invert, smoothing, a pitch clamp, optional head bob |
| `Person` | `third_person` | the same rig as an orbit behind the character, with a sphere cast through pfx-physics so the camera never goes through a wall |

## The maths (`pfx_core::rig`)

Everything steps at the fixed tick: `Rig::tick(dt, &Input, comfort, &probe)`, then `Rig::pose(alpha)` gives the `View` (`at`, `look_at`, `up`) between the last two ticks. Nothing reads a clock or a random number, and every sine, cosine and exponential is `pfx_core::sim`'s own (`sinf`, `cosf`, `expf`), never the platform's, so the same inputs by tick give the same views bit for bit, whatever the frame times and on every platform. A test pins the poses after a fixed input stream to exact bits. Damping is `1 - exp(-dt / time)`, so a rig covers the same ground at any tick rate; a time of 0 is instant.

### Follow

`FollowTuning`:

| Field | Meaning |
|---|---|
| `offset` | added to the target: what the camera centres on |
| `arm` | from the focus to the camera (default `[0, 4, 8]`; `[0, 0, 10]` for a plane) |
| `dead_zone` | half extents per axis around the focus; the target moves freely inside, and outside it drags the focus by the excess only |
| `damping` | seconds per axis to close the gap (0 is instant) |
| `look_ahead`, `look_ahead_speed`, `look_ahead_damping` | the focus leads in the direction of travel by up to `look_ahead` at `look_ahead_speed`, eased by its own damping |
| `bounds` | the world box the focus never leaves; `set_view_half` shrinks it by the view's half size so the whole view stays inside, and a view bigger than the box centres on it |
| `handover` | seconds to slide to a new target (`hand_over()`), ignoring the dead zone; 0 cuts |
| `pixels_per_unit` | plane rigs only: the focus snaps to whole pixels in the pose, never in the stored state, so damping is not disturbed |
| `plane` | x and y only; z follows the target |

### Person

`PersonTuning`: `eye` (from the target to the eye), `yaw` and `pitch` (degrees), `mouse` (degrees per count), `stick` (degrees per second at full tilt), `invert_x`, `invert_y`, `smoothing` (seconds; the turn is spread over time and none of it is lost), `pitch_range` (degrees, always short of straight up), `bob`, `third`, `handover`.

- Yaw 0 looks along -z; the mouse turns right and down for positive counts (window coordinates, y down), the stick turns right and up (y up).
- `Bob { stride, vertical, sway, full_speed, ease }`: the phase advances with the distance walked (`stride` metres per cycle), the amplitude follows the speed up to `full_speed`, eases out when standing still, and is zero in the air (`Input::grounded`). It fades out as the camera moves back into third person.
- `Third { distance, radius, margin, min_distance, recover, shoulder }`: the camera sits `distance` behind the pivot, a sphere of `radius` is cast from the pivot backwards through the `Probe`, and the arm shortens at once to what is free (minus `margin`, never below `min_distance`) and eases back out over `recover` seconds. `Person::set_third` switches the same rig between first and third person; the arm eases in and out.

### Reduced motion

`Comfort` is `pfx_core::fx::Comfort`, the setting the shake and flashes already use (`world.fx.comfort`). Head bob and look-ahead scale with `comfort.amount()`: `Comfort::reduced(0.0)` turns both off.

## In a game (`pfx-play`)

### The data

Rigs are `[[rig]]` tables. Until pfx-scene's keys land in a scene, they are read from a `rigs.toml` at the project root (the folder with `project.toml`), or from the file `project.toml` names with `rigs = "cameras.toml"`; `Options::rigs` hands a session its own, and `Rigs::parse` reads the same tables from any TOML text. An unknown key, kind or a value out of range is refused with the rig's name and the key.

```toml
[[rig]]
name = "side"
kind = "follow2d"        # follow2d, follow3d, first_person or third_person
target = "hero"          # an object's name; its world position
height = 9.0             # follow2d: the orthographic view's height
fit_view = true          # follow2d: bounds keep the whole view inside
offset = [0, 1.0]
dead_zone = [1.5, 0.75]
look_ahead = 2.0
look_ahead_speed = 6.0
look_ahead_damping = 0.3
damping = [0.25, 0.4]    # a number or 2 to 3 numbers
bounds = { min = [0, 0], max = [64, 20] }
handover = 0.6
pixels_per_unit = 16
blend = 0.4              # seconds to blend from the previous camera when this rig takes over

[[rig]]
name = "eyes"
kind = "first_person"
target = "hero"
eye = [0, 1.65, 0]
mouse_sensitivity = 0.08 # degrees per count
stick_sensitivity = 150  # degrees per second
invert_y = false
smoothing = 0.02
pitch_min = -85
pitch_max = 85
look = "look"            # the stick action that turns the view
mouse = true             # the pointer's moves turn it too (the default for a person rig)
bob = { stride = 2.2, vertical = 0.04, sway = 0.02, full_speed = 5.0, ease = 0.12 }

[[rig]]
name = "behind"
kind = "third_person"
target = "hero"
eye = [0, 1.5, 0]
third = { distance = 4.0, radius = 0.25, margin = 0.05, min_distance = 0.4, recover = 0.35, shoulder = [0.4, 0.1] }
view = "third"           # first or third at the start
collide = 4294967295     # the physics layers the camera collides with
```

Keys that belong to the other family are refused (`dead_zone` on a person rig, `bob` on a follow rig).

### The world

| Call | What it does |
|---|---|
| `world.rig(name)` | makes it the camera's driver and blends from the camera as it stands over its `blend`; `false` for an unknown name |
| `world.rig_off()`, `world.active_rig()` | back to the scene's camera, and which rig drives |
| `world.rig_target(name, Some(object))` | hands the rig over to another object, sliding over its `handover` (`None` leaves it with no target, at the origin) |
| `world.rig_look(mouse)` | adds raw mouse counts for the next tick, for a game that reads its own mouse |
| `world.rig_grounded(grounded)` | tells head bob whether the character stands on something |
| `world.rig_state(name)`, `rig_state_mut(name)`, `rig_desc(name)` | the rig itself (`Rig::Person(p).set_third(true)`), for a game to toggle the view or read its yaw |
| `world.set_view_aspect(aspect)` | the aspect a plane rig fits its bounds to; `PlaySession::present` sets it from the frame's |

Each tick, after the game's tick and the physics step, the active rig steps from its target's position (after the game and physics moved it), the action named by `look` (its `vector`), the mouse counts and the pointer's moves since the last tick, and a sphere cast through the world's physics that ignores the target's own body. At the end of every `advance` the camera is set from the rig's pose at the tick's `alpha`, and after a paused `step` from the newest tick. Presentation then draws it, shake included; stop leaves the scene's own camera untouched.

The pointer's moves are the layout position's difference between ticks, so a game that captures the cursor should feed its raw deltas through `rig_look` instead; pfx-input has no raw motion event yet.

## Tests

- `crates/core/src/rig/tests.rs`: dead zones, offsets, bounds (with and without a view size), damping convergence, per axis and across tick rates, look-ahead and its reduced-motion off switch, a plane rig's depth, pixel snapping at any alpha, handovers, pitch clamps (including past straight up), sensitivity, invert, smoothing, head bob and its off switches, third-person distance and collision against a wall, recovery, switching view, and every rig replaying bit for bit.
- `crates/play/src/tests/rig.rs`: the file's keys and refusals, the project's file, a game driving the camera and stop restoring it, alpha interpolation, the same ticks under different frame times, first-person clamps and bob through the world, comfort, third-person collision against a physics wall (and none in the open, nor against the target's own body), blending between rigs, target handover, the pointer turning the view.
- `crates/play/tests/rig.rs` (GPU, ignored): `a_follow_rig_keeps_a_moving_body_in_the_middle_of_the_frame` plays the neutral room with a body walking away and checks the body stays in the middle of the frame, against a control whose camera stays behind.

```
cargo test -p pfx-play --test rig --no-run
pgpu run --class interactive --as pfx -- cargo test -p pfx-play --test rig -- --ignored --test-threads=1
```
