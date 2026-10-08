# Play mode

`pfx-play` (`crates/play`) runs a scene as a game inside an editor, headless: press play and the scene runs live with its game logic, input and sound; pause, step and resume; stop, and the editor is back exactly where it was, its data files untouched. It is what `pfx edit`'s play, pause and stop buttons drive, as Godot, Unity and Unreal do.

The crate is a library an app or a game links. It links no harness and no job-status crate, opens no window and reads no clock: the editor passes the real seconds of each frame, the input events of its viewport and the renderer it already has.

A game that wants pfx to own its window, pacing, input, sound and frame loop calls `pfx_game::run` instead, which drives a `PlaySession` the same way ([game.md](game.md)).

## The pieces

| Piece | What it is |
|---|---|
| `Game` | the game's logic, a trait the game implements in its own repo |
| `SceneGame` | the default game: plays what the scene data defines |
| `World` | the runtime copy of a scene the game ticks |
| `PlaySession` | play, pause, step, resume and stop; input, queued reloads and presentation |
| `Stopped` | what stop hands back: the game, the hooks, the scene to show and the restore |
| `PlayHooks` | calls on play, pause, resume and stop; the editor's `PgpuPlay` tells pgpu a game is running |

## The game

A game's logic is the game's own code; every repo is a black box. The engine defines the interface, the game implements it:

```rust
pub trait Game {
    fn actions(&self) -> Vec<ActionSpec> { Vec::new() }
    fn start(&mut self, world: &mut World);
    fn tick(&mut self, world: &mut World, tick: &Tick, input: &Input);
    fn events(&mut self, world: &mut World, events: &[WorldEvent]) {}
    fn animated(&mut self, world: &mut World, events: &[AnimationEvent]) {}
    fn frame(&mut self, world: &World, alpha: f32) {}
    fn ui(&mut self, world: &World, frame: &UiFrame) -> Ui { Ui::default() }
    fn settings(&self) -> Option<&Settings> { None }
    fn stop(&mut self) {}
}
```

- `actions` names the game's input actions (`pfx_input::ActionSpec`); the session builds its `Input` from them.
- `start` runs once on play, before the first tick, on the fresh world.
- `tick` runs at the clock's fixed sim ticks (`pfx_core::clock`), `1 / rate` game seconds each, with the frame's `Tick` and the input as it stands at that tick. Game logic changes the world only here.
- `events` runs right after the tick's physics step with that step's contact and trigger events, when there are any ([Queries and triggers](#queries-and-triggers)).
- `animated` runs right after the animators and sprite players advance, with that tick's animation events, when there are any ([animation.md](animation.md)).
- `frame` runs once per presented frame with the clock's `alpha` (how far game time is toward the next tick), for presentation the game keeps itself; it sees the world read only.
- `stop` runs once on stop, before the world is dropped.
- `ui` and `settings` are for `pfx-game`: the game's 2D UI each frame, and the settings value it keeps (display, pacing, audio volumes, binds). A session calls `ui` only through `PlaySession::ui`; [game.md](game.md) has both.

The game keeps its run's state in the world (`world.set_state(state)`, `world.state::<T>()`, `world.state_mut::<T>()`), so stop drops it with everything else and the next play starts clean. Fields on the game struct survive a stop, since stop hands the game back.

### Linking it into a game

A game depends on `pfx-play` and pfx's editor crates, and builds its own editor binary around its own `Game`:

```rust
let scene = Scene::open("content/level.scene.toml")?;
let mut staged = renderer.stage(&scene)?;
let session = PlaySession::play(&scene, &staged, Box::new(MyGame::default()))?;
```

`pfx edit` alone plays `SceneGame`. A game that wants the scene's own behaviour as well calls the same world helpers `SceneGame` calls, or holds a `SceneGame` and calls its `start` and `tick` from its own.

## The world

`World` is built on play from the snapshot of the edited scene, never from the files:

| Part | Access |
|---|---|
| objects: transform, visibility, look | `object(name)`, `model`, `set_model`, `local`, `set_local`, `place`, `move_to`, `at`, `rest`, `hidden`, `set_hidden`, `look`, `set_look` |
| the camera | `world.camera`, a scene `Camera`; `exposure()`, `set_exposure` (the finish's leading exposure) |
| materials and lights | `material(name)` (the live copy of a library material), `light(name)`, `lights()`, `set_light` |
| camera rigs | `rig(name)` makes a data-driven follow, first-person or third-person rig drive `world.camera` ([camera-rigs.md](camera-rigs.md)) |
| rigid bodies | `world.physics`, a `pfx_physics::rigid::World` built from `[physics]` and `[body.*]`; `body(id)`, `contacts()` |
| queries, triggers and layers | `raycast`, `shape_cast`, `overlaps`, `overlaps_point`, `events()`, `inside(trigger)`, `layers()` ([below](#queries-and-triggers)) |
| animation | `animate(id, clip)` (`.blend`, `.fade`, `.speed`, `.start`, `.once`, `.done`), `play_clip`, `stop_animation`, `animator`, `sprite`, `palette`, `sprite_frame`, `animation_events()`, `posed()`; objects play their `[object.animation]` from the start ([animation.md](animation.md)) |
| sound | `world.sounds`: the scene's `[sound.*]` decoded (WAV, or Ogg Vorbis through `pfx-sound`'s `vorbis` feature, which `pfx-play` turns on), on a deterministic `Mixer`; `play_sound(name)`, `play_cued(cue)`, `play_hits()`; `sound_level(name)` (volume, pan, loop as they stand) |
| tunables | `tunable::<T>(name)`, `set_tunable`, `tunables()` (below) |
| live edits | `apply(&edit)`, what `PlaySession::edit` calls (below) |
| effects | `world.fx`: trauma shake on the camera and the comfort setting; the scene's glass draws as it does live |
| randomness | `world.rng`, a `pfx_core::sim::Rng` seeded from the session's seed |
| rumble | `rumble(rumble)` on the active pad, `rumble_pad(pad, rumble)`, `rumble_queue()`; the session plays the queue after each tick and hands the step's motor commands to `take_motors()` |
| time | `ticks()`, `time()`, `clock()`, `hit_stop(stop)`, `set_speed(speed)` |
| the game's state | `set_state`, `state`, `state_mut` |
| the snapshot | `scene()`, read only |

Objects keep the scene's hierarchy: a transform is local to the parent (`local`), and `model` is the world matrix through the parents. `set_model` takes a world matrix and stores it relative to the parent, so moving a parent moves its children. An object hidden in the scene has no draws; a game shows it by declaring it visible and hiding it at `start`.

A `Look` swaps an object's material for a library one (`material`), replaces its base colour (`color`) or its emission (`emission`), for every part of the object, without touching the library.

### Bodies and sound per tick

Each tick runs in this order:

1. the tick count rises, and the input takes the events that arrived since the last tick (or the recording's step for that tick);
2. `game.tick`;
3. the rigid world steps once (`[physics] rate` is both the clock's tick rate and the world's step), dynamic and kinematic bodies write their pose into their object's transform, and `contacts()` holds the contacts that step started; `events()` holds the step's contact and trigger events, which `Game::events` receives;
4. the animators and sprite players advance to the tick, and `Game::animated` receives their events ([animation.md](animation.md));
5. the mixer renders the audio of that tick, `rate / tick rate` frames, which the editor drains with `take_audio()` and plays on its device.

A paused session runs no ticks and renders no audio, so it is silent. Stop drops the mixer, so a player takes the last `take_audio()` before it calls `stop`. `Options::audio_rate` and `audio_channels` are what the mixer renders at; the sender does no resampling, so a player on a device sets them to the device's. `pfx edit` does this and plays the audio through a PCM stream (`docs/editor.md`, "Play mode").

## The default game

`SceneGame` plays what the scene data defines:

- movers animate at the world's time (`pose_movers`), as `Staged::frame` poses them;
- bodies declared in `[object.body]` simulate under `[physics]`;
- `[sound.*]` with `play = "start"` play on play, `play = "hit"` play when their object's body starts a contact, and `play = "game"` wait for a game's `play_sound`;
- glass and the scene's other effects draw as they do live.

The keys are in pfx-scene's format ([scenes.md](scenes.md#physics-bodies-and-sounds)): `[physics]`, `[object.body]` and `[sound.<name>]`, each refusing an unknown key by name.

## Queries and triggers

Queries, triggers and layers run on the rigid world's own colliders, in scene units, and name objects, never colliders or handles.

### Layers

A project names up to 32 collision layers in `[layers]` of its `project.toml`; each lists the layers it collides with, written on both sides (pfx-scene's format, [scenes.md](scenes.md#physics-bodies-and-sounds)). pfx-load gives them as `Scene::layers`, and the play world numbers them by name order, so a layer's bit never depends on its position in the file:

```toml
[layers]
world = ["world", "player", "enemy"]
player = ["world", "enemy", "pickup"]
enemy = ["world", "player"]
pickup = ["player"]
ghost = []
```

A body takes a layer with `[object.body] layer = "world"`; an object without one is on every layer and meets every layer that lists any, as everything meets everything in a project without `[layers]`. pfx-scene refuses an unknown layer name at its file and line when the scene loads.

- Two layers collide when each lists the other, which is rapier's two-sided test on the member bits of one and the mask of the other. A layer with `[]` collides with nothing.
- A layer's list decides only what its bodies touch. Queries and triggers go by the layer a collider is a member of, so a layer that collides with nothing (`ghost = []`) is still seen by a query for it and by a trigger that senses it.
- `world.layers()` is the `Layers` value: `bit(name)`, `mask(&[names])`, `names_of(mask)`, `collides(a, b)`, `collision_mask(bit)` and `groups(bit)`, the rigid world's member and mask pair. A character controller or another system takes its layers through it by name. `Layers::from_scene` refuses more than 32 layers and a list that names a missing one.

### Triggers

An object's `[object.trigger]` makes it a trigger: a sensor volume, never solid.

```toml
[[object]]
name = "goal"
mesh = "block"
hidden = true

[object.trigger]
shape = "box"            # box, sphere, capsule or cylinder; the size defaults to the mesh bounds
layer = "volume"         # the layer queries and other triggers see it on; every layer when left out
mask = ["player"]        # the layers it senses; its layer's list when left out, every layer without a layer
```

pfx-load gives them as `Scene::triggers` (shape, offset, layer, mask), with the sizes resolved as a body's are.

### Queries

```rust
let hit = world.raycast(from, direction, 30.0, &Query::layers(&["world", "enemy"]))?;
let hit = world.shape_cast(BodyShape::Sphere { radius: 0.3 }, at, rotation, direction, 5.0, &Query::all())?;
let found = world.overlaps(BodyShape::Box { half: [1.0, 1.0, 1.0] }, at, rotation, &Query::all().excluding("player"))?;
let here = world.overlaps_point(point, &Query::all().with_triggers())?;
```

- A `Hit` has `object` and `name` (None only for a collider the game added to `world.physics` itself), `trigger`, `distance`, `point` and `normal`, in scene units. An overlap lists `Overlap { object, name, trigger }`, once per object, in scene order.
- A `Query` selects layers by name (`Query::layers(&[..])` or `.on(&[..])`, `.mask(bits)` for raw bits), sees triggers only with `.with_triggers()`, and leaves one object out with `.excluding(name)`. An unknown layer or object is a `QueryError`, naming it; a shape with a size of 0 or less is `QueryError::Shape`.
- A direction need not be unit length; a zero one hits nothing.
- The queries read the rigid world as it stands when you ask: at the end of tick N - 1 during tick N, and as built in `Game::start` and the first tick. The same ticks give the same answers on every machine. The world refreshes its query structure after each step, when it is built, and after a live edit of an object; a game that moves a body or collider of `world.physics` itself calls `world.physics.refresh_queries()` for the move to show before the next step.

### Triggers and events

A trigger's sensor sits on its object. An object with a body carries it on that body; any other object gets a kinematic body that follows the object's transform at each tick (its position and rotation; the scale is not applied, as for bodies). A game moves a trigger by moving its object. After each step the world asks, for every trigger, which bodies overlap its shape on the layers its mask names; the difference from the previous tick gives the events, so a trigger and the layers it senses never depend on what a body collides with.

`world.events()` holds the events of the latest step, and `Game::events` receives the same slice right after it, once per tick that has any. Events are in tick order, and in this order within a tick:

1. `WorldEvent::Contact(Contact { a, b, impulse })`, the contacts the step started (what `contacts()` and `[sound]` hits use), in the engine's order;
2. `WorldEvent::Trigger(TriggerEvent { phase: Enter, trigger, other })`, sorted by object;
3. the same with `Stay`, for every pair that was already inside before this tick and still is;
4. the same with `Exit`.

`world.inside(trigger)` lists the objects inside it now. A trigger senses the colliders on the layers its mask names, bodies and the sensors of other triggers alike (a trigger on every layer with no mask senses all of them); it never senses its own object.

```rust
fn events(&mut self, world: &mut World, events: &[WorldEvent]) {
    for event in events {
        if let WorldEvent::Trigger(TriggerEvent { phase: Phase::Enter, trigger, other }) = *event {
            if world.name(trigger) == "goal" && world.name(other) == "player" {
                world.play_sound("chime");
            }
        }
    }
}
```

## The session

```rust
let mut session = PlaySession::play(&scene, &staged, game)?;
loop {
    for event in viewport_events() {
        session.feed(event);
    }
    if let Some(reload) = watch.poll() {
        session.queue(reload);
    }
    session.advance(real_seconds);
    let frame = session.present(&staged, &mut renderer, aspect, seed)?;
    renderer.render(&frame.scene, &frame.text, &frame.effects, staged.finish(), &output)?;
    audio.push(&session.take_audio());
}
let stopped = session.stop();
stopped.restore(&mut staged, &mut renderer)?;
```

| Call | What it does |
|---|---|
| `PlaySession::play(scene, staged, game)` | snapshots the edited `Scene` and the staged state (its overrides), builds the world from the snapshot and runs `game.start` |
| `PlaySession::play_with(scene, staged, game, options)` | the same with `Options`: `seed`, `audio_rate`, `audio_channels`, `max_ticks` per frame (8), `record` and `replay`, `edits` (live edits to replay, below) and `tunables` (`None` reads the project's); `staged` may be `None` for a run with no renderer |
| `PlaySession::play_hooked(scene, staged, game, options, hooks)` | the same with `PlayHooks`; `play` and `play_with` take `NoHooks` |
| `advance(real_seconds)` | feeds the clock and runs the ticks that fall due, at most `max_ticks`, then `game.frame` |
| `pause()`, `resume()`, `paused()` | through the game clock |
| `step()` | one tick while paused; `false` while running |
| `feed(event)` | a viewport input event for the next tick |
| `queue(reload)` | a `SceneWatch` result that arrived during play; see below |
| `world()`, `world_mut()` | the runtime world, for the inspector |
| `edit(edit)` | a live edit from the inspector (below); refused by name when it does not apply |
| `edits()`, `pending()` | the live edits so far, stamped by tick, and the pending patch |
| `present(staged, renderer, aspect, seed)` | the frame to render: the staged meshes posed by the world |
| `take_audio()`, `take_recording()` | the audio rendered so far, the input recorded so far |
| `stop()` | runs `game.stop`, drops the world and returns `Stopped` |
| `game()`, `game_mut()` | the running game, for a runtime that reads its settings |
| `ui(frame)` | the game's UI for this frame (`Game::ui` over the world) |
| `take_motors()`, `set_rumble_intensity(percent)` | the motor commands the rumbler sent since the last call (none in a replay; at most 256 are kept), for the player's pad backend; and the player's rumble setting |
| `set_bindings(map)`, `set_viewport(viewport)` | the action map's bindings, and the viewport the pointer maps through |
| `set_frame_stats(stats)` | times `advance` and `step` as each frame's `sim_ms` in the frame stats ([frame-stats.md](frame-stats.md)); pass the renderer's `frame_stats()` |

### Hooks and pgpu

`PlayHooks` has four calls, each with a default that does nothing: `on_play(&scene)` before `game.start`, `on_pause` and `on_resume` when the clock's state changes (a second `pause` calls nothing, and `step` calls nothing), and `on_stop` after `game.stop`. `Stopped::hooks` hands them back for the next play.

The editor runs outside pgpu, never under `pgpu run` (its trace preview included), and keeps each submission short (2 to 6 ms). pgpu's play-mode contract (its README's "Play mode") is that a playing editor says so: `pfx_editor::pgpu::PgpuPlay` is the hook for it. It lives in pfx-editor, not here, because pgpu is an optional external tool and a game that links pfx-play never spawns it.

```rust
let hooks = Box::new(PgpuPlay::editor(&scene));
let session = PlaySession::play_hooked(&scene, Some(&staged), game, Options::default(), hooks)?;
```

- On play it runs `pgpu play on --pid <this process> --name "pfx edit: <scene file>"`; `PgpuPlay::new(name)` takes any other name.
- Pause and resume run nothing: the player is still in the game, so play stays on.
- On stop it runs `pgpu play off --pid <pid>`, once; dropping it while on does the same, and pgpu counts the process's exit as off.
- While it is on, pgpu freezes clip, truth and search jobs and holds gate tests to 30%.
- A failed call never stops play: `error()` says what went wrong and `is_on()` stays false.

Tests and `play`/`play_with` use `NoHooks`, so nothing calls pgpu unless the editor asks for it. The editor asks only when `pgpu` is on `PATH` and `PFX_EDIT_PGPU` is not `off` (`docs/editor.md`). `with_pid` and `with_program` point the hook elsewhere, which pfx-editor's tests use with a stand-in script.

### What play never touches

- **The files.** Nothing in the crate writes a file. `SceneEdit` and `SceneWatch` are untouched by play; the editor keeps polling its watch.
- **The staged scene.** `present` builds its own instance, material and glass lists from the staged ones each frame (models from the world, hidden objects as shadow-free, looks as extra materials), so `Staged` keeps the edited state through play.
- **The snapshot.** The world copies what it changes; `world.scene()` is the snapshot, read only.

### Reloads during play

A file change that arrives during play goes to `queue` instead of `Staged::apply`. The session keeps the latest good scene, and the errors since it (a good reload clears them, as `SceneWatch` recovers). The world keeps playing the snapshot. On stop, `Stopped::scene` is the latest queued scene (or the snapshot), `Stopped::diff` is the snapshot's diff to it, and `Stopped::errors` the errors still standing. Edits made while stopped hot-reload as always.

### Live edits during play

The inspector changes the running world through `session.edit(Edit)`, never through the files. An `Edit` is the key the inspector would write when stopped, as `SceneEdit` names it (a `Target`, a key path and a `Value`), or a tunable and its value:

```rust
session.edit(Edit::scene(Target::Material("clay".into()), &["roughness"], 0.2f32))?;
session.edit(Edit::scene(Target::Object("crate".into()), &["body", "mass"], 12.0f32))?;
session.edit(Edit::tunable("jump_height", TunableValue::Float(2.5)))?;
```

| Target | Live keys | Reaches |
|---|---|---|
| `Object(name)` | `at`, `rotate`, `scale`, `hidden`, `material` | the object's transform (and its body's pose, woken), its visibility, its look |
| `Object(name)`, `body.<key>` | `mass`, `density`, `friction`, `restitution`, `offset`, `half`, `radius`, `half_height` | the body's collider, rebuilt in Rapier with the new values (its mass recomputed), no restart |
| | `damping`, `gravity_scale`, `velocity`, `spin` | the body in Rapier directly |
| `Material(name)` | every key a materials file takes, nested keys included | the live copy of the library material, which `present` draws in place of the staged one (through `Staged`'s draws, by name), glass and looks included |
| `Light(name)` | `position`, `color`, `intensity`, `radius`, `range`, `shadow` | the renderer's local lights, set again at the next `present` |
| `Camera` | `at`, `look_at`, `up`, `fov` (or `height` when orthographic), `near`, `far`, `fstop`, `focus` | `world.camera`, so the lens and view follow at the next `present` |
| `Finish` | `exposure` | the leading exposure of the scene's finish (or of the standard chain), set at the next `present` |
| `Sound(name)` | `volume`, `pan`, `loop` | the sound's level for its next play; volume and pan also reach a voice that plays |

Anything else is refused with the key and the reason (`material clay nonsense: unknown field`, `light warm range: needs a number of 0 or more`), and the world is left as it was.

**On the next tick, deterministically.** An edit applies at once, between ticks, so it is in the world when the next tick runs. Each is stamped with the ticks done so far (`edits()`, a list of `Stamped { tick, edit }`). `Options { edits }` replays such a list: each edit applies just before the tick after its stamp, whatever the frame times, and live edits are refused for that session. The same scene, seed, input recording and edits at the same ticks give the same run, bit for bit.

**The pending patch.** Every edit that applied is held in `pending()`, the latest value per key, in the form a file takes: a body's `mass` is held as the `density` that gives it, for the body's shape. Stop hands it back as `Stopped::pending` (and the stamped list as `Stopped::edits`), and the world goes with the session. The crate writes nothing; the editor discards the patch on stop, or writes it through `SceneEdit` when he presses Keep ([editor.md](editor.md), "Play mode"). A game's own changes to the world (`set_local`, `set_tunable`, a body's velocity in its tick) are runtime state, never pending.

### Tunables

A game declares its tunables as data in its project's `project.toml` (pfx-scene's format at `e856875`), and reads them while it plays:

```toml
[tunables.jump_height]
type = "float"      # float, int, bool or vector (3 floats)
default = 1.2
min = 0.0           # optional; not on a bool; per axis for a vector
max = 4.0
group = "movement"  # optional; the inspector's heading
```

```rust
let jump: f32 = world.tunable("jump_height").unwrap_or(1.0);
let lives: i64 = world.tunable("lives").unwrap_or(3);
```

- `tunable::<T>` reads `f32` (a float, or an int as a float), `i64`, `i32`, `bool`, `[f32; 3]` or `TunableValue`; another type or an unknown name is `None`.
- `pfx_play::Tunables` loads them: `Tunables::of_scene(scene file)` finds the project root (the nearest folder up with a `project.toml`) and reads its `[tunables.*]`, through pfx-scene's `ProjectFile` (pfx pins pfx-scene at `v0.1.0`, which checks `[tunables]`); `project.toml` is the only place they live. `Options::tunables` hands a session its own instead.
- Loading refuses, by name: an unknown type, a value of another type, a min or max on a bool, a min above its max, a default outside them, and any other key.
- `set_tunable` and a live `Edit::tunable` are range-checked: a value of another type or outside its range is refused and the old value stays.
- `Tunables::written(text, name, value)` rewrites one default with the file's text kept (comments and spacing), which Keep and the stopped inspector use.

### Stop

`stop()` drops the world. `Stopped::restore(staged, renderer)`:

1. puts the staged overrides back to the snapshot's;
2. sets the renderer's lens back to the staged camera's, and its lights and finish back to the snapshot's when a live edit changed them;
3. cuts the renderer's temporal history, so the first frame after stop draws as the edited frame did, with nothing of play blended in;
4. applies the queued diff to the staged scene (`Staged::apply`), which touches nothing when nothing was queued.

The editor then draws `staged.frame` as before play. Play's first `present` makes the same cut, so the first played frame never blends the editor's history either.

The cut is a same-size `Renderer::resize`, the only public call that drops the history today. It reallocates the render targets once per play and once per stop.

## Determinism

The same scene, seed and input recording give the same ticks and the same final world, bit for bit, on the same machine:

- the world never reads a clock; ticks come from the clock the editor feeds, and inputs are taken per tick;
- `world.rng` is seeded from `Options::seed`;
- bodies are added in name order, so the rigid world is built the same way each play;
- a recording (`Options { record: true }`, then `take_recording()`) replays with `Options { replay: Some(recording) }`: each tick feeds that tick's recorded events, whatever the real frame times, and live events are ignored.

Play, stop and play again give the same first frame, pixel for pixel: the world starts from the snapshot, presentation starts with a history cut, and the first frame's time is 0.

## Tests

- `src/tests/queries.rs` (headless): each query on neutral shapes (a raycast's object, point, normal and distance; a shape cast; overlaps in scene order and by layer; triggers seen only when asked); layer filtering by name, by mask and by exclusion; trigger enter, stay and exit in tick order with the stays counted, filtered by the trigger's layers, following an object that moves and a body that carries it; contact events with their impulse; layers that do not collide passing through each other; triggers sensing triggers; layers from the scene in name order, past 32 and with a missing name refused, and a scene naming an unknown layer refused with its file and line; a layer that collides with nothing seen by queries and triggers; queries from `Game::start` and the first tick, and right after a live edit; the same events at the same ticks whatever the frame pacing, against fixed expected ticks (natively and under Wine).
- `src/tests/live.rs` (headless): each kind of live edit reaches the world (materials, lights, the camera and exposure, objects and their bodies, bodies in Rapier with a bouncier ball bouncing higher, sounds on a playing voice) and bad ones are refused; tunables load, are range-checked and read, from `project.toml` or the stopgap file, with a default rewritten with the text kept; stop hands back the pending patch and the next play starts clean; edits recorded by tick replay to the same run.
- `tests.rs` (headless): the hooks' order; a test game moving an object by its input; play, pause, step, resume and stop with the scene restored exactly; no file written during play (contents and modification times); inspector edits discarded; queued reloads and errors applied on stop; the default game's movers, bodies and sounds; determinism from a recording; play, stop, play again from the same world; the game's hooks.
- `tests/restore.rs` (GPU, ignored): `play_pause_step_resume_and_stop_restore_the_rendered_pixels` renders the edited frame, plays, pauses, steps, resumes with an inspector look, stops, and checks the frame after stop and the first frame of a second play against the first, bit for bit.

- `tests/live.rs` (GPU, ignored): `live_material_and_light_edits_draw_as_the_edited_files_do_and_stop_undoes_them` plays the neutral room with a material and a light edited live, and checks its first frame against the same scene with those values in its files, bit for bit, and that a play after stop draws as before.

```
cargo test -p pfx-play --test restore --test live --no-run
pgpu run --class clip --as pfx -- cargo test -p pfx-play --test restore --test live -- --ignored --test-threads=1
```

## The example

`crates/play/examples/play_headless.rs` copies the engine's neutral room scene into `tmp/play_headless/scene`, adds a ball and a box with bodies, a turning pillar and a knock sound, and runs a scripted play, pause, fifteen steps, resume, stop and play again, writing a still at each stage into `tmp/play_headless/` and the play's audio as `play-audio.wav`. It prints whether stop restored the edited pixels and whether play again gave the same first frame.

```
cargo build --release -p pfx-play --example play_headless
pgpu run --class clip --as pfx -- target/release/examples/play_headless
```
