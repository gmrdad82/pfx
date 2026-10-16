# Material libraries

`pfx_materials::Material` is the one physically based description the live renderer, the tracer and the bake share. `Material::to_toml` and `Material::from_toml` write and read one material as TOML: every field has `Material::default()`'s value when left out, unknown keys are refused, and `layers` is a list of at most four `{ kind, frequency, amplitude, seed, params }` tables (`params` left out is twelve zeros; see Noise layers below).

A product ships its materials as a **library**: one TOML file of `[materials.<name>]` tables, each in that same material TOML. `Library::from_toml(text)` parses it and `Library::to_toml()` writes it back, sorted by name, so a library round-trips to the same text and every material to the same bits (`PackedMaterial` included). A product loads its file with `include_str!` or from disk and looks materials up by name:

```toml
[materials.glass]
family = "glass"
base = [1.0, 1.0, 1.0]
roughness = 0.02
transmission = 1.0
ior = 1.5
thickness = 0.01

[materials."set/blue"]
base = [0.1, 0.2, 0.9]

[[materials.wood.layers]]
kind = "fbm"
frequency = 8.0
amplitude = 0.3
seed = 1
```

```rust
let library = Library::from_toml(include_str!("../render/materials.toml"))?;
let glass = library.get("glass").copied().unwrap_or_default();
```

- `get(name)` is an exact lookup; `contains`, `names`, `iter`, `len` and `is_empty` read the rest, in name order. `insert(name, material)` adds or replaces one, as a product does when it generates its file from code.
- A name is any non-empty text without control characters or spaces at its ends; a name with `/` or `.` is quoted, as above.
- `from_toml` refuses an unknown top-level key, an unknown material field or family, a non-table entry, a duplicate name and a fifth noise layer, naming the material.
- `pfx bake` reads a library through the recipe's `materials = "<library.toml>"` (`docs/bake.md`), resolving each glTF material by its name, then by its name before the first `.`, then by the recipe's `fallback`.

## See-through and tinted surfaces

A material lets light through when `transmission` is above 0, and the live renderer then draws it in the glass pass (`glass::Surface`), not as an opaque instance: a staged scene does that for every material of the `glass` family and every material whose `transmission` is above 0, whatever its family. What shows is the material's own mix, the same split the tracer makes:

- **Transmitted:** the scene behind the surface times `base`, times `transmission`. `base` is the tint: a yellow lens is `base = [0.9, 0.8, 0.05]`.
- **Body:** the remaining `1 - transmission` is the surface's own lit `base` (diffuse, from the sky and the sun), so a half-clear lens reads as tinted plastic, not as a mirror.
- **Reflected:** dielectric Fresnel from `ior` over both, and `clearcoat` (with `clearcoat_roughness`) on top.
- `transmission = 0` is an opaque surface and stays an opaque instance; `transmission = 1` is clear glass, drawn as before.

**Thin and solid.** A closed mesh with some thickness refracts: the glass pass finds its entry and exit faces and bends the view through them, with `absorption` over the distance inside. A single open sheet, or a closed lens thinner than the pass can resolve (its front and back depths are stored as 16-bit floats, so about 4 mm at 5 m and 1 mm at 1.5 m), draws as a thin wall: the scene straight behind it, tinted, with Fresnel and the coat, and no offset. Spectacle lenses, visors and windows that are modelled as one sheet or a 1–3 mm box are thin walls; nothing has to be set for it. A sheet shows its front face only, the side its winding faces.

What to write for a tinted lens, in a library or a recipe's `[materials.*]`:

```toml
[materials.lens]
base = [0.9, 0.8, 0.05]
roughness = 0.05
transmission = 0.6
ior = 1.5
clearcoat = 1.0
clearcoat_roughness = 0.03
```

From glTF, a material maps as `base` ← `pbrMetallicRoughness.baseColorFactor` (rgb), `roughness` ← `roughnessFactor`, `transmission` ← `KHR_materials_transmission.transmissionFactor`, `ior` ← `KHR_materials_ior.ior` (1.5 without it), `clearcoat` and `clearcoat_roughness` ← `KHR_materials_clearcoat`. glTF's `KHR_materials_volume` says a surface is a solid with a `thicknessFactor`; without it the material is thin-walled, which is what a sheet or a 2 mm lens draws as here. A refracting solid (a glass block, a bottle, a water tank) is `family = "glass"` on a closed mesh, with `absorption` (from `attenuationDistance`) and `subsurface_tint` (`attenuationColor`) absorbing over the distance the view travels inside, and `thickness` (`thicknessFactor`) for the tint of its cast shadow. The scene loader takes materials from the library by the glTF material's name, not from the glTF extensions, so the exporter writes these keys into the library.

## The engine's fixture library

`crates/materials/fixtures/library.toml` (`pfx_materials::FIXTURE`, parsed by `Library::fixture()`) holds eleven neutral materials for the engine's own tests: `plain`, `grey`, `metal`, `chrome`, `glass`, `water`, `lacquer`, `card`, `wood`, `felt` and `lamp`. Their values are round, made-up numbers, named after the stuff and not after any product. The engine holds no product's presets any more: a product ships its own library.

Other neutral fixtures for engine tests live beside the crates they exercise: a post finish at `crates/post/tests/fixtures/finish.toml` (read with `pfx_post::parse_str`: exposure, ring bloom, saturation, contrast, AgX tone, dither and encode, no product style), camera presets at `crates/core/tests/fixture/mod.rs` (`Preset` and `Drift` built from their public fields) and physics presets at `crates/physics/tests/fixture/mod.rs` (a `ChainParams` and a `FluidDesc` with its own table and tuning).

`pfx_materials::fixtures` builds a second neutral set in code, one material per parameterised noise kind (`fixtures::all()`: plain, lacquer, metal, wood, wall, sheet, aged, glow, glass, liquid), and `fixtures::glass()` and `fixtures::liquid()` stand in wherever a test needs a transmissive surface.

## Noise layers

A `Material` carries up to four `NoiseLayer`s: a `kind`, a `frequency`, an `amplitude`, a `seed` and twelve `params` whose meaning the kind defines. A layer whose amplitude is 0 does nothing. Each parameterised kind has a typed builder that writes its params in order and reads them back (`Kind::from_params`), and a neutral `SAMPLE` the engine's own tests use. Unset params are 0, and a kind with no params draws nothing visible (plank wood and coat wobble need a positive first param to draw at all).

| Kind (shader id) | Builder | Params, in order |
|---|---|---|
| `Fibre` (1) | `Fibre` | `across`, `along` (the fibre streaks' frequencies across and along the surface's y), `fine`, `mottle`, `ripple`, `swell` (two bump frequencies), `fibre_gain`, `mottle_gain`, `ripple_gain`, `swell_gain` |
| `Crinkle` (2) | `Crinkle` | `frequencies[2]`, `gains[2]` of two bump octaves |
| `PlankWood` (3) | `PlankWood` | `width` of a plank in metres (planks run along x and stack in z), `rings` per metre, `warp` of the rings in metres, `figure[2]` frequencies along and across the grain, `tone` spread between planks, `figure_depth`, `seam_floor` (the seam's darkest factor), `roughness` gain from the figure, `jitter` of the rings |
| `WallMottle` (4) | `WallMottle` | `broad[2]` and `fine[2]` albedo octaves, `swell[2]` bump octaves, `broad_gain`, `fine_gain`, `swell_gains[2]` |
| `Grime` (5) | `Grime` | `frequencies[2]`, `dirty` and `clean` tints, `roughness[2]` factors for dirty and clean |
| `Leaf` (7), `Bark` (8) | — | none: fixed fbm patterns with a height relief |
| `Flow` (14) | — | `frequency` scales the uv; a warped 2D value noise that moves with `At::time` and shifts the thin film |
| `Value` (15), `Fbm` (16) | — | `frequency` scales the position |
| `Scratch` (17) | `Scratch` | `line` frequency across the scratches (stretched by `SCRATCH_STRETCH` = 0.02 along them), `smudge[2]`, `speck` |
| `CoatWobble` (18) | `CoatWobble` | `frequencies[3]` (fine, then two broad), `weights[2]` of the broad octaves relative to the fine one, whose weight is the layer's amplitude |

The shader ids are stable; ids 6 and 9 to 13 are retired. The algorithms' shape (mix weights, thresholds, the warp octaves inside the plank rings) is fixed; scales and gains are the params.

On the GPU each layer is five `vec4` rows: the kind row and four parameter rows. Twelve values are the params; `gpu_params` adds four the packer derives (a plank's reciprocal width and seam span, a scratch's stretch), and the live renderer adds its own (the ring fade rate, `RING_FADE` = 8/7 of the ring frequency, and a coat's fine scale and tile match). `PackedMaterial` is 512 bytes: the layer rows, then one `wet` row last (see Wet surfaces).

## Noise functions

`value_noise3(p, hash)` is one value noise over a 3D lattice, on `Hash::Xor` (the xor of three products through `pcg`) or `Hash::Nested` (`pcg` nested per axis). `fbm3` and `fbm3_filtered` sum its octaves at lacunarity 2.03; `value_noise3_gradient(p, scale)` is the central-difference gradient (±0.35 of a cell) on the xor lattice, reusing the corner hashes; `value_noise2_signed` is the 2D noise `Flow` warps. The WGSL in `NOISE` carries the same names (`value_noise3(p, nested)`), and the tracer's copy matches.

## Wet surfaces

`Material::wet` (`Wet { amount, puddles, puddle_scale, seed }`, the `[wet]` table) soaks a material after its maps, ageing, noise layers and content, the same way in the live renderer and the tracer: `soak` and `puddle` on the CPU, `wet_soak` and `wet_puddle` in `WET`, which both renderers' shaders include. It follows Sébastien Lagarde's published wet-surface model ("Water drop 3b: physically based wet surfaces" and "Water drop 2b: dynamic rain and its effects", 2013):

- **Wet level.** `level = max(amount, water)`, where `water` is the puddle mask below: under a puddle the ground is fully wet.
- **Porosity darkens.** `porosity = clamp((roughness − 0.5) / 0.4, 0, 1)`: a surface smoother than 0.5 takes no water in. `factor = mix(1, 0.2, (1 − metalness) · porosity)`, `base *= mix(1, factor, level)` and `roughness *= mix(1, factor, 0.5 · level)`, Lagarde's darkest 0.2 and half-weighted gloss gain. Metals and smooth surfaces keep their colour.
- **A water film.** The clear coat turns on: `clearcoat = max(clearcoat, mix(0.5 · level, 1, water))`, so a damp film has water's normal-incidence reflectance (half the coat's 0.04 is water's 0.02) and a puddle the full coat, grazing Fresnel included. The coat's roughness goes from the wet roughness to `WATER_ROUGHNESS` (0.03) as `water` goes to 1; an existing coat keeps the smaller of its own and that. The tracer floors every roughness at 0.05, so its puddles are a touch broader.
- **Puddles are flat.** The shading normal is blended to the geometric normal by `water`, so a puddle hides the bumps and normal map under it.
- **The mask.** `water` is `fbm3_filtered`'s three octaves of value noise on the xor lattice (in WGSL `wet_fbm`, the same sum over `value_noise3`, so the bake's copy of the tracer carries it too) at `(x / puddle_scale, seed slice, z / puddle_scale)` in world space, thresholded at `0.4375 − x · (0.24 + 2.1 x²)` with `x = puddles − 0.5` and a ±0.04 soft shore, times `smoothstep(0.8, 0.95, n_y)` of the geometric normal: water stands only on ground facing up, never on walls. The cubic spreads the threshold over the noise's spread, so `puddles` 0 is dry ground, 0.5 covers about half and 1 floods it. The `seed` (`u64`, folded to 32 bits) picks the slice through the noise. Live filters the octaves by the pixel's footprint (`dpdx` of the world position); the tracer, which supersamples, takes them all.

A `[wet]` whose `amount` and `puddles` are both 0 is skipped in both shaders, and a material without one packs a zero `wet` row, so dry scenes render bit for bit as before (`an_idle_wet_table_renders_byte_identical_in_both_renderers`). `a_wet_floor_is_darker_and_glossier_and_its_puddles_follow_the_mask` renders one floor in both: damp at 0.8 the floor reads 0.41 of dry in both renderers, a flooded floor shows the sun's glint, and every pixel the CPU mask calls in or out of a puddle (7,937 of 9,216 at 96×96) is darker or unchanged accordingly, in both.

**Reflections.** A puddle reflects what live already has for a glossy coat: screen-space reflections (the opaque pass writes the coat's roughness into the G-buffer for a wet pixel, so SSR traces it at water's roughness), then the local reflection cubes, then the sky. SSR loses whatever is off screen, and a street's buildings mostly are, so a street wants reflection cubes baked along it with `pfx bake`: one `[[reflection]]` every 10 to 20 m down the street, each at the street's centre about 1.5 m up, its box spanning the street between the façades and overlapping its neighbours by a `fade` of 2 to 4 m, at `resolution` 256 or 512 (`docs/bake.md`, Local reflections). The cubes carry their hit distances, so the parallax holds across each box, and the live renderer blends the nearest two at every pixel.

```toml
[[reflection]]
name = "street-020"
position = [0.0, 1.5, -20.0]
min = [-8.0, 0.0, -30.0]
max = [8.0, 12.0, -10.0]
resolution = 256
fade = 3.0
```

**Cost.** On a full-screen street at 2160×3840 (a road and two walls, every pixel wet, an AMD Radeon RX 9060 XT under Linux, `pfx bench` under `pgpu --class bench`, 300 frames each, p50 GPU frame time): dry 3.32 ms, damp (`amount` 0.6) 3.52 ms, damp with `puddles` 0.5 at 2 m 4.32 ms. The damp film costs 0.20 ms (the soak and the coat's own reflection lobe); puddles add 0.80 ms more, for the noise mask and the sharper screen-space reflections water's roughness asks for (the split between the two was not measured). Dry materials cost nothing.

## Content

A `ContentLayer` composites a content slot as `Over`, `Multiply` or `Emit`. `ContentLayer::from_kind(content, clearcoat, slot, &look)` maps a `Content` kind with the caller's `ContentLook`: `ink_roughness` (−1 keeps the surface's), `ink_emboss_coated` and `ink_emboss_bare`, `photo_gain`, `screen_gain` and `photo_window` (the photo's uv offset and scale). `ContentLook::default()` is neutral: no roughness change, no emboss, unit gains, the whole image. The tracer takes the look from `Detail::content_look` for materials whose content layer it fills in itself.

