# `pfx-mod`: sandboxed, deterministic mods

`pfx-mod` hosts a game's mods and scripts as WebAssembly components on wasmtime (Cranelift, pinned at `=49.0.2`, with the component model). A mod is untrusted code from a stranger, so the host gives it nothing the game did not hand it, meters every instruction, caps its memory, tables and stack, and stops it for the run the moment it misbehaves. The game keeps going.

The game declares what a mod may call and what it must export as a WIT world in its own repository. pfx knows nothing of any game's world: it hosts whatever the game hands it, plus one optional import of its own, a rate-limited log line.

## The security model

- **No WASI.** The crate does not link `wasmtime-wasi`. A mod gets no files, network, clock, environment, arguments, stdio or host randomness. Its only imports are the interfaces the game lists in its `Api` and registers in the host's linker, and `pfx:mods/log` if the game turns the log on.
- **Imports are checked twice at load.** Every import of the component must be a name the game listed (`Api::imports`, plus the log interface when it is on); anything else is refused by name. Then the linker type-checks each import against what the game registered, so an import with the right name and the wrong signature is refused too.
- **One Store per mod.** Each mod runs in its own wasmtime `Store`, with its own fuel, its own `StoreLimits` and its own copy of the game's per-mod state `T`. Mods never share memory or tables, with each other or with the game.
- **Fuel.** Every wasm instruction costs fuel. The game picks a budget per call or per tick (`Fuel::PerCall` or `Fuel::PerTick`). A mod that runs out traps at once.
- **Memory and tables.** `StoreLimits` caps a mod's linear memory in bytes, its table elements, and its counts of instances, tables and memories. Growth past a cap traps instead of returning -1, so a mod cannot probe the cap and carry on.
- **Call depth.** Every function of every core module counts its depth in a global the host adds at load, and a call past `Limits::depth` traps with `Cause::Depth` at the same instruction and fuel on every platform ("The call-depth limit" below).
- **Stack.** `Config::max_wasm_stack` stays as the safety net above the depth limit; overflow is a trap, never a crash of the game.
- **Start functions.** A component-level start function is refused at load. A core module's start function inside the component runs at instantiation under the same fuel budget and limits as a call, so it cannot hang the game or grow past its caps either.
- **Sizes.** A manifest over `Limits::manifest_bytes` (16 KiB) or a component over `Limits::component_bytes` (16 MiB) is refused before it is parsed. The loader reads at most one byte past a cap.
- **Core modules are refused.** Only components load, so every boundary is typed by the world.

### When a mod fails

A mod that runs out of fuel, calls past its depth limit, traps, overflows its stack, grows past a cap, or gets an error from a game import is stopped: its instance is dropped and every later call returns `Cause::Disabled` for the rest of the run. The call returns a `ModError`:

| field | meaning |
| --- | --- |
| `id` | the mod's id |
| `cause` | `Fuel`, `Depth`, `Stack`, `Trap(wasmtime::Trap)` (unreachable, divide by zero, out-of-bounds access and the rest), `Limit(Limit::Memory { bytes })` or `Limit(Limit::Table { elements })`, `Error(text)` (a game import's error, a failed bind) or `Disabled` |
| `fuel` | the fuel the call had used when it stopped |
| `at` | the trap point: the core function index and the byte offset in its module, when wasmtime knows it |

Its `Display` reads, for example, ``mod `greedy` ran out of fuel after 1000000 fuel in function 7 at offset 0x2c9; it is stopped and disabled for this run``. The game's own state for that mod (`Mod::game`) survives the stop. Other mods keep running.

## The limits

`Limits::default()`:

| limit | default |
| --- | --- |
| `fuel` | `Fuel::PerCall(10_000_000)` |
| `memory_bytes` | 64 MiB |
| `table_elements` | 10,000 |
| `instances`, `tables`, `memories` | 32, 16, 4 |
| `stack_bytes` | 512 KiB |
| `depth` | 1,000 nested calls |
| `component_bytes` | 16 MiB |
| `manifest_bytes` | 16 KiB |
| `log` | `None` (off) |

With `Fuel::PerCall(n)` each call starts with `n`. With `Fuel::PerTick(n)` the game calls `ModSet::tick()` (or `Mod::tick()`) at the start of each tick to refill to `n`, and every call in the tick draws from it. Instantiation starts with one budget. `Called::fuel` is what a call used, and `Mod::fuel_left()` what remains.

The stack limit is engine-wide, so it lives in `Limits` with the rest and is set when the `Host` is made. Keep it well under the stack of the thread that calls mods (Rust's spawned threads get 2 MiB), and well above `depth` times the largest native frame a mod's functions compile to, so the depth limit always trips first.

## The call-depth limit

wasmtime's stack check measures from a host frame, and the host frames below the first wasm frame differ in size between Linux and Windows and between debug and release builds, so the exact depth at which a recursion overflows differs too: under Wine a test's overflow used 43,536 fuel against 43,592 on Linux. A mod recursing to within a few frames of the stack could succeed on one build and fail on another, and its side effects would diverge.

So the host counts depth itself. At load, before compiling, it rewrites every core module inside the component with wasmparser and wasm-encoder (`crates/mod/src/depth.rs`), keeping the component's types, imports, exports and every index:

- it adds one mutable `i32` global, the depth, and one function whose body is `unreachable`, each at the end of its index space, so no existing index moves;
- every function starts by trapping through that function when the depth has reached `Limits::depth`, then adds one;
- every way out subtracts one: the end of the body, `return`, `return_call` and `return_call_indirect` (before the call, so a tail call does not grow the depth), and `br`, `br_if` and `br_table` to the function's own label (`br_if` and `br_table` through a scratch local the rewrite appends, subtracting only when the branch is taken);
- a module with no functions of its own is left as it is.

A call that would nest deeper than `depth` frames of one core module traps there, and the host reports `Cause::Depth` ("called deeper than its call-depth limit"), with `at` naming the function that tried to go deeper and the offset of its check. The host recognises its own trap by the trap function's index and its byte offset in the rewritten component, which no code of the mod can share. Fuel counts the added instructions like any others, so the trap point and the fuel at it are the same bits natively and under Wine; the determinism golden holds both. The count is per core module: a component whose modules call each other may nest `depth` frames in each.

The exceptions proposal is off as well (`WasmFeatures::EXCEPTIONS` joins the disabled features), since a throw would leave the frames it unwinds counted. Fuel figures and trap offsets are those of the rewritten modules; the fingerprint is still over the bytes the mod shipped.

## The world

The game writes a WIT world in its own repository: its imports in interfaces, its exports as functions or interfaces. The neutral test world in `crates/mod/tests/wit/play.wit` shows the shape:

```wit
package pfx-test:play@0.1.0;

interface game {
  seed: func() -> u64;
}

world play {
  import game;
  import pfx:mods/log@0.1.0;

  export bump: func(by: u32) -> u32;
  export roll: func() -> u64;
  export mix: func(a: f64, b: f64, steps: u32) -> f64;
}
```

- **Imports go in interfaces,** so the game can register each interface's `add_to_linker` on its own and leave `pfx:mods/log` to the host.
- **Randomness reaches a mod only through the world.** `seed` above is the game's own seeded RNG (`pfx_core::sim::Rng`), forked per mod as the game likes; a mod has no other source.
- **The log** is `pfx:mods/log@0.1.0` with one function, `line: func(text: string)`. Its WIT is `crates/mod/wit/log.wit` (also `pfx_mod::LOG_WIT`); a game that imports it copies that file into its own `wit/deps/pfx-mod/`. With `Limits::log = Some(LogLimits { lines_per_tick, line_bytes })` each mod keeps up to `lines_per_tick` lines per tick, each cut to `line_bytes` on a character boundary; the rest are counted in `Mod::dropped_lines()`. The game drains lines with `Mod::take_log()`. With `log: None` the interface is not registered and a mod that imports it is refused.

The game generates bindings with wasmtime's `bindgen!` through pfx-mod's re-export, so both sides use the same wasmtime:

```rust
pfx_mod::wasmtime::component::bindgen!({
    path: "wit",
    world: "play",
    wasmtime_crate: pfx_mod::wasmtime,
});

struct Game { rng: pfx_core::sim::Rng }

impl my_game::play::game::Host for Game {
    fn seed(&mut self) -> u64 { self.rng.next_u64() }
}

let api = Api::new("play", "0.1.2").import("my-game:play/game@0.1.0");
let mut host = Host::<Game>::new(api, Limits { log: Some(LogLimits::default()), ..Limits::default() })?;
my_game::play::game::add_to_linker::<_, HasSelf<Game>>(host.linker(), |cx: &mut ModCtx<Game>| &mut cx.game)?;

let loaded = host.load_dir(Path::new("mods"));
for refused in &loaded.refused { eprintln!("{refused}"); }
let (mut mods, failed) = ModSet::start(&host, loaded.packages, |s, i| Play::new(s, i), |_| Game { rng: run_rng.fork(MOD_STREAM) });

mods.tick();
for m in mods.mods_mut() {
    match m.call(|play, store| play.call_bump(store, 1)) {
        Ok(called) => { /* called.value, called.fuel */ }
        Err(error) => eprintln!("{error}"),
    }
}
```

Register every import before loading: `Host::load` type-checks against the linker as it is then.

## Loading

Mods live in a content folder, one folder per mod, named after its id:

```
mods/
  counter/
    mod.toml
    counter.wasm
```

`mod.toml` has exactly four fields; any other is refused:

```toml
id = "counter"          # 1 to 64 of a-z, 0-9, - and _, starting with a letter or digit; the folder's name
version = "1.0.0"       # the mod's own version, semantic
api = "0.1.0"           # the version of the game's mod API it was made for, semantic
entry = "counter.wasm"  # a file name beside mod.toml, ending in .wasm; no folders
```

`Host::load_dir(mods)` loads every subfolder in id order (sorted, never directory listing order) and returns `Loaded { packages, refused }`. A missing `mods` folder is an empty set. Each refusal is a `LoadError` naming the folder, the id when it got that far, and a `Refusal` whose message says what is wrong and what is wanted. `Host::load_folder` loads one folder, and `Host::load(manifest, component)` loads from bytes.

The API version follows caret rules against the game's `Api::version`: a mod made for 1.2.0 runs on a game at 1.4.2 but not at 1.1.0 or 2.0.0; under 1.0, the minor is the major (0.3.0 runs on 0.3.4, not on 0.4.0). A mismatch reads, for example:

```
mod `future` (mods/future) was made for version 0.2.0 of the game's `play` mod API, but this game has 0.1.2; it needs a mod made for a version compatible with 0.1.2
```

Steam Workshop items will land in this folder later; the Workshop calls stay in the Steam plan.

## Fingerprints

Each package has a `Fingerprint`, a sha256 over the manifest's bytes and the component's bytes, each length-prefixed (`b"pfx-mod 1\0"`, then the manifest's length as a little-endian u64 and the manifest, then the same for the component). Changing either changes it; loading the same bytes from a folder or from memory gives the same one.

`ModSet::fingerprint()` is a sha256 over the set's ids and fingerprints, sorted by id, so it does not depend on load order. A mod stopped during the run still counts: it ran. The empty set has its own fingerprint, and `ModSet::is_empty()` says a run was unmodded. A run's record stores the set's fingerprint (`Fingerprint::hex()`), so a game can keep modded runs off its boards or keep separate boards, and the Steam crate's checked runs can carry it.

## Determinism

The same mods, given the same calls and the same game state, give the same bits, the same fuel and the same trap points on every supported platform:

- **NaN canonicalisation** is on (`cranelift_nan_canonicalization`), so every float operation that makes a NaN makes the canonical one (`0x7ff8000000000000`, `0x7fc00000`), whatever the payload or the CPU's default NaN. Without it, x86 returns `0x7ffc000000000001` for a signalling NaN times one; the determinism test catches that.
- **Nondeterministic proposals are off**: threads and shared-everything threads, relaxed SIMD (and `relaxed_simd_deterministic` is set should it ever be on), memory64, custom page sizes, memory control, GC and function references, stack switching, legacy exceptions, custom descriptors, and the component model's async, threading, error-context, GC and 64-bit options (`pfx_mod::disabled_features()`). The cargo features that would allow some of them (`threads`, `async`, `gc`) are not compiled in either. Plain SIMD stays on: its lanes are IEEE operations, and canonicalisation covers its NaNs.
- **Fuel counts wasm instructions**, not time, so it does not depend on the CPU, the OS or the load. Out-of-fuel traps land at the same instruction everywhere.
- **No clock and no host randomness** reach a mod; time and seeds come from the game's imports.
- **Call depth is counted, not measured.** A recursion stops at `Limits::depth` with `Cause::Depth` at the same instruction and fuel everywhere ("The call-depth limit" above). wasmtime's own stack limit stays beneath it as a safety net; an overflow there (`Cause::Stack`) only happens when `stack_bytes` is too small for `depth`, and its exact point is not held to be the same across platforms, so the test checks only its cause.
- **Basic float operations** (`+ - * /`, `sqrt`, min, max, conversions) are IEEE-exact in wasm. A mod written in a language that pulls a libm into its wasm carries its own libm, so its transcendentals are identical on every host too.

`crates/mod/tests/determinism.rs` runs the test world's component through float-heavy loops (f64 and f32), NaN-producing operations on signalling, negative, payload and plain inputs, the game's seeded RNG, a fuel-exhausting loop, an unreachable trap nine calls deep, a memory grow past the cap, a recursion one frame inside the depth limit (it reaches its own `unreachable`), one at the limit and one without end (both `Depth`, with the same fuel and trap point), and, with the depth limit lifted, a stack overflow. It writes each value's bits, the fuel used and the trap point (for the overflow, its cause only), and compares them line by line with `crates/mod/tests/golden/determinism.txt`. pfx's Windows phase (`bin/windows-check test`) runs the same test under Wine, so the Linux and Windows builds are held to the same file; on aarch64 it runs once a Mac is here. Fuel figures change with a wasmtime upgrade, so a wasmtime bump re-blesses the golden (`PFX_MOD_BLESS=1 cargo test -p pfx-mod --test determinism`) in its own reviewed commit.

The test components are written in the component text format in `crates/mod/tests/fixtures/*.wat` and built from source in the test with the `wat` crate, so no binary fixture is committed.

## Swapping a module

`Mod::swap(&host, package)` replaces a mod's component between calls and keeps the game's state for it (`Mod::game`). It also restarts a stopped mod. The new package must keep the mod's id; if it fails to start, the mod is stopped with that error.

`Mod::reload(&host, package)` is the same swap with the module's own state carried across, through an explicit export pair the module may have beside its world:

```wit
export save: func() -> list<u8>;
export restore: func(state: list<u8>);
```

The host looks both up by name on the running instance, so a game's world may declare them or not. `reload` calls the old module's `save`, swaps, and calls the new one's `restore` with the bytes, each under one call's fuel budget, and returns `Swapped`:

| `Swapped` | when |
| --- | --- |
| `Kept { bytes }` | both exports ran; the state crossed |
| `Fresh(Fresh::NoSave)` | the old module has no `save`; the new one starts clean |
| `Fresh(Fresh::NoRestore)` | the new module has no `restore`; it starts clean |
| `Fresh(Fresh::Mismatch { export })` | `save` or `restore` has another type; it starts clean |
| `Fresh(Fresh::Stopped)` | the old module was stopped, so there was nothing to save |
| `Fresh(Fresh::Save(error))` | `save` trapped (the old module is stopped with that error); the new one starts clean |

A `restore` that traps or runs out of fuel stops the new module and `reload` returns its `ModError`. Its `Display` reads, for example, `kept its state (4 bytes through save and restore)` or ``restarted clean: the old module exports no `save` ``.

## Gameplay modules

A game's own logic can live in WASM components it builds from its own repo, hosted the same way as mods: no WASI, fuel, caps, the depth limit and NaN canonicalisation. `pfx_mod::Modules<T, B>` (re-exported by pfx-game's `modules` feature as `pfx_game::modules`) holds them and swaps them live.

- **Which modules.** The project names them in `project.toml`, `[authoring.pfx] modules = ["target/wasm32-unknown-unknown/release/brain.wasm"]`: the built files, relative to the project's folder (`docs/projects.md`). A module's id is its file's stem. `pfx_project::Project::modules()` lists them.
- **Loading.** `Host::load_module(id, bytes)` loads one with no `mod.toml`: the API version is the game's own, and the fingerprint is over the bytes. It takes a component, or a core module that carries its component type, the output of `cargo build --target wasm32-unknown-unknown` on a crate using wit-bindgen: the host makes it a component itself with wit-component, as `wasm-tools component new` does. A core module without a component type is refused (`Refusal::Module`). Rust's `wasm32-wasip2` target is not usable: its standard library imports WASI, which the host refuses.
- **Opening.** `Modules::open(host, bind, files, make)` takes the game's `Host` (with its imports registered), its bind function, the `(id, path)` pairs and a closure making each module's game state `T`. A file that is missing or does not load leaves that module off, with a notice, and the game runs without it.
- **Each tick.** The game calls `modules.tick()` first in `Game::tick`. That is the tick boundary: queued swaps happen there, then each module's fuel and log rate are refilled. Then `modules.each(|id, bindings, store| …)` calls every running module in the project's order through the game's world and returns what succeeded; `modules.call(id, …)` calls one.
- **Failures.** A call that traps, runs out of fuel or passes a cap stops that module for the run with its typed `ModError`; the others carry on, and so does the game.
- **Swaps.** `modules.swap(id, bytes)`, or `Reload::changed(path)` (which reads the file), queues a new build; it is loaded and swapped in at the next `tick()` through `Mod::reload`, so state crosses through `save` and `restore` and a module without them restarts clean. A build with the same bytes as the running one is not swapped. A build that does not load leaves the running module as it was. A module that was never running starts.
- **Notices.** Each of these is an `Event { tick, id, what }` (`What::Started`, `Missing`, `Refused`, `Swapped` or `Failed`) with a level and a line, such as ``module `brain` swapped at tick 412 and kept its state (4 bytes through save and restore)``. `take_events()` drains them typed; `Reload::notices()` drains them as `pfx_core::modules::Notice` lines.
- **The editor's view.** `Modules` implements `pfx_core::modules::Reload` (`files`, `changed`, `notices`), and a game hands it over through `Game::modules()` (pfx-play), which returns `None` by default. pfx-game's runtime prints the notices to stderr each frame; `pfx edit` and `<game> edit` show them in the log and watch the files while the game plays (`docs/projects.md`, "Live reload").

No Rust is hot-patched anywhere: a swap replaces a sandboxed component, and the game's own binary never changes while it runs.

## Tests

- `crates/mod/src/tests.rs`: manifests, API versions, fingerprints, the log's rate and the host's refusals.
- `crates/mod/tests/host.rs`: loading a folder, refusals, limits, traps, start functions, swaps and, at the depth limit and with it lifted, `Depth` and `Stack`.
- `crates/mod/tests/determinism.rs`: the golden, natively and in the Windows phase.
- `crates/mod/tests/reload.rs`: a swap keeping state through `save` and `restore`; restarting clean without them; a failing `restore` stopping the module and a fixed build starting it again; the depth limit's boundary; every way out of a function (`br_if`, `br_table`, `return`, `return_call`, the end) leaving the count balanced over 600 calls, in a module with no globals of its own; a core module with its component type loading as a gameplay module; and `Modules`: loading from files, a swap only at a tick boundary, the same bytes not swapping, a clean restart with its notice, a failing module disabled while the others run, a build that does not load leaving the running one, and a missing module starting once its file appears.

```
cargo test -q -p pfx-mod
PFX_MOD_BLESS=1 cargo test -p pfx-mod --test determinism
bin/windows-check test
```
