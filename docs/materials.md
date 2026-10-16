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

On the GPU each layer is five `vec4` rows: the kind row and four parameter rows. Twelve values are the params; `gpu_params` adds four the packer derives (a plank's reciprocal width and seam span, a scratch's stretch), and the live renderer adds its own (the ring fade rate, `RING_FADE` = 8/7 of the ring frequency, and a coat's fine scale and tile match). `PackedMaterial` is 624 bytes: the 496 of the rows above, then eight rows for the lobes and texture maps below (`PackedMaterial::LOBES_OFFSET`).

## Noise functions

`value_noise3(p, hash)` is one value noise over a 3D lattice, on `Hash::Xor` (the xor of three products through `pcg`) or `Hash::Nested` (`pcg` nested per axis). `fbm3` and `fbm3_filtered` sum its octaves at lacunarity 2.03; `value_noise3_gradient(p, scale)` is the central-difference gradient (±0.35 of a cell) on the xor lattice, reusing the corner hashes; `value_noise2_signed` is the 2D noise `Flow` warps. The WGSL in `NOISE` carries the same names (`value_noise3(p, nested)`), and the tracer's copy matches.

## Content

A `ContentLayer` composites a content slot as `Over`, `Multiply` or `Emit`. `ContentLayer::from_kind(content, clearcoat, slot, &look)` maps a `Content` kind with the caller's `ContentLook`: `ink_roughness` (−1 keeps the surface's), `ink_emboss_coated` and `ink_emboss_bare`, `photo_gain`, `screen_gain` and `photo_window` (the photo's uv offset and scale). `ContentLook::default()` is neutral: no roughness change, no emboss, unit gains, the whole image. The tracer takes the look from `Detail::content_look` for materials whose content layer it fills in itself.

## Figurine lobes: sheen, anisotropy, scatter

Three lobes for small figurine heads (cloth, hair, skin, vinyl), cheap on every vendor and the same in both renderers: the WGSL is one file, `pfx_materials::LOBES` (`crates/materials/src/wgsl/lobes.wgsl`), that the live opaque pass, its local lights and the tracer all call through `eval_lobed` and `eval_lobes`. Each is off by default, and a material that leaves all three off takes the old `eval_filmed` path unchanged, so existing materials render byte for byte as before. In a material's TOML (and so in a scene's `*.materials.toml`) they are inline tables:

```toml
[materials.knit]
base = [0.25, 0.05, 0.05]
roughness = 0.9
fabric = { color = [0.8, 0.8, 0.8], roughness = 0.5 }

[materials.hair]
base = [0.12, 0.08, 0.05]
roughness = 0.35
anisotropy = { strength = 0.8, rotation = 0.0 }

[materials.skin]
base = [0.8, 0.6, 0.5]
roughness = 0.6
scatter = { color = [1.0, 0.3, 0.2], radius = 0.01, amount = 1.0 }
```

Being material keys, the edit path reaches them by their key path (`SceneEdit::set_material("knit", &["fabric", "roughness"], 0.4)`, so the scene CLI's `--set materials.knit.fabric.roughness=0.4` too), pfx-scene checks their ranges, and scene watch shows a file edit at once. They are new pfx-scene keys (`Material::fabric`, `anisotropy`, `scatter`); a pfx-scene release that carries them is the pin that lets a scene file use them.

### Sheen: `fabric`

glTF's `KHR_materials_sheen`, for cotton, knit and felt. Keys: `color` (linear RGB, 0 to 1, default `[0, 0, 0]`, which is off) and `roughness` (0 to 1, default 0, floored at 0.045 as Filament does). The lobe is Filament's: the Charlie distribution of Estevez and Kulla (`d_charlie`) with Neubelt and Pettineo's visibility (`v_neubelt`), sheen alpha = roughness². The layer beneath is scaled by `1 - max(color) · E(n·v, roughness)` and the lobe added on top, under a clearcoat if there is one. `E` is the directional albedo of that lobe, tabulated 16 × 16 over n·v and roughness (`SHEEN_TABLE`, `crates/materials/src/wgsl/sheen_table.txt`, the integral `sheen_albedo_exact` computes; a unit test keeps the two equal), the energy compensation Filament takes from its DFG table's third channel. Live adds the ambient sheen as `color · E · irradiance / π` and scales its ambient diffuse and reflections by the same factor; the tracer gets both from its paths.

The older scalar `sheen` (a grazing tint on the diffuse) is unchanged and independent.

### Anisotropy: `anisotropy`

glTF's `KHR_materials_anisotropy`, for hair and beard. Keys: `strength` (0 to 1, default 0, off) and `rotation` (radians, default 0, turning the direction from the tangent toward the bitangent). As the extension says, the roughness along the direction is `mix(α, 1, strength²)` and across it `α`; the distribution and the height-correlated visibility are Filament's anisotropic GGX (`d_ggx_anisotropic`, `v_smith_anisotropic`). Live bends its reflection normal toward the anisotropic normal as Filament does (`lobes_reflection_normal`, by `strength · min(5 · roughness, 1)`). The tracer samples the anisotropic distribution (`anisotropic_half`) and weighs it with its own density, so hair converges as fast as a plain metal.

The direction is the mesh's tangent, so a mesh needs tangents: a glTF without `TANGENT` gets them from its UVs (`surface::tangents`, the per-triangle UV derivative summed per vertex) for every part whose material has anisotropy. A glTF's `anisotropyTexture` (red and green the direction in tangent space, blue a factor on the strength) turns it per texel.

### Scatter: `scatter`

A cheap subsurface stand-in for skin and vinyl. Keys: `color` (the colour that bleeds past the terminator, linear RGB 0 to 1, default `[1, 0.35, 0.2]`), `radius` (metres, above 0, default 0.01) and `amount` (0 to 1, default 0, off). It is Filament's cloth subsurface, the energy-conserving wrap `(n·l + w) / (1 + w)²` tinted by `saturate(color + n·l)`, mixed into the diffuse by `amount`. The wrap width is `w = radius · curvature`, clamped to 1: live takes the curvature from the pixel's normal and position derivatives, the tracer from the hit triangle's vertex normals over its edges. So a small radius on a big, flat surface does nothing and the same radius on a small head softens and warms its terminator, as a pre-integrated skin ramp would (Penner), without a lookup texture or a new binding. Past the terminator, where the wrap still lights the skin, live lights it with the sun's shadow map and the tracer with a shadow ray bent into the tangent plane, so a head's own curve does not shadow it but a hat does. Local lights wrap only on their lit side.

The tracer uses the stand-in and not a random walk: Cycles' random-walk subsurface needs a second walk through the mesh per hit, which is not bounded in cost for a game's heads, and the stand-in matches live. Bounces off a scattering surface take the plain diffuse, so the wrap adds no fireflies.

## Texture maps on imported meshes

A GLB's own materials render in both renderers when the scene names no library material for them (the order in `docs/scenes.md`, Meshes). pfx-load reads, per glTF material (`pfx_load::surface::Surface`, `Mesh::surfaces`):

- the core factors: `baseColorFactor`, `metallicFactor`, `roughnessFactor`, `emissiveFactor`;
- `baseColorTexture` (sRGB) multiplying the base colour;
- `normalTexture` with its `scale`, a tangent-space normal map;
- `metallicRoughnessTexture` (green roughness, blue metalness, as factors) and `occlusionTexture` with its `strength`, packed into one ORM map (red occlusion, green roughness, blue metalness; a packed ORM texture is used as is, separate ones are combined, the occlusion resampled to the other's size);
- `KHR_materials_sheen` (`sheenColorFactor`, `sheenRoughnessFactor`) into `fabric`, and `KHR_materials_anisotropy` (`anisotropyStrength`, `anisotropyRotation`, `anisotropyTexture`) into `anisotropy`;
- a detail normal map, which glTF has no slot for, from the material's extras as a textureInfo: `"extras": { "detailNormalTexture": { "index": <texture>, "texCoord": 1, "scale": 1.0, "extensions": { "KHR_texture_transform": { … } } } }` (`texCoord` 1 when left out), blended over the normal map (whiteout blend: the slopes add, the heights multiply);
- `KHR_texture_transform` (`offset`, `rotation`, `scale`, `texCoord`) on each of those textures, applied as the extension says (`uv' = offset + R(rotation) · (scale · uv)`), the gradients turned with it.

The base, normal, ORM and anisotropy maps share one UV set and one transform: the first of them that the material has decides (base, then normal, then metallic-roughness, then occlusion), and a texture on a set past `TEXCOORD_1` is skipped. The detail map has its own set and transform. Live reads `TEXCOORD_1` from its vertex stream; the tracer carries it per triangle only when a material reads it. Images are PNG (8 or 16 bit); another format leaves its slot empty, and the mesh still loads. Sheen and anisotropy textures other than `anisotropyTexture`, emissive textures, `KHR_texture_transform` and alpha modes are not read. The material is `Material` with `textures: Textures` (base, normal, ORM, detail and anisotropy slots, the normal scale, the occlusion strength, the detail scale, and the two `UvTransform`s), which TOML never carries: it is the import's alone.

`Scene::draws()` gathers the maps of the drawn glTF materials into `Draws::textures` (`colour`, `linear` and `orm` lists, shared images once) and points each material's slots at them. Live uploads them into its four material map arrays when staged (`pfx_live::textured::upload`; each array's layers are resampled on the CPU to the largest image in it, at most 4096 a side, so a GLB may carry its maps at any sizes) and samples them in the opaque pass (`TEXTURED_WGSL`); no binding is added, so the GPU floor is unchanged. A scene with glTF maps replaces the renderer's `set_maps` images while it is staged and restores the empty set when released. The tracer appends them to its content texels (`pfx_trace::textured`) and samples them bilinearly with wrapping UVs; `TEXCOORD_1` rides along per triangle only when a material has a detail map. Occlusion darkens live's ambient diffuse and reflections, and in the tracer the sky's light and each bounce, never the sun or a lamp, as glTF says.

## Cost

Measured once on this machine's AMD RDNA 4 (Linux, RADV) under `pgpu run --class bench`, with `cargo run --release -p pfx-live --example lobes_cost`: the generated ball (`pfx_load::fixture::ball`, 4,225 vertices) as a head 600 px across, about 283k shaded pixels, in a 2160 × 3840 frame under a sun and an analytic sky. Live times are GPU medians over 120 frames after 30; the tracer's are wall time per sample at 2160 × 3840.

| Case | Live opaque pass (ms) | Live frame (ms) | Tracer, per sample (ms) |
|---|---|---|---|
| plain material | 0.219 | 1.819 | 13.0 |
| `fabric` | 0.226 (+0.007) | 1.831 | 13.2 |
| `anisotropy` | 0.224 (+0.006) | 1.827 | 12.8 |
| `scatter` | 0.225 (+0.006) | 1.832 | 12.9 |
| all three | 0.235 (+0.016) | 1.839 | 13.0 |
| glTF maps: base, normal, ORM with occlusion, detail normal on `TEXCOORD_1` with a transform (512² each) | 0.245 (+0.026) | 1.853 | 13.8 |

A head costs at most 0.03 ms more in live with every lobe and map on; in the tracer the lobes are within its run-to-run noise (about ±0.4 ms a sample) and the maps add about 0.7 ms a sample at this size. Materials with everything off take the same shader path as before, so they cost nothing.
