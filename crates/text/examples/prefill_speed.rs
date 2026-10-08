use std::time::Instant;

use pfx_text::{Anchor, Face, Placement, Prefill, Representation, Span, TextEngine};

const SANS: &[u8] = include_bytes!("../tests/fonts/DMSans[opsz,wght].ttf");
const SERIF: &[u8] = include_bytes!("../fonts/EBGaramond[wght].ttf");

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

fn engine() -> TextEngine {
    let mut engine = TextEngine::new(SANS).unwrap();
    engine.register_font(SERIF).unwrap();
    engine.set_atlas_budget(256 << 20);
    engine
}

fn on_demand(list: &[Prefill]) -> TextEngine {
    let mut engine = engine();
    for entry in list {
        let at = Placement {
            scale: entry.scale,
            representation: entry.representation,
            origin: [0.0, 0.0],
            alpha: 1.0,
            clip: [-f32::MAX, -f32::MAX, f32::MAX, f32::MAX],
            turn: [0.0; 3],
        };
        for c in entry.chars.chars() {
            let span = Span {
                text: c.to_string(),
                face: entry.face.clone(),
                color: [1.0; 4],
            };
            engine
                .place_spans(&[span], None, Anchor::Start, &at)
                .unwrap();
        }
    }
    engine
}

fn main() {
    let chars = (33u8..127).map(char::from).collect::<String>();
    let list = [
        ("DM Sans", 32.0, 400),
        ("DM Sans", 48.0, 800),
        ("EB Garamond", 40.0, 500),
        ("EB Garamond", 150.0, 700),
    ]
    .map(|(family, size, weight)| Prefill {
        face: face(family, size, weight),
        chars: chars.clone(),
        scale: 2.0,
        representation: Representation::Msdf,
        bins: [true; 4],
        number: None,
    });
    let start = Instant::now();
    let mut serial = on_demand(&list);
    let took = start.elapsed().as_secs_f64() * 1000.0;
    let expected = serial.take_atlas_changes();
    println!(
        "on demand, {} fields on the caller's thread: {took:.0} ms",
        serial.atlas_stats().rasterized
    );
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    for count in [1, threads] {
        let mut warm = engine();
        let start = Instant::now();
        let made = warm.prefill_on(&list, count).unwrap();
        let took = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(warm.take_atlas_changes(), expected);
        println!("prefill of {made} fields on {count} threads: {took:.0} ms, the same bytes");
    }
}
