# Licence badges

pfx's four licence badges (Free, Indie, Pro and F&F), path-traced as small enamel plaques by pfx's asset renderer.

- `logos/badge-<state>.svg`: each badge's art, written by `cargo run --release -p pfx-workbench --example badges render/badges/logos` from `crates/workbench/src/badge.rs`, the same shapes the editor's window draws. The words are Jost (SIL OFL, `crates/editor-style/fonts/jost/`) as outlines.
- `preset.toml`: the look: the master's own colours under a clear coat, a thin extrusion with a small round bevel, a front orthographic camera.

Render one badge at 2x with its 1x copy, on a transparent ground:

```
pfx render --asset logo --product badge-pro --recipes render/badges --preset-file render/badges/preset.toml --assets render/badges --size 212x96 --background transparent --shadow off --derive 106 --no-archive
```

The sizes keep each master's aspect at 96 pixels tall: Free 234x96, Indie 306x96, Pro 212x96, F&F (`badge-friends-family`) 272x96.

The finals live in `docs/images/badges/`: the 2x delivery as `badge-<state>@2x.png` and its 1x copy as `badge-<state>.png`.
