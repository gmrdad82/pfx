# Scenes as data

A product's content is a scene: meshes, materials, lights, sky, haze, camera, finish, objects, movers, content, text, bodies and sounds, all data. Adding or changing one is a file edit; nothing compiles. `pfx_load::scene::Scene::open(path)` reads it, `pfx_live::renderer::Renderer::stage` uploads it, `pfx_trace::stage::scene` builds the tracer's stage from the same `Scene`, and `SceneWatch` reloads it into a running renderer as its files change. The engine stays headless: a viewer's window (Manfred's, a product's) calls these.

## The format is pfx-scene's

The files are format 1 of the public crate `pfx-scene`, which owns the format: every file kind, key, type, default and range, ids and references, includes and prefabs, paths, diagnostics and the format 0 to 1 migration. Its spec is [docs/format.md in pfx-scene](https://github.com/gmrdad82/pfx-scene/blob/v0.1.0/docs/format.md), at the tag `crates/load/Cargo.toml` pins (`v0.1.0`). This page keeps only what the engine does with a loaded scene.

## Loading

`Scene::open(path)` finds the project root (the nearest folder upward with a `project.toml`, else the scene's own folder), resolves the scene through `pfx_scene::Project::scene`, and converts the resolved types into the engine's:

- **Checking is pfx-scene's.** Unknown keys, types, ranges, names, ids, references, include and parent cycles, a table held twice, and each path's existence and case are refused there, before the engine reads an asset. A refusal is a `SceneError` with the file, line and column of the value, its message, and the other places it names in `related` (the first use of a duplicate, the object that makes a proxy's object dynamic).
- **Format 0 migrates in memory.** A file without `format` is read through pfx-scene's migration, and `Scene::warnings` carries one warning for it, at its file. So do the ids a hand-written format 1 entry leaves out (`pfx scene fix` writes them). `pfx scene migrate` rewrites the files on disk.
- **Asset contents are the engine's.** pfx-scene checks paths and their case only. pfx-load reads and checks what they hold: the glTF (and the node names in `node`, `nodes` and an object's `materials`, which must be in the mesh; a node tree with a cycle is refused, and a `.gltf` buffer `uri` must stay inside the glTF's folder: an absolute path or `..` is refused), PNG content images (at most 16,384 pixels a side, checked from the header before any pixel is decoded), WAV sounds and Ogg Vorbis sounds (an Ogg file's pages must be whole, its Vorbis header must name a rate and channels, and its last page's position must give a duration; `pfx-play` decodes it with `pfx-sound`'s `vorbis` feature, which refuses a clip of more than 8 channels or more than 57,600,000 samples, 10 minutes of 48 kHz stereo), fonts with a family name, Radiance HDR skies, a room sky's lamps, the finish through `pfx_post`, and a body's size from its mesh bounds. Its refusals carry the file, line and column of the entry that names the asset.
- **Keys.** The engine keys meshes, objects, lights, emitters, movers, content, text, bodies and sounds by the resolved scene's keys: an entry's name, and `"<placement>/<inner>"` for an entry a prefab placed. References arrive rewritten to those keys, so `Object::parent`, `Mover::objects` and the rest name keys. A prefab placement is an object with no mesh (`Object::mesh` empty): it draws nothing and is the parent of the prefab's root objects.
- **Paths** are relative to the project root. The engine's `Scene::files`, `SceneMesh::file`, `Content::image`, `Text::font` and `Sound::file` are the root joined with them, in the form the caller passed.
- `Object::id` is the 1-based pick id (`pick`, or the object's place among the resolved objects), and `Object::dynamic` is the resolved `dynamic`, derived where the file leaves it out.

## What the engine does with each table

- **Meshes.** A glTF material resolves to the library entry of its name, else of its name before the first `.`, else `fallback`, else the glTF's own material when it declares anything (a factor off the glTF default, a texture or a read extension; named `gltf <mesh>/<material>`, see `docs/materials.md`, Texture maps on imported meshes), else `Material::default()`. Missing normals are computed (smooth, area weighted), missing tangents are `[1, 0, 0, 1]`, or computed from the UVs where the part's material has anisotropy or its glTF material a normal, detail or anisotropy map, and missing UVs zero. With `node`, the node's own placement in the file is dropped; the object places it. A node whose `MSFT_lod` extension lists level nodes (`head.lod1`, `head.lod2`, …, each with `extras.lod.error` in metres) loads as one part with prebuilt levels of detail, and the level nodes are not drawn on their own; the live renderer picks a level by projected error ([live-renderer.md](live-renderer.md#prebuilt-levels-of-detail)). There is no scene key: the levels come with the file.
- **Mesh files.** A mesh `file` is a `.glb`, a `.gltf` (its buffers and images beside it) or a `.manf`, Manfred's container: its `GLTF` and `GBIN` chunks are read as a GLB would be, and buffer and image `uri`s resolve from its named `FILE` chunks first, then beside the `.manf`. A cut or damaged container is refused with its reason and the file. Scenes stored inside a `.manf` are not read yet. See [manf.md](manf.md).
- **Wet materials.** A material's `[wet]` table (in a materials file or a scene's own `[materials.<name>]`) soaks it in both renderers: `amount` (0 to 1, default 0) is the overall dampness, `puddles` (0 to 1, default 0) roughly the share of flat ground under standing water, `puddle_scale` (metres, above 0, default 2) the size of the puddle pattern and `seed` (default 0) which pattern. The table reads into `Material::wet` through the library, so scene watch and the editor change it live, and `--set materials.<name>.wet.amount=0.7` sets it from the CLI through the same edit. Without `[wet]`, or with `amount` and `puddles` both 0, the material renders exactly as before. The model, the puddle mask and its cost are in [Wet surfaces](materials.md#wet-surfaces).
- **Material lobes.** A material's `fabric = { color, roughness }` (sheen, colour 0 to 1 default `[0, 0, 0]` = off, roughness 0 to 1 default 0), `anisotropy = { strength, rotation }` (strength 0 to 1 default 0 = off, rotation in radians default 0) and `scatter = { color, radius, amount }` (colour 0 to 1 default `[1, 0.35, 0.2]`, radius in metres above 0 default 0.01, amount 0 to 1 default 0 = off) shade in both renderers; `docs/materials.md` has the lobes and their cost.
- **Sun.** An authored sun follows `hour`: its direction, colour and intensity come from `Daylight::authored(&Authored, reference_hour)` for that hour, day, latitude and heading, so at `hour = reference_hour` it is exactly as authored, and away from it the sun turns, lowers, warms and dims as the day does. Its colour keeps the authored colour's peak; its intensity is `irradiance` times the day's intensity at `hour`. `hour` also picks the light probes' hour. `radius` is the soft sun: live soft shadows (`Quality::sun_radius_deg`) and the tracer's sun disc (`Detail::sun_radius_deg`). Without `[sun]` the sun is dark.
- **Sky.** An analytic sky follows the sun. A mix sky is baked once, on load, into one HDR at the width of its widest HDR or room layer (`256` when every layer is analytic); an analytic layer is evaluated per texel with `AnalyticSky::diffuse`, the sky without its sun disc. The live renderer and the tracer both draw that HDR. `intensity` scales every kind's radiance, as seen and as the light it casts: an HDR or room sky's texels, and an analytic sky's dome (`AnalyticSky::ambient`; its sun disc stays the sun's). `backdrop` and the `horizon_` keys change only the sky as seen behind the scene ([`[sky]` backdrop and horizon](#sky-backdrop-and-horizon)). Without `[sky]` the sky is black.
- **Lights.** A `[[light]]` is a point light by default and a spot with `kind = "spot"`. Its keys, all optional but `name` and `intensity`:

  | Key | Default | Range | Meaning |
  |---|---|---|---|
  | `kind` | `"point"` | `"point"`, `"spot"` | the light's shape |
  | `position` | `[0, 0, 0]` | metres | where a free light stands (a parented light uses `at`) |
  | `color` | `[1, 1, 1]` | ≥ 0 | linear colour, times `intensity` |
  | `intensity` | | ≥ 0 | power: irradiance `intensity / d²` at `d` metres, before the window |
  | `radius` | `0` | ≥ 0 m | the bulb: it softens highlights and caps the falloff at `d = radius` |
  | `range` | `10` | > 0 m | the smooth window `(1 − (d / range)⁴)²` ends the light at `range`, live and traced |
  | `shadow` | `true` | | casts shadows; at most four lights do (below) |
  | `direction` | `[0, −1, 0]` | nonzero | where a spot points, in its parent's space when parented |
  | `cone` | `[30, 45]` | `0 ≤ inner ≤ outer ≤ 90`° | full light inside `inner`, none outside `outer`, glTF's `KHR_lights_punctual` smooth falloff between: `t = clamp((cos θ − cos outer) / (cos inner − cos outer))`, times `t²` |
  | `parent` | none | an object key or id | the object the light rides; it follows the object's movers, physics and animation every frame |
  | `node` | none | a glTF node name | a node inside the parent's mesh (`"hero.body"`, `"hero.wheel_rl"`): the light follows that node's animated transform |
  | `at` | `[0, 0, 0]` | metres | the light's place in the parent's (or the node's) local space, scale included |

  A refused light names itself: a `node` without a `parent`, a `parent` that is no object, a `node` not in the parent's mesh, a cone out of range or a zero direction. `Light::spot` (`Spot { direction, inner, outer }`, world space) and `Light::attach` (`Attach { object, joint, rest, at, direction }`) carry them; `Light::position` and `spot.direction` are already placed at the objects' rest. `Light::placed(&posed)` places it on posed objects, and `Light::placed_with(&posed, &palettes)` on posed objects and their animated nodes; `Staged::frame` does the second at its time (with the node palettes of the scene's playing `[object.animation]` clips, empty when none plays), `pfx_play`'s presenter the second with the world's animators, both through `pfx_live::scene::posed_lights`, and `pfx_trace::stage::scene_posed` the second for the tracer, so live and traced lights stand in the same place. Every key is live under a scene watch and settable with `--set light.<name>.<key>=<value>`. The live renderer takes any number up to `MAX_LIGHTS + MAX_CLUSTERED` (4 + 1,024): with four lights or fewer they all go through the shadowed path as before; with more, the first four lit lights with `shadow = true` keep their shadows and every other light is lit through the clustered list, unshadowed ([live-renderer.md](live-renderer.md#many-lights-clustered-forward)).
- **Emitters.** An emitter is a 12 × 24 sphere with a black, fully rough, emissive material named `emitter <name>` radiating `intensity / (π radius²)` per channel; it casts no shadow.
- **Camera.** `shift` is live's off-centre projection and the tracer's `Lens::shift`. `fstop` is depth of field through a thin lens of the camera's focal length: live `dof::Lens`, traced `Lens::aperture` (`focal / fstop`). `[camera.preset]` tunes `pfx_core::camera::Preset`. `keys` animates `fov`, `fstop` and `focus` over time ([Keys over time](#keys-over-time)); `focus_keys = [[t, metres], ...]` is another spelling of `keys.focus`. A keyed `fstop` without a static one gives the camera depth of field from its first key, and a keyed `fov` moves the lens's focal length with it when `focal` is not set. Every key is live under a scene watch and settable with `--set camera.<key>=<value>`.
- **Finish.** `file`, or the inline keys, build the chain as `pfx_post::parse` does: a style's passes first, then each key adds or tunes its pass at that parser's own place. `[[finish.pass]]` tables apply in the order written, each built from nothing, after the base. A pass that builds nothing is refused with its line. Without `[finish]` the renderer's standard finish. `lut = "<path.cube>"` grades through an Adobe or Resolve `.cube` file (see [`[finish] lut`](#finish-lut-a-cube-file)); `lut` as a table (`size`, `values`) is the inline cube.
- **Haze.** The live renderer marches it: a room haze in its `lo`/`hi` box (`Renderer::set_haze`), or, when `height` or `falloff` is set, an outdoor height fog over the whole view (`Renderer::set_height_fog`, [`[haze]` outdoor](#haze-outdoor) below). The tracer has no haze and draws the scene clear.
- **Movers.** At time `t`, `u = (t + offset) / period` and the mover stands at `from + (to − from) · f(u)`. A child's own mover applies first, then its parent's, up the chain. The time is the one the viewer passes to `Staged::frame` (and `pfx_trace::stage::scene_at`); at `0` every swing and once mover stands at `from`.
- **Content.** An object that names a content shows its image in its material's content layer slot; two objects showing different content through one slot are refused when staged.
- **Text.** The block is laid out with the font at `size` per em and centred on `at`; it reads along its local +X with +Y up and faces +Z. `Staged` draws it through the live text pass (`Staged::text`), and the tracer letters it on a carrier card, so both agree.
- **A prefab's text moves with its placement.** Its `at` and `rotate` are in the prefab's space: `Text::place` is the placement object's world matrix (the identity for a scene's own text), and `Text::model()` applies it, so the text moves, turns and scales with its group, and a prefab placed twice draws its text twice.
- **Bound text.** A `{key}` in `text` is filled when the scene loads, with the value bound to `key`: `pfx_load::scene::bind(key, value)` sets it for the process, `bound(key)` reads it and `filled(text)` fills a string. A key is letters, digits, `_`, `-` and `.`; a token with no value, and anything else in braces, stays as written. `pfx_game::Config::bind_text()` binds `name` and `version` from the game's config: the game's window, `Headless` and `<game> edit` call it before a scene loads, and `pfx edit <project>` binds the project's name and its `Cargo.toml`'s `[package] version` (`Project::version()`). The text is filled at load, so play, the editor's viewport, hot reload and the tracer show the same text. `pfx bake` runs with nothing bound, so a bound text in a plated scene is `dynamic = true`.

### Keys over time

`[[light]]`, `[camera]`, `[[particles]]`, `[finish]`, `[haze]`, `[sun]`, `[sky]` and each `[[sky.layer]]` take an inline `keys` table that animates some of their keys over the scene's time (pfx-scene's `docs/format.md`, "Keys over time"):

```toml
[[light]]
name = "lamp"
position = [0.0, 2.0, 0.0]
intensity = 2.0
keys = { intensity = [[0.0, 2.0], [1.5, 8.0]], color = [[0.0, [1.0, 0.2, 0.1]], [1.5, [1.0, 0.9, 0.8]]] }

[sun]
model = "daylight"
hour = 17.5
keys = { hour = [[0.0, 17.5], [10.0, 18.2]] }
```

Each entry is a keyed key and its `[seconds, value]` keys: seconds of scene time, at least 0 and rising; the value has the key's shape and range (a number, `[r, g, b]` for a colour, two numbers for `cone`, `speed` and `size`). Between keys each component is linear; before the first key the value holds the first, after the last the last (`pfx_scene::types::sample`). A keyed key overrides its static value. pfx-scene refuses a name a table does not key, a wrong shape, a value out of range and times that do not rise, each at its line.

| Table | Keyed keys | Where it lands |
|---|---|---|
| `[[light]]` | `intensity`, `color`, `range`, `cone` (spot) | the frame's lights (`Effects::lights`, the presenter's `set_lights`) |
| `[camera]` | `fov`, `fstop`, `focus` (`focus_keys`) | the frame's camera and lens (`Effects::lens`) |
| `[[particles]]` | `rate` (`rate_keys`), `speed`, `size`, `color`, `alpha` | the emitter's `Recipe::keys` |
| `[finish]` | `exposure`, `lut_amount` | the post chain's gain and its LUT's amount, without a new chain |
| `[haze]` | `amount`, `color` | `Renderer::set_haze` and `set_height_fog` |
| `[sun]` | `hour` | the frame's sun, the plates' and probes' hour, and an analytic sky |
| `[sky]` | `intensity`, `backdrop`, `horizon_color`, `horizon_width`, `horizon_amount` | the sky's environment, rebuilt when its keyed values change, and the sky pass's look |
| `[[sky.layer]]` | `intensity` | the mix, baked again when a layer's value changes |

The tracks stay on the loaded scene: `Scene::keys` is a `SceneKeys` with one entry per keyed table (the lights by name), and samples them at a time: `light(name, t)` (a `LightAt` that tunes a light), `camera(&camera, t)`, `sun(t)` (the sun at the keyed hour, built as at load), `haze(t)`, `finish(t)` (`FinishAt { gain, lut_amount }`, the keyed exposure over the static one), `look(t)` (`SkyLook`) and `environment(t)` with `sky_stamp(t)`, which changes only when the sky's keyed values do. `SceneDiff` flags a section whose keys changed, so scene watch and `--set` make keys live like any other key.

Both live paths sample them at the frame's time through `pfx_live::keyed::Keyer`: `Staged::frame` (and `frame_at`) keys the camera and the sun before posing, the lights after, and hands the rest to the renderer as `Effects::tuned` (a `Tuned`), which `render` applies before it draws (`Renderer::tune`); `pfx-play`'s presenter, which `pfx shot` renders through, samples the same keyer at the world's tick time and applies it to the renderer itself, so a shot waits for a rebuilt sky (`wait_sky`) before it draws the frame. The same scene through `pfx shot` and through `Staged::frame` at the same times draws the same bytes (`keyed_light_and_sun_hour_draw_the_same_frames_in_a_shot_and_staged`). A window shows a rebuilt sky when its worker is done, a frame or two later; call `renderer.tune(frame.effects.tuned)` and `wait_sky` before `render` to have it at once. A scene without `keys` builds no keyer and renders byte-identical to before.

**Cost.** Keyed lights, camera, sun, haze and finish add no GPU work: the frame takes other values. The finish's exposure is a uniform factor and its LUT amount a uniform, so no pipeline is built. A keyed sun hour over an analytic sky, or a keyed sky or layer `intensity`, rebuilds the sky's SH and GGX cube on its worker whenever the sampled value changes (about 31 ms for the analytic sky, see [the sky's settle](live-renderer.md#submission-late-timings-and-the-skys-settle)), and resets history as any new sky does; `pfx shot` waits for it each frame the value moves. A keyed hour over an HDR sky only sets the probes' hour.

### `[sky]` backdrop and horizon

These shape the sky the camera sees behind the scene, after any mix; the light the sky casts, its reflections and the haze's ambient are untouched. Every kind takes them, and they are live under scene watch and settable with `--set sky.<key>=<value>`.

| Key | Range | Default | Meaning |
|---|---|---|---|
| `backdrop` | > 0 | `1` | scales the seen sky (its sun disc too) |
| `horizon_color` | `[r, g, b]`, each ≥ 0 | none | the tint's linear colour; without it there is no tint |
| `horizon_width` | above 0, at most 90 degrees | `10` | the elevation over which the tint fades out |
| `horizon_amount` | 0 to 1 | `0` | the tint's strength at the horizon |

The seen sky is `mix(backdrop · sky, horizon_color, amount · (1 − smoothstep(0, width, |elevation|)))`, elevation in degrees from the horizon, so the tint is full on the horizon line and gone `width` degrees above it (and below it). The live sky pass (`sky_seen` in `pfx_core::sky::SKY_MODEL_WGSL`, set with `Renderer::set_sky_look`) and the tracer's camera rays that leave the scene draw it the same; a ray that has bounced sees the plain sky. `pfx_core::sky::seen` is the same formula on the CPU. With the defaults the pass returns the sky unchanged, bit for bit.

**Cost.** Measured once on this machine's AMD RDNA 4 (Linux, RADV) under `pgpu run --class bench` with `cargo run --release -p pfx-live --example sky_look_cost`: a 2160 × 3840 frame of an analytic sky filling the view, GPU medians over 120 frames after 30. The sky pass takes 0.154 ms plain, 0.154 ms with `backdrop` (one multiply) and 0.185 ms with the horizon tint (+0.031 ms: an `asin` and a `smoothstep` per sky pixel); the frame 1.897, 1.894 and 1.935 ms. The tracer adds the same few instructions to camera rays that leave the scene.

### `[haze]` outdoor

Setting `height` or `falloff` turns `[haze]` into an outdoor height fog that fills the whole view; `lo` and `hi` are then optional, and the room-haze keys keep their meaning in a room haze. `pfx_load::scene::Haze::outdoor` holds it as `OutdoorHaze`, and the live renderer marches it (`pfx_live::height_fog`, [live-renderer.md](live-renderer.md#outdoor-height-fog)). Every key is live under scene watch and settable with `--set haze.<key>=<value>`.

| Key | Range | Default | Meaning |
|---|---|---|---|
| `height` | any metres | `0` | The fog's base height: its density is `fog` there and falls off above it |
| `falloff` | ≥ 0, per metre | `1` | Height falloff: density is `fog · exp(−falloff · (y − height))`; `0` is uniform fog |
| `fog` | ≥ 0 | `amount / 6` when `amount` is set, else `0.1` | Density at `height`; extinction is `0.6 · density` per metre (absorption 0.3, scattering 0.3) |
| `reach` | ≥ 0 metres | `300` | How far the march goes: fog stops at the first surface or at `reach` |
| `shafts` | 0 to 1 | `1` | How much the sun's shadow cascades carve the in-scatter: `0` lights all fog, `1` full light shafts and shadowed volumes |
| `steps` | 1 to 256 | `64` | March steps per half-resolution pixel |
| `color` (alias `colour`) | `#rrggbb` or three numbers | white | Tints the light the fog scatters |
| `phase` | −0.9 to 0.9 | `0.5` | Henyey-Greenstein asymmetry: above 0 the fog glows toward the sun, so a low sun ahead blooms in the haze |
| `ambient` | three numbers ≥ 0 | the sky's mean radiance | The isotropic light the fog scatters besides the sun and the lights |

Toward a low sun, the glow is the phase lobe and the fog's density: `phase` 0.8 narrows it into a soft halo around the sun that bleeds over the buildings in front of it, `fog` about 0.05 keeps it large without hiding the street, `shafts` below 1 leaves some light in the shadowed street so the shafts stay gentle, and `color` tints it (a pink-white such as `#f0c8f0` against a warm, low daylight sun). A haze without `height` and `falloff` is the room haze, and a scene without `[haze]` renders byte-identical to one rendered before the outdoor mode existed.

### `[finish] lut`: a `.cube` file

```toml
[finish]
lut = "looks/lilac.cube"
lut_amount = 0.8
```

| Key | Range | Default | What it does |
|---|---|---|---|
| `lut` | a `.cube` path from the project root, or a `{ size, values }` table | none | adds the LUT pass before the tone curve |
| `lut_amount` | 0 to 1 | 1 | mixes the graded colour over the ungraded one; needs a `lut` |

The file is read through the scene's reader, so scene watch reloads the finish when the `.cube` changes, and its bytes are in the finish's hash. `pfx_post::cube::parse` reads `LUT_3D_SIZE` (2 to 64), `LUT_1D_SIZE` (2 to 65536, one curve per channel), `DOMAIN_MIN` and `DOMAIN_MAX` (the input is mapped from them to 0 to 1), Resolve's `LUT_1D_INPUT_RANGE` and `LUT_3D_INPUT_RANGE`, `TITLE`, `#` comments and other upper-case keywords before the data; red changes fastest. It refuses, with the line, a file over 16 MiB, one that is not UTF-8, no size or two sizes, a size out of range, a keyword after the data, an entry that is not three finite numbers, too few or too many entries, and a domain whose minimum is not below its maximum; nothing in a malformed file panics. The table form's `size` is 2 to 64. `pfx_post::cube::generate(size, map)` and `cube::format` write a `.cube` from a colour function, which is how the tests make their identity and lilac cubes.

### `[finish] bloom`

```toml
[finish]
bloom = { strength = 0.15, threshold = 0.9 }
```

`bloom = 0.15` sets the strength alone. The light above `threshold` (with the soft knee below) scatters into a glow around its source; `kind` picks how the glow is drawn:

| `kind` | The glow |
|---|---|
| `round` (the default) | a round, wide glow that falls off like a lens's: Bevy's progressive downsample and upsample (below). `strength` is the share of the light above the threshold that leaves its pixel for the glow (0 to 1); the total light stays the same within 2% |
| `box` | the frame at `1/down` size (box average), blurred by a separable Gaussian `radius` texels wide with `sigma` (weights `exp(−k² / sigma)`), added at `strength`. The glow stops at `radius × down` pixels on each axis, so a source far over the threshold, a low sun or a lamp, draws its square. The `crt` and `neon` styles use it for a screen's glow |
| `tent` | as `box`, with a tent downsample and one tap per texel |
| `rings` | four rings of 20 taps at `radii` (times `unit`) with `gains`, `clamp` capping the extract; a narrow, beaded glow |

| Key | Range | Default | What it does |
|---|---|---|---|
| `kind` | `round`, `box`, `tent`, `rings` | `round` | the glow, above; naming a kind resets the other keys to that kind's defaults before the ones written apply |
| `strength` | 0 and up (`round`: 0 to 1) | 0 | how much light goes into the glow; 0 turns the pass off |
| `threshold` | 0 and up | 1 | the brightest channel above it blooms; `round` at 0 is Bevy's energy-conserving natural bloom, every pixel scattering `strength` of its light |
| `knee`, `knee_width`, `soft` | 0 and up | 0 | a soft start below the threshold: from `knee` over `knee_width` the kept share rises as a square to `soft` |
| `radius`, `sigma`, `down`, `linear_taps` | | 4, 16, 2, true | `box` and `tent` only |
| `radii`, `gains`, `unit`, `clamp` | | 1, 2, 4, 8; 1, 0.5, 0.25, 0.125; 1; 4 | `rings` only |

A finish that names no `kind` blooms `round`. Before, it bloomed `box` with `radius` 4 and `sigma` 16 at half size, so such a scene now shows a wider, softer glow, its sources lose `strength` of their light over the threshold to it, and its `radius`, `sigma` and `down` are ignored; `kind = "box"` brings the old glow back. A finish that names a kind, and a style's own bloom (`crt`, `neon`), keep theirs and render as before.

The round kind follows Bevy's bloom (tag `v0.16.1`; NOTICE.md): a 13-tap downsample with Karis firefly weights on the first level, so a lone bright pixel does not bloom, then halving levels down to 8 levels below one 512 texels tall, then a 3x3 tent upsample that blends each level into the one above by Bevy's `compute_blend_factor` with its natural constants. The glow's size is a share of the frame's height, so a 1080x1920 draft and a 2160x3840 final glow alike. Its cost is in [live-renderer.md](live-renderer.md#round-bloom).

### `[flare]`

```toml
[flare]
amount = 0.15
threshold = 6.0
```

Lens flare and sun glare from the frame's brightest parts: ghosts mirrored through the centre, a halo ring and a starburst, as Filament draws them. Ghosts fall off with their source's distance from the centre of the frame, so a light near the middle ghosts and lit windows toward the edges do not. Without `[flare]` nothing changes; with it the flare pass goes into the finish after its bloom (before the LUT and tone curve when there is no bloom), and a scene with `[flare]` but no `[finish]` runs it on the standard finish (bloom, flare, ACES). Every key is optional:

| Key | Range | Default | What it does |
|---|---|---|---|
| `amount` | 0 and up | 0.1 | strength of the composite; 0 turns the pass off |
| `ghosts` | 0 to 16 | 4 | mirrored ghosts; 0 drops them |
| `ghost_spacing` | 0 and up, under 1 in use | 0.6 | spacing of the ghosts toward the centre, in screen units |
| `halo` | 0 and up | 0.1 | halo ring thickness, in screen heights; 0 drops it |
| `halo_radius` | 0 and up, 0 to 0.5 in use | 0.4 | halo ring radius, in screen heights |
| `starburst` | 0 to 1 | 1 | how much of the starburst mask the flare takes |
| `threshold` | 0 and up | 10 | where ghosts and halo start, in HDR above the bloom input's 1: a source makes none at or below it, faint ones just over it (rising as the square of how far over it is) and full ones from 5 × `threshold` up |

Pick `threshold` above the scene's ordinary emitters. In a dusk street with lit windows, tail lights and lamps at emission 3 to 6, a threshold of 1.5 still lets the windows make faint amber ghosts, 3 leaves fainter ones, and the default 10 leaves ghosts only to the sun and sources far brighter than the street's lights. `--set flare.amount=0.2` and the editor edit it through the scene edit path (`Target::Flare`). The renderer side and its cost are in [live-renderer.md](live-renderer.md#lens-flare-and-cube-luts).

### `[trace]`

Settings only the tracer reads; the live renderer ignores them. `Scene::trace` holds them as `TraceSettings`:

- `transmissive_shadows` lets shadow rays toward the sun, the sky and local lights pass glass and liquid surfaces, tinted by live's `cast_tint`, so a traced glass shadow matches live's instead of Cycles' opaque one ([trace.md](trace.md#transmissive-shadows)).
- `clamp_indirect` (off at `0`) caps the luminance of each light contribution reached after the first bounce, as Cycles' Clamp Indirect; a brighter one is scaled down to it, keeping its hue. Direct light at the first hit is never clamped ([trace.md](trace.md#fireflies)).
- `filter_glossy` (off at `0`) evaluates glossy, clearcoat and glass lobes at least this rough after a diffuse bounce, as Cycles' Filter Glossy; the first hit keeps its own roughness ([trace.md](trace.md#fireflies)). `0.25` with `clamp_indirect` as a backstop is a good start for a small sun.

pfx-scene refuses a value out of range; `TraceSettings::check` holds the same ranges for a caller that builds the settings itself. `SceneDiff::trace` flags an edit; a `SceneWatch` reload changes nothing in a live renderer for it.

### `[plates]`

The baked plates of the scene's static part (`docs/plates.md`, baked by `pfx bake` with `[plates]` in its recipe, `docs/bake.md`). `Renderer::stage` reads the plate when the scene has `[plates]`: static objects, static text and the emitters are no longer drawn live, the plate is composited in their place, and only the dynamic objects and text are drawn over it. The plate folder's `manifest.json` is read and watched, so a re-bake reloads it; a folder not baked yet is no error. `casters` says what shadows the dynamic objects in the static objects' place: each static object's own mesh as a shadow-only caster, the proxies, or nothing.

`Scene::dynamic(index)` and `Scene::dynamic_names()` say what is dynamic, `Scene::static_subset()` is the scene with every dynamic object hidden and dynamic text dropped (what a plate traces), and `Scene::open_at(path, hour)` opens a scene with its `[sun] hour` set to `hour` (the bake's anchors).

Proxies are never drawn. With `casters = "proxies"` they shadow the dynamic objects; `pfx_load::scene::Proxies::pick(origin, direction)` returns the nearest proxy a ray meets (its object's pick id and distance) for picks on the CPU, and `Renderer::pick` reads the plate's own ids on the GPU where nothing dynamic covers them.

### `[motion_blur]`

Per-object motion blur in the live renderer, from its velocity buffer ([live-renderer.md](live-renderer.md#motion-blur)); the tracer ignores it. Without the table there is no blur and the frame is the same bytes as before the key existed.

| Key | Range | Default | Meaning |
|---|---|---|---|
| `shutter` | 0 to 1 | `0.5` | The share of the frame the shutter is open (0.5 is a 180° shutter). `0` turns the blur off. |
| `max_px` | above 0 | `64` | The longest streak, in pixels either side of a pixel; the pass clamps it to 1..256. It also sets the tile size, the next multiple of 8 at or above it. |
| `samples` | 1 and up | `8` | Taps per pixel along the motion; the pass clamps it to 2..64 and takes half on each side. |

`Scene::motion_blur` holds it as `pfx_post::MotionBlur`. `Renderer::stage` hands it to `Renderer::set_motion_blur`; `SceneDiff::motion_blur` flags an edit and `Staged::apply` sets it again (`Applied::motion_blur`), so a scene watch turns it on, off or retunes it in the window. On the CLI, `--set motion_blur.shutter=0.25` edits it through the same path (`Target::MotionBlur`, or `Target::Scene` with the path `motion_blur.shutter`).

The blur reads the velocity the frame wrote: moving and posed instances (`previous_model`), skinned meshes (their previous joints), morphed meshes (their previous weights), swaying trees and deformers, and the camera's own motion (`Camera::previous_view_projection`, carried to the sky and props by the background motion pass). While the scene's blur is on, `Staged` gives the renderer the camera of the frame it built last as the previous camera, so a camera moved by edits between frames streaks the frame; call `Staged::cut()` at a cut so the first frame after it does not streak. With the blur off the previous camera stays the current one, as before.

### `[physics]`, bodies and sounds

Play mode's rigid world (`pfx-play`, [play.md](play.md)); the live renderer and the tracer ignore them. A body is an object's `[object.body]`; `Scene::bodies` keys it by the object. Its shape's size and offset default to the object's mesh bounds times its scale: a box's `half` is half the bounds, a sphere's `radius` the largest of those halves, a capsule's or cylinder's `radius` the larger of the X and Z halves, and `half_height` the Y half (less the radius for a capsule). A body with no size this way is refused. A body's `layer`, an object's `[object.trigger]` (shape, `offset`, `layer`, `mask`, sized as a body is) and the project's `[layers]` come through as `Scene::body_layers`, `Scene::triggers` and `Scene::layers` ([play.md](play.md#queries-and-triggers)); pfx-scene refuses a layer name that names none. An object's `[object.character]` comes through as `Scene::characters`, with pfx-scene's defaults and its `radius` (the larger of the X and Z halves) and `height` (the Y size) from the mesh bounds times the scale; one with no size or a height under twice its radius is refused ([characters.md](characters.md)). `[sound.<name>]` plays as the format says (`World::play_sound`).

### `[[tiles]]`: tile layers

A tile layer is pfx-scene's grid of prefab placements in a plane (`[[tiles]]`: `cell`, `origin`, `plane`, `palette`, and `rows` or sparse `cells`). pfx-scene checks it and hands the loader `Scene::tiles`, each palette value rewritten to its prefab's root-relative path; `Tiles::cells` lists the filled cells and `Tiles::position` gives a cell's place. pfx-load (`pfx_load::scene::tiles`) places them after the scene's own objects, cheaply:

- **Each palette prefab is resolved and built once,** with `Project::scene` on its path, and only when a cell uses it. Its meshes join the scene keyed `"<prefab path>:<mesh>"`, so ten thousand cells of one prefab share one mesh entry, and the renderer uploads each geometry once and draws its cells as instances of it.
- **Each filled cell becomes the prefab's objects,** keyed `"<layer>[<i>,<j>]/<inner>"` (`tiles::cell_key`, read back by `tiles::cell_of`), each with its model the cell's translation times the prefab's own: a root object's `at` is the cell's position plus its own, an inner object keeps its parent, renamed into the cell. Their pick ids follow the scene's own objects. A prefab's bodies, characters, triggers, animations and body layers are placed per cell the same way.
- **What a tile may hold.** Objects, bodies, characters, triggers and glTF-clip animations (`Scene::animations`, per cell). A tile prefab with lights, emitters, movers, text, content, sounds or a sprite (which would take a content slot per cell) is refused at its palette key, and so is a material of the prefab's that differs from the scene's material of that name (an equal one is shared). A cell whose object key is already an object's name is refused at the layer.
- `Scene::files` lists the tile prefabs and what they read, so `SceneWatch` reloads on a change to any of them. Painting adds and removes only the painted cells' objects (`objects.added` and `removed`), except a paint that grows a `rows` layer down or left: that moves `origin`, renumbers every cell of the layer and so restages its objects.
- **Cost.** A 100 × 100 layer of one prefab (10,000 objects) loads in about 50 ms in a debug build; `docs/editor.md` has how it opens and draws in the editor.

`tiles::cell_at` finds the cell nearest a point in the layer's plane, `tiles::corners` the square a cell covers, and `tiles::painted` applies a brush's changes to a layer: a `rows` layer grows up and right by adding rows and characters, and down or left by adding them and moving `origin` back by whole cells, so no filled cell moves in the world; a `cells` layer keeps its order and appends new cells. The editor writes the result through `SceneEdit` (below).

### `[object.animation]`

An object's glTF clips or a sprite of an image atlas, played in play mode and drawn live ([animation.md](animation.md)). pfx-load reads it into `Scene::animations`, keyed by the object, and checks the glTF clip names and the sprite frames against the files. An animated object is dynamic, so a plate never bakes it. A sprite's atlas becomes the object's content (`sprite:<object>`), shown through its material's content layer at its first frame until play steps it.

### `[[particles]]`: 3D emitters

A deterministic emitter simulated on a 1/120 s grid and drawn by the live renderer's `sprays` pass ([particles.md](particles.md)). pfx-load reads each into `Scene::particles` (`pfx_load::scene::Particles`, keyed by `name`), and `SceneDiff::particles` flags an edit; a scene watch reload restarts the emitters. Every key but `name` is optional; without `[[particles]]` nothing is drawn and the frame is byte-identical. The tracer draws no particles.

| Key | Meaning | Range | Default |
|---|---|---|---|
| `name` | the emitter's key | text | required |
| `parent` | the object it rides; refused when the scene has no such object | object key | none, world space |
| `node` | a glTF node inside the parent's mesh (`hero.wheel_rl`); its animated motion in play mode, its file placement otherwise; refused when the mesh has no such node, or without `parent` | node name | none |
| `at` | the spawn point, in the node's (else the parent's) local space | metres | `[0, 0, 0]` |
| `rate` | particles per second | ≥ 0 | `0` |
| `rate_keys` | `[[t, rate], ...]` over scene time `t` in seconds; another spelling of `keys.rate` | t in order, rate ≥ 0 | none |
| `keys` | `rate`, `speed`, `size`, `color` and `alpha` over scene time ([Keys over time](#keys-over-time)): a keyed `speed` is the spawn speed at each particle's birth, a keyed `size` and `color` are the whole system's at the frame's time (`color` replaces `color` and `colors`), and a keyed `alpha` (0 to 1) scales the `alpha` curve | | none |
| `burst` | particles spawned at once on the first step | ≥ 0 | `0` |
| `max` | the pool cap; spawns past it are dropped | ≥ 0 | `1024` |
| `lifetime` | `[min, max]` life, uniform | seconds, 0 ≤ min ≤ max, max > 0 | `[1, 1]` |
| `speed` | `[min, max]` spawn speed, uniform | m/s, 0 ≤ min ≤ max | `[1, 1]` |
| `direction` | the cone's axis, turned with the parent | vector | `[0, 1, 0]` |
| `cone` | the cone's half-angle; `180` is every direction | 0 to 180 degrees | `0` |
| `inherit` | share of the spawn point's own velocity added | 0 to 1 | `0` |
| `gravity` | acceleration | m/s² | `[0, 0, 0]` |
| `drag` | `v *= exp(−drag · dt)` | ≥ 0, 1/s | `0` |
| `size` | `[start, end]` across, linear over life | metres, ≥ 0 | `[0.1, 0.1]` |
| `alpha` | `[[t, a], ...]` over life `t` | both 0 to 1 | `[[0, 1], [1, 0]]` |
| `color` (`colour`) | one colour, `"#rrggbb"` (sRGB) or `[r, g, b]` linear | | white |
| `colors` (`colours`) | `[[t, "#rrggbb"], ...]` over life; replaces `color` | t 0 to 1 | none |
| `spin` | `[min, max]` spin, either way | rad/s | `[0, 0]` |
| `stretch` | velocity stretch: the quad lengthens by `stretch · speed` metres along its motion on screen, `speed` its velocity against the camera's; 0.01 to 0.05 streaks it by one to three frames at 60 fps | ≥ 0 seconds | `0` |
| `blend` | `"alpha"` covers what is behind, `"add"` adds light | | `"alpha"` |
| `lit` | lit as a scattering puff: thin parts glow toward a light behind them by the haze's phase, dense parts shade diffusely; by the sun and its shadows (lit in shade as much as the haze is), the probes or sky and local lights; unlit emits its colour | | `true` |
| `seed` | the emitter's random stream | u64 | a hash of `name` |

From the CLI and the editor these are keys of `Target::Particles(name)`, so `--set particles.<name>.<key>=<value>` (`--set particles.spray.rate=800`) goes through the same `SceneEdit` path as a hand edit and reaches the window through the scene watch.

## Reload

`SceneWatch::open(path)` loads the scene and watches the folder of every file it read: the scene, its includes, libraries, glTF files and their buffers, the HDR, the finish file, content images and fonts. It decides by content: an event leads to rehashing every file (SHA-256), and only a changed hash re-reads the scene. Events are debounced (100 ms by default, `set_debounce`), so an editor's write-then-rename lands as one reload.

On a change the scene is re-read whole and compared with the last good one, by name:

- **meshes** by the hash of their glTF bytes, buffers and node overrides;
- **materials** by value;
- **objects**, **lights**, **emitters**, **text** by value; **content** by its image's hash;
- **movers** by value;
- **bodies** and **sounds** by value (a sound by its file's hash), and **physics** by value; none of them touches a live renderer.
- **sun**, **sky**, **haze**, **camera** and **finish** by value.

`Staged::apply` then touches only what changed:

- a changed mesh's parts are swapped into the handles they had with `replace_mesh`; parts nothing uses any more are released with `release_mesh`, and new ones uploaded;
- materials are patched in the per-frame list;
- lights go through `set_lights`, the finish through `set_finish` (or back to the standard one), the haze through `set_haze`, the lens through `set_lens`, the sun's radius through `set_shadow_quality`;
- the sky goes through `request_sky`: it is prepared off the frame path and shown when it settles, so a sky edit never stalls a frame (`Staged::stage` still waits for the first sky with `set_sky`);
- a changed text is laid out again and its atlas rewritten;
- added or removed objects add or drop instances; a changed content image is rewritten into its slot.

An empty diff touches nothing on the device. A broken file keeps the last good scene drawing: `poll` returns the error with its file and line once, and the next good save recovers (`Reload::recovered`). Nothing restarts.

### Waking a viewer

`SceneWatch::open_with(path, wake)` takes a `wake: impl Fn() + Send + 'static` that is called from the watcher's own thread once a burst of file events has been quiet for the debounce, so a redraw-on-demand window (winit's `EventLoopProxy::send_event`, say) repaints on a file change without polling. `watch.pending()` returns the `Instant` a seen change falls due, or `None`, for a viewer that would rather set a timer. Either way the viewer then calls `poll()` on its own thread.

## What a viewer calls each frame

```rust
let proxy = event_loop.create_proxy();
let mut watch = SceneWatch::open_with("content/room.scene.toml", move || {
    let _ = proxy.send_event(Wake);
})?;
let mut staged = renderer.stage(watch.scene())?;
loop {
    match watch.poll() {
        Some(Ok(reload)) => staged.apply(&mut renderer, &reload.scene, &reload.diff)?,
        Some(Err(error)) => show(error),
        None => {}
    }
    staged.set_override("door", Override { highlight: Some([0.2, 0.5, 1.0]), ..Override::default() });
    let finish = staged.finish();
    let frame = staged.frame(aspect, time, seed);
    renderer.render(&frame.scene, &frame.text, &frame.effects, finish, &output)?;
}
```

`watch.poll()` never blocks and does no work until a watched file changed. `staged.frame(aspect, time, seed)` poses the movers and cards for `time` (their previous pose is the pose of the last frame built) and returns a `StagedFrame`: `scene`, the staged instances and materials as the renderer's `Scene`; `text`, the scene's `[text.*]` for the live text pass; and `effects`, its glass. The parts are there apart too: `staged.pose(time)`, then `staged.scene(aspect, time, seed)`, `staged.text(aspect)` and `staged.effects()`, which borrow the staging immutably. `staged.frame_at(aspect, time, interval, seed)` poses them for `time` and their previous pose for `time - interval`, so a frame rendered alone has the motion of its place in the film (`interval = 1 / fps` for frame-to-frame velocity; the motion blur then scales it by `shutter`). A caller that poses the previous frame at `time - shutter / fps` instead sets `Renderer::set_velocity_spans_shutter(true)`, and the blur takes the velocity as it is. `apply` returns an `Applied` that says what it touched on the device; `Applied::sky_request` is the `request_sky` ticket, and `renderer.wait_sky()` waits for it when a viewer wants the new sky at once.

- **Glass.** A draw whose material is of the glass family (`family = "glass"`) or lets light through (`transmission` above 0, any family: a tinted lens is a plain material with `transmission`, see [materials.md](materials.md#see-through-and-tinted-surfaces)) is uploaded with `upload_glass` and drawn as a `glass::Surface` through `Effects::glass`, not as an opaque instance; it keeps its shadow, two_sided, clip and pick id.
- **Overrides.** `staged.set_override(object, Override { hidden, color, highlight, transform })` hides, recolours (`color` replaces the base colour), highlights (`highlight` adds that emission) or moves (`transform`) one object's draws, its glass among them, without editing the scene; its children keep their own looks. It returns `false` for a name the staging has not; `clear_override(object)` and `clear_overrides()` undo. They live in `Staged`, survive `apply`, and touch no data and nothing on the device.
  - `transform` is a world-space matrix applied on the left of each of the object's instance models (and their previous models, so a still preview has no motion), which is how an editor previews a drag without restaging: for a move from pose A to pose B it is `world_B × world_A⁻¹`, set on the object and on each of its descendants, and the frame equals a restaged copy of the moved scene (`crates/live/tests/override_transform.rs` checks the pixels match). It moves meshes and glass only: lights, emitters and `[text.*]` stay where the scene puts them, and a plated static object stays in its plate. Unset (`None`, the default) it changes no instance and no pixel.
- **Lens.** `stage` and `apply` set the depth of field for the renderer's own aspect; after a resize the viewer calls `renderer.set_lens(staged.lens(aspect))`.

For the tracer, `pfx_trace::stage::scene(&scene, aspect)` builds the same instances, materials, lights, sky, sun, lens and camera into a `Staged` ready to `trace`, with the movers at rest and the cards facing the camera; `scene_at(&scene, aspect, time)` poses the movers at `time`, as `Staged::frame` does live. Text is lettered on carrier cards with `pfx_trace::text`.

## Editing

`pfx_load::scene::SceneEdit` is the data layer of every scene editor (pfx's, Manfred's). It edits a format 1 scene's own files on pfx-scene's patch model: its `Target`, `Value`, `Kind`, `Patch`, `PatchGroup` and `EditError` are pfx-scene's, every edit lands through `pfx_scene::SceneEdit::apply`, which checks the edited texts as pfx-scene's `check` does before it writes, and then pfx-load loads the result with the engine's asset checks. A running app's `SceneWatch` hot-reloads the edit like a hand edit, and every editor writes the same bytes.

```rust
let mut edit = SceneEdit::open("content/room.scene.toml")?;
edit.set_at("crate", [-0.9, 0.35, 1.5])?;
edit.set_material("clay", &["roughness"], 0.5)?;
edit.set(&Target::Light("warm".into()), &["intensity"], 9.5)?;
edit.add(Kind::Object, Id::from_bits(random), "ball", &[("mesh", "block".into())], None)?;
edit.undo()?;
```

- **Cycles and depth.** A project whose includes or prefabs form a cycle, or nest deeper than `pfx_scene::MAX_DEPTH` (64), is refused when it is opened, by pfx-scene's own walk (`include-cycle`, `prefab-cycle`, `include-depth`, `prefab-depth`). The finding's message names the chain (`a.scene.toml -> b.scene.toml -> a.scene.toml`) and its `related` places hold each link, which `SceneError` prints as `see here` lines; pfx maps an `EditError` through its `diagnostics`, so `pfx scene fix`, `SceneEdit::open`, pfx edit's log and pfx new all show the chain's files.
- **Text is kept.** Files are edited as TOML documents (`toml_edit`): every comment, blank line, key order and number spelling the author wrote stays, and an edit changes only the value it sets. Of an array, only the components that differ are replaced. A new key goes after the entry's existing ones, a new entry's keys in the format's order, and a new table after the last of its kind. A value that is already equal writes nothing and is no undo step. A file that can't be written back byte for byte (CRLF line ends), or isn't format 1, is refused when it is opened.
- **Ids.** `add`, `add_material`, `add_mesh` and `duplicate_object` take the new entry's `Id`: an editor passes its own random bits (`Id::from_bits`), and a build derives one from a key it owns (`Id::derive`). The first write of a file fills the ids its entries lack, as pfx-scene's writer does.
- **The file that defines the item.** An entry is edited in the scene file or include that holds it, a material in its library; `[sun]`, `[sky]`, `[haze]`, `[camera]`, `[finish]`, `[trace]`, `[plates]` and `[physics]` where they are, and in the root scene file when no file has them yet. An entry a prefab placed is edited through its placement's `[object.set]`.
- **What it edits.** `set(&Target, path, value)`, `unset`, `push` and `remove(&Target)` take any key: a target and a path of keys into it, where a number picks an entry of a list (`["layers", "1", "frequency"]`); a missing table on the way is created. Typed helpers cover an object (`set_at`, `set_rotate`, `set_scale`, `set_object_material`, `set_object_materials`, `set_shadow`, `set_two_sided`, `set_hidden`, `set_clip`, `set_parent`), and `add`, `add_material`, `add_mesh`, `set_node`, `duplicate_object` and `rename_object` cover the rest.
- **Checked before it is written.** Every edit is first made as a pfx-scene dry run, which checks the group as a whole; then the engine's asset checks run on `Project::scene_with(scene, group.after())`, the scene with the edited texts in memory (an override naming a node the glTF lacks is refused there). Only a group both accept is written, through `pfx_scene::SceneEdit::apply`. A refused edit writes nothing and returns an `EditError { file, key, line, code, message }` and records nothing. An edit also refuses when a file changed on disk since it was read (`reload()` takes the disk's text and drops the history); files are written by a temporary file and a rename.
- **Undo and redo.** Every edit returns a `PatchGroup`: a label (`set object crate at`) and one `Patch` per file it touched, each with the file, its exact text before and its exact text after. `undo()` and `redo()` apply the inverse of those values, so the bytes come back exactly; a new edit clears the redo list. `begin(label)` … `end()` makes everything between one undo step (a gizmo drag); `cancel()` takes it back. Nothing is pushed for a group whose net text is unchanged.
- **For an editor with its own history.** `Patch` and `PatchGroup` are plain values with `label()`, `file()` / `files()` and `inverse()`; `SceneEdit::apply(&PatchGroup)` applies one without recording it (refused when a file is not at the text the patch starts from).
- **Dry runs.** `edit.dry_run(|edit| { edit.set_at("crate", at)?; edit.set_hidden("lid", true) })` runs any edits (set, unset, push, remove, add, rename, duplicate and the rest) on the scene's texts in memory and returns the one `PatchGroup` they would make: the disk's text before, the last text after, per file. It writes no file, touches neither the undo nor the redo list, and leaves `scene()` as it was, so the editor crates' viewport can land the group in its own in-memory files and make its save the only write. The group is checked as a whole, several files included, by pfx-scene and by the engine's asset checks, and refused as an edit would be. `apply(&group)` lands it later with exactly the bytes the direct edits write. A dry run is refused inside `begin` … `end`, and a dry run takes no group of its own.
- **Tile layers.** `set(&Target::Tiles(name), …)` edits a layer like any entry: `["rows", "<n>"]` is one row's whole new string, `["rows"]` or `["cells"]` the whole list, `["palette", "<key>"]` a palette entry and `["origin"]` the origin; `Kind::Tiles` adds a layer. The edited scene is reloaded with its cells placed again.
- **With a watch.** After an edit, `SceneWatch::check` (or `poll`) reads exactly the changed files and its `SceneDiff` names only what was edited: an `at` change is that object in `objects.changed`, a material value is its name in `materials.changed`, a light or the camera only theirs, an added or removed object is in `objects.added` or `removed`; no mesh is read again.

An edit that needs the viewer to show it at once calls `SceneWatch::poll` (or `check`) on the next frame as in "What a viewer calls each frame"; `SceneEdit::scene()` is already the edited scene, for a viewer that applies the change itself, and `SceneEdit::resolved()` is pfx-scene's resolved scene.

## Commands

- `pfx scene migrate <scene.toml> [--dry-run]` rewrites the format 0 files the scene reads (the scene, its includes, libraries and proxies file) as format 1 through pfx-scene's migration, keeping comments, spacing and order: `format = 1` on the first line, paths relative to the project root, an object's `id` renamed `pick`, each `[body.<object>]` moved into its object, and an id in every entry. It refuses to write when the scene or a migrated file doesn't check, and names each file it rewrote.
- `pfx scene fix <scene.toml> [--dry-run]` writes the ids that the entries of the scene's format 1 files lack, derived as the migration derives them, and refuses a format 0 file and a scene that doesn't check.
- `--dry-run` writes nothing and names each file it would rewrite. `pfx_load::scene::migrate(path, dry_run)` and `fix(path, dry_run)` are the same as functions, returning the `PatchGroup` (each file's text before and after); `fix` is `pfx_scene::Project::fix`, and both write through `PatchGroup::write` (a temporary file and a rename, refused when a file changed, put back on failure).
