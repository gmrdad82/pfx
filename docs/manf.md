# .manf files

A `.manf` is Manfred's container (the `manf` crate in pfx-scene, read through `pfx_scene::manf` at tag v1.5.0; format in pfx-scene's `docs/format.md`): a header, a chunk table with names, then 8-byte aligned chunks, each CRC-32 checked and stored or compressed (LZ4 block or raw deflate). It holds a mesh, a whole scene, or both. As a mesh, pfx opens one wherever it opens a `.glb` or `.gltf`: a scene's `[mesh.<name>] file`, prefabs and tile palettes, `pfx shot`, `pfx film`, `pfx edit` (the asset browser places `.manf` files like glTF ones), the game runtime, `Cache::load_mesh_from` and every bake recipe key that names a mesh file (`scene`, `[[file]] path`, `[[pose]] file`, a prop's `file`).

## As a mesh

- `GLTF`: the glTF JSON. Required; a GLB in this chunk is refused.
- `GBIN`: the glTF BIN buffer, the one buffer without a `uri`.
- `FILE`: named files. A glTF buffer or image `uri` resolves to the `FILE` chunk of that name first (`./` dropped), then to the file beside the `.manf` on disk under the same rules as a `.gltf`'s (inside its folder, no absolute path or `..`).
- Everything else is skipped: `THMB`, Manfred's `RCPE`, `FACT`, `NODE` and `OVRD`.
- A `.manf` holding a scene and no `GLTF` is refused as a mesh; open it as a scene.

The parsed glTF goes through the same code as a `.glb` (`Mesh::parse_with`, which detects the container by its magic `MANFRED\0`), so geometry, materials, texture maps, nodes, skins, morph targets, animations and levels of detail come out equal to the GLB the container was written from. `pfx_load::open_gltf(bytes)` gives the `gltf::Gltf` of either kind to code that reads the document itself; `pfx_load::is_mesh_file(name)` is the one suffix check (`.glb`, `.gltf`, `.manf`, any case).

## As a scene

A `.manf` with an `SMET` chunk is a scene (pfx-scene's `docs/format.md`, Scenes in a .manf file): `SCNE` chunks hold its TOML files and `FILE` chunks every other file it names, each by its path from the project root. `Scene::open`, `SceneWatch`, `SceneEdit`, `pfx shot <scene.manf>`, `pfx edit <scene.manf>`, `pfx bench`, the game runtime and `Staged` all take one where they take a `.scene.toml`; a `.scene.toml` opens exactly as before.

- **Files.** Every file the scene reads (its TOML, meshes and their buffers, images, fonts, sounds, LUTs, tile prefabs) resolves to the container's chunk of that name first, then to the file in the project on disk, as `pfx_scene::Project::asset` does. A mesh `file = "@gltf"` (with `node`) is the container's own `GLTF` and `GBIN`; in a scene that is not a `.manf` it is refused.
- **Errors** name a file inside the container as `<the .manf>/<its name>`, with line and column (`scenes/room.manf/materials.toml:4:13`). A cut or corrupt container, one without `SMET` or naming an entry it doesn't hold, is refused.
- **Reload.** Scene watch hashes the `.manf` once for everything inside it; files the scene reads from the disk beside it are watched as usual.
- **Editing.** `SceneEdit::open(<scene.manf>)` and `pfx edit <scene.manf>` edit the TOML inside the container: an edit, undo or redo rewrites the `.manf` once (a temporary file and a rename), keeping every other chunk byte for byte, and undoing every edit gives back the same bytes. The editor's file panel shows the inner files as `<the .manf>/<its name>`. `pfx_load::scene::held_text(path)` reads one such file, and `container_of(path)` splits the path.
- **Packing.** `pfx scene migrate --manf <scene.toml> <out.manf>` packs a scene and every file it names into one `.manf` (`pfx_load::scene::pack`, through `pfx_scene::pack`: the same files give the same bytes); `pfx scene migrate --unpack <scene.manf> <dir>` writes its TOML and files back out (`unpack`). A glTF's own buffers and images are not scene paths and are not packed: pack a `.glb`, or keep them beside the `.manf`.
- **Not read through the container:** the scene-level `sound = "<path.sound.toml>"` mix script that `pfx shot` renders. pfx-sound reads the script and its samples by path from the disk, so a `.manf` scene's mix script stays a file beside it.

## Refusals

A `.manf` that is cut short, has a bad table, an unknown codec, a chunk that does not decode or a checksum mismatch in `GLTF`, `GBIN` or a buffer's `FILE` chunk is refused with the container's reason (`corrupt .manf: chunk GBIN: checksum mismatch`), and the scene's error names the file. A texture whose `FILE` chunk is damaged is dropped like an unreadable glTF image.

## Reload of a mesh

Scene watch hashes the `.manf` like any mesh file, so saving it in Manfred reloads the mesh in a running `pfx edit` or viewer. Files resolved from inside the container need no watch of their own; files resolved beside it are watched as a `.gltf`'s are.

## Load time

The file is read whole into memory (no memory map: the scene hashes every byte for reload and the chunks are checksummed and decoded whole, so a map would save no reads). Stored chunks borrow from that buffer; the BIN is copied once into the glTF blob. Measured on 2026-10-16 with `cargo run --release -p pfx-load --features fixture --example manf_load` (generated UV spheres, read plus `Mesh::parse`, cold after dropping the file from the page cache, warm the median of seven, NVMe, Ryzen 9 9950X):

| model | triangles | file | bytes | cold ms | warm ms |
|---|---|---|---|---|---|
| car-sized | 159,200 | .glb | 5,780,288 | 8.8 | 4.2 |
| car-sized | 159,200 | .manf LZ4 | 2,974,760 | 7.9 | 3.4 |
| car-sized | 159,200 | .manf stored | 5,780,376 | 8.4 | 5.0 |
| scene-sized | 3,996,000 | .glb | 144,097,120 | 143 | 128 |
| scene-sized | 3,996,000 | .manf LZ4 | 80,932,304 | 166 | 158 |
| scene-sized | 3,996,000 | .manf stored | 144,097,208 | 154 | 144 |

LZ4 halves the file. At car size it loads as fast as the GLB or faster; at four million triangles the container's checksum over 144 MB and the LZ4 decode add 12 to 23 percent over the GLB, which reads its BIN unchecked. Parsing the glTF into meshes is most of every number.
