# Tracer cameras and staging

The offline path tracer (`crates/trace`) takes a `Camera` (origin, forward, right, up) and a `Projection` that chooses how each pixel becomes a primary ray. The tracer's `Scene` and the live renderer's `Scene` can be staged from one set of mesh arrays, instances and materials.

How the tracer samples the sun, the sky, emissive surfaces and local lights is in [trace-lights.md](trace-lights.md). How it paces its submissions is under [Pacing](#pacing) below, and the least GPU it runs on is in [gpu-floor.md](gpu-floor.md#the-tracers-floor).

The tracer's Cycles references (`crates/trace/film/cycles.csv`, `wavelength.csv` and `cycles_sheet.csv`) are frozen data: rendered once in Blender 5.2.1, kept with their provenance in `crates/trace/film/README.md`, and read by the tests as files. No test, build script or doc runs Blender, so the machine needs none to gate the tracer.

## Projections

`Camera`'s four fields and every struct literal that builds one stay as they were. The projection is carried next to the lens in `Detail::projection` and changed on a running trace with `Trace::set_projection`, which, like `set_camera`, restarts the accumulation. It reaches the shader in the `w` lanes of the `Frame` uniform's `origin` (the kind), `right` and `up` (an orthographic view's half width and height), which were zero, so the uniform's layout, the bindings and the bake's batch shader that shares the struct are unchanged.

| `Projection` | primary ray | lens |
|---|---|---|
| `Perspective` (default) | `forward + right·(x + shift.x) + up·(−y + shift.y)` from `origin`, with `x`, `y` in −1…1 across the image; `right` and `up` carry the half-angle tangents | shift and depth of field, as before |
| `Orthographic { width, height }` | along `forward`, from `origin + r̂·(x + shift.x)·width/2 + û·(−y + shift.y)·height/2` | shift in the same NDC units; aperture is ignored |
| `Equirectangular` | the full sphere, see below | ignored |

- An orthographic view's `width` and `height` are the full view in metres; rays start on the plane through `origin`, so nothing behind that plane is seen. A 1 m face in a 2 m view across 64 pixels covers exactly 32 of them at any depth (`tests/cameras.rs`).
- Every projection keeps the jitter, the per-pixel seeds, the albedo and normal outputs (first hit, averaged over the samples) and therefore the denoiser.

### Equirectangular

The pixel at `(u, v)` (0…1 across, 0…1 down) looks along

```
φ = (u − ½)·τ,  θ = v·π
local = (sin θ sin φ, cos θ, −sin θ cos φ)
direction = r̂·local.x + û·local.y − f̂·local.z
```

`pfx_trace::equirect_direction(u, v)` is `local`. It is the exact inverse of the engine's equirect lookup (`u = atan2(x, −z)/τ + ½`, `v = acos(y)/π`, in `pfx_live::sky::SkySource::radiance` and `pfx_trace::sky::Environment::radiance`). With the world-aligned camera (`forward = −z`, `right = +x`, `up = +y`) the image is the world's sky: the centre looks along −z, a quarter to the right looks along +x, the top row is +y. Any other basis turns the panorama with it: the centre is always `forward`, the top `up`, a quarter to the right `right`. Render it 2:1 (`width = 2·height`) so its texels are square in angle.

**Round trip.** A traced equirect is a `pfx_load::Sky` as it stands: its colour texels, in row order, are the sky's texels. `tests/cameras.rs` traces an emitter-only room (six walls of different emission and a bright softbox) and an analytic daylight sky to 512×256, loads each as a `Sky`, and compares `radiance(direction)` with a direct narrow-field trace along 96 Fibonacci directions. Every direction whose 3×3 texel neighbourhood is flat within 2% (92 in the room, 63 in the sky, which skips 5° around the sun and the horizon's step) agrees within 1%. The room's albedo coverage is 1 at every pixel and every normal faces its ray.

### Blender's panoramic camera

A Blender script that renders a sky with a `PANO` camera, `panorama_type = "EQUIRECTANGULAR"`, at the origin with `rotation_euler = (π/2, 0, −π/2)`, written as a 2:1 Radiance HDR, lines up with the engine as follows.

- **Blender's camera is the engine's camera.** Cycles turns a panoramic camera so that the image centre is the camera's view (its local −Z), the top row its local +Y and a quarter to the right its local +X, longitude growing to the right, the top row the up pole. That is the engine's equirect for `forward`, `up` and `right` respectively.
- **That rotation** points the view at Blender +X, with Blender +Z up and Blender −Y to the right; that is the orientation Blender's own environment texture uses (its centre is +X).
- **In engine axes.** With glTF's Blender-to-engine axes, `(x, y, z) → (x, z, −y)`, that camera is `forward = (1, 0, 0)`, `up = (0, 1, 0)`, `right = (0, 0, 1)`: the engine's world-aligned camera yawed by −90° about +Y. Tracing with that basis gives Blender's image pixel for pixel. Traced with the world-aligned camera instead, the image is Blender's a quarter turn on: `u_engine = u_blender + ¼`, so `Sky::parse(blender_hdr).rotated(π/2)` is that same world-aligned sky.
- **Softboxes.** A softbox at azimuth `az` and elevation `el` placed at Blender `(cos el cos az, −cos el sin az, sin el)·distance` (y flipped) is `(cos el cos az, sin el, cos el sin az)·distance` in engine axes, facing the origin; under that camera azimuth 0 is the image centre and azimuth 90° a quarter to the right. A room box spanning `−floor` to `size.z − floor` on Blender z spans that on engine y. A spec whose own axes are z up with azimuth from +x towards +y reaches the engine mirrored, `(x, y, z) → (x, z, y)`, when y is flipped before Blender's axes turn.

`an_equirect_follows_the_camera_basis` in `tests/cameras.rs` pins the yaw: a red panel on −z and a green one on +x sit at the centre and a quarter right under the world-aligned camera, and at a quarter left and the centre under Blender's.

## One staged scene

`pfx_trace::stage::Stage` builds a tracer scene from the same arrays the live renderer uploads, and does not depend on `pfx-live`:

```rust
let staged = Stage {
    meshes: &[Mesh { positions, normals, tangents, uvs, alpha, indices }],
    instances: &[Placement::new(0, model, 2)],
    materials: &materials,
    sky,
    sun,
    camera,
    projection: Projection::Perspective,
    lens: Lens::default(),
}
.build()?;
let mut trace = staged.trace(&gpu, width, height)?;
```

- `stage::Mesh` has `pfx_live::frame::MeshData`'s fields, with the same checks (vertex and index counts, indices inside the vertex buffer), so a caller converts one into the other field by field.
- `Placement` is an instance: a mesh index, a column-major model matrix as `Instance::model` takes it, a material index, `casts_shadow`, optional `content`, two `clip` planes, `shadow_only`, `alpha_cutoff` and `two_sided` (see [Placements](#placements) below). `Placement::new(mesh, model, material)` casts shadows, carries no content, clips nothing, is seen by the camera, cuts at `stage::ALPHA_CUTOFF` and is two-sided. A mesh shared by several instances is converted once for each cutoff and placed by each instance's transform, with normals by the inverse transpose.
- `materials` are `pfx_materials::Material`, the ones the live renderer binds; `sky` is anything the tracer takes (`Sky`, or an `Environment`).
- `build` returns `Staged { scene, detail }`: the `Scene` keeps materials, sky, camera and sun, and the triangles, with their per-vertex normals and UVs, travel as `Detail::instances`, since the tracer's flat `Scene::triangles` carry neither. `Staged::trace` makes the `Trace`.
- `stage::scene(scene, aspect)` and `scene_at(scene, aspect, time)` stage a loaded scene file; `scene_posed(scene, aspect, &posed)` stages it in a given pose, a `pfx_load::scene::Posed` of object matrices, joint palettes (skinned parts CPU-skinned into static meshes, node-animated parts placed by their node) and sprite frames, as a playing world's `posed()` gives it ([animation.md](animation.md)).
- Per-vertex alpha is cut where the live renderer discards, at that placement's `alpha_cutoff` (default `stage::ALPHA_CUTOFF`, 0.5, the same default as live's `Instance::new`): a triangle's alpha is linear across it, so the kept part is the polygon on the side where alpha is at least the cutoff, split exactly on that line and fanned into triangles with interpolated normals and UVs. Two placements of one mesh at different cutoffs keep different polygons; the same cutoff is converted once.
- Tangents are checked but not used: the tracer derives each triangle's tangent from its UVs.

`crates/live/tests/staged.rs` stages a red Lambert sphere, a turned green cube, a blue emissive panel and a dark screen slab showing a 48×32 sRGB image through a `Content::Screen` layer once, traces them (256 samples) and draws them live with the same arrays, materials, sun and a black sky; the live side puts the image in content slot 0, the staged side hands it to the slab's `Placement::content`. Over the pixels whose 3×3 neighbourhood shares one live instance id, every instance's mean colour agrees within 5%.

### Placements

What a placement adds to its instance lands on the tracer's `detail::InstanceSurface`, beside the clip planes and content UVs it already had, and from there in one word per triangle, the `w` of its second vertex that held only the shadow-only flag: bit 0 shadow only, bit 1 casts no shadow, bit 2 cutout, bits 3–4 the content face, bits 5–22 the placement's own content layer plus one (0 for the material's, 18 bits, up to 2^18 layers) and bit 23 one-sided (0 means two-sided, which is the default). It is stored as an exact integer in an `f32` (the largest word is 16777215); the bindings, the buffers' layouts and the bake's batch shader are unchanged.

- **`casts_shadow`** (default `true`), live's `Instance::casts_shadow` and a recipe's `shadow = "none"`. When it is false no shadow ray sees the instance: the sun's, the sky's, the emitters' and the local lights', all of which go through `occluded`, and the steam's sun rays. Camera and BSDF rays still hit it, so it renders, reflects and blocks indirect light as before.
- **`shadow_only`** (default `false`), live's `Instance::shadow_only`. Camera and BSDF rays miss the placement, and shadow rays hit it. `Stage::build` copies the flag onto the instance.
- **`clip`**, two planes, default `[[0.0; 4]; 2]`, live's `InstanceSurface::clip`. A plane clips when any of its first three components is non-zero and `dot(normal, point) > w`. `Stage::build` copies the pair onto the instance's surface. A non-finite component is refused.
- **`alpha_cutoff`** (default `stage::ALPHA_CUTOFF`, 0.5), live's `Instance::alpha_cutoff`: a fraction from 0 to 1. It is the vertex-alpha cut above and the content-alpha cutout below. Alpha below the cutoff is discarded and alpha equal to it is kept. A non-finite value, or one outside `0.0..=1.0`, is refused. Placements of one mesh at different cutoffs are cut separately.
- **`two_sided`** (default `true`). Live's `Instance::two_sided` defaults to false and culls `Face::Back` unless the instance says otherwise (or a deformer is drawing it, which the tracer does not infer). The tracer stays two-sided unless the placement says `two_sided: false`, so a stage that never sets the field keeps today's flag word and the same frames. Front is the counter-clockwise winding, the same side live's `FrontFace::Ccw` draws. A camera ray or a BSDF ray that meets a one-sided back face misses and continues, so an orbit behind a one-sided wall sees through it, and a bounce does not reflect the face the live G-buffer never stored. A shadow ray still hits that back face: live's shadow casters, cutout casters, local-light casters, transmission casters and the cascade depth passes all use `cull_mode: None`, so the wall still throws a solid shadow onto the floor behind it. Analytic shapes stay two-sided. Marking a closed mesh one-sided removes the exit face, so the volume does not contain its interior; leave the default on for that. Thin transmission still leaves from the front hit along the refracted direction.
- **`content`**, a `PlacementContent`: its `kind`, `ContentKind::Image` (RGBA8 at any size, `srgb` decoding its colour like an `Rgba8UnormSrgb` texture, alpha always linear) or `ContentKind::Text` (glyphs drawn from their atlas at every hit, see [Text](#text)), `uv_offset`, `uv_scale` and `crop` as live's `InstanceSurface` takes them, `face` (`ContentFace::Both`, `Front` or `Back`, front being the side a triangle's counter-clockwise winding faces) and `cutout`. `PlacementContent::new(image)` shows the whole image on both faces with no cutout; `PlacementContent::text(text)` does the same with text.
  - The image is blended by the placement's material's `content_layer` (its blend, ink roughness, emboss and strength), with the slot pointed at the image: a `Content::Screen` material adds the image to its emission, a plain material lays it `Over` the base.
  - Images go into `Detail::content_slots` and text into `Detail::text_slots` at the same index, one or the other per slot. Placements whose images are equal (same size, encoding and texels; the same slice is recognised without comparing) share one slot. The slots start after the highest slot any material's own `content_layer` names, so a material's layer keeps reading an empty slot, as it did before, on placements without content. The tracer's slots are records in one storage buffer, not textures, so any number of images fit and none has to be packed into a shared one; `Image::linear` is the conversion.
  - Each placement's layer is appended to the tracer's content layers after the materials' own, and its index sits in its triangles' word, so two placements of one material can show different images.
- **The cutout.** With `cutout` set, a hit on the placement whose content alpha is below that placement's `alpha_cutoff` (bilinear between texels) is no hit: camera, BSDF and every shadow ray pass through, so the placement and its shadow keep only the shape of its content, seen from either face. Outside the crop, and on a placement without content, the alpha is 0, so a cutoff of 0 keeps those texels. This is the carrier for free-standing text: a quad carrying its glyphs as content. Cutout and content placements are never sampled as emitters, since the sampler would count the whole triangle. The cutoff rides the spare lane of the per-triangle uv record, `triangle_uv_v.w`, which stored 0. The bindings, the buffers' layouts and `trace_floor()` are unchanged. At the default 0.5 the comparison stays the literal content alpha below 0.5, and a stage that leaves `clip`, `shadow_only`, `alpha_cutoff` and `two_sided` at the `Placement::new` defaults traces the same frames as before these fields existed.

A ray leaves a hit by the offset in Wächter and Binder, "A Fast and Robust Method for Avoiding Self-Intersection" (Ray Tracing Gems, chapter 6): per component, add `n / 65536` when the coordinate is inside 1/32, otherwise add or subtract 256 ULPs of that component along the outgoing direction, then add `direction · 4e-7 · (|origin| + t)`. There is no 0.5 mm floor. The same offset is the previous point of the emitter MIS weight and the start of a steam shadow ray. The hit test still rejects `t` at or below 0.1 mm, and the shadow limit still ends `distance · 0.9999 − 0.0002` short of a shape light. Near the origin the step is about 0.02 mm, so a shadow-casting quad 0.5 mm above a face is a real occluder and the face's own shadow ray starts in front of it. The `4e-7` term is what keeps a camera 2,600 units away from lighting a plane with itself (`a_far_camera_does_not_light_a_plane_with_itself`). At tens of metres, 256 ULPs approaches 0.5 mm, so a carrier that tight belongs near the origin.

`crates/trace/tests/placements.rs` traces a top-down orthographic floor under a low sun (1, 0.5, 0) with a red sphere above it: with `casts_shadow` false the floor where its shadow fell reads within 2% of the floor with no sphere, while the sphere reads as with its shadow on. A black 1 m quad at 0.5 m carrying a 64×64 red ring (alpha 1 between radii 0.3 and 0.45 of the quad, 0 elsewhere) on its front face as cutout content shows the ring, darkens the floor only under the ring's shadow, and leaves the floor seen through the hole, through its clear corners and lit through the hole within 3% of the floor with no quad (it reads equal); the same quad without the cutout is solid in both. The quad is black so that the ring's own bounce stays out of the comparison: shown on both faces, its red underside lights the floor beneath by about 8% in red. `crates/trace/tests/one_sided.rs` puts a one-sided wall in front of an emissive panel and checks the live frame against the trace from both sides (the back shows the panel; the same wall with `two_sided: true` shows the wall), and that the wall still darkens the floor on the far side of the sun in both renderers. It also traces a quad 0.5 mm above a floor: with `shadow_only` and `casts_shadow` the floor under it is dark, without `casts_shadow` that floor matches the open floor, and the quad's own top matches a quad at 5 cm.


## Transmissive shadows

By default a shadow ray stops at the first surface it meets, glass included. That is Cycles' rule: glass is not a transparent BSDF, so in Cycles and in the tracer the sun, the sky and a local light reach a floor under a pane only by paths, and a delta light (the sun, a point or spot light) never does. The floor in a glass pane's shadow reads dark, which is what the clear-sheet tests in [trace-film.md](trace-film.md) compare against Cycles. The live renderer instead tints the shadow by the glass's `cast_tint` ([live-renderer.md](live-renderer.md)). **`Detail::transmissive_shadows`** (default `false`) makes the tracer use live's model, for a viewer that must match live:

- **What passes.** Every shadow ray toward the sun, the sky, an emitter or a local light walks the whole BVH and every analytic shape instead of stopping at its first hit. A surface whose material has `transmission > 0` (glass, liquid) multiplies the ray by its tint, and any other surface blocks it. Placements' `casts_shadow`, `shadow_only`, clip planes and cutouts apply as they do to an opaque shadow ray.
- **The tint.** It is live's `glass::cast_tint`: `transmission × base`, or `transmission × beer(thickness, base, absorption)` when thickness and absorption are both above zero, each channel clamped to 0–1, with no Fresnel term, as live applies it. Live tints by the nearest caster once per closed glass object. The tracer has no idea of an object, so each triangle crossing multiplies by `√cast_tint`: a closed pane, crossed once in and once out, passes `cast_tint`, as live's map does, and as the tracer's own camera paths take `√base` per crossing. A single open sheet passes `√cast_tint`. An analytic shape is closed and is found once along the ray, so it multiplies by `cast_tint` itself.
- **No double counting.** The light a shadow ray now carries through glass would otherwise come a second time by paths: a diffuse bounce that refracts through the pane and reaches the sky, a rect light or an emitter. A path whose last scatter before such a hit was a diffuse or glossy vertex followed only by refractions drops what it hits there (the sky when it is sampled, rect lights and sampled emitters). A mirror reflection on the way keeps it, since no shadow ray covers that direction. Camera rays through glass keep what they see.
- **Russian roulette.** A shadow ray that walks every node to the sky costs far more than one that stops at the first pane. In a deep glass stack most path vertices are glass, whose light sample is only a narrow specular lobe, so most of those rays carry almost nothing. Each shadow ray is kept with probability `min(1, 16π · max(unshadowed) / max(throughput · strength))` and divided by it when kept, which is unbiased and keeps a diffuse floor's ray (whose ratio is near `16 · albedo · cos`) every time.
- **Same bytes when off.** The switch is a pipeline-overridable constant, `TRANSMISSIVE_SHADOWS`, set only when the option is on, so the off pipeline gets exactly the constants it had, its shader takes the old `occluded` branch and draws no extra random numbers. `the_option_off_keeps_the_opaque_glass_shadow_bytes` pins the FNV-1a hashes of a glass pane traced with the option off to the ones main gave before the option existed (`ec54cf52…`, `41efcdf1…`, `b632fda5…`), and `paced_traces_keep_the_bytes_of_eight_row_bands` still matches its pins. The bake's batch shader shares `trace.wgsl` and keeps the default.
- **The scene file.** `[trace] transmissive_shadows = true` in a `scene.toml` ([scenes.md](scenes.md#trace)) sets it on the `Detail` that `stage::scene` builds.

`crates/trace/tests/glass_shadows.rs` puts a closed 1 × 1 × 0.4 m glass slab above a floor and lights it once by the sun alone and once by a 2.5 W local light alone, with the camera above. For each, it renders live (`glass::Surface` with `casts_shadow`) and traces with the option on and off, and reads the floor in the slab's shadow over 5 × 5 pixels as `(with glass − dark) / (without glass − dark)`. Two panes: a tinted one (`base [0.5, 0.62, 0.95]`, transmission 0.9, `cast_tint` [0.450, 0.558, 0.855]) and an absorbing one (`base [0.85, 0.7, 0.6]`, transmission 0.95, thickness 0.4, absorption 2, `cast_tint` [0.843, 0.747, 0.690]).

| pane, light | live | traced, direct light | traced, reference bounces | traced, option off |
|---|---|---|---|---|
| tinted, sun | 0.451, 0.557, 0.854 | 0.450, 0.558, 0.855 | 0.460, 0.570, 0.873 | 0.009, 0.010, 0.013 |
| tinted, local light | 0.452, 0.558, 0.856 | 0.450, 0.558, 0.855 | 0.514, 0.636, 0.972 | 0.043, 0.050, 0.071 |
| absorbing, sun | 0.843, 0.749, 0.690 | 0.843, 0.747, 0.690 | 0.861, 0.763, 0.703 | 0.014, 0.012, 0.011 |
| absorbing, local light | 0.845, 0.751, 0.691 | 0.843, 0.747, 0.690 | 0.945, 0.834, 0.766 | 0.071, 0.060, 0.053 |

- **Direct light** (`Bounces::deep(0)`, 32 samples) is the shadow model alone. The traced floor passes exactly `cast_tint`, and live agrees within 0.004. The test asserts 0.02 per channel, and the option off passes nothing.
- **The reference budget** (128 samples) adds what live does not draw: light the pane's faces reflect onto the floor, and the lit floor around it bounced back by the pane's underside. The option-off column is that light alone. It weighs most under the weak local light, whose open floor reads 0.021 against the sun's 0.235. The test asserts the traced ratio is no more than 0.02 under live's and no more than 0.15 over it. At 32 samples the local-light rows came out about 0.03 lower in both the on and off columns: rare paths off the glass's narrow highlight that 32 samples miss.

**Cost.** GPU time per sample with the option off and on, two rounds each on the RX 9060 XT under `pgpu`:

| scene | off | on |
|---|---|---|
| the pane above, 128 × 128, sun and local light, 16 samples (`transmissive_shadows_cost_on_the_pane`) | 0.634, 0.634 ms | 0.578, 0.574 ms |
| the deep-glass pacing fixture, 96 × 48, 4 samples (`transmissive_shadows_cost_on_deep_glass`) | 240, 250 ms (longest submission 14.9, 15.0 ms) | 249, 249 ms (longest 8.3, 8.3 ms) |

Before the roulette, the deep-glass fixture with the option on took 2.9 s per sample, 14 to 27 times the off figure. Its stacked open panes, liquid grid and liquid volumes put nearly every path vertex on glass, and every one of their shadow rays walked the whole BVH to the sky. The lane controller held it at one lane, with each one-strip slice near 7.7 ms, so the longest submission stayed under 10 ms. With the roulette the option costs what off costs on deep glass, within this desktop's run-to-run spread (an earlier pair of rounds read 200 and 147 ms off against 109 and 172 ms on), and on the pane it is about 9% cheaper: its paths stop where they reach the sky through glass after a lit vertex, and a glass vertex skips most of its shadow rays.

## Fireflies

A sharp lobe reached after a diffuse bounce (polished metal, a clearcoat at its default roughness 0.05, the reflection lobe of glass at 0.01) meets next-event estimation toward a small sun. A diffuse ray lands on the lobe and the sun sample falls inside it once in thousands of samples, with a weight that pays for all the misses. The pixel lights up alone, the edge-aware denoiser keeps it as detail, and more samples find more of them. Two settings remove them. Both are off by default, and both are in a scene's `[trace]` ([scenes.md](scenes.md#trace)) and the bake's `[plates]` ([bake.md](bake.md#plates)):

- **`Detail::clamp_indirect`** (`[trace] clamp_indirect`, `0` to `10000`, `0` off) caps light reached after the first bounce, as Cycles' Clamp Indirect does. Each contribution is capped on its own, as Cycles X does, not the path's sum: a light sample (sun, sky, local light, emitter) taken at the second vertex or later, and emission, the sky or a light that a ray leaving the second vertex or later runs into. One whose luminance (Rec. 709 weights) is over the cap is scaled down to it, keeping its hue. Light at the first hit is never clamped: its own light samples, and what the ray leaving it runs into, the two halves of its MIS.
  - *What it costs the look:* it takes energy away, from the fireflies and from the caustics they were converging to, such as the sun's reflection off chrome onto a floor. What a mirror or a glass shows is light after the first bounce too, so a cap below the brightest thing it reflects dims that reflection. Set it well above the sunlit radiance of the brightest diffuse surface.
- **`Detail::filter_glossy`** (`[trace] filter_glossy`, `0` to `1`, `0` off) is Cycles' Filter Glossy, applied after a diffuse bounce. Once a path has taken a diffuse bounce, every later vertex has its roughness and clearcoat roughness raised to at least this value before its light samples, its BSDF sample and its pdf. That removes the cause instead of capping it. The first hit, and every vertex of a chain of mirror, glossy or glass bounces from the camera, keeps its own roughness, so chrome and crisp glass stay crisp where the camera sees them, directly or through each other.
  - The tracer's refraction is a perfect delta with no roughness to filter. What the filter softens on glass is its reflection lobe toward the lights.
  - Roughness is squared into GGX's α, so `0.15` is α 0.0225, still sharp against a 6° sun. At 1024 samples it left a cloud of dimmer speckles that `0.25` did not.
  - *What it costs the look:* glints and caustics seen in diffuse light go soft and spread, so a small sun's reflection off chrome lands on a floor as a broad glow instead of a sharp spot. The energy stays.
- **A starting point:** `filter_glossy = 0.25`, with `clamp_indirect` as the backstop at several times the sunlit radiance of the brightest diffuse surface (`4` in the test below, about eight times its sunlit floor).
- **Same bytes when off.** Each is a pipeline-overridable constant, `CLAMP_INDIRECT` and `FILTER_GLOSSY` (default `0`), set only when its value is above zero. So the off pipeline gets exactly the constants it had before, and neither setting draws a random number. `Trace::new_detailed` refuses either out of range through `TraceSettings::check`. The bake's batch shader shares `trace.wgsl` and keeps both off.

`crates/trace/tests/fireflies.rs` is a neutral firefly scene at 160 × 90: a chrome sphere (metal, roughness 0.02), a red clearcoated panel (coat roughness 0.05) and a glass block (transmission 1, roughness 0.01) on a diffuse floor before a diffuse wall, under a sun of intensity 3 with a 6° disc (`sun_radius_deg` 3) and a dim uniform sky (0.03). `both_settings_off_keep_the_firefly_scene_bytes` pins the FNV-1a hashes of 16 samples, with both settings off, to the ones the shader gave before they existed (`674fa934…`, `a3812026…`, `8dfd8091…`). `glass_shadows`' pins still match too. `the_settings_cut_the_fireflies_of_sharp_lobes_after_a_diffuse_bounce` counts outliers: pixels whose luminance is over 4 times the median of their 5 × 5 neighbours (the floor of that median is 0.01). On the RX 9060 XT:

| samples | off | `clamp_indirect = 4` | `filter_glossy = 0.25` | both |
|---|---|---|---|---|
| 64: outliers, mean luminance | 8, 0.4745 | 4, 0.4697 | 5, 0.4750 | 4, 0.4721 |
| 1024: outliers, mean luminance | 17, 0.4868 | 4, 0.4763 | 5, 0.4820 | 4, 0.4787 |

The 4 left in every column are the sun's glint on the chrome, seen directly. Its brightest pixel reads 90.08 in all four columns at 1024 samples, and the test holds them within 2%. Off, the count grows with the samples. The test asserts that, and that each setting at 1024 leaves at most a third of off's count, with both together at most either alone. The clamp takes 2.2% of the mean luminance away and the filter 1.0%: the sun's caustics off the chrome and the panel, which the filter spreads and the clamp removes. A first sweep at 1024 samples, as outliers over 1.5, 2, 4 and 8 times the median, gave: off 50, 35, 17, 13; `filter_glossy` 0.15 gave 71, 20, 5, 5; 0.25 gave 16, 8, 5, 5; 0.4 gave 15, 9, 5, 5; `clamp_indirect` 2 and 4 gave 15, 9, 4, 4 each. The whole test takes about 17 s under pgpu, and its longest submission stays under 100 ms (the test asserts it).

## Text

`pfx_trace::text` turns laid-out text into content on a placement. Its default is **lettering**: the glyph quads and the MSDF atlas themselves, evaluated at every hit, so an edge stays exact at any zoom and nothing is baked. `text::print` still rasterizes the same glyphs into an image for callers that want one. Text rides the content records the tracer already had: no binding is added, and a stage without text traces the same bytes on two runs (`a_stage_without_text_traces_the_same_bytes` in `crates/live/tests/traced_text.rs`, digest `162a8cf4…`). `pfx-trace` depends on `pfx-text` for it; `pfx-live` reaches the tracer only through the bake, so the text-to-content path sits here, beside `stage`.

- **`Glyph`** is one glyph quad as live draws it: `rect` in layout units, the atlas `uv` rect, `color` (linear, alpha already multiplied by the quad's alpha), `clip` and `turn`, the fields of live's `RichQuad`. `text::glyphs(&rich)` takes them from a `RichParagraph`, leaving out colour-page glyphs, which live's surface text does not draw either.
- **`text::lettering(&atlas, &glyphs)`** keeps them as a `Lettering`: the glyphs, a copy of the atlas, `rect` (the union of the glyph quads in layout units, not snapped) and `density` (the atlas's texels per layout unit, the mean over the glyphs). Its content space is 0…1 across `rect`, as a print's is across its image.
- **`text::print(&atlas, &glyphs, density)`** rasterizes them into a `Print`: an RGBA8 image at `density` texels per layout unit (texels per metre for manf, whose layout unit is the metre at `UNITS_PER_METRE`), covering the union of the glyph quads snapped outward to whole texels (`Print::rect`). Colour is premultiplied linear, sRGB-encoded; alpha is linear: what `stage::Image { srgb: true, .. }` decodes and what the content layer's `Over` (`base·(1 − a) + rgb`) and `Emit` expect. Each texel runs live's glyph shader at level 0 (a bilinear sample clamped half a texel inside the glyph's cell, its median, and `msdf_coverage(rgb, 8·t)` with `t` texels per atlas texel and 8 the atlas's distance scale, `text::MSDF_SPREAD`), which gives the one-texel ramp live's `0.5 + sd / fwidth(sd)` gives on an axis-aligned edge.
- Both implement `text::Laid` (`rect()` and the `ContentKind` they show), so `carrier()`, `content()` and `Fit::content` take either.
- **`text::material(lit)`** is a carrier material: black Lambert, no specular. Unlit (`false`) blends the content by `Emit`, so the glyphs show their colour whatever the light, like live's `TextSpace::Surface`. Lit (`true`) blends by `Over`, so the glyphs are their colour as a Lambert base, like `LitSurface` (`paint.rgb · irradiance / π`).

### Lettering at a hit

`TextContent` is what a placement carries for text: the `Lettering`, the map from layout to the content's UV (`uv = layout · scale + offset`; `Lettering::shown()` gives the one across `rect`) and an optional picture under the glyphs. At staging, `TextContent::records()` packs it into `detail::ContentText`, `vec4<f32>` records appended to the content buffer, whose `ContentInfo` word that was padding now says `kind` 1. Every value is an exact float, integers included, so nothing relies on bit patterns surviving a float load:

| records | what |
|---|---|
| 0 | the grid's corner in content UV and its cells per UV unit |
| 1 | columns, rows, where the glyphs and the index list start |
| 2 | the atlas's width, height, start and channels (3 for MSDF, 1 for coverage) |
| 3 | the picture's width, height and start, or zeros |
| 4… | per cell, its first index and count, two cells per record |
| then | the glyph indices of each cell, four per record, in drawing order |
| then | per glyph five records: the 2×2 map and shift from content UV to the glyph's 0…1 fraction (its turn folded in), its clip in that fraction, its atlas cell, its colour |
| then | the atlas's level 0, four texels per record, each `r + 256·g + 65536·b` |
| then | the picture's linear texels, if any |

The grid is about one glyph per cell (the glyphs' mean extent), at most 16 cells per glyph and 2^18 in all; each glyph is listed in every cell its quad's box touches. At a hit, `text_at` in `trace.wgsl` finds the cell under the content UV, and for each glyph listed there takes the fraction, skips it outside 0…1 or its clip, samples the atlas bilinearly half a texel inside the cell as live does, and takes coverage 1 where the median is at least 0.5 and 0 elsewhere (a coverage atlas gives its sample). The glyphs blend premultiplied `Over`, in order, over the picture or over nothing. No ramp is drawn: each sample's own pixel jitter does the anti-aliasing, so the edge is one pixel's box filter at any distance. The cutout reads the same `content_at`, so text is cut at its placement's `alpha_cutoff` exactly as an image is. An MSDF glyph's coverage is 0 or 1, so its edge stays on the median's 0.5 line at any cutoff up to the ink's alpha, and above that the glyph is cut whole; a coverage atlas's soft edge moves with the cutoff as an image's does. The `ContentInfo` width and height are a nominal size (`density / scale`, or the picture's), used only by the emboss's finite differences.

`TextContent::at(uv)` and `sampler()` evaluate the same thing on the CPU, glyph by glyph without the grid; `lettering_records_find_every_glyph_through_the_grid` checks a mirror of the shader over the records against it, a turned, a clipped and a half-transparent glyph included.

The cost is a cell lookup and a few glyphs per hit. On a 1024×512 frame filled with five lines of 255 glyphs (`lettering_cost_per_sample_against_the_print`), lettering takes 0.328 ms of GPU per sample and the print at two texels per pixel 0.302 ms: 1.09×. The records hold the whole atlas page at 4 bytes a texel (4 MB for a 1024² page) and 80 bytes a glyph, where the print took 16 bytes a texel (14 MB for that frame's).

### Free-standing text

No `on` in the recipe. manf lays it with `render_spans(…, Representation::Msdf, …)` and places it with `Place::World { model }`, the matrix it multiplies into live's `model_view_projection`. The tracer gets a carrier quad with the glyphs as cutout content:

```rust
let glyphs: Vec<Glyph> = laying.quads.iter().map(|q| Glyph {
    rect: q.rect,
    uv: q.uv,
    color: [q.color[0], q.color[1], q.color[2], q.color[3] * q.alpha],
    clip: q.clip,
    turn: q.turn,
}).collect();
let lettering = text::lettering(&laying.atlas, &glyphs)?;
let carrier = lettering.carrier();
meshes.push(carrier.mesh());
materials.push(text::material(lit));
placements.push(Placement {
    content: Some(lettering.content()),
    ..Placement::new(carrier_mesh, narrow(model), carrier_material)
});
```

- `carrier()` is a two-triangle `stage::Mesh` on the layout's `z = 0` over `rect`, UV 0…1 across it, normal +z (`Carrier::over(rect)`). `content()` shows the text with `cutout` on, so camera and every shadow ray pass outside the glyphs (see [Placements](#placements)): with lettering the cutout is the glyphs' median ≥ 0.5 itself, so shadows keep the glyphs' exact shape too.
- Placements of the same `Lettering` with the same map share one slot.
- With a print instead (`print.carrier()`, `print.content()`), the traced edge is the 0.5 line of the image's alpha, and it is only as sharp as the density: print at one texel per screen pixel or more at the closest view. Each texel takes 16 bytes in the slot buffer, so a word 0.4 m wide filling half of a 4K frame (4800 px/m) costs about 37 MB at one texel per pixel.

### Text on a part

`on = "node"`: manf places the text with `world(on) · face(on) · LIFT · local(text) · flip`. The text becomes the part's own content, mapped onto the face the text sits on:

```rust
let fit = text::fit(&part_mesh, part_model, narrow(model))?;
let lettering = text::lettering(&laying.atlas, &glyphs)?;
let content = fit.content(&lettering);
let shared = fit.elsewhere(&part_mesh, &content);
```

- `fit` takes the part's triangles parallel to the text's plane and nearest to it (the face under the 0.5 mm lift), fits the face's UVs as an affine function of the layout by least squares, and returns `Fit { scale, offset, face }` (`uv = layout · scale + offset`). It refuses a part with no parallel face, UVs that are not a flat map of the face, and UVs turned against the text: `PlacementContent` has a per-axis scale and no rotation, so the face's U must run along the text's x and V along its y, in either direction.
- `Fit::content(&lettering)` (or `&print`) is the `PlacementContent` (`uv_offset`, `uv_scale`, `crop = [0, 0, 1, 1]`, no cutout) that puts `rect` where live's surface quads land. The part keeps its own material, whose content layer blends the text: `Over` by default, which is lit ink like `LitSurface`. Unlit `Surface` text on a lit part has no single blend in the content layer (it would need the part's base under `1 − a` and the text as emission); use the carrier route below for it.
- **Content is per placement, not per face.** `Fit::elsewhere` lists the part's other triangles whose UVs land inside the crop, where the text would show too. Manfred's `box_uvs` project both z faces onto the same square and give the sides 0…1 as well, so on a box it names all ten other triangles; `ContentFace` only picks the winding side. When `elsewhere` is not empty, stage the text as a carrier instead: the free-standing call above with the same `model`, which already includes Manfred's lift, so the carrier sits 0.5 mm off the face exactly where live draws its quads.
- **A part with a picture and text.** `fit.over(&picture_content, &lettering)` keeps the picture's `PlacementContent` (its UVs, crop, face and cutout) and makes it text over the picture: the picture's texels are sampled as an image's are, and the glyphs, mapped through the picture's `uv_offset` and `uv_scale`, blend over them at every hit, so the text is sharp whatever the picture's resolution. Text outside the picture's crop is not shown. `fit.onto(&picture_content, &laying.atlas, &glyphs)` is the print route: it prints the glyphs into the picture's own texels and returns them in the picture's encoding, as sharp as the picture.

### Overlay text

Live draws overlay text after the finish chain, onto the linear display intermediate, before the blit encodes it for an sRGB output (Manfred's `Rgba8UnormSrgb`). The traced overlay goes at the same point of `pfx_post::Chain` on the CPU, at the frame's own pixels, so it is never resampled:

```rust
let mut image = finish.apply(&film_image, None, None);
text::overlay(&mut image.pixels, width, height, &laying.atlas, &glyphs)?;
let mut encode = finish.clone();
encode.passes = vec![Pass::Encode];
let display = encode.apply(&image, None, None);
```

`finish` is the chain without `Pass::Encode`. `glyphs` are the overlay quads in pixels, moved to their screen anchor as Manfred's `Drawn::text` moves them. Coverage atlases (Manfred's overlays) are sampled at level 0 as live does at 1:1, and MSDF atlases with live's ramp. With an `Rgba16Float` output live's chain itself ends in `Encode` and overlay text blends over encoded values; the traced overlay follows the sRGB-output case.

### Tests

`crates/trace/src/text.rs` checks that a print covers every glyph on whole texels with premultiplied sRGB colour, that MSDF edges stay about a texel wide at 400, 1500 and 6000 texels/m, that the records found through the grid give what the glyphs give one by one, that lettering and an 8000 texels/m print agree inside the glyphs (IoU above 0.995), that text over a picture lands where the print onto it does, the box fit for both kinds and its ten shared triangles, the staging of text into its own slot beside images, the refusals, and the overlay's blend. `crates/live/tests/traced_text.rs` (GPU, under pgpu) compares the tracer with live's own text pass:

| test | lettering | print |
|---|---|---|
| "Pito", 0.2 m, turned 20°, perspective at 512×256: traced carrier (64 samples) against live's `Surface` quads, coverage masks at 0.5 | IoU 0.992 | IoU 0.992 (4000 texels/m) |
| front-on orthographic at 512×256, the word at 1024 px/m and 8× closer: partial pixels per edge pixel (128 samples) | 0.882 → 0.903, 1.02× | 1.435 → 7.496, 5.22× (1024 texels/m) |
| the same word black 0.5 m over a floor, sun at (2, 1, 0), top-down orthographic: floor darker than half its lit value against the content's alpha ≥ 0.5 projected along the sun | IoU 0.951 | IoU 0.950 |
| "Bench" on the front of a 0.6 × 0.3 × 0.1 m box with Manfred's box UVs, through `fit`, front-on orthographic at one pixel per content texel: centroids of live's coverage and the traced text | 0.036 and 0.013 texels apart | 0.001 and 0.013 |
| "Bench" over a 512² picture on that face: `fit.over` against the picture `fit.onto` composed, ink centroids | 0.18 and 0.05 picture texels apart | |
| five lines, 255 glyphs, 1024×512, 16 samples | 0.328 ms per sample | 0.302 ms (two texels per pixel) |
| a soft disc in a 64² coverage atlas as one glyph, against the same alpha as an image, cut at 0.25 and 0.9 before an emissive backdrop: radius kept | 82.672 and 47.977 px | the same (the ramp says 82.667 and 48.000) |
| "Overlay" at 40 px, alpha 0.85, over a linear gradient: `text::overlay` against live's overlay pass, both encoded to sRGB 8-bit | at most 1/255 | |
| a floor and a box with no text, traced twice | the same digest on both runs (`162a8cf4…`) | |

The shadow test projects the glyph alpha from the 0.5 mm lift live uses (`LIFT` in `crates/live/tests/traced_text.rs`). The tracer's own ray start, on this floor and this camera, is about 0.02 mm, so the old 1 mm bias along the sun's x (0.5 mm times the sun's x/y of 2) is under half a pixel of the 1 m view and stays inside the IoU. The zoom test picks the 8× window on the glyph whose window is closest to half ink; the print's edge grows about 5× rather than 8× because its cutout keeps the outer half of the ramp hard.

## The steam's look

`detail::Steam` is a volume the tracer marches in a box: a source and radius, a density, an ambient term, a phase anisotropy, the box and a time, plus a `look: PlumeLook`. The source was called `cup` before; it is `source`. `PlumeLook` carries the shape of the plume, and its fields are named and typed as live's `Plume` (`pfx_live::effects`) names them, so a scene's plume looks the same live and traced: `spread`, `drift`, `sway`, `fade`, `grain`, `lift`, `warp`, `churn` and `threshold`, with the same formulas (the Gaussian column, the drift and sway by height above the source, the fade ramps, the warped four-octave fbm and its smoothstep). `Plume::source` and `Plume::radius` map to `Steam::source` and `Steam::radius`. The tracer does not depend on live, so a caller copies the fields across; `crates/trace/tests/steam_look.rs` does it with an exhaustive destructure of `Plume`, so a field added to it fails the build there.

`PlumeLook::default()` is the tracer's own look, the one it had before the look existed: a traced still of it is byte-identical to the old one (`the_neutral_plume_traces_the_same_bytes_with_the_default_look`, and a live `Plume` holding the same numbers traces the same bytes). The look reaches the shader as pipeline constants (`STEAM_*`, named by `PlumeLook::NAMES`), as live's does, so the compiler folds them like literals.

What a live `Plume` has that the tracer's steam doesn't:

- **`depth_per_width`** sizes live's box; the tracer's box is `box_lo` and `box_hi`.
- **The noise** is the tracer's own hashed value noise at four octaves, live's is its own at the cuts' octave counts, so the wisps' pattern differs even where every number matches; the warp and density octaves (`VolumeCuts`) have no counterpart.
- **Density and light:** live scales the density by its volume's control and shades by its own lighting; the tracer scatters the sun and ambient through 20 steps with its own extinction. `Steam::density`, `ambient` and `anisotropy` have no live counterpart in `Plume`.
- **The top of the column:** the tracer ends it at `fade[2]`, where the fade reaches zero; live's fade ends the same place.

## Pacing

`Trace::sample(gpu, count, seed)` never sends a whole frame in one submission. pgpu's rule 2 wants 2–6 ms per submission with one in flight, and on a desk scene one sample of the whole image costs about 290 ms; a 1.8 s dispatch reset the desktop GPU's ring three times on 2026-10-03.

- **Strips, lanes and the order they go in.** The image is cut into strips of 8 rows (`BAND_STEP`), one row of 8×8 workgroups each, and every pixel of a strip is one lane of its workgroup (lane = row in the tile × 8 + column). A dispatch names a run of strips and a range of lanes: it launches the whole strip width for each strip and every lane outside the range returns at once. Strips go out in a golden-ratio order, not top to bottom: strip `k` of the order is `k · BLOCK_STRIDE mod strips`, with the stride the nearest number to 0.382 of the strip count that shares no factor with it, a pipeline-overridable constant set when the `Trace` is built. So any run of strips samples the whole frame, and a cheap sky above dear glass cannot size the band that lands on the glass.
- **Why a glass band was missing 6 ms.** A submission waits on its slowest wave, and a wave of divergent glass paths runs its lanes' long loops one after another. On a neutral 96×48 fixture (four stacked glass panes over a 48×32 liquid mesh and four blended liquid-like volumes, `deep_glass_cost_by_lanes_and_blocks`), one 8-row band took 37.4 ms median and 40.7 ms at the longest, and the whole frame in one submission took 40.4 ms: six strips cost what one does, because the width is not what the submission waits on. One row took 15.0 ms median and 19.1 ms at the longest. Per lane of every strip at once, one lane took 2.5 ms median (4.3 longest), two lanes 4.2 (4.5), four 6.2 (8.2), eight 10.1 (10.2), sixteen 13.8 (14.1) and thirty-two 17.7 (19.5). The old pacer could not go below one 8-row band, so a glass fill sat on that 37–41 ms floor, near the 46.76 ms a caller measured on glass and liquid scenes; and while the upper rows were cheap its fit grew the band, and the band already in flight was the same size, so both landed on the dear rows. Timings move by up to a factor of two between runs on this desktop (eight lanes took 10.1 ms in one run and 14.8 ms in the next), and cost is not monotone in the strip count: in one run two lanes took 7.1 ms for two strips and 4.5 ms for six.
- **The lane controller.** Each `Trace` keeps a lane width (1, 2, 4 … 64) and a strip count, starting at one lane and one strip. Below 64 lanes a sample goes out in passes: each pass is one range of lanes over every strip, split into slices of the strip count rounded so the slices come out even, with the Pacer's fixed units and one slice in flight. After each pass the longest slice decides: at or under 6 ms (half of `SAMPLE_SPAN_MS`, 12 ms) the strip count doubles, or once a slice holds every strip the lane width doubles; a doubling that comes in over 9 ms is undone and that width or count is never tried again on this `Trace`; over 12 ms without a doubling, the strip count, or at one strip the lane width, is cut in proportion to fit 6 ms. A doubled lane width or strip count costs at most twice what was measured, so a step taken from 6 ms stays under 12. At 64 lanes the trace goes back to whole-row bands through the adaptive `Pacer` (3.5 ms target, 6 ms ceiling, step 8 rows, a stack of samples per submission once one slice holds a whole sample); a dense slice of one strip over 12 ms sends it back to fewer lanes. On the fixture, two samples went out in 74 submissions, median 7.9 ms and longest 8.2 ms, at two lanes and every strip per slice: 543 ms of GPU time, where 8-row bands took 467 ms with a longest of 44.9 ms. A cheap 160×96 room is back at 64 lanes after its first sample, and eight more samples go out in 7 submissions.
- **The pass in the shader.** `main` takes its workgroup and local ids. `Frame.counts.w` carries the first strip of the order in its low 16 bits, the first lane in bits 16–22 and the end lane in bits 23–29; the strip count is the dispatch's height in workgroups. The `Frame` layout, the bindings and the bake's batch shader (whose own entry point uses `counts.w` for its face count and ignores the stride) are unchanged. Images taller than 65,535 rows are refused.
- **Same bytes.** Every pixel keeps its absolute address, its seed (`seed`, the absolute sample index and the pixel index) and its accumulation order, so any split gives the bytes one unsliced dispatch would. `sliced_samples_trace_the_same_bytes` traces a 37×29 softbox room, lit and unlit, as 3 then 2 samples in one dispatch each, and again in bands of 1, 7 and 64 rows at step 1, 8 and 16 rows at step 8, adaptive at both steps, through `sample_paced` and through `sample`; all of them equal the unsliced output byte for byte. `paced_traces_keep_the_bytes_of_eight_row_bands` pins the FNV-1a hashes of the colour, albedo and normal outputs of the glass fixture (2 samples) and the lit and unlit room (3 then 2 samples) to the hashes the 8-row bands gave before the lane controller (main at 6538edc).
- **Blocking.** `sample` returns once its last band has finished on the GPU. `samples()` counts the samples whose bands are all done.

Two ways to call it:

- `sample(gpu, count, seed)` keeps a `Pacer` of its own inside the `Trace` and turns through `pfx_gpu::pace::Turns`: `pgpu turn --work-ms … --longest-ms …` after about 50 ms of measured work, only when run under pgpu (`GPU_QUEUE_TURN` or `GPU_QUEUE_PID` set). Elsewhere it only counts.
- `sample_paced(gpu, count, seed, &mut pacer, between)` uses the caller's `Pacer` and returns the run's `Stats` (every submission's time, its timing source and the wall time). It sets the Pacer's step (8 rows for whole-row bands, one strip for a lane pass) and its fixed units for a lane pass, and puts the caller's step and fixed units back afterwards. Hand `between` to the adapter library's `turn_soon` (`|ms| adapter::turn_soon(&mut turns, ms)`), or a `Turns`, or `|_| ()`. The Pacer learns its whole-row bands under the label `pfx_trace::gpu::SAMPLE_LABEL` (`"trace samples"`); the lane width and strip count live in the `Trace` (`Trace::lanes()` reads the width), so a new `Trace` starts again at one lane.

To measure an unsliced sample, call `sample_paced` with a count of 1 and a Pacer whose fixed units are the image height rounded up to the step (`pacer.set_fixed_units(Some(height.next_multiple_of(8)))`): fixed units skip the lane controller, every pass is whole-row bands of that many rows, and that sample is one dispatch of the whole image in one submission. Never send more than one sample unsliced on a desktop that is in use.

## The asset look

What `crates/assets` needs to render marks and icons on the tracer instead of Cycles: the rig's area lights, a softbox room the engine owns, a shadow catcher with straight alpha, Blender's AgX, Freestyle-like outlines and adaptive sampling. Every piece is generic; the tests use a neutral grey box, a red capsule and a ball on a floor.

### Disc lights

`LocalLight::disc(centre, normal, radius, colour, intensity)` adds `LightShape::Disc { normal, radius, two_sided }` beside `Rect`. Like a rect, its radiance is `colour·intensity / area` (area `π r²`), it emits along `normal` (both ways when `two_sided`), it is sampled uniformly by area with MIS against BSDF sampling, and a BSDF or camera ray that crosses a shadowed disc picks up its radiance with the matching weight. Packed as kind 3: `c = (normal, two_sided)`, `d = (radius, 0, 0, shadow)`. The rect's math is unchanged.

`disc_lights_give_their_irradiance` (GPU) lights a 0.6 grey, 0.7 rough floor with a 0.4 m disc 1.2 m up and compares every pixel of a 24×24 view (8,192 samples) with a 32×64 polar quadrature of the disc through the shared BRDF: worst 1.2% (tolerance 3%).

### Converting Blender's units

Measured on Blender 5.2 with Cycles on the CPU (a 1 m square area light of 100 W, 2 m over a white 0.8 Lambert plane, and a world of strength 1): an area light's radiance is `P / (π·A)` (31.82 measured against 31.83), and a world of strength `s` and colour `c` is radiance `s·c`. So a Blender area light of power `P` is `LocalLight::rect(…, intensity = P / π)`, and a Blender world is the same `Sky` texels times its strength.

### The softbox room and the rig

`pfx_trace::rig` replaces `render.py`'s softbox rig and Blender's bundled studio-light HDRI.

- **`room(turn_deg, strength)`** is the engine's own neutral softbox room (`softbox_room(turn_deg)` is its `RoomSky`), 1024×512, fitted to Blender's bundled studio-light world (the one the asset renders lit by before the tracer): a dark room (walls `ROOM_WALL`, ceiling `ROOM_CEILING`) over a floor that brightens to `ROOM_FLOOR` straight down, all slightly blue as in Blender's, lit by four small lamps near the horizon and nothing overhead (`ROOM_LAMPS`: a 4° spot at azimuth 133.8°, 27.8° up; a 15.5°×1.5° greenish strip at −1.1°, 22.7° up; and two globes, 6° at 21.4°, 4.7° up and 5° at −146.4°, 1.5° up), each with its own slightly blue colour, edges softened over 18% of their half-size. Each lamp's place, size, colour and power come from a fit to the world's texels (each lamp's light kept, its size by least squares), and the room's irradiance matches the world's within 5% on every axis and on the 45° diagonals. It is exposed so its solid-angle mean luminance is `ROOM_MEAN` times `strength`: 0.80, which is the 0.77 that Blender's world measures times 1.039, the one scale that best matches the frozen asset references' exposure (least squares over the room's share of the light on the logo, the tile and the dot icon, `the_room_and_the_rig_split_the_light_on_a_mark`); about nine tenths of that is the lamps, as in Blender's, so metal still reads small highlights from a dark room. `turn_deg` turns the room about +Y by −`turn` (Blender's mapping turns its environment the same way about +Z). Azimuth 0 is +Z, 90° is +X.
- **`Rig`** holds `render.py`'s `[rig]` keys: `key`, `fill`, `rim`, `key_from` (Blender axes), `key_color`, `rim_color` (a list; a second colour adds `rim2` at 0.8 of the rim's power), `strength` (default 0.6) and `turn` (default 25). `Rig::lights()` gives the softboxes as one-sided rects: Blender's position `(x, y, z)` is the engine's `(x, z, −y)`, each faces `(0, 0, 0.1)` in Blender axes with its long side level (Blender's `to_track_quat("-Z", "Y")`), sized `size × 0.6·size` (key 3 m, fill 2.4 m, rims 2 m), and `intensity = power·|position|²`, which is Blender's `energy = power·π·|position|²` divided by π. `Rig::sky()` is `room(turn, strength)`.

### The shadow catcher and straight alpha

`Trace::set_matte(Matte)` switches the film; it rides the spare `Frame.forward.w` (0 off, 1 transparent, `2 + material` with a catcher), so the bindings and buffers stay as they were and a trace without a matte draws the same bytes.

- **`Matte::transparent()`**: a camera ray that misses everything adds nothing and has alpha 0, so the colour is premultiplied by coverage. The output's alpha is the first hit's coverage, `albedo.w` averaged (it was always 1.0 before, and still is without a matte). Rays that leave through glass still see the sky, as in Cycles.
- **`Matte::catcher(material)`**: every surface with that material is drawn only as the shadow it receives. A camera ray that lands on it stops there with colour 0 and alpha `1 − seen/open`: the sun, four sky samples and one sample of each light or emitter (by luminance, cosine-weighted, unweighted by MIS) are summed open and again counting only those no shadow ray finds blocked. With `Detail::transmissive_shadows` on, each of those shadow rays is weighted by the luminance of `shadow_passed`, so glass over a catcher throws the light shadow it throws on any floor (its colour is not kept, alpha being one channel). Bounced rays still see the catcher as an ordinary surface, so a metal mark reflects the floor it stands on, as Cycles' catcher does. The catcher needs the transparent film. A catcher material past the scene's materials is refused.
- **Composite.** Over a flat background, `matte::over(texel, background)` is `rgb + background·(1 − a)`; `matte::straight` and `Rgba::straight` divide by alpha for the straight-alpha PNG (alpha under 1/65535 is transparent black).

`the_shadow_catcher_keeps_only_its_shadow_in_alpha` (GPU): a ball 0.1 m over a catcher floor, a disc light to the side and a dim sky, seen from above. Without a matte every alpha is 1. With the catcher the ball is alpha 1, the floor in its shadow 0.93 with colour 0, the lit floor 0.003 with colour 0 and the sky beyond the floor 0; transparent alone keeps the floor at alpha 1; and switching the matte off again traces the bytes it traced before. `the_catcher_follows_transmissive_shadows` (GPU) makes the ball clear glass: its catcher shadow is dark with the option off and light with it on.

### The probe

`Trace::probe(gpu)` traces one ray through each pixel centre (no jitter, no lens blur) and returns a `Probe`: `depth` (along the view axis, or the distance for an equirect), `distance`, `id` (`material + 1`, 0 for a miss) and `normal` (facing the camera). It is a second entry point, `probe` in `probe.wgsl`, over the same scene buffers with two storage textures of its own (bindings 17 and 18), built when called. `Probe::without(&[ids])` zeroes the ids that are not outlined, the catcher's for one. `the_probe_reads_depth_id_and_normal_at_pixel_centres` (GPU) checks the ball's top, the floor and a miss.

### Outlines

`pfx_post::lines` draws `render.py`'s `[outline]` from a probe: `Lines { enabled, thickness, color, alpha, crease_angle, silhouette, border, crease, contour, external_contour }`, with Freestyle's defaults (2 px at 1024 wide, `#171a34`, 134°, all on but `contour`).

Each pair of neighbouring pixels is one candidate edge, half-way between them:
- **external** (`external_contour` or `silhouette`): one side outlined, the other not (a miss or an id left out);
- **silhouette**: a depth jump between two ids; **contour**: a depth jump within one id. A jump is more than twice the depth step on either side plus 0.2% of the depth, so slopes seen edge-on are not lines;
- **border** (`border` or `crease`): two ids meeting without a jump, where two elements' faces meet;
- **crease**: one id, the normals more than `180° − crease_angle` apart.

`Lines::coverage` draws every edge as a unit segment with round caps, `thickness·width/1024` pixels wide, anti-aliased over one pixel; `Lines::draw` lays the colour at `alpha` over premultiplied RGBA, alpha included, so a line outside the silhouette shows on a transparent background. Tests: a square's external contour, a 50° crease kept at 134° and dropped at 120°, slopes against jumps, borders, and the composite.

### AgX

`pfx_post::view` finishes a still: `View::Standard` (sRGB, clipped) or `View::Agx`, after `exposure` stops; `finish` un-premultiplies and applies the view, `master16` makes the 16-bit master and `delivery8` the 8-bit delivery with an 8×8 Bayer dither of ±½ code value on colour.

`agx_base` follows Blender 5.2's `AgX` view (`AgX Base`, display sRGB): Rec.709 to FilmLight E-Gamut, an inset, log2 from −10 to +6.5 stops around 0.18, Blender's sigmoid (pivot at middle grey, slope 2.4, toe and shoulder power 1.5) to a 2.4-power encoding, a 65% hue shift back toward the input, an outset, back to Rec.709 and the sRGB curve. The sigmoid is the published one; the inset, outset and hue mix are fitted (Levenberg–Marquardt on 1,540 colours from 2⁻⁹ to 2⁵) to Blender's own transform evaluated by OpenColorIO 2.5 from Blender 5.2's `config.ocio`, because Blender bakes them into a 57³ LUT. Against 31 stored Blender values (`view::tests::BLENDER_AGX`): greys within 0.0007, colours within 0.007, pure Rec.709 primaries within 0.042 (sRGB code values, 0–1); over the fit set, colours below 85% saturation are within 0.0024 rms (p95 0.0047). The older `tone::agx` approximation and its chain pass are unchanged.

### Adaptive sampling

`Trace::sample_adaptive(gpu, Adaptive { threshold, min_samples, max_samples, growth }, seed, &mut pacer, between)` samples in rounds (`min/2`, then `min`, then ×`growth` to `max`) through `sample_paced`, and after each round checks every pixel on the GPU (`converge.wgsl`, its own two small passes over the accumulation buffer):

- **measure** compares the pixel's mean now (`b` samples) with its mean at the last check (`a` samples): the mean of the samples since, `D = (b·C_b − a·C_a)/(b − a)`, gives `|C_a − D|·√(1/b) / √(1/a + 1/(b − a))`, the expected error of the mean now. At a half split this is Cycles' half-buffer difference, and the error is Cycles': the summed RGB difference over `0.0001 + √(R + G + B)`, plus the alpha difference.
- **mark** stops a pixel once it has `min_samples` and no pixel in its 3×3 is above `threshold`, by negating its sample count in the accumulation buffer; the trace shader returns at once for a negative count and its outputs keep their last values. It counts the pixels still running.

The result, `Converged`, lists each round's samples and running pixels, the mean samples per pixel and the run's `Stats`. `Trace::sample_counts(gpu)` reads every pixel's own count. The same seed and threshold stop the same pixels on the same GPU. `adaptive_sampling_stops_where_the_image_is_clean` (GPU): on the catcher scene at 0.02 the sky stops at 16 samples, the mean is 105.5 samples against a cap of 1024, regions agree with 1024 fixed samples within 3%, and a second run gives the same rounds and bytes.

### A clean 512 px icon

`a_clean_icon_at_512` and `clean_icon_budget` (GPU, release build) render a neutral icon at 512×512: a brushed-metal rounded tile (`#8c9096`-like grey, metal, roughness 0.22) under a red clearcoated capsule on a catcher floor, the default `Rig` and its room, orthographic from 45° above. Errors are 8-bit code values of the composite over mid-grey against 16,384 fixed samples (which carry about 0.6 of noise themselves); GPU time is the sum of the paced submissions (none over 6.7 ms) on the RX 9060 XT, shared with other jobs.

| threshold, cap | mean samples | GPU | objects rms / p99.9 | catcher rms / p99.9 |
|---|---|---|---|---|
| 0.01, 1024 | 582 | 2.6 s | 3.37 / 22.6 | 0.41 / 2.0 |
| 0.01, 2048 | 981 | 4.9 s | 2.53 / 17.3 | 0.38 / 1.8 |
| 0.01, 4096 | 1,712 | 6.6 s | 1.96 / 13.4 | 0.38 / 1.7 |
| 0.005, 4096 | 2,326 | 8.2 s | 1.96 / 13.4 | 0.22 / 1.1 |

Measured with the room fitted to Blender's world (wave 74). Its lamps are a few degrees across instead of 10° to 40°, so the tile's glossy metal carries more grain at every cap than with the earlier room (objects 2.44 / 11.3 at 1,024 and 1.34 / 6.2 at 4,096 before); the catcher is unchanged.

- **Where the noise is.** The floor and the shadow settle by 300–600 samples. Glossy metal does not: its noise is the lit floor and room seen two bounces deep, plain Monte Carlo grain with no fireflies, and it falls as 1/√n, so at 0.01 most metal pixels run to the cap.
- **No denoiser.** An edge-aware à-trous filter guided by albedo and normal (the tracer's `denoise::filter`, tried with alpha carried too) doubled the object error (2.4 → 4.9 rms at 1,024): the reflections on metal are detail that neither albedo nor normal sees. So clean pixels come from samples: **0.01 with a cap of 4,096 for icons (about 1,700 samples, 7.5 s at 512 px), 0.005 when the shadow must be smoother.** Time grows with the pixels: a 2048 px mark at the same settings is about 16× that.
