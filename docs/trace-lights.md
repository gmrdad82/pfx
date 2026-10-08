# Tracer lights

How `pfx_trace` finds light: the sun, the sky, emissive surfaces and local lights. The live renderer's side of the same light list is in [live-renderer.md](live-renderer.md#bindings).

## The light list both renderers share

`pfx_trace::lights::LocalLight` has the live renderer's `LocalLight` fields with the same meanings and units, plus a shape:

| field | meaning |
|---|---|
| `position` | the light's position, or a rect's centre, in metres |
| `colour`, `intensity` | linear colour times intensity. For a point or spot it is radiant intensity, so irradiance at distance `d` is `colour·intensity·window²/max(d, radius)²` with `window = clamp(1 − (d/range)⁴, 0, 1)`, exactly live's `falloff` |
| `radius` | softens like live: the BRDF roughness becomes at least `√(radius / 2d)`, and the shadow ray aims at a random point of a disc of that radius facing the receiver, so shadows get a penumbra |
| `range` | live's window; ignored by a rect |
| `shadow` | `false` lights everything in range through geometry, as live does without a shadow map |
| `shape` | `Point`; `Spot { direction, inner_deg, outer_deg }` with glTF's cone falloff (`t = clamp(cos·scale + offset)`, squared); `Rect { half_u, half_v, two_sided }` with orthogonal half-edges, facing `half_u × half_v` |

A rect's `intensity` keeps the point light's unit along its normal: its radiance is `colour·intensity / area`, so a rect replacing a point of the same intensity looks as bright head-on. Live has only points today; `LocalLight::point(...)` with the live light's fields is the conversion (`shape: Point`), and the tracer crate does not depend on `pfx-live`.

Lights reach the tracer as `Detail::lights`. Emissive surfaces need nothing: every triangle or analytic shape whose material emits is found when the scene is built.

## Emissive surfaces

`SceneBuffers::build` lists every emitting triangle and analytic shape:

- a triangle emits when its material's packed emission is non-negative with positive luminance, the material has no active content layer (a content `Emit` layer varies across the surface, so those stay found by chance), and it is not `shadow_only`;
- a shape emits on the same material rule; the children of a smooth union are skipped and the union stands for them.

Each entry's weight is its power, area × luminance of emission (a shape's area is closed-form for spheres, capsules and boxes, Thomsen's approximation for ellipsoids, and the sum of its children for a union). The CDF over the list sits at group 0 binding 15, the entries at binding 14, with a header entry (counts and total power) first. A triangle carries its entry index in its own record (`c.w`) and a shape in its `d.y`, so a path that hits an emitter finds its sampling pdf without a search.

At every bounce the tracer draws one emitter from the CDF:

- a triangle is sampled uniformly by area; its pdf in solid angle is `p · d² / (area · |cos θ_light|)`; emission is two-sided, as the tracer has always drawn it; a point clipped by its instance's clip planes contributes nothing; a receiver in the emitter's own plane takes nothing from it;
- a shape is sampled by the solid angle of its bounding sphere (uniformly over the sphere of directions when the receiver is inside it); the direction counts only if the first thing it hits is that shape.

Emitters and area lights are weighed against BSDF sampling with the **power heuristic** (β = 2). A path that hits an emitter after a non-delta bounce takes the matching weight, `pdf_bsdf² / (pdf_bsdf² + pdf_light²)`, with the light pdf measured from the previous vertex; after a delta (glass) bounce or from the camera it takes weight 1. The sky keeps its balance heuristic.

## Local lights

Every local light is sampled at every bounce, as the live renderer loops over its lights:

- points and spots are delta lights, sampled directly with live's falloff, with live's check that the light is above the geometric normal;
- a rect is sampled uniformly by area, with MIS against BSDF sampling. A BSDF ray that crosses a shadowed rect (in front of whatever it hits next) picks up its radiance with the matching weight; rects are transparent to rays and to shadow rays, so the two strategies see the same light. Camera rays see a rect's front face as a glowing pane. An unshadowed rect is sampled only, with weight 1, since a BSDF ray cannot see through walls.

## Shadow rays

All shadow rays (sun, sky, emitter and local light) go through one any-hit traversal, `occluded`, over the scene's own BVH, triangles, clip planes and analytic shapes. It answers "is anything between here and there", stops at the first blocker and carries no hit record; it counts `shadow_only` triangles, as the sun's shadow always has. The answer is the one the closest-hit `scene()` gave. Sun and sky shadows are unchanged, and shadow rays are cheaper: a desk scene without emitters traces in 180.5 ms per sample against 213.4 ms before.

## One shader per scene

The light code lives in `crates/trace/src/lights.wgsl`. `trace.wgsl` carries no-op stand-ins for it between `trace_lights_start` and `trace_lights_end`. `Trace` swaps the real code in only when the scene has emitters or local lights, and only then binds 14 to 16. A scene without them compiles the stand-ins, so it pays nothing: on RDNA 4 the light code raises the kernel from 192 to 252 VGPRs and costs occupancy. The bake's batch shader includes `trace.wgsl` as it stands, so probe bakes keep the stand-ins and gain only the faster shadow rays.

Sampling a scene's lights draws random numbers only when the scene has lights, so a scene without them keeps its random sequence: the same seed gives the same bytes run to run.

## Cost

A desk scene (1586×992, 184,833 triangles, 3,080 of them emissive in two materials), median of single-sample dispatches after warm-up under `pgpu`:

| scene | before | after |
|---|---|---|
| emission zeroed (no emitters) | 213.4 ms | 180.5 ms |
| as authored (3,080 emitters) | 213.7 ms | 291.2 ms |

Emitter sampling adds one shadow ray per bounce and lowers occupancy, so a scene with emitters pays about 36% per sample here. In exchange each sample converges much faster: in a closed room lit by two softboxes, the per-pixel variance at 64 samples is 18.6× lower with emitter sampling. One sample of that scene costs about 180 ms without emitters and 290 ms with them; four samples with emitters took about 1.8 s per dispatch with an earlier version of this shader and hit the GPU's 2 s ring timeout. `Trace::sample` now sends each sample as Pacer-sized bands of a few milliseconds ([trace.md](trace.md#pacing)); splitting samples across dispatches does not change the bytes.

## Tests

GPU tests in `crates/trace/src/gpu.rs`, run under `pgpu`:

- `emitter_sampling_is_unbiased_and_quieter`: a closed room with two emissive softboxes and an emissive sphere, sun and sky off. The image mean with emitter sampling at 8×64 samples matches an 8192-sample BSDF-only render within 4σ and 2% (0.15967 against 0.15934, σ 0.00032), each 12×12 block within 5%, and the per-pixel variance at 64 samples is 18.6× lower than BSDF-only. The same seed gives the same bytes.
- `local_lights_match_the_live_direct_term`: a point light (radius 0.1), a spot and a rect over a diffuse-glossy floor with a black occluding sphere. Where the floor is unshadowed the trace matches live's point and spot formula, and a 48×48 quadrature of the rect with the shared BRDF, within 2% per pixel and channel (worst 0.52% over 1,851 pixels). In the point light's umbra it matches the rect alone (worst 0.26%).
