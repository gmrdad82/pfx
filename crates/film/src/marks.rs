use crate::sheet::{Sheet, painted};
use crate::titles::{Align, Setter, Words};
use pfx_live::flat::{
    Draw, Fill, Icon, IconShape, Matrix, Shape, SpriteFill, Srgba, multiply, scaling, svg,
    translation,
};

pub const LABEL: f32 = 0.62;
const GAP: f32 = 0.4;
const BADGE: f32 = 0.66;
const WEIGHT: u16 = 600;
const ELEVATION: f32 = 20.0;
const RING: usize = 16;
const INNER: usize = 8;

pub const MOST_RASTER: u32 = 4096;

#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub name: String,
    pub view: [f32; 2],
    draws: Vec<Draw>,
    rasters: Vec<Placed>,
}

#[derive(Clone, Debug, PartialEq)]
struct Placed {
    raster: svg::Raster,
    icons: usize,
    sprite: Option<([f32; 4], u32)>,
}

pub struct Rastered {
    pub name: String,
    pub size: [u32; 2],
    pub rgba: Vec<u8>,
}

fn raster_name(mark: &str, index: usize) -> String {
    format!("\u{1}mark/{mark}/{index}")
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

fn parse(name: &str, source: &str, shapes: &mut Vec<IconShape>) -> Result<Mark, String> {
    let fail = |e: String| format!("{name}: {e}");
    let board = svg::read_mark(source, Srgba::WHITE).map_err(fail)?;
    let base = shapes.len() as u32;
    shapes.extend(board.icons);
    let draws: Vec<Draw> = board
        .draws
        .into_iter()
        .map(|mut draw| {
            if let Shape::Icon(Icon(i)) = draw.shape {
                draw.shape = Shape::Icon(Icon(base + i));
            }
            if let Some(g) = draw.group.take() {
                draw.opacity *= board.groups.get(usize::from(g)).map_or(1.0, |g| g.opacity);
            }
            draw
        })
        .collect();
    if draws.is_empty() {
        return Err(fail("no shapes".into()));
    }
    let icons = base as usize;
    Ok(Mark {
        name: name.to_string(),
        view: board.size,
        draws,
        rasters: board
            .rasters
            .into_iter()
            .map(|raster| Placed {
                raster,
                icons,
                sprite: None,
            })
            .collect(),
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

    pub fn rasters(&self, scale: impl Fn(&Mark) -> Option<f32>) -> Result<Vec<Rastered>, String> {
        let mut out = Vec::new();
        for mark in &self.list {
            let Some(s) = scale(mark).filter(|s| s.is_finite() && *s > 0.0) else {
                continue;
            };
            for (i, placed) in mark.rasters.iter().enumerate() {
                let pixels = placed.raster.pixels(s, MOST_RASTER);
                let painted = placed
                    .raster
                    .paint(&self.shapes[placed.icons..], pixels)
                    .map_err(|e| format!("{}: {e}", mark.name))?;
                out.push(Rastered {
                    name: raster_name(&mark.name, i),
                    size: painted.size,
                    rgba: painted.rgba,
                });
            }
        }
        Ok(out)
    }

    pub fn place(&mut self, find: impl Fn(&str) -> Option<([f32; 4], u32)>) {
        for mark in &mut self.list {
            for (i, placed) in mark.rasters.iter_mut().enumerate() {
                placed.sprite = find(&raster_name(&mark.name, i));
            }
        }
    }

    fn icons(
        &self,
        mark: &Mark,
        frame: Matrix,
        tint: Option<Srgba>,
        ring: &Ring,
        out: &mut Layers,
    ) {
        for (index, part) in mark.draws.iter().enumerate() {
            let mut draw = *part;
            if matches!(draw.fill, Fill::Image(_)) {
                let Some((uv, id)) = mark
                    .rasters
                    .iter()
                    .find(|p| p.raster.draw == index)
                    .and_then(|p| p.sprite)
                else {
                    continue;
                };
                draw.fill = Fill::Image(SpriteFill::new(uv));
                draw.id = id;
                out.put(None, draw.transform(multiply(frame, draw.transform)));
                continue;
            }
            if let Shape::Icon(Icon(i)) = draw.shape {
                draw.shape = Shape::Icon(Icon(self.base + i));
            }
            let placed = multiply(frame, draw.transform);
            ring.each(|shift, edge| {
                let at = multiply(translation(shift[0], shift[1]), placed);
                let mut d = draw.transform(at);
                let opacity = d.opacity;
                match (edge, tint) {
                    (Some(e), _) => d = painted(d, Some(e), None),
                    (None, Some(t)) => d = painted(d, Some(t), None),
                    (None, None) => {}
                }
                if edge.is_some() || tint.is_some() {
                    d.opacity = opacity;
                }
                if matches!(d.fill, Fill::None) && d.stroke.is_none() {
                    return;
                }
                out.put(edge, d);
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
