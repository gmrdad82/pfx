# Physics

`pfx-physics` holds what the apps simulate: SPH fluid, ripples and Verlet rope, flocks and schools, and wind, all on the GPU or in plain Verlet steps. It also holds the 3D rigid bodies that games need, in the `rigid` module, described below.

## Rigid bodies

`pfx_physics::rigid` simulates 3D rigid bodies with collisions. It is built on rapier3d 0.36.0 and parry3d 0.31.1, both pinned exactly, behind the engine's own types: positions are `[f32; 3]`, rotations `[x, y, z, w]` quaternions, and bodies and colliders are `BodyId` and `ColliderId`. No rapier type appears in the API, so the backend can change without touching a caller.

It is compiled when the physics crate's `rigid` feature is on, which it is by default. An app that wants none of it depends on the crate with `default-features = false`. The `enhanced-determinism` feature turns on rapier's and parry's cross-platform determinism, for runs that must match across machines.

### The world

```rust
use pfx_physics::rigid::*;

let mut world = World::new(WorldDesc {
    rate: 60.0,
    substeps: 1,
    iterations: 4,
    gravity: [0.0, -9.81, 0.0],
    ccd: true,
});
let floor = world.add_body(BodyDesc::fixed(Pose::at([0.0, -0.5, 0.0])));
world.add_collider(floor, ColliderDesc::cuboid([50.0, 0.5, 50.0]));
let card = world.add_body(BodyDesc::dynamic(Pose::at([0.0, 1.0, 0.0])).with_locks(Locks::PLANE_XY));
world.add_collider(card, ColliderDesc::round_box([0.5, 0.05, 0.35], 0.02));
world.step();
let state = world.body(card);
```

- `World::step` advances one fixed step of `1 / rate` seconds (60 Hz by default), split into `substeps` equal solver steps. The caller decides how many steps to take for the time that passed, for example with `pfx_core::motion::Stepper`; the world never reads a clock.
- `WorldDesc::iterations` is the solver's iteration count per substep. `WorldDesc::ccd` is the world's continuous-detection switch (see Colliders).
- `gravity` / `set_gravity` is the world gravity vector; `time()` and `steps()` count the steps taken.

### Bodies

`BodyDesc::dynamic`, `kinematic` and `fixed` start a body at a `Pose`:

| field | meaning |
|---|---|
| `kind` | `Dynamic` (simulated), `Kinematic` (moved by its pose, pushes dynamic bodies), `Fixed` |
| `linear_velocity`, `angular_velocity` | the starting velocities |
| `gravity_scale` | how much world gravity and gravity sources act on it (1 by default) |
| `linear_damping`, `angular_damping` | velocity damping |
| `locks` | translation and rotation axes held fixed, per axis |
| `ccd` | continuous detection against every body, for fast projectiles |
| `can_sleep`, `asleep` | whether it may sleep, and whether it starts asleep |
| `mass` | `Some(Mass { mass, center, inertia })` sets mass, centre and principal inertia; `None` derives them from the colliders' densities |

On a body: `set_pose`, `set_velocity`, `apply_impulse`, `apply_impulse_at` (a world point), `apply_torque_impulse`, `add_force`, `add_force_at`, `add_torque`, `set_gravity_scale`, `set_damping`, `set_ccd`, `set_locks`, `sleep`, `wake`, `remove_body`. Impulses change the velocity at once. Forces and torques act through the whole of the next `step` and are then cleared, so a steady force is added every step. `body` returns a `BodyState` with the pose, both velocities and whether it sleeps; `mass` and `is_asleep` answer directly.

#### Locked axes

`Locks { translation: [bool; 3], rotation: [bool; 3] }` locks any of the three world translation axes and three world rotation axes; `Locks::PLANE_XY` keeps a body in its z plane turning only about z, and `Locks::ROTATION` stops all turning. The solver treats a locked axis as having infinite mass, and after every substep the world puts the locked coordinates back exactly where they were when the body was made, posed with `set_pose` or given new locks with `set_locks`, and zeroes the locked velocity components. A locked translation coordinate never changes by a single bit, and with one free rotation axis the rotation stays a pure turn about it from that anchor (with an identity anchor the other quaternion components stay exactly zero; tested over 10,000 steps of collisions, impulses and spin). With only one rotation axis locked, the angular velocity about it is held at zero.

### Gravity sources

Any number of `Source`s act on every awake dynamic body, scaled by its gravity scale and sampled at its centre of mass. Each substep starts with their acceleration given to the body as a change of velocity, so the integration is symplectic Euler and an orbit stays bounded (a circular orbit held within 5% of its radius over ten turns, tested). `add_source` and `set_gravity` wake every sleeping body; changing a source through `source_mut` does not, so call `wake` for the bodies it should reach. There are two kinds:

- `Source::Point { center, strength, falloff, range, softening }` accelerates towards the centre by `strength` (negative repels) times `1`, `1/d` or `1/d²` (`Falloff::Constant`, `Inverse`, `InverseSquare`), within `range`, with the distance held at least `softening`. `Source::attractor(center, strength)` is an inverse-square point with no range limit.
- `Source::Zone { region, gravity, replace }` applies inside a `Region::Sphere` or a posed `Region::Box`. A replacing zone takes the place of world gravity inside it (the last added wins where they overlap); an adding zone adds to it.

`add_source` returns a `SourceId` for `source_mut` and `remove_source`.

### Colliders

`add_collider(body, ColliderDesc)` attaches a shape at an `offset` pose and returns its `ColliderId`, or `None` when the body is gone or a hull is degenerate.

- `Shape::Sphere`, `Cylinder` and `Capsule` (along y), `Disc` (a thin cylinder by radius and thickness), `Box` and `RoundBox` (outer half extents and a corner radius, for cards) and `Hull` (the convex hull of points).
- `density` gives the mass when the body's mass isn't given.
- `friction: Friction { still, sliding, combine }` has a static coefficient, used while the contact's tangential speed is under `Friction::SLIP` (0.01 m/s), and a dynamic one above it. `restitution` and `restitution_combine` set the bounce. A pair combines with the stronger of the two rules, in the order `Average`, `Min`, `Multiply`, `Max`.
- `sensor` makes the collider report overlaps without touching anything; sensors see every kind of body.
- `layers: Layers { member, mask }`: two colliders meet when each one's member bits meet the other's mask.
- `impulse_threshold` turns on `ContactImpulse` events for the collider's contacts whose impulse in a substep passes it.

Continuous detection has two tiers. With `WorldDesc::ccd` on, any dynamic body fast enough to tunnel is swept against fixed colliders automatically. A body with `ccd` set is swept against kinematic and dynamic bodies too. With the world switch off, nothing is swept.

### Kinematic motion

A kinematic body moves to a target pose each step and pushes the dynamic bodies it meets; substeps move it there in equal parts.

- `move_to(body, pose)` sets the target for the next step.
- `set_drive(body, Some(Drive::Spring(..)))` follows a `pfx_core::motion::Spring<[f32; 3]>`: change its `target` through `drive_mut` and the body follows the spring's value, keeping its rotation.
- `Drive::ease(from, to, duration, curve)` eases position and rotation from one pose to another over `duration` seconds through any `fn(f32) -> f32`, such as `motion::cubic_in_out`.
- `drive(body)` reads the drive back; `settled()` says when it has arrived.

### Queries

Queries see the colliders as `step` left them, and as `refresh_queries()` leaves them: it moves each collider to its body's current pose and rebuilds the query structure without stepping, so a collider added or a body placed with `set_pose` shows in queries at once, before any step. A `Filter { mask, sensors, exclude }` keeps colliders whose member bits meet `mask` (the query's own mask decides, whatever the collider's collision mask is, so a layer that collides with nothing is still found), skips sensors unless `sensors` is set, and can leave out one body.

- `raycast(origin, direction, max_distance, filter)` returns the first `Hit { collider, body, distance, point, normal }`.
- `shape_cast(shape, pose, direction, max_distance, filter)` sweeps a shape and returns the first `Hit`, with the point on the swept shape at impact.
- `point_overlaps(point, filter)` and `shape_overlaps(shape, pose, filter)` return the colliders that contain the point or overlap the shape, sorted by id.

### Events

`events()` lists what happened during the last `step`; the next step clears it.

- `ContactStarted { a, b, impulse }`: two colliders started touching, with the total normal impulse their contact applied during the rest of that step.
- `ContactStopped { a, b }`.
- `ContactImpulse { a, b, impulse }`: the step's summed impulse of a contact that passed a collider's `impulse_threshold`.
- `SensorEntered { sensor, other }` and `SensorExited { sensor, other }`.

`body_of(collider)` gives the body a collider belongs to.

### Determinism

The same world built with the same calls in the same order, stepped the same number of times, gives bit-identical poses, velocities, sleep states and events on one machine and one build. The wrapper walks its bodies in index order, runs on the calling thread and never reads the clock or system randomness, and rapier is deterministic for the same inputs in the same order without its `parallel` feature, which stays off. Two worlds stepping a scene of 43 bodies with a gravity source, a locked card, an eased kinematic sweeper and impulses for 1,000 steps compare equal bit for bit (`rigid::tests`). For runs that must match across machines, build with the `enhanced-determinism` feature, which makes rapier and parry use cross-platform floating-point paths.

### Cost

500 spheres of 5 cm radius with CCD, thrown down at 10 m/s into a 1.6 m box at 60 Hz, cost 0.78 ms per step on the CPU while they fall and pile up (the first second), and 0.18 ms per step over the next nine seconds as they settle and sleep (one thread, release build, Ryzen 9 9950X, 2026-10-05). `cargo test --release -p pfx-physics --lib five_hundred_spheres_timed -- --ignored --nocapture` prints it; the debug build in the gate takes about 23 ms per step for the same scene.
