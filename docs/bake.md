# Probe baking

`pfx bake --scene <dir> [--assets <dir>] [--out <dir>] [--anchors 8,12,16,20] [--samples N] [--name label] [--force] [--detail]` traces the scene's probe volumes and prerendered props and writes versioned, checked artifacts. It is part of `pfx`: the command re-enters the build slice like render, then runs its GPU work as a second `pfx bake` under `pgpu` (class `truth`), taking a `pgpu turn` after about every 50 ms of measured GPU work and between anchors, each reporting `--work-ms` and `--longest-ms` (`docs/pacing.md`). `PFX_DIRECT=1` bypasses the wrapper for GPU tests only. `crates/bake` stays a library the apps can compile in; it never links the harness, the queue or a job-status crate.

The default output is `<caller git root>/tmp/bakes/<scene folder name>`. `--out` must be inside that caller's `tmp/`, including when an absolute path is used. With one volume, `--out` is the artifact folder. With several volumes, each gets a named subfolder. An unchanged scene and recipe with the same sample budget and seed is skipped. A different result at an existing path requires `--force`.

A scene folder contains `bake.toml` and a glTF or GLB. With `--assets <dir>`, or with `assets_env = "NAME"` in the recipe (the variable `NAME` then holds the folder, and a missing variable is an error that names it), the recipe stays in the scene folder and every file it names (glTF, buffers, HDR sky) is read from that folder instead, under the same rules: nothing escapes it. The flag wins over the variable. A product's own assets stay in its repository this way and are never written. External glTF buffers and an HDR sky must also be inside the folder. Paths cannot escape it, including through symbolic links. Scene nodes and transforms are loaded by `pfx-load`. It also reads glTF `extras` as written (`serde_json::Value`, `None` when absent) onto every `Node`, mesh (`Group`) and `Material`; `extra_str`, `extra_f64`, `extra_bool`, `extra_array`, `extra_object` and `extra_f64s` return `None` for a missing or mistyped key. Extras do not enter the scene hash, which covers triangles, materials and shapes only. By default glTF base colour, roughness, metalness and emission factors feed the tracer. `materials = "<library.toml>"` names a material library the product ships (`pfx_materials::Library`, `docs/materials.md`) by a path relative to the recipe's folder, not the assets folder; it may climb out of it with `..`, so a product can keep one library beside all its recipes. Each glTF material takes the library entry of its name, else of its name before the first `.` (Blender's `.001` suffixes), else `fallback = "<name>"`, else `Material::default()`. The library's bytes and path join the scene hash. Texture maps are not yet loaded into the bake. Analytic shapes and lightmap UVs are future extensions.

```toml
scene = "room.glb"
app = "my-app"
materials = "../materials.toml"
fallback = "grey"
anchors = [8.0, 12.0, 16.0, 20.0]
samples = 64
seed = 7
day = 172
latitude = 45.0
heading = 180.0

[sky]
kind = "hdr"
path = "sky.hdr"
rotation_deg = 169.4
intensity = 4.2
ambient = "authored"

[sky.prepare]
cap = 60.0
balance = [1.0, 0.92, 0.8]
mean = 0.3

[sun]
model = "authored"
toward = [0.4507896, 0.6210879, 0.6411229]
color = [1.0, 0.86, 0.66]
irradiance = 4.5

[tint]
plaster = [0.9, 0.85, 0.8]

[[volumes]]
name = "room"
min = [-2.0, 0.5, -2.0]
max = [2.0, 2.5, 2.0]
spacing = 0.5
```

`app` is the caller's slug (lowercase letters, digits and hyphens). It names the bake for attribution and changes nothing in the light. The `day`, `latitude` and `heading` values give the clock's place. `reference_hour` (0 to 24, default `daylight::REFERENCE_HOUR`, noon) is the hour at that place whose sun has intensity 1 and ambient 1: `Daylight::light` and `Daylight::authored` divide by it, so it sets how bright the fixed and authored suns and their ambient read through the day. Anchors are strictly increasing hours from 0 through 24. `--anchors` and `--samples` override the recipe for one run.

- **Sun.** `[sun] model` picks how each anchor's sun is made, and each model takes exactly its own values:
  - `"solar"` (the default without `[sun]`): the sun of `Daylight` at that hour, day, latitude and heading. It takes no values.
  - `"fixed"`: `toward`, `color` and `irradiance` are required. The direction and colour stay put; the intensity is `irradiance` times the daylight's intensity at the hour.
  - `"authored"`: `toward`, `color` and `irradiance` are required, and `sky_fill` (default 0) is optional. `Daylight::authored` turns and tints the authored direction by the hour, and the intensity is `irradiance` times its intensity. `sky_fill` lifts the ambient by `1 + sky_fill × cool` while the turned sun is far from the authored one and above the horizon.
  - `"clock"`: a `[clock]` table is required, holding `toward` and every `DampedClock` field (`sun`, `tint`, `elevation`, `azimuth`, `skywarm`, `lamp`, `exposure`, `warmth`, `turn`), with a `[clock.arc]` table giving the sun's path: `start` and `span` hours, a peak `elevation` and a starting `azimuth` in degrees, the `sweep` in degrees over the span, and the `reference` hour the clock holds `toward` at (`SunArc`; every key required, `span` positive). Each anchor's sun is `DampedClock::at(hour, toward)` at intensity 1. `[sun]` itself then takes no values.

  `[sun]` without `model` and `[clock]` without `model = "clock"` are refused.
- **Sky.** `[sky] kind` is `"daylight"` (the default: a flat fill from the daylight's sky level), `"hdr"` (with `path`) or `"room"` (with a `[sky.room]` table, below). `rotation_deg` turns an HDR or room sky by its negative, then `intensity` scales it; a daylight sky takes neither.
- **Preparation.** `[sky.prepare]` prepares an HDR sky before it turns: `cap` limits each texel's luminance, `balance` sets the mean colour's balance, `mean` scales to that mean luminance, in that order (`Sky::capped_luminance`, `balanced_to` and `scaled_to_mean_luminance`). Each is optional, but the table needs one, and each must be positive.
- **Ambient.** `ambient = "authored"` (with `[sun] model = "authored"`, on an HDR or room sky) scales each anchor's sky by `Daylight::authored(&authored, reference_hour).ambient` for its hour. `ambient = "clock"` (with `[sun] model = "clock"`, on an HDR sky, with `[sky.prepare]` holding `mean` alone, and no rotation or intensity) exposes the raw HDR to `mean` each anchor, turns it by the clock's `turn` and multiplies it by the clock's shade for the hour; `[sky.shade]` overrides that shade per anchor.
- **Tints.** `[tint]` maps library names to linear RGB multipliers of their `base`. It needs a library, and every name must be in it.

`shadow_only_nodes = ["name", …]` lists glTF nodes that cast shadows in the app but are never seen, such as a light rig's occluders; the bake leaves their triangles out of probes, reflections and props. Each volume is a `GridSpec`: three dimensional minimum and maximum bounds on multiples of `spacing`. A finer volume over a recess narrower than the main grid (a 1 cm lip under a ledge, say) is baked the same way and drawn live as a local volume of the main one (`ProbeLighting::from_artifacts`, `docs/live-renderer.md`); the bake stores nothing more for it. A prop-only scene may omit `[[volumes]]`.

A recipe that names a library is refused when its `fallback`, a `[tint]` name, a `[[pose]]` material or an `[[emitter]] preset` is not in the library, when the library is missing or malformed, or when its path is absolute.

### Older spellings, removed in v0.20.0

v0.20.0 removed the product spellings the bake used to accept. The bake carries no product's numbers any more: a recipe states every value it uses, and each old spelling is refused with an error that names its new key. `product` stays only as a plain alias of `app` (a recipe takes one of them), and every product name is just a slug.

| Removed | Use |
|---|---|
| `kind = "<old name>"` | `kind = "room"` with a `[sky.room]` table |
| `prepare = "<old name>"` and a top-level `balance` | a `[sky.prepare]` table of `cap`, `balance` and `mean`, with `ambient = "authored"` where the hour should scale the sky |
| `ambient = "<old name>"` | `ambient = "authored"` with `[sun] model = "authored"` |
| `[sun]` values without `model`, or a product slug standing for an authored sun | `[sun] model = "authored"` with `toward`, `color`, `irradiance` and `sky_fill` |
| `materials = "<set name>"` | `materials = "<library.toml>"` with `fallback` |
| an emitter without `preset` | the library's `fallback` |

A recipe without `[sun]` and `[clock]` is lit by the solar model. `kind = "analytic"` is the old name of `kind = "daylight"` and is still read. The bake crate's tests (`recipe::keys`) cover each refusal and each new spelling on a neutral app named `test`, and the ignored GPU test `product_and_app_bake_the_same_bytes` bakes three recipes under both keys to the same artifact bytes.

The sample budget is per face texel of each probe. A probe has six faces with 16 texels each, so `samples = 64` traces 6,144 rays per probe and anchor. Each lobe is the cosine-weighted irradiance `∫ L cos θ dω` around its axis, so a unit sky bakes π on every lobe; the 4×4 face quadrature is normalized to give exactly that (before, it gave 3.6% more).
The bake loads up to 512 probes into one batch and traces it in Pacer-sized dispatches of a few samples each, about 3.5 ms apiece, accumulating samples in place; a batch whose single sample overruns 6 ms makes the next batch smaller. It preserves the old face's pixel seed and sample order, so the bytes match the 64-probe, one-dispatch batches it replaced. `bake_anchor_slow` remains available for byte comparisons and timing checks. A desk scene baked its 19×11×12 probe grid at three anchors and 16 samples in 6.5 s on the current tracer, compared with 6.6 s before the detailed-triangle change.

### Rounds

The probe bake runs in rounds of `samples / 4` samples per face texel (16 at 64 samples). Every probe gets at least four rounds; from the fourth on, a probe whose rounds agree to within 4% of its level stops, and the rest go on to 64 rounds. `bake_anchor_paced` returns a `Baked { grid, rounds }` and `bake_anchor_rounds` is the same call without pacing; `bake_anchor` still returns only the `Grid`. `Rounds { enabled, active }` holds the probes that could be sampled and, after each round, how many were still active: `run()` is the number of rounds and `converged()` the probes that stopped in each.

`pfx bake` prints one line per anchor, `<artifact> at <hour>h: <probes> probes, <rounds> rounds, converged per round [..], <seconds> s`. The artifact's `manifest.json` records the same as `"rounds": [{ "enabled": N, "active": [..] }, ..]`, one entry per anchor, and `read_artifact` returns it as `Artifact::rounds` (`None` for an artifact written without it). The record is informational. It stays out of `scene_hash` and out of the probe files, so two bakes of the same inputs write the same `probe-NNN.bin` bytes and the same manifest.

### The room sky

`kind = "room"` builds the sky from a `[sky.room]` table with `pfx_load::Sky::room` and a `RoomSky` of `RoomLamp`s. Its keys and neutral defaults: `width` (64; the sky is `width × width/2`, from 2 to 8192), `floor`, `wall` and `ceiling` (linear RGB, each 0.5 grey; a vertical blend from wall to floor below the horizon and wall to ceiling above), and `[[sky.room.lights]]` lamps with `name` (optional), `az` and `el` in degrees (0 and 0; azimuth 0 looks along +Z, positive turns towards +X), `power` (1) times `color` (white), angular `width` and `height` in degrees (45 and 45), `soft` (0.1, the edge falloff as a fraction of the half extent), and blinds: `slats` bands across the lamp's height (0 for none), each `open` (1) of its band lit. Unknown keys are refused.

```toml
[sky]
kind = "room"
rotation_deg = 0.0
intensity = 1.0

[sky.room]
width = 1024
floor = [0.2197, 0.1352, 0.0676]
wall = [0.19, 0.13, 0.07]
ceiling = [0.2845, 0.2276, 0.1593]

[[sky.room.lights]]
name = "window"
az = -64.0
el = 14.0
width = 37.5
height = 29.0
power = 10.585
color = [1.0, 0.86, 0.66]
soft = 0.05
slats = 9.0
open = 0.62
```

`rotation_deg` and `intensity` apply as for an HDR: the room turns by the negative of `rotation_deg`, then scales by `intensity`. `prepare` and `balance` are refused, as is `path`; a missing `[sky.room]` is an error naming it, and `[sky.room]` with another kind is refused. By default the room is the same at every anchor. With `ambient = "authored"` and `[sun] model = "authored"`, each anchor's room is scaled by the hour dependent ambient, `Daylight::authored(&authored, reference_hour).ambient` from the recipe's `[sun]` and `reference_hour`, the same scalar the authored ambient applies to an HDR; live, the same `.exposed(light.ambient)` on `Sky::room` matches the bake at every anchor. `[sky.shade]` still applies. The scene hash takes the room (its canonical JSON beside the recipe bytes, and its texels through every anchor's sky), so a changed lamp rebakes.

## Local reflections

Add `[[reflection]]` entries for objects that need nearby specular detail. A reflection uses a world-space position and an axis-aligned box: the box says where the cube applies (with its `fade` seam); a cube baked with distances (v2) corrects its parallax by them, so the box need not match the room's walls, and only a v1 cube treats the box as the geometry it reflects. `resolution` is the width of one cube face and defaults to 256; it must be a power of two from 4 through 1024. Optional `anchors` override the scene hours. `fade` is the seam width at the box boundary, in world units, and defaults to 0.25. An optional `priority` raises a probe's weight in an overlap. A scene may contain reflections without a diffuse probe volume.

```toml
[[reflection]]
name = "desk"
position = [0.0, 1.0, 0.0]
min = [-2.0, 0.0, -2.0]
max = [2.0, 2.5, 2.0]
resolution = 256
anchors = [8.0, 12.0, 16.0, 20.0]
fade = 0.25
```

The CLI writes each reflection under `<output>/reflections/<name>/`. An omitted name becomes `reflection-000`, numbered in recipe order. Its schema 1 manifest records the scene hash, recipe, anchor hours, sample count, seed, format, and the SHA-256 and byte count of every `cube-NNN.bin`. `pfx_bake::reflection::read_reflection_artifact` checks the manifest and each cube. `read_reflection_artifact_from(&|name| Option<Vec<u8>>)` checks the same from bytes (`manifest.json`, `manifest.sha256`, `cube-NNN.bin`), and the path reader wraps it; `upload_reflections` takes the parsed artifacts, so embedded cubes go from `include_bytes!` to the GPU without a folder. Every cube stores RGBA16F linear Rec. 709 radiance in RGB and, in alpha the inverse of the distance from the capture point to the first surface along each base texel's direction (0 where the ray leaves the scene), faces in positive X, negative X, positive Y, negative Y, positive Z, negative Z order, with GGX-prefiltered mips from sharp to rough; the inverse distance is filtered with the radiance, so open sky beside a wall doesn't push the wall away the way an averaged distance would. The distance comes from a CPU ray cast against the scene's triangles and shapes, so it is exact and deterministic. The manifest's format is `cube_array_rgba16f_ggx_mips_inverse_distance_face_major_le_v2` (`REFLECTION_FORMAT`); the reader still accepts `cube_array_rgba16f_ggx_mips_face_major_le_v1` (`REFLECTION_FORMAT_V1`), whose alpha is 1, and the live renderer corrects those cubes' parallax by their box alone, as before. The reflection hash's salt moved to v2 with the distances, so `pfx bake` rebakes every existing cube once, recipe unchanged. Its 16-byte header is `PITOREF1`, face resolution and mip count, followed by face-major texels at every mip in little-endian half floats. The same inputs and GPU/driver produce byte-identical cubes.

For the joined renderer, load each reflection artifact, call `upload_reflections(device, queue, &artifacts, hour)` to create the `texture_cube_array` view for group 3 binding 14, and pass the returned `max_mip` as `reflection_radiance`'s final argument. The returned `ProbeRecord` has exactly the WGSL `ReflectionProbe` field order and 48-byte layout: center and layer; box minimum and priority; box maximum and fade. At each surface, `nearest_two(&records, world_position)` returns two indices and weights, including edge fade. Pass their records and weights to `reflection_radiance`. Rebuild or update the array when the anchor hour changes. The live renderer currently binds a black placeholder and passes two empty records; its caller supplies this hook.

Each artifact contains `manifest.json`, `manifest.sha256`, and one `probe-NNN.bin` per anchor. The manifest records the scene hash, sample count, seed, grid, binary format, coordinate system and SHA-256 of every probe file. `pfx_bake::read_artifact` verifies the manifest and probe bytes. `read_artifact_from(&|name| Option<Vec<u8>>)` does the same from bytes: it asks the closure for `manifest.json`, `manifest.sha256`, each `probe-NNN.bin` and `emitters.bin` by file name, so an app that embeds its bake with `include_bytes!` never unpacks it to a folder. It checks the manifest and every file's byte count and SHA-256 exactly as the path reader does, and a file the closure cannot give is an error naming it (`missing bake file <name>`). `read_artifact(dir)` is a thin wrapper over it. `ProbeLighting::from_artifact(device, queue, &artifact, hour, fallback)` takes the result straight to the GPU, and `ProbeLighting::load` is `read_artifact` followed by it. The scene hash includes the recipe bytes, glTF bytes, external buffer and HDR bytes or room, the material library's path and bytes when the recipe names one, engine version, trace inputs, volume, sample count and seed. The binary stores RGB16F ambient cube lobes and F16 visibility in right handed Y-up coordinates; X changes fastest within a grid. The same inputs produce byte-identical probe data on the same GPU and driver.

## Prerendered props

A scene may add `[[prop]]` tables to `bake.toml`. Each prop selects one named glTF node and its descendants from the scene file, or all nodes in a separate glTF/GLB file inside the scene folder. `frames_per_side` is the octahedral view grid width; `atlas_size` is the full square atlas width in texels. The atlas size must divide evenly by the frame count, leaving at least eight texels per view. The default is a full sphere. Set `hemisphere = true` when all live views are above the prop; lower views are mirrored onto the upper hemisphere. A prop's optional `anchors` overrides the scene hours. Anchors must be strictly increasing from 0 to 24.

```toml
[[prop]]
name = "desk"
node = "Desk"
frames_per_side = 8
atlas_size = 2048
anchors = [8.0, 12.0, 16.0, 20.0]

[[prop]]
name = "vase"
file = "vase.glb"
frames_per_side = 8
atlas_size = 2048
hemisphere = true
```

The CLI writes each prop under `<output>/props/<name>/`. When props and probes share a scene, each probe volume also gets a named subfolder under `<output>/`. Its schema 1 manifest names the bounds, view layout, daylight anchors, sample count, seed, SHA-256 and byte count of every file. `geometry.bin` stores four F32 channels per texel: world normal XYZ and camera-ray hit distance. `radiance-NNN.bin` stores premultiplied linear Rec. 709 radiance RGB and coverage as four F16 channels for one anchor. A fully covered texel has alpha 1; edge texels retain the tracer's fractional first-hit coverage. The same verified inputs are skipped on later runs. Changed inputs at an existing output path require `--force`; a replacement is rendered and checked before the old artifact is swapped.

`pfx_bake::impostor::read_prop_artifact` verifies the manifest and every payload before a live renderer uploads it. `pfx_live::impostor::ImpostorPass` accepts those bytes, camera uniforms and instances; it draws camera-facing cards into a scene-linear color target with a depth attachment, or into a sun shadow depth target. `mesh_fraction(distance, switch_distance, transition_width)` returns the caller's LOD crossfade weight. The mesh pass must use `lod_keep` with that weight and the same object ID and pixel coordinates. The app selects the switch distance from its content and should keep the prop's texture atlas resident before entering the transition.

The ignored `rounded_box_and_chrome_sphere_at_4k` GPU test writes `tmp/impostor-side-by-side.png`. It compares the mesh and impostor at 3840×2160 using mean absolute error after a simple `x / (1 + x)` scene-linear tone map, reports foreground error separately, and timestamps a draw of 200 instances into existing color and depth targets, excluding their clear. Run it through `pgpu`.

## Procedural detail maps

Add `[detail]` to `bake.toml` and run `pfx bake --scene <dir> --detail`. This CPU bake rasterizes static mesh parts with unique UVs into base color, roughness and tangent-space normal maps. It crops each part's maps to the UV box its triangles use, packs the crops into atlas pages and stores the pages block-compressed as KTX2 in `<output>/detail/`. `pfx_bake::detail::atlas::read` verifies a folder, and `atlas::load` reads either format (below). An unchanged bake is reused. A changed one needs `--force`, and it replaces the folder only if the folder holds nothing but a detail artifact, of either format.

```toml
[detail]
size = 1024
padding = 8
page = 2048
dynamic_nodes = ["MovingPart"]
```

`size` is the largest map, a power of two from 64 to 4096, default 1024. `padding` defaults to 8 texels and may not exceed a quarter of any map's size. `page` is the atlas page side, a power of two from 64 to 16,384 (the live floor's 2D side), default 2048. `dynamic_nodes` lists additional glTF node names that move; animated glTF nodes and their descendants are skipped automatically. The default list is empty. `parts` names the mesh nodes to bake. The default list is empty, which bakes every static part. A name that is not a mesh node is an error. The baked parts keep the scene-order numbers they have in a full bake, and their bytes match a bake that lists every other node in `dynamic_nodes`. An animated node, or one named in `dynamic_nodes`, stays procedural when `parts` also names it. Parts without active procedural layers are skipped. Parts whose UVs repeat within the tile or fall outside it keep procedural detail, with their names reported, so repeated textures are never flattened into a map. A detail-only scene needs no probe volume or reflection. The CLI runs `--detail` on the CPU without entering the GPU queue.

Each part gets its own size. With a `[detail.view]` camera the size follows the part's screen footprint: for every triangle edge it takes the screen length in pixels over the edge's length in UV, the largest of those is the texels the map needs across the tile, and the size is that rounded up to a power of two between 64 and `size`. Without a view every part takes `size`. `[detail.sizes]` overrides a part by node name (the glTF node's name, or its mesh's when the node has none) with any power of two from 64 to 4096, even above `size`; a name that matches no node is an error, so a typo cannot pass quietly.

```toml
[detail]
size = 2048
padding = 8

[detail.view]
position = [0.0, 0.12, 1.6]
target = [0.0, 0.12, 0.0]
fov_y_deg = 13.37775
shift = [0.0, -0.6395161]
width = 3840
height = 2160

[detail.sizes]
"knob" = 512
```

The view is a pinhole looking from `position` at `target` with +Y up. `fov_y_deg` is the vertical field of view, `shift` the lens shift in normalized device coordinates (a positive x moves the picture left), and `width` and `height` the frame in pixels (3840 by 2160 unless given). `pfx_bake::detail::View::look` builds the same matrix as `studio_desk`'s `Lens`, which the example's test `the_recipe_sizes_every_part_like_the_example` checks, matrix and sizes.

### Crops, pages and mips

A part's size is its density, texels per UV unit. `detail::crop` takes the texels whose centres its triangles cover, widens that box by `padding` and then outward to multiples of 16 texels, inside the tile. `detail::bake_crop` rasterizes only that box, and it holds exactly the texels the part's full-tile map has at the same places, byte for byte (`a_crop_is_the_uv_box_with_padding_on_the_alignment_and_bakes_like_the_tile`). Outside the box the full-tile map is empty.

`atlas::assemble` packs the crops onto pages. It shelf-packs them tallest first, ties broken by width and then part number, so the same parts always land in the same places. When the parts need several pages, every page is `page`². A single page shrinks to the smallest power-of-two rectangle that holds them, trying narrower shelves. Sides stay powers of two because the live renderer samples a part at `uv × scale + offset`. With power-of-two sides that is exact in f32. With trimmed sides that are not powers of two, the rounding moved samples across the GPU's sub-texel filter steps and put the desk test 1.05/255 off. A crop larger than `page` raises the page to the crop's power of two.

Each page has five mip levels, built exactly as the live renderer builds a map's chain: colour filtered in linear light with premultiplied alpha, normals renormalized. Because crops and places sit on 16-texel boundaries, levels 0 to 4 of a part are the full-tile map's levels texel for texel, and no level mixes two parts. A part drawn more than 16 times smaller than its density stops at level 4.

### A second UV set

A part whose glTF primitive carries `TEXCOORD_1` bakes through it; every other part bakes through `TEXCOORD_0` as before. `pfx_load::Primitive::uvs1` holds the set (`None` when the file has none, so such a file loads as it did), and `detail::Input::uvs1` hands it to the bake. `Input::raster()` is the set the bake lays out in the tile: the footprint size, the repeat check, the crop and the rasterizer all use it, so a part whose `TEXCOORD_0` repeats (planks that share one plate projection) still bakes once its `TEXCOORD_1` is unique. The procedural layers are still evaluated as the live renderer evaluates them: at the texel's world position, and at its `TEXCOORD_0` interpolated with the same barycentric weights, not at the texel's place in the second layout. Tangents still come from `TEXCOORD_0`, because the live renderer decodes the normal map in the mesh's own tangent frame. The input hash takes the second set in only when there is one.

A part without `TEXCOORD_1` takes exactly the old path, texel centre and all, so its crop holds the same bytes as before. Only the input hashes change, as they do with any edit to the bake's code, since they hash `detail.rs`; a cached atlas rebakes once to the same pages.

manf writes `TEXCOORD_1` on the objects a recipe marks: a unique, non-overlapping layout sized by footprint, with 16-texel gutters (the atlas alignment, so mips stay clean). On its deck (commit 45db4e9) only the `desk` node has one, at 835 texels per metre, a 2004 × 1419.5 chart in a 2048 map.

Measured on a product's desk scene with that deck, at its camera, 2026-10-05:

- **The desk's bake.** 2048², crop 2032×1456 at (0, 592), through `TEXCOORD_1`, 9,855,080 bytes on the page. PSNR against the uncompressed crop: base 56.9 dB, roughness 53.8 dB, normal lossless (the wood layer leaves the normal flat). With the six likely parts the atlas takes two 2048² pages, 27,936,272 bytes on disk. The other six parts' crops are the same bytes as before.
- **The detail check.** Over the seven parts' 969,564 pixels, the BC frame is 53.7 dB against the uncompressed atlas, worst 45/255; the CPU-decoded frame is 53.0 dB, worst 62/255. Full-tile maps sample `TEXCOORD_0`, so the check skips its tile frames when a part bakes through `TEXCOORD_1` and compares against the cropped atlas instead.
- **The opaque pass at 4K**, light bakes on, 360 paired frames each, median ms at rest with the GPU idle, on the tree before the noise footprint fade (`fine_noise`) merged (a first set, run while the GPU was shared with a game, came within 0.5 ms of these):

  | parts baked | shipped, the rest procedural | detail compiled out | the procedural detail left |
  |---|---:|---:|---:|
  | none | 10.77 | 6.02 | 4.77 |
  | the six likely parts | 10.62 | 6.29 | 4.33 |
  | the six and the planks | 10.20 | 6.02 | 4.21 |

  Baking the planks takes 0.2 to 0.4 ms off (0.20 in the shared set, 0.42 here), because at that camera the desk shows on 325,200 of the frame's 8.3 million pixels, most of it hidden under the machine. None of the three reaches the 4.1 ms budget: even with every procedural layer compiled out the pass takes about 6 ms on this build. Wave 29 measured 6.50 ms procedural and 3.56 ms plain on that deck; on the same day as these runs, the deck measured 10.37 ms procedural both before and after this change, so the second UV set costs nothing and the gap to wave 29's figures lies elsewhere.
- **The planks, procedural against baked**, on the 325,200 pixels the bake changed: mean 1.23/255, worst 31/255 (mean 0.08/255 over the whole frame).

### Files and manifest

`<output>/detail/` holds `manifest.json` (with `manifest.sha256`) and three KTX2 files per page, `page-NNN-base.ktx2`, `page-NNN-normal.ktx2` and `page-NNN-roughness.ktx2`:

| map | format | bytes per texel |
|---|---|---|
| base colour | BC7, sRGB (`VK_FORMAT_BC7_SRGB_BLOCK`), opaque | 1 |
| normal | BC5 (`VK_FORMAT_BC5_UNORM_BLOCK`), tangent x and y; z = √(1 − x² − y²) | 1 |
| roughness | BC4 (`VK_FORMAT_BC4_UNORM_BLOCK`) | ½ |

The KTX2 files are single 2D images with their mip levels, a basic data format descriptor and no supercompression. `atlas::ktx2` writes and reads them, and a test parses them with the `ktx2` crate. The manifest says `schema_version: 2` and `format: "detail-atlas"`, or `schema_version: 3` when any part bakes through `TEXCOORD_1`, so that an engine from before the second set refuses the atlas instead of sampling those parts through the wrong UVs. An atlas without such parts stays at 2, readable by both. It records `input_hash` (every part's input hash with the page side, padding and the atlas code), `padding`, `align`, `page` (`width`, `height`, `count`, `levels`), the format of each map, `total_bytes` (the KTX2 files), `vram_bytes` (the levels as uploaded), `decoded_vram_bytes` (the same as RGBA8, the CPU-decode path), `psnr` per map kind against the uncompressed pages (dB over each part's crop: base RGB, normal XY, roughness; `null` when lossless), the files with bytes and sha256, and per part `part` (the scene-order number), `node`, `size`, `crop` (x, y, width and height in tile texels), `page`, `at` (the crop's corner on the page), `scale` and `offset` (the UV transform), its share of `bytes`, its `input_hash` and `uv_set` (0 for `TEXCOORD_0`, 1 for `TEXCOORD_1`). A reader takes a missing `uv_set` as 0 and refuses a set above 1, or a schema 2 manifest with a part on set 1.

The encoder is [`block_compression`](https://crates.io/crates/block_compression) 0.10 (MIT). It is a pure-Rust CPU port of Intel's ISPC texture compressor, built with its default features off (no wgpu, so only `bytemuck`), from crates.io with nothing else fetched. BC7 uses its `opaque_slow` settings. The same input gives the same bytes. The bake splits a level into bands of block rows across threads, and a test checks that the split gives the same blocks as one call. Its decoder is the CPU path.

### Loading, and the first format

`atlas::read(folder)` checks the manifest's SHA-256, accepts schema 2 or 3, and checks each KTX2 page's byte count and SHA-256. `atlas::read_from` takes `|name| -> Option<Vec<u8>>` and runs those same checks on the bytes the closure returns for `manifest.json`, `manifest.sha256` and each `page-NNN-base.ktx2`, `page-NNN-normal.ktx2` and `page-NNN-roughness.ktx2`. A missing name is the error `missing detail file <name>`. `read` asks the closure for the folder's files. A changed byte fails the check. `atlas::load(folder)` returns an `Atlas` for either format. A folder from before this change (schema 1: `manifest.json` listing `part-NNNN/` folders, each with raw `base.rgba`, `roughness.rgba` and `normal.rgba` at the full tile) still loads. It checks every checksum, crops each map to its non-empty texels on 16-texel boundaries, and packs them into an uncompressed atlas that samples the same. `pfx bake --detail` writes only the new format. It has no flag for the old one, because every reader loads the new format.

The live renderer takes an atlas through `Renderer::set_detail(&atlas)` (`Frame::set_detail`, `maps::MapTextures::detail`). A material picks its part with `maps::select_baked_detail(&mut material, k)`, where `k` is the part's position in `atlas.parts`. The detail maps replace the frame's other material maps, as `set_maps` does. The live renderer uploads the BC pages when the device has `TEXTURE_COMPRESSION_BC`. Otherwise it decodes them to RGBA8 on the CPU (`Texture::decode`, which also fills a normal's z), and the floor stays where it was. `pfx_gpu::Gpu::headless` does not ask for `TEXTURE_COMPRESSION_BC` yet, so devices opened that way take the CPU path. `studio_desk` opens its own device with it.

`pfx bake --detail` prints a line per part with its size and crop, then `detail bake: N parts on P pages of W×H` and the summary `detail bake: N parts, largest S², P pages of W×H, B bytes on disk, V bytes in VRAM (D decoded), F bytes as full-tile maps; PSNR base …, normal …, roughness …`. "Full-tile maps" is what the first format would have taken, `12 × size²` per part.

A detail check renders a 4K frame five ways, each from a fresh history: full-tile uncompressed maps, the cropped uncompressed atlas, the BC atlas, the BC atlas decoded on the CPU, and full tiles again. It compares each frame with the first over the frame and over the baked parts' pixels, and fails if the crop moves a part pixel by more than 1/255 or the repeat differs at all. For six parts of a desk scene, with light bakes skipped: the crop is within 1/255 on all 625,582 part pixels (one pixel elsewhere in the 8.3 million moved by 2/255), and the repeat is identical. The BC frame is 52.8 dB on the parts, with 2.4% of part pixels more than 1/255 off, 258 more than 16/255 and 25 more than 32/255, the worst 76/255 on a thin specular edge. The CPU-decoded frame is 52.5 dB, worst 62/255. There the 4K opaque pass measured 3.574 ms with baked detail and 4.239 ms with procedural detail, each over 600 moving-camera frames with light bakes skipped. A device opened with `TEXTURE_COMPRESSION_BC` keeps the atlas compressed.

## Composed scenes

A recipe can assemble its scene from more than one file and place parts and lights itself. The keys below are all optional and leave every older recipe's hash alone.

- **More files.** `[[file]]` adds a glTF after `scene`, in order: `path`, and optionally `translation = [x, y, z]` with `keep_prefix = "floor_"`, which moves every root node of that file by the translation except those whose name starts with the prefix (a plant lifted onto a shelf while the petals on the floor stay). Materials are shared by name across files. `shadow_only_materials = ["occluder", …]` leaves the triangles of those glTF materials out of the bake and still resolves them; `shadow_only_nodes` does the same by node name. `drop_degenerate = true` drops triangles whose edge cross product is shorter than 1e-14, after the file's transform.
- **Posed parts.** `[[pose]]` names a `node` (and optionally its `file`, default `scene`) and gives `translation`, `yaw` in radians about +Y and a per-axis `scale` (the matrix is `[R·scale | translation]`, applied to the node's world positions). A node with poses is baked only at its poses, once per entry; a pose can name a file that is not part of the scene, which then contributes only its posed copies. A pose's `materials` table maps glTF material names to library names, so copies of one part can wear different materials; it needs `materials = "<library.toml>"`.
- **Shade.** `[sky.shade]` multiplies each anchor's sky: its keys are the anchors as text (`9` or `"9.5"`; a dotted key needs quotes) and its values are RGB multipliers, over the clock's own shade under `ambient = "clock"`. A key that is not an anchor is refused.
- **Emitters.** `[[emitter]]` gives either a `node` (its triangles take the emitter's material) or a `position` and `radius` (a 12 × 24 sphere, 528 triangles), a `color` (or `colour`), an `intensity` and optional per-anchor `scales` (default 1). A sphere radiates `intensity / (π radius²)` per colour channel, so `intensity` is the light's intensity as `LocalLight` states it; a node radiates `intensity` itself. The material is black, fully rough and takes its other fields from `preset` (a library name when the recipe names a library, else an engine preset), default the library's `fallback`. At each anchor the emitters are scaled by that anchor's scale, so a lamp baked at 0 leaves only the daylight. `emitter_layer = { direct = false }` also bakes the emitters alone under a black sky (`direct = false` keeps only their bounce) and writes it through `write_artifact_with_emitters`, so one artifact carries `emitters.bin` and the manifest's `emitters` stanza, and its scene hash is `emitter_hash`, with the scales and `direct`. A layer needs every emitter to share one `scales`, because the artifact stores one. Without `emitter_layer`, emitters are only baked in at their scales, and the scales count in the hash. `--anchors` cannot change the count of explicit `scales`.

## Plates

A plate is the static part of a scene traced once at the camera's rest pose, at each light anchor, so the live renderer can composite it and draw only what moves over it (`docs/plates.md`). `pfx bake` traces one when `bake.toml` holds a `[plates]` table; Manfred's `[bake.plates]` is written into the bake recipe as this table. The plate is traced from a `scene.toml` (`docs/scenes.md`), the same file the live renderer stages, so live and traced read one description: its static objects, static text, emitters, lights, sky and sun are traced, and every dynamic object and dynamic text is left out (with its shadow and its bounce).

```toml
[plates]
scene = "desk.scene.toml"
size = [3840, 2160]
overscan = 0.1
scale = 1.0
anchors = [12.0, 16.0, 19.0]
layers = ["color", "depth", "normal", "id", "sun", "transfer"]
threshold = 0.01
min_samples = 16
max_samples = 1024
key = "manf:3f9c…"

[[plates.view]]
name = "desk"
at = [0.0, 1.2, 2.4]
look_at = [0.0, 0.8, 0.0]
fov = 32.0
shift = [0.0, -0.1]
```

| key | value |
|---|---|
| `scene` | the `scene.toml` whose static subset is traced, relative to the recipe's folder (or inside the assets folder with `--assets` or `assets_env`, under the same rules: nothing escapes it). Required. |
| `size` | `[width, height]` in pixels: the frame the plate is drawn at live. Required, each 16 to 16,384. |
| `overscan` | `0` to `1`? `0.1`: the plate covers `1 + overscan` times the view's width and height (half of it on each side), for drift and small pushes. |
| `scale` | `0.25` to `4`? `1`: plate texels per live pixel, so a plate stays sharp up to a zoom of `scale` (`1.25` for a 1.25× push). |
| `anchors` | strictly increasing hours from 0 through 24? the recipe's `anchors`: the scene's `[sun] hour` is set to each in turn (the sky follows it as the scene says). `--anchors` overrides both. |
| `layers` | a subset of `"color"`, `"depth"`, `"normal"`, `"id"`, `"sun"` and `"transfer"`, in any order? all six. `color` and `depth` are required; `transfer` needs `sun` and `normal`. |
| `threshold`, `min_samples`, `max_samples`, `growth` | the tracer's adaptive sampling (`docs/trace.md`, Adaptive sampling)? `0.01`, `16`, `1024`, `2`. `max_samples` is at most 65,536; `--samples` sets it for one run. |
| `seed` | `u32`? the recipe's `seed`. |
| `key` | a string? none: the caller's own key for the static subset (Manfred's plate key). It is recorded and enters the engine's key. |
| `clamp_indirect`, `filter_glossy` | the scene's `[trace]` keys of the same names ([scenes.md](scenes.md#trace), [trace.md](trace.md#fireflies)), `0` to `10000` and `0` to `1`? the scene's own: either one given here replaces the scene's for every view's colour pass (`PlateRecipe::trace`); the sun and transfer passes have no bounce, so they never change them. Both enter the key with the rest of `[trace]`. |
| `[[plates.view]]` | the views to bake, each a `name` (an ASCII label, unique) and any of the scene `[camera]`'s keys `at`, `look_at`, `up`, `fov` (or `focal` and `sensor`, `sensor` 24), `shift`, `near` and `far`; a key a view leaves out is the scene camera's. Without a view, one view named `main` takes the scene's `[camera]`. A view is perspective; an orthographic camera is refused. |

`pfx bake` traces the plates after the probe volumes, props and reflections, in the same GPU job, into `<output>/plates/<view>/`, and prints `plate <view>: <W'>x<H'>, <bytes> bytes per anchor, <bytes> shared, baked` (or `unchanged`). The recipe's usual keys (`scene`, `anchors`, `samples`) stay required. `--anchors` replaces the plates' anchors too, and `--samples` sets their `max_samples`. The library call is `pfx_bake::plate::bake_plates(PlateRun { gpu, recipe, scene, anchors, seed, max_samples, out, force, between, log })`, with `parse_plates(bytes)` reading the table from the recipe.

The plate ignores the camera's `fstop` and `focus`: it is traced through a pinhole, and the live renderer applies depth of field to the composite. The finish is live's too: the plate holds linear scene radiance before exposure.

### The plate camera

The plate is `W' × H'` texels, `W' = 2·round(width · scale · (1 + overscan) / 2)` and `H'` likewise, so its centre lies on the view's centre. Its camera has the view's origin and axes, with the tangent extents widened by `W' / (width · scale)` and `H' / (height · scale)` and the shift divided by the same factors, so one texel spans `1 / scale` of a live pixel and the view's frame sits in the middle. Texel `(i, j)` (rows top to bottom) is traced around the ray `forward + right·(x + shift.x) + up·(−y + shift.y)`, `x = (i + ½)·2/W' − 1`, `y = (j + ½)·2/H' − 1`, where `right` and `up` carry the widened tangents; this is the tracer's own perspective ray (`docs/trace.md`). The manifest records that camera as it was traced.

### What is traced

- **colour**: the full trace of the static subset at the anchor, adaptively sampled (`Trace::sample_adaptive`) until clean: linear Rec. 709 scene radiance, the sky included where a ray leaves the scene.
- **sun**: the sun's direct share of that colour: the same subset and pixels traced with only the sun (black sky, no local lights, no emission) and no bounce, so it is the light a shadow of a moving object takes away. The composite subtracts it where a dynamic caster stands between the plate and the sun.
- **transfer**: the sun's direct light again, with nothing casting a shadow: what the sun would give each texel unshadowed. With it the composite relights the sun between anchors under live shadows (`docs/plates.md`) instead of cross-fading two anchors' shadows.
- **depth**, **normal** and **id**: one ray through each texel centre (`Trace::probe`), which do not change with the hour, so they are stored once per view: the depth along the view axis, the shading normal facing the camera, and the object's scene `id` (0 for text, emitters, the sky and an object whose id is above 65,535, which is refused). Each static draw is traced with its own copy of its material so the probe's material id names its object; the copies shade identically.

### Layers and files

`<output>/plates/<view>/` holds `manifest.json`, `manifest.sha256` and one raw file per layer: `depth.bin`, `normal.bin` and `id.bin` once, and `color-NNN.bin` and `sun-NNN.bin` for anchor `NNN` (in anchor order), and `transfer-NNN.bin` with the `transfer` layer. Each is `W' × H'` texels, rows top to bottom, little-endian, no header:

| layer | format | bytes | texel |
|---|---|---:|---|
| `color` | `rgb9e5` | 4 | the GPU's shared-exponent `Rgb9e5Ufloat` (Vulkan `E5B9G9R9_UFLOAT_PACK32`): red in bits 0–8, green 9–17, blue 18–26, exponent 27–31, bias 15. |
| `sun` | `rgb9e5` | 4 | as `color`. |
| `transfer` | `rgb9e5` | 4 | as `color`. |
| `depth` | `r16-inverse` | 2 | `0` where no surface was hit (the sky); else `d` in 1 to 65,535 with `1/z = d / 65535 / depth_near`, `z` the depth along the view axis and `depth_near` the manifest's (0.999 of the nearest depth the plate holds). Inverse depth is linear across a plane on screen, so the live composite interpolates it exactly; at `depth_near = 0.5` the step is 0.03 mm at 1 m and 0.3 mm at 3 m. |
| `depth` | `r32f` | 4 | instead of `r16-inverse` when the farthest surface is more than 64 times the nearest (`plate::R16_RANGE`), where 16 bits would lose millimetres at the back: `z` itself as an `f32`, `0` for the sky. |
| `normal` | `oct16` | 4 | the world-space unit normal, octahedrally encoded: `x` in the low 16 bits and `y` in the high, each a signed 16-bit `round(v · 32767)`; `0` for the sky. |
| `id` | `r16` | 2 | the object's scene `id`, `0` for none. |

Per anchor the plate takes 12 bytes a texel (`color`, `sun` and `transfer`), and 8 bytes a texel once for `depth`, `normal` and `id`: 120 MB per anchor and 80 MB shared for a 3840 × 2160 view at the default overscan (4224 × 2376 texels); without `transfer`, 80 MB per anchor. The live renderer uploads them as they are, `Rgb9e5Ufloat` arrays and `R16Uint` (or `R32Float`), `R32Uint` and `R16Uint` textures, so no device feature is needed. BC6H colour (1 byte a texel) is not written yet; it would take the three per-anchor layers to 30 MB per anchor but needs `TEXTURE_COMPRESSION_BC` or a CPU decode on devices without it.

`manifest.json` (`schema_version` 1, `format` `"plate"`) records `engine` (the engine version), `key` (below), `caller_key`, `view`, `size`, `overscan`, `scale`, `width` and `height` (`W'`, `H'`), `camera` (`origin`, `forward`, `right`, `up`, `shift`, `near`, `far`: the plate camera, `right` and `up` scaled by the widened tangents), `depth_near`, `bounds` (`min` and `max`, the world box of every surface point the plate holds), `anchors`, `suns` (per anchor the scene sun it was traced under: `direction`, `color`, `intensity`), `layers` (each layer's format), `sampling` (`threshold`, `min_samples`, `max_samples`, `growth`, `seed`), per anchor `samples` (`mean`, `max`, and each round's samples and running texels, for colour, sun and transfer), `ids` (each static object's `id` and name), and `files` (each file's `name`, `layer`, `anchor` (null for the shared layers), `bytes` and `sha256`). `manifest.sha256` is the SHA-256 of `manifest.json`.

### The key

`key` is the SHA-256 of the engine version, the plate code's version, the view's camera, `size`, `overscan`, `scale`, `anchors`, `layers`, the sampling and seed, `caller_key`, and the static subset alone: every static object by name (its mesh's hash, its world model at rest, the values of the materials it resolves to, `shadow`, `two_sided`, `clip`, `alpha_cutoff`, `id` and its content image's hash), every static text (its values and font hash), the emitters, lights, sun (without its hour), sky hash and `[trace]` settings (with the recipe's `clamp_indirect` and `filter_glossy` in place of the scene's). Dynamic objects and dynamic text do not enter it, so an edit to a moving part keeps the plate; an edit to a static part, or an object turning dynamic or static, changes the key. `pfx bake` skips a view whose manifest has the same key and whose files verify, and re-bakes a view whose key changed (or whose files fail to verify) without `--force`, since the key is the plate's identity; `--force` re-bakes an unchanged one. The replacement is written beside the old folder and swapped in once it verifies, and a folder at the path that is not a plate is never replaced.

`pfx_bake::plate::read_plate(folder)` checks the manifest's SHA-256, the schema and every file's byte count and SHA-256, and returns a `Plate`; `read_plate_from(&|name| Option<Vec<u8>>)` does the same from bytes, so an app can embed its plates with `include_bytes!`. A missing file is the error `missing plate file <name>`.
