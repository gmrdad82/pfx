# Scenes as data

A product's content is a scene: meshes, materials, lights, sky, haze, camera, finish, objects, movers, content, text, bodies and sounds, all data. Adding or changing one is a file edit; nothing compiles. `pfx_load::scene::Scene::open(path)` reads it, `pfx_live::renderer::Renderer::stage` uploads it, `pfx_trace::stage::scene` builds the tracer's stage from the same `Scene`, and `SceneWatch` reloads it into a running renderer as its files change. The engine stays headless: a viewer's window (Manfred's, a product's) calls these.

## The format is pfx-scene's

The files are format 1 of the public crate `pfx-scene`, which owns the format: every file kind, key, type, default and range, ids and references, includes and prefabs, paths, diagnostics and the format 0 to 1 migration. Its spec is [docs/format.md in pfx-scene](https://github.com/gmrdad82/pfx-scene/blob/v0.1.0/docs/format.md), at the tag `crates/load/Cargo.toml` pins (`v0.1.0`). This page keeps only what the engine does with a loaded scene.

## Loading

`Scene::open(path)` finds the project root (the nearest folder upward with a `project.toml`, else the scene's own folder), resolves the scene through `pfx_scene::Project::scene`, and converts the resolved types into the engine's:

- **Checking is pfx-scene's.** Unknown keys, types, ranges, names, ids, references, include and parent cycles, a table held twice, and each path's existence and case are refused there, before the engine reads an asset. A refusal is a `SceneError` with the file, line and column of the value, its message, and the other places it names in `related` (the first use of a duplicate, the object that makes a proxy's object dynamic).
- **Format 0 migrates in memory.** A file without `format` is read through pfx-scene's migration, and `Scene::warnings` carries one warning for it, at its file. So do the ids a hand-written format 1 entry leaves out (`pfx scene fix` writes them). `pfx scene migrate` rewrites the files on disk.
- **Asset contents are the engine's.** pfx-scene checks paths and their case only. pfx-load reads and checks what they hold: the glTF (and the node names in `node`, `nodes` and an object's `materials`, which must be in the mesh), PNG content images, WAV sounds and Ogg Vorbis sounds (an Ogg file's pages must be whole, its Vorbis header must name a rate and channels, and its last page's position must give a duration; `pfx-play` decodes it with `pfx-sound`'s `vorbis` feature), fonts with a family name, Radiance HDR skies, a room sky's lamps, the finish through `pfx_post`, and a body's size from its mesh bounds. Its refusals carry the file, line and column of the entry that names the asset.
- **Keys.** The engine keys meshes, objects, lights, emitters, movers, content, text, bodies and sounds by the resolved scene's keys: an entry's name, and `"<placement>/<inner>"` for an entry a prefab placed. References arrive rewritten to those keys, so `Object::parent`, `Mover::objects` and the rest name keys. A prefab placement is an object with no mesh (`Object::mesh` empty): it draws nothing and is the parent of the prefab's root objects.
- **Paths** are relative to the project root. The engine's `Scene::files`, `SceneMesh::file`, `Content::image`, `Text::font` and `Sound::file` are the root joined with them, in the form the caller passed.
- `Object::id` is the 1-based pick id (`pick`, or the object's place among the resolved objects), and `Object::dynamic` is the resolved `dynamic`, derived where the file leaves it out.

## What the engine does with each table

- **Meshes.** A glTF material resolves to the library entry of its name, else of its name before the first `.`, else `fallback`, else `Material::default()`. Missing normals are computed (smooth, area weighted), missing tangents are `[1, 0, 0, 1]` and missing UVs zero. With `node`, the node's own placement in the file is dropped; the object places it.
- **Sun.** An authored sun follows `hour`: its direction, colour and intensity come from `Daylight::authored(&Authored, reference_hour)` for that hour, day, latitude and heading, so at `hour = reference_hour` it is exactly as authored, and away from it the sun turns, lowers, warms and dims as the day does. Its colour keeps the authored colour's peak; its intensity is `irradiance` times the day's intensity at `hour`. `hour` also picks the light probes' hour. `radius` is the soft sun: live soft shadows (`Quality::sun_radius_deg`) and the tracer's sun disc (`Detail::sun_radius_deg`). Without `[sun]` the sun is dark.
- **Sky.** An analytic sky follows the sun. A mix sky is baked once, on load, into one HDR at the width of its widest HDR or room layer (`256` when every layer is analytic); an analytic layer is evaluated per texel with `AnalyticSky::diffuse`, the sky without its sun disc. The live renderer and the tracer both draw that HDR. Without `[sky]` the sky is black.
- **Emitters.** An emitter is a 12 × 24 sphere with a black, fully rough, emissive material named `emitter <name>` radiating `intensity / (π radius²)` per channel; it casts no shadow.
- **Camera.** `shift` is live's off-centre projection and the tracer's `Lens::shift`. `fstop` is depth of field through a thin lens of the camera's focal length: live `dof::Lens`, traced `Lens::aperture` (`focal / fstop`). `[camera.preset]` tunes `pfx_core::camera::Preset`.
- **Finish.** `file`, or the inline keys, build the chain as `pfx_post::parse` does: a style's passes first, then each key adds or tunes its pass at that parser's own place. `[[finish.pass]]` tables apply in the order written, each built from nothing, after the base. A pass that builds nothing is refused with its line. Without `[finish]` the renderer's standard finish.
- **Haze.** The live renderer marches it (`Renderer::set_haze`); the tracer has no room haze and draws the scene clear.
- **Movers.** At time `t`, `u = (t + offset) / period` and the mover stands at `from + (to − from) · f(u)`. A child's own mover applies first, then its parent's, up the chain. The time is the one the viewer passes to `Staged::frame` (and `pfx_trace::stage::scene_at`); at `0` every swing and once mover stands at `from`.
- **Content.** An object that names a content shows its image in its material's content layer slot; two objects showing different content through one slot are refused when staged.
- **Text.** The block is laid out with the font at `size` per em and centred on `at`; it reads along its local +X with +Y up and faces +Z. `Staged` draws it through the live text pass (`Staged::text`), and the tracer letters it on a carrier card, so both agree.
- **A prefab's text moves with its placement.** Its `at` and `rotate` are in the prefab's space: `Text::place` is the placement object's world matrix (the identity for a scene's own text), and `Text::model()` applies it, so the text moves, turns and scales with its group, and a prefab placed twice draws its text twice.
- **Bound text.** A `{key}` in `text` is filled when the scene loads, with the value bound to `key`: `pfx_load::scene::bind(key, value)` sets it for the process, `bound(key)` reads it and `filled(text)` fills a string. A key is letters, digits, `_`, `-` and `.`; a token with no value, and anything else in braces, stays as written. `pfx_game::Config::bind_text()` binds `name` and `version` from the game's config: the game's window, `Headless` and `<game> edit` call it before a scene loads, and `pfx edit <project>` binds the project's name and its `Cargo.toml`'s `[package] version` (`Project::version()`). The text is filled at load, so play, the editor's viewport, hot reload and the tracer show the same text. `pfx bake` runs with nothing bound, so a bound text in a plated scene is `dynamic = true`.

### `[trace]`

Settings only the tracer reads; the live renderer ignores them. `Scene::trace` holds them as `TraceSettings`:

- `transmissive_shadows` lets shadow rays toward the sun, the sky and local lights pass glass and liquid surfaces, tinted by live's `cast_tint`, so a traced glass shadow matches live's instead of Cycles' opaque one ([trace.md](trace.md#transmissive-shadows)).
- `clamp_indirect` (off at `0`) caps the luminance of each light contribution reached after the first bounce, as Cycles' Clamp Indirect; a brighter one is scaled down to it, keeping its hue. Direct light at the first hit is never clamped ([trace.md](trace.md#fireflies)).
- `filter_glossy` (off at `0`) evaluates glossy, clearcoat and glass lobes at least this rough after a diffuse bounce, as Cycles' Filter Glossy; the first hit keeps its own roughness ([trace.md](trace.md#fireflies)). `0.25` with `clamp_indirect` as a backstop is a good start for a small sun.

pfx-scene refuses a value out of range; `TraceSettings::check` holds the same ranges for a caller that builds the settings itself. `SceneDiff::trace` flags an edit; a `SceneWatch` reload changes nothing in a live renderer for it.

### `[plates]`

The baked plates of the scene's static part (`docs/plates.md`, baked by `pfx bake` with `[plates]` in its recipe, `docs/bake.md`). `Renderer::stage` reads the plate when the scene has `[plates]`: static objects, static text and the emitters are no longer drawn live, the plate is composited in their place, and only the dynamic objects and text are drawn over it. The plate folder's `manifest.json` is read and watched, so a re-bake reloads it; a folder not baked yet is no error. `casters` says what shadows the dynamic objects in the static objects' place: each static object's own mesh as a shadow-only caster, the proxies, or nothing.

`Scene::dynamic(index)` and `Scene::dynamic_names()` say what is dynamic, `Scene::static_subset()` is the scene with every dynamic object hidden and dynamic text dropped (what a plate traces), and `Scene::open_at(path, hour)` opens a scene with its `[sun] hour` set to `hour` (the bake's anchors).

Proxies are never drawn. With `casters = "proxies"` they shadow the dynamic objects; `pfx_load::scene::Proxies::pick(origin, direction)` returns the nearest proxy a ray meets (its object's pick id and distance) for picks on the CPU, and `Renderer::pick` reads the plate's own ids on the GPU where nothing dynamic covers them.

### `[physics]`, bodies and sounds

Play mode's rigid world (`pfx-play`, [play.md](play.md)); the live renderer and the tracer ignore them. A body is an object's `[object.body]`; `Scene::bodies` keys it by the object. Its shape's size and offset default to the object's mesh bounds times its scale: a box's `half` is half the bounds, a sphere's `radius` the largest of those halves, a capsule's or cylinder's `radius` the larger of the X and Z halves, and `half_height` the Y half (less the radius for a capsule). A body with no size this way is refused. A body's `layer`, an object's `[object.trigger]` (shape, `offset`, `layer`, `mask`, sized as a body is) and the project's `[layers]` come through as `Scene::body_layers`, `Scene::triggers` and `Scene::layers` ([play.md](play.md#queries-and-triggers)); pfx-scene refuses a layer name that names none. An object's `[object.character]` comes through as `Scene::characters`, with pfx-scene's defaults and its `radius` (the larger of the X and Z halves) and `height` (the Y size) from the mesh bounds times the scale; one with no size or a height under twice its radius is refused ([characters.md](characters.md)). `[sound.<name>]` plays as the format says (`World::play_sound`).

### `[[tiles]]`: tile layers

A tile layer is pfx-scene's grid of prefab placements in a plane (`[[tiles]]`: `cell`, `origin`, `plane`, `palette`, and `rows` or sparse `cells`). pfx-scene checks it and hands the loader `Scene::tiles`, each palette value rewritten to its prefab's root-relative path; `Tiles::cells` lists the filled cells and `Tiles::position` gives a cell's place. pfx-load (`pfx_load::scene::tiles`) places them after the scene's own objects, cheaply:

- **Each palette prefab is resolved and built once,** with `Project::scene` on its path, and only when a cell uses it. Its meshes join the scene keyed `"<prefab path>:<mesh>"`, so ten thousand cells of one prefab share one mesh entry, and the renderer uploads each geometry once and draws its cells as instances of it.
- **Each filled cell becomes the prefab's objects,** keyed `"<layer>[<i>,<j>]/<inner>"` (`tiles::cell_key`, read back by `tiles::cell_of`), each with its model the cell's translation times the prefab's own: a root object's `at` is the cell's position plus its own, an inner object keeps its parent, renamed into the cell. Their pick ids follow the scene's own objects. A prefab's bodies, characters, triggers, animations and body layers are placed per cell the same way.
- **What a tile may hold.** Objects, bodies, characters, triggers and glTF-clip animations (`Scene::animations`, per cell). A tile prefab with lights, emitters, movers, text, content, sounds or a sprite (which would take a content slot per cell) is refused at its palette key, and so is a material of the prefab's that differs from the scene's material of that name (an equal one is shared). A cell whose object key is already an object's name is refused at the layer.
- `Scene::files` lists the tile prefabs and what they read, so `SceneWatch` reloads on a change to any of them. Painting adds and removes only the painted cells' objects (`objects.added` and `removed`), except a paint that grows a `rows` layer down or left: that moves `origin`, renumbers every cell of the layer and so restages its objects.
- **Cost.** A 100 × 100 layer of one prefab (10,000 objects) loads in about 50 ms in a debug build; `docs/editor.md` has how it opens and draws in the editor.

`tiles::cell_at` finds the cell nearest a point in the layer's plane, `tiles::corners` the square a cell covers, and `tiles::painted` applies a brush's changes to a layer: a `rows` layer grows up and right by adding rows and characters, and down or left by adding them and moving `origin` back by whole cells, so no filled cell moves in the world; a `cells` layer keeps its order and appends new cells. The editor writes the result through `SceneEdit` (below).

### `[object.animation]`

An object's glTF clips or a sprite of an image atlas, played in play mode and drawn live ([animation.md](animation.md)). pfx-load reads it into `Scene::animations`, keyed by the object, and checks the glTF clip names and the sprite frames against the files. An animated object is dynamic, so a plate never bakes it. A sprite's atlas becomes the object's content (`sprite:<object>`), shown through its material's content layer at its first frame until play steps it.

## Reload

`SceneWatch::open(path)` loads the scene and watches the folder of every file it read: the scene, its includes, libraries, glTF files and their buffers, the HDR, the finish file, content images and fonts. It decides by content: an event leads to rehashing every file (SHA-256), and only a changed hash re-reads the scene. Events are debounced (100 ms by default, `set_debounce`), so an editor's write-then-rename lands as one reload.

On a change the scene is re-read whole and compared with the last good one, by name:

- **meshes** by the hash of their glTF bytes, buffers and node overrides;
- **materials** by value;
- **objects**, **lights**, **emitters**, **text** by value; **content** by its image's hash;
- **movers** by value;
- **bodies** and **sounds** by value (a sound by its file's hash), and **physics** by value; none of them touches a live renderer.
- **sun**, **sky**, **haze**, **camera** and **finish** by value.

`Staged::apply` then touches only what changed:

- a changed mesh's parts are swapped into the handles they had with `replace_mesh`; parts nothing uses any more are released with `release_mesh`, and new ones uploaded;
- materials are patched in the per-frame list;
- lights go through `set_lights`, the finish through `set_finish` (or back to the standard one), the haze through `set_haze`, the lens through `set_lens`, the sun's radius through `set_shadow_quality`;
- the sky goes through `request_sky`: it is prepared off the frame path and shown when it settles, so a sky edit never stalls a frame (`Staged::stage` still waits for the first sky with `set_sky`);
- a changed text is laid out again and its atlas rewritten;
- added or removed objects add or drop instances; a changed content image is rewritten into its slot.

An empty diff touches nothing on the device. A broken file keeps the last good scene drawing: `poll` returns the error with its file and line once, and the next good save recovers (`Reload::recovered`). Nothing restarts.

### Waking a viewer

`SceneWatch::open_with(path, wake)` takes a `wake: impl Fn() + Send + 'static` that is called from the watcher's own thread once a burst of file events has been quiet for the debounce, so a redraw-on-demand window (winit's `EventLoopProxy::send_event`, say) repaints on a file change without polling. `watch.pending()` returns the `Instant` a seen change falls due, or `None`, for a viewer that would rather set a timer. Either way the viewer then calls `poll()` on its own thread.

## What a viewer calls each frame

```rust
let proxy = event_loop.create_proxy();
let mut watch = SceneWatch::open_with("content/room.scene.toml", move || {
    let _ = proxy.send_event(Wake);
})?;
let mut staged = renderer.stage(watch.scene())?;
loop {
    match watch.poll() {
        Some(Ok(reload)) => staged.apply(&mut renderer, &reload.scene, &reload.diff)?,
        Some(Err(error)) => show(error),
        None => {}
    }
    staged.set_override("door", Override { highlight: Some([0.2, 0.5, 1.0]), ..Override::default() });
    let finish = staged.finish();
    let frame = staged.frame(aspect, time, seed);
    renderer.render(&frame.scene, &frame.text, &frame.effects, finish, &output)?;
}
```

`watch.poll()` never blocks and does no work until a watched file changed. `staged.frame(aspect, time, seed)` poses the movers and cards for `time` and returns a `StagedFrame`: `scene`, the staged instances and materials as the renderer's `Scene`; `text`, the scene's `[text.*]` for the live text pass; and `effects`, its glass. The parts are there apart too: `staged.pose(time)`, then `staged.scene(aspect, time, seed)`, `staged.text(aspect)` and `staged.effects()`, which borrow the staging immutably. `apply` returns an `Applied` that says what it touched on the device; `Applied::sky_request` is the `request_sky` ticket, and `renderer.wait_sky()` waits for it when a viewer wants the new sky at once.

- **Glass.** A draw whose material is of the glass family (`family = "glass"`) is uploaded with `upload_glass` and drawn as a `glass::Surface` through `Effects::glass`, not as an opaque instance; it keeps its shadow, two_sided, clip and pick id.
- **Overrides.** `staged.set_override(object, Override { hidden, color, highlight, transform })` hides, recolours (`color` replaces the base colour), highlights (`highlight` adds that emission) or moves (`transform`) one object's draws, its glass among them, without editing the scene; its children keep their own looks. It returns `false` for a name the staging has not; `clear_override(object)` and `clear_overrides()` undo. They live in `Staged`, survive `apply`, and touch no data and nothing on the device.
  - `transform` is a world-space matrix applied on the left of each of the object's instance models (and their previous models, so a still preview has no motion), which is how an editor previews a drag without restaging: for a move from pose A to pose B it is `world_B × world_A⁻¹`, set on the object and on each of its descendants, and the frame equals a restaged copy of the moved scene (`crates/live/tests/override_transform.rs` checks the pixels match). It moves meshes and glass only: lights, emitters and `[text.*]` stay where the scene puts them, and a plated static object stays in its plate. Unset (`None`, the default) it changes no instance and no pixel.
- **Lens.** `stage` and `apply` set the depth of field for the renderer's own aspect; after a resize the viewer calls `renderer.set_lens(staged.lens(aspect))`.

For the tracer, `pfx_trace::stage::scene(&scene, aspect)` builds the same instances, materials, lights, sky, sun, lens and camera into a `Staged` ready to `trace`, with the movers at rest and the cards facing the camera; `scene_at(&scene, aspect, time)` poses the movers at `time`, as `Staged::frame` does live. Text is lettered on carrier cards with `pfx_trace::text`.

## Editing

`pfx_load::scene::SceneEdit` is the data layer of every scene editor (pfx's, Manfred's). It edits a format 1 scene's own files on pfx-scene's patch model: its `Target`, `Value`, `Kind`, `Patch`, `PatchGroup` and `EditError` are pfx-scene's, every edit lands through `pfx_scene::SceneEdit::apply`, which checks the edited texts as pfx-scene's `check` does before it writes, and then pfx-load loads the result with the engine's asset checks. A running app's `SceneWatch` hot-reloads the edit like a hand edit, and every editor writes the same bytes.

```rust
let mut edit = SceneEdit::open("content/room.scene.toml")?;
edit.set_at("crate", [-0.9, 0.35, 1.5])?;
edit.set_material("clay", &["roughness"], 0.5)?;
edit.set(&Target::Light("warm".into()), &["intensity"], 9.5)?;
edit.add(Kind::Object, Id::from_bits(random), "ball", &[("mesh", "block".into())], None)?;
edit.undo()?;
```

- **Cycles and depth.** A project whose includes or prefabs form a cycle, or nest deeper than `pfx_scene::MAX_DEPTH` (64), is refused when it is opened, by pfx-scene's own walk (`include-cycle`, `prefab-cycle`, `include-depth`, `prefab-depth`). The finding's message names the chain (`a.scene.toml -> b.scene.toml -> a.scene.toml`) and its `related` places hold each link, which `SceneError` prints as `see here` lines; pfx maps an `EditError` through its `diagnostics`, so `pfx scene fix`, `SceneEdit::open`, pfx edit's log and pfx new all show the chain's files.
- **Text is kept.** Files are edited as TOML documents (`toml_edit`): every comment, blank line, key order and number spelling the author wrote stays, and an edit changes only the value it sets. Of an array, only the components that differ are replaced. A new key goes after the entry's existing ones, a new entry's keys in the format's order, and a new table after the last of its kind. A value that is already equal writes nothing and is no undo step. A file that can't be written back byte for byte (CRLF line ends), or isn't format 1, is refused when it is opened.
- **Ids.** `add`, `add_material`, `add_mesh` and `duplicate_object` take the new entry's `Id`: an editor passes its own random bits (`Id::from_bits`), and a build derives one from a key it owns (`Id::derive`). The first write of a file fills the ids its entries lack, as pfx-scene's writer does.
- **The file that defines the item.** An entry is edited in the scene file or include that holds it, a material in its library; `[sun]`, `[sky]`, `[haze]`, `[camera]`, `[finish]`, `[trace]`, `[plates]` and `[physics]` where they are, and in the root scene file when no file has them yet. An entry a prefab placed is edited through its placement's `[object.set]`.
- **What it edits.** `set(&Target, path, value)`, `unset`, `push` and `remove(&Target)` take any key: a target and a path of keys into it, where a number picks an entry of a list (`["layers", "1", "frequency"]`); a missing table on the way is created. Typed helpers cover an object (`set_at`, `set_rotate`, `set_scale`, `set_object_material`, `set_object_materials`, `set_shadow`, `set_two_sided`, `set_hidden`, `set_clip`, `set_parent`), and `add`, `add_material`, `add_mesh`, `set_node`, `duplicate_object` and `rename_object` cover the rest.
- **Checked before it is written.** Every edit is first made as a pfx-scene dry run, which checks the group as a whole; then the engine's asset checks run on `Project::scene_with(scene, group.after())`, the scene with the edited texts in memory (an override naming a node the glTF lacks is refused there). Only a group both accept is written, through `pfx_scene::SceneEdit::apply`. A refused edit writes nothing and returns an `EditError { file, key, line, code, message }` and records nothing. An edit also refuses when a file changed on disk since it was read (`reload()` takes the disk's text and drops the history); files are written by a temporary file and a rename.
- **Undo and redo.** Every edit returns a `PatchGroup`: a label (`set object crate at`) and one `Patch` per file it touched, each with the file, its exact text before and its exact text after. `undo()` and `redo()` apply the inverse of those values, so the bytes come back exactly; a new edit clears the redo list. `begin(label)` … `end()` makes everything between one undo step (a gizmo drag); `cancel()` takes it back. Nothing is pushed for a group whose net text is unchanged.
- **For an editor with its own history.** `Patch` and `PatchGroup` are plain values with `label()`, `file()` / `files()` and `inverse()`; `SceneEdit::apply(&PatchGroup)` applies one without recording it (refused when a file is not at the text the patch starts from).
- **Dry runs.** `edit.dry_run(|edit| { edit.set_at("crate", at)?; edit.set_hidden("lid", true) })` runs any edits (set, unset, push, remove, add, rename, duplicate and the rest) on the scene's texts in memory and returns the one `PatchGroup` they would make: the disk's text before, the last text after, per file. It writes no file, touches neither the undo nor the redo list, and leaves `scene()` as it was, so the editor crates' viewport can land the group in its own in-memory files and make its save the only write. The group is checked as a whole, several files included, by pfx-scene and by the engine's asset checks, and refused as an edit would be. `apply(&group)` lands it later with exactly the bytes the direct edits write. A dry run is refused inside `begin` … `end`, and a dry run takes no group of its own.
- **Tile layers.** `set(&Target::Tiles(name), …)` edits a layer like any entry: `["rows", "<n>"]` is one row's whole new string, `["rows"]` or `["cells"]` the whole list, `["palette", "<key>"]` a palette entry and `["origin"]` the origin; `Kind::Tiles` adds a layer. The edited scene is reloaded with its cells placed again.
- **With a watch.** After an edit, `SceneWatch::check` (or `poll`) reads exactly the changed files and its `SceneDiff` names only what was edited: an `at` change is that object in `objects.changed`, a material value is its name in `materials.changed`, a light or the camera only theirs, an added or removed object is in `objects.added` or `removed`; no mesh is read again.

An edit that needs the viewer to show it at once calls `SceneWatch::poll` (or `check`) on the next frame as in "What a viewer calls each frame"; `SceneEdit::scene()` is already the edited scene, for a viewer that applies the change itself, and `SceneEdit::resolved()` is pfx-scene's resolved scene.

## Commands

- `pfx scene migrate <scene.toml> [--dry-run]` rewrites the format 0 files the scene reads (the scene, its includes, libraries and proxies file) as format 1 through pfx-scene's migration, keeping comments, spacing and order: `format = 1` on the first line, paths relative to the project root, an object's `id` renamed `pick`, each `[body.<object>]` moved into its object, and an id in every entry. It refuses to write when the scene or a migrated file doesn't check, and names each file it rewrote.
- `pfx scene fix <scene.toml> [--dry-run]` writes the ids that the entries of the scene's format 1 files lack, derived as the migration derives them, and refuses a format 0 file and a scene that doesn't check.
- `--dry-run` writes nothing and names each file it would rewrite. `pfx_load::scene::migrate(path, dry_run)` and `fix(path, dry_run)` are the same as functions, returning the `PatchGroup` (each file's text before and after); `fix` is `pfx_scene::Project::fix`, and both write through `PatchGroup::write` (a temporary file and a rename, refused when a file changed, put back on failure).
