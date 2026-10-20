# Arrange a scene in the window

`pfx gui` opens a recipe, a `.pitofx` document, in a window drawn by the engine's live renderer. Every edit you make is kept aside, keeping the recipe's comments and formatting, until Ctrl+S writes it into the `.pitofx`; Ctrl+Z puts an edit back. An old `.toml` recipe is imported once into a `.pitofx` beside it, which then opens.

## Open a recipe

```sh
pfx gui desk.pitofx                 # that recipe
pfx gui samples/                    # the picker, on the recipes of that folder
pfx gui                             # the start scene, the window's own mark on a floor grid
pfx gui desk.pitofx --camera top    # that recipe, looking through its camera top
```

The window opens on the first `[[camera]]`; `--camera <name>` opens on that one instead, in the window, a `--shot` or a `--clip`, and refuses a name the recipe or scene doesn't have in one line that lists the ones it has.

Ctrl+O opens the picker over the viewport at any time; Esc closes it. The window watches the recipe and every file it includes: an edit made outside, by an editor or an agent, shows at once.

## Files

The File menu in the top bar opens, saves, imports and exports, through the window's own file picker:

- **Open** (Ctrl+O) takes a `.pitofx` document (a recipe or a scene) or a `.glb`, which opens as a new document holding that model; an old `.toml` recipe or `.scene.toml` is imported once into a `.pitofx` beside it.
- **Save** (Ctrl+S) writes the open `.pitofx`. A recipe `.pitofx` keeps its edits aside until you save, and shows ● after its name, in the top bar and the window's title, until then.
- **Save As .pitofx** (Ctrl+Shift+S) packs the recipe and the files it uses into one `.pitofx`.
- **Import glTF** adds a `.glb` to the scene as a new mesh object at the origin; in a `.pitofx` the file goes inside the document.
- **Export glTF** writes the scene, or the selected object, as a standalone `.glb` for other tools.

Drop a file on the window to open it, or hold Shift as you drop a `.glb` to import it. The recent list keeps `.pitofx` documents with their thumbnails.


## Move around

Drag in the viewport to orbit, Shift+drag or middle-drag to pan, and the wheel to dolly. `0` looks through the chosen camera again. `7`, `1`, `3` and `9` snap to the orthographic top, front, side and rear views and `5` goes back to perspective; a `[[reference]]` image shows behind the scene in its view, and `i` hides it. A selected generator's curve params show there as point handles to drag, Ctrl+click to add and Delete to remove (see Tracing curves in the editing guide).

## Clay and wireframe

`z` cycles the viewport through shaded, clay and wireframe, and the `clay`, `wire` and `x-ray` buttons at its bottom right switch each one. Clay draws every surface a matte grey under a soft key light and sky, without textures, emission or effects, the selection outlined in the accent colour; `pfx gui --clay` opens in clay. Wireframe draws light edges over a dark ground, the selection in the accent colour and the recipe's routes on top. The flat diagonals of a face are left out and hidden edges are dimmed; Shift+Z shows them brighter, as an x-ray. It works on a recipe, and the window remembers the mode and the x-ray the next time it opens. `pfx gui --wireframe` opens in wireframe.

## Frame rate and VRR

The `vrr` and frame-rate buttons beside them are the window's two pacing switches (`docs/window.md`, "Pacing"). `vrr`, on by default, asks for adaptive sync where the platform offers it; off is steady vsync at the display's refresh. The frame-rate button reads `uncapped` by default, where each frame presents as soon as it is ready; a click cycles it to `60 fps`, `120 fps`, `auto` and back, holding the window to 60 or 120 frames a second, or aiming at 120 and settling lower when the scene can't hold it. Both are shortcuts to the settings panel (`ctrl ,`, [settings.md](settings.md)), which also holds the quality, each effect and the licence. Both switch in place, with no restart, and the window keeps them for next time in its settings (`settings.pitofx`, `[display] vrr` and `frame_rate`). The window is event-driven: it draws when something changes or animates (a drag, an orbit, playback, a progressive render, a play session) and idles at zero frames otherwise. `PFX_PACING=<file>` writes its pacing report (`docs/window.md`, "Pacing report").

## Render scale

The last button on that row is the viewport's render scale. It reads `auto` by default, with the scale it draws at (`auto 100%`, `auto 75%`); a click cycles it to `100%`, `75%`, `50%` and back. Below 100% the live renderer draws the viewport at that fraction of its pixels in each direction and FSR 1 (AMD's edge-adaptive upscale and sharpener, `pfx_live::fsr`) brings it up to the panel's size; the grid, wireframe, reference images and selection are drawn after it, at full size, so lines and text stay sharp. `auto` starts at 100% and lowers the scale in steps of 5% down to 50% when the viewport's GPU time runs over 5 ms a frame, the share of a 120 fps frame (8.33 ms) left after the upscale, the overlays, egui and the compositor; it raises it again once the frame has fit for 60 frames. At 100%, whether chosen or picked by `auto`, the viewport is the same image, byte for byte, as with no scaling at all. The window keeps the choice in its settings (`[display] render_scale`: `"auto"`, `"100"`, `"75"` or `"50"`). Picking reads the scaled image, so a click lands on the part under the pointer at any scale.

## Pick and move a part

- **Pick:** click a part in the viewport, or its row in the outliner. Glass and text pick like any part; the outliner also reaches hidden and shadow-only objects.
- **The gizmo:** `g` move, `r` rotate, `s` scale, with the viewport focused. `l` switches between world axes and the object's own.
- **Drag a handle:** an arrow moves along its axis, a square in its plane, a ring rotates, the centre box scales evenly. Hold Ctrl to snap: 1 mm, 5° or 0.05 of scale. Esc mid-drag cancels.
- **Release** writes one edit: only the key that changed (`at`, `rotate` or `scale`), in the units the recipe uses.

## Edit a route

A recipe's `[[route]]` lines show in the viewport while one is chosen in the outliner (its row under the objects), while an object whose generator names it in `params.route` is selected, or for every route with `p`.

A route with `show = false` never shows. `h` in the viewport hides every overlay of a recipe's view (route lines and handles, guides, grid, gizmo, axes and scale) until it is pressed again, for a clean recording; `pfx gui --clean` does the same for an offscreen take (`--shot`, `--clip` with `--tape`, and `--bare`).

- **Move a point:** drag its disc. It slides on the ground at its own height; hold Shift to move it up and down. The recipe's `points` change live as you drag, and the release is one edit; Esc mid-drag puts it back.
- **Add a point:** Ctrl+click on the line inserts one there.
- **Remove a point:** click its disc, then Delete or Backspace. A route keeps at least 2 points.

## Change the properties

The properties panel shows the selection's name, location, rotation, scale, material and smoothing. A drag on a number commits on release and a typed value on Enter. The material list holds the recipe's materials; typing a new name and pressing Enter writes it. A rename updates every reference to the object in the same edit.

## Cameras

The cameras panel lists the recipe's cameras. Choose one to look through it; orbit away, and **Set from view** (`s`) writes what you see into that camera's `position`, `target`, `up` and field of view. **New** (`n`) appends a camera from the view, named `view1`, `view2` and so on. **Delete** asks first.

## The hour

The hour slider in the lights panel moves the sun and the sky live without writing anything. **Use hour** writes the shown hour into the recipe's daylight `[sun]`.

## Play errors

When Play fails (a missing or broken module, a trap in the game, a factory that refuses), the error stays in the status bar until the next key or click. F6, or a click on that error, opens the window's log with the full message.

## Undo

Ctrl+Z undoes, Ctrl+Shift+Z or Ctrl+Y redoes, from any panel except while a text field has the keyboard. Each step restores the file byte for byte, comments and spacing included.

## Render from the window

The render panel holds a camera, a size and samples, and **Render** runs `pfx render --recipe` as its own process, so the window stays responsive. The finished picture shows as a thumbnail; `v` shows it full size over the viewport. [Path-trace a still](recipe-render.md) covers the renderer itself.
