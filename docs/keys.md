# Keys and the help sheet

`pfx-keys` (`crates/keys`) is one keymap per app or window. An app declares its keys once; from that one declaration come the action a key does, the `?` help sheet's model and the short hints a legend shows, so the sheet, the legend and what a key really does never disagree. It draws nothing: each app draws the sheet and the hints in its own look. The pfx editor and the game window's built-in keys declare theirs with it, and any app can depend on it, with or without the rest of pfx.

A game's own controls stay in `pfx-input`'s action map ([input.md](input.md)): `pfx-keys` is for the keys a person reads on a help sheet, such as an editor's or a tool's shortcuts.

## Take it

```toml
[dependencies]
pfx-keys = { git = "https://github.com/gmrdad82/pfx", tag = "<tag>", features = ["egui"] }
```

Put the pfx tag your project pins in place of `<tag>`. With no feature it depends on nothing. Each feature adds one converter from a toolkit's key events to its chords:

| feature | converter | for |
| --- | --- | --- |
| `egui` | `pfx_keys::egui` (egui 0.36.2) | an egui window: `pressed(&events)`, `chord(key, modifiers)` (also `Chord::from_egui(key, modifiers)`), and `events(chord)` / `name(chord)` for scripted input |
| `winit` | `pfx_keys::winit` (winit 0.30.13) | a winit window: `chord(&logical_key, modifiers)` |
| `terminal` | `pfx_keys::terminal` (crossterm 0.29.0) | a terminal app: `chord(&key_event)`, also `Chord::from_crossterm(&key_event)` |

## Declare the keymap

```rust
use pfx_keys::{Keymap, Mode, When, bind};

const LIST: Mode = Mode("list");
const PAGE: Mode = Mode("page");
const PICKED: When = When("while a row is picked");

let keys = Keymap::new()
    .modes(&[LIST, PAGE])
    .group("List", [
        bind("up down", "steps through the rows", Act::Step).on(LIST).shown("up and down").rank(1),
        bind("enter", "opens the row", Act::Open).on(LIST),
        bind("delete", "removes it", Act::Remove).on(LIST).when(PICKED),
    ])
    .group("Edit", [
        bind("ctrl+z", "undoes", Act::Undo).note("as often as you like"),
        bind("ctrl+shift+z ctrl+y", "redoes", Act::Redo),
        bind("esc", "goes back", Act::Back).on(PAGE),
    ]);
```

- **Group**: a section of the sheet, in the order declared.
- **bind(keys, does, action)**: `keys` is one or more chords, space-separated (`"ctrl+z"`, `"up down"`, `"shift+space"`, `"f2"`, `"?"`). A misspelt key panics at declaration with `pfx-keys: 'ctlr+z' is not a key`, so any test that builds the keymap catches it. `does` is the one phrase both the sheet and the legend show.
- **.on(mode) / .modes(&[..])**: the modes or screens it answers in; none means everywhere. **.when(context)**: only while that context holds; a contextual bind declared before a general one on the same key overrides it while the context holds.
- **.shown(text)** writes the keys differently (`"up and down"`); otherwise the chords are written `ctrl+z`, `shift+a`, `space`, `pgdn`, `f2`, `cmd+…` on macOS and `super+…` elsewhere, joined by `" / "` (`.joiner(..)`).
- **.note(text)**: longer words shown only on the sheet. **.rank(n)**: the legend's priority, 0 first. **.quiet()**: on the sheet only, never in the legend.
- **Now**: where the app is: `Now::new(LIST).when(&[PICKED])`.

## What it answers

| call | gives |
| --- | --- |
| `keys.action(now, chord)` | the action that key does here, the first live bind in declared order |
| `keys.hints(now)` | the live hints for a legend, by rank: `Hint { keys, does, rank }`, `"? help"` among them |
| `keys.ticker(now)` | a `Line` per mode, the current one first (all its live hints), then the others' plain hints; `line.to_string()` joins them as `"<keys> <does> · <keys> <does>"` |
| `sheet.view(&keys, now)` | the sheet: `here` (the current mode's binds, grouped, `live` false for a context that doesn't hold) then `elsewhere` (the rest, grouped, with their modes), filtered by the search |
| `keys.clashes()` | binds that answer the same chord in the same place, where the first silently wins; assert it empty in the app's tests |
| `keys.button()` | the help button's words, `"? Help"` |

Every string is borrowed from the keymap; keep the keymap in a `static` (`LazyLock`) and they are `&'static str`.

The keymap does its work once, when it is declared: each bind's written keys and search text, and each mode's hints in rank order. `action` is one pass with no allocation, `ticker` and `hints` copy the ready lists, and the sheet's search keeps its first 64 characters and its first 8 words, so a pasted page can't stall a frame.

## The help sheet

`Sheet` holds the conventions, so every app behaves the same:

- `?` opens and closes the sheet; Esc closes it, first clearing a search; `/` searches it (typing goes to the search, Enter keeps it, Esc drops it).
- It never steals focus: `sheet.key(chord)` returns `Outcome::Took` only for those keys and `Outcome::Pass` for every other, which the app handles as if the sheet were closed. Opening it changes no selection, mode or playback.
- A visible Help or `?` button calls `sheet.toggle()`.
- The sheet lists its own keys in a last group, `Help`; `Words` changes the wording to the app's voice.
- The app calls `sheet.key` only when it isn't typing into its own text field, so a `?` typed there stays text.

Each frame, for each key event:

```rust
let Some(chord) = pfx_keys::terminal::chord(&event) else { return };
if sheet.key(chord) == Outcome::Took {
    return;
}
if let Some(act) = keys.action(now, chord) {
    // do it
}
```

## Chords

`Chord { key, mods }` is the crate's own. A `Char` carries shift in its case (`shift+a` is `Char('A')`), so `?` is `?` however the platform reports it, and `Char(' ')` is `Space`. A terminal's back-tab reads as `shift+tab`, and a key release reads as no key. The egui converter reads command and macOS cmd as `ctrl`, so one `ctrl+s` bind answers on every platform; the winit converter reads cmd as `ctrl` on macOS and super as `super` elsewhere, and the terminal converter reads super as `super`. Any other toolkit maps in a few lines with `Chord::new(Key::…, Mods::…)`.

The model was first written as a separate MIT-licensed library and ported into this crate ([NOTICE.md](../NOTICE.md)).
