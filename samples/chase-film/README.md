# Chase film

A 9:16 chase shot made from pfx's CLI end to end: a sports coupe racing down a wet city street at golden hour, buildings on both sides, crosswalks, traffic, a chase camera with lag and shake, and road spray. Before the chase, five build beats show how the street was made: each beat is a Claude prompt, the pfx CLI running, then a jump cut into pfx's window. The last beat ends on the chase camera's first frame in clay and cuts straight into the rendered chase. pfx writes the recipe and builds the scene, pfx renders it on its live renderer, and pcod encodes the film.

Run everything from the repo's root. Rendered frames and videos go to your video folder, never into the repo:

```
FILM="${XDG_VIDEOS_DIR:-$HOME/Videos}/chase-film"
```

## 1. Write the recipe

`pfx new <out.pitofx> --from street` writes a starter chase recipe into a `.pitofx`: a 9:16 timeline, a route, the street along it, a hero car, traffic and a chase camera. The sample's own recipe is `samples/chase.pitofx`; to start a fresh one:

```
pfx new tmp/street.pitofx --from street --seed 1
```

Shape the route by hand in the window (`pfx gui samples/chase.pitofx`) or with `pfx route samples/chase.pitofx list|set|add|remove ...`, and check the timing with `pfx timeline samples/chase.pitofx`.

## 2. The five build beats

The recipe's `[[stage]]` tables split the street into the order it is built in: `street`, `car`, `city`, `camera` and `look`. `pfx build --stage <name>` builds every stage up to that one. Each beat runs its CLI step, then cuts to its window clip, a tape of quick shots (0.3 to 0.7 s each, 2 to 4 s a beat, at 60 fps) in `breakdown/`, mostly in clay with the selection outlined in the accent:

1. **Street**: `pfx build samples/chase.pitofx --stage street`. The S route over the grid in wireframe, then the road laid down and seen from the top (`breakdown/1-street.tape`).
2. **Car**: `pfx build samples/chase.pitofx --stage car`. The coupe with its lights' gizmos and its tail lights selected, then the rear left wheel with the spray emitter's dots (`breakdown/2-car.tape`).
3. **City**: `pfx build samples/chase.pitofx --stage city`. The road strip, the buildings arraying along it, the plan with the crosswalk stripes selected, the props, then the power poles and their wires (`breakdown/3-city.tape`).
4. **Look**: `pfx build samples/chase.pitofx --stage look`, then `pfx set` tuning of the grade and the lights. A street lamp's spot cone, then a render region on the tail lights through the chase camera, rendered inside and clay outside (`breakdown/4-look.tape`).
5. **Camera**: `pfx shoot samples/chase.pitofx --final --out "$FILM/chase"`. The chase camera's frustum and baked path, then through the camera at t = 0 in clay: the match cut into the chase (`breakdown/5-camera.tape`).

One beat's clip, as PNG frames in its own folder under `$FILM/breakdown`, with the viewport over the whole frame and the timeline strip under it:

```
pfx gui samples/chase.pitofx --clip "$FILM/breakdown" --tape samples/chase-film/breakdown/1-street.tape --fps 60 --size 2160x3840 --scale 2 --bare --strip --frames --json
```

All five in order, each its own GPU job:

```
bin/breakdown "$FILM/breakdown"
```

## 3. Render the chase

`pfx shoot` (beat 5) builds the scene, then has pfx render it through the chase camera, frame n at t = n / 60, into numbered PNG frames, the scene's sound as a WAV and a manifest. The same render by hand, from the scene `pfx build` writes into the `.pitofx`:

```
pfx build samples/chase.pitofx
pgpu run --class clip --as pfx -- pfx shot samples/chase.pitofx --seconds 10 --fps 60 --size 2160x3840 --camera chase --final --out "$FILM/chase" --audio "$FILM/chase.wav" --json
```

## 4. Score it

`samples/chase.pitofx` carries its music style beside the recipe, `[compose.style.build_drop]`: a through-composed build and drop for a film cue, rounds that add a layer each, a lift of a tone with a riser and a roll, the drop on a hit and out on a second one, all on pfx's synthesised patches. `pfx compose` writes an original cue in that style into a copy of the document as its recipe, `[compose.score.title]`: the seed, the sections and every note, a few KB. The audio is rendered from it, the same bytes for the same recipe, and the master is kept in pfx's machine cache; `--wav` writes it out. With `--music` the document also gets a `[music]` table naming each role's stem, its sections and its stingers, and a game loads it with `pfx_compose::render::game::music`, which renders the stems on load (on a worker thread, before the first play). Sections start on the bar nearest their start, so a style that changes tempo loops close to, not exactly on, its bars. `--hits` puts the drop and the end card on bar lines at 18 s and 26 s. [docs/compose.md](../../docs/compose.md) describes the style and score format.

```
cp samples/chase.pitofx "$FILM/chase.pitofx"
pfx compose "$FILM/chase.pitofx" --style build_drop --seed 11 --seconds 30 --hits 18,26 --name title --music --wav "$FILM/title.wav" --json
```

The score goes into a copy, so the tracked sample stays as it is. `pfx compose list "$FILM/chase.pitofx"` names its styles and scores, `pfx compose print "$FILM/chase.pitofx" --score title` prints the notes, and after a hand edit `pfx compose render "$FILM/chase.pitofx" --score title` renders them again.

## 5. Record the Claude Code part

prec records the terminal session that sends each beat's prompt and runs its CLI step, at the film's size and rate:

```
prec clip <app> <shot> --app-dir . --size 2160x3840 --fps 60 --no-archive --out "$FILM/prec"
```

## 6. Assemble and encode the film

`pfx film` lays the beats (prompt, CLI, window clip), the chase and the cards end to end into PNG frames and a WAV, with a manifest naming the pcod command that encodes them:

```
pfx film samples/chase-film/chase.film.toml --out tmp/chase-film --json
```

Then run the `pcod frames` line the manifest names, with `"$FILM/chase-film.mp4"` as its output.

The film script, its score and the prec tapes are added to this folder later.
