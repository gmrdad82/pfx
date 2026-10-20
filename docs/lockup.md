# The lockup on a board

A game or app that credits pfx on its own screen, a credits or licences page, draws pfx's Powered by lockup on its flat board with `pfx_splash::Lockup`: the same artwork the start-up splash shows ([splash.md](splash.md)), drawn whole, its two glows included, at any height and position. The lockup is drawn as it is: the trademark grant (TRADEMARKS.md) covers the unmodified lockup alone, so a game places and sizes it and changes nothing else.

## Why not read the SVG

`flat::svg::read` of `assets/marks/powered-by.svg` gives the shapes, but its two star groups come with their group opacities left for the caller and its two blurred circles as `Fill::Image` placeholders with no sprite behind them, so a board drawn from it loses the glows. `Lockup` is the splash's own baking of the lockup made public: the splash builds its lockup through the very same code, with the group opacities folded into the draws, the shadows and elevations cleared, and the glows painted once into a small sprite sheet. Drawing it on a board is then plain draws, icons and one sprite sheet.

## Making it

A game adds the crate `pfx-splash`, pinned as its other pfx crates are: pfx-game pulls it in only through its `splash` feature and does not re-export `Lockup`, and a game that turns `splash` off still needs the crate for its credits page. The draws, shapes, icons and sheet it hands back are pfx-live's types (`Draw`, `IconShape`, `Icons`, `SpriteImage`).

```rust
use pfx_splash::{Lockup, Srgba};

let lockup = Lockup::new(ground, height, density)?;
```

- `ground` is the colour behind the lockup. It picks the artwork the splash picks for that ground: `powered-by.svg`, its words lime, on a dark ground; `powered-by-light.svg`, its words violet, on a light one (a ground counts as light above 0.179 relative luminance, as in [splash.md](splash.md)).
- `height` is the lockup's height in board units; its width follows the artwork's aspect, `lockup.size()`.
- `density` is the target's pixels per board unit, so the glows are painted at the resolution they show at: 1.0 when the board's layout is the window's pixel size, `2.0` for a 1920x1080 layout shown at 3840x2160. A lockup drawn at another height is made again at that height.
- It fails on a height or density that is not a finite number above zero, or if the compiled-in artwork cannot be read or painted.

`Lockup::new` reads the compiled-in SVG, paints the glows and packs them: the same work the splash does for its lockup while composing. Make it once at load, or on a worker thread, never inside a frame. After that, drawing it costs a few dozen draws a frame and nothing else.

## Drawing it

A lockup's draws are in board units with its top-left corner at `at`:

```rust
let draws = lockup.draws([x, y]);
```

On a board of its own (the scene's `icons` and `sprites` are the lockup's):

```rust
let icons = FlatIcons::new(&gpu.device, &gpu.queue, lockup.icons()?);
let sprites = lockup.sprites().map(|s| FlatSprites::new(&gpu.device, &gpu.queue, s.clone()));
```

A pfx-game board has one icon atlas and one sprite sheet, the game's own (`Config::icons`, `Config::sprites`), so the lockup's shapes and sheet go into them:

- append `lockup.shapes()` after the game's own icon shapes before `Icons::bake`, and pass the count of the game's shapes as `icon_base`;
- copy `lockup.sprites()` into the game's sprite sheet wherever there is room, and pass the uv rectangle it landed in, `[u0, v0, u1, v1]`, as `frame`. Keep a transparent row or column of texels around it, as the sheet itself has, so linear filtering never reaches the game's own sprites. The sheet it lands in must be filtered `Linear`, as the lockup's own is: a `Nearest` pixel-art sheet would draw the glows blocky, so a game with one keeps the lockup on a board of its own, `draws(at)` with its own icons and sheet. `frame` lies within [0, 1] with u0 < u1 and v0 < v1;
- draw `lockup.draws_in(at, icon_base, frame)`.

`draws(at)` is `draws_in(at, 0, [0.0, 0.0, 1.0, 1.0])`. The draws sit on the board (elevation 0, no shadow) in the order the artwork paints them; put them after the page's background. They can be moved by the board's view like any draw.

| Call | Gives |
|---|---|
| `Lockup::new(ground, height, density)` | the lockup, baked once: `Result<Lockup, String>` |
| `size()` | its width and height in board units |
| `shapes()` | its icon shapes, for a board's own icon atlas |
| `icons()` | those shapes baked alone, for a board with no icons of its own |
| `sprites()` | its glow sheet, a `SpriteImage`, or `None` for artwork with no glow |
| `draws(at)` | its draws with its top-left corner at `at`, for its own icons and sheet |
| `draws_in(at, icon_base, frame)` | the same draws for a board whose icons start at `icon_base` and whose sheet holds the lockup's in `frame` |

## Same as the splash

`a_lockup_on_a_board_draws_exactly_the_splashs_lockup` (pfx-splash's unit tests, on the CPU twin) builds the splash at 320x180 and 180x320 on the dark ground and on a light one, makes a `Lockup` at the splash's lockup height and places it where the splash puts it, and renders both: the splash's lockup draws with its icons and sheet, the lockup on its own, and the lockup sharing a board whose icon atlas starts with another icon and whose sheet holds the lockup's in its right half. Every pixel of both agrees with the splash within 2/255.
