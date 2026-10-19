# glTF in and out of recipes

pfx reads any standard glTF 2.0 model, a binary `.glb`, a `.gltf` with its buffers and images beside it, or an `.obj` with its `.mtl` and textures, and writes GLBs other tools open: Blender, a game engine, a viewer. Both commands are CPU only, except `--thumb`.

## Import

```
pfx import <file.glb|file.gltf|file.obj> (--as <kind>/<name> | --out <x.pitofx>) [--licence <SPDX id>] [--credit <line>] [--author <name>] [--source <url>] [--tags <tag,tag>] [--thumb] [--json]
```

```
$ pfx import coupe.glb --as vehicle/coupe --tags car,red
library/vehicle/coupe.pitofx: vehicle/coupe, 4 nodes, 1 wheels, 1 light anchors, 9480 bytes
```

It writes a `.pitofx` document:

- **GLTF and GBIN** hold the file's JSON and BIN chunks as they are, so the node hierarchy, the materials and their KHR extensions, the textures, the morph targets and the animations all stay.
- **RCPE** is a recipe with one object, `kind = "mesh"`, `path = "@gltf"`, the document's own glTF.
- **FACT** holds what pfx detects: `size`, `min` and `max` from the bounds (metres, Y up), `nodes`, `triangles`, `wheels` from nodes named `wheel*` (`wheel_fl` is `fl`: its `centre`, its `radius` and its `axle`, the thinnest axis of its bounds), `anchors` from nodes named for a light, lamp, head or tail (`kind` `head`, `tail` or `lamp`, a `position` and the node's `-Z` as `direction`), and `kind`, `name`, `tags`, and the `licence`, `credit`, `author` and `source` the options give (`licence` is empty without `--licence`).
- **NODE** maps each node, as `<name>.<node>`, to its glTF index.
- **THMB**, with `--thumb`, a 256x256 PNG drawn headless on the live renderer.

`--as <kind>/<name>` writes `library/<kind>/<name>.pitofx` under the current folder, `--out` writes where it says and takes the file's name. It refuses a file that is none of the three, a model whose buffers, images, `.mtl` or textures are missing or outside its folder, and, for a `.glb` or `.gltf` when `gltf_validator` (the Khronos glTF Validator) is on the PATH, a file it reports errors in. `--json` prints one object: `out`, `kind`, `name`, `bytes`, `nodes`, `thumb`, `validator` (`passed` or `absent`) and `facts`.

### Split glTF and OBJ

Both are packed into one standalone GLB first (`pfx_load::pack::model`), and that GLB is what the document keeps:

- **A `.gltf`:** every buffer is read from its `uri`, a relative path resolved beside the `.gltf` and percent-decoded (`wood%20diff.jpg` is `wood diff.jpg`), or a base64 `data:` URI, the way cgltf resolves them; the buffers are joined into one BIN chunk with their views moved along, and every image with a `uri` is embedded as a buffer view with its `mimeType` (PNG or JPEG, read from its bytes). A path that climbs out of the model's folder (`../`), an absolute path and any other scheme (`https:`, `file:`) are refused. A `.glb` that names outside files is packed the same way; one that names none is kept byte for byte.
- **An `.obj`:** read as tobj reads it, with fan-triangulated faces; points and lines are dropped. Each `o` or `g` is a node with one mesh, a primitive per `usemtl` in it; texture coordinates are flipped to glTF's top-left origin, `v x y z r g b` colours become `COLOR_0`, and normals stay out when the file has none (the renderer smooths them). Its `mtllib` files are read beside it and their textures beside each `.mtl`, PNG or JPEG only, each embedded once. Each MTL material becomes a glTF metallic-roughness material:

| MTL | glTF |
|---|---|
| `Kd`, `map_Kd` | `baseColorFactor` and `baseColorTexture` |
| `d`, or `Tr` as 1 − `Tr` (`d` wins) | the base colour's alpha, `alphaMode` `BLEND` below 1 |
| `Pr` | `roughnessFactor`; without it, `Ns` through the Beckmann width, (2 / (`Ns` + 2))^¼, and 1 when `Ks` is black |
| `Pm` | `metallicFactor` (0 without it) |
| `Ks` (when there is no `Pr` or `Pm`) | `KHR_materials_specular`'s `specularColorFactor` |
| `Ke`, `map_Ke` | `emissiveFactor` and `emissiveTexture`, with `KHR_materials_emissive_strength` above 1 |
| `norm`, else `map_Bump` or `bump` (with `-bm`) | `normalTexture` and its `scale` |
| `Pc`, `Pcr` | `KHR_materials_clearcoat` |
| a map's `-s` and `-o` | `KHR_texture_transform` |

`map_Ka`, `map_Ks`, `map_Ns`, `map_d`, `map_Pr`, `map_Pm`, `Ni`, `illum` and `Ka` are read but not carried: glTF keeps roughness and metalness in one image, which an OBJ does not have.

## Export

```
pfx export <file.pitofx|recipe> --glb <out.glb> [--only <object>] [--json]
```

An imported model's `.pitofx` exports its own glTF. A recipe, in a `.pitofx` or not, is built first: one object (the recipe's only one, or `--only`) exports as a node tree with real pivots, the library's form, and a scene of many keeps its objects' parents as the node hierarchy. Either way the GLB is standalone and carries no pfx extras: every `extras` is dropped but a mesh's `extras.targetNames`, the morph target names glTF tools read, and so is every extension not named `KHR_`, `EXT_` or `MSFT_`. `--json` prints `source`, `glb`, `bytes` and `nodes`.

A recipe's materials are written as glTF metallic-roughness materials, every object's:

| Recipe material | glTF |
|---|---|
| `base`, `alpha` | `baseColorFactor` |
| `metalness`, `roughness` | `metallicFactor`, `roughnessFactor` |
| `base_map`, `roughness_map`, `normal_map` (`normal_scale`), `occlusion_map`, with `uv_scale`, `uv_offset` and `uv_rotation` | `baseColorTexture`, `metallicRoughnessTexture`, `normalTexture`, `occlusionTexture`, each with `KHR_texture_transform` |
| `emission` | `emissiveFactor`, with `KHR_materials_emissive_strength` above 1 |
| `transmission`, `ior` | `KHR_materials_transmission`, `KHR_materials_ior` |
| `thickness`, `absorption` (unless `thin_walled`) | `KHR_materials_volume`: `thicknessFactor`, and an `attenuationDistance` of 1 / `absorption` with an `attenuationColor` of e^(`base` − 1), so the base-tinted absorption reads back exactly |
| `clearcoat`, `clearcoat_roughness` | `KHR_materials_clearcoat` |
| `alpha_mode`, `alpha_cutoff` | `alphaMode`, `alphaCutoff` |
| `fabric`, `anisotropy` | `KHR_materials_sheen`, `KHR_materials_anisotropy` |

A GLB used as a recipe's mesh reads the same fields back. A transmissive glTF material without `KHR_materials_volume` is thin-walled, as glTF defines it. Both renderers weight transmission by 1 − `metalness`, the way glTF does: a metallic surface reflects and lets nothing through, so a "glass" with metalness 1 draws as a mirror. `pfx import` prints one line on stderr for each material with a metalness above 0.5 and any transmission, naming it.

```
$ pfx export library/vehicle/coupe.pitofx --glb coupe.glb
coupe.glb: 4 nodes, 9480 bytes
```
