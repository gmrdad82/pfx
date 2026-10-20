# The shared cache

pfx keeps work it can reuse in one cache folder per machine, shared by every run and every checkout: two renders of the same scene, from two copies of a repo or two terminals, compile and bake once. An entry is found by a hash of everything that made it, so a hit is the same bytes a recompute would give, and anything that changes the output (the inputs, the code that made it, the pfx version, the GPU, driver and ray path where the output depends on them, the format) finds a different entry.

The `pfx` tool uses the cache on its own; when the cache is misconfigured (a relative `PFX_CACHE_DIR`, a bad `PFX_CACHE_CAP`, a folder that is not private), each run says why on one line on stderr and goes on without it. The engine's library crates, which apps and games compile in, never touch it unless the program asks (see From code); the live bench example asks for its bakes only, so its pipeline timings stay its own.

## Where it lives

| Variable | What it does |
|---|---|
| `PFX_CACHE_DIR` | The folder, an absolute path. `off` turns the cache off: nothing is read or written. |
| `XDG_CACHE_HOME` | Without `PFX_CACHE_DIR`, the cache is `$XDG_CACHE_HOME/pfx`. |
| `HOME` | Without either, `~/.cache/pfx` (Linux and macOS). On Windows, `%LOCALAPPDATA%\pfx`. |
| `PFX_CACHE_CAP` | The most the cache keeps, such as `4GB` (the default), `500MB` or `20GB`. |

Only absolute paths count: a relative `XDG_CACHE_HOME`, `HOME` or `LOCALAPPDATA` is skipped, so by default the cache is never inside a checkout and nothing in it depends on the folder a run started in. The folder must be private to you: pfx makes it, and the folders in it, readable and writable by you alone, and on Linux and macOS it turns the cache off when another user owns the folder or other users can write in it, checked when the cache opens and again before each write. On Windows pfx checks neither: the default folder is under your own `%LOCALAPPDATA%`, and a `PFX_CACHE_DIR` there is yours to keep private. An entry is opened only when it is a plain file, never a link or a pipe. pfx writes a `CACHEDIR.TAG` in it, which backup tools know to skip, and only counts, evicts or cleans entries in a folder that has one, so a `PFX_CACHE_DIR` pointed at the wrong folder never removes files pfx did not write.

The build cache is a separate folder (adapters, checkouts and target folders under `PFX_CACHE`, shown by `pfx cache` alone and pruned by `pfx cache --prune`).

## What it holds

Each kind is a subfolder.

| Kind | What | Key |
|---|---|---|
| `pipelines` | The live renderer's compiled pipelines (a wgpu pipeline cache), loaded by every live renderer the `pfx` tool starts (`pfx shot`, `pfx bench`, `pfx film`, recipe stills and clips, the editor's viewport), and saved when the renderer closes if it compiled anything new. Vulkan only: other backends keep their own driver caches. | The backend, adapter, driver and driver version, and the pfx version. |
| `bakes` | `pfx bake`'s probe volumes, prerendered props and reflection cubes, each a whole artifact folder. | The artifact's own scene hash (scene geometry, materials, sky, grid, samples, seed), a hash of the source of the bake library and every engine crate it is built from with the dependency lock file, `pfx bake`'s own source (tests and docs aside; of the lock file, the registry and git packages), the GPU, driver and backend that traced it, and the ray path: hardware ray queries or the software BVH (`PFX_TRACE_RT`), the box BLAS (`PFX_TRACE_RT_BOXES`) and the sampler (`PFX_TRACE_SAMPLER`). |

A bake found in the cache prints `… (from the machine cache)` instead of tracing; a bake that traces keeps its result there for the next run. `pfx bake --force` traces again without reading the cache, and still keeps the result.

The `pipelines` entry is one per adapter, driver and pfx version. Each run loads it when its renderer starts and writes it back when the renderer closes, unless nothing new was compiled or another run wrote the entry in the meantime, so with many runs at once the entry fills up over a few runs rather than all at once.

## Reading and writing

- **Whole entries.** A writer writes a hidden `.partial` file beside the entry, flushes it, then renames it into place, so a reader sees a whole entry or none, even with several runs writing the same entry at once.
- **Checked.** Every entry starts with a header (`PFXCACH1`, the payload's length and its SHA-256). A reader checks all three before using an entry; an entry that fails is removed and the work is done again. An entry larger than this run's cap, or one that does not fit in memory, is skipped and kept for a run that can read it.
- **Capped.** Reading an entry marks it used. After each write, when the cache is over its cap, the entries used longest ago go first until it fits; the entry just written stays. An entry larger than the whole cap is not kept. A `.partial` file left by a run that died is removed after an hour.

## `pfx cache info`

```sh
pfx cache info
pfx cache info --json
```

The folder, its size, its entries and its cap, then each kind's entries and size; the two kinds above are always listed, with any other kind a caller made. `--json` prints one line:

```json
{"v":1,"tool":"pfx","tool_version":"3.1.2","root":"~/.cache/pfx","root_set_by":null,"cap":4000000000,"bytes":52428800,"entries":3,"kinds":[{"kind":"bakes","entries":2,"bytes":41943040,"oldest":1792500000000,"newest":1792500600000},{"kind":"pipelines","entries":1,"bytes":10485760,"oldest":1792500300000,"newest":1792500300000}]}
```

`root_set_by` is `"PFX_CACHE_DIR"` when that variable chose the folder. `oldest` and `newest` are each kind's least and most recent use, in milliseconds since 1970, or null for an empty kind.

## `pfx cache clean`

```sh
pfx cache clean
pfx cache clean --kind bakes
pfx cache clean --older-than 30d
pfx cache clean --kind pipelines --older-than 12h --json
```

Removes entries: all of them, those of one kind (`pipelines`, `bakes`, or any other kind a caller made), or those unused for longer than the age (`45s`, `90m`, `12h`, `30d`, `2w`). It prints what it removed and freed; `--json` prints `{"v":1,"removed":N,"freed":BYTES}`. A kind that is not lowercase letters, digits and dashes, an unknown age or an unknown flag is refused in the usual shape, exit 2:

```text
error: invalid value '3y' for '--older-than <OLDER_THAN>': unknown unit 'y': use s, m, h, d or w

For more information, try '--help'.
```

## From code

`pfx-cache` is the library behind it. A caller names a kind, builds a key from everything its output depends on, and asks for the entry:

```rust
use pfx_cache::{Keyer, Store};

let store = Store::open()?;
let key = Keyer::new("meshes", 1)
    .part(source_bytes)
    .part(detail.to_le_bytes())
    .part(code_identity)
    .finish();
let (bytes, source) = store.get_or_make("meshes", &key, || build_mesh())?;
```

`Keyer::new(kind, format)` already holds the kind, the format number and the pfx version; each `part` is length-prefixed, so two parts never read as one. Add a part for the code that makes the entry (a hash of its source, as the bakes do), and bump the format number when what an entry holds changes. `get_or_make` returns the cached bytes or makes, stores and returns them; a cache that cannot be written never fails the caller (`Source::Made { stored: Err(..) }`). `pack_dir(folder, store.cap())` packs a whole folder as one entry, refusing one over the cap before reading it, and `restore_dir` swaps it into place.

A program opts its live renderers in once, at start: `pfx_cache::share(Store::open()?)` gives every `Renderer` it creates afterwards the `pipelines` kind, as the `pfx` tool does. Without it, a renderer uses no shared cache; `Renderer::use_machine_pipeline_cache` opts in one renderer, and `Renderer::use_pipeline_cache_in` takes any store, such as a game's own folder.
