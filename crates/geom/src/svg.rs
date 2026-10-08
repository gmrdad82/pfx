use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, TAU};
use std::str::FromStr;

use svgtypes::{SimplePathSegment, SimplifyingPathParser, Transform};

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const SHAPES: [&str; 7] = [
    "path", "rect", "circle", "ellipse", "polygon", "polyline", "line",
];
const SKIPPED: [&str; 11] = [
    "defs",
    "mask",
    "clipPath",
    "linearGradient",
    "radialGradient",
    "title",
    "desc",
    "symbol",
    "pattern",
    "marker",
    "metadata",
];
const STYLE_KEYS: [&str; 8] = [
    "fill",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "mask",
    "fill-rule",
    "opacity",
];

#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Color([u8; 3]),
    Gradient(Gradient),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub id: String,
    pub stops: Vec<(f32, [u8; 3])>,
    pub x1: f32,
    pub x2: f32,
    pub y1: f32,
    pub y2: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub points: Vec<[f64; 2]>,
    pub closed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub tag: String,
    pub paths: Vec<Path>,
    pub fill: Option<Paint>,
    pub stroke: Option<Paint>,
    pub stroke_width: f64,
    pub joined: bool,
    pub fill_rule: FillRule,
    pub holes: Vec<Vec<[f64; 2]>>,
    pub ground: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Drawing {
    pub view: [f64; 4],
    pub elements: Vec<Element>,
    pub tolerance: f64,
}

impl Drawing {
    pub fn parse(text: &str, tolerance: f64) -> Result<Drawing, String> {
        if text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
            return Err("refusing an SVG with a DOCTYPE or ENTITY".to_string());
        }
        let document = roxmltree::Document::parse(text).map_err(|e| format!("svg: {e}"))?;
        let root = document.root_element();
        if root.tag_name().name() != "svg" {
            return Err("svg: the root element is not <svg>".to_string());
        }
        let view = match root.attribute("viewBox") {
            Some(value) => view_box(value)?,
            None => [0.0, 0.0, 100.0, 100.0],
        };
        let size = view[2].max(view[3]).max(1e-9);
        let tolerance = (tolerance * size).max(1e-9);
        let mut gradients = BTreeMap::new();
        let mut masks = BTreeMap::new();
        for node in root.descendants().filter(|n| is_svg(n)) {
            let id = node.attribute("id").unwrap_or("").to_string();
            match node.tag_name().name() {
                "linearGradient" | "radialGradient" => {
                    gradients.insert(id.clone(), gradient(&node, id)?);
                }
                "mask" => {
                    masks.insert(id, mask_holes(&node, tolerance)?);
                }
                _ => {}
            }
        }
        let mut walker = Walker {
            view,
            tolerance,
            gradients: &gradients,
            masks: &masks,
            elements: Vec::new(),
        };
        let mut start = BTreeMap::new();
        start.insert("fill".to_string(), "black".to_string());
        walker.walk(root, &start, Transform::default())?;
        Ok(Drawing {
            view,
            elements: walker.elements,
            tolerance,
        })
    }
}

fn is_svg(node: &roxmltree::Node) -> bool {
    node.is_element() && matches!(node.tag_name().namespace(), None | Some(SVG_NS))
}

fn view_box(value: &str) -> Result<[f64; 4], String> {
    let numbers = numbers(value)?;
    if numbers.len() != 4 || numbers[2] <= 0.0 || numbers[3] <= 0.0 {
        return Err(format!("svg: bad viewBox \"{value}\""));
    }
    Ok([numbers[0], numbers[1], numbers[2], numbers[3]])
}

fn numbers(value: &str) -> Result<Vec<f64>, String> {
    value
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<f64>()
                .map_err(|_| format!("svg: \"{part}\" is not a number"))
        })
        .collect()
}

fn length(value: Option<&str>, default: f64) -> Result<f64, String> {
    let Some(value) = value else {
        return Ok(default);
    };
    let trimmed = value.trim().trim_end_matches('%');
    if trimmed.is_empty() {
        return Ok(default);
    }
    svgtypes::Length::from_str(trimmed)
        .map(|length| length.number)
        .map_err(|_| format!("svg: \"{value}\" is not a length"))
}

fn style_of(
    node: &roxmltree::Node,
    inherited: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut out = inherited.clone();
    for key in STYLE_KEYS {
        if let Some(value) = node.attribute(key) {
            out.insert(key.to_string(), value.to_string());
        }
    }
    for part in node.attribute("style").unwrap_or("").split(';') {
        if let Some((key, value)) = part.split_once(':') {
            out.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    out
}

fn color(value: &str) -> Result<[u8; 3], String> {
    let parsed = svgtypes::Color::from_str(value.trim())
        .map_err(|_| format!("svg: bad colour \"{value}\""))?;
    Ok([parsed.red, parsed.green, parsed.blue])
}

fn gradient(node: &roxmltree::Node, id: String) -> Result<Gradient, String> {
    let mut stops = Vec::new();
    for stop in node
        .children()
        .filter(|n| is_svg(n) && n.tag_name().name() == "stop")
    {
        let style = style_of(&stop, &BTreeMap::new());
        let value = stop
            .attribute("stop-color")
            .or(style.get("stop-color").map(String::as_str))
            .unwrap_or("#000000");
        let offset = stop.attribute("offset").unwrap_or("0").trim();
        let offset = match offset.strip_suffix('%') {
            Some(percent) => percent.parse::<f64>().map(|v| v / 100.0),
            None => offset.parse::<f64>(),
        }
        .map_err(|_| format!("svg: gradient {id} has a bad stop offset \"{offset}\""))?;
        stops.push((offset as f32, color(value)?));
    }
    let coordinate = |key: &str, default: &str| -> Result<f32, String> {
        let raw = node.attribute(key).unwrap_or(default);
        raw.trim()
            .trim_end_matches('%')
            .parse::<f32>()
            .map_err(|_| format!("svg: gradient {id} has a bad {key} \"{raw}\""))
    };
    Ok(Gradient {
        x1: coordinate("x1", "0")?,
        x2: coordinate("x2", "1")?,
        y1: coordinate("y1", "0")?,
        y2: coordinate("y2", "0")?,
        id,
        stops,
    })
}

fn mask_holes(node: &roxmltree::Node, tolerance: f64) -> Result<Vec<Vec<[f64; 2]>>, String> {
    let mut holes = Vec::new();
    for shape in node.descendants().filter(|n| is_svg(n)) {
        if shape.tag_name().name() != "rect" {
            continue;
        }
        let fill = shape.attribute("fill").unwrap_or("").to_ascii_lowercase();
        if !matches!(fill.as_str(), "black" | "#000" | "#000000") {
            continue;
        }
        let x = length(shape.attribute("x"), 0.0)?;
        let y = length(shape.attribute("y"), 0.0)?;
        let w = length(shape.attribute("width"), 0.0)?;
        let h = length(shape.attribute("height"), 0.0)?;
        let rx = length(shape.attribute("rx"), 0.0)?;
        if w > 0.0 && h > 0.0 {
            holes.push(rounded_rect(x, y, w, h, rx, rx, tolerance));
        }
    }
    Ok(holes)
}

struct Walker<'a> {
    view: [f64; 4],
    tolerance: f64,
    gradients: &'a BTreeMap<String, Gradient>,
    masks: &'a BTreeMap<String, Vec<Vec<[f64; 2]>>>,
    elements: Vec<Element>,
}

impl Walker<'_> {
    fn walk(
        &mut self,
        node: roxmltree::Node,
        inherited: &BTreeMap<String, String>,
        parent: Transform,
    ) -> Result<(), String> {
        if !is_svg(&node) {
            return Ok(());
        }
        let tag = node.tag_name().name();
        if SKIPPED.contains(&tag) {
            return Ok(());
        }
        let own = match node.attribute("transform") {
            Some(text) => {
                Transform::from_str(text).map_err(|_| format!("svg: bad transform \"{text}\""))?
            }
            None => Transform::default(),
        };
        let transform = multiply(&parent, &own);
        let style = style_of(&node, inherited);
        if SHAPES.contains(&tag) {
            self.shape(&node, tag, &style, &transform)?;
        }
        for child in node.children() {
            self.walk(child, &style, transform)?;
        }
        Ok(())
    }

    fn paint(&self, value: Option<&String>) -> Result<Option<Paint>, String> {
        let Some(value) = value.map(|v| v.trim()) else {
            return Ok(None);
        };
        if value.is_empty() || value == "none" || value == "transparent" {
            return Ok(None);
        }
        if let Some(rest) = value.strip_prefix("url(#") {
            let id = rest.trim_end_matches(')').trim();
            return self
                .gradients
                .get(id)
                .map(|g| Some(Paint::Gradient(g.clone())))
                .ok_or_else(|| format!("svg: no gradient #{id}"));
        }
        Ok(Some(Paint::Color(color(value)?)))
    }

    fn shape(
        &mut self,
        node: &roxmltree::Node,
        tag: &str,
        style: &BTreeMap<String, String>,
        transform: &Transform,
    ) -> Result<(), String> {
        let det = transform.a * transform.d - transform.b * transform.c;
        let spread = det.abs().sqrt();
        let local_tolerance = self.tolerance / spread.max(1e-9);
        let attr = |key: &str| length(node.attribute(key), 0.0);
        let mut ground = false;
        let local: Vec<Path> = match tag {
            "path" => path_data(node.attribute("d").unwrap_or(""), transform, self.tolerance)?,
            "rect" => {
                let (x, y, w, h) = (attr("x")?, attr("y")?, attr("width")?, attr("height")?);
                let square = (self.view[2] - self.view[3]).abs() < 1e-6;
                ground = square && w >= self.view[2] * 0.98 && h >= self.view[3] * 0.98;
                let rx = node.attribute("rx").map(|_| attr("rx")).transpose()?;
                let ry = node.attribute("ry").map(|_| attr("ry")).transpose()?;
                let (rx, ry) = match (rx, ry) {
                    (Some(rx), Some(ry)) => (rx, ry),
                    (Some(r), None) | (None, Some(r)) => (r, r),
                    (None, None) => (0.0, 0.0),
                };
                closed(rounded_rect(x, y, w, h, rx, ry, local_tolerance), transform)
            }
            "circle" => {
                let r = attr("r")?;
                closed(
                    ellipse(attr("cx")?, attr("cy")?, r, r, local_tolerance),
                    transform,
                )
            }
            "ellipse" => closed(
                ellipse(
                    attr("cx")?,
                    attr("cy")?,
                    attr("rx")?,
                    attr("ry")?,
                    local_tolerance,
                ),
                transform,
            ),
            "polygon" | "polyline" => {
                let values = numbers(node.attribute("points").unwrap_or(""))?;
                let points = values
                    .chunks_exact(2)
                    .map(|p| apply(transform, [p[0], p[1]]))
                    .collect::<Vec<_>>();
                vec![Path {
                    points,
                    closed: tag == "polygon",
                }]
            }
            _ => vec![Path {
                points: vec![
                    apply(transform, [attr("x1")?, attr("y1")?]),
                    apply(transform, [attr("x2")?, attr("y2")?]),
                ],
                closed: false,
            }],
        };
        let paths = local
            .into_iter()
            .map(|mut path| {
                path.points.dedup_by(|a, b| distance(*a, *b) <= 1e-12);
                if path.closed
                    && path.points.len() > 1
                    && distance(path.points[0], *path.points.last().unwrap()) <= 1e-12
                {
                    path.points.pop();
                }
                path
            })
            .filter(|path| path.points.len() > 1)
            .collect::<Vec<_>>();
        let fill = self.paint(style.get("fill"))?;
        let stroke = self.paint(style.get("stroke"))?;
        let width = length(style.get("stroke-width").map(String::as_str), 0.0)?;
        let joined = fill.is_some()
            && stroke.is_some()
            && width > 0.0
            && style.get("stroke") == style.get("fill");
        let fill_rule = match style.get("fill-rule").map(String::as_str) {
            Some("evenodd") => FillRule::EvenOdd,
            _ => FillRule::NonZero,
        };
        let holes = match style.get("mask").map(|m| m.trim()) {
            Some(mask) => match mask.strip_prefix("url(#") {
                Some(rest) => self
                    .masks
                    .get(rest.trim_end_matches(')').trim())
                    .cloned()
                    .unwrap_or_default(),
                None => Vec::new(),
            },
            None => Vec::new(),
        };
        self.elements.push(Element {
            tag: tag.to_string(),
            paths,
            fill,
            stroke,
            stroke_width: width * spread,
            joined,
            fill_rule,
            holes,
            ground,
        });
        Ok(())
    }
}

fn multiply(outer: &Transform, inner: &Transform) -> Transform {
    Transform::new(
        outer.a * inner.a + outer.c * inner.b,
        outer.b * inner.a + outer.d * inner.b,
        outer.a * inner.c + outer.c * inner.d,
        outer.b * inner.c + outer.d * inner.d,
        outer.a * inner.e + outer.c * inner.f + outer.e,
        outer.b * inner.e + outer.d * inner.f + outer.f,
    )
}

fn apply(t: &Transform, p: [f64; 2]) -> [f64; 2] {
    [t.a * p[0] + t.c * p[1] + t.e, t.b * p[0] + t.d * p[1] + t.f]
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn closed(points: Vec<[f64; 2]>, transform: &Transform) -> Vec<Path> {
    if points.len() < 3 {
        return Vec::new();
    }
    vec![Path {
        points: points.into_iter().map(|p| apply(transform, p)).collect(),
        closed: true,
    }]
}

fn arc_steps(radius: f64, tolerance: f64, span: f64) -> usize {
    if radius <= tolerance {
        return 1;
    }
    let half = (1.0 - tolerance / radius).clamp(-1.0, 1.0).acos();
    ((span / (2.0 * half)).ceil() as usize).clamp(1, 4096)
}

fn ellipse(cx: f64, cy: f64, rx: f64, ry: f64, tolerance: f64) -> Vec<[f64; 2]> {
    if rx <= 0.0 || ry <= 0.0 {
        return Vec::new();
    }
    let steps = arc_steps(rx.max(ry), tolerance, TAU).max(8);
    (0..steps)
        .map(|i| {
            let angle = TAU * i as f64 / steps as f64;
            [cx + rx * angle.cos(), cy + ry * angle.sin()]
        })
        .collect()
}

fn rounded_rect(x: f64, y: f64, w: f64, h: f64, rx: f64, ry: f64, tolerance: f64) -> Vec<[f64; 2]> {
    if w <= 0.0 || h <= 0.0 {
        return Vec::new();
    }
    let rx = rx.clamp(0.0, w * 0.5);
    let ry = ry.clamp(0.0, h * 0.5);
    if rx <= 0.0 || ry <= 0.0 {
        return vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
    }
    let steps = arc_steps(rx.max(ry), tolerance, FRAC_PI_2).max(2);
    let corners = [
        ([x + w - rx, y + ry], -FRAC_PI_2),
        ([x + w - rx, y + h - ry], 0.0),
        ([x + rx, y + h - ry], FRAC_PI_2),
        ([x + rx, y + ry], std::f64::consts::PI),
    ];
    let mut out = Vec::with_capacity(4 * (steps + 1));
    for (center, from) in corners {
        for i in 0..=steps {
            let angle = from + FRAC_PI_2 * i as f64 / steps as f64;
            out.push([center[0] + rx * angle.cos(), center[1] + ry * angle.sin()]);
        }
    }
    out
}

fn path_data(d: &str, transform: &Transform, tolerance: f64) -> Result<Vec<Path>, String> {
    let mut paths = Vec::new();
    let mut current: Vec<[f64; 2]> = Vec::new();
    let mut start = [0.0, 0.0];
    let mut last = [0.0, 0.0];
    let flush = |current: &mut Vec<[f64; 2]>, paths: &mut Vec<Path>, closed: bool| {
        if current.len() > 1 {
            paths.push(Path {
                points: std::mem::take(current),
                closed,
            });
        } else {
            current.clear();
        }
    };
    for segment in SimplifyingPathParser::from(d) {
        let segment = segment.map_err(|e| format!("svg: path data: {e}"))?;
        match segment {
            SimplePathSegment::MoveTo { x, y } => {
                flush(&mut current, &mut paths, false);
                start = apply(transform, [x, y]);
                last = start;
                current.push(start);
            }
            SimplePathSegment::LineTo { x, y } => {
                if current.is_empty() {
                    current.push(last);
                }
                last = apply(transform, [x, y]);
                current.push(last);
            }
            SimplePathSegment::Quadratic { x1, y1, x, y } => {
                if current.is_empty() {
                    current.push(last);
                }
                let c = apply(transform, [x1, y1]);
                let end = apply(transform, [x, y]);
                let bend = [last[0] - 2.0 * c[0] + end[0], last[1] - 2.0 * c[1] + end[1]];
                let steps = wang(bend, 0.25, tolerance);
                for i in 1..=steps {
                    let t = i as f64 / steps as f64;
                    let u = 1.0 - t;
                    current.push([
                        u * u * last[0] + 2.0 * u * t * c[0] + t * t * end[0],
                        u * u * last[1] + 2.0 * u * t * c[1] + t * t * end[1],
                    ]);
                }
                last = end;
            }
            SimplePathSegment::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                if current.is_empty() {
                    current.push(last);
                }
                let c1 = apply(transform, [x1, y1]);
                let c2 = apply(transform, [x2, y2]);
                let end = apply(transform, [x, y]);
                let b1 = [last[0] - 2.0 * c1[0] + c2[0], last[1] - 2.0 * c1[1] + c2[1]];
                let b2 = [c1[0] - 2.0 * c2[0] + end[0], c1[1] - 2.0 * c2[1] + end[1]];
                let bend = if b1[0].hypot(b1[1]) > b2[0].hypot(b2[1]) {
                    b1
                } else {
                    b2
                };
                let steps = wang(bend, 0.75, tolerance);
                for i in 1..=steps {
                    let t = i as f64 / steps as f64;
                    let u = 1.0 - t;
                    let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    current.push([
                        w0 * last[0] + w1 * c1[0] + w2 * c2[0] + w3 * end[0],
                        w0 * last[1] + w1 * c1[1] + w2 * c2[1] + w3 * end[1],
                    ]);
                }
                last = end;
            }
            SimplePathSegment::ClosePath => {
                flush(&mut current, &mut paths, true);
                last = start;
            }
        }
    }
    flush(&mut current, &mut paths, false);
    Ok(paths)
}

fn wang(bend: [f64; 2], factor: f64, tolerance: f64) -> usize {
    let size = bend[0].hypot(bend[1]);
    ((factor * size / tolerance).sqrt().ceil() as usize).clamp(1, 1024)
}
