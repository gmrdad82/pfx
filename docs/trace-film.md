# The tracer's thin film

`pfx_trace` draws a thin film on a transmissive surface the way Cycles does with Principled BSDF's `Thin Film Thickness` and `Thin Film IOR`. A product that retires Blender needs the tracer to give the colour Cycles gives from the same film. The Cycles references it is checked against are frozen data (`crates/trace/film/README.md`): they were rendered once in Blender 5.2.1 and no test, build or doc runs Blender to read them.

## The model

A surface with `transmission > 0`, `thin_film > 0` and `thin_film_amount > 0` is a delta reflect or refract surface in the tracer. The film changes its reflectance from the plain Fresnel of `ior` to the reflectance of the stack: the outside medium, the film (`thin_film` nanometres thick, index `thin_film_ior`) and the substrate (`ior`). `thin_film_amount` blends the plain Fresnel and the filmed one.

Leaving the substrate the tracer reads the film as Cycles does (`adjust_thin_film_ior_at_backface` in Cycles' `bsdf_microfacet.h`): the incident side is 1, the film `thin_film_ior / ior` and the substrate `1 / ior`. The Fresnel ratios are those of the physical stack (substrate, film, air), but the phase runs through a film of index `thin_film_ior / ior`, so on the way out the film reads as one `ior` times thinner. `film::surface_reflectance(cosine, thickness_nm, film_ior, ior, entering)` is the CPU copy of the shader's `cf_surface`, and `leaving_reads_the_film_thinner_by_the_ior_as_cycles_does` pins the equivalence.

### One path for every colour

The delta branch keeps the three colour channels on one path unless their directions differ:

- **Clear glass and films** refract all three channels together at full weight. With a film the reflectance is a colour `F`, so the branch reflects with the chance `p = mean(F)` and weight `F / p`, and refracts with weight `(1 − F) / (1 − p)`, the way Cycles samples its spectral Fresnel. Each weight is at most 3, and a path without a film always carries weight 1.
- **Dispersion** (`dispersed_ior` differs between channels by more than 1e-4) picks one channel at triple weight, because the channels refract in different directions. The path then carries that channel (`carried` in `radiance`) and every later dispersive or filmed surface follows it at weight 1, where the old branch drew a fresh channel at every surface and lost two thirds of the paths at each one.

Before, every transmissive vertex picked a channel at triple weight, film or not, so a path through four interfaces was one channel at weight 81 with a chance of 1 in 27, or black.

### The transmission tint

A refraction multiplies the throughput by `sqrt(base)`, clamped to 1, not by `base`: Cycles' Principled BSDF sets its glass lobe's transmission tint to `sqrt(clamped_base_color)` (`generalized_schlick_setup` in `svm/closure.h`), so light through a closed body, in and out, takes the base colour once. This changes every tinted transmissive material in the tracer: a body crossed twice is as light as Cycles draws it, where it was tinted by `base²`.

### Light through one surface: no radiance scaling, Cycles' bounce budget

Cycles has no rule of its own for light through a single surface. Two things made a one-sided clear sheet read darker in Cycles than in the tracer, and the tracer now does both as Cycles does.

- **No radiance scaling.** Cycles' refraction carries no `(η_i/η_t)²`. Its microfacet glass (`bsdf_microfacet_eval` and `bsdf_microfacet_sample` in `closure/bsdf_microfacet.h`) puts the refraction Jacobian `η²·|cos_HI·cos_HO| / (cos_HO + cos_HI/η)²` into both `eval` and `pdf`, so a sampled refraction weighs `transmittance / (1 − pdf_reflect)`, and a singular one weighs the same. The tracer used to multiply the throughput by `1/η²` entering and `η²` leaving (PBRT's radiance mode). A path from a camera outside to a sky outside crosses every surface as often inward as outward, so the scaling cancelled on every such path and never explained the sheet. It only shows when the camera sits inside the one-sided medium: with the camera between the sheet and the table Cycles reads 0.6724 at depth and the tracer now 0.6731, where the scaling would make it 1.96 times brighter. The refraction keeps the `sqrt(base)` tint and Beer absorption.
- **Cycles' bounce budget.** The light through a sheet onto a table under it comes only by paths: the table's shadow rays stop at the glass in Cycles and in the tracer, since glass is not a transparent BSDF. Light that bounces between the table and the sheet's total internal reflection takes many diffuse bounces, and Cycles cuts those paths by its per-kind limits. `path_state_next` (`integrator/path_state.h`) counts every scatter and each kind, diffuse reflection, glossy reflection (total internal reflection included) and transmission; when a count reaches its limit the path is marked `PATH_RAY_TERMINATE_AFTER_TRANSPARENT`, so its next surface still adds emission and the sky is still collected on a miss, but the surface gets no closures, no light samples and no bounce. The kernel limit is the setting plus one (`kintegrator->max_diffuse_bounce = max_diffuse_bounce + 1` in `scene/integrator.cpp`), so Blender's "diffuse 4" allows five diffuse scatters. The tracer used a flat 12 iterations.

The tracer's `Detail` now has `bounces: Bounces { total, diffuse, glossy, transmission }`, in Blender's terms, passed to the shader as pipeline-overridable constants (`TOTAL_BOUNCES` and the rest), so the frame uniform keeps its 144 bytes and `pfx-bake`, which compiles `trace.wgsl` with its own frame, gets the default budget unchanged. The default is `Bounces::REFERENCE` (16, 4, 8, 12, with Blender's default diffuse limit), the budget the engine's Cycles references render with. `Bounces::BLENDER` is Blender's defaults (12, 4, 4, 12) and `Bounces::deep(n)` lifts every limit to `n` (at most 64). In `radiance` the delta reflection and total internal reflection count as glossy, the delta refraction as transmission, the diffuse lobe as diffuse and the specular and coat lobes as glossy; the loop runs `total + 2` hits. Russian roulette is unchanged; it is unbiased.

The sheet's Cycles figures are frozen data, `crates/trace/film/cycles_sheet.csv`: a 100 × 100 Diffuse BSDF table, a 100 × 100 one-sided Principled sheet (transmission 1, roughness 0.02), a closed slab or a closed glass sphere (a smooth-shaded 256 × 128 UV sphere; the tracer's is an exact ellipsoid shape) over it, a uniform white world, an orthographic camera 60 units above (or 2.5 units, between the sheet and the table), Filter Glossy off and no clamp, rendered once in Blender 5.2.1 at 4096 samples on 8 × 8 pixels (34 cases). Its `thickness` column is the slab's thickness or the sphere's radius, `tint` the glass's base colour, and `r`, `g`, `b` the channels. The script is gone; the file's provenance (Blender version, script, script commit, command) is in `crates/trace/film/README.md`, and nothing in the engine runs Blender. The tracer's figures are the same scene through `Trace::sample_paced`, 4096 samples, a pure Lambert table. The sheet is 5 units over the table unless the row says otherwise; IOR 1.4.

| case | Cycles, reference budget | tracer, reference budget | Cycles, 64 bounces | tracer, 64 bounces |
|---|---|---|---|---|
| albedo 1 | 0.9506 | 0.9572 | 0.9990 | 1.0014 |
| albedo 0.8 | 0.6662 | 0.6712 | | |
| albedo 0.5 | 0.3547 | 0.3547 | 0.3581 | 0.3557 |
| albedo 0.2 | 0.1384 | 0.1369 | | |
| albedo 1, IOR 1.33 / 1.5 / 1.8 | 0.9686 / 0.9200 / 0.8122 | 0.9751 / 0.9240 / 0.8131 | | |
| albedo 0.5, IOR 1.33 / 1.5 / 1.8 | 0.3724 / 0.3334 / 0.2972 | 0.3737 / 0.3327 / 0.2941 | | |
| albedo 0.8, sheet 1 / 20 units up | 0.6406 / 0.7699 | 0.6459 / 0.7729 | | |
| albedo 0.8, slab 1 thick | 0.7875 | 0.7926 | 0.7935 | 0.7926 |
| albedo 0.5, IOR 1.5, slab 1 thick | 0.5049 | 0.5056 | | |
| albedo 0.8, camera between | 0.6603 | 0.6603 | 0.6724 | 0.6731 |

The worst difference is 0.0066. With 64 bounces both are the lossless cavity (a white table under a white sky reads 1). The budget is what darkens the sheet: albedo 1 at IOR 1.8 reads 0.8122 in Cycles at the reference budget, and lifting one limit at a time shows which ones bind, the same in both:

| reference budget, with | Cycles | tracer |
|---|---|---|
| nothing changed | 0.8122 | 0.8131 |
| diffuse 64 | 0.9662 | 0.9685 |
| glossy 64 | 0.8115 | 0.8132 |
| transmission 64 | 0.8133 | 0.8132 |
| total 64 | 0.8124 | 0.8132 |
| diffuse 2 | 0.5702 | 0.5686 |
| glossy 2 | 0.7031 | 0.7032 |
| total 6 | 0.7024 | 0.7032 |

A closed glass sphere and a tinted, liquid-like slab over the same table check the closed volumes and the transmission tint per channel. The sphere has radius 1, its centre 1.5 over the table; the slab is 2 thick and 1 over the table, IOR 1.33, base colour (0.9, 0.92, 1.0). The camera looks straight down through them.

| case | budget | Cycles (r, g, b) | tracer (r, g, b) |
|---|---|---|---|
| sphere, IOR 1.5, albedo 0.8 | reference | 0.8081 | 0.8060 |
| sphere, IOR 1.5, albedo 0.8 | 64 bounces | 0.8088 | 0.8068 |
| sphere, IOR 1.33, tinted, albedo 0.8 | reference | 0.6962, 0.7170, 0.8029 | 0.6943, 0.7152, 0.8011 |
| slab, albedo 0.8 | reference | 0.6379, 0.6657, 0.7837 | 0.6421, 0.6701, 0.7887 |
| slab, albedo 0.8 | 64 bounces | 0.6390, 0.6673, 0.7892 | 0.6422, 0.6702, 0.7889 |
| slab, albedo 0.5 | reference | 0.4008, 0.4178, 0.4893 | 0.4031, 0.4201, 0.4918 |

The worst channel is 0.0050 (the slab's blue). The sphere reads about 0.002 under Cycles since the Wächter-Binder ray start, which moved it 0.0007 darker and left every sheet and slab figure unchanged. The tint comes out right per channel: light through the closed body takes the base colour once, as `sqrt(base)` at each of its two surfaces.

Before the off-by-one was found the tracer cut each limit one scatter early: 0.7127 against 0.8122, and 0.3649 against 0.5702 at diffuse 2.

The figure this note used to carry, 0.4385 in Cycles against 0.738, came from a Cycles scene that was not checked in. Blender's defaults with Filter Glossy on (12, 4, 4, 12, `blur_glossy = 1`, clamp 10) read 0.6561 and 0.2983 at albedos 1 and 0.5 here: Filter Glossy roughens the sheet after the first diffuse bounce to an α near 0.45 and does not recompute the closure's energy compensation (`bsdf_blur`, "TODO: Recompute energy preservation after blur?"), so each pass through the cavity loses light. That accounts for the shape of the old fit (`r = 0.25`, where the lossless cavity's is about 0.5), but not its level, which the old scene's table or IOR set and which cannot be recovered.

`tests/film.rs` holds the checks, all on the GPU through `Trace::sample_paced` and every slice under 300 ms:

- `a_clear_sheet_over_a_table_reads_like_cycles`: the fifteen reference-budget rows, each within 0.01.
- `a_deep_clear_sheet_is_a_lossless_cavity_like_cycles`: the four 64-bounce rows, each within 0.01.
- `each_bounce_limit_cuts_paths_where_cycles_does`: the eight budget rows, each within 0.01.
- `a_glass_sphere_and_a_tinted_slab_over_a_table_read_like_cycles`: the six sphere and slab rows, each channel within 0.01.
- `a_closed_glass_sphere_keeps_its_energy`: still 0.999994 in every channel. A closed volume never depended on the scaling.

### The film's reflectance

The reflectance is Cycles' own:

- Airy's closed form for a film between two interfaces, which sums every internal reflection;
- both polarisations, averaged, from the amplitude Fresnel coefficients of each interface;
- a spectral integral over 380 to 780 nm every 5 nm (81 wavelengths), exact to 1e-4 up to a 2000 nm film;
- the wavelengths reach RGB through the weights in `crates/trace/film/wavelength.csv`, the table of Cycles' Wavelength node, normalised per channel so a flat spectrum is white. The node clips the negative lobes of the colour matching functions, so those come from Wyman's multi-lobe fit. `film/film_weights.py` writes the table into `trace.wgsl` (`CF_WEIGHTS`) and `film.rs` (`WEIGHTS`), and a unit test checks the two are the same numbers;
- total internal reflection at either interface gives 1; a thickness of 0 is no film.

`pfx_trace::film::film_reflectance(cosine, thickness_nm, outer_ior, film_ior, inner_ior)` is the CPU copy of `cf_film` in `trace.wgsl`.

Two earlier behaviours changed with it, both in `trace.wgsl`:

- the transmissive branch of `sample_bsdf` used the plain Fresnel and ignored the film, so the film only showed in the glossy lobe of the light samples;
- a transmissive surface showed its reflection twice, film or not: the delta branch reflects the sky and any emitter it hits, and the glossy lobe of the light samples added the same reflection again. `glossy_share` is `1 − transmission · (1 − metalness)`, the share the delta branch does not take, and it scales the glossy lobe of the light samples a delta ray can also reach, the sky, emissive triangles and shapes and rect lights (a sample with a pdf), and the glossy BSDF branch. The sun and the point and spot lights are not scaled: the delta branch cannot reach them, so the glossy lobe is their only highlight on glass.

The tracer's other film paths go through the shared `eval` of `pfx-materials`: an opaque dielectric or a metal with a film, and the glossy lobe of a filmed transmissive surface's light samples. They evaluate the spectral film of the next section, the same functions the live renderer uses. Until then they were `film_rgb` and `metal_film`, three wavelengths and one polarisation, which did not match Cycles; those two stay as the three-wavelength approximation.

## How it was checked

The film's Cycles figures are frozen data, `crates/trace/film/cycles.csv` (4096 samples), rendered once in Blender 5.2.1: a flat patch for each film thickness (0 to 400 nm every 10 nm) at 0, 45 and 70 degrees, an orthographic camera, a uniform white world, and a black plate parallel behind each patch so only the reflection shows. The patch is Principled with IOR 1.4, transmission 1, roughness 0.02, thin film IOR 1.9 and the thickness from a per-vertex `film` attribute, the way `truth.py` sets it. `crates/trace/film/wavelength.csv` captured the Wavelength node table the same way. The scripts are gone; each file's provenance is in `crates/trace/film/README.md`, and nothing in the engine runs Blender.

`tests/film.rs` holds the checks:

- `film_reflectance_matches_cycles_within_two_bytes`: the CPU function against every row of the table, within 2/255 in sRGB. It runs in the gate. The worst difference is 1.2/255.
- `patch_at_180_nm_matches_cycles` (GPU): the tracer's patch at 180 nm and 0, 45 and 70 degrees against the table, within 2/255 (worst 0.5).
- `film_curve_dump` (GPU): the same patches at every thickness, printed as CSV.
- `a_clear_glass_slab_refracts_all_channels_together` (GPU): a rounded glass slab over a grey table under a coloured 4×2 sky, 32 independent runs of 64 samples on 8×8 pixels. Its per-pixel variance must be under a fiftieth of the old branch's, its colour variance (of r − g and b − g) under a thousandth, and its mean within 4σ of the old branch's, pinned from 1024 runs before the change.

### Result

The tracer's patch against Cycles, every thickness from 10 to 400 nm and the three angles, 256 passes of 32 samples:

| | before | after |
|---|---|---|
| worst difference | 90.7/255 (45 degrees, 350 nm) | 1.2/255 (0 degrees, 160 nm, green) |
| mean of the per-thickness worst | 51.6/255 | 0.5/255 |

At 0 nm there is no film, and a plain transmissive surface used to show its sky reflection twice: 0.044, 0.080 and 0.189 against Cycles' 0.028, 0.037 and 0.150 at 0, 45 and 70 degrees. With `glossy_share` it is 0.028, 0.037 and 0.151 (`plain_transmissive_surface_reflects_once_like_cycles`, within 0.6/255).

### The clear slab

On `a_clear_glass_slab_refracts_all_channels_together` the variance drops from 8.47 to 0.031 and the colour variance from 15.3 to 0.0004; the mean stays within 1.8σ (0.578, 0.577, 0.578 against 0.574 ± 0.003, 0.586 ± 0.011, 0.585 ± 0.004).

### What the checks against Cycles taught


- **The sky mapping is not mirrored.** Blender's equirect is `u = atan2(y, −x)/2π + ½`, `v = atan2(z, hypot(x, y))/π + ½` (z up, v up). The engine's is `u = atan2(x, −z)/2π + ½`, `v = acos(y)/π` (y up, v down). Both run clockwise seen from above, so Blender's axes map to the engine's by the proper rotation `(x, y, z) → (y, z, x)`; `(−y, z, −x)` with the sky turned by 180° is the same rotation and needs no calibration. Mirroring the sky and turning it through twelve azimuths gave a worse match than the rotation (rms 9.4 against 6.9).
- **The sky sampling is exact.** The sampler picks a row, a column and a point in the cell, and its pdf is the cell's mass over its solid angle, as the code that built the CDF defines it. `sky_sample` used to look the pdf up again from the direction it had drawn, and about 7 in 100,000 samples landed in a neighbouring cell through rounding, with that cell's pdf (0 against a black cell: the NaN pixels of round 3). It now returns the pdf of the cell it drew from, plus the sun's density when the direction is inside the sun's disc; a sample drawn from the sun still looks up the sky's density under it. `the_pdf_is_the_density_of_what_the_sampler_draws` (in `sky.rs`) draws two million samples from a speckled sky and checks the counts per cell against the CDF's masses within 5σ and that every sample has a pdf. A bare table under an HDR sky matches the irradiance of the HDR and Cycles to 0.05% (0.7079, 0.7081 and 0.7081 for the tracer, Cycles and the integral).
- **Ray offsets scale with the hit.** A table seen from 2,600 units read 1.9% too bright: a hit at that distance carries an error of about 3e-4, past the old fixed 0.0005 m start, and the table lit itself from underneath. A ray now leaves a hit by the offset in Wächter and Binder, "A Fast and Robust Method for Avoiding Self-Intersection" (Ray Tracing Gems, chapter 6): per component, add `n / 65536` when the coordinate is inside 1/32, otherwise add or subtract 256 ULPs of that component along the outgoing direction, then add `direction · 4e-7 · (|origin| + t)`. The fixed 0.5 mm floor is gone, and a scene near the origin moves by about 0.02 mm. `a_far_camera_does_not_light_a_plane_with_itself` puts a plane 0.05 below zero 40 and 2,600 units from the camera under a uniform sky and wants the two to agree within 1%.
- **Refraction carried radiance scaling, then dropped it.** Every refraction used to multiply the throughput by `(η_i/η_t)²`, `1/η²` entering and `η²` leaving (PBRT's radiance mode). A closed volume cancels either way (`a_closed_glass_sphere_keeps_its_energy`: 1.0009, 1.0048 and 0.9920 with a channel picked at every surface, 0.99999 with the channels together, 0.999994 without the scaling). Cycles has no such scaling, and the tracer dropped it (see *Light through one surface* above).
- **A one-sided clear sheet.** It read 0.738 in the tracer against 0.439 in a Cycles scene that was never checked in, and the radiance scaling was blamed. The cause was Cycles' per-kind bounce budget (with its kernel's +1) and, in that scene, Filter Glossy; with the budget the tracer matches Cycles within 0.0066 on the checked-in sheets (see *Light through one surface* above).

## The live renderer's film, and film on opaque and metal surfaces

The live renderer, and the tracer's shared `eval`, evaluate the same film as the delta branch with fewer wavelengths: `film_dielectric` and `film_conductor` in `crates/materials/src/wgsl/film.wgsl`, mirrored on the CPU in `film.rs`. Both are Airy's closed form with the two polarisations averaged, as `cf_film` is, summed over 24 wavelengths and weighted straight into linear sRGB.

- **The wavelengths** run from 740 to 390 nm evenly in wavenumber (`FILM_NU`, `FILM_STEP`), so the phase `4π·n_f·d·cos θ_f / λ` of each is the previous one's turned by a fixed angle: one `sin` and `cos` per film, and a complex multiply per wavelength.
- **The weights** (`FILM_WEIGHTS`) carry CIE colour matching to linear sRGB, as the tracer's table does. `crates/materials/film/live_weights.py` fits them by least squares to the tracer's 81-wavelength film (`film_weights.py`'s table, Cycles' Wavelength node) over films of index 1.33 to 2.6 on substrates of 1 to 2.4 up to 1.1 µm and the five metals below up to 400 nm, at 0 to 78°, each sample weighted by the sRGB slope so the fit is in levels, held near the tracer's density resampled to the 24 wavelengths and summing to 1 per channel, so a flat spectrum stays white. `python3 crates/materials/film/live_weights.py` prints the WGSL table and `--rust` the Rust one; a unit test checks the two are the same numbers. Since the weights sum to 1, each wavelength costs one reciprocal per polarisation: `R = 1 + ½·Σ w·(a − e)/(e + x)`.
- **At normal incidence** the two polarisations are the same, so the sum takes one.

How many wavelengths: `live_weights.py --measure` fits 8, 12, 16 and 24 the same way and measures them against the tracer, in levels of 255, on a dielectric sweep to 1 µm and a metal sweep to 400 nm, both 0 to 70°, and the largest step between neighbouring samples 5 nm apart from 0.9 to 1.1 µm:

| wavelengths | dielectric to 1 µm | metal to 400 nm | largest step at 1 µm |
|---|---|---|---|
| 3 (`film_rgb`) | | | 24.5 |
| 8 | 140.3 | 49.2 | 55.2 |
| 12 | 13.0 | 8.9 | 6.6 |
| 16 | 5.6 | 4.1 | 5.0 |
| 24 | 0.62 | 0.82 | 4.97 |
| the tracer's 81 | | | 4.97 |

Eight band worse than three at 1 µm; sixteen no longer band, but are 5.6 levels off where a film of index 2.6 is near 1 µm; 24 follow the tracer to within a level everywhere on the sweep. A film under about 300 nm of optical thickness would do with every other wavelength (0.05 levels), but oil slicks and anodised oxides sit at 380 to 700 nm, so there is one table.

### Over a dielectric and over a metal

`reflection_colour` mixes the film in by `thin_film_amount`, as before: a material with `metalness` under 0.5 has a dielectric substrate of index `ior` (a lacquer with an oil slick: air, film, lacquer), and one at 0.5 or more a conductor whose colour is `f0 = mix(specular, base, metalness)` (anodised titanium: air, oxide, metal).

The conductor's complex index `n + ik` per channel comes from its colour the way Cycles' Principled BSDF does it for a metal under a thin film (`microfacet_fresnel` in `bsdf_microfacet.h`): Gulbrandsen's artist-friendly mapping with the edge tint taken from the F82-tint model at `cos θ = 1/7` with a white tint,

```
r = min(f0, 0.999)
g = f0 + (1 − f0)·(6/7)⁵
n = g·(1 − r)/(1 + r) + (1 − g)·(1 + √r)/(1 − √r)
k = √((r·(n + 1)² − (n − 1)²) / (1 − r))
```

(`film_conductor_ior`, `conductor_ior`), which gives back `f0` at normal incidence exactly. The film-to-metal amplitudes are the complex Fresnel coefficients `r_s = (n_f cos θ_f − u)/(n_f cos θ_f + u)` and `r_p = (N² cos θ_f − n_f u)/(N² cos θ_f + n_f u)`, `N = n + ik`, `u = √(N² − n_f² sin² θ_f)` on the principal branch, so the metal's reflection carries its phase into the Airy sum (`R = |r₁₂ + r₂₃e^{iφ}|² / |1 + r₁₂r₂₃e^{iφ}|²`). Cycles takes the magnitude of `r₂₃` from its F82 model instead of these coefficients; here it is the coefficients' own. Each channel uses its own `n + ik` at every wavelength, so a bare metal (a film of 0 nm) is the complex Fresnel of its colour, and the film's interference is spectral. `pfx_trace::film::conductor_reflectance` is the 81-wavelength reference in f64.

### Where it runs

- **The live opaque pass**: `OpaqueFeatures::film` is on only for parts whose material has `thin_film_amount > 0`, and the shared library's `override use_film` compiles the film out of every other pipeline. The pass evaluates the film twice per pixel, at the view angle and at normal incidence (`film_colour`), and hands both to `eval_filmed`, the local lights and the environment's split sum (`reflection_filmed`), where the shared `eval` and `reflection_colour` used to evaluate it once per call, five times a pixel.
- **Live glass**: the compose pass's front reflection, which had its own three-wavelength `film_color`.
- **The tracer**: opaque and metal films, and a filmed transmissive surface's glossy lobe, through the shared `eval`. The delta branch keeps `cf_surface` and its 81 wavelengths.

### How it was checked

`crates/live/tests/film.rs`:

- `the_live_film_matches_the_tracer_within_a_level` (in the gate) and `the_shader_film_matches_the_tracer_within_a_level` (GPU, a compute shader running `film.wgsl`): eight dielectric stacks (film 1.33 to 2.6 over 1 to 2.4) from 0 to 1000 nm and five metals (copper, gold, silver-grey, aluminium-like and a blue) under films of 1.5, 2.0 and 2.6 from 0 to 400 nm, every 5 nm at 0 to 70° every 5°: 42,345 samples against the tracer's `film_reflectance` and `conductor_reflectance`, within 1/255 in sRGB. The worst is 0.62/255 on the dielectrics (a 2.6 film over 1.5 at 70° and 910 nm) and 0.82/255 on the metals (a 2.6 film on aluminium at 70° and 305 nm), the same on the GPU, which is within 0.02/255 of the CPU mirror.
- `a_micron_film_does_not_band` (in the gate, and on the GPU in the test above): the dielectric stacks from 0.9 to 1.1 µm every 5 nm. The largest step between neighbouring thicknesses is 4.97/255, the tracer's own; the three-wavelength film's is 24.5. The worst difference from the tracer there is 0.89/255.
- `parts_without_film_draw_the_same_bytes` (GPU): a floor, a brass block, an oil slick on a dark lacquer and anodised titanium. Without film, the frame with the film compiled out and the one with it compiled into every part are the same bytes; with film it compiles into the two filmed parts only, and every pixel that changes lies on them.
- `the_film_costs_this_much_at_4k` (GPU): 40 parts covering a 3840 × 2160 frame, the median of 32 frames each. The opaque pass is 0.95 ms without film; with half the parts an oil slick (420 nm of index 1.45 over a lacquer of 1.5) and half anodised titanium (160 nm of 2.4) it is 3.54 ms. All oil slick costs 1.27 ms over the bare frame, all anodised titanium 3.76 ms: a metal takes three substrates per wavelength. The cost is per filmed pixel; a part without film pays nothing. Before the film was evaluated once per pixel and angle, the half-and-half frame cost 8.0 ms over the bare one.

Frames without film are byte-identical to 0.18.19's: a live frame of plain, metal, clear-coated and grimy parts, with every opaque feature compiled in and with none, and a traced frame of plain, metal, clear-coated, glass and sheen materials, hash the same before and after.
