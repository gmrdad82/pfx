# Plates

A plate is the static part of a scene traced offline at the camera's rest pose, at a few light anchors, and composited live behind the parts that move. The live renderer draws only what moves over it, so a frame costs a fraction of drawing the whole scene, and the static part looks like the trace because it is the trace. This is phase 2 of the live renderer's 4K60 plan (`docs/live-renderer.md`); phase 1 (frames on demand, crisp text) runs beside it.

The pieces, each documented where its keys live:

- **What is static.** `scene.toml` marks each object and text `dynamic` or static (`docs/scenes.md`, `dynamic`): movers, camera-facing cards and their children are dynamic unless forced; everything else is static. `Scene::static_subset()` is what a plate traces.
- **The bake.** `pfx bake` with a `[plates]` table in `bake.toml` traces the static subset into colour, sun, transfer, depth, normal and id layers per view (`docs/bake.md`, Plates), keyed by a hash of the static subset alone.
- **The composite.** `Renderer::set_plates` (and `Renderer::stage` on a scene with `[plates]`) draws the plate behind the dynamic objects, below.

## The composite

`pfx_live::plates::Plates` holds one plate on the GPU: the colour, sun and transfer layers as `Rgb9e5Ufloat` arrays with one layer per anchor, the depth (`R16Uint` inverse or `R32Float`), normals (`R32Uint`) and ids (`R16Uint`) once, and a 2048² shadow map of its own. `Renderer::set_plates(Some(&plate), PlateSettings::default())` uploads it; `None` drops it. Each frame, after the opaque pass and before linear depth, it runs two passes, `plate shadows` (when a dynamic caster stands in the sun) and `plate composite`:

- **Reprojection.** A full-screen pass finds, for every pixel, where its ray meets the plate: it projects a point of the ray into the plate camera, reads the plate's depth there, moves the point to that depth along the ray, and repeats (`PlateSettings::iterations`, 8, stopping early once a step moves the point by less than 10⁻⁵ of its distance). The depth is read bilinearly in inverse depth, which is exact across a plane, and point-sampled across a silhouette (when the four texels differ by more than 3%). A rotation or zoom about the camera's centre lands in one step; a drift of a few centimetres converges in two or three. The overscan covers the drift, `scale` the zoom.
- **Crisp at rest.** The colour is looked up along the unjittered ray, so a camera at the rest pose reads every texel at its centre and the frame is the plate, texel for texel. The depth the pass writes comes from the jittered ray, so dynamic objects, which TAA jitters, are depth-tested against the plate where they are drawn. Plate pixels write 1 into the TAA's reactive mask, so they take no history and TAA's neighbourhood clamp does not soften them (`PlateSettings::temporal = true` lets TAA filter them like everything else). A frame with glass uses the glass pass's reactive mask instead.
- **Depth test.** The pass writes the plate's depth (pushed back by `depth_bias`, 0.15% of the distance, so a card lying on a plate surface wins) with `Less`, after the dynamic objects drew theirs: the nearer of the two shows, and a dynamic object behind a static one is hidden by the plate.
- **Ids, motion and normals.** It writes the plate's object id into the pick target (so `Renderer::pick` reads the static object where nothing dynamic covers it), the camera's motion into the velocity target, and the plate normal with roughness 1 into the normal target, so screen-space reflections never trace from a plate pixel; dynamic glossy surfaces still reflect the plate.
- **Daylight.** The plate's anchors bracket the hour set with `Renderer::set_plate_hour` (`Staged` sets the scene's sun hour). The pass blends the two nearest anchors per pixel in linear light: the light that is not the sun's direct share blends linearly, and the direct share is blended per unit of each anchor's sun (`suns` in the manifest) and scaled by the current sun, so a dimmer or warmer sun between anchors dims or warms the share it lights. Past the first or last anchor the plate holds that anchor.
- **Relighting between anchors.** A shadow cannot be cross-faded from one anchor's sun to the next: half of each lies where neither is. With the `transfer` layer (the unshadowed direct sun at each anchor), the pass also relights the direct share at the current sun: it recovers each texel's response to the sun from the two anchors' transfer layers and their sun directions, `k = (T_a/E_a·n·l_a + T_b/E_b·n·l_b) / ((n·l_a)² + (n·l_b)²)`, and lights it with the current sun `E·k·max(n·l, 0)` under the frame's own sun cascades, which hold the static stand-ins and the dynamic casters. The traced share and the relit one are mixed by `1 − |1 − 2t|`, so each anchor shows exactly its trace and the middle between two is relit live. `PlateSettings::relight` turns it off (`Renderer::set_plate_settings`).
- **Shadows of moving parts.** The plate's shadow pass renders every dynamic instance that casts (`Instance::casts_shadow`, shadow-only ones included) into an orthographic map fitted to the plate's `bounds` along the current sun, with no static caster in it. The composite samples it at the plate point (offset along its normal), 3×3 with a comparison sampler, and takes the sun's share away by what the casters hide: `colour − share·(1 − lit)·shadow_strength`. A plate without a `sun` layer takes no shadows from moving parts. Contact shadows, sky occlusion by moving parts and their bounce onto the plate are not drawn (phase 3).
- **Static casters.** The renderer's own cascades shadow the dynamic objects. With a plate set, `Renderer::set_plate_casters(&[Instance])` adds stand-ins for the static objects to them as shadow-only instances that never draw: `Staged` passes each static object's own mesh (`[plates] casters = "meshes"`), the proxies (`"proxies"`), or nothing (`"none"`). The plate's shadow pass leaves them out.
- **Lighting the moving parts.** Dynamic objects are shaded live as before: sun, sky, local lights and whatever probes and reflection cubes the app set (`Renderer::set_probes`, the reflection upload), which should come from the same bake as the plate so both agree.
- **The sky.** Plate texels with no surface (the sky seen past the scene) are left to the live sky pass.

`Staged` (`Renderer::stage`) handles a scene with `[plates]` itself: it reads the plate folder's manifest (a folder without one is drawn live, whole, and `Staged::plated()` says so), leaves the static objects, static text and emitters out of the frame, gives the static objects to `set_plate_casters`, and sets the plate hour from the sun. A re-bake rewrites `manifest.json`, which `SceneWatch` sees, and `Staged::apply` reloads the plate. Overrides on static objects (hide, recolour, highlight) show nothing while plated, since the plate is baked.

### Frames on demand

A plated scene takes part in frames on demand (`Renderer::set_frames_on_demand`, `docs/live-renderer.md`) like any other: the plate composite is part of a full frame, so the cached frame holds it.

- **Still.** With a still camera and no moving part, `frame_needed` answers `None`: nothing is submitted and `present_last` re-presents the last image.
- **Drift.** A drift inside the limits answers `Reproject`: the cached frame (plate and moving parts together) is warped by its depth, without running the composite.
- **Moving parts.** A moving part, or a drift past the limits, answers `Full`, and the frame redraws the moving parts over a fresh composite. A camera-facing card (`face_camera`) turns with the camera, so with one in view every drift is a full frame. So does surface text laid out for another camera: `Staged::text` uses the scene's `[camera]`, so a product that moves the frame camera itself must lay its moving text out for that camera, or each drift reads as moved text.
- **What redraws the plate.** `set_plates`, `set_plate_casters`, an hour that differs (`set_plate_hour`) and settings that differ (`Renderer::set_plate_settings`) each bump the renderer's revision, so the next frame is full. Setting the same hour or the same settings again changes nothing.
- **The test.** `a_plated_scene_on_demand_idles_still_reprojects_a_drift_and_redraws_moving_parts` checks each case.

## Look

`crates/live/tests/plates_look.rs` (`plates_look_like_the_trace_at_an_anchor_and_between_anchors`, GPU) renders the neutral desk (`crates/bake/tests/plates/desk.scene.toml`: floor, two walls, a desk on four legs, a lamp, a shelf and a metal orb, all static; a sliding sheet, a camera-facing screen card showing an image, a hinged lid and a line of text, all dynamic) at 640 × 400 three ways at the same hour: the full trace of the whole scene (adaptive, threshold 0.005, up to 1,024 samples), live with plates (plate baked at threshold 0.005, up to 1,024 samples), and live drawing everything (no plates). Each pair goes through the same display transform (clamped, sRGB) into CIEDE2000; "dynamic" pixels are the ones that change when the dynamic objects are hidden (the objects and the shadows they cast), "static" the rest. It writes every image and `tmp/plates/contact.png`: per row, the trace, live with plates, live without, and the ΔE of each live frame against the trace (blue none, orange 6 or more).

Measured 2026-10-06 on the RX 9060 XT (mean and 95th-percentile CIEDE2000 against the full trace; "full raster" is the same scene drawn live with no plate):

| case | region | pixels | plates mean / p95 | full raster mean / p95 |
|---|---|---:|---:|---:|
| at 12 h, anchors 12 and 17 | static | 246,902 | **0.51** / 1.18 | 5.45 / 11.38 |
| | dynamic | 9,098 | 3.87 / 9.03 | 3.98 / 9.03 |
| at 14.5 h, anchors 12 and 17, blended only | static | 245,738 | 9.29 / 18.88 | 8.27 / 21.23 |
| at 14.5 h, anchors 12 and 17, relit | static | 246,989 | **2.30** / 5.91 | 8.27 / 21.20 |
| | dynamic | 9,011 | 4.45 / 9.98 | 4.55 / 9.27 |
| at 14.5 h, anchors 14 and 15, relit | static | 246,989 | **0.99** / 2.17 | 8.27 / 21.20 |
| | dynamic | 9,011 | 4.04 / 9.39 | 4.55 / 9.27 |

- **At an anchor** the static part is the trace: 0.51 mean is the two traces' own noise (the plate and the reference are traced with different seeds). Live drawing everything is ten times further off (5.45), mostly brighter walls and softer shadows.
- **Between anchors**, a plain blend of anchors five hours apart cross-fades two shadow layouts and two sun strengths (9.29, worse than drawing everything live). Relighting the direct sun under live cascades brings it to 2.30, and anchors an hour apart to 0.99. The residue is where the live cascades and the trace's soft sun disagree (shadow edges, the thin side wall at grazing sun) and the sun's bounce, which stays blended.
- **Dynamic regions** are drawn live in both, so they stay at raster quality (about 4): the objects themselves, and the shadows they cast on the plate.

## Cost

`crates/live/examples/plates_bench.rs` bakes the desk at the frame size and times 240 frames after 30 of warm-up, GPU time from the renderer's pass timestamps, under `pgpu run --class clip`:

| frame | 1280 × 800 GPU p50 / p95 | 3840 × 2160 GPU p50 / p95 |
|---|---:|---:|
| full redraw (no plate), still | 0.55 / 0.56 ms | 4.39 / 4.42 ms |
| full redraw, drift and moving parts | 0.90 / 0.92 ms | 4.77 / 4.87 ms |
| plates, still | 0.48 / 0.49 ms | 3.64 / 3.67 ms |
| plates, drift | 0.79 / 0.82 ms | 4.01 / 4.07 ms |
| plates, drift and moving parts | 0.84 / 1.09 ms | 4.01 / 4.10 ms |

- **Where the time goes at 4K.** The plate takes the opaque pass from 1.84 ms to 0.20 (only the four moving parts are shaded), and the composite costs 0.98 ms still and 1.09 to 1.11 ms drifting (the 4K column is after the depth taps became one `textureGather`, which took 0.12 to 0.15 ms off it; the 1280 × 800 column is from before) (reprojection steps, two anchors of colour, sun and transfer, the moving parts' shadow). The rest of the frame is the passes every frame runs whatever is drawn: TAA resolve 1.08, scene colour mips 0.26, post 0.22, reflections 0.21. The neutral desk is light (its full redraw is 4.4 ms, against 10.5 ms for a product desk with 6.6 ms of opaque PBR), so the saving here is 0.75 ms; on a scene whose opaque pass dominates it is that pass, less about 1.1 ms.
- **Idle.** With frames on demand a still plated frame submits nothing (`present_last` re-presents it for 0.15 ms at 4K), and a drift inside the limits is a reprojection of the cached frame (about 0.5 ms at 4K, `docs/live-renderer.md`); only moving parts or a drift past the limits pay for the composite and the moving parts.
- **Plate memory**, as uploaded and on disk: 14.9 MB per anchor and 9.9 MB shared at 1280 × 800 (1408 × 880 texels), 120.4 MB per anchor and 80.3 MB shared at 4K (4224 × 2376 texels), with colour, sun and transfer in RGB9E5. Without `transfer`, 80.3 MB per anchor at 4K; BC6H would bring the three per-anchor layers to 30 MB.
- **The bake** of the 4K plate at two anchors (threshold 0.01, up to 256 samples, six traced layers) took 56 minutes of wall time on a GPU shared with other jobs; the 1280 × 800 plate took 7 minutes.

## Tests

- `crates/load/src/scene/plates.rs`: `dynamic_is_derived_from_movers_cards_and_parents_and_overridden_by_the_key`, `dynamic_text_leaves_the_static_subset_and_static_text_stays`, `plates_read_their_manifest_and_proxies_and_watch_both`, `plates_refuse_a_dynamic_proxy_an_unknown_kind_and_casters_without_proxies`, `open_at_sets_the_sun_hour_and_the_sky_follows`.
- `crates/bake/src/plate/codec.rs` and `tests.rs`: RGB9E5 within its mantissa and in the GPU's bit layout, octahedral normals within 0.01°, inverse depth within half a step, the recipe's defaults and refusals, the view camera, the plate camera against the live projection, strip cameras against the whole plate, the anchor blend, and the artifact's round trip from a folder and from bytes, its refusal of a changed byte or manifest, and float depth.
- `crates/bake/tests/plates.rs`: `the_key_holds_only_the_static_subset` (CPU) and `a_plate_bakes_the_static_desk_at_its_anchors_and_keeps_it_while_its_key_holds` (GPU): ids hold only static objects, depth and normals match the desk top, the sun's share never exceeds the colour, an unchanged key keeps the plate, the same inputs bake the same bytes, a dynamic edit keeps it and a static edit re-bakes.
- `crates/live/src/plates.rs`: the shaders validate and the uniform matches the shader's struct.
- `crates/live/tests/plates.rs` (GPU): `a_plated_frame_is_the_plate_where_nothing_moves_and_draws_the_moving_parts_over_it` (with the dynamic objects hidden every surface pixel is the plate within 1%; between anchors, relighting off, the frame is the sun-normalised blend; dynamic objects draw over the plate, a dynamic object behind the wall is hidden, picks read the desk, the wall, the lid and the card, and the lid darkens the desk under it without brightening anything) and `a_small_drift_reprojects_the_plate_by_its_depth` (after a 7 cm drift, five points on the desk, the wall and the floor read the plate where their surfaces lie, within 0.3%), and `a_plated_scene_on_demand_idles_still_reprojects_a_drift_and_redraws_moving_parts` (frames on demand: a still plated desk settles and then submits nothing, and `present_last` shows the same bytes; a new hour redraws while the same hour or settings change nothing; a 4 mm drift reprojects without the composite; moving parts redraw full frames with the composite every tick).
