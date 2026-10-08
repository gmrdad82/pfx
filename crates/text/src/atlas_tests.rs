use std::collections::BTreeMap;

use crate::{
    Anchor, Atlas, AtlasChanges, Error, Face, Figures, MipLevel, NumberForms, PageQuad, Placed,
    Placement, Representation, Span, TextEngine,
};

const NUMBERS: &str = "0123456789,.e+-Na";
const SANS: &[u8] = include_bytes!("../tests/fonts/DMSans[opsz,wght].ttf");
const SERIF: &[u8] = include_bytes!("../fonts/EBGaramond[wght].ttf");
const MONO: &[u8] = include_bytes!("../tests/fonts/IBMPlexMono-Regular.ttf");

fn face(family: &str, size: f32, weight: u16) -> Face {
    Face {
        family: family.into(),
        size,
        line: size * 1.25,
        weight,
        italic: false,
        spacing: 0.0,
    }
}

fn span(text: &str, face: &Face) -> Span {
    Span {
        text: text.into(),
        face: face.clone(),
        color: [0.9, 0.8, 0.7, 1.0],
    }
}

fn placement(scale: f32, representation: Representation, origin: [f32; 2]) -> Placement {
    Placement {
        scale,
        representation,
        origin,
        alpha: 1.0,
        clip: [-f32::MAX, -f32::MAX, f32::MAX, f32::MAX],
        turn: [0.0; 3],
    }
}

#[derive(Default)]
struct Mirror {
    pages: BTreeMap<u32, Atlas>,
}

impl Mirror {
    fn apply(&mut self, changes: &AtlasChanges) {
        for id in &changes.dropped {
            assert!(self.pages.remove(id).is_some(), "dropped an unknown page");
        }
        for page in &changes.created {
            let levels = (0..page.levels)
                .map(|level| {
                    let side = page.size >> level;
                    MipLevel {
                        width: side,
                        height: side,
                        bytes: vec![0; (side * side * page.channels) as usize],
                    }
                })
                .collect();
            self.pages.insert(
                page.id,
                Atlas {
                    channels: page.channels,
                    levels,
                },
            );
        }
        for cell in &changes.cells {
            let atlas = self
                .pages
                .get_mut(&cell.page)
                .expect("upload to an unknown page");
            let channels = atlas.channels as usize;
            let level = &mut atlas.levels[cell.level as usize];
            let [x, y, w, h] = cell.rect;
            assert_eq!(cell.bytes.len(), (w * h) as usize * channels);
            for row in 0..h {
                let to = (((y + row) * level.width + x) as usize) * channels;
                let from = (row * w) as usize * channels;
                level.bytes[to..to + w as usize * channels]
                    .copy_from_slice(&cell.bytes[from..from + w as usize * channels]);
            }
        }
    }

    fn matches(&self, engine: &TextEngine) {
        assert_eq!(self.pages.len(), engine.atlas_stats().pages);
        for (id, atlas) in &self.pages {
            let page = engine
                .atlas_page(*id)
                .expect("mirror holds a page the engine dropped");
            assert_eq!(atlas.levels, page.levels, "page {id}");
        }
    }
}

fn cell_bytes(atlas: &Atlas, quad: &PageQuad) -> Vec<u8> {
    let level = &atlas.levels[0];
    let channels = atlas.channels as usize;
    let size = level.width as f32;
    let [x0, y0, x1, y1] = quad.quad.uv.map(|v| (v * size).round() as u32);
    let mut bytes = Vec::new();
    for row in y0..y1 {
        let start = ((row * level.width + x0) as usize) * channels;
        bytes.extend_from_slice(&level.bytes[start..start + (x1 - x0) as usize * channels]);
    }
    bytes
}

struct Numbers(u64);

impl Numbers {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn label(&mut self) -> String {
        let pick = self.next() % 8;
        let value = self.next();
        match pick {
            0 => "NaN".into(),
            1..=3 => {
                let n = value % 10_000;
                let text = if n >= 1000 {
                    format!("{},{:03}", n / 1000, n % 1000)
                } else {
                    n.to_string()
                };
                if pick == 3 { format!("-{text}") } else { text }
            }
            4 => format!("+{}", value % 1000),
            _ => {
                let mantissa = value % 90 + 10;
                let exponent = value % 300 + 4;
                let sign = if pick == 5 { "-" } else { "" };
                format!("{}.{}e{sign}{exponent}", mantissa / 10, mantissa % 10)
            }
        }
    }
}

#[test]
fn eighty_labels_change_every_frame_for_a_thousand_frames_within_the_budget() {
    let mut engine = TextEngine::new(SANS).unwrap();
    let budget = 12 << 20;
    engine.set_atlas_budget(budget);
    let scale = 2.0;
    let kinds = [
        (face("DM Sans", 19.0, 400), Representation::Coverage),
        (face("DM Sans", 24.0, 600), Representation::Msdf),
        (face("DM Sans", 40.0, 800), Representation::Msdf),
        (face("DM Sans", 60.0, 400), Representation::Msdf),
        (face("DM Sans", 32.0, 600), Representation::Coverage),
    ];
    let mut mirror = Mirror::default();
    engine.begin_frame();
    for (index, (face, representation)) in kinds.iter().enumerate() {
        for quarter in 0..4 {
            let origin = [quarter as f32 * 0.125, index as f32 * 80.0];
            engine
                .place_number(
                    &span(NUMBERS, face),
                    NUMBERS,
                    Anchor::Start,
                    &placement(scale, *representation, origin),
                )
                .unwrap();
        }
    }
    engine.end_frame();
    mirror.apply(&engine.take_atlas_changes());
    mirror.matches(&engine);
    let warm = engine.atlas_stats();
    assert!(warm.bytes <= budget, "{warm:?}");
    let mut numbers = Numbers(85);
    for frame in 0..1000 {
        engine.begin_frame();
        for label in 0..80 {
            let (face, representation) = &kinds[label % kinds.len()];
            let text = numbers.label();
            let origin = [
                (label / 10) as f32 * 117.3 + frame as f32 * 0.37,
                (label % 10) as f32 * 61.7,
            ];
            let anchor = [Anchor::Start, Anchor::Center, Anchor::End][label % 3];
            let placed = engine
                .place_number(
                    &span(&text, face),
                    NUMBERS,
                    anchor,
                    &placement(scale, *representation, origin),
                )
                .unwrap();
            assert_eq!(placed.quads.len(), text.len());
        }
        engine.end_frame();
        let changes = engine.take_atlas_changes();
        assert!(changes.is_empty(), "frame {frame} uploaded {changes:?}");
        let stats = engine.atlas_stats();
        assert!(stats.bytes <= budget);
        assert_eq!(stats.shaped, warm.shaped, "frame {frame} shaped a label");
        assert_eq!(stats.rasterized, warm.rasterized);
    }
    let stats = engine.atlas_stats();
    assert_eq!(stats.fast, warm.fast + 80_000);
    assert_eq!(stats.evicted, 0);
    assert_eq!(engine.cached_blocks(), 0);
    assert_eq!(engine.cached_number_faces(), kinds.len());
}

#[test]
fn a_tight_budget_evicts_the_least_recently_used_and_uploads_them_again() {
    let mut engine = TextEngine::new(MONO).unwrap();
    engine.set_atlas_page(256).unwrap();
    engine.set_atlas_budget(2 * crate::PageKind::Coverage.page_bytes(256));
    let mut mirror = Mirror::default();
    let mut fresh = TextEngine::new(MONO).unwrap();
    let plain = placement(1.0, Representation::Coverage, [0.0, 0.0]);
    let frame = |engine: &mut TextEngine, mirror: &mut Mirror, text: &str, size: f32| {
        engine.begin_frame();
        let placed = engine
            .place_spans(
                &[span(text, &face("IBM Plex Mono", size, 400))],
                None,
                Anchor::Start,
                &plain,
            )
            .map(|placed| (placed.quads.len(), placed));
        engine.end_frame();
        mirror.apply(&engine.take_atlas_changes());
        mirror.matches(engine);
        placed
    };
    let (_, first) = frame(&mut engine, &mut mirror, "ABC", 40.0).unwrap();
    let before = first
        .quads
        .iter()
        .map(|quad| cell_bytes(engine.atlas_page(quad.page).unwrap(), quad))
        .collect::<Vec<_>>();
    let filler = "DEFGHIJKLMNOPQRSTUVWXYZ";
    let mut evicted = 0;
    for size in [40.0, 41.0, 42.0, 43.0, 44.0, 45.0] {
        let (count, _) = frame(&mut engine, &mut mirror, filler, size).unwrap();
        assert_eq!(count, filler.len());
        evicted = engine.atlas_stats().evicted;
        assert!(engine.atlas_stats().bytes <= 2 * crate::PageKind::Coverage.page_bytes(256));
    }
    assert!(evicted > 0);
    let uploaded = engine.atlas_stats().uploaded;
    let (_, again) = frame(&mut engine, &mut mirror, "ABC", 40.0).unwrap();
    assert!(engine.atlas_stats().uploaded > uploaded);
    let reference = fresh
        .place_spans(
            &[span("ABC", &face("IBM Plex Mono", 40.0, 400))],
            None,
            Anchor::Start,
            &plain,
        )
        .unwrap();
    for ((quad, old), expected) in again.quads.iter().zip(&before).zip(&reference.quads) {
        let bytes = cell_bytes(engine.atlas_page(quad.page).unwrap(), quad);
        assert_eq!(&bytes, old);
        assert_eq!(
            bytes,
            cell_bytes(fresh.atlas_page(expected.page).unwrap(), expected)
        );
        assert_eq!(quad.quad.rect, expected.quad.rect);
    }
}

#[test]
fn cells_drawn_this_frame_are_never_evicted() {
    let mut engine = TextEngine::new(MONO).unwrap();
    engine.set_atlas_page(128).unwrap();
    engine.set_atlas_budget(crate::PageKind::Coverage.page_bytes(128));
    let plain = placement(1.0, Representation::Coverage, [0.0, 0.0]);
    engine.begin_frame();
    let first = engine
        .place_spans(
            &[span("AB", &face("IBM Plex Mono", 40.0, 400))],
            None,
            Anchor::Start,
            &plain,
        )
        .unwrap();
    let held = first
        .quads
        .iter()
        .map(|quad| cell_bytes(engine.atlas_page(quad.page).unwrap(), quad))
        .collect::<Vec<_>>();
    let overflow = engine.place_spans(
        &[span(
            "CDEFGHIJKLMNOPQRSTUVWXYZ",
            &face("IBM Plex Mono", 40.0, 400),
        )],
        None,
        Anchor::Start,
        &plain,
    );
    assert_eq!(overflow.err(), Some(Error::AtlasTooLarge));
    for (quad, bytes) in first.quads.iter().zip(held) {
        assert_eq!(
            cell_bytes(engine.atlas_page(quad.page).unwrap(), quad),
            bytes
        );
    }
    assert_eq!(engine.atlas_stats().evicted, 0);
}

fn shaped_tabular(
    engine: &mut TextEngine,
    text: &str,
    face: &Face,
    placement: &Placement,
) -> Placed {
    let spans = [span(text, face)];
    let key = engine
        .layout_block(
            &spans,
            None,
            placement.scale,
            Anchor::Start,
            Some(NumberForms::default()),
            None,
        )
        .unwrap();
    engine.place_block(key, &spans, placement).unwrap()
}

#[test]
fn the_fast_path_lays_numbers_out_as_tabular_shaping_does() {
    let digits = ["1,284", "4.2e9", "-0.5e+3", "7", "11,111"];
    let letters = ["NaN", "Na-e+.,", "e+-"];
    for (bytes, family, tnum) in [(SANS, "DM Sans", false), (SERIF, "EB Garamond", true)] {
        let mut engine = TextEngine::new(bytes).unwrap();
        for weight in [400, 800] {
            let face = face(family, 31.0, weight);
            for representation in [Representation::Coverage, Representation::Msdf] {
                let at = placement(2.0, representation, [3.3, 7.1]);
                for text in letters.iter().chain(digits.iter().filter(|_| tnum)) {
                    let before = engine.atlas_stats();
                    let fast = engine
                        .place_number(&span(text, &face), NUMBERS, Anchor::Start, &at)
                        .unwrap();
                    let after = engine.atlas_stats();
                    assert_eq!(after.fast, before.fast + 1, "{text}");
                    let shaped = shaped_tabular(&mut engine, text, &face, &at);
                    assert_eq!(fast.quads.len(), shaped.quads.len(), "{text}");
                    assert!((fast.width - shaped.width).abs() < 1e-3, "{text} {family}");
                    assert!((fast.baseline - shaped.baseline).abs() < 1e-4, "{text}");
                    assert!((fast.height - shaped.height).abs() < 1e-4, "{text}");
                    for (a, b) in fast.quads.iter().zip(&shaped.quads) {
                        assert_eq!(a.page, b.page, "{text}");
                        assert_eq!(a.quad.uv, b.quad.uv, "{text}");
                        for k in 0..2 {
                            assert!((a.quad.pen[k] - b.quad.pen[k]).abs() < 1e-3, "{text}");
                        }
                        for k in 0..4 {
                            assert!((a.quad.rect[k] - b.quad.rect[k]).abs() < 1e-3, "{text}");
                        }
                        assert_eq!(a.quad.color, b.quad.color);
                    }
                }
            }
        }
    }
}

#[test]
fn digits_without_tnum_are_centred_in_the_widest_advance() {
    let mut engine = TextEngine::new(SANS).unwrap();
    let face = face("DM Sans", 31.0, 400);
    let at = placement(1.0, Representation::Msdf, [0.0, 0.0]);
    let solo = "0123456789".chars().map(|c| {
        let text = c.to_string();
        shaped_tabular(&mut engine, &text, &face, &at).width
    });
    let widths = solo.collect::<Vec<_>>();
    let widest = widths.iter().copied().fold(0.0f32, f32::max);
    assert!(widths.iter().any(|&w| w < widest - 1.0), "{widths:?}");
    let fast = engine
        .place_number(&span("1104", &face), NUMBERS, Anchor::Start, &at)
        .unwrap();
    let shaped = shaped_tabular(&mut engine, "1104", &face, &at);
    assert!((fast.width - widest * 4.0).abs() < 1e-3);
    for (index, (a, b)) in fast.quads.iter().zip(&shaped.quads).enumerate() {
        let digit = "1104".as_bytes()[index] - b'0';
        let own = widths[digit as usize];
        let expected = index as f32 * widest + (widest - own) * 0.5;
        assert!((a.quad.pen[0] - expected).abs() < 1e-3);
        assert_eq!(a.quad.key, b.quad.key);
        assert_eq!(a.quad.rect[2], b.quad.rect[2]);
    }
}

#[test]
fn digits_take_the_same_advance_and_anchors_move_the_label() {
    let mut engine = TextEngine::new(SANS).unwrap();
    let face = face("DM Sans", 24.0, 600);
    let at = placement(1.0, Representation::Msdf, [100.0, 0.0]);
    let widths = ["0000", "1111", "4444", "8888"].map(|text| {
        engine
            .place_number(&span(text, &face), NUMBERS, Anchor::Start, &at)
            .unwrap()
            .width
    });
    for width in widths {
        assert!((width - widths[0]).abs() < 1e-4, "{widths:?}");
    }
    let start = engine
        .place_number(&span("1,284", &face), NUMBERS, Anchor::Start, &at)
        .unwrap();
    let end = engine
        .place_number(&span("1,284", &face), NUMBERS, Anchor::End, &at)
        .unwrap();
    let centre = engine
        .place_number(&span("1,284", &face), NUMBERS, Anchor::Center, &at)
        .unwrap();
    assert!((start.quads[0].quad.pen[0] - end.quads[0].quad.pen[0] - start.width).abs() < 1e-3);
    assert!(
        (start.quads[0].quad.pen[0] - centre.quads[0].quad.pen[0] - start.width * 0.5).abs() < 1e-3
    );
    let other = engine
        .place_number(&span("apples 12", &face), NUMBERS, Anchor::End, &at)
        .unwrap();
    assert_eq!(other.quads.len(), 8);
    assert!((other.quads[0].quad.pen[0] + other.width - 100.0).abs() < 1e-3);
    let ones = engine
        .place_number(&span("apples 11", &face), NUMBERS, Anchor::End, &at)
        .unwrap();
    assert!((ones.width - other.width).abs() < 1e-3);
}

#[test]
fn shared_coverage_cells_and_quads_match_render_spans() {
    let mut legacy = TextEngine::new(SERIF).unwrap();
    let mut shared = TextEngine::new(SERIF).unwrap();
    let spans = [
        span("Ledger 1,284", &face("EB Garamond", 22.0, 500)),
        span(" ffi été", &face("EB Garamond", 30.0, 700)),
    ];
    for origin in [[0.0, 0.0], [10.25, 3.6], [-4.6, 1.1]] {
        let old = legacy
            .render_spans(
                &spans,
                Some(160.0),
                1.5,
                Anchor::Start,
                Representation::Coverage,
                origin,
                0.8,
                [0.0, 0.0, 400.0, 400.0],
                [0.1, 0.2, 0.3],
            )
            .unwrap();
        let new = shared
            .place_spans(
                &spans,
                Some(160.0),
                Anchor::Start,
                &Placement {
                    scale: 1.5,
                    representation: Representation::Coverage,
                    origin,
                    alpha: 0.8,
                    clip: [0.0, 0.0, 400.0, 400.0],
                    turn: [0.1, 0.2, 0.3],
                },
            )
            .unwrap();
        assert_eq!(old.quads.len(), new.quads.len());
        for (a, b) in old.quads.iter().zip(&new.quads) {
            assert_eq!(a.key, b.quad.key);
            assert_eq!(a.rect, b.quad.rect);
            assert_eq!(a.pen, b.quad.pen);
            assert_eq!(a.color, b.quad.color);
            assert_eq!(a.line_index, b.quad.line_index);
            let level = &old.atlas.levels[0];
            let [x0, y0, x1, y1] = [
                (a.uv[0] * level.width as f32).round() as u32,
                (a.uv[1] * level.height as f32).round() as u32,
                (a.uv[2] * level.width as f32).round() as u32,
                (a.uv[3] * level.height as f32).round() as u32,
            ];
            let mut bytes = Vec::new();
            for row in y0..y1 {
                let start = (row * level.width + x0) as usize;
                bytes.extend_from_slice(&level.bytes[start..start + (x1 - x0) as usize]);
            }
            assert_eq!(bytes, cell_bytes(shared.atlas_page(b.page).unwrap(), b));
        }
    }
}

#[test]
fn msdf_glyphs_share_one_cell_across_sizes_and_positions() {
    let mut engine = TextEngine::new(SANS).unwrap();
    engine.begin_frame();
    for (size, x) in [(19.0, 0.0), (24.0, 0.3), (40.0, 7.77), (60.0, 1.5)] {
        engine
            .place_number(
                &span("1,284", &face("DM Sans", size, 400)),
                NUMBERS,
                Anchor::Start,
                &placement(1.0, Representation::Msdf, [x, 0.0]),
            )
            .unwrap();
    }
    let opsz_sizes = engine.atlas_stats().cells;
    assert_eq!(opsz_sizes, 5 * 3);
    let mut engine = TextEngine::new(SERIF).unwrap();
    for (size, x) in [(19.0, 0.0), (24.0, 0.3), (40.0, 7.77), (60.0, 1.5)] {
        engine
            .place_number(
                &span("1,284", &face("EB Garamond", size, 400)),
                NUMBERS,
                Anchor::Start,
                &placement(2.0, Representation::Msdf, [x, 0.0]),
            )
            .unwrap();
    }
    assert_eq!(engine.atlas_stats().cells, 5);
    engine
        .place_number(
            &span("1,284", &face("EB Garamond", 75.0, 400)),
            NUMBERS,
            Anchor::Start,
            &placement(2.0, Representation::Msdf, [0.0, 0.0]),
        )
        .unwrap();
    assert_eq!(engine.atlas_stats().cells, 10);
}

#[test]
fn a_new_font_clears_the_shared_atlas_and_drops_its_pages() {
    let mut engine = TextEngine::new(MONO).unwrap();
    engine
        .place_number(
            &span("42", &face("IBM Plex Mono", 20.0, 400)),
            NUMBERS,
            Anchor::Start,
            &placement(1.0, Representation::Msdf, [0.0, 0.0]),
        )
        .unwrap();
    let created = engine.take_atlas_changes();
    assert_eq!(created.created.len(), 1);
    engine.register_font(SANS).unwrap();
    let dropped = engine.take_atlas_changes();
    assert_eq!(dropped.dropped, vec![created.created[0].id]);
    assert_eq!(engine.atlas_stats().pages, 0);
    assert_eq!(engine.cached_number_faces(), 0);
}

#[test]
fn per_paragraph_atlases_expire_after_an_unused_frame() {
    let mut engine = TextEngine::new(MONO).unwrap();
    let render = |engine: &mut TextEngine, text: &str| {
        engine
            .render_spans(
                &[span(text, &face("IBM Plex Mono", 20.0, 400))],
                None,
                1.0,
                Anchor::Start,
                Representation::Msdf,
                [0.0; 2],
                1.0,
                [0.0; 4],
                [0.0; 3],
            )
            .unwrap()
    };
    engine.begin_frame();
    for n in 0..20 {
        render(&mut engine, &format!("{n}"));
    }
    engine.end_frame();
    assert_eq!(engine.cached_atlases(), 19);
    engine.begin_frame();
    let kept = render(&mut engine, "7");
    engine.end_frame();
    assert_eq!(engine.cached_atlases(), 1);
    assert_eq!(engine.cached_rasters(), 1);
    engine.begin_frame();
    let again = render(&mut engine, "7");
    engine.end_frame();
    assert_eq!(kept.atlas.levels, again.atlas.levels);
}

#[test]
fn place_spans_anchors_a_label_with_no_wrap_width_as_place_number_does() {
    let mut engine = TextEngine::new(SANS).unwrap();
    let face = face("DM Sans", 24.0, 600);
    for representation in [Representation::Msdf, Representation::Coverage] {
        let at = placement(1.0, representation, [100.0, 20.0]);
        let spans = [span("1,284", &face)];
        let start = engine
            .place_spans(&spans, None, Anchor::Start, &at)
            .unwrap();
        for (anchor, share) in [(Anchor::Center, 0.5), (Anchor::End, 1.0)] {
            let placed = engine.place_spans(&spans, None, anchor, &at).unwrap();
            let proportional = NumberForms {
                figures: Figures::Proportional,
                ..NumberForms::default()
            };
            let number = engine
                .place_number_with(&spans[0], "xyz", proportional, anchor, &at)
                .unwrap();
            assert_eq!(placed.quads.len(), start.quads.len());
            assert_eq!(placed.quads.len(), number.quads.len());
            assert!((placed.width - start.width).abs() < 1e-4);
            for ((a, b), c) in placed.quads.iter().zip(&start.quads).zip(&number.quads) {
                let moved = b.quad.pen[0] - a.quad.pen[0];
                assert!(
                    (moved - start.width * share).abs() < 1e-3,
                    "{anchor:?} moved {moved}"
                );
                assert_eq!(a.quad.pen[1], b.quad.pen[1]);
                assert!((a.quad.pen[0] - c.quad.pen[0]).abs() < 1e-3);
                assert!((a.quad.pen[1] - c.quad.pen[1]).abs() < 1e-3);
            }
        }
    }
}

#[test]
fn place_spans_anchors_every_line_of_a_multi_line_label_in_its_widest_line() {
    let mut engine = TextEngine::new(SANS).unwrap();
    let face = face("DM Sans", 24.0, 400);
    let at = placement(1.0, Representation::Msdf, [200.0, 0.0]);
    let spans = [span("wide label here\nab", &face)];
    let start = engine
        .place_spans(&spans, None, Anchor::Start, &at)
        .unwrap();
    let centre = engine
        .place_spans(&spans, None, Anchor::Center, &at)
        .unwrap();
    let left = |placed: &Placed| {
        placed
            .quads
            .iter()
            .map(|q| q.quad.pen[0])
            .fold(f32::MAX, f32::min)
    };
    let right = |placed: &Placed| {
        placed
            .quads
            .iter()
            .map(|q| q.quad.pen[0])
            .fold(f32::MIN, f32::max)
    };
    assert!((left(&start) - 200.0).abs() < 1.0);
    assert!((left(&centre) - (200.0 - centre.width * 0.5)).abs() < 1.0);
    assert!(right(&centre) < 200.0 + centre.width * 0.5);
}

#[test]
fn place_spans_with_a_wrap_width_still_aligns_inside_the_box() {
    let mut engine = TextEngine::new(SANS).unwrap();
    let face = face("DM Sans", 24.0, 400);
    let at = placement(1.0, Representation::Msdf, [50.0, 0.0]);
    let spans = [span("ab", &face)];
    let start = engine
        .place_spans(&spans, Some(300.0), Anchor::Start, &at)
        .unwrap();
    let end = engine
        .place_spans(&spans, Some(300.0), Anchor::End, &at)
        .unwrap();
    let moved = end.quads[0].quad.pen[0] - start.quads[0].quad.pen[0];
    assert!((moved - (300.0 - start.width)).abs() < 1.0, "{moved}");
}
