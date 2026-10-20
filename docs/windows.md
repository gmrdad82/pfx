# Windows

Steam ships one Windows build. Linux and the Steam Deck run that build through Proton. A native Linux build stays for development and tests.

The whole workspace builds for Windows, the `pfx` tool included: `bin/gate windows-lint` runs clippy over every crate and target on Windows, and `bin/gate windows` builds the workspace and runs every crate's unit tests there, then the game crates' integration tests. Each tag's GitHub Release carries `pfx-vX.Y.Z-windows-x86_64.zip`, which `bin/build-release <tag> windows-x86_64` cross-compiles on Linux with mingw-w64 (target `x86_64-pc-windows-gnu`, linked with `-static`, so no DLL ships beside it); [install.md](install.md) installs it and registers the `.pitofx` file type.

What the tool does differently on Windows:

- The GPU queue, pgpu, is Linux only. With no `pgpu` on `PATH`, renders, bakes and benches run directly.
- A stop ends a render's child process (`Child::kill`) instead of sending it SIGTERM, and the processes below a queue wrapper are not looked up (that reads `/proc`).
- The archive lock falls back to the temporary folder when `XDG_RUNTIME_DIR` is unset, and copies are plain copies (no reflink).
- Windows ships two binaries, a `.com` and an `.exe` of one name. `pfx.com` is the console tool, which a terminal runs for `pfx` because `.COM` comes before `.EXE`. `pfx.exe` is a window-subsystem launcher built from `crates/launcher`, carrying `pfx.ico` as its icon: it starts `pfx.com gui <arguments>` with no console window and returns. `pfx.com` hands a bare call or a lone `.pitofx` to `pfx.exe`, detached ([install.md](install.md), "How pfx starts").
- Tests that drive a `#!/bin/sh` stand-in, symlinks or Unix signals are `cfg(unix)` or `cfg(target_os = "linux")`, so the Windows build skips them.

## What builds

The game crates `bin/windows-check` covers: `pfx-core`, `geom`, `gpu`, `materials`, `text`, `post`, `load`, `physics`, `sound`, `trace`, `bake`, `live`, `input`, `steam`, `mod`, `play` and `film`. The full gate builds the rest of the workspace for Windows as well.

wgpu is pinned at 30.0.1, with `default-features = false`, and the features are chosen per target:

| Target | gpu, live, post, trace | physics, and the materials tests |
|---|---|---|
| Windows | `dx12`, `vulkan`, `wgsl` | `std`, `wgsl`, `dx12`, `vulkan` |
| Linux and every other target | `vulkan`, `metal`, `wgsl` | `std`, `wgsl`, `vulkan` |

Linux keeps the features it had. On Windows the openers (`Gpu::headless`, `Gpu::window` and the checked variants) take a Vulkan adapter first and use DX12 only when no Vulkan adapter is found. DX12 stays compiled in for a Windows machine with no usable Vulkan driver. Under Proton, Vulkan passes through winevulkan to the host driver, and DX12 goes through vkd3d-proton, so Vulkan is the path on the Deck. Elsewhere the openers request Vulkan and Metal, and they still take the first discrete adapter. A floor refusal names the adapter's backend: Vulkan, DX12 or Metal. Which DX12 resource-binding tiers meet the floors is in `docs/gpu-floor.md`. The flat pass needs only `wgpu::Limits::default()`.

These crates have no Linux-only call on the path a game runs. The pidfd ending stays in the harness, behind `cfg(target_os = "linux")`. The live renderer and the tracer take the harness as a dev-dependency for their examples on every target, so their examples build on Windows too.

## How to check

`bin/gate windows` and `bin/gate windows-lint` are the gate on Windows. On a Linux machine, `bin/windows-check` checks the game crates and their tests, target `x86_64-pc-windows-gnu`, with `--lib --tests`. `bin/windows-check test` is the shipped target's CPU check: the same check, then the suite under Wine. Examples stay out of this script; the full gate's `windows-lint` builds them. Ignored tests stay out, and that includes every GPU test. Two pace tests launch a `#!/bin/sh` stand-in for `pgpu` and re-exec a child test. Those four tests are `cfg(unix)`, so this target does not build them.

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

## GPU

GPU tests stay out of the gate's phases and of `bin/windows-check`, so the Windows GPU paths are built but untested on Windows hardware. The Deck-class floor check is the Windows build under Proton on an AMD GPU; DX12 under vkd3d does not count.
