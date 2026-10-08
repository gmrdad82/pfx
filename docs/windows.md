# Windows

Steam ships one Windows build. Linux and the Steam Deck run that build through Proton. A native Linux build stays for development and tests.

The crates a game links typecheck for `x86_64-pc-windows-gnu`. `pfx`, the root package, stays Linux-only, and so do `crates/assets` and `crates/run` (the harness: the pidfd ending, the archive lock).

## What builds

`pfx-core`, `geom`, `gpu`, `materials`, `text`, `post`, `load`, `physics`, `sound`, `steam`, `trace`, `bake` and `live`. `crates/input` is not in this tree. When it appears, the check below includes it.

wgpu is pinned at 27.0.1, with `default-features = false`, and the features are chosen per target:

| Target | gpu, live, post, trace | physics, and the materials tests |
|---|---|---|
| Windows | `dx12`, `vulkan`, `wgsl` | `std`, `wgsl`, `dx12`, `vulkan` |
| Linux and every other target | `vulkan`, `metal`, `wgsl` | `std`, `wgsl`, `vulkan` |

Linux keeps the features it had. On Windows the openers (`Gpu::headless`, `Gpu::window` and the checked variants) take a Vulkan adapter first and use DX12 only when no Vulkan adapter is found. DX12 stays compiled in for a Windows machine with no usable Vulkan driver. Under Proton, Vulkan passes through winevulkan to the host driver, and DX12 goes through vkd3d-proton, so Vulkan is the path on the Deck. Elsewhere the openers request Vulkan and Metal, and they still take the first discrete adapter. A floor refusal names the adapter's backend: Vulkan, DX12 or Metal. Which DX12 resource-binding tiers meet the floors is in `docs/gpu-floor.md`. The flat pass needs only `wgpu::Limits::default()`.

These crates have no Linux-only call on the path a game runs. The pidfd ending stays in the harness. The live renderer and the tracer depend on that harness only off Windows, as a dev-dependency for their examples, so this check does not compile `crates/run`.

## How to check

`bin/windows-check` checks those crates and their tests, target `x86_64-pc-windows-gnu`, with `--lib --tests`. `bin/windows-check test` is the shipped target's CPU check: the same check, then the suite under Wine. Examples stay out, because they link the harness. Ignored tests stay out, and that includes every GPU test. Two pace tests launch a `#!/bin/sh` stand-in for `pgpu` and re-exec a child test. Those four tests are `cfg(unix)`, so this target does not build them. `crates/input` is included once `crates/input/Cargo.toml` is in the tree.

The same seed on the native Linux dev build and on the Windows build under Wine or Proton produces the same result. Sim's golden tests are that check.

1. `cargo check`
2. `cargo clippy -- -D warnings`
3. `bin/windows-check test`: `cargo test` under Wine

The test step is one command, so Wine never reads its default prefix and never opens a display; the variables sit inside the command:

```
env -u DISPLAY -u WAYLAND_DISPLAY \
  WINEPREFIX=<worktree>/tmp/wine \
  WINEDEBUG=-all \
  WINEDLLOVERRIDES='mscoree,mshtml=' \
  CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine \
  bin/windows-check test
```

`WINEPREFIX` is the checkout's `tmp/wine`. `WINEDLLOVERRIDES` keeps Wine from offering Mono or Gecko. The script runs `wine wineboot --init` under that same environment before the tests, so the prefix exists before the suite starts. Geometry's `meshopt` build script compiles C++ with `x86_64-w64-mingw32-g++`. After `cargo test --no-run`, the script copies `libstdc++-6.dll`, `libgcc_s_seh-1.dll` and `libwinpthread-1.dll` from `/usr/x86_64-w64-mingw32/bin`, and `steam_api64.dll` from the Steam crate's build output, next to the test executables. Wine loads those from the executable's directory.

## In the full gate

The full gate runs `bin/windows-check test` as its Windows phase: the compile and the CPU suite under Wine. MSVC comes later through cargo-xwin if a release needs it, and a GPU smoke test under Proton later still. The Deck-class floor check is the Windows build under Proton on an AMD GPU; DX12 under vkd3d does not count. GPU tests stay out of this script.
