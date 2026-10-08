# Text: fallback faces, CJK, subsetting and grazing views

`pfx_text` lays text out with cosmic-text 0.15 and draws it from the shared glyph atlas (`docs/live-renderer.md`, "Glyph atlas"). This page covers what happens when the primary face lacks a glyph: how the fallback face is picked, how it is matched to the primary in size and baseline, how CJK lines break, and how a game ships a small CJK font. It also covers how surface text keeps its weight when seen at a low angle. The examples use DM Sans as the primary face and Noto Sans SC as the CJK fallback, the two OFL fonts committed under `crates/text/tests/fonts`.

## Anchoring a label

`place_spans(spans, width, anchor, placement)` puts `placement.origin` where the anchor says:

- With a wrap `width`, the origin is the left edge of a box that wide, and `Anchor::Center` and `Anchor::End` align each line inside it (cosmic-text's alignment).
- With **no wrap width**, `Anchor::Start` puts the origin at the label's left edge, `Anchor::Center` puts the label's measured width centred on the origin and `Anchor::End` ends it there. The offset is the label's width (`Placed::width`, the widest line) times 0, 0.5 or 1, the same arithmetic `place_number` uses, so a label placed this way lands where `place_number` puts the same shaped label. Lines of a multi-line label stay aligned to each other inside that width (centred lines stay centred on the label's centre).
- `place_spans_anchors_a_label_with_no_wrap_width_as_place_number_does` compares the quads' pens with `place_number`'s and with the start-anchored label shifted by the width.

`render_spans` keeps its older behaviour: with no wrap width its anchor aligns lines inside the widest line only and does not move the origin.

## Registering fallbacks

```rust
let mut text = TextEngine::new(&dm_sans)?;
text.register_font(&noto_sans_sc)?;
text.set_fallbacks("DM Sans", None, &[Fallback::new("Noto Sans SC")])?;
text.set_fallbacks(
    "DM Sans",
    Some(800),
    &[Fallback { weight: Some(900), ..Fallback::new("Noto Sans SC") }],
)?;
```

- `set_fallbacks(primary, weight, faces)` sets an ordered list for one primary family. `weight: None` is the list for every weight. `Some(w)` overrides it for spans at exactly that weight. An empty list removes the entry.
- A `Fallback` names a registered family. `weight: None` follows the span's weight; `Some(w)` asks for a fixed one. `scale: None` measures the scale (see below); `Some(s)` sets it.
- Unknown families give `Error::InvalidFont`. A weight outside 1–1000 or a scale that is not finite and positive gives `Error::InvalidStyle`.
- `picks(&span)` returns what will draw each byte range of a span: family, weight and scale. The golden tests use it, and so can a game's own checks.
- `families()` lists the registered families; `face_metrics(family, weight)` returns ascent, descent, cap height and x-height in ems.

## How a glyph's face is chosen

The choice is made per grapheme cluster, before shaping, from the fonts' character maps:

1. The primary face, if it maps every character of the cluster.
2. Otherwise the first fallback in the list that maps every character.
3. Otherwise the first fallback that maps the cluster's base character.
4. Otherwise the primary, and cosmic-text's own search runs over the registered faces.

Zero-width joiners, variation selectors and control characters never decide a choice: they stay with their cluster, or with the previous run. Neighbouring clusters with the same choice form one run, so Latin keeps its kerning and ligatures across a word.

The choice depends only on the registered fonts and the lists. The engine's `FontSystem` is built with an empty platform fallback table (`fallback::Explicit`), so cosmic-text never reaches for "Noto Sans" or "DejaVu Sans" on Linux, or for other names on Windows and macOS. The same text gives the same faces on every machine and in any registration order (`layout_does_not_depend_on_the_order_fonts_were_registered`).

Punctuation follows the same rule. "，。、：！？（）《》「」" are not in DM Sans and come from the CJK face; "“”‘’—…" are in DM Sans and come from it. A game that wants full-width quotes in Chinese strings writes the full-width code points.

## Weight

The fallback's weight follows the span's weight unless the list fixes one:

- **Variable fallback** (a `wght` axis): the axis is set to the weight, clamped to the axis range. Noto Sans SC's variable font covers 100–900.
- **Static faces:** fontdb's CSS matching picks one (for 600 that is 700 if 700 exists, otherwise the nearest lighter face), and the span is shaped at that face's own weight.

## Size and baseline matching

**Scale.** A fallback glyph is drawn at `size × scale`. The measured scale is the primary's cap height over the fallback's (OS/2 `sCapHeight`), or the x-height ratio when a cap height is missing. It is rounded to four decimals. The fallback keeps its own designed proportion between ideographs and its own Latin, and that Latin then matches the primary's.

| | DM Sans | Noto Sans SC |
|---|---|---|
| cap height | 700 | 733 |
| x-height | 526 | 543 |
| x / cap | 0.751 | 0.741 |
| ideograph ink box (209 common ideographs, wght 400) | — | −72.7 to 821.9, height 894.6 |

Units are thousandths of an em. **The chosen scale is 0.955** (700 / 733). Because x / cap is nearly the same in both faces, matching cap heights also matches x-heights: 543 × 0.955 = 518.6 against 526. After scaling, an ideograph's ink is 854 units tall, 1.22 times the primary's cap height, the usual proportion for Chinese beside a Latin sans. Its ink centre sits at 358 units against the cap-height centre at 350, 0.8 % of the em (1.2 px at 150 px). So no baseline shift is applied.

**Baseline.** Every glyph sits on the line's alphabetic baseline, as ideographs are designed to. Left alone, cosmic-text would centre each line on the tallest ascent and descent among the faces on it. Noto Sans SC's (1000/200) differ from the primary's, so a line holding ideographs would sit lower than a Latin line. The engine recomputes the baseline of any line that holds fallback glyphs as if those glyphs were the primary's at `size / scale`, using the primary's ascent and descent. The baseline is therefore set by the primary alone: a Latin line, a mixed line and a line of ideographs at the same span size have the same baseline and height, and wrapped lines are spaced by exactly the line height. `Block::line_baselines()` returns them. Lines without fallback glyphs keep cosmic-text's value bit for bit.

**Line height** comes from the span's `Face::line`, which the fallback runs share, so a mixed line is as tall as a Latin one. Letter spacing is divided by the scale so its absolute size matches.

## Line breaking

cosmic-text breaks lines with `unicode-linebreak` 0.1.5, Unicode's line-breaking algorithm (UAX #14), and the engine keeps that:

- Ideographs break between each other without spaces (ID ÷ ID).
- Kinsoku follows UAX #14's classes:
  - closing punctuation, full stops, commas, "！" and "？" (CL, CP, IS, EX) never start a line;
  - "々", "ー", small kana and other non-starters (NS, and CJ resolved to NS) never start a line;
  - opening brackets (OP) never end one;
  - an ellipsis stays with what comes before it (IN).
- Latin words beside ideographs stay whole, and the boundary between a Latin word and an ideograph is a break opportunity.

A run that cannot break and is longer than the line, such as a word wider than the wrap width, is broken between glyphs (`Wrap::WordOrGlyph`). That is the only case where a line can start with a non-starter. The tests check UAX #14's opportunities for twelve CJK cases, then lay out a mixed paragraph at 16 widths and confirm every line starts at a UAX #14 opportunity. They also lay out a punctuation-heavy paragraph at 4 to 13 ideographs wide and confirm no line starts with a closing mark or ends with an opening one.

## The atlas under CJK load

CJK cells live on their own pages, since pages are per face and kind, and share the budget with everything else. Measured with the full Noto Sans SC, at 4K (scale 2), with tests that run by hand (`PITO_CJK_FONT`):

| Load | Pages | Bytes |
|---|---|---|
| 2,000 distinct ideographs as coverage, 800 at 19 px, 600 at 24, 350 at 32, 200 at 48, 50 at 75 | 12 | 15.9 MiB, inside the default 32 MiB |
| the same 2,000 as MSDF | 19 | 76.0 MiB |

The second and third frames rasterize and upload nothing, and nothing is evicted.

MSDF cells are size-free but large: an ideograph's 64 px field plus its border and padding takes an 88 px cell, about 121 to a 4 MiB page. A CJK-heavy screen should draw its UI text as coverage. A game that wants many ideographs as MSDF (surface text in 3D) sets the budget accordingly with `set_atlas_budget`.

**Scrolling.** A 20 × 20 window of 24 px ideographs at 4K moves one line a frame through 5,000 distinct ideographs (231 frames):

| Budget | Most rasters a frame | Most cell uploads a frame | Most bytes a frame | Rasterized in all | Evicted |
|---|---|---|---|---|---|
| 4 MiB | 20 | 20 | 108,800 | 5,000 | 4,242 |
| 32 MiB | 20 | 20 | 108,800 | 5,000 | 0 |

Each frame uploads only the line that scrolled in, and no glyph is rasterized twice. Least-recently-used eviction under a budget that holds the window does not thrash.

**Numbers stay the primary's.** `place_number(span, chars, anchor, placement)` takes the glyph set from the caller: a label made only of `chars` is laid out without shaping, and `chars` is shaped with the primary family only, and the fallback lists never touch it. A game passes its own list, for instance the digits and its separators. With a CJK fallback registered, a label still takes the fast path (no shaping after its face's first use) and lands on the same cells as without one.

The gate runs proxies of both tests with the committed subset, whose 231 ideographs at several weights stand in for distinct ideographs.

## Number sets: the fast path, ligatures and figures

`place_number(span, chars, anchor, placement)` lays out a label made only of the characters in `chars` without shaping it. The first use of a face shapes each character of the set alone, then shapes one sequence that holds every ordered pair once (an Eulerian circuit of the set) and records each pair's kerning. After that, a label is a sum of advances and pair kerns. `place_number_with(span, chars, forms, anchor, placement)` takes `NumberForms { figures, ligatures }`; `place_number` uses the default, `Figures::Tabular` with `Ligatures::Kept`.

**Ligatures.** A face can ligate pairs inside a set: EB Garamond forms ﬁ and ﬀ from "fi" and "ff", and a common UI face ligates fi, ft and ff. The engine records them pair by pair:

- A pair that comes out of the sequence as two glyphs, each the one its character shapes to alone, keeps the kern measured there.
- A pair that does not (a ligature, or a substitution caused by a neighbouring ligature) is shaped again as a two-character string. If that gives the two solo glyphs, its kern comes from there. Otherwise the pair is recorded as ligating.
- A label that contains a ligating pair falls back to full shaping, which forms the ligature as any other text would. Every other label of the set stays on the fast path. `ligating_pairs(face, chars, forms, scale)` lists them.

There are two choices, and the default is `Ligatures::Kept`:

- `Ligatures::Kept` draws the ligatures, so a label looks exactly as it does everywhere else in the game. The labels that hold a ligating pair are shaped once and then stay in the shaping cache (below), so they cost one shaping, not one a frame.
- `Ligatures::Off` shapes the set and its fallback labels with `liga` and `clig` off, so every label stays fast. It changes what is drawn ("fit" shows f, i and t), so the set's owner chooses it. A face can still form required ligatures (`rlig`) or contextual ones through other features; those pairs are recorded and fall back as above.

Before this change, one ligating pair anywhere in the set turned the fast path off for the whole set.

**Figures.**

- `Figures::Tabular` (the default) shapes the set with `tnum`. In a face with `tnum`, that also gives every other glyph the feature covers its tabular form; some faces give round brackets tabular widths.
- `Figures::TabularDigits` lays digits out in columns and leaves everything else proportional. cosmic-text applies OpenType features to a whole run, not to a range of characters inside a word, so the engine lays out the columns itself. It shapes without `tnum`, gives each digit the advance of the widest digit (all ten, shaped alone), centres the digit's own glyph in it, and drops kerning on either side of a digit, as fonts do for their tabular figures. "(11)" and "(88)" have the same width, and "()" keeps its proportional width.
- `Figures::Proportional` shapes without `tnum` and leaves digits alone. A label then lands exactly where `place_spans` puts it.

`Figures::Tabular` falls back to the same column layout when a face has no `tnum` (DM Sans, whose digits shaped with `tnum` still differ in width). Labels outside the set get the same columns, so "Score 1111" and "Score 8888" have the same width whichever path draws them. The column width comes from all ten digits, not only those in the set.

The tests compare the fast path with full shaping for every combination, in DM Sans and EB Garamond, in both representations (`warm_tests.rs`):

- `only_labels_with_a_ligating_pair_leave_the_fast_path`;
- `ligatures_off_keeps_ligating_labels_on_the_fast_path`;
- `proportional_figures_lay_out_as_plain_shaping`;
- `tabular_digits_keep_punctuation_proportional`;
- `tabular_digits_line_up_in_labels_that_fall_back`.

## Keeping shaped text warm

`end_frame` used to drop every shaped block and number face the frame had not used, so a menu that came back a frame later was shaped again. Now they stay under a byte budget, `set_shaping_budget(bytes)`, which defaults to `DEFAULT_SHAPING_BUDGET`, 8 MiB.

- At `end_frame`, if the cache holds more than the budget, it drops the least recently used blocks and number faces until it fits. Blocks and number faces share one budget and one order, the order of their last use.
- What the frame used always stays, even over the budget, so a frame never shapes the same text twice.
- A budget of 0 gives the old behaviour: everything the frame did not use is dropped.
- `shaping_stats()` returns the blocks, number faces, estimated bytes, budget and evictions.

A block's bytes are estimated from its glyph, line and span counts, the sizes of cosmic-text's glyph records and the text, times 7/4. The factor covers vector slack and the allocator, and was calibrated against the growth of resident memory (`cargo run --release -p pfx-text --example shaping_memory`, DM Sans at 24 px, scale 2, glibc):

| Labels | Glyphs a label | Estimate a label | Resident growth a label |
|---|---|---|---|
| 40,000 like "OK 39999" | 7.7 | 3,599 B | 3,918 B |
| 20,000 like "Menu entry 19999" | 15.4 | 6,099 B | 6,202 B |
| 5,000 like "Settings and controls page 4999" | 30.8 | 11,063 B | 10,015 B |
| 2,000 lines of 78 glyphs of help text | 78.4 | 26,495 B | 29,063 B |

A shaped label costs about 350 to 470 bytes a glyph, and a number face of 18 characters about 11 KB (most of it the 18 × 18 kerning table). The default 8 MiB keeps about 1,350 labels of 15 glyphs, or 300 lines of help text, warm: more than a game's whole UI on screen and in the menus it returns to. A game with more sets a larger budget.

## Prefilling the atlas

A game can warm its glyph cells before its first frame:

```rust
let made = text.prefill(&[Prefill {
    face: hud_face.clone(),
    chars: "0123456789,.:-+%".into(),
    scale: 2.0,
    representation: Representation::Msdf,
    bins: [true; 4],
    number: Some(NumberForms::default()),
}])?;
```

- Each entry names a face, its characters, the scale and the representation.
- `bins` picks the quarter-pixel positions to rasterize for `Representation::Coverage`; MSDF cells have none.
- `number: Some(forms)` prefills the glyphs `place_number_with` draws for that set and those forms (with `tnum`, a digit can be a different glyph). The set's number face is built too, so its shaping is warm.
- `number: None` prefills each character as `place_spans` draws it alone, through the fallback lists.

Prefill shapes on the caller's thread and lists the cells the atlas does not have yet. It generates them on worker threads (std threads, `prefill_on(list, threads)` to choose the count, `available_parallelism` by default), then inserts them on the caller's thread in list order: entry, then bin, then character. The atlas changes are the uploads, taken with `take_atlas_changes` as usual. The cells, the pages and the uploads are byte for byte those of placing the same characters one by one on demand, at any thread count. `prefilled_atlases_match_on_demand_byte_for_byte_at_any_thread_count` checks 1, 2, 5 and 32 threads against on-demand placement in the same order, and `prefill_with_every_bin_leaves_nothing_to_rasterize_on_demand` checks that labels drawn afterwards rasterize nothing.

`cargo run --release -p pfx-text --example prefill_speed` warms 376 MSDF fields (printable ASCII in four faces at scale 2):

| | Time |
|---|---|
| on demand, on the caller's thread | 732 ms |
| prefill on 1 thread | 730 ms |
| prefill on 24 threads | 65 ms |

**A faster field generator.** The field generator now gives the same bytes four to five times faster:

- It counts winding once a row: it finds the row's crossings through an index of edges by row, then sweeps them, instead of testing every edge at every pixel.
- It finds each channel's nearest edges in any order, pruned by the bounding boxes of groups of edges, starting from the previous pixel's nearest group. It then replays the original sequential choice, with its tie-break on the dot product, over only the edges that can change it. An edge farther than the chain of near-ties above the nearest distance plus 0.001 px can never be chosen, and cannot stop a nearer edge from being chosen, so dropping it changes nothing. A pixel whose chain of ties runs past 0.05 px falls back to the full loop.

`field_reference.rs` keeps the previous generator. `the_field_generator_matches_its_reference_byte_for_byte` compares every third of the 421 fields at 64 ppem in the gate, and `every_glyph_matches_the_reference` (run by hand) compares 8,200 fields: every glyph of DM Sans, EB Garamond and the CJK subset, at 64 ppem at 400 and 128 ppem at 800, with fake italics and quarter-pixel offsets. `field_speed`, run by hand in release with one test thread:

| Fields | Before | After |
|---|---|---|
| 421 at 64 ppem (Latin in DM Sans and EB Garamond at two weights, nine ideographs) | 5.87 ms | 1.36 ms |
| 421 at 128 ppem | 18.2 ms | 3.64 ms |

The "about 2.8 ms a field" a game measured on a 16-core desktop CPU was for its own UI face; that face was not measured here, but it goes through the same generator.

## Choosing the CJK face

Noto Sans SC (notofonts/noto-cjk, Sans2.004, `Variable/TTF/Subset/NotoSansSC-VF.ttf`) and Source Han Sans CN (adobe-fonts/source-han-sans, 2.005R, `Variable/TTF/Subset/SourceHanSansCN-VF.ttf`) were compared beside a Latin primary at 19–150 px at 1080p and at twice that at 4K, at 400, 600 and 800 (`crates/text/examples/cjk_compare.rs`).

| | Noto Sans SC | Source Han Sans CN |
|---|---|---|
| vertical stem, "l", 400 / 600 / 800 | 91 / 131 / 162 | same |
| horizontal stroke, "一", 400 / 600 / 800 | 82 / 117 / 145 | same |
| ink per em², 400 / 600 / 800 (209 ideographs) | 0.265 / 0.367 / 0.442 | same |
| line metrics (ascent / descent) | 1000 / 200 | 1160 / 288 |
| variable TTF, full | 17.77 MB | 17.75 MB |
| GB 2312 level 1 subset | 1.87 MB | 1.86 MB |
| licence | OFL, no reserved name | OFL, Reserved Font Name "Source" |

The two are the same design: the outlines measure identically at every weight. The differences lie elsewhere:

- **Licence:** Source Han Sans reserves the name "Source". A subset is a Modified Version under the OFL, so a shipped Source Han Sans subset would have to be renamed. Noto Sans SC has no reserved name and ships under its own name.
- **Line metrics:** Noto's are tighter and closer to a typical Latin face's (the engine's baseline matching makes this cosmetic).
- **Version:** Source Han Sans 2.005 is newer; its changes do not touch the GB 2312 repertoire.

**Recommendation: Noto Sans SC, the variable TTF from notofonts.** It has the same design as Source Han Sans and no renaming. One variable file serves every weight, and it subsets with a pure Rust tool. Its stroke contrast (horizontals about 10 % lighter than verticals) is close to a Latin sans's own, so the two read as one texture at 19 px and up.

**Weight.** At the same nominal weight the ideographs' strokes are lighter than a bold Latin face's. A game whose primary looks heavier at 600 and 800 can map those weights to 700 and 900 in the fallback list, and checks the comparison sheets (`cjk_compare`) against its own primary:

```rust
let noto = |weight| Fallback { weight: Some(weight), ..Fallback::new("Noto Sans SC") };
text.set_fallbacks("DM Sans", None, &[Fallback::new("Noto Sans SC")])?;
text.set_fallbacks("DM Sans", Some(600), &[noto(700)])?;
text.set_fallbacks("DM Sans", Some(800), &[noto(900)])?;
```

At 19 px at 1080p the densest ideographs at 900 start to close their counters. If that shows, use 800 for 800 below 24 px.

## Subsetting

With the `subset` feature, the text crate cuts a font down to the characters a game uses. The feature is off by default, so apps never link the subsetter.

- `subset::subset(font, &chars, options)` subsets a font. It uses skera 0.8.0, googlefonts' Rust port of HarfBuzz's subsetter from fontations, pinned exactly.
  - It keeps `cmap`, `GSUB`/`GPOS` (with closure over substitutions, so contextual and localized forms survive), `glyf`/`gvar`/`HVAR` (a variable font stays variable), metrics, `OS/2` and the name records 0–6, 13 and 14 (the licence).
  - The typst `subsetter` crate the task suggested drops `cmap` and layout tables by design (PDF embedding only), so it cannot serve a game.
- `subset::gb2312_level1()` returns GB 2312's level 1, 3,755 ideographs from 啊 to 座, decoded with encoding_rs.
- `subset::command(args)` is the command-line form. Today it runs as `cargo run --release -p pfx-text --features subset --example font_subset -- --font <ttf> --chars <file> --out <ttf> [--base gb2312-1] [--no-closure]`. `pfx font subset` takes the same arguments once the root package wires it in.

Only TrueType outlines subset. The CFF and CFF2 OTFs are refused with a clear message, so use the variable TTF.

The subset keeps exactly the requested characters that the font maps (`the_subset_keeps_exactly_the_requested_characters`; without layout closure the glyph count is those characters plus `.notdef`). Outlines match the source exactly at the default instance and to within one font unit at every other weight (`the_subset_keeps_every_outline_at_every_weight`).

**Shipped sizes** of Noto Sans SC:

| | Size | Covers |
|---|---|---|
| variable TTF, full | 17.77 MB | every weight 100–900 |
| variable TTF, GB 2312 level 1 + Latin digits and test strings | 1.90 MB | every weight; about 0.63 MB a weight at three weights |
| variable TTF, the 302 characters of the test strings | 120 KB | every weight |
| static OTF, full, Regular 400 / Medium 500 / Bold 700 / Black 900 | 8.33 / 8.35 / 8.54 / 8.86 MB | one weight each; CFF, so they cannot be subset here |

A game ships the variable subset: one file for all three weights, smaller than any single static face.

## Surface text at grazing views

Live text on a surface draws from the MSDF atlas (`Representation::Msdf`). Its coverage turns the field's signed distance into screen pixels. Seen face on, one scale does: the distance over its own screen derivative (`fwidth`), clamped around one half. On a plane seen at a low angle, a screen pixel covers a long thin strip of the glyph, and one sample in the middle of it point-samples the compressed axis. Bold strokes then break into aliased hairlines, and live and traced text part ways (the trace averages its samples over the pixel).

**Both screen derivatives.** The text shader takes the glyph UV's two screen derivatives in atlas texels, `jx` and `jy`, the columns of the screen-to-texel Jacobian `J`. The singular values of `J` are the footprint's major and minor extents in texels, and the eigenvector of `JᵀJ` for the larger one is the screen direction the plane is compressed along. Their ratio is the anisotropy: 1 face on, about `1 / sin θ` for a plane seen `θ` above it.

**Taps along the major axis.** Where the anisotropy passes 1.2, the coverage comes from `n = ceil(major / max(minor, 1))` taps, at most 8, spread evenly across the pixel along the major axis. Each tap reads the signed distance in texels (`(median − 0.5) × MSDF_SPREAD`, with `MSDF_SPREAD` = 8 from the text crate) and the field's own gradient `g` from two neighbouring texels. That gradient comes from the texture, not from screen differences, so it stays right where neighbouring pixels land on different strokes. The tap's box is `1 / n` of a pixel along the major axis and one pixel across, so its filter width is `|g · J·major| / n + |g · J·minor|`, and its coverage is the distance over that width, clamped around one half. The pixel's coverage is the mean of its taps. This box-filters the compressed axis rather than point-sampling it, and the minor axis keeps the face-on sharpness.

**Face on is unchanged.** Below an anisotropy of 1.2 the shader keeps the isotropic formula exactly. Between 1.2 and 1.5 it blends towards the filtered coverage, so a turning plane never pops, and from 1.5 (a plane under about 42°) the filtered coverage alone draws. The check is on squared terms, so face-on text pays three dot products and a square root. Overlay text is screen space and never anisotropic: its pipelines use `fs_overlay` and `fs_overlay_id`, which compile the filtered path out. `unlit_text_is_byte_identical_to_the_shader_before_lit_text` still matches bit for bit. `surface_text_seen_nearly_face_on_keeps_the_isotropic_coverage` draws surface text at 90°, 85° and 80° with the shader from before this change and changes no value. The lit and id passes share the same coverage.

**Against the tracer.** `crates/live/tests/grazing_text.rs` lays a pangram on a floor in DM Sans and EB Garamond (the engine's own OFL fonts) at 400 and 700, with an em of 40 and of 20 px face on. Both sizes draw from the 128 px field. It views them from 90°, 45°, 25° and 10° and compares live with the tracer's lettering at 64 samples. IoU is taken over the pixels at or above half coverage; ink is the summed coverage of live over traced.

| View | Worst IoU, before | Worst IoU, after | Worst bold IoU, before | Worst bold IoU, after |
|---|---|---|---|---|
| 90° | 0.917 | 0.917 | 0.935 | 0.935 |
| 45° | 0.862 | 0.881 | 0.934 | 0.942 |
| 25° | 0.777 | 0.890 | 0.825 | 0.911 |
| 10° | 0.465 | 0.812 | 0.660 | 0.815 |

Live keeps 0.91 to 1.05 of the traced ink at every angle, where it kept 0.55 to 1.05 before. The worst cases are the 20 px em regular serif, whose face-on IoU of 0.917 is the ceiling: there one pixel spans six texels of the field even face on. The test asserts bold at 0.9 or more down to 25°, every case within 0.05 of its own face-on IoU at 45° and 25°, 0.78 or more at 10°, and the ink within 15 %.

**Cost.** `a_page_of_msdf_text_at_4k_face_on_and_tilted` draws 5,504 glyphs of 36 px DM Sans, a page at 4K, through the text pass on the RX 9060 XT, median of ten frames:

| Page | Before | After |
|---|---|---|
| overlay | 0.073 ms | 0.075 ms |
| surface, face on | 0.075 ms | 0.086 ms |
| surface, 45° | 0.039 ms | 0.104 ms |
| surface, 25° | 0.029 ms | 0.103 ms |
| surface, 10° | 0.016 ms | 0.090 ms |

A tilted page covers fewer pixels, so it cost less than face on before. It now costs about as much as a face-on page, because each pixel reads up to 8 taps of three texels. Face-on surface text pays 0.011 ms a page, the cost of keeping the filtered path in its shader.

## Tests and fonts

- `crates/text/src/fallback_tests.rs` holds the picks (golden), the scale and baseline, the line-break cases, the subsetter, the atlas under CJK load and the number fast path.
- `crates/text/src/warm_tests.rs` holds the ligating pairs, the figures, the shaping cache's budget and eviction order, and prefill against on demand. `crates/text/src/field_reference.rs` holds the previous field generator and the tests that compare it with the current one.
- `crates/text/tests/fonts/NotoSansSC-subset.ttf` (120 KB, 302 characters, `wght` kept) is cut from Noto Sans SC 2.004 by the subsetter itself. `NotoSansSC-OFL.txt` is its licence.
- DM Sans, EB Garamond and Noto Sans SC are committed under their OFL licences, and the tests use only those. The tests that need the full CJK face read `PITO_CJK_FONT=<path to NotoSansSC-VF.ttf>`. Without it they print that they were skipped and pass.

```sh
PITO_CJK_FONT=$PWD/tmp/fonts/NotoSansSC-VF.ttf \
  cargo test --release -p pfx-text --lib -- full_face --nocapture
cargo run --release -p pfx-text --example cjk_compare -- tmp \
  crates/text/tests/fonts/DMSans\[opsz,wght\].ttf tmp/fonts/NotoSansSC-VF.ttf
```
