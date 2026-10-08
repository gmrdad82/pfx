use std::collections::BTreeSet;

use crate::subset::{GB2312_LEVEL1, SubsetOptions, gb2312_level1, subset};
use crate::{
    Anchor, Error, Face, Fallback, Pick, Placement, Representation, Span, TextEngine, matched_scale,
};

const SANS: &[u8] = include_bytes!("../tests/fonts/DMSans[opsz,wght].ttf");
const CJK: &[u8] = include_bytes!("../tests/fonts/NotoSansSC-subset.ttf");
const SERIF: &[u8] = include_bytes!("../fonts/EBGaramond[wght].ttf");
const MONO: &[u8] = include_bytes!("../tests/fonts/IBMPlexMono-Regular.ttf");

fn face(family: &str, size: f32, weight: u16) -> Face {
    Face {
        family: family.into(),
        size,
        line: size * 1.4,
        weight,
        italic: false,
        spacing: 0.0,
    }
}

fn span(text: &str, face: &Face) -> Span {
    Span {
        text: text.into(),
        face: face.clone(),
        color: [1.0; 4],
    }
}

fn engine() -> TextEngine {
    let mut engine = TextEngine::new(SANS).unwrap();
    engine.register_font(CJK).unwrap();
    engine
        .set_fallbacks("DM Sans", None, &[Fallback::new("Noto Sans SC")])
        .unwrap();
    engine
}

fn pick(range: std::ops::Range<usize>, family: &str, weight: u16, scale: f32) -> Pick {
    Pick {
        range,
        family: family.into(),
        weight,
        scale,
    }
}

fn placement(scale: f32, representation: Representation) -> Placement {
    Placement {
        scale,
        representation,
        origin: [0.0; 2],
        alpha: 1.0,
        clip: [-f32::MAX, -f32::MAX, f32::MAX, f32::MAX],
        turn: [0.0; 3],
    }
}

fn full_cjk() -> Option<Vec<u8>> {
    let path = std::env::var_os("PITO_CJK_FONT")?;
    std::fs::read(path).ok()
}

#[test]
fn glyphs_the_primary_lacks_come_from_the_fallback_at_the_requested_weight() {
    let mut engine = engine();
    let text = "Hello 0：设置语言你好。Hello";
    for weight in [400, 600, 800] {
        assert_eq!(
            engine.picks(&span(text, &face("DM Sans", 40.0, weight))),
            [
                pick(0..7, "DM Sans", weight, 1.0),
                pick(7..31, "Noto Sans SC", weight, 0.955),
                pick(31..36, "DM Sans", weight, 1.0),
            ]
        );
    }
    let latin = "Setări, țară și Straße — déjà 0123";
    assert_eq!(
        engine.picks(&span(latin, &face("DM Sans", 40.0, 400))),
        [pick(0..latin.len(), "DM Sans", 400, 1.0)]
    );
}

#[test]
fn the_fallback_list_is_tried_in_order() {
    let mut engine = engine();
    engine.register_font(SERIF).unwrap();
    let text = "《设置语》";
    let order = [Fallback::new("EB Garamond"), Fallback::new("Noto Sans SC")];
    engine.set_fallbacks("DM Sans", None, &order).unwrap();
    let garamond = 0.7 / engine.face_metrics("EB Garamond", 400).unwrap().cap_height;
    let garamond = (garamond * 10_000.0).round() / 10_000.0;
    assert_eq!(
        engine.picks(&span(text, &face("DM Sans", 40.0, 400))),
        [
            pick(0..3, "EB Garamond", 400, garamond),
            pick(3..12, "Noto Sans SC", 400, 0.955),
            pick(12..15, "EB Garamond", 400, garamond),
        ]
    );
    let order = [Fallback::new("Noto Sans SC"), Fallback::new("EB Garamond")];
    engine.set_fallbacks("DM Sans", None, &order).unwrap();
    assert_eq!(
        engine.picks(&span(text, &face("DM Sans", 40.0, 400))),
        [pick(0..15, "Noto Sans SC", 400, 0.955)]
    );
}

#[test]
fn a_list_for_one_weight_overrides_the_default_list() {
    let mut engine = engine();
    let heavier = Fallback {
        weight: Some(900),
        ..Fallback::new("Noto Sans SC")
    };
    engine
        .set_fallbacks("DM Sans", Some(800), &[heavier])
        .unwrap();
    assert_eq!(
        engine.picks(&span("中文", &face("DM Sans", 40.0, 800)))[0],
        pick(0..6, "Noto Sans SC", 900, 0.955)
    );
    assert_eq!(
        engine.picks(&span("中文", &face("DM Sans", 40.0, 600)))[0],
        pick(0..6, "Noto Sans SC", 600, 0.955)
    );
    let fixed = Fallback {
        scale: Some(1.0),
        ..Fallback::new("Noto Sans SC")
    };
    engine
        .set_fallbacks("DM Sans", Some(600), &[fixed])
        .unwrap();
    assert_eq!(
        engine.picks(&span("中文", &face("DM Sans", 40.0, 600)))[0],
        pick(0..6, "Noto Sans SC", 600, 1.0)
    );
    engine.set_fallbacks("DM Sans", Some(600), &[]).unwrap();
    assert_eq!(
        engine.picks(&span("中文", &face("DM Sans", 40.0, 600)))[0],
        pick(0..6, "Noto Sans SC", 600, 0.955)
    );
    engine.set_fallbacks("DM Sans", None, &[]).unwrap();
    engine.set_fallbacks("DM Sans", Some(800), &[]).unwrap();
    assert_eq!(
        engine.picks(&span("中文", &face("DM Sans", 40.0, 800))),
        [pick(0..6, "DM Sans", 800, 1.0)]
    );
}

#[test]
fn a_static_fallback_takes_its_nearest_weight() {
    let mut engine = engine();
    engine.register_font(MONO).unwrap();
    let order = [
        Fallback::new("IBM Plex Mono"),
        Fallback::new("Noto Sans SC"),
    ];
    engine.set_fallbacks("DM Sans", None, &order).unwrap();
    let picks = engine.picks(&span("┌─┐中", &face("DM Sans", 40.0, 800)));
    assert_eq!(picks[0].family, "IBM Plex Mono");
    assert_eq!(picks[0].weight, 400);
    assert_eq!(picks[0].range, 0..9);
    assert_eq!(picks[1], pick(9..12, "Noto Sans SC", 800, 0.955));
}

#[test]
fn marks_and_selectors_stay_with_their_base() {
    let mut engine = engine();
    let text = "中\u{fe0f}文\u{200d}A\u{0301}";
    assert_eq!(
        engine.picks(&span(text, &face("DM Sans", 40.0, 400))),
        [
            pick(0..12, "Noto Sans SC", 400, 0.955),
            pick(12..15, "DM Sans", 400, 1.0),
        ]
    );
}

#[test]
fn unknown_families_and_bad_values_are_refused() {
    let mut engine = engine();
    assert_eq!(
        engine.set_fallbacks("Missing Sans", None, &[Fallback::new("Noto Sans SC")]),
        Err(Error::InvalidFont)
    );
    assert_eq!(
        engine.set_fallbacks("DM Sans", None, &[Fallback::new("Nope")]),
        Err(Error::InvalidFont)
    );
    let zero = Fallback {
        scale: Some(0.0),
        ..Fallback::new("Noto Sans SC")
    };
    assert_eq!(
        engine.set_fallbacks("DM Sans", None, &[zero]),
        Err(Error::InvalidStyle)
    );
    assert_eq!(
        engine.set_fallbacks("DM Sans", Some(0), &[Fallback::new("Noto Sans SC")]),
        Err(Error::InvalidStyle)
    );
}

#[test]
fn layout_does_not_depend_on_the_order_fonts_were_registered() {
    let mut first = engine();
    let mut second = TextEngine::new(CJK).unwrap();
    second.register_font(SANS).unwrap();
    second
        .set_fallbacks("DM Sans", None, &[Fallback::new("Noto Sans SC")])
        .unwrap();
    let spans = [span(
        "Hello 0 设置语言你好。你好，世界！Hello 中文",
        &face("DM Sans", 24.0, 600),
    )];
    for representation in [Representation::Coverage, Representation::Msdf] {
        let a = first
            .place_spans(
                &spans,
                Some(200.0),
                Anchor::Start,
                &placement(2.0, representation),
            )
            .unwrap();
        let b = second
            .place_spans(
                &spans,
                Some(200.0),
                Anchor::Start,
                &placement(2.0, representation),
            )
            .unwrap();
        assert_eq!(a.quads.len(), b.quads.len());
        for (a, b) in a.quads.iter().zip(&b.quads) {
            assert_eq!(a.quad.rect, b.quad.rect);
            assert_eq!(a.quad.uv, b.quad.uv);
            assert_eq!(a.quad.pen, b.quad.pen);
            assert_eq!(a.quad.line_baseline, b.quad.line_baseline);
        }
        assert_eq!(
            (a.width, a.height, a.baseline),
            (b.width, b.height, b.baseline)
        );
    }
}

#[test]
fn the_scale_matches_cap_heights() {
    let mut engine = engine();
    let sans = engine.face_metrics("DM Sans", 400).unwrap();
    let cjk = engine.face_metrics("Noto Sans SC", 400).unwrap();
    assert_eq!(sans.cap_height, 0.7);
    assert_eq!(cjk.cap_height, 0.733);
    assert_eq!(matched_scale(sans, cjk), 0.955);
    let block = engine
        .layout_spans(
            &[span("H中", &face("DM Sans", 40.0, 400))],
            None,
            2.0,
            Anchor::Start,
        )
        .unwrap();
    let sizes = block
        .buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.font_size))
        .collect::<Vec<_>>();
    assert_eq!(sizes, [80.0, 80.0 * 0.955]);
}

#[test]
fn cjk_lines_keep_the_latin_baseline_and_line_height() {
    let mut engine = engine();
    let mut lines = Vec::new();
    for text in ["Hxgy Hxgy", "Hx中文", "中文中文", "，。、"] {
        for weight in [400, 800] {
            let block = engine
                .layout_spans(
                    &[span(text, &face("DM Sans", 40.0, weight))],
                    None,
                    1.0,
                    Anchor::Start,
                )
                .unwrap();
            lines.push((block.line_baselines(), block.height, block.baseline));
        }
    }
    assert!(lines.windows(2).all(|pair| pair[0] == pair[1]), "{lines:?}");
    let block = engine
        .layout_spans(
            &[span(
                "设置语言你好 Hello 0 中文中文中文 Hello world 设置语言",
                &face("DM Sans", 40.0, 400),
            )],
            Some(180.0),
            1.0,
            Anchor::Start,
        )
        .unwrap();
    let baselines = block.line_baselines();
    assert!(baselines.len() >= 4);
    for pair in baselines.windows(2) {
        assert!((pair[1] - pair[0] - 56.0).abs() < 1e-3, "{baselines:?}");
    }
    assert_eq!(block.height, 56.0 * baselines.len() as f32);
}

#[test]
fn cjk_glyphs_sit_on_the_line_baseline() {
    let mut engine = engine();
    let spans = [span("Hx设置语gy中文。", &face("DM Sans", 32.0, 600))];
    for representation in [Representation::Coverage, Representation::Msdf] {
        let placed = engine
            .place_spans(&spans, None, Anchor::Start, &placement(2.0, representation))
            .unwrap();
        let pens = placed
            .quads
            .iter()
            .map(|q| q.quad.pen[1])
            .collect::<Vec<_>>();
        assert!(pens.iter().all(|&y| y == pens[0]), "{pens:?}");
        assert!(
            placed
                .quads
                .iter()
                .all(|q| q.quad.line_baseline == placed.baseline)
        );
        assert_eq!(pens[0], placed.baseline);
    }
}

#[test]
fn ideographs_draw_at_the_matched_size() {
    let mut fallback = engine();
    let mut direct = TextEngine::new(CJK).unwrap();
    let size = 60.0;
    let height = |placed: &crate::Placed| placed.quads[0].quad.rect[3];
    let mixed = fallback
        .place_spans(
            &[span("中", &face("DM Sans", size, 400))],
            None,
            Anchor::Start,
            &placement(1.0, Representation::Coverage),
        )
        .unwrap();
    let scaled = direct
        .place_spans(
            &[span("中", &face("Noto Sans SC", size * 0.955, 400))],
            None,
            Anchor::Start,
            &placement(1.0, Representation::Coverage),
        )
        .unwrap();
    let unscaled = direct
        .place_spans(
            &[span("中", &face("Noto Sans SC", size, 400))],
            None,
            Anchor::Start,
            &placement(1.0, Representation::Coverage),
        )
        .unwrap();
    assert_eq!(
        mixed.quads[0].quad.rect[2..],
        scaled.quads[0].quad.rect[2..]
    );
    assert!(height(&unscaled) > height(&mixed));
    let cap = fallback
        .place_spans(
            &[span("H", &face("DM Sans", size, 400))],
            None,
            Anchor::Start,
            &placement(1.0, Representation::Coverage),
        )
        .unwrap();
    let ratio = height(&mixed) / height(&cap);
    assert!((1.2..1.3).contains(&ratio), "中 over H: {ratio}");
}

fn line_texts(engine: &mut TextEngine, text: &str, width: f32) -> Vec<String> {
    let block = engine
        .layout_spans(
            &[span(text, &face("DM Sans", 40.0, 400))],
            Some(width),
            1.0,
            Anchor::Start,
        )
        .unwrap();
    block
        .buffer
        .layout_runs()
        .filter_map(|run| {
            let start = run.glyphs.iter().map(|glyph| glyph.start).min()?;
            let end = run.glyphs.iter().map(|glyph| glyph.end).max()?;
            Some(text[start..end].to_owned())
        })
        .collect()
}

#[test]
fn ideographs_break_between_each_other_without_spaces() {
    let mut engine = engine();
    let em = 40.0 * 0.955;
    assert_eq!(
        line_texts(&mut engine, "设置语言你好设置语言你好", em * 5.0 + 0.5),
        ["设置语言你", "好设置语言", "你好"]
    );
    assert_eq!(
        line_texts(&mut engine, "设置语言你好。设置语", em * 6.0 + 0.5),
        ["设置语言你", "好。设置语"]
    );
    assert_eq!(
        line_texts(&mut engine, "你好「中文」世界", em * 4.0 + 0.5),
        ["你好「中", "文」世界"]
    );
}

#[test]
fn latin_words_stay_whole_beside_ideographs() {
    let mut engine = engine();
    let text = "Hello设置语Hello设置语Hello";
    for width in [120.0, 160.0, 200.0, 260.0] {
        let lines = line_texts(&mut engine, text, width);
        assert_eq!(lines.concat(), text);
        for line in &lines {
            assert!(
                !line.starts_with("nbox") && !line.ends_with("Inbo"),
                "{lines:?}"
            );
        }
        assert!(lines.len() > 1);
    }
}

const NO_START: &str = "，。、；：？！）」』》】〉…ーぁぃぅぇぉっゃゅょァィゥェォッャュョ々";
const NO_END: &str = "（「『《【〈";

#[test]
fn kinsoku_keeps_closing_marks_off_line_starts_and_opening_marks_off_line_ends() {
    let mut engine = engine();
    let text = "我在，你好。他（是）「中文」！对？大、小……老师说：《书》【二】中ー好々。好っ。ァ中";
    let em = 40.0 * 0.955;
    for ems in 4..14 {
        let lines = line_texts(&mut engine, text, em * ems as f32 + 0.5);
        assert_eq!(lines.concat(), text);
        for line in &lines {
            let first = line.chars().next().unwrap();
            let last = line.chars().last().unwrap();
            assert!(!NO_START.contains(first), "{ems} ems: {lines:?}");
            assert!(!NO_END.contains(last), "{ems} ems: {lines:?}");
        }
    }
}

fn opportunities(text: &str) -> Vec<usize> {
    unicode_linebreak::linebreaks(text)
        .map(|(at, _)| at)
        .filter(|&at| at < text.len())
        .collect()
}

#[test]
fn breaks_follow_uax14() {
    let cases: [(&str, &[&str]); 12] = [
        ("中文字", &["中", "文", "字"]),
        ("文。中", &["文。", "中"]),
        ("（中）文", &["（中）", "文"]),
        ("中ー中", &["中ー", "中"]),
        ("中ぁ中", &["中ぁ", "中"]),
        ("中々中", &["中々", "中"]),
        ("中、文！", &["中、", "文！"]),
        ("「中」", &["「中」"]),
        ("Hello 0是", &["Hello ", "0", "是"]),
        ("10％是", &["10％", "是"]),
        ("中文，Hello", &["中", "文，", "Hello"]),
        ("《书》说", &["《书》", "说"]),
    ];
    for (text, pieces) in cases {
        let mut at = 0;
        let expected = pieces[..pieces.len() - 1]
            .iter()
            .map(|piece| {
                at += piece.len();
                at
            })
            .collect::<Vec<_>>();
        assert_eq!(opportunities(text), expected, "{text}");
    }
    let mut engine = engine();
    let text = "设置语言你好。你好（世界）「中文」好々 中ーぁ Hello 0是10％！《书》说，Hello世界";
    let allowed = opportunities(text);
    for width in (130..400).step_by(17) {
        let block = engine
            .layout_spans(
                &[span(text, &face("DM Sans", 40.0, 400))],
                Some(width as f32),
                1.0,
                Anchor::Start,
            )
            .unwrap();
        let starts = block
            .buffer
            .layout_runs()
            .filter_map(|run| run.glyphs.iter().map(|glyph| glyph.start).min())
            .collect::<Vec<_>>();
        assert!(starts.len() > 1);
        for start in &starts[1..] {
            assert!(allowed.contains(start), "{width}: a line starts at {start}");
        }
    }
}

#[test]
fn the_subset_keeps_exactly_the_requested_characters() {
    let keep = "中文H，"
        .chars()
        .chain(['z', '\u{1f600}'])
        .collect::<BTreeSet<_>>();
    let bytes = subset(
        CJK,
        &keep,
        SubsetOptions {
            layout_closure: false,
        },
    )
    .unwrap();
    let font = swash::FontRef::from_index(&bytes, 0).unwrap();
    let mut mapped = Vec::new();
    font.charmap()
        .enumerate(|c, _| mapped.push(char::from_u32(c).unwrap()));
    mapped.sort();
    let source = swash::FontRef::from_index(CJK, 0).unwrap();
    let mut expected = keep
        .iter()
        .copied()
        .filter(|&c| source.charmap().map(c) != 0)
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(mapped, expected);
    assert_eq!(expected.len(), 4);
    assert_eq!(font.glyph_metrics(&[]).glyph_count(), 5);
    let closed = subset(CJK, &keep, SubsetOptions::default()).unwrap();
    let closed = swash::FontRef::from_index(&closed, 0).unwrap();
    let mut closed_mapped = Vec::new();
    closed
        .charmap()
        .enumerate(|c, _| closed_mapped.push(char::from_u32(c).unwrap()));
    closed_mapped.sort();
    assert_eq!(closed_mapped, expected);
    assert!(closed.glyph_metrics(&[]).glyph_count() >= 5);
    assert!(bytes.len() < CJK.len() / 10);
}

fn outline_points(font: &[u8], c: char, weight: u16) -> Vec<[f32; 2]> {
    let font = swash::FontRef::from_index(font, 0).unwrap();
    let mut context = swash::scale::ScaleContext::new();
    let mut scaler = context
        .builder(font)
        .hint(false)
        .variations([swash::Setting {
            tag: swash::Tag::from_be_bytes(*b"wght"),
            value: f32::from(weight),
        }])
        .build();
    let outline = scaler.scale_outline(font.charmap().map(c)).unwrap();
    outline.points().iter().map(|p| [p.x, p.y]).collect()
}

#[test]
fn the_subset_keeps_every_outline_at_every_weight() {
    let text = "中文设置语";
    let keep = text.chars().collect::<BTreeSet<_>>();
    let bytes = subset(CJK, &keep, SubsetOptions::default()).unwrap();
    let mut worst = 0.0f32;
    for weight in [100, 250, 400, 600, 800, 900] {
        for c in text.chars() {
            let whole = outline_points(CJK, c, weight);
            let cut = outline_points(&bytes, c, weight);
            assert_eq!(whole.len(), cut.len());
            for (a, b) in whole.iter().zip(&cut) {
                worst = worst.max((a[0] - b[0]).abs()).max((a[1] - b[1]).abs());
            }
            if weight == 100 {
                assert_eq!(whole, cut);
            }
        }
    }
    assert!(worst <= 1.0, "{worst} font units");
}

#[test]
fn gb2312_level_one_holds_its_3755_ideographs() {
    let chars = gb2312_level1();
    assert_eq!(chars.len(), GB2312_LEVEL1);
    assert_eq!(chars.iter().collect::<BTreeSet<_>>().len(), GB2312_LEVEL1);
    assert_eq!(chars.first(), Some(&'啊'));
    assert_eq!(chars.last(), Some(&'座'));
    assert!(chars.iter().all(|c| ('\u{4e00}'..='\u{9fff}').contains(c)));
    assert!(gb2312_level1().contains(&'的'));
}

#[test]
fn a_font_without_truetype_outlines_is_refused() {
    let keep = BTreeSet::from(['a']);
    assert_eq!(
        subset(b"not a font", &keep, SubsetOptions::default()),
        Err(Error::InvalidFont)
    );
}

struct Ideographs {
    chars: Vec<char>,
    weights: Vec<u16>,
}

impl Ideographs {
    fn len(&self) -> usize {
        self.chars.len() * self.weights.len()
    }

    fn get(&self, index: usize) -> (char, u16) {
        (
            self.chars[index % self.chars.len()],
            self.weights[index / self.chars.len()],
        )
    }
}

fn subset_ideographs(count: usize) -> Ideographs {
    let font = swash::FontRef::from_index(CJK, 0).unwrap();
    let mut chars = Vec::new();
    font.charmap().enumerate(|c, _| {
        if let Some(c) = char::from_u32(c).filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c)) {
            chars.push(c);
        }
    });
    let rounds = count.div_ceil(chars.len());
    let weights = (0..rounds)
        .map(|i| 100 + (800 * i / rounds.max(2)) as u16)
        .collect();
    Ideographs { chars, weights }
}

fn font_ideographs(font: &[u8], count: usize) -> Ideographs {
    let font = swash::FontRef::from_index(font, 0).unwrap();
    let mut chars = gb2312_level1();
    let known = chars.iter().copied().collect::<BTreeSet<_>>();
    chars.extend(
        ('\u{4e00}'..='\u{9fff}').filter(|c| !known.contains(c) && font.charmap().map(*c) != 0),
    );
    chars.retain(|&c| font.charmap().map(c) != 0);
    chars.truncate(count);
    Ideographs {
        chars,
        weights: vec![400],
    }
}

fn cjk_engine(fallback: &[u8]) -> TextEngine {
    let mut engine = TextEngine::new(SANS).unwrap();
    engine.register_font(fallback).unwrap();
    let family = engine
        .families()
        .into_iter()
        .find(|family| family != "DM Sans")
        .unwrap();
    engine
        .set_fallbacks("DM Sans", None, &[Fallback::new(&family)])
        .unwrap();
    engine
}

fn place_line(
    engine: &mut TextEngine,
    set: &Ideographs,
    range: std::ops::Range<usize>,
    size: f32,
    placement: &Placement,
) -> usize {
    let mut by_weight = std::collections::BTreeMap::<u16, String>::new();
    for index in range {
        let (c, weight) = set.get(index);
        by_weight.entry(weight).or_default().push(c);
    }
    let spans = by_weight
        .into_iter()
        .map(|(weight, text)| span(&text, &face("DM Sans", size, weight)))
        .collect::<Vec<_>>();
    engine
        .place_spans(&spans, None, Anchor::Start, placement)
        .unwrap()
        .quads
        .len()
}

fn cjk_screen(
    fallback: &[u8],
    set: &Ideographs,
    representation: Representation,
    budget: u64,
) -> crate::AtlasStats {
    let mut engine = cjk_engine(fallback);
    engine.set_atlas_budget(budget);
    let sizes = [19.0, 24.0, 32.0, 48.0, 75.0];
    let shares = [800, 600, 350, 200, 50];
    let placement = placement(2.0, representation);
    let mut frames = Vec::new();
    for _ in 0..3 {
        engine.begin_frame();
        let mut at = 0;
        let mut drawn = 0;
        for (size, share) in sizes.into_iter().zip(shares) {
            for start in (at..at + share).step_by(40) {
                drawn += place_line(
                    &mut engine,
                    set,
                    start..(start + 40).min(at + share),
                    size,
                    &placement,
                );
            }
            at += share;
        }
        engine.end_frame();
        engine.take_atlas_changes();
        assert_eq!(drawn, 2000);
        frames.push(engine.atlas_stats());
    }
    let stats = frames[2];
    assert!(stats.bytes <= stats.budget, "{stats:?}");
    assert_eq!(stats.evicted, 0, "{stats:?}");
    assert_eq!(frames[1].rasterized, frames[0].rasterized);
    assert_eq!(frames[2].uploaded, frames[0].uploaded);
    stats
}

#[test]
fn two_thousand_distinct_ideographs_fit_the_default_budget() {
    let set = subset_ideographs(2000);
    let coverage = cjk_screen(
        CJK,
        &set,
        Representation::Coverage,
        crate::DEFAULT_ATLAS_BUDGET,
    );
    assert_eq!(coverage.cells, 2000);
    assert!(coverage.bytes <= 24 << 20, "{coverage:?}");
}

#[test]
fn two_thousand_ideographs_from_the_full_face_fit_the_default_budget() {
    let Some(font) = full_cjk() else {
        eprintln!("PITO_CJK_FONT is not set; skipped");
        return;
    };
    let set = font_ideographs(&font, 2000);
    assert_eq!(set.len(), 2000);
    for (representation, budget) in [
        (Representation::Coverage, crate::DEFAULT_ATLAS_BUDGET),
        (Representation::Msdf, 96 << 20),
    ] {
        let stats = cjk_screen(&font, &set, representation, budget);
        eprintln!(
            "{representation:?}: {} cells on {} pages, {:.1} MiB of {:.1} MiB",
            stats.cells,
            stats.pages,
            stats.bytes as f64 / (1 << 20) as f64,
            stats.budget as f64 / (1 << 20) as f64
        );
    }
}

struct Scroll {
    frames: usize,
    most_rasters: u64,
    most_uploads: usize,
    most_bytes: u64,
    rasterized: u64,
    evicted: u64,
}

fn scroll(fallback: &[u8], set: &Ideographs, budget: u64) -> Scroll {
    let mut engine = cjk_engine(fallback);
    engine.set_atlas_budget(budget);
    let placement = placement(2.0, Representation::Coverage);
    let (columns, rows) = (20, 20);
    let mut result = Scroll {
        frames: 0,
        most_rasters: 0,
        most_uploads: 0,
        most_bytes: 0,
        rasterized: 0,
        evicted: 0,
    };
    let lines = set.len() / columns;
    let mut before = engine.atlas_stats();
    for top in 0..=lines - rows {
        engine.begin_frame();
        for row in top..top + rows {
            place_line(
                &mut engine,
                set,
                row * columns..(row + 1) * columns,
                24.0,
                &placement,
            );
        }
        engine.end_frame();
        let changes = engine.take_atlas_changes();
        let stats = engine.atlas_stats();
        assert!(stats.bytes <= budget);
        if top > 0 {
            result.most_rasters = result
                .most_rasters
                .max(stats.rasterized - before.rasterized);
            result.most_uploads = result
                .most_uploads
                .max(changes.cells.iter().filter(|cell| cell.level == 0).count());
            result.most_bytes = result.most_bytes.max(changes.bytes());
        }
        before = stats;
        result.frames += 1;
    }
    result.rasterized = before.rasterized;
    result.evicted = before.evicted;
    result
}

fn assert_no_thrash(set: &Ideographs, result: &Scroll) {
    assert_eq!(result.most_rasters, 20);
    assert_eq!(result.most_uploads, 20);
    assert_eq!(result.rasterized, (set.len() / 20 * 20) as u64);
    assert!(result.evicted > 0);
}

#[test]
fn scrolling_through_five_thousand_ideographs_uploads_only_the_new_line() {
    let set = subset_ideographs(5000);
    let set = Ideographs {
        weights: set.weights[..5000usize.div_ceil(set.chars.len())].to_vec(),
        ..set
    };
    let result = scroll(CJK, &set, 4 << 20);
    assert_no_thrash(&set, &result);
}

#[test]
fn scrolling_through_five_thousand_ideographs_of_the_full_face() {
    let Some(font) = full_cjk() else {
        eprintln!("PITO_CJK_FONT is not set; skipped");
        return;
    };
    let set = font_ideographs(&font, 5000);
    assert_eq!(set.len(), 5000);
    for budget in [4u64 << 20, 32 << 20] {
        let result = scroll(&font, &set, budget);
        eprintln!(
            "budget {} MiB, {} frames: at most {} rasters, {} cell uploads and {} bytes a frame; {} rasterized, {} evicted",
            budget >> 20,
            result.frames,
            result.most_rasters,
            result.most_uploads,
            result.most_bytes,
            result.rasterized,
            result.evicted
        );
        if budget == 4 << 20 {
            assert_no_thrash(&set, &result);
        }
    }
}

#[test]
fn the_number_fast_path_stays_with_the_primary_face() {
    let mut plain = TextEngine::new(SANS).unwrap();
    let mut mixed = engine();
    let label = span("12,345.6", &face("DM Sans", 24.0, 600));
    let placement = placement(2.0, Representation::Coverage);
    mixed
        .place_spans(
            &[span("设置语 12 封", &face("DM Sans", 24.0, 600))],
            None,
            Anchor::Start,
            &placement,
        )
        .unwrap();
    let a = plain
        .place_number(&label, "0123456789,.", Anchor::End, &placement)
        .unwrap();
    let fast = mixed.atlas_stats().fast;
    let shaped = mixed.atlas_stats().shaped;
    let b = mixed
        .place_number(&label, "0123456789,.", Anchor::End, &placement)
        .unwrap();
    assert_eq!(mixed.atlas_stats().fast, fast + 1);
    assert_eq!(mixed.atlas_stats().shaped, shaped + 1);
    let b2 = mixed
        .place_number(&label, "0123456789,.", Anchor::End, &placement)
        .unwrap();
    assert_eq!(mixed.atlas_stats().shaped, shaped + 1);
    for (a, b) in a.quads.iter().zip(b.quads.iter().chain(&b2.quads)) {
        assert_eq!(a.quad.rect, b.quad.rect);
        assert_eq!(a.quad.pen, b.quad.pen);
    }
    assert_eq!(
        mixed.picks(&label),
        [pick(0..label.text.len(), "DM Sans", 600, 1.0)]
    );
}

#[test]
fn the_primary_face_sets_the_cjk_scale_from_its_cap_height() {
    let mut engine = engine();
    let mut lines = Vec::new();
    for weight in [400, 600, 800] {
        let metrics = engine.face_metrics("DM Sans", weight).unwrap();
        assert_eq!((metrics.cap_height, metrics.x_height), (0.7, 0.526));
        for text in [
            "Setări și țară",
            "Straße, déjà, ¿Qué?",
            "Hello 0：设置语言你好。",
            "设置语",
        ] {
            let picks = engine.picks(&span(text, &face("DM Sans", 40.0, weight)));
            let cjk = picks
                .iter()
                .filter(|p| p.family == "Noto Sans SC")
                .collect::<Vec<_>>();
            assert_eq!(cjk.is_empty(), !text.contains('设'), "{picks:?}");
            assert!(cjk.iter().all(|p| p.scale == 0.955 && p.weight == weight));
            let block = engine
                .layout_spans(
                    &[span(text, &face("DM Sans", 40.0, weight))],
                    None,
                    1.0,
                    Anchor::Start,
                )
                .unwrap();
            lines.push((block.line_baselines(), block.height));
        }
    }
    assert!(lines.windows(2).all(|pair| pair[0] == pair[1]), "{lines:?}");

    let mut serif = TextEngine::new(SERIF).unwrap();
    serif.register_font(CJK).unwrap();
    serif
        .set_fallbacks("EB Garamond", None, &[Fallback::new("Noto Sans SC")])
        .unwrap();
    for weight in [400, 600, 800] {
        let cap = serif
            .face_metrics("EB Garamond", weight)
            .unwrap()
            .cap_height;
        let noto = serif
            .face_metrics("Noto Sans SC", weight)
            .unwrap()
            .cap_height;
        let expected = ((cap / noto) * 10_000.0).round() / 10_000.0;
        let picks = serif.picks(&span("设置语", &face("EB Garamond", 40.0, weight)));
        assert_eq!(picks.len(), 1, "{picks:?}");
        assert!(
            (picks[0].scale - expected).abs() < 1e-3,
            "{picks:?} {expected}"
        );
        assert!((picks[0].scale - 0.955).abs() > 1e-3, "{picks:?}");
    }
}

#[test]
fn the_subset_command_writes_the_font_and_explains_its_arguments() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/text-subset-command");
    std::fs::create_dir_all(&dir).unwrap();
    let font = dir.join("in.ttf");
    let chars = dir.join("chars.txt");
    let out = dir.join("out.ttf");
    std::fs::write(&font, CJK).unwrap();
    std::fs::write(&chars, "中文\n设置语\n").unwrap();
    let args = |list: &[&std::path::Path]| {
        [
            "--font",
            list[0].to_str().unwrap(),
            "--chars",
            list[1].to_str().unwrap(),
            "--out",
            list[2].to_str().unwrap(),
        ]
        .map(String::from)
        .to_vec()
    };
    let line = crate::subset::command(&args(&[&font, &chars, &out])).unwrap();
    assert!(line.starts_with("5 characters requested"), "{line}");
    let written = std::fs::read(&out).unwrap();
    let face = swash::FontRef::from_index(&written, 0).unwrap();
    assert!("中文设置语".chars().all(|c| face.charmap().map(c) != 0));
    assert!(face.charmap().map('的') == 0);
    let mut gb = args(&[&font, &chars, &out]);
    gb.extend(["--base", "gb2312-1"].map(String::from));
    let line = crate::subset::command(&gb).unwrap();
    assert!(line.starts_with("3755 characters requested"), "{line}");
}
