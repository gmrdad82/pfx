# The rig: the live window, hidden

`bin/rig` runs `pfx gui` in a hidden compositor on wlroots' headless backend, takes screenshots of it, types into it and closes it, so a test or a script can drive the live window without it ever appearing on a desktop. For a frame with no compositor at all, `pfx gui --shot` is quicker (`docs/editor-shots.md`); the rig is for what only a live window shows: keys, focus, the pointer, the real swapchain.

It needs `cage` or `labwc`, `grim`, `wtype`, `unshare` with user namespaces, and `wlrctl` for the pointer.

## What `up` sets up

- A rig folder `tmp/rig-<name>` in the repo, never a path outside `tmp/`. The name is letters, digits, dot, dash and underscore.
- A copy: the folder of the file it opens is copied into `tmp/rig-<name>/scene/`, so the tracked files never change, whatever the window writes.
- A private `/tmp`: the compositor and the window run under `unshare -Urm` with `tmp/rig-<name>/tmp` bind-mounted over `/tmp`, so nothing of the rig reaches the host's `/tmp`.
- Isolation: `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME` and `XDG_DATA_HOME` under the rig (so the recent list and the start copy are the rig's own), `PFX_EDIT_PGPU=off`, and the sound muted on three legs: `PIPEWIRE_RUNTIME_DIR=/nonexistent`, `PULSE_SERVER=/nonexistent` and `ALSA_CONFIG_PATH` at a null config in the rig.
- At most `RIG_SLOTS` compositors at once on the machine (4 by default); `up` waits up to `RIG_SLOT_WAIT` seconds (600 by default) for a slot, then refuses.
- It records the compositor's, the window's and the launcher's pids, and the shell's own `WAYLAND_DISPLAY`. Every process is found through the `RIG=<folder>` in its environment, never by name alone, and every verb but `up` refuses a caller whose `WAYLAND_DISPLAY` differs from the one recorded, so a call from inside a rig never reaches a desktop by mistake.

## The verbs

```sh
build-step pfx-build tmp/build.log -- 'cargo build --release --bin pfx'
bin/rig up neutral edit                 # the engine's neutral fixture scene; or a path to a scene or a recipe
bin/rig keys edit -k b -k 1             # focus B1 in one call
bin/rig shot edit outliner              # tmp/rig-edit/tmp/outliner.png
bin/rig down edit
```

- `bin/rig up <file|neutral> <name>` runs `target/release/pfx gui <copy>` under `cage`, or under `labwc` with `RIG_COMPOSITOR=labwc`. It refuses a path that is not a file, a missing release binary or one older than any file under `src/` or `crates/` (build it first), and a rig already up under that name.
- `bin/rig shot <name> <file>`: `grim` from the rig's socket into `tmp/rig-<name>/tmp/<file>.png`.
- `bin/rig keys <name> <wtype args…>`: one `wtype` call. A panel code is a chord: give both keys in one call (`-k b -k 1`), since the window reads any other key as the end of a pending chord. Give the window a moment before the next `shot`; it redraws on input and idles at zero frames, so a shot taken right after `keys` can be a step behind.
- `bin/rig pointer <name> <wlrctl pointer args…>`: labwc only (cage has no virtual pointer); moves by a difference, clicks and scrolls.
- `bin/rig down <name>`: the patient teardown: the window first, waited for, then the compositor and the launcher. A `kill -9` on a live window makes cage abort with a crash card on the host desktop, so never stop a rig any other way.
- `bin/rig ls`: the live rigs.

Run a rig in one step so `down` always runs:

```sh
bin/rig up samples/workshop.pitofx w2 && bin/rig shot w2 first && bin/rig keys w2 -k b -k 1 && bin/rig shot w2 second; status=$?; bin/rig down w2; exit $status
```

## When a rig fails

Say which process died, if one did (the window, the compositor, a tool), whether the window came up, and whether the fault is the window's or the rig's, before any retry.

- `cage did not come up` or `labwc did not come up`: a rig fault before the window booted; `tmp/rig-<name>/rig.log` holds the compositor's output.
- The window did not come up, or its pid is not running: the window's fault. `rig.log` holds its stderr; read it for a panic or a wgpu error.
- A refused caller Wayland display: the call came from another shell than `up`'s. Run it from the same shell.
- `build first` or `is older than`: the release binary is missing or stale; run the build step.
- `grim` fails or writes nothing: the compositor has no output yet or the window has not drawn; take the shot again after a second.
- Captures and logs live under `tmp/rig-<name>/`, which git ignores.
