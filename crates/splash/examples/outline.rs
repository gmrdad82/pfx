use std::fmt::Write as _;
use std::path::Path;

use swash::scale::ScaleContext;
use swash::zeno::Verb;
use swash::{FontRef, GlyphId};

const EM: f32 = 1000.0;

fn number(v: f32) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn outline(font: FontRef, text: &str, tracking: f32) -> Result<String, String> {
    let mut context = ScaleContext::new();
    let mut scaler = context.builder(font).size(EM).hint(false).build();
    let metrics = font.glyph_metrics(&[]).scale(EM);
    let cap = font.metrics(&[]).scale(EM).cap_height;
    let mut d = String::new();
    let mut pen = 0.0f32;
    let mut ink = [f32::MAX, f32::MIN];
    let ids: Vec<GlyphId> = text.chars().map(|c| font.charmap().map(c)).collect();
    for (i, id) in ids.iter().enumerate() {
        if *id == 0 && !text.chars().nth(i).is_some_and(char::is_whitespace) {
            return Err(format!("the font has no {:?}", text.chars().nth(i)));
        }
        if let Some(shape) = scaler.scale_outline(*id) {
            let points = shape.points();
            for q in points {
                ink = [ink[0].min(pen + q.x), ink[1].max(pen + q.x)];
            }
            let mut at = 0;
            let mut p = || {
                let q = points[at];
                at += 1;
                format!("{} {}", number(pen + q.x), number(-q.y))
            };
            for verb in shape.verbs() {
                match verb {
                    Verb::MoveTo => d.push_str(&format!("M{}", p())),
                    Verb::LineTo => d.push_str(&format!("L{}", p())),
                    Verb::QuadTo => {
                        let a = p();
                        let b = p();
                        d.push_str(&format!("Q{a} {b}"));
                    }
                    Verb::CurveTo => {
                        let a = p();
                        let b = p();
                        let c = p();
                        d.push_str(&format!("C{a} {b} {c}"));
                    }
                    Verb::Close => d.push('Z'),
                }
            }
        }
        pen += metrics.advance_width(*id);
        if i + 1 < ids.len() {
            pen += tracking * EM;
        }
    }
    let mut svg = String::new();
    let _ = write!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\" width=\"{}\" height=\"{}\">\n<path fill=\"currentColor\" d=\"{d}\"/>\n</svg>\n",
        number(ink[0]),
        number(-cap),
        number(ink[1] - ink[0]),
        number(cap),
        number((ink[1] - ink[0]) / cap * 10.0),
        10
    );
    Ok(svg)
}

fn main() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = root.join("../../assets/fonts/JetBrainsMonoNerdFont-Bold.ttf");
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let font = FontRef::from_index(&bytes, 0).ok_or("not a font")?;
    let out = root.join("art").join("site.svg");
    std::fs::write(&out, outline(font, "PITOMD.COM", 0.32)?)
        .map_err(|e| format!("{}: {e}", out.display()))?;
    println!("{}", out.display());
    Ok(())
}
