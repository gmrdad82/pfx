use super::icons::Icon;
use super::{
    Draw, Fill, Glow, Gradient, Group, IDENTITY, IconShape, Matrix, Shadow, Shape, Srgba, Stroke,
    multiply, rotation, scaling, translation,
};
use std::collections::HashMap;

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

fn colour(value: &str) -> Result<Srgba, String> {
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

fn path(d: &str) -> Result<Vec<Vec<[f32; 2]>>, String> {
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
    Ok(subpaths)
}

#[derive(Clone)]
struct Context {
    transform: Matrix,
    filter: Option<String>,
    group: Option<u16>,
    first: bool,
    rotated: bool,
    paint: Paint,
}

#[derive(Clone, Default)]
struct Paint {
    fill: Option<String>,
    stroke: Option<String>,
    stroke_width: Option<String>,
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
        }
    }
}

#[derive(Clone, Debug, Default)]
struct LinearGradient {
    user_space: bool,
    ends: [Option<String>; 4],
    stops: Vec<(f32, Srgba)>,
}

impl LinearGradient {
    fn of(tag: &Tag) -> Result<Self, String> {
        if tag.get("gradientTransform").is_some() {
            return Err("a gradientTransform is unsupported".into());
        }
        Ok(Self {
            user_space: tag.get("gradientUnits") == Some("userSpaceOnUse"),
            ends: ["x1", "y1", "x2", "y2"].map(|key| tag.get(key).map(str::to_string)),
            stops: Vec::new(),
        })
    }

    fn vertical_box(&self) -> bool {
        let at = |index: usize, value: f32| {
            self.ends[index]
                .as_deref()
                .map_or(
                    Ok(index == 2 && value == 1.0 || index != 2 && value == 0.0),
                    |end| fraction(end).map(|v| v == value),
                )
                .unwrap_or(false)
        };
        !self.user_space
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
        let defaults = [0.0, 0.0, 1.0, 0.0];
        let mut ends = [0.0f32; 4];
        for (index, end) in ends.iter_mut().enumerate() {
            let value = self.ends[index].as_deref();
            *end = match (self.user_space, value) {
                (_, None) if self.user_space => defaults[index] * view[index % 2],
                (_, None) => defaults[index],
                (true, Some(v)) if v.trim().ends_with('%') => fraction(v)? * view[index % 2],
                (_, Some(v)) => fraction(v)?,
            };
        }
        let point = |x: f32, y: f32| {
            if self.user_space {
                [x - origin[0], y - origin[1]]
            } else {
                [
                    bounds[0] + x * (bounds[2] - bounds[0]) - origin[0],
                    bounds[1] + y * (bounds[3] - bounds[1]) - origin[1],
                ]
            }
        };
        let stops: Vec<(f32, [f32; 4])> = self.stops.iter().map(|(at, c)| (*at, c.0)).collect();
        Gradient::linear(point(ends[0], ends[1]), point(ends[2], ends[3]), &stops)
            .map(Fill::Gradient)
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
    for tag in all {
        match (tag.name.as_str(), tag.closing) {
            ("svg", false) => (size, view) = root(&tag)?,
            ("svg", true) => {}
            ("defs", closing) => in_defs = !closing,
            ("text", false) => in_text += 1,
            ("text", true) => in_text = in_text.saturating_sub(1),
            _ if in_text > 0 => {}
            ("linearGradient", false) => {
                let id = tag.need("id")?.to_string();
                gradients.insert(id.clone(), LinearGradient::of(&tag)?);
                gradient = Some(id);
            }
            ("linearGradient", true) => gradient = None,
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
    let mut board = Board {
        size,
        ..Board::default()
    };
    let mut icon_keys: Vec<(String, u32)> = Vec::new();
    let mut level = 0.0f32;
    let mut stack = vec![Context {
        transform: view,
        filter: None,
        group: None,
        first: false,
        rotated: false,
        paint: Paint::default(),
    }];
    for tag in body {
        let top = stack.last().ok_or("a group closed twice")?.clone();
        let rotates = tag.get("transform").is_some_and(|t| t.contains("rotate"));
        if tag.name == "g" {
            if tag.closing {
                stack.pop();
                continue;
            }
            let own = tag.get("transform").map_or(Ok(IDENTITY), transform)?;
            let opacity = tag.get("opacity").map_or(Ok(1.0), number)?;
            let group = if opacity < 1.0 {
                board.groups.push(Group { opacity });
                Some(u16::try_from(board.groups.len() - 1).map_err(|_| "too many groups")?)
            } else {
                top.group
            };
            stack.push(Context {
                transform: multiply(top.transform, own),
                filter: tag.get("filter").map(str::to_string).or(top.filter.clone()),
                group,
                first: tag.get("filter").is_some(),
                rotated: top.rotated || rotates,
                paint: top.paint.under(&tag),
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
            Ok(Some(Ink::Solid(colour(value)?)))
        };
        let fill = match (element.name.as_str(), inherited.fill.as_deref()) {
            ("path", None) => None,
            (_, None) => Some(Ink::Solid(Srgba::hex(0))),
            (_, value) => ink(value)?,
        };
        let stroke_width = inherited.stroke_width.as_deref().map_or(Ok(1.0), number)?;
        let stroke = match ink(inherited.stroke.as_deref())? {
            None => None,
            Some(Ink::Solid(mut c)) => {
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
        let mut draw = match element.name.as_str() {
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
            "path" => {
                let d = element.need("d")?;
                let subpaths = path(d)?;
                box_of = (bounds_of(&subpaths), [0.0, 0.0]);
                let reach = stroke.map_or(0.0, |s| s.width * 0.5) + 4.0;
                let shape = if fill.is_some() {
                    IconShape {
                        polygons: subpaths,
                        reach,
                        ..IconShape::default()
                    }
                } else {
                    IconShape {
                        lines: subpaths,
                        reach,
                        ..IconShape::default()
                    }
                };
                let key = (d.to_string(), reach.to_bits());
                let index = match icon_keys.iter().position(|k| *k == key) {
                    Some(index) => index,
                    None => {
                        icon_keys.push(key);
                        board.icons.push(shape);
                        icon_keys.len() - 1
                    }
                };
                Draw::new(Shape::Icon(Icon(index as u32))).transform(base)
            }
            other => return Err(format!("<{other}> is unsupported")),
        };
        if matches!(draw.shape, Shape::Ring { .. } | Shape::Arc { .. }) {
            draw.opacity = opacity;
        } else {
            draw.fill = match fill {
                None => Fill::None,
                Some(Ink::Solid(c)) => Fill::Solid(c),
                Some(Ink::Gradient(gradient)) => gradient.fill(box_of.0, box_of.1, size)?,
            };
            draw.stroke = stroke;
            draw.opacity = opacity;
        }
        draw.group = top.group;
        draw.shadow = Shadow::None;
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
        board.draws.push(draw);
    }
    Ok(board)
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
