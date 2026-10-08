use cosmic_text::fontdb;
use swash::scale::{Render, ScaleContext, Source};
use swash::zeno::{Command, PathData};

use crate::{CacheKeyFlags, GlyphImage, TextEngine, msdf_field, raster_key};

const NUMBERS: &str = "0123456789,.e+-Na";
const VARIABLE_FONT: &[u8] = include_bytes!("../tests/fonts/DMSans[opsz,wght].ttf");

pub struct Measure {
    pub worst: f32,
    pub worst_corner_distance: f32,
    pub worst_edge: f32,
    pub mean_edge: f32,
    pub wrong: u32,
}

type Segment = ([f32; 2], [f32; 2], [f32; 2]);

struct Glyph {
    font_id: fontdb::ID,
    glyph_id: u16,
}

fn glyph(engine: &mut TextEngine, c: char) -> Glyph {
    let font_id = engine.fonts.db().faces().next().unwrap().id;
    let font = engine.fonts.get_font(font_id, fontdb::Weight(400)).unwrap();
    let glyph_id = font.as_swash().charmap().map(c);
    Glyph { font_id, glyph_id }
}

fn corners(engine: &mut TextEngine, glyph: &Glyph, weight: u16, size: f32) -> Vec<[f32; 2]> {
    let font = engine
        .fonts
        .get_font(glyph.font_id, fontdb::Weight(weight))
        .unwrap();
    let key = raster_key(
        glyph.font_id,
        glyph.glyph_id,
        size,
        fontdb::Weight(weight),
        size,
        CacheKeyFlags::empty(),
        font.as_swash(),
    );
    let mut context = ScaleContext::new();
    let mut scaler = context
        .builder(font.as_swash())
        .size(size)
        .hint(false)
        .variations(key.variations())
        .build();
    let outline = scaler.scale_outline(glyph.glyph_id).unwrap();
    let mut contours: Vec<Vec<Segment>> = Vec::new();
    let mut contour = Vec::new();
    let mut current = [0.0f32; 2];
    let mut start = [0.0f32; 2];
    let sub = |a: [f32; 2], b: [f32; 2]| [a[0] - b[0], a[1] - b[1]];
    for command in outline.path().commands() {
        let segment = match command {
            Command::MoveTo(p) => {
                if !contour.is_empty() {
                    contours.push(std::mem::take(&mut contour));
                }
                current = [p.x, -p.y];
                start = current;
                continue;
            }
            Command::LineTo(p) => {
                let end = [p.x, -p.y];
                (sub(end, current), sub(end, current), end)
            }
            Command::QuadTo(c, p) => {
                let (c, end) = ([c.x, -c.y], [p.x, -p.y]);
                (sub(c, current), sub(end, c), end)
            }
            Command::CurveTo(c, d, p) => {
                let (c, d, end) = ([c.x, -c.y], [d.x, -d.y], [p.x, -p.y]);
                (sub(c, current), sub(end, d), end)
            }
            Command::Close => {
                if current != start {
                    contour.push((sub(start, current), sub(start, current), start));
                    current = start;
                }
                continue;
            }
        };
        contour.push(segment);
        current = segment.2;
    }
    if !contour.is_empty() {
        contours.push(contour);
    }
    let mut found = Vec::new();
    for contour in &contours {
        let n = contour.len();
        for i in 0..n {
            let incoming = contour[i].1;
            let outgoing = contour[(i + 1) % n].0;
            let a = incoming[0].hypot(incoming[1]);
            let b = outgoing[0].hypot(outgoing[1]);
            if a > 0.0
                && b > 0.0
                && (incoming[0] * outgoing[0] + incoming[1] * outgoing[1]) / (a * b) < 0.9
            {
                found.push(contour[i].2);
            }
        }
    }
    found
}

fn field(
    engine: &mut TextEngine,
    glyph: &Glyph,
    weight: u16,
    size: f32,
    ppem: f32,
    sharp: bool,
) -> GlyphImage {
    let font = engine
        .fonts
        .get_font(glyph.font_id, fontdb::Weight(weight))
        .unwrap();
    let key = raster_key(
        glyph.font_id,
        glyph.glyph_id,
        ppem,
        fontdb::Weight(weight),
        size,
        CacheKeyFlags::empty(),
        font.as_swash(),
    );
    let mut scaler = engine
        .context
        .builder(font.as_swash())
        .size(ppem)
        .hint(false)
        .variations(key.variations())
        .build();
    msdf_field(&mut scaler, glyph.glyph_id, false, [0.0; 2], sharp).unwrap()
}

fn reference(
    engine: &mut TextEngine,
    glyph: &Glyph,
    weight: u16,
    size: f32,
) -> (i32, i32, u32, u32, Vec<u8>) {
    let font = engine
        .fonts
        .get_font(glyph.font_id, fontdb::Weight(weight))
        .unwrap();
    let key = raster_key(
        glyph.font_id,
        glyph.glyph_id,
        size,
        fontdb::Weight(weight),
        size,
        CacheKeyFlags::empty(),
        font.as_swash(),
    );
    let mut scaler = engine
        .context
        .builder(font.as_swash())
        .size(size)
        .hint(false)
        .variations(key.variations())
        .build();
    let image = Render::new(&[Source::Outline])
        .render(&mut scaler, glyph.glyph_id)
        .unwrap();
    (
        image.placement.left,
        -image.placement.top,
        image.placement.width,
        image.placement.height,
        image.data,
    )
}

fn sample(image: &GlyphImage, t: [f32; 2]) -> f32 {
    let x = t[0].clamp(0.0, image.width as f32 - 1.0);
    let y = t[1].clamp(0.0, image.height as f32 - 1.0);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |px: f32, py: f32| {
        let px = (px as u32).min(image.width - 1);
        let py = (py as u32).min(image.height - 1);
        let base = ((py * image.width + px) * 3) as usize;
        let [r, g, b] = [0, 1, 2].map(|k| f32::from(image.bytes[base + k]) / 255.0);
        [r, g, b]
    };
    let mut rgb = [0.0f32; 3];
    for (texel, weight) in [
        (at(x0, y0), (1.0 - fx) * (1.0 - fy)),
        (at(x0 + 1.0, y0), fx * (1.0 - fy)),
        (at(x0, y0 + 1.0), (1.0 - fx) * fy),
        (at(x0 + 1.0, y0 + 1.0), fx * fy),
    ] {
        for k in 0..3 {
            rgb[k] += texel[k] * weight;
        }
    }
    rgb[0].min(rgb[1]).max(rgb[0].max(rgb[1]).min(rgb[2])) - 0.5
}

pub fn emulate(image: &GlyphImage, ppem: f32, size: f32, area: [i32; 4]) -> Vec<f32> {
    let [x0, y0, width, height] = area;
    let ratio = ppem / size;
    let distance = |x: i32, y: i32| {
        let f = [(x as f32 + 0.5) * ratio, (y as f32 + 0.5) * ratio];
        sample(image, [f[0] - image.left - 0.5, f[1] - image.top - 0.5])
    };
    let mut out = vec![0.0; (width * height) as usize];
    for y in y0..y0 + height {
        for x in x0..x0 + width {
            let f = [(x as f32 + 0.5) * ratio, (y as f32 + 0.5) * ratio];
            let inside = f[0] >= image.left
                && f[0] <= image.left + image.width as f32
                && f[1] >= image.top
                && f[1] <= image.top + image.height as f32;
            if !inside {
                continue;
            }
            let (qx, qy) = (x & !1, y & !1);
            let dx = distance(qx + 1, y) - distance(qx, y);
            let dy = distance(x, qy + 1) - distance(x, qy);
            let sd = distance(x, y);
            let alpha = (0.5 + sd / (dx.abs() + dy.abs()).max(1e-5)).clamp(0.0, 1.0);
            out[((y - y0) * width + (x - x0)) as usize] = alpha;
        }
    }
    out
}

pub fn measure(
    engine: &mut TextEngine,
    c: char,
    weight: u16,
    size: f32,
    ppem: f32,
    sharp: bool,
) -> Measure {
    let glyph = glyph(engine, c);
    let image = field(engine, &glyph, weight, size, ppem, sharp);
    let (left, top, width, height, coverage) = reference(engine, &glyph, weight, size);
    let area = [left - 2, top - 2, width as i32 + 4, height as i32 + 4];
    let alpha = emulate(&image, ppem, size, area);
    let corners = corners(engine, &glyph, weight, size);
    let mut result = Measure {
        worst: 0.0,
        worst_corner_distance: f32::INFINITY,
        worst_edge: 0.0,
        mean_edge: 0.0,
        wrong: 0,
    };
    let mut edge_sum = 0.0;
    let mut edge_count = 0;
    for y in area[1]..area[1] + area[3] {
        for x in area[0]..area[0] + area[2] {
            let (rx, ry) = (x - left, y - top);
            let truth = if rx >= 0 && ry >= 0 && rx < width as i32 && ry < height as i32 {
                f32::from(coverage[(ry as u32 * width + rx as u32) as usize]) / 255.0
            } else {
                0.0
            };
            let got = alpha[((y - area[1]) * area[2] + (x - area[0])) as usize];
            let error = (got - truth).abs();
            let centre = [x as f32 + 0.5, y as f32 + 0.5];
            let near = corners
                .iter()
                .map(|p| (p[0] - centre[0]).hypot(p[1] - centre[1]))
                .fold(f32::INFINITY, f32::min);
            if error > result.worst {
                result.worst = error;
                result.worst_corner_distance = near;
            }
            if error > 0.5 {
                result.wrong += 1;
            }
            if near > 2.0 {
                result.worst_edge = result.worst_edge.max(error);
            }
            if truth > 0.0 && truth < 1.0 {
                edge_sum += error;
                edge_count += 1;
            }
        }
    }
    result.mean_edge = edge_sum / edge_count.max(1) as f32;
    result
}

pub fn sans() -> TextEngine {
    match std::env::var("PITO_TEXT_SANS") {
        Ok(path) => TextEngine::new(&std::fs::read(path).unwrap()).unwrap(),
        Err(_) => TextEngine::new(VARIABLE_FONT).unwrap(),
    }
}

#[test]
fn a_finer_field_keeps_large_corners_sharp() {
    let mut engine = TextEngine::new(VARIABLE_FONT).unwrap();
    let mut wrong = [0u32; 3];
    for c in ['1', '4', 'N'] {
        for (slot, (ppem, sharp)) in
            wrong
                .iter_mut()
                .zip([(64.0, false), (64.0, true), (128.0, true)])
        {
            *slot += measure(&mut engine, c, 800, 300.0, ppem, sharp).wrong;
        }
    }
    assert!(wrong[1] * 4 < wrong[0], "{wrong:?}");
    assert!(wrong[2] * 2 <= wrong[1], "{wrong:?}");
    let small = measure(&mut engine, '4', 400, 38.0, 64.0, true);
    assert!(small.mean_edge < 0.1, "{}", small.mean_edge);
    assert_eq!(small.wrong, 0);
}

#[test]
#[ignore = "measurement; run with --release --nocapture"]
fn msdf_against_exact_coverage_at_4k() {
    let mut engine = sans();
    let family = engine.fonts.db().faces().next().unwrap().families[0]
        .0
        .clone();
    println!("{family}");
    println!(
        "| weight | px | field | worst /255 | at corner (px) | worst edge away from corners /255 | mean edge /255 | pixels off by more than half |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    for weight in [400u16, 600, 800] {
        for size in [38.0f32, 56.0, 76.0, 96.0, 112.0, 128.0, 150.0, 200.0, 300.0] {
            for (ppem, sharp) in [(64.0f32, false), (64.0, true), (128.0, true)] {
                let mut worst = Measure {
                    worst: 0.0,
                    worst_corner_distance: 0.0,
                    worst_edge: 0.0,
                    mean_edge: 0.0,
                    wrong: 0,
                };
                let mut where_char = ' ';
                let mut mean = 0.0;
                for c in NUMBERS.chars() {
                    let m = measure(&mut engine, c, weight, size, ppem, sharp);
                    mean += m.mean_edge / NUMBERS.len() as f32;
                    worst.worst_edge = worst.worst_edge.max(m.worst_edge);
                    worst.wrong += m.wrong;
                    if m.worst > worst.worst {
                        worst.worst = m.worst;
                        worst.worst_corner_distance = m.worst_corner_distance;
                        where_char = c;
                    }
                }
                println!(
                    "| {weight} | {size} | {ppem}{} | {:.0} ('{where_char}') | {:.1} | {:.0} | {:.1} | {} |",
                    if sharp { " sharp" } else { " round" },
                    worst.worst * 255.0,
                    worst.worst_corner_distance,
                    worst.worst_edge * 255.0,
                    mean * 255.0,
                    worst.wrong
                );
            }
        }
    }
}
