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

On the GPU each layer is five `vec4` rows: the kind row and four parameter rows. Twelve values are the params; `gpu_params` adds four the packer derives (a plank's reciprocal width and seam span, a scratch's stretch), and the live renderer adds its own (the ring fade rate, `RING_FADE` = 8/7 of the ring frequency, and a coat's fine scale and tile match). `PackedMaterial` is 496 bytes.

## Noise functions

`value_noise3(p, hash)` is one value noise over a 3D lattice, on `Hash::Xor` (the xor of three products through `pcg`) or `Hash::Nested` (`pcg` nested per axis). `fbm3` and `fbm3_filtered` sum its octaves at lacunarity 2.03; `value_noise3_gradient(p, scale)` is the central-difference gradient (±0.35 of a cell) on the xor lattice, reusing the corner hashes; `value_noise2_signed` is the 2D noise `Flow` warps. The WGSL in `NOISE` carries the same names (`value_noise3(p, nested)`), and the tracer's copy matches.

## Content

A `ContentLayer` composites a content slot as `Over`, `Multiply` or `Emit`. `ContentLayer::from_kind(content, clearcoat, slot, &look)` maps a `Content` kind with the caller's `ContentLook`: `ink_roughness` (−1 keeps the surface's), `ink_emboss_coated` and `ink_emboss_bare`, `photo_gain`, `screen_gain` and `photo_window` (the photo's uv offset and scale). `ContentLook::default()` is neutral: no roughness change, no emboss, unit gains, the whole image. The tracer takes the look from `Detail::content_look` for materials whose content layer it fills in itself.

