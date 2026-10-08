# The tracer's Cycles references

`cycles.csv`, `wavelength.csv` and `cycles_sheet.csv` are frozen data: the outputs of three Blender scripts, rendered once in Cycles and kept as the references the tracer's film and sheet checks compare against. Nothing in the engine runs Blender to read them, and the scripts are gone (frozen on 2026-10-06, when Blender left the engine's tests). They are not regenerated; a new reference would need a new Cycles run and a new entry here.

Every file was rendered with Blender 5.2.1 LTS (Arch package `blender 17:5.2.1-2`, installed 2026-09-27 and not upgraded since), Cycles on the CPU, headless, as a bare `blender -b --factory-startup -P <script> -- <out> …` under pgpu.

| file | rows | made by | script commit | script sha256 | arguments |
|---|---|---|---|---|---|
| `cycles.csv` | 123, `angle_deg,thickness_nm,r,g,b` | `crates/trace/film/cycles_film.py` | `6104c8048f5cae7beb2c956db7080764b84363b1` (2026-10-03) | `6baff13173d1ce5d51838209be1e9244e57bdb41988de875612168940f62d1b2` | `tmp/cycles` (defaults: 4096 samples, IOR 1.4, film IOR 1.9, 10 nm steps to 400 nm) |
| `wavelength.csv` | 95, `nm,r,g,b` | `crates/trace/film/wavelength.py` | `6104c8048f5cae7beb2c956db7080764b84363b1` (2026-10-03) | `5a64f30a259514aa38592d18771523c563820a787758b1cc3c02cde6d9aa125b` | `tmp/wavelength` (4 samples, 360 to 830 nm every 5 nm, no denoising) |
| `cycles_sheet.csv` | 34, `budget,kind,albedo,ior,height,thickness,camera,tint,value,r,g,b` | `crates/trace/film/cycles_sheet.py` | `ff724fee942065afd7fe4381d2963db60da8dafe` (2026-10-05) | `e7dc92a44141dcc14808f4dae92f84c0a111a8abbed9a557f0f197a15a860353` | `tmp/cycles_sheet 4096` |

The command of each, from a checkout of the script's commit:

```
pgpu run --class clip --as pfx -- blender -b --factory-startup -P crates/trace/film/cycles_film.py -- tmp/cycles
pgpu run --class clip --as pfx -- blender -b --factory-startup -P crates/trace/film/wavelength.py -- tmp/wavelength
pgpu run --class clip --as pfx -- blender -b --factory-startup -P crates/trace/film/cycles_sheet.py -- tmp/cycles_sheet 4096
```

Each writes `<out>.csv`; the file here is that output, unedited.

| file | sha256 |
|---|---|
| `cycles.csv` | `bfdc8db1c78c3925cf90b03eadd83610c27f83d43b5ebf949dd622841ac31437` |
| `wavelength.csv` | `de8bbfbd7ae75a6c1cf0b1edc13e79f296f12b66c0924d046b9225a55e26dec3` |
| `cycles_sheet.csv` | `614e7faa221c8938bf5271a41c074be2a2bc87049849ad81854dfdf5608dfd0a` |

Who reads them:

- `cycles.csv`: `tests/film.rs` (`include_str!`), the film reflectance checks.
- `wavelength.csv`: `crates/materials/film/live_weights.py`, which fits the live renderer's film weights to it (plain Python, no Blender).
- `cycles_sheet.csv`: the sheet figures in `docs/trace-film.md`, and the constants of `cycles_sheets()` in `tests/film.rs`.

`film_weights.py` stays: it is plain Python (NumPy) that computes the film weights, and runs no Blender.

The asset renderer's own references, one Blender render of each asset kind, are in `crates/assets/tests/reference/`.
