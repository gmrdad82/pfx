# Animation

Skeletal animation from glTF, node animation, and sprite clips from an image atlas, all deterministic by tick, drawn live with GPU skinning and traced in the same pose (the engine plan's block D4). Everything is generic: clips, atlases and settings come from a scene's data or a game's code, and nothing of any product is in the engine.

| Piece | Where | What |
|---|---|---|
| sampling, blending, crossfades, events, sprite clips, CPU skinning | `pfx_core::anim` | pure Rust, no dependencies, bit-exact across platforms |
| glTF skins, joints, weights, clips; `[object.animation]` | `pfx-load` | `Mesh::rig`, `Mesh::node_rig`, `Scene::animations` |
| GPU skinning | `pfx-live` (`skin.rs`, `frame.rs`) | linear blend skinning in the vertex stage of every mesh pass |
| sprite fills | `pfx-live` (`flat`) | `Fill::Image` and `FlatSprites` |
| playing it in a game | `pfx-play` | `world.animate(...)`, `world.play_clip(...)`, `Game::animated` |
| the traced pose | `pfx-trace` | `stage::scene_posed` with a `Posed` snapshot |

## The data

A scene object plays an animation through pfx-scene's `[object.animation]` (format 1, pfx-scene at `696888c`; its `docs/format.md` has every key):

```toml
[[object]]
name = "walker"
mesh = "walker"

[object.animation]
blend = [{ clip = "Walk", weight = 0.7 }, { clip = "Run", weight = 0.3 }]
events = [{ name = "step", clip = "Walk", time = 0.4 }]

[[object]]
name = "hero"
mesh = "card"
material = "sprite"
face_camera = true

[object.animation]
atlas = "art/hero.png"
grid = [8, 4]
fps = 12.0
clip = "idle"
events = [{ name = "step", clip = "run", frame = 2 }]

[object.animation.clips.idle]
from = 0
to = 3

[object.animation.clips.run]
from = 8
to = 15
fps = 16.0
```

pfx-scene checks the keys, names and kinds. pfx-load checks what only the assets can say and refuses at the key, with the file and line:
- a glTF `clip`, a `blend` clip and an event's clip must be clips of the object's mesh (`mesh walker has no clip Jog`);
- an event of a glTF clip takes a `time`; a time past the clip's end never comes and is dropped;
- a sprite's atlas must read as a PNG, its `frames` must lie inside it, and each clip's `to` must be one of its frames;
- an object takes a sprite or `content`, not both.

`Scene::animations` maps each animated object's name to an `Animation`: `clip`, `blend` (clip and weight pairs), `speed`, `playback` (`Loop`, `Once`, `PingPong`), `start`, and either `rig` (the mesh's clips with this object's events on them) or `sprite` (its `SpriteSheet` and the content key of its atlas). `Animation::still_frame()` is the frame a sprite shows before play.

### glTF

`Mesh::parse` reads `skins` (joints, inverse bind matrices, identity when the file has none), `animations` (translation, rotation and scale channels with their keys in `channels`, and morph weights channels in `weights`), each node's `skin` and decomposed `trs`, and each primitive's `JOINTS_0` and `WEIGHTS_0`. 

- **Skinned meshes.** `Mesh::rig(skin)` builds a `Rig`: the skin's joints ordered parents first (a vertex's joints are remapped with `SkinRig::joints`), each joint's parent the nearest joint above it, the root joints' common parent node as the skeleton's root matrix, and every animation that moves its joints as a clip named after the animation. A scene mesh holds one skin; its skinned parts keep their bind-space vertices with an identity part transform, weights normalised to sum to one, and `Geometry::skin` carries the joints and weights. `SceneMesh::rig` is the rig.
- **Node animation.** A mesh with animations and no skin gets a rig of its nodes (`Mesh::node_rig`): each walked node is a joint with an identity inverse bind, so its palette entry is the node's world matrix, and each part carries its node as `Part::joint` (and `Draw::joint`). A part then moves with its node: `Draws::models_posed` places it at the object's matrix times its node's pose.
- **Morph targets.** Each primitive's `targets` holds its `MorphTarget`s (`positions` deltas, and `normals` and `tangents` deltas when the file has them; a target without `POSITION` reads zero deltas), one delta per vertex, sparse accessors included and an accessor with no buffer view reading zeros. A target whose accessor count is not its primitive's vertex count is refused, as are deltas that are not float three-component or not finite, primitives of one mesh with different target counts, and more default weights than targets. `Group::weights` is the mesh's default `weights` as written (`Group::default_weights` pads the missing ones with 0) and `Group::targets` the target count. A `weights` animation channel is kept as a `WeightsChannel` (its node, interpolation, target count, key times and `times × targets` values, three times that for a cubic spline) and refused when its node has no mesh or its value count does not match; `WeightsChannel::sample(time, out)` samples it step, linear or cubic spline as glTF says. Node `weights` overrides are not read.
- **Morphed scene meshes.** Every walked node whose mesh has targets is a morph set of the scene mesh (in walk order, named after the node, with the mesh's default weights), its parts carry `Geometry::morph` (`GeometryMorph { morph, targets }`, hashed into the geometry), and the scene mesh always has a rig (`Mesh::morphed_rig` for a skin, `Mesh::morphed_node_rig` otherwise) whose `Rig::morphs()` lists the sets, even with no animation. A clip takes the animation's weights channels of the morphed nodes as `MorphChannel`s.

## Clips, poses and blending (`pfx_core::anim`)

- `Skeleton` holds joints (a name, a parent before it, a rest `Trs`), inverse bind matrices and a root matrix. `Rig` is a skeleton with named clips.
- `Clip` holds `Channel`s (a joint, a property, an interpolation, key times and values) and `ClipEvent`s. Sampling follows glTF 2.0:
  - **step** holds a key until the next;
  - **linear** lerps translation and scale and slerps rotation (the shorter way, nlerp within 0.9995);
  - **cubic spline** uses the Hermite basis with the keys' in and out tangents scaled by the key interval; rotations come back normalised;
  - before the first key the first value holds, after the last the last value.
- `Clip::with_weights` adds `MorphChannel`s (a morph set, an interpolation, key times and `width` weights per key), which `pfx_core::anim::sample_weights` samples by the same rules, and the clip's duration covers them. `Rig::morphed(skeleton, clips, morphs)` takes the `Morph` sets (a name and default weights) and refuses a channel that names a missing set or holds a different number of weights.
- `Pose` is a local `Trs` per joint. `globals` chains them through the parents from the root; `palette` multiplies each by its inverse bind, which is what skinning takes.
- `Animator` plays clips on a rig by tick, at the game's tick rate:
  - `play(clip, tick)` starts a clip; `crossfade(clip, seconds, tick)` starts it as a new mix that fades in over that many ticks, rounded, while the mixes before it keep playing underneath (at most `MAX_MIXES`, 4).
  - `blend(clip, weight, tick)` adds a layer to the newest mix: each layer mixes over the layers before it by its weight, only on the joints and properties its clip animates. `world.animate(id, "walk").blend("run", 0.3)` is 70% walk and 30% run wherever both move a joint.
  - `play_blend(&[(clip, weight)], tick)` shares the weights out by their sum, as `[object.animation] blend` says: `[(Walk, 0.7), (Run, 0.3)]` and `[(Walk, 7), (Run, 3)]` pose the same.
  - `set_speed` (0 or more; rebased so the clip time never jumps), `set_start` (seconds into the clip), `set_playback` and `set_weight` change a playing layer; `stop` returns the rest pose.
  - `weights(tick)` (and `weights_into`) gives one weights list per morph set: the defaults, each layer mixing its sampled weights over them by its weight, and newer crossfading mixes over older ones by their progress, as poses blend. Nothing playing gives the defaults.
  - Clip time is `offset + ticks × speed / rate`. A loop wraps at the clip's duration, once holds the last pose, ping-pong runs to the end and back.
- **Events.** `advance(tick)` returns every event whose mark play passed since the last call, in tick order: by mix, by layer, then by time. A loop fires on every pass; ping-pong on every pass in either direction; once only once; a clip started at `start` skips the marks before it. A stretch of skipped ticks still delivers each crossing, up to `MAX_REPEATS` (64) passes per mark.
- **Determinism.** Only `+ - * /`, `floor`, `round` and `pfx_core::sim`'s `sqrtf`, `sinf`, `cosf` and `atan2f` touch floats: no std transcendentals and no fused multiply-add, which a source guard test checks. Ticks are integers and seconds are formed in f64 from them. A scripted run (plays, blends, crossfades, a speed change, once) hashes the palette bits of 400 ticks to one golden value, the same in debug, release and under Wine.

### Sprite clips

- `Atlas::grid(size, cell, count)` cuts an image into cells, left to right then top to bottom; `Atlas::rects(size, rects)` takes pixel rectangles. `uv(frame)` is `[u0, v0, u1, v1]`.
- `SpriteClip` is a list of frames with an fps, a playback and `FrameEvent`s on its frames. `SpriteSheet` is an atlas with named clips.
- `SpritePlayer` plays one clip by tick: the step is `floor(offset × fps + ticks × speed × fps / rate)`, the frame the clip's frame at that step (looping, held at the end, or to and fro). `advance(tick)` fires each frame's events once per step that enters it.

### CPU skinning

`skin(positions, normals, tangents, joints, weights, palette)` blends each vertex's four palette matrices by its weights, in joint order from zero, as the GPU does, and transforms the position, normal and tangent (renormalised). It is the reference the GPU test holds the vertex stage to, and what the tracer's snapshot uses.

## Playing it (`pfx-play`)

The world gives every object with a rigged mesh an `Animator`, and every sprite object a `SpritePlayer`, at the physics tick rate. What `[object.animation]` plays starts at tick 0, with its speed, loop and start; an animator with nothing to play holds the rest pose and a sprite its first frame.

```rust
fn tick(&mut self, world: &mut World, _tick: &Tick, input: &Input) {
    let hero = world.object("hero").unwrap();
    if input.pressed("run") {
        world.animate(hero, "run").fade(0.2);
    } else if input.pressed("walk") {
        world.animate(hero, "walk").blend("run", 0.3);
    }
}

fn animated(&mut self, world: &mut World, events: &[AnimationEvent]) {
    for event in events {
        if event.event == "step" {
            world.play_sound("step");
        }
    }
}
```

| Call | What it does |
|---|---|
| `play_clip(id, clip)` | starts a clip now, a skeletal one with the object's `speed` and `loop` as defaults, or a sprite clip |
| `animate(id, clip)` | the same, as an `Animate` builder: `.blend(clip, weight)`, `.fade(seconds)` (crossfade from what played), `.speed(s)`, `.start(seconds)`, `.once()`, `.looped()`, `.ping_pong()`, `.playback(p)`, `.done()` for the first error |
| `stop_animation(id)` | the rest pose, or no sprite clip |
| `animator(id)`, `animator_mut(id)`, `sprite(id)`, `sprite_mut(id)` | the players themselves |
| `palette(id)`, `sprite_frame(id)` | the joint palette and the sprite frame at the current tick |
| `morph_weights(id)` | the object's weights per morph set at the current tick, with the game's set weights over them |
| `set_morph_weights(id, morph, weights)`, `clear_morph_weights(id, morph)` | hold the first weights of a morph set (named after its node) until cleared, over what the clips sample; more weights than targets or a weight that is not finite is refused |
| `animation_events()` | the latest tick's `AnimationEvent { object, clip, event }`s, by object |
| `animation_errors()` | this tick's refused calls (an unknown clip, a sprite asked to blend) |
| `posed()` | a `Posed` snapshot (object matrices with camera-facing ones turned, palettes, sprite frames) for the tracer |

An unknown clip is never silent: `.done()` and `play_clip` return the `AnimError`, and it is kept in `animation_errors()` until the next tick begins. A sprite takes `speed` and `start` but neither blends nor crossfades.

The tick runs `game.tick`, the physics step, `game.events`, then the animators and sprite players advance and `Game::animated` receives their events (only on ticks that have any), then the camera rigs. A clip a game starts in `tick` or `events` therefore plays from that tick, and its marks at 0 fire on it. The same scene, seed and inputs give the same poses and events, bit for bit, natively and under Wine (`the_same_ticks_give_the_same_poses_and_events_on_every_platform`, a golden hash over 240 frames of uneven length).

`present` poses the frame from the world: each skinned draw gets its object's palette and the previous frame's as an `InstancePose`, node-animated parts their node's pose, and each sprite object's surface its frame's UV offset, scale and crop. Each opaque instance of a morphed part comes back in `PlayFrame::weights` as a `MorphWeights { instance, current, previous }` (its object's weights for the part's morph set and the last presented tick's), the shape the renderer's weights take, and `present` has already handed them to `Renderer::set_weights`; a draw without an owning object gets none. Stop clears the poses and the weights and puts the staged surfaces back. Poses are taken at the latest tick, not interpolated between ticks.

## GPU skinning (`pfx-live`)

```rust
let mesh = renderer.upload_skinned_mesh(data, SkinData { joints: &joints, weights: &weights })?;
renderer.set_poses(&[InstancePose { instance: 0, joints: &palette, previous: &last_palette }])?;
```

- A skinned mesh's vertex records (four 16-bit joints and four f32 weights, 32 bytes) live in one storage buffer with every posed instance's palettes after them (`skin::SkinStore`, group 0 binding 4). Records are allocated with a free list, as sway ranges are; palettes are rewritten each frame. The instance carries a skin word: the record base, the current and previous palette offsets, and the joint count.
- The vertex stage blends the four joint matrices by their weights and skins the position, normal and tangent before the instance's model, in the opaque, prepass (velocity and ids), sun and local-light shadow, cut shadow and transmissive passes. Picking and shadows follow the pose.
- The prepass takes the previous palette for the previous position, so a moving joint writes its own motion vector and TAA holds; a still joint writes none.
- `set_poses` holds until the next call and counts as a change for frames on demand. An instance whose mesh is skinned and has no pose draws its bind pose. A pose with fewer joints than its mesh names is an error from `render`.
- Skinned instances are moving batches: they never enter the static shadow cache.
- `Staged` uploads skinned scene geometry with `upload_skinned_mesh` and shows a sprite object's still frame. It poses node clips itself (`[object.animation]`, [live-renderer.md](live-renderer.md#staged-scenes-clips-and-keys)) but sets no skin poses, so a skinned part draws its bind pose at its rest model; play's `present` skins it.
- Not covered: a skinned mesh does not also take a deformer or tree sway, cannot be replaced in place (`replace_mesh` refuses it; the staged path uploads a new mesh), and a skinned part with a glass material draws its bind pose in the glass pass. LOD sets, impostors and baked plates have not been tried with skinned meshes. Morphed meshes take prebuilt levels of detail from `MSFT_lod`, each level with its own targets ([live-renderer.md](live-renderer.md#prebuilt-levels-of-detail)); skinned ones do not.

The binding floor holds: the shadow casters' vertex stage now binds 7 storage buffers of the default 8, morph targets included ([gpu-floor.md](gpu-floor.md)), and `tests/binding_floor.rs` checks the layouts.

## Morph targets on the GPU (`pfx-live`)

```rust
let targets = [MorphTargetData { positions: &smile, normals: Some(&smile_normals) }, MorphTargetData { positions: &blink, normals: None }];
let mesh = renderer.upload_morphed_mesh(data, None, MorphData { targets: &targets })?;
renderer.set_weights(&[InstanceWeights { instance: 0, current: &[0.8, 0.0], previous: &[0.6, 0.0] }])?;
```

- Each target holds a position delta per vertex and, optionally, a normal delta per vertex, as glTF 2.0's morph targets do. The deltas live in their own storage buffer (`morph::MorphStore`, group 0 binding 5), vertex by vertex, one 16-byte record per target (two when any target has normals; a target without them adds none), allocated with a free list as skin records are. Every weighted instance's weights follow them.
- The vertex stage adds the sum of each weight times its target's delta to the rest position and normal before skinning, a deformer, tree sway and the instance's model, in the opaque, prepass (velocity and ids), sun and local-light shadow, cut shadow and transmissive passes. So a mesh can be morphed, skinned (`upload_morphed_mesh(data, Some(skin), morph)`), or both; the blended normal is normalised after skinning.
- Each instance's weights are current and previous: the prepass blends the previous weights for the previous position, so a changing weight writes its own motion vector and TAA holds; unchanged weights write none.
- Only weights that are not zero now or before reach the GPU: the instance's morph word is the delta base, the mesh's target count, its weights' offset and their count, and the vertex loops over those weights alone. An instance whose weights are all zero, or that has none, draws exactly the plain mesh, byte for byte. Fewer weights than targets leave the rest at 0; more is an error from `render`.
- `set_weights` holds until the next call; through `Renderer` the same weights again are a no-op and do not count as a change for frames on demand.
- Morphed instances are moving batches, as skinned ones are: they never enter the static shadow cache.
- `Staged` uploads a part whose `Geometry` has a morph with `upload_morphed_mesh` (with its skin when it has one) and never reuses a morphed mesh slot for other geometry, as with skinned ones. `Staged` alone sets no weights, so it draws the rest shape; play's `present` hands each tick's `PlayFrame::weights` to `set_weights` (instance indices as its poses), and stop clears them.
- Not covered: tangent deltas (the rest tangent is kept), replacing a morphed mesh in place (`replace_mesh` refuses it) and the glass pass (a morphed part with a glass material draws its rest shape there).

## Sprites on 3D cards and on the flat pass

- **Cards.** A sprite object's atlas is loaded as the object's content (`content_key(object)`, `sprite:<object>`), so it shows through its material's content layer as any content does: give it a material with a `content_layer`. Its surface's UV offset, scale and crop pick one frame (`pfx_load::scene::surface(frame)`).
- **Flat.** A game draws a frame with `Draw::sprite(centre, size, player.uv(world.ticks()).unwrap())` (or `Fill::Image` on any shape) over a `FlatSprites` atlas in `FlatScene::sprites`, and steps a `SpritePlayer` of its own by tick ([flat.md](flat.md)). A pfx-game game hands its atlas over once with `Config::sprites(image)`, and its `Ui` draws use it ([game.md](game.md)).

## The traced pose (`pfx-trace`)

`stage::scene_posed(scene, aspect, &posed)` stages a scene for `Trace` in a given pose: `posed.models` (or, when empty, the movers at `posed.time`), each object's palette (skinned parts are CPU-skinned into their own static meshes; node-animated parts are placed by their node's pose) and each sprite's frame. `scene_at(scene, aspect, time)` is `scene_posed` with `Posed::at(time)`. A game traces what it plays with `scene_posed(world.scene(), aspect, &world.posed())`.

## Tests

- `pfx-core` (`anim::tests`, `anim::tests_sprite`): linear, step, cubic-spline and slerp samples against hand-computed values; refusals; the rest palette; parent chains; blends, shared weights, crossfades, events in loops, once, ping-pong, skipped ticks and after a start offset; speed changes; CPU skinning; atlases, sprite steps, events and ping-pong; the golden hash; the source guard.
- `pfx-load` (`rig_tests`): a generated skinned glb round-trips to the engine fixture's rig; joints listed children first are reordered and remapped; a scene mesh carries its rig and skin.
- `pfx-load` (`morph_tests`): the generated head keeps its three targets (one sparse), default weights, hat child and weights channel, sampled linear and step; a cubic channel without tangents, a target shorter or longer than its primitive and too many default weights are refused; a scene head carries its morph set and an animator samples its weights.
- `pfx-core` (`morph_weights_sample_cubic_tangents_by_span_and_a_wrong_width_is_refused`): cubic-spline weights against hand-computed values, a layer's weight over the defaults, and a set of the wrong width refused.
- `pfx-play` (`tests::animation`): scene objects play their clips and fire events at fixed ticks; morph weights follow the clip per tick and a game sets and clears them; a sprite card steps its frames and a game switches its clip; a game's blends and crossfades match a reference animator; refused calls; the loader's refusals with their lines; the golden run natively and under Wine; the posed snapshot.
- `pfx-play`, GPU: `tests/morph.rs` (the generated head staged and played: weights set through `World::set_morph_weights` change its pixels, and weights `[0, 0, 0]` draw byte for byte the same frame as the head with its morph targets dropped).
- `pfx-live`, GPU: `tests/morphing.rs` (a generated sphere with two targets, one with normals, against its CPU-blended twin within 1/255 with its shadow and ids and no motion at unchanged weights; two instances of one mesh with their own weights; zero and missing weights byte for byte the plain mesh; motion vectors on a changing weight); `tests/skinning.rs` (the skinned fixture against its CPU-skinned twin within 1/255 with its shadow and ids, its bind pose without a pose, and motion vectors on a moving joint and none on a still one); `tests/posed_trace.rs` (the live silhouette against the traced snapshot in the same pose); `flat::sprite_tests` (the GPU against the CPU twin and its picks, and a sprite clip stepping a flat draw by tick).
- `pfx-live`, GPU: `tests/skinning.rs` (the skinned fixture against its CPU-skinned twin within 1/255 with its shadow and ids, its bind pose without a pose, and motion vectors on a moving joint and none on a still one); `tests/posed_trace.rs` (the live silhouette against the traced snapshot in the same pose); `flat::sprite_tests` (the GPU against the CPU twin and its picks, and a sprite clip stepping a flat draw by tick).

The fixtures are generated: `pfx_core::anim::fixture` (a chain of bones, a skinned square column and four clips, one per interpolation), `pfx_load::fixture::skinned_column` (that rig and column written as a glb with its skin and animations) `pfx_load::fixture::atlas_png` (a grid of flat cells) and `pfx_load::fixture::head` (a 561-vertex UV-sphere head with jaw open, smile and brow raise targets, the brow one sparse, default weights, a `talk` weights animation and a `hat` child node), which `cargo run -p pfx-load --features fixture --example head_glb` writes to `crates/load/tests/data/head.glb`.

```
cargo test -p pfx-core --lib anim
cargo test -p pfx-load --lib rig_tests
cargo test -p pfx-load --lib morph_tests
cargo test -p pfx-play --lib animation
cargo test -p pfx-live --test skinning --test morphing --test posed_trace --lib --no-run
pgpu run --class clip --as pfx -- cargo test -p pfx-live --test skinning --test morphing --test posed_trace -- --ignored --test-threads=1
pgpu run --class clip --as pfx -- cargo test -p pfx-live --lib flat::sprite_tests -- --ignored --test-threads=1
pgpu run --class test --as pfx -- cargo test -p pfx-play --test morph -- --ignored
```

## Not covered

Morph target tangent deltas, node `weights` overrides, quantized (integer) morph deltas, root motion, inverse kinematics, additive layers and joint masks, more than four influences per vertex, more than one skin per scene mesh, and interpolating poses between ticks.
