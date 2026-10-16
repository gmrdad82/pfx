# 3D particles

pfx's 3D particles are a deterministic emitter simulation in `pfx-core` and a lit, soft, blended billboard pass in the live renderer. A scene declares them with `[[particles]]` ([scenes.md](scenes.md#particles-3d-emitters)); a game can drive the same emitters from code. The flat view's 2D `Burst` ([effects.md](effects.md#bursts)) is the same recipe in layout units; this is it in metres and seconds.

## The simulation: `pfx_core::sim::particles`

An `Emitter` holds a `Recipe`, its attach point `at` (in the parent's local space), a seeded `Rng` and a pool of `Particle`s. The caller steps it with a fixed `dt` and the parent's transform at the start and the end of the step:

```rust
use pfx_core::sim::particles::{Emitter, Recipe, STEP};

let mut spray = Emitter::new(Recipe { rate: 600.0, inherit: 0.3, ..Recipe::default() }, [0.0, -0.3, -0.1]);
spray.step(STEP, wheel_before, wheel_after);
spray.sprites(&mut out);
```

- **Spawning.** Each step adds `rate · dt` (the trapezoid of `rate_keys` over the step when they are set) to a remainder and spawns its whole part, keeping the fraction for the next step, as bevy_hanabi's spawner does; `burst` particles spawn once, on the first step. Spawn `i` of `n` in a step is born at `(i + 0.5) / n` of it: its position is the attach point interpolated between the parent's two transforms at that fraction, and it is integrated over the rest of the step, so a car at 30 m/s leaves an even trail and no gaps between steps.
- **Direction.** A unit vector uniform in a cone of half-angle `cone` (degrees, 0 to 180) around `direction` turned by the parent's rotation, built on Duff et al.'s branchless basis as bevy_hanabi builds its circle basis. Speed is uniform in `speed`; `inherit` adds that share of the attach point's own velocity over the step (the parent's motion, a glTF node's animated motion when the emitter rides a node).
- **Motion.** Each step, as the flat `Burst`: `v = (v + gravity · dt) · exp(−drag · dt)`, `x += v · dt`, `angle += spin · dt`. Spin is uniform in `spin` with a random sign. Particles die at their life, uniform in `lifetime`, and leave by swap-remove.
- **The pool.** `max` caps the live particles; a spawn past it is dropped and counted in `dropped()`.
- **Over life** `t = age / life`: size goes linearly from `size[0]` to `size[1]` (metres across), alpha follows the `alpha` keys and colour the `colours` keys, both sampled as bevy_hanabi's `Gradient::sample` (linear between keys, held before the first and after the last). `Sprite` is what a renderer draws: position, previous position, velocity, size, angle, colour and alpha.
- **Determinism.** No wall clock and no global RNG: the same recipe, seed, steps and parent transforms give the same particles bit for bit. `Emitter::seek(time, step, parent)` steps on a fixed grid from 0 to `time` (resetting when time goes back), so the state at a time depends on that time alone; `STEP` is 1/120 s, which divides 24, 30, 60 and 120 fps frames.

Tests (`crates/core/src/sim/tests_particles.rs`): the same seed gives the same particles and seeking matches stepping; inherit carries the parent's velocity and gravity pulls as integrated; the pool cap holds and counts drops; a parent moving a metre in a step leaves an even trail; `rate_keys` drive the rate deterministically.

## In a scene

`pfx_load::scene::Scene::particles` holds each `[[particles]]` table as `Particles { attach, at, recipe }`. `attach` names the parent object and, with `node`, a glTF node of that object's mesh; pfx-load refuses a parent that is no object of the scene and a node that is not in its mesh, with the file, line and column. A node's world transform is the object's times the node's animated joint matrix when play mode poses it, else the node's own placement in the file.

`pfx_live::particles::Sprays` runs a scene's emitters in lockstep on the 1/120 s grid:

- `Staged::frame(aspect, time, seed)` seeks them to `time` with the objects posed by the scene's movers (`Staged::sprays()` exposes them), so the viewer, `pfx edit`'s viewport and anything that renders a `Staged` frame show them at that time; a scene watch reload restarts them when their tables or their parents change.
- Play mode (`pfx-play`'s presenter) steps them from the last presented time to this one, with the parents' transforms interpolated between the last frame's and this frame's poses (glTF node animation included).
- A catch-up longer than 120 s restarts from 120 s before the target, so a far seek stays bounded.

## The live pass

`Effects::sprays` carries `SprayInstance`s (80 bytes each); the renderer draws them in one instanced pass, `sprays`, after the effects' own particles and before the creatures, at full resolution. A frame with none draws nothing new and allocates nothing: the pass and its pipeline are made on first use, so an existing scene renders byte-identical.

- **Sorting and blending.** Instances are sorted back to front by view depth and drawn with premultiplied blending: `blend = "alpha"` covers what is behind by its alpha, `blend = "add"` writes alpha 0 so it only adds light. Both go through one pipeline.
- **Soft depth fade.** Each fragment fades with `clamp((scene depth − particle depth) / (size / 2), 0, 1)` against the linear depth, so a particle meeting a wall or the road fades out instead of cutting.
- **Velocity stretch.** With `stretch > 0` the quad turns to its velocity across the screen and lengthens by `stretch · speed` metres (seconds of travel), unspun; otherwise it spins by its angle.
- **Lit.** A lit particle shades a sphere normal across its disc with half-wrapped diffuse: the sun times its shadow cascades (a 2 × 2 tap of the cascade the particle's distance picks), the probe grid's irradiance where the scene has probes and the sky's spherical harmonics where it has none, and up to 16 local lights with the opaque pass's falloff (`window² / max(d, radius)²`, spots counted as points, no shadow), times `colour / π`. Unlit particles emit their colour.
- **Motion vectors.** A second target blends the particle's own screen motion into the velocity buffer, weighted by its alpha (`SrcAlpha, OneMinusSrcAlpha` on `Rg16Float`); the motion is the corner's current position under the unjittered view projection against its previous position under the last frame's, as the opaque prepass writes it. The motion blur and TAA passes read that buffer, so sprays blur along their own motion.
- **Bindings.** Group 0 holds the frame uniform, the linear depth, the instances, the shadow atlas and its uniform; group 3 the probe grid's own layout (`ProbeLighting::layout_entries`), with groups 1 and 2 empty. `crates/live/tests/binding_floor.rs` counts it as `live spray layout`. Only floor features: storage in the vertex stage, blending on `Rgba16Float` and `Rg16Float`.

The GPU test `particles_draw_lit_and_blended` (`crates/live/tests/sprays.rs`, under pgpu) checks that black alpha particles cover a glowing wall, that additive particles only add, and that lit particles brighten with the sun.

## Cost

Measured once with `spray_cost_at_2160x3840` (`cargo test --release -p pfx-live --test sprays -- --ignored spray_cost --nocapture` under `pgpu run --class bench`), a road spray at the shot's 2160 × 3840: particles 0.08 to 0.2 m across, lit, alpha 0.5, velocity-stretched, spread over a few metres about 5 m from the camera.

| Live particles | GPU `sprays` pass, median (p10 to p90) | CPU step and pack, median |
|---:|---:|---:|
| 2,000 | 0.45 ms (0.11 to 1.06) | 0.05 ms |
| 20,000 | 4.20 ms (1.08 to 10.31) | 0.45 ms |

The cost is overdraw: the 100 timed frames run from 1.3 to 3 s of scene time, as the spray thrown toward the camera spreads and falls, and the frames where it fills more of the screen are the slow tail. Fill, not count, sets the cost; the CPU side (two 1/120 s steps and the sprite list packed into 80-byte instances; the renderer then sorts them back to front) stays near 22 ns a particle. Measured on this machine's AMD RDNA 4 under Linux; other vendors are built but untested here.

## Credits

The spawner's fractional accumulation, the gradient sampling and the cone basis are ported from [bevy_hanabi](https://github.com/djeedai/bevy_hanabi) 0.19.0 (commit `cf35d53d54fb973e30c635e7097ff3e6abf33099`, MIT OR Apache-2.0, used under MIT), its GPU-free parts only; the integration, spin, size and alpha over life follow pfx's own flat `Burst`. See [INSPIRATION.md](../INSPIRATION.md) and [NOTICE.md](../NOTICE.md).
