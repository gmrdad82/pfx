# User stories

`tests/stories/` holds pfx's user-story suite: each story is one path a user takes with pfx from start to end, through the `pfx` binary as it ships or through the public crates a game links, with its happy path and its bad paths. A story checks what the user would see and keep (files written, values read back, the one line a refusal prints, the exit code, the JSON keys other tools read), not how the code inside gets there.

Every feature path a user takes has a story for its happy path, one for each of its bad paths, and a check of what it produces. Each story also names, in one line, what would hurt a user if it broke, which ranks the stories by what matters most.

## Run it

```sh
cargo nextest run --test stories
cargo test --test stories
```

The stories that draw (live and traced renders, the editor's offscreen frames) need a GPU and are ignored by default with the reason `needs a GPU; run under pgpu`. Run them on a machine with a GPU:

```sh
cargo test --test stories --no-run
cargo test --test stories -- --ignored --test-threads=1
```

Every other story is portable and runs on Linux, macOS and Windows.

Filter by area with the module name, for example `cargo nextest run --test stories licence::`.

## Areas

| Module | What the user does |
|---|---|
| `scaffold` | `pfx new` a game and build it, `pfx new --from` a template, `pfx git setup` and a `git diff` of a `.pitofx` edit |
| `file` | open, print, list, write, save, fold and compact a `.pitofx`, migrate or fix an older one, and have a truncated, corrupt, foreign or missing file refused in one line |
| `edit` | edit a document from the shell: `set` and `get`, `add`, `remove` and `duplicate`, `key` and `unkey`, routes, curves, `align`, `timeline`, `check`, `build`, `diff`, glTF import and export, the object library and credits |
| `render` | live and traced stills and clips, shots, films, benches, bakes and jobs, the same frames on every run, a stopped render leaving no half output, a render past `--memory-limit` refused before it starts, a bad scene refused with its cause |
| `editor` | edit, undo, redo and save through the editor's offscreen frames, never a window |
| `licence` | Free, Indie and Pro keys and their groups, who may hide the Powered by pfx splash, the chime on its line at its own level, a bad or expired key |
| `cli` | clap's usage errors, one-line refusals, `--help` and `--version`, settings, the cache, fonts, assets, Steam staging, and the `--json` keys other tools read |
| `sound` | `pfx sound mix` and `clean`, and the composer: compose a cue, list, print and render it, and derive a style from music |
| `game` | what a game linking pfx's crates does: a headless run, input recording and rebinding, physics, the sound mixer, loading a scene, settings and gameplay modules within their limits |

## How a story is written

- The table in `tests/stories/table.rs` lists every story: its area, its test name and the one line of what it guards. A story's test is named `story_<area>_<what the user does>` and lives in its area's module.
- A test `every_story_is_one_test_in_its_area_and_every_story_test_is_in_the_table` reads the table and the test binary's own list of tests, and fails on a story test the table doesn't list, a test in the wrong area, an ignored test that isn't a GPU story or a GPU story that isn't ignored, an ignore reason other than `needs a GPU; run under pgpu`, and the wall clock, a sleep or system randomness in a story. Once `ALL_WRITTEN` is set it also fails on a story in the table with no test.
- `tests/stories/kit.rs` runs `pfx` inside a sandbox under the tree's `tmp/stories/<story>/`: a private HOME, XDG and app-data folders, a private 0700 runtime folder, no display variables, no colour, the machine cache off, git with an empty global config and a fixed author and date, and stdin closed unless a story pipes something in. It refuses a call with no verb, a lone document path, or the editor without `--shot` or `--clip`, because each of those opens a window.
- Stories are deterministic: no sleeps, no wall clock, fixed seeds and dates from the kit (`SEED`, `BUILD_DATE`, `EPOCH`). Fixtures are the engine's own tracked samples and fixtures, or generated in the story itself; a GPU story may also take a media set materialized into its sandbox. A story never reads another project's files.
- Licence stories sign their own keys with a fixed test key pair and check them against its public key through `pfx_splash`, so no real key is needed.
