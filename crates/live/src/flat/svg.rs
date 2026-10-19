use super::icons::{Icon, Icons};
use super::{
    Draw, Fill, FlatScene, Glow, Gradient, Group, IDENTITY, IconShape, Light, Matrix, Shadow,
    ShadowCurve, Shape, SpriteFill, Srgba, Stroke, cpu, multiply, rotation, scaling, translation,
};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::mesh::float::stroke::offset::StrokeOffset;
use i_overlay::mesh::float::style::{LineCap, LineJoin, StrokeStyle};
use std::collections::HashMap;

const MARK_REACH: f32 = 0.04;
const ARC_STEP: f32 = 0.1;
const MITER_LIMIT: f32 = 4.0;
const ELLIPSE_STEPS: usize = 72;

mod blur;
pub mod filter;
#[cfg(test)]
mod filter_tests;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filter {
    Elevation(f32),
    Glow { sigma: f32 },
}

#[derive(Clone, Debug, Default)]
pub struct Board {
    pub size: [f32; 2],
    pub draws: Vec<Draw>,
    pub groups: Vec<Group>,
    pub icons: Vec<IconShape>,
    pub rasters: Vec<Raster>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Raster {
    pub draw: usize,
    pub region: [f32; 4],
    pub place: Matrix,
    bounds: [f32; 4],
    view: [f32; 2],
    filter: filter::Filter,
    source: Vec<Draw>,
    groups: Vec<Group>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Painted {
    pub size: [u32; 2],
    pub rgba: Vec<u8>,
}

fn apply(m: &Matrix, p: [f32; 2]) -> [f32; 2] {
    [
        m[0][0] * p[0] + m[1][0] * p[1] + m[3][0],
        m[0][1] * p[0] + m[1][1] * p[1] + m[3][1],
    ]
}

fn inverted(m: &Matrix) -> Result<Matrix, String> {
    let (a, b, c, d) = (m[0][0], m[0][1], m[1][0], m[1][1]);
    let det = a * d - b * c;
    if det.abs() <= f32::EPSILON * (a.abs() + b.abs() + c.abs() + d.abs()).max(1.0) {
        return Err("a filtered shape's transform cannot be inverted".into());
    }
    let mut out = IDENTITY;
    out[0][0] = d / det;
    out[0][1] = -b / det;
    out[1][0] = -c / det;
    out[1][1] = a / det;
    let [tx, ty] = [m[3][0], m[3][1]];
    out[3][0] = -(out[0][0] * tx + out[1][0] * ty);
    out[3][1] = -(out[0][1] * tx + out[1][1] * ty);
    Ok(out)
}

fn stretch(m: &Matrix) -> f32 {
    let column = |c: usize| (m[c][0] * m[c][0] + m[c][1] * m[c][1]).sqrt();
    column(0).max(column(1))
}

fn shape_bounds(draw: &Draw, icons: &[IconShape]) -> Option<[f32; 4]> {
    let half = match draw.shape {
        Shape::Rect { half, .. } => half,
        Shape::Circle { radius } => [radius; 2],
        Shape::Ring { radius, width } | Shape::Arc { radius, width, .. } => {
            [radius + width * 0.5; 2]
        }
        Shape::Icon(Icon(i)) => return icons.get(i as usize)?.bounds(),
        Shape::Text(_) => return None,
    };
    Some([-half[0], -half[1], half[0], half[1]])
}

fn union(draws: &[Draw], icons: &[IconShape], to_user: &Matrix) -> Option<[f32; 4]> {
    let mut out: Option<[f32; 4]> = None;
    for d in draws {
        let Some([x0, y0, x1, y1]) = shape_bounds(d, icons) else {
            continue;
        };
        let m = multiply(*to_user, d.transform);
        for corner in [[x0, y0], [x1, y0], [x0, y1], [x1, y1]] {
            let [x, y] = apply(&m, corner);
            out = Some(match out {
                None => [x, y, x, y],
                Some([a, b, c, e]) => [a.min(x), b.min(y), c.max(x), e.max(y)],
            });
        }
    }
    out
}

impl Raster {
    fn new(
        filter: filter::Filter,
        source: Vec<Draw>,
        place: Matrix,
        board: &Board,
    ) -> Result<Raster, String> {
        let to_user = inverted(&place)?;
        let bounds = union(&source, &board.icons, &to_user).unwrap_or([0.0; 4]);
        let region = filter.region(bounds, board.size)?;
        Ok(Raster {
            draw: board.draws.len(),
            region,
            place,
            bounds,
            view: board.size,
            filter,
            source,
            groups: board.groups.clone(),
        })
    }

    fn placeholder(&self, opacity: f32, group: Option<u16>, elevation: f32) -> Draw {
        let [x0, y0, x1, y1] = self.region;
        let mut d = Draw::new(Shape::Rect {
            half: [(x1 - x0) * 0.5, (y1 - y0) * 0.5],
            radii: [0.0; 4],
        })
        .transform(multiply(
            self.place,
            translation((x0 + x1) * 0.5, (y0 + y1) * 0.5),
        ));
        d.fill = Fill::Image(SpriteFill::new([0.0, 0.0, 1.0, 1.0]));
        d.opacity = opacity;
        d.group = group;
        d.elevation = elevation;
        d.shadow = Shadow::None;
        d
    }

    pub fn pixels(&self, scale: f32, most: u32) -> [u32; 2] {
        let k = scale * stretch(&self.place);
        let size = [
            (self.region[2] - self.region[0]) * k,
            (self.region[3] - self.region[1]) * k,
        ];
        let fit = (most.max(1) as f32 / size[0].max(size[1])).min(1.0);
        size.map(|v| ((v * fit).ceil() as u32).clamp(1, most.max(1)))
    }

    pub fn paint(&self, icons: &[IconShape], pixels: [u32; 2]) -> Result<Painted, String> {
        let [x0, y0, x1, y1] = self.region;
        let to_pixels = multiply(
            scaling(pixels[0] as f32 / (x1 - x0), pixels[1] as f32 / (y1 - y0)),
            multiply(translation(-x0, -y0), inverted(&self.place)?),
        );
        let mut used: Vec<u32> = Vec::new();
        let mut draws: Vec<Draw> = self
            .source
            .iter()
            .map(|d| {
                let mut d = d.transform(multiply(to_pixels, d.transform));
                if let Shape::Icon(Icon(i)) = d.shape {
                    let at = used.iter().position(|u| *u == i).unwrap_or_else(|| {
                        used.push(i);
                        used.len() - 1
                    });
                    d.shape = Shape::Icon(Icon(at as u32));
                }
                d
            })
            .collect();
        let shapes = used
            .iter()
            .map(|i| {
                icons
                    .get(*i as usize)
                    .cloned()
                    .ok_or_else(|| format!("a filtered shape names icon {i}, which is missing"))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let baked = if shapes.is_empty() {
            None
        } else {
            Some(Icons::bake(&shapes)?)
        };
        for d in &mut draws {
            if let Some(g) = d.group
                && usize::from(g) >= self.groups.len()
            {
                d.group = None;
            }
        }
        let curve = ShadowCurve::none();
        let scene = FlatScene {
            layout: [pixels[0] as f32, pixels[1] as f32],
            clear: None,
            curve: &curve,
            light: Light::default(),
            draws: &draws,
            groups: &self.groups,
            text: &[],
            icons: None,
            sprites: None,
            environment: None,
            post: false,
            frame: 0,
            seed: 0,
        };
        let source = cpu::render(&scene, baked.as_ref(), pixels)?;
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        let source = source.iter().map(|p| p.map(byte)).collect();
        let steps = self
            .filter
            .plan(self.bounds, self.region, self.view, pixels)?;
        let out = filter::run(&steps, source, pixels);
        let mut rgba = Vec::with_capacity(out.len() * 4);
        for p in out {
            if p[3] == 0 {
                rgba.extend_from_slice(&[0; 4]);
                continue;
            }
            let a = p[3] as f32 / 255.0;
            let c = |v: u8| ((v as f32 / a + 0.5) as u32).min(255) as u8;
            rgba.extend_from_slice(&[c(p[0]), c(p[1]), c(p[2]), p[3]]);
        }
        Ok(Painted { size: pixels, rgba })
    }
}

#[derive(Clone, Debug)]
struct Tag {
    name: String,
    attributes: HashMap<String, String>,
    closing: bool,
}

impl Tag {
    fn get(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).map(String::as_str)
    }

    fn need(&self, key: &str) -> Result<&str, String> {
        self.get(key)
            .ok_or_else(|| format!("<{}> has no {key}", self.name))
    }
}

fn tags(source: &str) -> Result<Vec<Tag>, String> {
    let mut out = Vec::new();
    let mut rest = source;
    while let Some(open) = rest.find('<') {
        let close = rest[open..].find('>').ok_or("a tag is not closed")? + open;
        let body = &rest[open + 1..close];
        rest = &rest[close + 1..];
        if body.starts_with('?') || body.starts_with('!') {
            continue;
        }
        let closing = body.starts_with('/');
        let body = body.trim_start_matches('/').trim_end_matches('/');
        let name_end = body.find(char::is_whitespace).unwrap_or(body.len());
        let name = body[..name_end].to_string();
        let mut attributes = HashMap::new();
        let mut text = &body[name_end..];
        while let Some(eq) = text.find("=\"") {
            let key = text[..eq].trim().to_string();
            let value_end = text[eq + 2..]
                .find('"')
                .ok_or("an attribute is not closed")?
                + eq
                + 2;
            attributes.insert(key, text[eq + 2..value_end].to_string());
            text = &text[value_end + 1..];
        }
        out.push(Tag {
            name,
            attributes,
            closing,
        });
    }
    Ok(out)
}

fn number(value: &str) -> Result<f32, String> {
    value
        .trim()
        .trim_end_matches("px")
        .parse()
        .map_err(|_| format!("{value:?} is not a number"))
}

fn fraction(value: &str) -> Result<f32, String> {
    match value.trim().strip_suffix('%') {
        Some(percent) => Ok(number(percent)? / 100.0),
        None => number(value),
    }
}

fn styled<'a>(tag: &'a Tag, key: &str) -> Option<&'a str> {
    tag.get(key).or_else(|| {
        tag.get("style")?.split(';').find_map(|rule| {
            let (name, value) = rule.split_once(':')?;
            (name.trim() == key).then(|| value.trim())
        })
    })
}

fn numbers(value: &str) -> Result<Vec<f32>, String> {
    value
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(number)
        .collect()
}

fn rgb(value: &str) -> Option<Result<Srgba, String>> {
    let inner = value
        .trim()
        .strip_prefix("rgb(")
        .or_else(|| value.trim().strip_prefix("rgba("))?
        .strip_suffix(')')?;
    let parts: Vec<&str> = inner
        .split([',', ' ', '/'])
        .filter(|p| !p.is_empty())
        .collect();
    if !(3..=4).contains(&parts.len()) {
        return Some(Err(format!("{value:?} is not an rgb() colour")));
    }
    let channel = |part: &str| -> Result<f32, String> {
        match part.strip_suffix('%') {
            Some(percent) => Ok(number(percent)? / 100.0),
            None => Ok(number(part)? / 255.0),
        }
    };
    let made = (|| -> Result<Srgba, String> {
        let alpha = match parts.get(3) {
            Some(a) => fraction(a)?,
            None => 1.0,
        };
        Ok(Srgba([
            channel(parts[0])?.clamp(0.0, 1.0),
            channel(parts[1])?.clamp(0.0, 1.0),
            channel(parts[2])?.clamp(0.0, 1.0),
            alpha.clamp(0.0, 1.0),
        ]))
    })();
    Some(made)
}

fn named(value: &str) -> Option<Srgba> {
    Some(match value.trim() {
        "black" => Srgba::hex(0x000000),
        "white" => Srgba::hex(0xffffff),
        "red" => Srgba::hex(0xff0000),
        "green" => Srgba::hex(0x008000),
        "blue" => Srgba::hex(0x0000ff),
        "transparent" => Srgba([0.0; 4]),
        _ => return None,
    })
}

fn colour(value: &str) -> Result<Srgba, String> {
    if let Some(made) = rgb(value) {
        return made;
    }
    if let Some(c) = named(value) {
        return Ok(c);
    }
    let hex = value
        .strip_prefix('#')
        .ok_or_else(|| format!("{value:?} is not a #rgb colour"))?;
    let hex = if hex.len() == 3 {
        hex.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        hex.to_string()
    };
    u32::from_str_radix(&hex, 16)
        .map(Srgba::hex)
        .map_err(|_| format!("{value:?} is not a #rgb colour"))
}

fn transform(value: &str) -> Result<Matrix, String> {
    let mut m = IDENTITY;
    let mut rest = value;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim();
        let close = rest.find(')').ok_or("a transform is not closed")?;
        let args = numbers(&rest[open + 1..close])?;
        rest = &rest[close + 1..];
        let first = *args.first().ok_or("a transform has no arguments")?;
        let step = match name {
            "translate" => translation(first, args.get(1).copied().unwrap_or(0.0)),
            "scale" => scaling(first, args.get(1).copied().unwrap_or(first)),
            "rotate" => {
                let turn = rotation(first.to_radians());
                if args.len() == 3 {
                    multiply(
                        translation(args[1], args[2]),
                        multiply(turn, translation(-args[1], -args[2])),
                    )
                } else {
                    turn
                }
            }
            other => return Err(format!("the {other} transform is unsupported")),
        };
        m = multiply(m, step);
    }
    Ok(m)
}

fn arc_points(
    from: [f32; 2],
    radius: f32,
    large: bool,
    sweep: bool,
    to: [f32; 2],
) -> Vec<[f32; 2]> {
    let mid = [(from[0] - to[0]) * 0.5, (from[1] - to[1]) * 0.5];
    let d2 = mid[0] * mid[0] + mid[1] * mid[1];
    let r = radius.max(d2.sqrt());
    let factor =
        ((r * r - d2).max(0.0) / d2.max(1e-12)).sqrt() * if large == sweep { -1.0 } else { 1.0 };
    let centre = [
        factor * mid[1] + (from[0] + to[0]) * 0.5,
        -factor * mid[0] + (from[1] + to[1]) * 0.5,
    ];
    let start = (from[1] - centre[1]).atan2(from[0] - centre[0]);
    let mut end = (to[1] - centre[1]).atan2(to[0] - centre[0]);
    if sweep && end < start {
        end += std::f32::consts::TAU;
    }
    if !sweep && end > start {
        end -= std::f32::consts::TAU;
    }
    (1..=24)
        .map(|i| {
            let a = start + (end - start) * i as f32 / 24.0;
            [centre[0] + r * a.cos(), centre[1] + r * a.sin()]
        })
        .collect()
}

fn cubic(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2]) -> Vec<[f32; 2]> {
    let second = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| {
        (a[0] - 2.0 * b[0] + c[0]).hypot(a[1] - 2.0 * b[1] + c[1])
    };
    let bend = second(p0, p1, p2).max(second(p1, p2, p3));
    let (mut x0, mut y0, mut x1, mut y1) = (p0[0], p0[1], p0[0], p0[1]);
    for p in [p1, p2, p3] {
        (x0, y0, x1, y1) = (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1]));
    }
    let tolerance = (x1 - x0).hypot(y1 - y0) * CURVE_TOLERANCE;
    let steps = if tolerance > 0.0 {
        (0.75 * bend / tolerance)
            .sqrt()
            .ceil()
            .clamp(1.0, CURVE_STEPS) as usize
    } else {
        1
    };
    (1..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            [
                a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0],
                a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1],
            ]
        })
        .collect()
}

const CURVE_TOLERANCE: f32 = 0.002;
const CURVE_STEPS: f32 = 64.0;

#[cfg(test)]
fn path(d: &str) -> Result<Vec<Vec<[f32; 2]>>, String> {
    path_closed(d).map(|(lines, _)| lines)
}

type Outline = (Vec<Vec<[f32; 2]>>, Vec<bool>);

fn path_closed(d: &str) -> Result<Outline, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut arc = false;
    let mut count = 0usize;
    let flush = |tokens: &mut Vec<String>, current: &mut String, count: &mut usize| {
        if !current.is_empty() {
            tokens.push(std::mem::take(current));
            *count += 1;
        }
    };
    for c in d.chars() {
        if arc && current.is_empty() && matches!(count % 7, 3 | 4) && matches!(c, '0' | '1') {
            tokens.push(c.to_string());
            count += 1;
        } else if c.is_ascii_alphabetic() && c != 'e' {
            flush(&mut tokens, &mut current, &mut count);
            tokens.push(c.to_string());
            arc = c.eq_ignore_ascii_case(&'a');
            count = 0;
        } else if c == '-' && !current.is_empty() && !current.ends_with('e') {
            flush(&mut tokens, &mut current, &mut count);
            current.push('-');
        } else if c == '.' && current.contains('.') {
            flush(&mut tokens, &mut current, &mut count);
            current.push('.');
        } else if c == ',' || c.is_whitespace() {
            flush(&mut tokens, &mut current, &mut count);
        } else {
            current.push(c);
        }
    }
    flush(&mut tokens, &mut current, &mut count);
    let mut subpaths: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut closed: Vec<bool> = Vec::new();
    let mut at = [0.0f32, 0.0];
    let mut command = 'M';
    let mut index = 0;
    let mut cubic_control: Option<[f32; 2]> = None;
    let mut quad_control: Option<[f32; 2]> = None;
    let take = |count: usize, index: &mut usize| -> Result<Vec<f32>, String> {
        let values = tokens
            .get(*index..*index + count)
            .ok_or("a path command is missing its numbers")?
            .iter()
            .map(|token| number(token))
            .collect::<Result<Vec<_>, _>>()?;
        *index += count;
        Ok(values)
    };
    while index < tokens.len() {
        if tokens[index].chars().all(|c| c.is_ascii_alphabetic()) {
            command = tokens[index].chars().next().unwrap_or('M');
            index += 1;
            if command == 'Z' || command == 'z' {
                let start = subpaths.last().ok_or("Z before a move")?[0];
                closed.resize(subpaths.len(), false);
                if let Some(last) = closed.last_mut() {
                    *last = true;
                }
                at = start;
                cubic_control = None;
                quad_control = None;
                continue;
            }
        }
        let relative = command.is_ascii_lowercase();
        let base = if relative { at } else { [0.0, 0.0] };
        let point = |v: &[f32], i: usize| [base[0] + v[i], base[1] + v[i + 1]];
        let reflect = |control: Option<[f32; 2]>| {
            control.map_or(at, |c| [2.0 * at[0] - c[0], 2.0 * at[1] - c[1]])
        };
        let (last_cubic, last_quad) = (cubic_control.take(), quad_control.take());
        match command.to_ascii_uppercase() {
            'M' => {
                let v = take(2, &mut index)?;
                at = [base[0] + v[0], base[1] + v[1]];
                subpaths.push(vec![at]);
                command = if relative { 'l' } else { 'L' };
                continue;
            }
            'L' => {
                let v = take(2, &mut index)?;
                at = [base[0] + v[0], base[1] + v[1]];
            }
            'H' => at[0] = base[0] + take(1, &mut index)?[0],
            'V' => at[1] = base[1] + take(1, &mut index)?[0],
            'C' | 'S' => {
                let smooth = command.eq_ignore_ascii_case(&'S');
                let v = take(if smooth { 4 } else { 6 }, &mut index)?;
                let (c1, rest) = if smooth {
                    (reflect(last_cubic), &v[..])
                } else {
                    (point(&v, 0), &v[2..])
                };
                let (c2, to) = (point(rest, 0), point(rest, 2));
                let line = subpaths.last_mut().ok_or("a curve before a move")?;
                let points = cubic(at, c1, c2, to);
                line.extend_from_slice(&points[..points.len() - 1]);
                cubic_control = Some(c2);
                at = to;
            }
            'Q' | 'T' => {
                let smooth = command.eq_ignore_ascii_case(&'T');
                let v = take(if smooth { 2 } else { 4 }, &mut index)?;
                let (control, to) = if smooth {
                    (reflect(last_quad), point(&v, 0))
                } else {
                    (point(&v, 0), point(&v, 2))
                };
                let c1 = [
                    at[0] + (control[0] - at[0]) * 2.0 / 3.0,
                    at[1] + (control[1] - at[1]) * 2.0 / 3.0,
                ];
                let c2 = [
                    to[0] + (control[0] - to[0]) * 2.0 / 3.0,
                    to[1] + (control[1] - to[1]) * 2.0 / 3.0,
                ];
                let line = subpaths.last_mut().ok_or("a curve before a move")?;
                let points = cubic(at, c1, c2, to);
                line.extend_from_slice(&points[..points.len() - 1]);
                quad_control = Some(control);
                at = to;
            }
            'A' => {
                let v = take(7, &mut index)?;
                let to = [base[0] + v[5], base[1] + v[6]];
                let points = arc_points(at, v[0], v[3] != 0.0, v[4] != 0.0, to);
                let line = subpaths.last_mut().ok_or("an arc before a move")?;
                line.extend_from_slice(&points[..points.len() - 1]);
                at = to;
            }
            other => return Err(format!("the {other} path command is unsupported")),
        }
        subpaths.last_mut().ok_or("a line before a move")?.push(at);
    }
    closed.resize(subpaths.len(), false);
    Ok((subpaths, closed))
}

fn points_of(value: &str) -> Result<Vec<[f32; 2]>, String> {
    let n = numbers(value)?;
    if n.len() < 4 || n.len() % 2 != 0 {
        return Err("a points list needs pairs, at least two".into());
    }
    Ok(n.chunks_exact(2).map(|p| [p[0], p[1]]).collect())
}

fn ellipse_points(c: [f32; 2], r: [f32; 2]) -> Vec<[f32; 2]> {
    (0..ELLIPSE_STEPS)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / ELLIPSE_STEPS as f32;
            [c[0] + r[0] * a.cos(), c[1] + r[1] * a.sin()]
        })
        .collect()
}

fn rect_points(x: f32, y: f32, w: f32, h: f32, r: f32) -> Vec<[f32; 2]> {
    let r = r.clamp(0.0, w.min(h) * 0.5);
    if r <= 0.0 {
        return vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
    }
    let mut out = Vec::new();
    let corners = [
        ([x + w - r, y + r], -0.25),
        ([x + w - r, y + h - r], 0.0),
        ([x + r, y + h - r], 0.25),
        ([x + r, y + r], 0.5),
    ];
    for (c, from) in corners {
        for i in 0..=8 {
            let a = std::f32::consts::TAU * (from + 0.25 * i as f32 / 8.0);
            out.push([c[0] + r * a.cos(), c[1] + r * a.sin()]);
        }
    }
    out
}

fn shape_of(element: &Tag) -> Result<Option<Outline>, String> {
    let get = |key: &str| element.get(key).map_or(Ok(0.0), number);
    Ok(Some(match element.name.as_str() {
        "path" => path_closed(element.need("d")?)?,
        "polyline" => (vec![points_of(element.need("points")?)?], vec![false]),
        "polygon" => (vec![points_of(element.need("points")?)?], vec![true]),
        "line" => (
            vec![vec![[get("x1")?, get("y1")?], [get("x2")?, get("y2")?]]],
            vec![false],
        ),
        "ellipse" => (
            vec![ellipse_points(
                [get("cx")?, get("cy")?],
                [number(element.need("rx")?)?, number(element.need("ry")?)?],
            )],
            vec![true],
        ),
        "circle" => {
            let r = number(element.need("r")?)?;
            (
                vec![ellipse_points([get("cx")?, get("cy")?], [r, r])],
                vec![true],
            )
        }
        "rect" => {
            let r = element
                .get("rx")
                .or(element.get("ry"))
                .map_or(Ok(0.0), number)?;
            (
                vec![rect_points(
                    get("x")?,
                    get("y")?,
                    number(element.need("width")?)?,
                    number(element.need("height")?)?,
                    r,
                )],
                vec![true],
            )
        }
        _ => return Ok(None),
    }))
}

fn dashed(line: &[[f32; 2]], pattern: &[f32]) -> Vec<Vec<[f32; 2]>> {
    let pattern: Vec<f32> = if pattern.len() % 2 == 1 {
        pattern.iter().chain(pattern).copied().collect()
    } else {
        pattern.to_vec()
    };
    let total: f32 = pattern.iter().map(|v| v.max(0.0)).sum();
    if total <= 0.0 || line.len() < 2 {
        return vec![line.to_vec()];
    }
    let distance =
        |a: [f32; 2], b: [f32; 2]| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let mut out = Vec::new();
    let mut current = vec![line[0]];
    let mut on = true;
    let mut index = 0;
    let mut left = pattern[0].max(0.0);
    for pair in line.windows(2) {
        let (mut a, b) = (pair[0], pair[1]);
        loop {
            let span = distance(a, b);
            if span <= left {
                left -= span;
                if on {
                    current.push(b);
                }
                break;
            }
            let t = left / span;
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            if on {
                current.push(p);
                if current.len() > 1 {
                    out.push(std::mem::take(&mut current));
                }
                current.clear();
            } else {
                current = vec![p];
            }
            on = !on;
            index = (index + 1) % pattern.len();
            left = pattern[index].max(0.0);
            a = p;
        }
    }
    if on && current.len() > 1 {
        out.push(current);
    }
    out
}

struct Pen {
    width: f32,
    join: String,
    cap: String,
    miter: f32,
    dash: Option<Vec<f32>>,
}

fn stroked(lines: &[Vec<[f32; 2]>], closed: &[bool], pen: &Pen) -> Vec<Vec<[f32; 2]>> {
    let cap = match pen.cap.as_str() {
        "round" => LineCap::Round(ARC_STEP),
        "square" => LineCap::Square,
        _ => LineCap::Butt,
    };
    let join = match pen.join.as_str() {
        "round" => LineJoin::Round(ARC_STEP),
        "bevel" => LineJoin::Bevel,
        _ => LineJoin::Miter(2.0 * (1.0 / pen.miter.max(1.0)).asin()),
    };
    let style = StrokeStyle::new(pen.width)
        .start_cap(cap.clone())
        .end_cap(cap)
        .line_join(join);
    let mut open: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut shut: Vec<Vec<[f32; 2]>> = Vec::new();
    for (line, &closed) in lines.iter().zip(closed) {
        if line.len() < 2 {
            continue;
        }
        match (&pen.dash, closed) {
            (Some(pattern), _) => {
                let mut whole = line.clone();
                if closed {
                    whole.push(line[0]);
                }
                open.extend(dashed(&whole, pattern));
            }
            (None, true) => shut.push(line.clone()),
            (None, false) => open.push(line.clone()),
        }
    }
    let mut contours: Vec<Vec<[f32; 2]>> = Vec::new();
    for (set, closed) in [(&open, false), (&shut, true)] {
        if set.is_empty() {
            continue;
        }
        for shape in set.as_slice().stroke(style.clone(), closed) {
            contours.extend(shape);
        }
    }
    contours
        .as_slice()
        .simplify_shape(FillRule::NonZero)
        .into_iter()
        .flatten()
        .collect()
}

fn filled(lines: Vec<Vec<[f32; 2]>>, rule: Option<&str>) -> Vec<Vec<[f32; 2]>> {
    let rule = match rule {
        Some("evenodd") => FillRule::EvenOdd,
        _ => FillRule::NonZero,
    };
    let lines: Vec<Vec<[f32; 2]>> = lines.into_iter().filter(|l| l.len() > 2).collect();
    lines
        .as_slice()
        .simplify_shape(rule)
        .into_iter()
        .flatten()
        .collect()
}

#[derive(Clone)]
struct Context {
    transform: Matrix,
    filter: Option<String>,
    group: Option<u16>,
    first: bool,
    rotated: bool,
    paint: Paint,
    filtered: Option<Filtering>,
}

#[derive(Clone)]
struct Filtering {
    start: usize,
    groups: usize,
    filter: filter::Filter,
    transform: Matrix,
    opacity: f32,
    group: Option<u16>,
}

const NESTED: &str = "a filter inside a filtered group is unsupported";

#[derive(Clone, Default)]
struct Paint {
    fill: Option<String>,
    stroke: Option<String>,
    stroke_width: Option<String>,
    join: Option<String>,
    cap: Option<String>,
    miter: Option<String>,
    fill_opacity: Option<String>,
    stroke_opacity: Option<String>,
    fill_rule: Option<String>,
}

impl Paint {
    fn under(&self, tag: &Tag) -> Self {
        let own = |key: &str, inherited: &Option<String>| {
            styled(tag, key)
                .map(str::to_string)
                .or_else(|| inherited.clone())
        };
        Self {
            fill: own("fill", &self.fill),
            stroke: own("stroke", &self.stroke),
            stroke_width: own("stroke-width", &self.stroke_width),
            join: own("stroke-linejoin", &self.join),
            cap: own("stroke-linecap", &self.cap),
            miter: own("stroke-miterlimit", &self.miter),
            fill_opacity: own("fill-opacity", &self.fill_opacity),
            stroke_opacity: own("stroke-opacity", &self.stroke_opacity),
            fill_rule: own("fill-rule", &self.fill_rule),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Linear,
    Radial,
}

const LINEAR_KEYS: [&str; 4] = ["x1", "y1", "x2", "y2"];
const RADIAL_KEYS: [&str; 3] = ["cx", "cy", "r"];

#[derive(Clone, Debug)]
struct LinearGradient {
    kind: Kind,
    units: Option<bool>,
    keys: HashMap<String, String>,
    stops: Vec<(f32, Srgba)>,
    turn: Option<Matrix>,
    href: Option<String>,
}

impl LinearGradient {
    fn of(tag: &Tag, kind: Kind) -> Result<Self, String> {
        let names: &[&str] = match kind {
            Kind::Linear => &LINEAR_KEYS,
            Kind::Radial => &RADIAL_KEYS,
        };
        let keys = names
            .iter()
            .filter_map(|k| tag.get(k).map(|v| (k.to_string(), v.to_string())))
            .collect();
        Ok(Self {
            kind,
            units: tag.get("gradientUnits").map(|u| u == "userSpaceOnUse"),
            keys,
            stops: Vec::new(),
            turn: tag.get("gradientTransform").map(transform).transpose()?,
            href: tag
                .get("href")
                .or(tag.get("xlink:href"))
                .map(|h| h.trim_start_matches('#').to_string()),
        })
    }

    fn user_space(&self) -> bool {
        self.units.unwrap_or(false)
    }

    fn end(&self, index: usize) -> Option<&str> {
        self.keys.get(LINEAR_KEYS[index]).map(String::as_str)
    }

    fn vertical_box(&self) -> bool {
        let at = |index: usize, value: f32| {
            self.end(index)
                .map_or(
                    Ok(index == 2 && value == 1.0 || index != 2 && value == 0.0),
                    |end| fraction(end).map(|v| v == value),
                )
                .unwrap_or(false)
        };
        self.kind == Kind::Linear
            && self.turn.is_none()
            && !self.user_space()
            && self.stops.len() == 2
            && self.stops[0].0 == 0.0
            && self.stops[1].0 == 1.0
            && at(0, 0.0)
            && at(1, 0.0)
            && at(2, 0.0)
            && at(3, 1.0)
    }

    fn fill(&self, bounds: [f32; 4], origin: [f32; 2], view: [f32; 2]) -> Result<Fill, String> {
        if self.vertical_box() {
            return Ok(Fill::Vertical(self.stops[0].1, self.stops[1].1));
        }
        let user = self.user_space();
        let value = |key: &str, default: f32, side: f32| -> Result<f32, String> {
            Ok(match (user, self.keys.get(key).map(String::as_str)) {
                (true, None) => default * side,
                (false, None) => default,
                (true, Some(v)) if v.trim().ends_with('%') => fraction(v)? * side,
                (_, Some(v)) => fraction(v)?,
            })
        };
        let turn = self.turn.unwrap_or(IDENTITY);
        let turned = |x: f32, y: f32| {
            [
                turn[0][0] * x + turn[1][0] * y + turn[3][0],
                turn[0][1] * x + turn[1][1] * y + turn[3][1],
            ]
        };
        let point = |x: f32, y: f32| {
            let [x, y] = turned(x, y);
            if user {
                [x - origin[0], y - origin[1]]
            } else {
                [
                    bounds[0] + x * (bounds[2] - bounds[0]) - origin[0],
                    bounds[1] + y * (bounds[3] - bounds[1]) - origin[1],
                ]
            }
        };
        let stops: Vec<(f32, [f32; 4])> = self.stops.iter().map(|(at, c)| (*at, c.0)).collect();
        match self.kind {
            Kind::Linear => {
                let defaults = [0.0, 0.0, 1.0, 0.0];
                let mut ends = [0.0f32; 4];
                for (index, end) in ends.iter_mut().enumerate() {
                    *end = value(LINEAR_KEYS[index], defaults[index], view[index % 2])?;
                }
                Gradient::linear(point(ends[0], ends[1]), point(ends[2], ends[3]), &stops)
                    .map(Fill::Gradient)
            }
            Kind::Radial => {
                let diagonal = ((view[0] * view[0] + view[1] * view[1]) * 0.5).sqrt();
                let cx = value("cx", 0.5, view[0])?;
                let cy = value("cy", 0.5, view[1])?;
                let r = value("r", 0.5, diagonal)?;
                let column = |c: usize| (turn[c][0] * turn[c][0] + turn[c][1] * turn[c][1]).sqrt();
                let mut radius = [r * column(0), r * column(1)];
                if !user {
                    radius[0] *= bounds[2] - bounds[0];
                    radius[1] *= bounds[3] - bounds[1];
                }
                Gradient::radial(point(cx, cy), radius, &stops).map(Fill::Gradient)
            }
        }
    }

    fn inherit(&mut self, from: &LinearGradient) {
        if self.stops.is_empty() {
            self.stops = from.stops.clone();
        }
        if self.units.is_none() {
            self.units = from.units;
        }
        if self.turn.is_none() {
            self.turn = from.turn;
        }
        if self.kind == from.kind {
            for (k, v) in &from.keys {
                self.keys.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    }
}

fn resolve(gradients: &mut HashMap<String, LinearGradient>) {
    for _ in 0..4 {
        let names: Vec<String> = gradients.keys().cloned().collect();
        for name in names {
            let Some(href) = gradients[&name].href.clone() else {
                continue;
            };
            let Some(from) = gradients.get(&href).cloned() else {
                continue;
            };
            if let Some(g) = gradients.get_mut(&name) {
                g.inherit(&from);
            }
        }
    }
}

enum Ink<'a> {
    Solid(Srgba),
    Gradient(&'a LinearGradient),
}

fn bounds_of(lines: &[Vec<[f32; 2]>]) -> [f32; 4] {
    lines.iter().flatten().fold(
        [f32::MAX, f32::MAX, f32::MIN, f32::MIN],
        |[x0, y0, x1, y1], p| [x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1])],
    )
}

fn root(tag: &Tag) -> Result<([f32; 2], Matrix), String> {
    let view = tag.get("viewBox").map(numbers).transpose()?;
    let view = match view {
        Some(v) if v.len() == 4 && v[2] > 0.0 && v[3] > 0.0 => Some([v[0], v[1], v[2], v[3]]),
        Some(_) => return Err("the viewBox needs four numbers and a positive size".into()),
        None => None,
    };
    let given = |key: &str| {
        tag.get(key)
            .filter(|v| !v.trim().ends_with('%'))
            .map(number)
    };
    let (width, height) = (given("width").transpose()?, given("height").transpose()?);
    let Some([x, y, w, h]) = view else {
        return match (width, height) {
            (Some(width), Some(height)) => Ok(([width, height], IDENTITY)),
            _ => Err("<svg> has no width and height or viewBox".into()),
        };
    };
    let size = match (width, height) {
        (Some(width), Some(height)) => [width, height],
        (Some(width), None) => [width, width * h / w],
        (None, Some(height)) => [height * w / h, height],
        (None, None) => [w, h],
    };
    Ok((
        size,
        multiply(scaling(size[0] / w, size[1] / h), translation(-x, -y)),
    ))
}

fn filter_of<'a>(filters: &'a [(&str, Filter)], value: &str) -> Result<&'a Filter, String> {
    let id = value
        .strip_prefix("url(#")
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| format!("{value:?} is not a filter reference"))?;
    filters
        .iter()
        .find(|(name, _)| *name == id)
        .map(|(_, filter)| filter)
        .ok_or_else(|| format!("the filter {id} has no elevation"))
}

pub fn read(source: &str, filters: &[(&str, Filter)]) -> Result<Board, String> {
    read_in(source, filters, None)
}

pub fn read_mark(source: &str, current: Srgba) -> Result<Board, String> {
    read_in(source, &[], Some(current))
}

fn read_in(
    source: &str,
    filters: &[(&str, Filter)],
    current: Option<Srgba>,
) -> Result<Board, String> {
    let mark = current.is_some();
    let all = tags(source)?;
    let mut gradients: HashMap<String, LinearGradient> = HashMap::new();
    let mut symbols: HashMap<String, Tag> = HashMap::new();
    let mut size = [0.0, 0.0];
    let mut view = IDENTITY;
    let mut body = Vec::new();
    let mut in_defs = false;
    let mut in_text = 0usize;
    let mut gradient: Option<String> = None;
    let mut symbol: Option<String> = None;
    let mut documented: HashMap<String, Result<filter::Filter, String>> = HashMap::new();
    let mut building: Option<(String, filter::Builder)> = None;
    for tag in all {
        match (tag.name.as_str(), tag.closing) {
            ("filter", _) => {
                if let Some((id, b)) = building.take() {
                    documented.insert(id, b.finish());
                }
                if !tag.closing {
                    building = Some((tag.need("id")?.to_string(), filter::Builder::new(&tag)));
                }
            }
            (name, _) if name.starts_with("fe") && building.is_some() => {
                if let Some((_, b)) = building.as_mut() {
                    b.add(&tag);
                }
            }
            ("svg", false) => (size, view) = root(&tag)?,
            ("svg", true) => {}
            ("defs", closing) => in_defs = !closing,
            ("text", false) => in_text += 1,
            ("text", true) => in_text = in_text.saturating_sub(1),
            _ if in_text > 0 => {}
            ("linearGradient" | "radialGradient", false) => {
                let kind = if tag.name == "radialGradient" {
                    Kind::Radial
                } else {
                    Kind::Linear
                };
                let id = tag.need("id")?.to_string();
                gradients.insert(id.clone(), LinearGradient::of(&tag, kind)?);
                gradient = Some(id);
            }
            ("linearGradient" | "radialGradient", true) => gradient = None,
            ("stop", false) => {
                let id = gradient.as_ref().ok_or("a stop outside a gradient")?;
                let mut c = colour(styled(&tag, "stop-color").ok_or("<stop> has no stop-color")?)?;
                if let Some(alpha) = styled(&tag, "stop-opacity") {
                    c = c.alpha(number(alpha)?);
                }
                let offset = tag.get("offset").map_or(Ok(0.0), fraction)?;
                if let Some(gradient) = gradients.get_mut(id) {
                    gradient.stops.push((offset, c));
                }
            }
            ("stop", true) => {}
            ("g", false) if in_defs => symbol = tag.get("id").map(str::to_string),
            ("path", _) if in_defs => {
                if let Some(id) = symbol.clone() {
                    symbols.insert(id, tag.clone());
                }
            }
            _ if in_defs => {}
            _ => body.push(tag),
        }
    }
    if let Some((id, b)) = building.take() {
        documented.insert(id, b.finish());
    }
    resolve(&mut gradients);
    let reach_of_mark = MARK_REACH * size[1] / view[1][1].abs().max(f32::MIN_POSITIVE);
    let mut board = Board {
        size,
        ..Board::default()
    };
    let mut icon_keys: HashMap<(String, u32), usize> = HashMap::new();
    let mut level = 0.0f32;
    let mut stack = vec![Context {
        transform: view,
        filter: None,
        group: None,
        first: false,
        rotated: false,
        paint: Paint::default(),
        filtered: None,
    }];
    let documented_of = |value: &str| -> Option<Result<filter::Filter, String>> {
        let id = value.strip_prefix("url(#")?.strip_suffix(')')?;
        if filters.iter().any(|(name, _)| *name == id) {
            return None;
        }
        documented.get(id).cloned()
    };
    for tag in body {
        let top = stack.last().ok_or("a group closed twice")?.clone();
        let rotates = tag.get("transform").is_some_and(|t| t.contains("rotate"));
        let inside = stack.iter().any(|c| c.filtered.is_some());
        if tag.name == "g" {
            if tag.closing {
                if let Some(f) = stack.pop().and_then(|c| c.filtered) {
                    let mut source: Vec<Draw> = board.draws.drain(f.start..).collect();
                    for d in &mut source {
                        if d.group.is_some_and(|g| usize::from(g) < f.groups) {
                            d.group = None;
                        }
                    }
                    let raster = Raster::new(f.filter, source, f.transform, &board)?;
                    let d = raster.placeholder(f.opacity, f.group, level);
                    board.rasters.push(raster);
                    board.draws.push(d);
                }
                continue;
            }
            let own = tag.get("transform").map_or(Ok(IDENTITY), transform)?;
            let opacity = tag.get("opacity").map_or(Ok(1.0), number)?;
            let document = tag.get("filter").and_then(documented_of).transpose()?;
            if document.is_some() && inside {
                return Err(NESTED.into());
            }
            let group = if opacity < 1.0 && document.is_none() {
                board.groups.push(Group { opacity });
                Some(u16::try_from(board.groups.len() - 1).map_err(|_| "too many groups")?)
            } else {
                top.group
            };
            let placed = multiply(top.transform, own);
            let filtered = document.map(|filter| Filtering {
                start: board.draws.len(),
                groups: board.groups.len(),
                filter,
                transform: placed,
                opacity,
                group: top.group,
            });
            stack.push(Context {
                transform: placed,
                filter: match filtered {
                    Some(_) => top.filter.clone(),
                    None => tag.get("filter").map(str::to_string).or(top.filter.clone()),
                },
                group,
                first: tag.get("filter").is_some() && filtered.is_none(),
                rotated: top.rotated || rotates,
                paint: top.paint.under(&tag),
                filtered,
            });
            continue;
        }
        if tag.closing {
            continue;
        }
        let own = tag.get("transform").map_or(Ok(IDENTITY), transform)?;
        let element = if tag.name == "use" {
            let href = tag
                .get("href")
                .or(tag.get("xlink:href"))
                .ok_or("<use> has no href")?;
            symbols
                .get(href.trim_start_matches('#'))
                .ok_or_else(|| format!("{href} is not a defined path"))?
                .clone()
        } else {
            tag.clone()
        };
        let base = multiply(top.transform, own);
        let rotated = top.rotated || rotates;
        let inherited = top.paint.under(&tag);
        let inherited = if tag.name == "use" {
            inherited.under(&element)
        } else {
            inherited
        };
        let ink = |value: Option<&str>| -> Result<Option<Ink>, String> {
            let Some(value) = value.filter(|value| *value != "none") else {
                return Ok(None);
            };
            if let Some(id) = value.strip_prefix("url(#") {
                let gradient = gradients
                    .get(id.trim_end_matches(')'))
                    .filter(|gradient| !gradient.stops.is_empty())
                    .ok_or_else(|| format!("{value} is not a linear gradient with stops"))?;
                return Ok(Some(Ink::Gradient(gradient)));
            }
            if let (Some(c), "currentColor") = (current, value.trim()) {
                return Ok(Some(Ink::Solid(c)));
            }
            Ok(Some(Ink::Solid(colour(value)?)))
        };
        let fill = match (element.name.as_str(), inherited.fill.as_deref()) {
            (_, None) if mark => current.map(Ink::Solid),
            ("path", None) => None,
            (_, None) => Some(Ink::Solid(Srgba::hex(0))),
            (_, value) => ink(value)?,
        };
        let stroke_width = inherited.stroke_width.as_deref().map_or(Ok(1.0), number)?;
        let stroke_ink = ink(inherited.stroke.as_deref())?;
        let explicit = |value: &Option<String>| value.as_deref().is_some_and(|v| v != "round");
        let geometric = stroke_ink.is_some()
            && (mark
                || matches!(stroke_ink, Some(Ink::Gradient(_)))
                || explicit(&inherited.join)
                || explicit(&inherited.cap));
        let alpha_of = |value: &Option<String>| value.as_deref().map_or(Ok(1.0), number);
        let fill_alpha = if mark {
            alpha_of(&inherited.fill_opacity)?
        } else {
            1.0
        };
        let pen = Pen {
            width: stroke_width,
            join: inherited
                .join
                .clone()
                .unwrap_or_else(|| if mark { "miter" } else { "round" }.to_string()),
            cap: inherited
                .cap
                .clone()
                .unwrap_or_else(|| if mark { "butt" } else { "round" }.to_string()),
            miter: inherited.miter.as_deref().map_or(Ok(MITER_LIMIT), number)?,
            dash: element
                .get("stroke-dasharray")
                .filter(|d| *d != "none")
                .map(numbers)
                .transpose()?
                .filter(|d| !d.is_empty()),
        };
        let stroke_alpha = if geometric {
            alpha_of(&inherited.stroke_opacity)?
        } else {
            1.0
        };
        let stroke = match stroke_ink.as_ref().filter(|_| !geometric) {
            None => None,
            Some(Ink::Solid(c)) => {
                let mut c = *c;
                if let Some(alpha) = element.get("stroke-opacity") {
                    c = c.alpha(number(alpha)?);
                }
                Some(
                    match element.get("stroke-dasharray").map(numbers).transpose()? {
                        Some(d) if d.len() >= 2 => Stroke::dashed(c, stroke_width, d[0], d[1]),
                        _ => Stroke::solid(c, stroke_width),
                    },
                )
            }
            Some(_) => return Err("gradient strokes are unsupported".into()),
        };
        let opacity = element.get("opacity").map_or(Ok(1.0), number)?;
        let filter = tag.get("filter").map(str::to_string).or(top.filter.clone());
        let casts = tag.get("filter").is_some() || top.first;
        let box_of: ([f32; 4], [f32; 2]);
        let geometry = if geometric
            || matches!(
                element.name.as_str(),
                "polyline" | "polygon" | "line" | "ellipse"
            ) {
            shape_of(&element)?
        } else {
            None
        };
        let stroke_draw = match (&geometry, stroke_ink.as_ref().filter(|_| geometric)) {
            (Some((lines, closed)), Some(ink)) => {
                let polygons = stroked(lines, closed, &pen);
                if polygons.is_empty() {
                    None
                } else {
                    let bounds = bounds_of(lines);
                    let shape = IconShape {
                        polygons,
                        reach: if mark { reach_of_mark } else { 4.0 },
                        ..IconShape::default()
                    };
                    board.icons.push(shape);
                    let mut d = Draw::new(Shape::Icon(Icon((board.icons.len() - 1) as u32)))
                        .transform(base);
                    d.fill = match ink {
                        Ink::Solid(c) => Fill::Solid(c.alpha(c.0[3] * stroke_alpha)),
                        Ink::Gradient(g) => faded(g.fill(bounds, [0.0, 0.0], size)?, stroke_alpha),
                    };
                    Some(d)
                }
            }
            _ => None,
        };
        let path_like = matches!(
            element.name.as_str(),
            "path" | "polyline" | "polygon" | "line" | "ellipse"
        ) || (geometric
            && matches!(element.name.as_str(), "rect" | "circle")
            && fill.is_none());
        let mut draw = match element.name.as_str() {
            _ if path_like && geometric && fill.is_none() => {
                box_of = ([0.0; 4], [0.0; 2]);
                Draw::new(Shape::Circle { radius: 0.0 })
            }
            _ if path_like => {
                let (subpaths, _) = match &geometry {
                    Some(g) => g.clone(),
                    None => path_closed(element.need("d")?)?,
                };
                box_of = (bounds_of(&subpaths), [0.0, 0.0]);
                let reach = if mark {
                    reach_of_mark
                } else {
                    stroke.map_or(0.0, |s| s.width * 0.5) + 4.0
                };
                let shape = if fill.is_some() {
                    IconShape {
                        polygons: if mark {
                            filled(subpaths.clone(), inherited.fill_rule.as_deref())
                        } else {
                            subpaths.clone()
                        },
                        reach,
                        ..IconShape::default()
                    }
                } else {
                    IconShape {
                        lines: subpaths.clone(),
                        reach,
                        ..IconShape::default()
                    }
                };
                let key = (format!("{:?}{}", subpaths, fill.is_some()), reach.to_bits());
                let index = match icon_keys.get(&key) {
                    Some(index) => *index,
                    None => {
                        board.icons.push(shape);
                        icon_keys.insert(key, board.icons.len() - 1);
                        board.icons.len() - 1
                    }
                };
                Draw::new(Shape::Icon(Icon(index as u32))).transform(base)
            }
            "rect" => {
                let x = element.get("x").map_or(Ok(0.0), number)?;
                let y = element.get("y").map_or(Ok(0.0), number)?;
                let w = number(element.need("width")?)?;
                let h = number(element.need("height")?)?;
                let r = element.get("rx").map_or(Ok(0.0), number)?;
                box_of = ([x, y, x + w, y + h], [x + w * 0.5, y + h * 0.5]);
                Draw::new(Shape::Rect {
                    half: [w * 0.5, h * 0.5],
                    radii: [r; 4],
                })
                .transform(multiply(base, translation(x + w * 0.5, y + h * 0.5)))
            }
            "circle" => {
                let cx = number(element.need("cx")?)?;
                let cy = number(element.need("cy")?)?;
                let r = number(element.need("r")?)?;
                let placed = multiply(base, translation(cx, cy));
                box_of = ([cx - r, cy - r, cx + r, cy + r], [cx, cy]);
                match (&fill, stroke) {
                    (None, Some(s)) => {
                        let shape = match s.dash {
                            Some(dash) => Shape::Arc {
                                radius: r,
                                width: s.width,
                                start: 0.0,
                                sweep: dash.on / r,
                            },
                            None => Shape::Ring {
                                radius: r,
                                width: s.width,
                            },
                        };
                        Draw::new(shape).transform(placed).fill(s.colour)
                    }
                    _ => Draw::new(Shape::Circle { radius: r }).transform(placed),
                }
            }
            other => return Err(format!("<{other}> is unsupported")),
        };
        if matches!(draw.shape, Shape::Ring { .. } | Shape::Arc { .. }) {
            draw.opacity = opacity;
        } else {
            draw.fill = match fill {
                None => Fill::None,
                Some(Ink::Solid(c)) => Fill::Solid(c.alpha(c.0[3] * fill_alpha)),
                Some(Ink::Gradient(gradient)) => {
                    faded(gradient.fill(box_of.0, box_of.1, size)?, fill_alpha)
                }
            };
            draw.stroke = stroke;
            draw.opacity = opacity;
        }
        draw.group = top.group;
        draw.shadow = Shadow::None;
        if let Some(found) = tag.get("filter").and_then(documented_of).transpose()? {
            if inside {
                return Err(NESTED.into());
            }
            let clean = |mut d: Draw| {
                d.opacity = 1.0;
                d.group = None;
                d.elevation = 0.0;
                d.shadow = Shadow::None;
                d
            };
            let mut source = Vec::new();
            if !(geometric && fill.is_none()) {
                source.push(clean(draw));
            }
            source.extend(stroke_draw.map(clean));
            let raster = Raster::new(found, source, base, &board)?;
            let d = raster.placeholder(opacity, top.group, level);
            board.rasters.push(raster);
            board.draws.push(d);
            if let Some(context) = stack.last_mut() {
                context.first = false;
            }
            continue;
        }
        if let Some(filter) = filter {
            match *filter_of(filters, &filter)? {
                Filter::Elevation(elevation) => {
                    level = level.max(elevation);
                    if casts {
                        draw.shadow = if rotated {
                            Shadow::Turned
                        } else {
                            Shadow::Cast
                        };
                    }
                }
                Filter::Glow { sigma } => {
                    if casts {
                        let Fill::Solid(c) = draw.fill else {
                            return Err("a glow needs a solid fill".into());
                        };
                        draw.fill = Fill::None;
                        draw.shadow = Shadow::Glow(Glow {
                            colour: c.alpha(c.0[3] * draw.opacity),
                            sigma,
                        });
                        draw.opacity = 1.0;
                    }
                }
            }
        }
        draw.elevation = level;
        if let Some(context) = stack.last_mut() {
            context.first = false;
        }
        let shown = !(geometric && fill.is_none());
        if shown {
            board.draws.push(draw);
        }
        if let Some(mut d) = stroke_draw {
            d.group = top.group;
            d.shadow = Shadow::None;
            d.opacity = opacity;
            d.elevation = level;
            board.draws.push(d);
        }
    }
    Ok(board)
}

fn faded(fill: Fill, alpha: f32) -> Fill {
    if alpha >= 1.0 {
        return fill;
    }
    match fill {
        Fill::Solid(c) => Fill::Solid(c.alpha(c.0[3] * alpha)),
        Fill::Vertical(a, b) => Fill::Vertical(a.alpha(a.0[3] * alpha), b.alpha(b.0[3] * alpha)),
        Fill::Gradient(g) => {
            let stops: Vec<(f32, [f32; 4])> = g
                .stops()
                .map(|(at, c)| (at, [c[0], c[1], c[2], c[3] * alpha]))
                .collect();
            Gradient::new(g.shape, &stops)
                .map(|faded| faded.space(g.space))
                .map_or(Fill::Gradient(g), Fill::Gradient)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flat::icons::segment;

    const FILTERS: [(&str, Filter); 2] = [
        ("lift1", Filter::Elevation(1.0)),
        ("halo", Filter::Glow { sigma: 8.0 }),
    ];

    #[test]
    fn paths_read_relative_moves_lines_and_arcs() {
        let lines = path("M10 20h5v-5l-5 5ZM0 0a14 14 0 0 1 28 0").unwrap();
        assert_eq!(
            lines[0],
            vec![[10.0, 20.0], [15.0, 20.0], [15.0, 15.0], [10.0, 20.0]]
        );
        let arc = &lines[1];
        assert_eq!(arc[0], [0.0, 0.0]);
        let end = arc.last().unwrap();
        assert!((end[0] - 28.0).abs() < 1e-4 && end[1].abs() < 1e-4);
        assert!(
            arc.iter()
                .all(|p| ((p[0] - 14.0).hypot(p[1]) - 14.0).abs() < 1e-3)
        );
        assert!(arc.iter().any(|p| p[1] < -13.0));
    }

    #[test]
    fn compact_arc_flags_read_as_single_digits() {
        let compact = path("M0 0a14 14 0 0128 0a.5.5 0 01.2.3").unwrap();
        let spaced = path("M0 0a14 14 0 0 1 28 0a.5 .5 0 0 1 .2 .3").unwrap();
        assert_eq!(compact, spaced);
    }

    #[test]
    fn curves_flatten_onto_their_beziers() {
        let lines = path("M0 0C0 10 10 20 20 20S40 0 40 0M0 0Q10 20 20 0T40 0").unwrap();
        let cubic = &lines[0];
        assert!(cubic.len() > 8);
        assert_eq!(cubic[0], [0.0, 0.0]);
        assert_eq!(*cubic.last().unwrap(), [40.0, 0.0]);
        assert!(cubic.contains(&[20.0, 20.0]));
        let near = |line: &[[f32; 2]], p: [f32; 2]| {
            line.windows(2)
                .any(|pair| segment(p, pair[0], pair[1]) < 0.1)
        };
        assert!(near(cubic, [6.25, 13.75]));
        let quad = &lines[1];
        assert_eq!(*quad.last().unwrap(), [40.0, 0.0]);
        assert!(near(quad, [10.0, 10.0]));
        assert!(near(quad, [30.0, -10.0]));
        assert_eq!(path("M0 0L.5.5").unwrap()[0], vec![[0.0, 0.0], [0.5, 0.5]]);
    }

    #[test]
    fn a_view_box_root_with_a_user_space_gradient_paints_across_its_paths() {
        let source = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -2 200 60">
  <defs>
    <linearGradient id="g" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="200" y2="0"><stop offset="0" stop-color="#ff0000"/><stop offset="50%" stop-color="#00ff00"/><stop offset="1" style="stop-color:#0000ff"/></linearGradient>
  </defs>
  <g fill="url(#g)" fill-rule="evenodd">
    <path d="M 0 0 L 40 0 C 50 0 60 10 60 20 C 60 30 50 40 40 40 L 0 40 Z M 10 10 L 30 10 L 30 30 L 10 30 Z"/>
    <path d="M 140 0 Q 200 0 200 20 T 140 40 Z"/>
  </g>
  <rect x="0" y="50" width="200" height="4" fill="url(#g)"/>
</svg>"##;
        let board = read(source, &FILTERS).unwrap();
        assert_eq!(board.size, [200.0, 60.0]);
        assert_eq!(board.draws.len(), 3);
        assert!(
            board
                .draws
                .iter()
                .all(|draw| matches!(draw.fill, Fill::Gradient(_)))
        );
        let icons = super::super::Icons::bake(&board.icons).unwrap();
        let curve = super::super::ShadowCurve::none();
        let scene = super::super::FlatScene {
            layout: board.size,
            clear: Some(Srgba::hex(0)),
            curve: &curve,
            light: super::super::Light::default(),
            draws: &board.draws,
            groups: &board.groups,
            text: &[],
            icons: None,
            sprites: None,
            environment: None,
            post: false,
            frame: 0,
            seed: 0,
        };
        let image = super::super::cpu::render(&scene, Some(&icons), [200, 60]).unwrap();
        let at = |x: usize, y: usize| image[y * 200 + x];
        let main = |c: [f32; 4]| (0..3).max_by(|a, b| c[*a].total_cmp(&c[*b])).unwrap();
        assert_eq!(main(at(4, 22)), 0, "the left stem: {:?}", at(4, 22));
        assert!(
            at(20, 22)[..3].iter().all(|v| *v < 0.05),
            "the hole: {:?}",
            at(20, 22)
        );
        assert!(
            at(56, 22)[0] + at(56, 22)[1] > 0.9,
            "the curved side: {:?}",
            at(56, 22)
        );
        assert!(
            at(64, 22)[..3].iter().all(|v| *v < 0.05),
            "past the curve: {:?}",
            at(64, 22)
        );
        assert_eq!(main(at(185, 22)), 2, "the right path: {:?}", at(185, 22));
        assert_eq!(main(at(2, 53)), 0, "the rule's start: {:?}", at(2, 53));
        assert_eq!(main(at(100, 53)), 1, "the rule's middle: {:?}", at(100, 53));
        assert_eq!(main(at(197, 53)), 2, "the rule's end: {:?}", at(197, 53));
        assert!(at(100, 47)[..3].iter().all(|v| *v < 0.05), "above the rule");
    }

    #[test]
    fn two_stop_box_gradients_stay_vertical_fills() {
        let source = r##"<svg width="10" height="10"><defs><linearGradient id="v" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#000000"/></linearGradient></defs><rect width="10" height="10" fill="url(#v)"/></svg>"##;
        let board = read(source, &FILTERS).unwrap();
        assert_eq!(
            board.draws[0].fill,
            Fill::Vertical(Srgba::hex(0xffffff), Srgba::hex(0))
        );
    }

    #[test]
    fn marks_read_radial_gradients_strokes_with_caps_and_fill_opacity() {
        let source = r##"<svg width="20" height="20"><defs><radialGradient id="r" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="rgb(0, 0, 255)"/></radialGradient></defs><circle cx="10" cy="10" r="8" fill="url(#r)" fill-opacity="0.5"/><path d="M2 10H18" fill="none" stroke="#00ff00" stroke-width="4"/><path d="M2 14H18" fill="none" stroke="#00ff00" stroke-width="4" stroke-linecap="square"/></svg>"##;
        let board = read_mark(source, Srgba::WHITE).unwrap();
        assert_eq!(board.draws.len(), 3);
        let Fill::Gradient(g) = &board.draws[0].fill else {
            panic!("a radial fill: {:?}", board.draws[0].fill);
        };
        assert!(matches!(
            g.shape,
            pfx_post::look::GradientShape::Radial { .. }
        ));
        assert!(g.stops().all(|(_, c)| (c[3] - 0.5).abs() < 1e-6));
        let reach = |draw: &Draw| {
            let Shape::Icon(Icon(i)) = draw.shape else {
                panic!("a stroke is an outline");
            };
            let xs: Vec<f32> = board.icons[i as usize]
                .polygons
                .iter()
                .flatten()
                .map(|p| p[0])
                .collect();
            (
                xs.iter().copied().fold(f32::MAX, f32::min),
                xs.iter().copied().fold(f32::MIN, f32::max),
            )
        };
        let (butt, square) = (reach(&board.draws[1]), reach(&board.draws[2]));
        assert!(
            (butt.0 - 2.0).abs() < 0.05 && (butt.1 - 18.0).abs() < 0.05,
            "{butt:?}"
        );
        assert!(
            (square.0 - 0.0).abs() < 0.05 && (square.1 - 20.0).abs() < 0.05,
            "{square:?}"
        );
        assert_eq!(board.draws[1].fill, Fill::Solid(Srgba::hex(0x00ff00)));
        let plain = read(source, &FILTERS).unwrap();
        assert!(
            plain.draws[1].stroke.is_some(),
            "boards keep their SDF strokes"
        );
    }

    #[test]
    fn unsupported_input_is_refused_by_name() {
        assert!(read("<svg width=\"1\" height=\"1\"><ellipse/></svg>", &FILTERS).is_err());
        assert!(read("<svg width=\"1\" height=\"1\"><rect width=\"1\" height=\"1\" filter=\"url(#x)\"/></svg>", &FILTERS).is_err());
        let board = read(
            "<svg width=\"4\" height=\"2\"><rect width=\"4\" height=\"2\" fill=\"#fff\"/><text x=\"1\">hi<tspan>x</tspan></text></svg>",
            &FILTERS,
        )
        .unwrap();
        assert_eq!(board.size, [4.0, 2.0]);
        assert_eq!(board.draws.len(), 1);
    }
}
