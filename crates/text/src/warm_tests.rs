use crate::{
    Anchor, Face, Figures, Ligatures, NumberForms, Placed, Placement, Prefill, Representation,
    Span, TextEngine,
};

const SANS: &[u8] = include_bytes!("../tests/fonts/DMSans[opsz,wght].ttf");
const SERIF: &[u8] = include_bytes!("../fonts/EBGaramond[wght].ttf");
const SET: &str = "0123456789fit(),.-";

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

fn forms(figures: Figures, ligatures: Ligatures) -> NumberForms {
    NumberForms { figures, ligatures }
}

fn same(a: &Placed, b: &Placed, label: &str) {
    assert_eq!(a.quads.len(), b.quads.len(), "{label}");
    assert!(
        (a.width - b.width).abs() < 1e-3,
        "{label} {} {}",
        a.width,
        b.width
    );
    assert!((a.baseline - b.baseline).abs() < 1e-4, "{label}");
    assert!((a.height - b.height).abs() < 1e-4, "{label}");
    for (a, b) in a.quads.iter().zip(&b.quads) {
        assert_eq!(a.page, b.page, "{label}");
        assert_eq!(a.quad.key, b.quad.key, "{label}");
        assert_eq!(a.quad.uv, b.quad.uv, "{label}");
        for k in 0..2 {
            assert!((a.quad.pen[k] - b.quad.pen[k]).abs() < 1e-3, "{label}");
        }
        for k in 0..4 {
            assert!((a.quad.rect[k] - b.quad.rect[k]).abs() < 1e-3, "{label}");
        }
        assert_eq!(a.quad.color, b.quad.color, "{label}");
    }
}

fn engines() -> [(TextEngine, &'static str); 2] {
    [
        (TextEngine::new(SERIF).unwrap(), "EB Garamond"),
        (TextEngine::new(SANS).unwrap(), "DM Sans"),
    ]
}

fn ligates(text: &str, pairs: &[(char, char)]) -> bool {
    text.chars()
        .zip(text.chars().skip(1))
        .any(|pair| pairs.contains(&pair))
}

#[test]
fn only_labels_with_a_ligating_pair_leave_the_fast_path() {
    let labels = [
        "tif", "1,284", "it.5", "(12)", "fit", "5fi", "ff", "-0.5", "if", "ti",
    ];
    for (mut engine, family) in engines() {
        let face = face(family, 31.0, 400);
        let kept = NumberForms::default();
        let pairs = engine.ligating_pairs(&face, SET, kept, 2.0).unwrap();
        if family == "EB Garamond" {
            assert!(pairs.contains(&('f', 'i')), "{pairs:?}");
            assert!(pairs.contains(&('f', 'f')), "{pairs:?}");
        }
        assert!(!pairs.contains(&('i', 'f')), "{family} {pairs:?}");
        for representation in [Representation::Coverage, Representation::Msdf] {
            let at = placement(2.0, representation, [3.3, 7.1]);
            for text in labels {
                let before = engine.atlas_stats();
                let fast = engine
                    .place_number_with(&span(text, &face), SET, kept, Anchor::Start, &at)
                    .unwrap();
                let after = engine.atlas_stats();
                let ligating = ligates(text, &pairs);
                assert_eq!(after.fast - before.fast, u64::from(!ligating), "{text}");
                let full = engine
                    .place_number_with(&span(text, &face), "", kept, Anchor::Start, &at)
                    .unwrap();
                same(&fast, &full, &format!("{family} {text}"));
                if ligating && family == "EB Garamond" {
                    assert!(fast.quads.len() < text.chars().count(), "{text}");
                }
            }
        }
    }
}

#[test]
fn ligatures_off_keeps_ligating_labels_on_the_fast_path() {
    let mut engine = TextEngine::new(SERIF).unwrap();
    let face = face("EB Garamond", 31.0, 400);
    let off = forms(Figures::Tabular, Ligatures::Off);
    let pairs = engine.ligating_pairs(&face, SET, off, 1.0).unwrap();
    assert!(pairs.is_empty(), "{pairs:?}");
    let at = placement(1.0, Representation::Msdf, [0.0, 0.0]);
    for text in ["fit", "5fi", "ff", "tif"] {
        let before = engine.atlas_stats().fast;
        let fast = engine
            .place_number_with(&span(text, &face), SET, off, Anchor::Start, &at)
            .unwrap();
        assert_eq!(engine.atlas_stats().fast, before + 1, "{text}");
        assert_eq!(fast.quads.len(), text.chars().count(), "{text}");
        let full = engine
            .place_number_with(&span(text, &face), "", off, Anchor::Start, &at)
            .unwrap();
        same(&fast, &full, text);
    }
    let kept = engine
        .place_number(&span("fit", &face), SET, Anchor::Start, &at)
        .unwrap();
    assert_eq!(kept.quads.len(), 2);
}

#[test]
fn proportional_figures_lay_out_as_plain_shaping() {
    let proportional = forms(Figures::Proportional, Ligatures::Kept);
    for (mut engine, family) in engines() {
        for weight in [400, 800] {
            let face = face(family, 27.0, weight);
            for representation in [Representation::Coverage, Representation::Msdf] {
                let at = placement(2.0, representation, [1.7, 4.2]);
                for text in ["(12)", "(1,284.50)", "-7", "1111", "(0)"] {
                    let before = engine.atlas_stats().fast;
                    let fast = engine
                        .place_number_with(
                            &span(text, &face),
                            SET,
                            proportional,
                            Anchor::Start,
                            &at,
                        )
                        .unwrap();
                    assert_eq!(engine.atlas_stats().fast, before + 1, "{text}");
                    let plain = engine
                        .place_spans(&[span(text, &face)], None, Anchor::Start, &at)
                        .unwrap();
                    same(&fast, &plain, &format!("{family} {weight} {text}"));
                }
            }
        }
    }
}

#[test]
fn tabular_digits_keep_punctuation_proportional() {
    let digits = forms(Figures::TabularDigits, Ligatures::Kept);
    let proportional = forms(Figures::Proportional, Ligatures::Kept);
    for (mut engine, family) in engines() {
        let face = face(family, 31.0, 600);
        let at = placement(2.0, Representation::Msdf, [0.0, 0.0]);
        let width = |engine: &mut TextEngine, text: &str, forms| {
            engine
                .place_number_with(&span(text, &face), SET, forms, Anchor::Start, &at)
                .unwrap()
                .width
        };
        let columns = ["(11)", "(88)", "(10)", "(47)"].map(|text| width(&mut engine, text, digits));
        for column in columns {
            assert!((column - columns[0]).abs() < 1e-3, "{family} {columns:?}");
        }
        let brackets = width(&mut engine, "()", digits);
        assert!((brackets - width(&mut engine, "()", proportional)).abs() < 1e-4);
        let tabular = width(&mut engine, "()", NumberForms::default());
        println!("{family}: () is {brackets:.3} proportional and {tabular:.3} with tnum");
        let ones = width(&mut engine, "11", proportional);
        let eights = width(&mut engine, "88", proportional);
        if family == "DM Sans" {
            assert!((ones - eights).abs() > 0.5, "{ones} {eights}");
        }
        for representation in [Representation::Coverage, Representation::Msdf] {
            let at = placement(2.0, representation, [3.3, 7.1]);
            for text in ["(12)", "-1,284.5", "(0)", "10.01"] {
                let fast = engine
                    .place_number_with(&span(text, &face), SET, digits, Anchor::Start, &at)
                    .unwrap();
                let full = engine
                    .place_number_with(&span(text, &face), "", digits, Anchor::Start, &at)
                    .unwrap();
                same(&fast, &full, &format!("{family} {text}"));
            }
        }
    }
}

#[test]
fn tabular_digits_line_up_in_labels_that_fall_back() {
    let mut engine = TextEngine::new(SANS).unwrap();
    let face = face("DM Sans", 24.0, 400);
    let at = placement(1.0, Representation::Msdf, [0.0, 0.0]);
    for forms in [
        NumberForms::default(),
        forms(Figures::TabularDigits, Ligatures::Kept),
    ] {
        let widths = ["Score 1111", "Score 8888", "Score 1048"].map(|text| {
            engine
                .place_number_with(&span(text, &face), SET, forms, Anchor::Start, &at)
                .unwrap()
                .width
        });
        for width in widths {
            assert!((width - widths[0]).abs() < 1e-3, "{forms:?} {widths:?}");
        }
    }
}

fn label_frame(engine: &mut TextEngine, labels: &[String], face: &Face) {
    engine.begin_frame();
    let at = placement(1.0, Representation::Msdf, [0.0, 0.0]);
    for label in labels {
        engine
            .place_spans(&[span(label, face)], None, Anchor::Start, &at)
            .unwrap();
    }
    engine.end_frame();
}

#[test]
fn shaped_text_stays_warm_across_frames_that_skip_it() {
    let mut engine = TextEngine::new(SANS).unwrap();
    let face = face("DM Sans", 24.0, 400);
    let menu = (0..40)
        .map(|i| format!("Menu entry {i}"))
        .collect::<Vec<_>>();
    label_frame(&mut engine, &menu, &face);
    let shaped = engine.atlas_stats().shaped;
    for _ in 0..10 {
        label_frame(&mut engine, &[], &face);
    }
    label_frame(&mut engine, &menu, &face);
    assert_eq!(engine.atlas_stats().shaped, shaped);
    let stats = engine.shaping_stats();
    assert_eq!(stats.blocks, 40);
    assert_eq!(stats.evicted, 0);
    assert!(stats.bytes <= stats.budget);
    println!("40 labels of 13 to 14 characters: {} bytes", stats.bytes);
}

#[test]
fn the_shaping_cache_evicts_the_least_recently_used_first() {
    let mut engine = TextEngine::new(SANS).unwrap();
    let face = face("DM Sans", 24.0, 400);
    let labels = (0..10).map(|i| format!("label {i}")).collect::<Vec<_>>();
    for label in &labels {
        label_frame(&mut engine, std::slice::from_ref(label), &face);
    }
    label_frame(&mut engine, &labels[..1], &face);
    let stats = engine.shaping_stats();
    assert_eq!(stats.blocks, 10);
    let each = stats.bytes / 10;
    assert_eq!(stats.bytes, each * 10);
    engine.set_shaping_budget(each * 4 + each / 2);
    label_frame(&mut engine, &[], &face);
    let stats = engine.shaping_stats();
    assert_eq!(stats.blocks, 4);
    assert_eq!(stats.evicted, 6);
    let shaped = engine.atlas_stats().shaped;
    let kept = [&labels[7], &labels[8], &labels[9], &labels[0]].map(String::clone);
    label_frame(&mut engine, &kept, &face);
    assert_eq!(engine.atlas_stats().shaped, shaped);
    label_frame(&mut engine, &labels[1..2], &face);
    assert_eq!(engine.atlas_stats().shaped, shaped + 1);
    assert_eq!(engine.shaping_stats().evicted, 7);
}

#[test]
fn text_in_use_this_frame_stays_whatever_the_budget() {
    let mut engine = TextEngine::new(SANS).unwrap();
    engine.set_shaping_budget(0);
    let face = face("DM Sans", 24.0, 400);
    let at = placement(1.0, Representation::Msdf, [0.0, 0.0]);
    engine.begin_frame();
    for text in ["one", "two", "three"] {
        engine
            .place_spans(&[span(text, &face)], None, Anchor::Start, &at)
            .unwrap();
    }
    engine
        .place_number(&span("1,284", &face), SET, Anchor::Start, &at)
        .unwrap();
    engine.end_frame();
    let stats = engine.shaping_stats();
    assert_eq!((stats.blocks, stats.number_faces), (3, 1));
    engine.begin_frame();
    engine
        .place_number(&span("12", &face), SET, Anchor::Start, &at)
        .unwrap();
    engine.end_frame();
    let stats = engine.shaping_stats();
    assert_eq!((stats.blocks, stats.number_faces, stats.evicted), (0, 1, 3));
    label_frame(&mut engine, &[], &face);
    assert_eq!(engine.shaping_stats().number_faces, 0);
}

fn warm_list() -> Vec<Prefill> {
    vec![
        Prefill {
            face: face("DM Sans", 24.0, 400),
            chars: "Score:0123456789".into(),
            scale: 2.0,
            representation: Representation::Coverage,
            bins: [true; 4],
            number: None,
        },
        Prefill {
            face: face("DM Sans", 40.0, 700),
            chars: "Hello world!".into(),
            scale: 2.0,
            representation: Representation::Msdf,
            bins: [true; 4],
            number: None,
        },
        Prefill {
            face: face("EB Garamond", 31.0, 500),
            chars: SET.into(),
            scale: 1.5,
            representation: Representation::Coverage,
            bins: [true, false, true, false],
            number: Some(NumberForms::default()),
        },
        Prefill {
            face: face("EB Garamond", 150.0, 400),
            chars: "fig 0123".into(),
            scale: 1.0,
            representation: Representation::Msdf,
            bins: [false; 4],
            number: Some(forms(Figures::TabularDigits, Ligatures::Kept)),
        },
    ]
}

fn warm_engine() -> TextEngine {
    let mut engine = TextEngine::new(SANS).unwrap();
    engine.register_font(SERIF).unwrap();
    engine
}

fn place_one(engine: &mut TextEngine, entry: &Prefill, c: char, origin: [f32; 2]) -> Placed {
    let at = placement(entry.scale, entry.representation, origin);
    let label = span(&c.to_string(), &entry.face);
    match entry.number {
        Some(forms) => engine
            .place_number_with(&label, &entry.chars, forms, Anchor::Start, &at)
            .unwrap(),
        None => engine
            .place_spans(&[label], None, Anchor::Start, &at)
            .unwrap(),
    }
}

fn on_demand(list: &[Prefill]) -> TextEngine {
    let mut engine = warm_engine();
    let mut probe = warm_engine();
    for entry in list {
        let mut chars = Vec::new();
        for c in entry.chars.chars() {
            if !chars.contains(&c) {
                chars.push(c);
            }
        }
        let passes = match entry.representation {
            Representation::Coverage => (0..4).filter(|&bin| entry.bins[bin]).collect(),
            Representation::Msdf => vec![0],
        };
        for bin in passes {
            for &c in &chars {
                let pen = place_one(&mut probe, entry, c, [0.0, 0.0])
                    .quads
                    .first()
                    .map_or(0.0, |quad| quad.quad.pen[0] * entry.scale);
                let x = (1.0 + bin as f32 * 0.25 - pen.rem_euclid(1.0)) / entry.scale;
                let placed = place_one(&mut engine, entry, c, [x, 0.0]);
                if let Some(quad) = placed.quads.first()
                    && entry.representation == Representation::Coverage
                {
                    let fraction = (quad.quad.pen[0] * entry.scale).rem_euclid(1.0);
                    assert!(
                        (fraction - bin as f32 * 0.25).abs() < 0.01,
                        "{c} {fraction}"
                    );
                }
            }
        }
    }
    engine
}

#[test]
fn prefilled_atlases_match_on_demand_byte_for_byte_at_any_thread_count() {
    let list = warm_list();
    let mut reference = on_demand(&list);
    let expected = reference.take_atlas_changes();
    let stats = reference.atlas_stats();
    assert!(expected.cells.len() > 100);
    for threads in [1, 2, 5, 32] {
        let mut engine = warm_engine();
        let made = engine.prefill_on(&list, threads).unwrap();
        assert_eq!(made as u64, stats.rasterized, "{threads}");
        let changes = engine.take_atlas_changes();
        assert!(changes == expected, "{threads} threads");
        let warm = engine.atlas_stats();
        assert_eq!(
            (warm.pages, warm.cells, warm.bytes),
            (stats.pages, stats.cells, stats.bytes)
        );
        for page in &changes.created {
            assert_eq!(
                engine.atlas_page(page.id).unwrap().levels,
                reference.atlas_page(page.id).unwrap().levels
            );
        }
        assert_eq!(engine.prefill_on(&list, threads).unwrap(), 0);
        assert_eq!(engine.atlas_stats().rasterized, warm.rasterized);
    }
}

#[test]
fn prefill_with_every_bin_leaves_nothing_to_rasterize_on_demand() {
    let mut list = warm_list();
    for entry in &mut list {
        entry.bins = [true; 4];
    }
    let mut engine = warm_engine();
    engine.prefill(&list).unwrap();
    let warm = engine.atlas_stats().rasterized;
    for entry in &list {
        for (index, c) in entry.chars.chars().enumerate() {
            place_one(&mut engine, entry, c, [index as f32 * 0.37, 1.3]);
        }
    }
    assert_eq!(engine.atlas_stats().rasterized, warm);
}
