use super::icons::Icon;
use super::{
    Draw, Fill, Glow, Group, IDENTITY, IconShape, Matrix, Shadow, Shape, Srgba, Stroke, multiply,
    rotation, scaling, translation,
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
        .parse()
        .map_err(|_| format!("{value:?} is not a number"))
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

fn path(d: &str) -> Result<Vec<Vec<[f32; 2]>>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for c in d.chars() {
        if c.is_ascii_alphabetic() && c != 'e' {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            tokens.push(c.to_string());
        } else if c == '-' && !current.is_empty() && !current.ends_with('e') {
            tokens.push(std::mem::take(&mut current));
            current.push('-');
        } else if c == ',' || c.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    let mut subpaths: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut at = [0.0f32, 0.0];
    let mut command = 'M';
    let mut index = 0;
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
                continue;
            }
        }
        let relative = command.is_ascii_lowercase();
        let base = if relative { at } else { [0.0, 0.0] };
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
    let mut gradients: HashMap<String, Vec<Srgba>> = HashMap::new();
    let mut symbols: HashMap<String, Tag> = HashMap::new();
    let mut size = [0.0, 0.0];
    let mut body = Vec::new();
    let mut in_defs = false;
    let mut in_text = 0usize;
    let mut gradient: Option<String> = None;
    let mut symbol: Option<String> = None;
    for tag in all {
        match (tag.name.as_str(), tag.closing) {
            ("svg", false) => {
                size = [number(tag.need("width")?)?, number(tag.need("height")?)?];
            }
            ("svg", true) => {}
            ("defs", closing) => in_defs = !closing,
            ("text", false) => in_text += 1,
            ("text", true) => in_text = in_text.saturating_sub(1),
            _ if in_text > 0 => {}
            ("linearGradient", false) if in_defs => gradient = Some(tag.need("id")?.to_string()),
            ("stop", _) if in_defs => {
                let mut c = colour(tag.need("stop-color")?)?;
                if let Some(alpha) = tag.get("stop-opacity") {
                    c = c.alpha(number(alpha)?);
                }
                gradients
                    .entry(gradient.clone().ok_or("a stop outside a gradient")?)
                    .or_default()
                    .push(c);
            }
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
        transform: IDENTITY,
        filter: None,
        group: None,
        first: false,
        rotated: false,
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
        let paint = |key: &str| -> Result<Option<Fill>, String> {
            let Some(value) = element.get(key) else {
                return Ok(None);
            };
            if value == "none" {
                return Ok(None);
            }
            if let Some(id) = value.strip_prefix("url(#") {
                let stops = gradients
                    .get(id.trim_end_matches(')'))
                    .filter(|stops| stops.len() == 2)
                    .ok_or_else(|| format!("{value} is not a two-stop gradient"))?;
                return Ok(Some(Fill::Vertical(stops[0], stops[1])));
            }
            Ok(Some(Fill::Solid(colour(value)?)))
        };
        let fill = match (element.name.as_str(), element.get("fill")) {
            ("path", None) => None,
            (_, None) => Some(Fill::Solid(Srgba::hex(0))),
            _ => paint("fill")?,
        };
        let stroke_width = element.get("stroke-width").map_or(Ok(1.0), number)?;
        let stroke = match paint("stroke")? {
            None => None,
            Some(Fill::Solid(mut c)) => {
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
        let mut draw = match element.name.as_str() {
            "rect" => {
                let x = element.get("x").map_or(Ok(0.0), number)?;
                let y = element.get("y").map_or(Ok(0.0), number)?;
                let w = number(element.need("width")?)?;
                let h = number(element.need("height")?)?;
                let r = element.get("rx").map_or(Ok(0.0), number)?;
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
                match (fill, stroke) {
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
            draw.fill = fill.unwrap_or(Fill::None);
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
