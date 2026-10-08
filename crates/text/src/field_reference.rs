use crate::{GlyphImage, MSDF_SPREAD as SPREAD};

const CORNER: f32 = 0.75;
const CLASH: f32 = 1.001 / SPREAD;
const COLORS: [u8; 3] = [0b011, 0b110, 0b101];

#[derive(Clone, Copy)]
pub(crate) struct Edge {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub mask: u8,
}

pub(crate) fn crossing(p: [f32; 2], edge: &Edge) -> i32 {
    let (a, b) = (edge.a, edge.b);
    if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
    {
        if b[1] > a[1] { 1 } else { -1 }
    } else {
        0
    }
}

pub(crate) fn boundary_edges(edges: &[Edge]) -> Vec<(Edge, f32)> {
    let winding = |p: [f32; 2]| edges.iter().map(|edge| crossing(p, edge)).sum::<i32>();
    let mut kept = Vec::with_capacity(edges.len());
    for edge in edges {
        let v = [edge.b[0] - edge.a[0], edge.b[1] - edge.a[1]];
        let length = v[0].hypot(v[1]);
        if length <= 0.0 {
            continue;
        }
        let pieces = length.ceil().max(1.0) as usize;
        let normal = [-v[1] / length * 0.05, v[0] / length * 0.05];
        let at = |t: f32| [edge.a[0] + v[0] * t, edge.a[1] + v[1] * t];
        let sides = (0..pieces)
            .map(|i| {
                let m = at((i as f32 + 0.5) / pieces as f32);
                let left = winding([m[0] + normal[0], m[1] + normal[1]]) != 0;
                let right = winding([m[0] - normal[0], m[1] - normal[1]]) != 0;
                (left != right).then_some(if left { 1.0 } else { -1.0 })
            })
            .collect::<Vec<_>>();
        if let Some(side) = sides
            .iter()
            .all(Option::is_some)
            .then(|| sides[pieces / 2])
            .flatten()
        {
            kept.push((*edge, side));
            continue;
        }
        let mut i = 0;
        while i < pieces {
            let Some(side) = sides[i] else {
                i += 1;
                continue;
            };
            let start = i;
            while i < pieces && sides[i] == Some(side) {
                i += 1;
            }
            kept.push((
                Edge {
                    a: at(start as f32 / pieces as f32),
                    b: at(i as f32 / pieces as f32),
                    mask: edge.mask,
                },
                side,
            ));
        }
    }
    kept
}

fn colored(contours: &[Vec<[f32; 2]>]) -> Vec<Edge> {
    let mut edges = Vec::new();
    for contour in contours {
        let mut points = contour.clone();
        points.dedup();
        while points.len() > 1 && points.first() == points.last() {
            points.pop();
        }
        let n = points.len();
        if n < 2 {
            continue;
        }
        let corners = (0..n)
            .filter(|&i| {
                let prev = points[(i + n - 1) % n];
                let a = points[i];
                let b = points[(i + 1) % n];
                let into = [a[0] - prev[0], a[1] - prev[1]];
                let out = [b[0] - a[0], b[1] - a[1]];
                let lens = into[0].hypot(into[1]) * out[0].hypot(out[1]);
                lens > 0.0 && (into[0] * out[0] + into[1] * out[1]) / lens < CORNER
            })
            .collect::<Vec<_>>();
        let k = corners.len();
        let first = corners.first().copied().unwrap_or(0);
        for step in 0..n {
            let i = (first + step) % n;
            let mask = match k {
                0 => 0b111,
                1 => COLORS[(step * 3 / n).min(2)],
                _ => {
                    let run = corners.partition_point(|&c| c <= i);
                    let run = if run == 0 { k - 1 } else { run - 1 };
                    if k % 3 == 1 && run == k - 1 {
                        0b110
                    } else {
                        COLORS[run % 3]
                    }
                }
            };
            edges.push(Edge {
                a: points[i],
                b: points[(i + 1) % n],
                mask,
            });
        }
    }
    edges
}

struct Prepared {
    a: [f32; 2],
    b: [f32; 2],
    unit: [f32; 2],
    length: f32,
    side: f32,
    mask: u8,
}

fn median(rgb: [f32; 3]) -> f32 {
    rgb[0].min(rgb[1]).max(rgb[0].max(rgb[1]).min(rgb[2]))
}

fn clash(a: [f32; 3], b: [f32; 3]) -> bool {
    let mut pairs = [(a[0], b[0]), (a[1], b[1]), (a[2], b[2])];
    pairs.sort_by(|x, y| (y.1 - y.0).abs().total_cmp(&(x.1 - x.0).abs()));
    let [(_, b0), (a1, b1), (a2, b2)] = pairs;
    (b1 - a1).abs() >= CLASH && !(b0 == b1 && b0 == b2) && (a2 - 0.5).abs() >= (b2 - 0.5).abs()
}

pub(crate) fn sharp(
    contours: &[Vec<[f32; 2]>],
    corner: [f32; 2],
    size: [u32; 2],
) -> Option<GlyphImage> {
    let edges = colored(contours);
    let boundary = boundary_edges(&edges)
        .into_iter()
        .filter_map(|(edge, side)| {
            let v = [edge.b[0] - edge.a[0], edge.b[1] - edge.a[1]];
            let length = v[0].hypot(v[1]);
            (length > 0.0).then_some(Prepared {
                a: edge.a,
                b: edge.b,
                unit: [v[0] / length, v[1] / length],
                length,
                side,
                mask: edge.mask,
            })
        })
        .collect::<Vec<_>>();
    if boundary.is_empty() {
        return None;
    }
    let [width, height] = size;
    let mut values = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let p = [corner[0] + x as f32 + 0.5, corner[1] + y as f32 + 0.5];
            let inside = edges.iter().map(|edge| crossing(p, edge)).sum::<i32>() != 0;
            let mut best = [(f32::INFINITY, f32::INFINITY, 0.0f32); 3];
            let mut global = f32::INFINITY;
            for edge in &boundary {
                let w = [p[0] - edge.a[0], p[1] - edge.a[1]];
                let t = w[0] * edge.unit[0] + w[1] * edge.unit[1];
                let perp = (w[1] * edge.unit[0] - w[0] * edge.unit[1]) * edge.side;
                let (distance, dot) = if t < 0.0 {
                    let d = w[0].hypot(w[1]);
                    (d, if d > 0.0 { -t / d } else { 0.0 })
                } else if t > edge.length {
                    let e = [p[0] - edge.b[0], p[1] - edge.b[1]];
                    let d = e[0].hypot(e[1]);
                    (d, if d > 0.0 { (t - edge.length) / d } else { 0.0 })
                } else {
                    (perp.abs(), 0.0)
                };
                global = global.min(distance);
                for (channel, slot) in best.iter_mut().enumerate() {
                    if edge.mask & (1 << channel) == 0 {
                        continue;
                    }
                    let closer = distance < slot.0 - 1e-4
                        || ((distance - slot.0).abs() <= 1e-4 && dot < slot.1);
                    if closer {
                        *slot = (distance, dot, perp);
                    }
                }
            }
            let truth = if inside { global } else { -global };
            let mut rgb = best.map(|slot| {
                if slot.0.is_finite() {
                    0.5 + slot.2 / SPREAD
                } else {
                    0.5 + truth / SPREAD
                }
            });
            if (median(rgb) - 0.5) * truth < 0.0 || (median(rgb) - 0.5 == 0.0 && truth != 0.0) {
                rgb = [0.5 + truth / SPREAD; 3];
            }
            values.push(rgb);
        }
    }
    let at = |x: u32, y: u32| (y * width + x) as usize;
    let mut flagged = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let here = values[at(x, y)];
            let neighbours = [
                (x > 0).then(|| at(x - 1, y)),
                (x + 1 < width).then(|| at(x + 1, y)),
                (y > 0).then(|| at(x, y - 1)),
                (y + 1 < height).then(|| at(x, y + 1)),
            ];
            if neighbours
                .into_iter()
                .flatten()
                .any(|other| clash(here, values[other]))
            {
                flagged.push(at(x, y));
            }
        }
    }
    for index in flagged {
        let m = median(values[index]);
        values[index] = [m; 3];
    }
    let bytes = values
        .into_iter()
        .flat_map(|rgb| rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
        .collect();
    Some(GlyphImage {
        left: corner[0],
        top: corner[1],
        width,
        height,
        bytes,
    })
}

mod tests {
    use std::time::Instant;

    use swash::scale::ScaleContext;

    use crate::{Contours, GlyphImage, field, glyph_contours};

    const SANS: &[u8] = include_bytes!("../tests/fonts/DMSans[opsz,wght].ttf");
    const SERIF: &[u8] = include_bytes!("../fonts/EBGaramond[wght].ttf");
    const CJK: &[u8] = include_bytes!("../tests/fonts/NotoSansSC-subset.ttf");

    fn outlines(bytes: &[u8], weight: f32, ppem: f32, chars: &str) -> Vec<Contours> {
        let font = swash::FontRef::from_index(bytes, 0).unwrap();
        let mut context = ScaleContext::new();
        let mut scaler = context
            .builder(font)
            .size(ppem)
            .hint(false)
            .variations([("wght", weight)])
            .build();
        let mut out = Vec::new();
        for (index, c) in chars.chars().enumerate() {
            let glyph = font.charmap().map(c);
            let offset = [index as f32 % 4.0 * 0.25, 0.0];
            if let Some(contours) = glyph_contours(&mut scaler, glyph, index % 3 == 1, offset) {
                out.push(contours);
            }
        }
        out
    }

    fn cases(ppems: &[f32]) -> Vec<Contours> {
        let latin = (33u8..127).map(char::from).collect::<String>() + "éßøÆœ€—“”";
        let mut all = Vec::new();
        for &ppem in ppems {
            for (bytes, weight) in [(SANS, 400.0), (SANS, 800.0), (SERIF, 400.0), (SERIF, 700.0)] {
                all.extend(outlines(bytes, weight, ppem, &latin));
            }
            all.extend(outlines(CJK, 400.0, ppem, "中文字體測試永龍鬱"));
        }
        all
    }

    fn same(a: Option<GlyphImage>, b: Option<GlyphImage>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.left == b.left
                    && a.top == b.top
                    && a.width == b.width
                    && a.height == b.height
                    && a.bytes == b.bytes
            }
            _ => false,
        }
    }

    #[test]
    fn the_field_generator_matches_its_reference_byte_for_byte() {
        let cases = cases(&[64.0]);
        assert!(cases.len() > 300);
        for (index, (contours, corner, size)) in cases.iter().enumerate().step_by(3) {
            assert!(
                same(
                    field::sharp(contours, *corner, *size),
                    super::sharp(contours, *corner, *size)
                ),
                "case {index}"
            );
        }
    }

    #[test]
    #[ignore = "a measurement; run by hand with --release"]
    fn field_speed() {
        for ppem in [64.0, 128.0] {
            let cases = cases(&[ppem]);
            let mut times = [0.0f64; 2];
            for _ in 0..3 {
                for (slot, run) in [
                    (
                        0,
                        field::sharp as fn(&[Vec<[f32; 2]>], [f32; 2], [u32; 2]) -> _,
                    ),
                    (1, super::sharp),
                ] {
                    let start = Instant::now();
                    for (contours, corner, size) in &cases {
                        std::hint::black_box(run(contours, *corner, *size));
                    }
                    times[slot] += start.elapsed().as_secs_f64();
                }
            }
            for (contours, corner, size) in &cases {
                assert!(same(
                    field::sharp(contours, *corner, *size),
                    super::sharp(contours, *corner, *size)
                ));
            }
            let per = |t: f64| t * 1000.0 / (3.0 * cases.len() as f64);
            println!(
                "{} fields at {ppem} ppem: {:.3} ms each now, {:.3} ms with the reference",
                cases.len(),
                per(times[0]),
                per(times[1])
            );
        }
    }

    #[test]
    #[ignore = "every glyph of the engine's fonts; run by hand with --release"]
    fn every_glyph_matches_the_reference() {
        let mut count = 0;
        for bytes in [SANS, SERIF, CJK] {
            let font = swash::FontRef::from_index(bytes, 0).unwrap();
            let glyphs = font.metrics(&[]).glyph_count;
            let mut context = ScaleContext::new();
            for (ppem, weight) in [(64.0, 400.0), (128.0, 800.0)] {
                let mut scaler = context
                    .builder(font)
                    .size(ppem)
                    .hint(false)
                    .variations([("wght", weight)])
                    .build();
                for glyph in 0..glyphs {
                    let offset = [f32::from(glyph % 4) * 0.25, 0.0];
                    let Some((contours, corner, size)) =
                        glyph_contours(&mut scaler, glyph, glyph % 5 == 2, offset)
                    else {
                        continue;
                    };
                    assert!(
                        same(
                            field::sharp(&contours, corner, size),
                            super::sharp(&contours, corner, size)
                        ),
                        "glyph {glyph} at {ppem}"
                    );
                    count += 1;
                }
            }
        }
        println!("{count} fields match");
    }
}
