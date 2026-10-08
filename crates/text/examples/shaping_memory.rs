use pfx_text::{Anchor, Face, NumberForms, Placement, Representation, Span, TextEngine};

const SANS: &[u8] = include_bytes!("../tests/fonts/DMSans[opsz,wght].ttf");

fn resident() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kib * 1024)
}

fn main() {
    let mut engine = TextEngine::new(SANS).unwrap();
    engine.set_shaping_budget(u64::MAX);
    let face = Face {
        family: "DM Sans".into(),
        size: 24.0,
        line: 30.0,
        weight: 400,
        italic: false,
        spacing: 0.0,
    };
    let at = Placement {
        scale: 2.0,
        representation: Representation::Msdf,
        origin: [0.0, 0.0],
        alpha: 1.0,
        clip: [-f32::MAX, -f32::MAX, f32::MAX, f32::MAX],
        turn: [0.0; 3],
    };
    let label = |text: String| Span {
        text,
        face: face.clone(),
        color: [1.0; 4],
    };
    engine
        .place_spans(&[label("warm up".into())], None, Anchor::Start, &at)
        .unwrap();
    for (count, words) in [
        (40_000, "OK"),
        (20_000, "Menu entry"),
        (5_000, "Settings and controls page"),
        (
            2_000,
            "A longer line of help text that explains what this option does in the game",
        ),
    ] {
        let before = (resident(), engine.shaping_stats().bytes);
        let mut glyphs = 0;
        for i in 0..count {
            let text = format!("{words} {i}");
            glyphs += text.chars().count();
            engine
                .place_spans(&[label(text)], None, Anchor::Start, &at)
                .unwrap();
        }
        let after = (resident(), engine.shaping_stats().bytes);
        let estimate = after.1 - before.1;
        let measured = after.0.zip(before.0).map(|(a, b)| a.saturating_sub(b));
        println!(
            "{count} labels like \"{words} {}\", {:.1} glyphs: estimate {} bytes a label ({:.0} a glyph), resident growth {}",
            count - 1,
            glyphs as f64 / count as f64,
            estimate / count as u64,
            estimate as f64 / glyphs as f64,
            measured.map_or("unknown".into(), |m| format!(
                "{} bytes a label",
                m / count as u64
            )),
        );
    }
    let set = "0123456789,.-+%:()";
    let before = engine.shaping_stats().bytes;
    engine
        .place_number_with(
            &label("1,284".into()),
            set,
            NumberForms::default(),
            Anchor::Start,
            &at,
        )
        .unwrap();
    println!(
        "a number face of {} characters: estimate {} bytes",
        set.len(),
        engine.shaping_stats().bytes - before
    );
}
