# .pfx files

A `.pfx` is pfx's native file: one container of named chunks, each checksummed and either stored as it is or compressed on its own, so a reader opens the table and decodes only the chunks it needs. It holds a mesh, a whole scene, or both. The `pfx-file` crate (`crates/file`) reads and writes it; std only, no `unsafe`, and its one dependency is miniz_oxide (raw deflate). Works on Linux, Windows and macOS.

As a mesh, pfx opens a `.pfx` wherever it opens a `.glb` or `.gltf`: a scene's `[mesh.<name>] file`, prefabs and tile palettes, `pfx shot`, `pfx film`, `pfx edit` (the asset browser places `.pfx` files like glTF ones), the game runtime, `Cache::load_mesh_from` and every bake recipe key that names a mesh file (`scene`, `[[file]] path`, `[[pose]] file`, a prop's `file`).

## The layout

Little-endian throughout.

| Bytes | Holds |
| --- | --- |
| 8 | magic `PFXFILE\0` |
| 4 | format version, `u32` (1) |
| 4 | chunk count, `u32` (at most 65,536) |
| 45 × count | the chunk table, uncompressed, one entry per chunk in file order |
| sum of name lengths | the name block, uncompressed UTF-8 |
| up to 7 | zero padding to an 8-byte boundary |
| … | the payloads, each starting on an 8-byte boundary, zero padded between |

A table entry:

| Bytes | Field |
| --- | --- |
| 4 | tag, `[u8; 4]` |
| 4 | flags, `u32`: bit 0 compressed, bit 1 stored as is; exactly one is set, the others are 0 |
| 1 | codec: 0 none, 1 raw deflate (RFC 1951, no zlib header), 2 LZ4 block (no frame, no size prefix) |
| 8 | payload offset from the start of the file, a multiple of 8 |
| 8 | stored length |
| 8 | raw length (at most 4 GiB) |
| 4 | CRC-32 (IEEE, as zlib's `crc32`) of the raw bytes |
| 4 | name offset into the name block |
| 4 | name length (0 for an unnamed chunk, at most 1024) |

A name is a relative `/` path in UTF-8: no leading `/`, no `\`, no `:` (so no drive), no empty, `.` or `..` part, no control character, at most 1024 bytes, and unique among the chunks of its tag.

## Compression

Each chunk is compressed on its own, so a reader decodes only what it asks for. `Codec::Auto` stores PNG, JPEG, RIFF, gzip, zip, zstd, Ogg and FLAC data as it is, compresses everything else with LZ4 and keeps the raw bytes when that saves nothing. `Codec::Store`, `Codec::Deflate` (level 6) and `Codec::Lz4` force one. Chunks of 1 MiB or more are encoded on their own threads.

LZ4 is the default because it opens files several times faster than deflate for a file about 1.6 times larger. Measured on 2026-10-16 on GLBs split into their JSON and binary chunks, the medians of 5 interleaved runs on a busy machine:

| Scene | GLB bytes | Deflate 6 | Deflate 1 | LZ4 | Decode, deflate 6 | Decode, LZ4 | Open to parsed glTF: stored, deflate 6, LZ4 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| a coupe | 549,592 | 173,102 (0.31) | 191,365 (0.35) | 243,349 (0.44) | 0.77 ms | 0.13 ms | 0.08, 0.83, 0.18 ms |
| a building piece | 94,580 | 14,173 (0.15) | 18,160 (0.19) | 24,451 (0.26) | 0.065 ms | 0.014 ms | 0.036, 0.12, 0.056 ms |
| a chase scene | 6,714,268 | 1,705,823 (0.25) | 1,957,058 (0.29) | 2,786,567 (0.42) | 6.48 ms | 1.19 ms | 0.86, 7.25, 2.00 ms |

LZ4 opens the chase scene 3.6 times faster than deflate and decodes 5.4 times faster, for a file 1.6 times larger; it also encodes in 6.4 ms against deflate 6's 145 ms. Deflate 1 is both larger and slower to decode than deflate 6, so deflate stays for a writer that wants the smaller file. The LZ4 port decodes as fast as lz4_flex's safe decoder and gives the same bytes.

## What a reader refuses

A file that does not start with the magic (`not a .pfx file`), a newer format version than the reader's, and a file whose table does not hold: a chunk count over the cap, a name out of the block or invalid, flags that disagree with the codec, a stored chunk whose stored and raw lengths differ, a raw length over 4 GiB or over what its stored bytes can expand to (1,032 times for deflate, 256 times for LZ4, plus 64), and a payload that is misaligned, overlaps the table or runs past the end. A chunk decodes into exactly its raw length, never more, and its checksum must match. Each comes back as an `Error`, never a panic (`corrupt .pfx: chunk GBIN: checksum mismatch`). Unknown tags are skipped: the reader lists them and decodes them only when asked. Older format versions read.

## Tags

The crate names the well-known tags; every other tag is opaque to it.

| Tag | Holds |
| --- | --- |
| `GLTF` | the glTF JSON |
| `GBIN` | the glTF binary buffer, so a reader hands both straight to its glTF path |
| `SCNE` | a scene TOML file, repeatable, named by its path (`main.toml`, `materials/wood.toml`) |
| `SMET` | the scene's metadata, unnamed, stored TOML: `format`, `entry`, `recipe` and `scene` |
| `RCPE` | the recipe TOML a scene builder made the file from |
| `FILE` | a referenced file's bytes alone, repeatable, named by its relative path |
| `THMB` | a PNG thumbnail, stored |
| `FACT` | the builder's facts, TOML |
| `NODE` | the builder's object and piece names to glTF node indices |
| `OVRD` | the builder's overrides of edits made outside it |

## The hashes and the rules for editing

`SMET`'s `recipe` is the lowercase hex SHA-256 of `RCPE`'s raw bytes, and its `scene` the lowercase hex SHA-256 over the `SCNE` chunks in table order, each as its name, a 0x00 byte, then its raw bytes, concatenated. `pfx_file::scene_hash(&reader)` computes the second, `pfx_file::sha256::sha256` and `pfx_file::sha256::hex` the first. A builder writes both when it builds; on open it recomputes the scene hash, and a mismatch says the scene was edited elsewhere.

The scene editor and pfx-scene change only `SCNE`, `FILE` and `SMET`'s `format` and `entry`, never its `recipe` or `scene`. They copy every other chunk through byte for byte, in order (`Writer::copy`), and add a new chunk at the end.

## The crate

```rust
use pfx_file::{Codec, FILE, GBIN, GLTF, Reader, THMB, Writer};

let bytes = Writer::new()
    .chunk(GLTF, json, Codec::Auto)
    .chunk(GBIN, bin, Codec::Auto)
    .named(FILE, "maps/wood.png", png, Codec::Store)
    .to_vec()?;
let reader = Reader::from_bytes(&bytes)?;
let thumb = reader.chunk(THMB)?;
let wood = reader.named(FILE, "maps/wood.png")?;
```

- `Reader::from_bytes(&[u8])` reads a slice it borrows; a stored chunk comes back borrowed from that slice, zero-copy, and may outlive the reader. `Reader::open(path)` reads only the header, the table and the name block, then each chunk's range when it is asked for.
- `reader.chunk(tag)` (the first of a tag), `reader.named(tag, name)` and `reader.chunks(tag)` (every one, with its name) return `Result<Option<Cow<[u8]>>>` or a list, `Ok(None)` or an empty list for a chunk that isn't there; `reader.entries()` lists every chunk's tag, name, flags, codec, offset, stored and raw lengths and checksum; `reader.read(index)` and `reader.stored(index)` take a table index; `pfx_file::tag_text(tag)` prints a tag.
- `Writer::new().chunk(tag, bytes, codec)`, `.named(tag, name, bytes, codec)` and `.copy(&reader, index)?` (a chunk copied through as stored, without recompressing) build a file; `.to_vec()` gives its bytes and `.write(path)` writes it atomically (a temporary file beside it, synced, then renamed). The same chunks give the same bytes.
- `pfx_file::lz4::compress` and `pfx_file::lz4::decompress(bytes, raw_len)` are the LZ4 block codec on its own; `pfx_file::MAGIC` and `pfx_file::VERSION` are the header's.

## As a mesh

- `GLTF`: the glTF JSON. Required; a GLB in this chunk is refused.
- `GBIN`: the glTF BIN buffer, the one buffer without a `uri`.
- `FILE`: named files. A glTF buffer or image `uri` resolves to the `FILE` chunk of that name first (`./` dropped), then to the file beside the `.pfx` on disk under the same rules as a `.gltf`'s (inside its folder, no absolute path or `..`).
- Everything else is skipped: `THMB`, `RCPE`, `FACT`, `NODE` and `OVRD`.
- A `.pfx` holding a scene and no `GLTF` is refused as a mesh; open it as a scene.

The parsed glTF goes through the same code as a `.glb` (`Mesh::parse_with`, which detects the container by its magic), so geometry, materials, texture maps, nodes, skins, morph targets, animations and levels of detail come out equal to the GLB the container was written from. `pfx_load::open_gltf(bytes)` gives the `gltf::Gltf` of either kind to code that reads the document itself; `pfx_load::is_pfx(bytes)` checks the magic, and `pfx_load::is_mesh_file(name)` is the one suffix check (`.glb`, `.gltf`, `.pfx`, any case).

## As a scene

A `.pfx` with an `SMET` chunk is a scene ([scene-format.md](scene-format.md#scenes-in-a-pfx-file)): `SCNE` chunks hold its TOML files and `FILE` chunks every other file it names, each by its path from the project root. `Scene::open`, `SceneWatch`, `SceneEdit`, `pfx shot <scene.pfx>`, `pfx edit <scene.pfx>`, `pfx bench`, the game runtime and `Staged` all take one where they take a `.scene.toml`; a `.scene.toml` opens exactly as before.

- **Files.** Every file the scene reads (its TOML, meshes and their buffers, images, fonts, sounds, LUTs, tile prefabs) resolves to the container's chunk of that name first, then to the file in the project on disk, as `pfx_scene::Project::asset` does. A mesh `file = "@gltf"` (with `node`) is the container's own `GLTF` and `GBIN`; in a scene that is not a `.pfx` it is refused.
- **Errors** name a file inside the container as `<the .pfx>/<its name>`, with line and column (`scenes/room.pfx/materials.toml:4:13`). A cut or corrupt container, one without `SMET` or naming an entry it doesn't hold, is refused.
- **Reload.** Scene watch hashes the `.pfx` once for everything inside it; files the scene reads from the disk beside it are watched as usual.
- **Editing.** `SceneEdit::open(<scene.pfx>)` and `pfx edit <scene.pfx>` edit the TOML inside the container: an edit, undo or redo rewrites the `.pfx` once (a temporary file and a rename), keeping every other chunk byte for byte, and undoing every edit gives back the same bytes. The editor's file panel shows the inner files as `<the .pfx>/<its name>`. `pfx_load::scene::held_text(path)` reads one such file, and `container_of(path)` splits the path.
- **Packing.** `pfx scene migrate --pfx <scene.toml> <out.pfx>` packs a scene and every file it names into one `.pfx` (`pfx_load::scene::pack`, through `pfx_scene::pack`: the same files give the same bytes); `pfx scene migrate --unpack <scene.pfx> <dir>` writes its TOML and files back out (`unpack`). The TOML files keep their exact text inside it, so unpacking gives them back byte for byte and packing them again gives the same `.pfx`. A glTF's own buffers and images are not scene paths and are not packed: pack a `.glb`, or keep them beside the `.pfx`.
- **Not read through the container:** the scene-level `sound = "<path.sound.toml>"` mix script that `pfx shot` renders. pfx-sound reads the script and its samples by path from the disk, so a `.pfx` scene's mix script stays a file beside it.

In code:

```rust
use std::path::Path;

use pfx_scene::{Project, SceneEdit, pack};

fn main() {
    let project = Project::open(Path::new("my-game")).unwrap();
    let bytes = pack(&project, Path::new("scenes/room.scene.toml")).unwrap();
    std::fs::write("my-game/room.pfx", bytes).unwrap();
    let scene = Project::open_pfx(Path::new("my-game/room.pfx"))
        .unwrap()
        .scene(Path::new("my-game/room.pfx"))
        .unwrap();
    println!("{} objects", scene.objects.len());
    let mut edit = SceneEdit::open("my-game/room.pfx").unwrap();
    edit.set_at("crate", [-0.9, 0.35, 1.5]).unwrap();
}
```

## Refusals

A `.pfx` that is cut short, has a bad table, an unknown codec, a chunk that does not decode or a checksum mismatch in `GLTF`, `GBIN` or a buffer's `FILE` chunk is refused with the container's reason (`corrupt .pfx: chunk GBIN: checksum mismatch`), and the scene's error names the file. A texture whose `FILE` chunk is damaged is dropped like an unreadable glTF image.

## Reload of a mesh

Scene watch hashes the `.pfx` like any mesh file, so saving it reloads the mesh in a running `pfx edit` or viewer. Files resolved from inside the container need no watch of their own; files resolved beside it are watched as a `.gltf`'s are.

## Load time

The file is read whole into memory (no memory map: the scene hashes every byte for reload and the chunks are checksummed and decoded whole, so a map would save no reads). Stored chunks borrow from that buffer; the BIN is copied once into the glTF blob. Measured on 2026-10-16 with `cargo run --release -p pfx-load --features fixture --example file_load` (generated UV spheres, read plus `Mesh::parse`, cold after dropping the file from the page cache, warm the median of seven, NVMe, Ryzen 9 9950X):

| model | triangles | file | bytes | cold ms | warm ms |
|---|---|---|---|---|---|
| car-sized | 159,200 | .glb | 5,780,288 | 8.8 | 4.2 |
| car-sized | 159,200 | .pfx LZ4 | 2,974,760 | 7.9 | 3.4 |
| car-sized | 159,200 | .pfx stored | 5,780,376 | 8.4 | 5.0 |
| scene-sized | 3,996,000 | .glb | 144,097,120 | 143 | 128 |
| scene-sized | 3,996,000 | .pfx LZ4 | 80,932,304 | 166 | 158 |
| scene-sized | 3,996,000 | .pfx stored | 144,097,208 | 154 | 144 |

LZ4 halves the file. At car size it loads as fast as the GLB or faster; at four million triangles the container's checksum over 144 MB and the LZ4 decode add 12 to 23 percent over the GLB, which reads its BIN unchecked. Parsing the glTF into meshes is most of every number.

The `.pfx` file is the container these numbers were taken on with a new magic, so it loads the same: on 2026-10-18 an A/B of the `.pfx` build against the build before it (the same generated models, 5 runs each, alternated, medians, the machine at a load of 11 to 14) gave the same file sizes and equal times, the car-sized LZ4 file 5.2 ms cold before and 5.6 ms after, the scene-sized one 174 ms and 171 ms.
