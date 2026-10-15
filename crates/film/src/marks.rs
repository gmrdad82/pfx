use crate::colour::hex;
use crate::sheet::{Sheet, painted};
use crate::titles::{Align, Setter, Words};
use pfx_live::flat::{
    Draw, Icon, IconShape, Matrix, Shape, Srgba, multiply, scaling, svg, translation,
};

const REACH: f32 = 0.04;
pub const LABEL: f32 = 0.62;
const GAP: f32 = 0.4;
const BADGE: f32 = 0.66;
const WEIGHT: u16 = 600;
const ELEVATION: f32 = 20.0;
const RING: usize = 16;
const INNER: usize = 8;

#[derive(Clone, Debug, PartialEq)]
struct Part {
    icon: u32,
    fill: Srgba,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub name: String,
    pub view: [f32; 2],
    parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub word: String,
    pub colour: Srgba,
}

impl Default for Check {
    fn default() -> Check {
        Check {
            word: "Checked".to_string(),
            colour: Srgba::hex(0x3f9f5a),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marks {
    pub list: Vec<Mark>,
    pub shapes: Vec<IconShape>,
    pub base: u32,
    pub check: Check,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Badge {
    pub mark: String,
    pub centre: [f32; 2],
    pub size: f32,
    pub tint: Option<Srgba>,
    pub label: Option<String>,
    pub colour: Srgba,
    pub check: bool,
    pub outline: f32,
    pub edge: Srgba,
}

fn attr<'a>(element: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let start = element.find(&key)? + key.len();
    let len = element[start..].find('"')?;
    Some(&element[start..start + len])
}

fn apply(m: &Matrix, p: [f32; 2]) -> [f32; 2] {
    [
        m[0][0] * p[0] + m[1][0] * p[1] + m[3][0],
        m[0][1] * p[0] + m[1][1] * p[1] + m[3][1],
    ]
}

fn bounds(points: &[[f32; 2]]) -> [f32; 4] {
    points.iter().fold(
        [f32::MAX, f32::MAX, f32::MIN, f32::MIN],
        |[x0, y0, x1, y1], p| [x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1])],
    )
}

fn area(b: [f32; 4]) -> f32 {
    (b[2] - b[0]) * (b[3] - b[1])
}

fn within(p: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    let mut inside = false;
    for (i, &a) in polygon.iter().enumerate() {
        let b = polygon[(i + 1) % polygon.len()];
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
        {
            inside = !inside;
        }
    }
    inside
}

fn groups(lines: Vec<Vec<[f32; 2]>>) -> Vec<Vec<Vec<[f32; 2]>>> {
    let boxes: Vec<[f32; 4]> = lines.iter().map(|l| bounds(l)).collect();
    let parent: Vec<Option<usize>> = (0..lines.len())
        .map(|j| {
            (0..lines.len())
                .filter(|&i| i != j && area(boxes[i]) > area(boxes[j]))
                .filter(|&i| within(lines[j][0], &lines[i]))
                .min_by(|&a, &b| area(boxes[a]).total_cmp(&area(boxes[b])))
        })
        .collect();
    let root = |mut j: usize| {
        while let Some(p) = parent[j] {
            j = p;
        }
        j
    };
    let mut out: Vec<(usize, Vec<Vec<[f32; 2]>>)> = Vec::new();
    for (j, line) in lines.into_iter().enumerate() {
        let r = root(j);
        match out.iter_mut().find(|(k, _)| *k == r) {
            Some((_, g)) => g.push(line),
            None => out.push((r, vec![line])),
        }
    }
    out.into_iter().map(|(_, g)| g).collect()
}

fn fill(element: &str) -> Result<Srgba, String> {
    match attr(element, "fill") {
        None | Some("currentColor") => Ok(Srgba::WHITE),
        Some(f) => hex(f),
    }
}

fn parse(name: &str, source: &str, shapes: &mut Vec<IconShape>) -> Result<Mark, String> {
    let fail = |e: String| format!("{name}: {e}");
    let root = source
        .split("<svg")
        .nth(1)
        .ok_or_else(|| fail("no <svg>".into()))?;
    let view = attr(root, "viewBox").ok_or_else(|| fail("no viewBox".into()))?;
    let v: Vec<f32> = view
        .split([' ', ','])
        .filter_map(|n| n.parse().ok())
        .collect();
    let [_, _, w, h] = v[..] else {
        return Err(fail("the viewBox needs four numbers".into()));
    };
    let reach = h * REACH;
    let mut parts = Vec::new();
    for element in source.split("<path").skip(1) {
        let element = element.split('>').next().unwrap_or_default();
        let d = attr(element, "d").ok_or_else(|| fail("a path has no d".into()))?;
        let doc = format!(
            "<svg width=\"{w}\" height=\"{h}\" viewBox=\"{view}\"><path fill=\"#ffffff\" d=\"{d}\"/></svg>"
        );
        let board = svg::read(&doc, &[]).map_err(fail)?;
        let colour = fill(element).map_err(fail)?;
        for draw in &board.draws {
            let Shape::Icon(Icon(i)) = draw.shape else {
                continue;
            };
            let shape = board
                .icons
                .get(i as usize)
                .ok_or_else(|| fail("a missing path".into()))?;
            let lines: Vec<Vec<[f32; 2]>> = shape
                .polygons
                .iter()
                .filter(|l| l.len() > 2)
                .map(|l| l.iter().map(|p| apply(&draw.transform, *p)).collect())
                .collect();
            for polygons in groups(lines) {
                parts.push(Part {
                    icon: shapes.len() as u32,
                    fill: colour,
                });
                shapes.push(IconShape {
                    polygons,
                    reach,
                    ..IconShape::default()
                });
            }
        }
    }
    if parts.is_empty() {
        return Err(fail("no paths".into()));
    }
    Ok(Mark {
        name: name.to_string(),
        view: [w, h],
        parts,
    })
}

impl Marks {
    pub fn load(base: u32, files: &[(&str, &str)]) -> Result<Marks, String> {
        let mut shapes = Vec::new();
        let mut list = Vec::with_capacity(files.len());
        for (name, source) in files {
            list.push(parse(name, source, &mut shapes)?);
        }
        Ok(Marks {
            list,
            shapes,
            base,
            check: Check::default(),
        })
    }

    pub fn with_check(mut self, check: Check) -> Marks {
        self.check = check;
        self
    }

    pub fn names(&self) -> Vec<&str> {
        self.list.iter().map(|m| m.name.as_str()).collect()
    }

    fn icons(
        &self,
        mark: &Mark,
        frame: Matrix,
        tint: Option<Srgba>,
        ring: &Ring,
        out: &mut Layers,
    ) {
        for part in &mark.parts {
            let draw = Draw::new(Shape::Icon(Icon(self.base + part.icon))).transform(frame);
            let fill = tint.unwrap_or(part.fill);
            ring.each(|shift, edge| {
                let at = multiply(translation(shift[0], shift[1]), frame);
                out.put(
                    edge,
                    painted(draw.transform(at), Some(edge.unwrap_or(fill)), None),
                );
            });
        }
    }

    pub fn draw<S: Sheet>(&self, b: &mut S, setter: &mut Setter, badge: &Badge, glow: bool) {
        let Some(mark) = self.list.iter().find(|m| m.name == badge.mark) else {
            return;
        };
        let s = badge.size / mark.view[1];
        let width = mark.view[0] * s;
        let text = badge.size * LABEL;
        let gap = badge.size * GAP;
        let label = badge.label.as_deref().unwrap_or_default();
        let label_width = if label.is_empty() {
            0.0
        } else {
            gap + setter.width(b, label, text, WEIGHT, 0.0)
        };
        let disc = badge.size * BADGE;
        let check_width = if badge.check {
            gap + disc + gap * 0.5 + setter.width(b, &self.check.word, text, WEIGHT, 0.0)
        } else {
            0.0
        };
        let left = badge.centre[0] - (width + label_width + check_width) * 0.5;
        let y = badge.centre[1];
        let ring = Ring {
            outline: badge.outline.max(0.0),
            edge: badge.edge,
        };
        let mut layers = Layers::default();
        let frame = multiply(translation(left, y - badge.size * 0.5), scaling(s, s));
        self.icons(mark, frame, badge.tint, &ring, &mut layers);
        let x = left + width + label_width;
        if badge.check {
            let green = self.check.colour;
            let centre = [x + gap + disc * 0.5, y];
            ring.each(|shift, edge| {
                let at = [centre[0] + shift[0], centre[1] + shift[1]];
                let draw = Draw::circle(at, disc * 0.5);
                layers.put(edge, painted(draw, Some(edge.unwrap_or(green)), None));
            });
            if let Some(tick) = b.check(centre, disc * 0.5) {
                layers.put(None, painted(tick, None, Some((Srgba::WHITE, disc * 0.12))));
            }
        }
        for mut d in layers.edges {
            d.elevation = ELEVATION;
            b.add(d);
        }
        for mut d in layers.fills {
            d.elevation = ELEVATION;
            d.fx.bloom = glow;
            b.add(d);
        }
        let mut set = |t: &str, x: f32| {
            let mut w = Words::new(t, [x, y], text, badge.colour);
            w.align = Align::Start;
            w.weight = WEIGHT;
            w.outline = badge.outline;
            w.edge = badge.edge;
            setter.set(b, &w, glow);
        };
        if !label.is_empty() {
            set(label, left + width + gap);
        }
        if badge.check {
            set(&self.check.word, x + gap + disc + gap * 0.5);
        }
    }
}

#[derive(Default)]
struct Layers {
    edges: Vec<Draw>,
    fills: Vec<Draw>,
}

impl Layers {
    fn put(&mut self, edge: Option<Srgba>, draw: Draw) {
        match edge {
            Some(_) => self.edges.push(draw),
            None => self.fills.push(draw),
        }
    }
}

struct Ring {
    outline: f32,
    edge: Srgba,
}

impl Ring {
    fn each(&self, mut f: impl FnMut([f32; 2], Option<Srgba>)) {
        if self.outline > 0.0 {
            for (count, reach) in [(RING, self.outline), (INNER, self.outline * 0.5)] {
                for k in 0..count {
                    let a = std::f32::consts::TAU * k as f32 / count as f32;
                    f([a.cos() * reach, a.sin() * reach], Some(self.edge));
                }
            }
        }
        f([0.0, 0.0], None);
    }
}
