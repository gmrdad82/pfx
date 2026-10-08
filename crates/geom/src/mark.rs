use std::collections::BTreeMap;

use i_overlay::core::fill_rule::FillRule as OverlayFill;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};
use lyon_tessellation::math::point;
use lyon_tessellation::path::Path as LyonPath;
use lyon_tessellation::{BuffersBuilder, FillOptions, FillTessellator, FillVertex, VertexBuffers};

use crate::field::Surface;
use crate::icon::join;
use crate::math::normalize;
use crate::mesh::Mesh;
use crate::svg::{Drawing, Element, FillRule, Paint};

pub type Region = Vec<Vec<Vec<[f64; 2]>>>;

pub const DEPTH: f32 = 0.12;
pub const BEVEL: f32 = 0.05;
pub const SEGMENTS: u32 = 8;
pub const LAYER: f32 = 0.25;
pub const SMOOTH_ANGLE: f32 = 40.0;
pub const TOLERANCE: f32 = 2e-4;
pub const BEVEL_SHARE: f32 = 0.2;
pub const CORE: f32 = 0.15;
pub const THIN_STROKE: f32 = 0.1;
pub const VOXEL_SPAN: f32 = 1400.0;
pub const RELAX: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Round,
    Chamfer,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Each {
    pub all: f32,
    pub by_element: BTreeMap<usize, f32>,
}

impl Each {
    pub fn new(all: f32) -> Self {
        Self {
            all,
            by_element: BTreeMap::new(),
        }
    }

    pub fn at(&self, element: usize) -> f32 {
        self.by_element.get(&element).copied().unwrap_or(self.all)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub depth: f32,
    pub bevel: f32,
    pub segments: u32,
    pub profile: Profile,
    pub layer: f32,
    pub inset: Each,
    pub element_depth: Each,
    pub element_z: Each,
    pub element_material: BTreeMap<usize, String>,
    pub merge: bool,
    pub smooth_angle: f32,
    pub tolerance: f32,
    pub relax: u32,
}

impl Default for Form {
    fn default() -> Self {
        Self {
            depth: DEPTH,
            bevel: BEVEL,
            segments: SEGMENTS,
            profile: Profile::Round,
            layer: LAYER,
            inset: Each::new(0.0),
            element_depth: Each::new(1.0),
            element_z: Each::new(0.0),
            element_material: BTreeMap::new(),
            merge: true,
            smooth_angle: SMOOTH_ANGLE,
            tolerance: TOLERANCE,
            relax: RELAX,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Solid {
    pub paint: Paint,
    pub material: Option<String>,
    pub order: usize,
    pub elements: Vec<usize>,
    pub span: Option<[f32; 2]>,
    pub mesh: Mesh,
}

#[derive(Clone, Debug)]
pub struct Mark {
    pub view: [f32; 4],
    pub scale: f32,
    pub ground: Option<Paint>,
    pub solids: Vec<Solid>,
}

enum Geometry {
    Region { region: Region, bevel: f64 },
    Field(Mesh),
}

struct Piece {
    paint: Paint,
    material: Option<String>,
    order: usize,
    element: usize,
    key: (usize, Option<String>, Option<usize>),
    mul: f32,
    base: f32,
    geometry: Geometry,
}

struct World {
    center: [f64; 2],
    size: f64,
    view: [f64; 4],
}

impl World {
    fn point(&self, p: [f64; 2]) -> [f64; 2] {
        [
            (p[0] - self.center[0]) / self.size,
            (self.center[1] - p[1]) / self.size,
        ]
    }

    fn uv(&self, x: f32, y: f32) -> [f32; 2] {
        let w = self.view[2] / self.size;
        let h = self.view[3] / self.size;
        [
            ((x as f64 + w * 0.5) / w) as f32,
            ((h * 0.5 - y as f64) / h) as f32,
        ]
    }
}

impl Mark {
    pub fn build(svg: &str, form: &Form) -> Result<Mark, String> {
        check(form)?;
        let drawing = Drawing::parse(svg, form.tolerance as f64)?;
        let view = drawing.view;
        let world = World {
            center: [view[0] + view[2] * 0.5, view[1] + view[3] * 0.5],
            size: view[2].max(view[3]),
            view,
        };
        let mut ground = None;
        let mut painted: Vec<Paint> = Vec::new();
        let mut pieces: Vec<Piece> = Vec::new();
        let mut n = 0usize;
        for element in &drawing.elements {
            if element.ground {
                ground = element.fill.clone();
                continue;
            }
            let made = element_pieces(element, n, form, &world)?;
            for (paint, geometry) in made {
                let order = match painted.iter().position(|p| *p == paint) {
                    Some(order) => order,
                    None => {
                        painted.push(paint.clone());
                        painted.len() - 1
                    }
                };
                let material = form.element_material.get(&n).cloned();
                let key = (
                    order,
                    material.clone(),
                    if form.merge { None } else { Some(n) },
                );
                let base =
                    order as f32 * form.layer * form.depth + form.element_z.at(n) * form.depth;
                pieces.push(Piece {
                    paint,
                    material,
                    order,
                    element: n,
                    key,
                    mul: form.element_depth.at(n).max(1e-3),
                    base,
                    geometry,
                });
            }
            n += 1;
        }
        let mut keys: Vec<(usize, Option<String>, Option<usize>)> = Vec::new();
        for piece in &pieces {
            if !keys.contains(&piece.key) {
                keys.push(piece.key.clone());
            }
        }
        let mut solids = Vec::new();
        for key in keys {
            let members = pieces.iter().filter(|p| p.key == key).collect::<Vec<_>>();
            let first = members[0];
            let mut meshes = Vec::new();
            let mut merged: Vec<(f32, f32, Vec<&Region>, f64)> = Vec::new();
            for piece in &members {
                match &piece.geometry {
                    Geometry::Region { region, bevel } => {
                        match merged.iter_mut().find(|(mul, base, _, own)| {
                            mul.to_bits() == piece.mul.to_bits()
                                && base.to_bits() == piece.base.to_bits()
                                && (*own - *bevel).abs() <= 1e-9
                        }) {
                            Some((_, _, regions, _)) => regions.push(region),
                            None => merged.push((piece.mul, piece.base, vec![region], *bevel)),
                        }
                    }
                    Geometry::Field(mesh) => meshes.push(lift(mesh, piece.mul, piece.base)),
                }
            }
            for (mul, base, regions, bevel) in merged {
                let region = if regions.len() == 1 {
                    regions[0].clone()
                } else {
                    let contours = regions
                        .iter()
                        .flat_map(|r| r.iter().flatten().cloned())
                        .collect::<Vec<_>>();
                    contours.simplify_shape_as::<i64>(OverlayFill::NonZero)
                };
                meshes.push(extrude(&region, form, bevel, mul, base, &world));
            }
            let mesh = if meshes.len() == 1 {
                meshes.pop().unwrap()
            } else {
                join(&meshes)
            };
            if mesh.indices.is_empty() {
                continue;
            }
            let span = match &first.paint {
                Paint::Gradient(g) => Some([
                    world.point([g.x1 as f64, 0.0])[0] as f32,
                    world.point([g.x2 as f64, 0.0])[0] as f32,
                ]),
                Paint::Color(_) => None,
            };
            let mut elements = members.iter().map(|p| p.element).collect::<Vec<_>>();
            elements.dedup();
            solids.push(Solid {
                paint: first.paint.clone(),
                material: first.material.clone(),
                order: first.order,
                elements,
                span,
                mesh,
            });
        }
        Ok(Mark {
            view: view.map(|v| v as f32),
            scale: (1.0 / world.size) as f32,
            ground,
            solids,
        })
    }
}

fn check(form: &Form) -> Result<(), String> {
    let finite = |name: &str, v: f32| {
        if v.is_finite() && v >= 0.0 {
            Ok(())
        } else {
            Err(format!("form: {name} must be 0 or more, got {v}"))
        }
    };
    finite("depth", form.depth)?;
    finite("bevel", form.bevel)?;
    finite("layer", form.layer)?;
    finite("smooth_angle", form.smooth_angle)?;
    if !(form.tolerance > 0.0 && form.tolerance.is_finite()) {
        return Err(format!(
            "form: tolerance must be above 0, got {}",
            form.tolerance
        ));
    }
    if form.depth <= 0.0 && form.bevel <= 0.0 {
        return Err("form: depth and bevel are both 0, so there is no solid".to_string());
    }
    Ok(())
}

fn element_pieces(
    element: &Element,
    n: usize,
    form: &Form,
    world: &World,
) -> Result<Vec<(Paint, Geometry)>, String> {
    let inset = form.inset.at(n) as f64 / world.size;
    let width = element.stroke_width / world.size;
    let holes = element
        .holes
        .iter()
        .map(|h| h.iter().map(|p| world.point(*p)).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let rule = match element.fill_rule {
        FillRule::NonZero => OverlayFill::NonZero,
        FillRule::EvenOdd => OverlayFill::EvenOdd,
    };
    let closed = element
        .paths
        .iter()
        .filter(|p| p.points.len() > 2)
        .map(|p| p.points.iter().map(|q| world.point(*q)).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let mut out = Vec::new();
    if element.joined {
        let fill = element.fill.clone().unwrap();
        let region = cut(closed.simplify_shape_as::<i64>(rule), &holes);
        let grow = (width * 0.5 - inset).max(width * 0.5 * 0.2);
        if !region.is_empty() {
            out.push((fill, Geometry::Field(pillow(&region, grow, &holes, form))));
        }
        return Ok(out);
    }
    if let Some(fill) = &element.fill {
        let size = extent(&closed);
        let mut region = closed.simplify_shape_as::<i64>(rule);
        if inset > 0.0 && !region.is_empty() {
            let style = OutlineStyle::new(-inset).line_join(LineJoin::Miter(0.05));
            region = region.outline(&style);
        }
        let region = cut(region, &holes);
        if !region.is_empty() {
            let bevel = (form.bevel as f64).min(size * BEVEL_SHARE as f64);
            out.push((fill.clone(), Geometry::Region { region, bevel }));
        }
    }
    if let Some(stroke) = &element.stroke
        && width > 0.0
    {
        let r = (width * 0.5 - inset).max(width * THIN_STROKE as f64);
        let segments = element
            .paths
            .iter()
            .flat_map(|path| {
                let points = path
                    .points
                    .iter()
                    .map(|q| world.point(*q))
                    .collect::<Vec<_>>();
                let mut pairs = points.windows(2).map(|w| [w[0], w[1]]).collect::<Vec<_>>();
                if path.closed && points.len() > 2 {
                    pairs.push([*points.last().unwrap(), points[0]]);
                }
                pairs
            })
            .collect::<Vec<_>>();
        if !segments.is_empty() {
            out.push((
                stroke.clone(),
                Geometry::Field(tube(&segments, r, &holes, form)),
            ));
        }
    }
    Ok(out)
}

fn cut(region: Region, holes: &[Vec<[f64; 2]>]) -> Region {
    if holes.is_empty() || region.is_empty() {
        return region;
    }
    region.overlay_as::<i64>(
        &holes.to_vec(),
        OverlayRule::Difference,
        OverlayFill::NonZero,
    )
}

fn extent(contours: &[Vec<[f64; 2]>]) -> f64 {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for p in contours.iter().flatten() {
        for i in 0..2 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    if !lo[0].is_finite() {
        return 0.0;
    }
    (hi[0] - lo[0]).max(hi[1] - lo[1])
}

fn lift(mesh: &Mesh, mul: f32, base: f32) -> Mesh {
    if mul == 1.0 && base == 0.0 {
        return mesh.clone();
    }
    let positions = mesh
        .positions
        .iter()
        .map(|p| [p[0], p[1], p[2] * mul + base])
        .collect();
    let normals = mesh
        .normals
        .iter()
        .map(|n| normalize([n[0], n[1], n[2] / mul]))
        .collect();
    Mesh::new(
        positions,
        normals,
        Vec::new(),
        mesh.uvs.clone(),
        mesh.indices.clone(),
    )
}

#[derive(Clone, Copy, Debug)]
struct Step {
    o: f32,
    z: f32,
    below: [f32; 2],
    above: [f32; 2],
    smooth: Option<[f32; 2]>,
}

fn profile(depth: f32, bevel: f32, form: &Form) -> Vec<Step> {
    let mut top: Vec<(f32, f32, Option<[f32; 2]>)> = Vec::new();
    if bevel > 1e-9 {
        let steps = match form.profile {
            Profile::Round => form.segments + 1,
            Profile::Chamfer => 1,
        };
        for k in 0..=steps {
            let g = std::f32::consts::FRAC_PI_2 * k as f32 / steps as f32;
            let (o, z) = if k == 0 {
                (bevel, depth)
            } else if k == steps {
                (0.0, depth + bevel)
            } else {
                (bevel * g.cos(), depth + bevel * g.sin())
            };
            let arc = (form.profile == Profile::Round).then_some(if k == steps {
                [0.0, 1.0]
            } else {
                [g.cos(), g.sin()]
            });
            top.push((o, z, arc));
        }
    } else {
        top.push((0.0, depth, None));
    }
    let mut points: Vec<(f32, f32, Option<[f32; 2]>)> = top
        .iter()
        .rev()
        .map(|(o, z, arc)| (*o, -*z, arc.map(|a| [a[0], -a[1]])))
        .collect();
    let skip = usize::from(depth <= 1e-9);
    points.extend(top.iter().skip(skip).copied());
    let facets = points
        .windows(2)
        .map(|w| {
            let (to, tz) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
            let len = (to * to + tz * tz).sqrt().max(1e-12);
            [tz / len, -to / len]
        })
        .collect::<Vec<_>>();
    let limit = form.smooth_angle.to_radians() + 1e-4;
    points
        .iter()
        .enumerate()
        .map(|(j, &(o, z, arc))| {
            let below = if j == 0 { [0.0, -1.0] } else { facets[j - 1] };
            let above = if j == points.len() - 1 {
                [0.0, 1.0]
            } else {
                facets[j]
            };
            let angle = (below[0] * above[0] + below[1] * above[1])
                .clamp(-1.0, 1.0)
                .acos();
            let smooth = (angle <= limit).then(|| {
                arc.unwrap_or_else(|| {
                    let s = [below[0] + above[0], below[1] + above[1]];
                    let len = (s[0] * s[0] + s[1] * s[1]).sqrt().max(1e-12);
                    [s[0] / len, s[1] / len]
                })
            });
            Step {
                o,
                z,
                below,
                above,
                smooth,
            }
        })
        .collect()
}

struct Column {
    index: usize,
    point: [f64; 2],
    miter: [f64; 2],
    before: [f64; 2],
    after: [f64; 2],
    smooth: bool,
}

fn columns(contour: &[[f64; 2]], limit: f64) -> Vec<Column> {
    let mut points = contour.to_vec();
    points.dedup_by(|a, b| (a[0] - b[0]).abs() <= 1e-12 && (a[1] - b[1]).abs() <= 1e-12);
    while points.len() > 1 {
        let (a, b) = (points[0], *points.last().unwrap());
        if (a[0] - b[0]).abs() <= 1e-12 && (a[1] - b[1]).abs() <= 1e-12 {
            points.pop();
        } else {
            break;
        }
    }
    let count = points.len();
    if count < 3 {
        return Vec::new();
    }
    let normal = |a: [f64; 2], b: [f64; 2]| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt().max(1e-300);
        [dy / len, -dx / len]
    };
    (0..count)
        .map(|i| {
            let prev = points[(i + count - 1) % count];
            let here = points[i];
            let next = points[(i + 1) % count];
            let before = normal(prev, here);
            let after = normal(here, next);
            let cos = before[0] * after[0] + before[1] * after[1];
            let sum = [before[0] + after[0], before[1] + after[1]];
            let sum_len = (sum[0] * sum[0] + sum[1] * sum[1]).sqrt();
            let miter = if 1.0 + cos > 0.125 {
                [sum[0] / (1.0 + cos), sum[1] / (1.0 + cos)]
            } else if sum_len > 1e-9 {
                [sum[0] / sum_len * 4.0, sum[1] / sum_len * 4.0]
            } else {
                [before[0] * 4.0, before[1] * 4.0]
            };
            Column {
                index: i,
                point: here,
                miter,
                before,
                after,
                smooth: cos.clamp(-1.0, 1.0).acos() <= limit,
            }
        })
        .collect()
}

struct Builder<'a> {
    world: &'a World,
    steps: &'a [Step],
    mul: f32,
    base: f32,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    made: BTreeMap<(usize, usize, u8, usize, u8), u32>,
}

impl Builder<'_> {
    fn vertex(&mut self, contour: usize, column: &Column, side: u8, j: usize, half: u8) -> u32 {
        let step = self.steps[j];
        let (normal2, half) = match step.smooth {
            Some(n) => (n, 0),
            None => (if half == 0 { step.below } else { step.above }, half),
        };
        let flat = normal2[0].abs() <= 1e-6;
        let side = if column.smooth || flat { 0 } else { side };
        let key = (contour, column.index, side, j, half);
        if let Some(found) = self.made.get(&key) {
            return *found;
        }
        let horizontal = if column.smooth {
            let s = [
                column.before[0] + column.after[0],
                column.before[1] + column.after[1],
            ];
            let len = (s[0] * s[0] + s[1] * s[1]).sqrt().max(1e-300);
            [s[0] / len, s[1] / len]
        } else if side == 0 {
            column.before
        } else {
            column.after
        };
        let x = (column.point[0] + column.miter[0] * step.o as f64) as f32;
        let y = (column.point[1] + column.miter[1] * step.o as f64) as f32;
        let z = step.z * self.mul + self.base;
        let normal = normalize([
            horizontal[0] as f32 * normal2[0],
            horizontal[1] as f32 * normal2[0],
            normal2[1] / self.mul,
        ]);
        let index = self.positions.len() as u32;
        self.positions.push([x, y, z]);
        self.normals.push(normal);
        self.uvs.push(self.world.uv(x, y));
        self.made.insert(key, index);
        index
    }

    fn tri(&mut self, a: u32, b: u32, c: u32) {
        if a == b || b == c || a == c {
            return;
        }
        self.indices.extend([a, b, c]);
    }
}

fn extrude(region: &Region, form: &Form, bevel: f64, mul: f32, base: f32, world: &World) -> Mesh {
    let steps = profile(form.depth, bevel as f32, form);
    let limit = (form.smooth_angle as f64).to_radians() + 1e-4;
    let mut builder = Builder {
        world,
        steps: &steps,
        mul,
        base,
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        indices: Vec::new(),
        made: BTreeMap::new(),
    };
    let last = steps.len() - 1;
    let mut contour_id = 0usize;
    for shape in region {
        let mut caps: BTreeMap<(u32, u32), (usize, usize)> = BTreeMap::new();
        let mut rings: Vec<(usize, Vec<Column>)> = Vec::new();
        for contour in shape {
            let cols = columns(contour, limit);
            if cols.is_empty() {
                continue;
            }
            let id = contour_id;
            contour_id += 1;
            let count = cols.len();
            for i in 0..count {
                let next = (i + 1) % count;
                for j in 0..last {
                    let a = builder.vertex(id, &cols[i], 1, j, 1);
                    let b = builder.vertex(id, &cols[next], 0, j, 1);
                    let c = builder.vertex(id, &cols[next], 0, j + 1, 0);
                    let d = builder.vertex(id, &cols[i], 1, j + 1, 0);
                    builder.tri(a, b, c);
                    builder.tri(a, c, d);
                }
            }
            for (i, col) in cols.iter().enumerate() {
                let key = (
                    (col.point[0] as f32).to_bits(),
                    (col.point[1] as f32).to_bits(),
                );
                caps.entry(key).or_insert((id, i));
            }
            rings.push((id, cols));
        }
        if rings.is_empty() {
            continue;
        }
        let mut path = LyonPath::builder();
        for (_, cols) in &rings {
            path.begin(point(cols[0].point[0] as f32, cols[0].point[1] as f32));
            for col in &cols[1..] {
                path.line_to(point(col.point[0] as f32, col.point[1] as f32));
            }
            path.end(true);
        }
        let path = path.build();
        let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
        let options = FillOptions::default().with_fill_rule(lyon_tessellation::FillRule::EvenOdd);
        let result = FillTessellator::new().tessellate_path(
            &path,
            &options,
            &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| v.position().to_array()),
        );
        if result.is_err() {
            continue;
        }
        let lookup = |p: [f32; 2]| caps.get(&(p[0].to_bits(), p[1].to_bits())).copied();
        for (j, half, up) in [(last, 1u8, true), (0usize, 0u8, false)] {
            let mut map: Vec<u32> = Vec::with_capacity(buffers.vertices.len());
            for v in &buffers.vertices {
                let index = match lookup(*v) {
                    Some((id, i)) => {
                        let cols = &rings.iter().find(|(c, _)| *c == id).unwrap().1;
                        builder.vertex(id, &cols[i], 0, j, half)
                    }
                    None => {
                        let z = steps[j].z * mul + base;
                        let index = builder.positions.len() as u32;
                        builder.positions.push([v[0], v[1], z]);
                        builder
                            .normals
                            .push([0.0, 0.0, if up { 1.0 } else { -1.0 }]);
                        builder.uvs.push(world.uv(v[0], v[1]));
                        index
                    }
                };
                map.push(index);
            }
            for tri in buffers.indices.chunks_exact(3) {
                let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| buffers.vertices[i as usize]);
                let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
                let (i0, i1, i2) = (
                    map[tri[0] as usize],
                    map[tri[1] as usize],
                    map[tri[2] as usize],
                );
                if (area > 0.0) == up {
                    builder.tri(i0, i1, i2);
                } else {
                    builder.tri(i0, i2, i1);
                }
            }
        }
    }
    Mesh::new(
        builder.positions,
        builder.normals,
        Vec::new(),
        builder.uvs,
        builder.indices,
    )
}

struct Raster {
    origin: [f64; 2],
    cell: f64,
    width: usize,
    height: usize,
    values: Vec<f32>,
}

const TILE: usize = 16;

impl Raster {
    fn new(lo: [f64; 2], hi: [f64; 2], cell: f64) -> Raster {
        let width = (((hi[0] - lo[0]) / cell).ceil() as usize + 2).max(2);
        let height = (((hi[1] - lo[1]) / cell).ceil() as usize + 2).max(2);
        Raster {
            origin: lo,
            cell,
            width,
            height,
            values: vec![0.0; width * height],
        }
    }

    fn at(&self, x: usize, y: usize) -> [f64; 2] {
        [
            self.origin[0] + x as f64 * self.cell,
            self.origin[1] + y as f64 * self.cell,
        ]
    }

    fn distances(&mut self, segments: &[[[f64; 2]; 2]], band: f64) {
        let reach = self.cell * TILE as f64 * std::f64::consts::SQRT_2 * 0.5 + band;
        for ty in (0..self.height).step_by(TILE) {
            for tx in (0..self.width).step_by(TILE) {
                let center = self.at(tx + TILE / 2, ty + TILE / 2);
                let near = segments
                    .iter()
                    .filter(|s| box_distance(s, center) <= reach)
                    .collect::<Vec<_>>();
                for y in ty..(ty + TILE).min(self.height) {
                    for x in tx..(tx + TILE).min(self.width) {
                        let p = self.at(x, y);
                        let d = near
                            .iter()
                            .map(|s| segment_distance(p, s[0], s[1]))
                            .fold(band, f64::min);
                        self.values[y * self.width + x] = d as f32;
                    }
                }
            }
        }
    }

    fn sign_inside(&mut self, segments: &[[[f64; 2]; 2]]) {
        let mut crossings = Vec::new();
        for y in 0..self.height {
            let row = self.origin[1] + y as f64 * self.cell;
            crossings.clear();
            for [a, b] in segments {
                if (a[1] <= row) != (b[1] <= row) {
                    crossings.push(a[0] + (row - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
                }
            }
            crossings.sort_by(f64::total_cmp);
            for pair in crossings.chunks_exact(2) {
                let from = ((pair[0] - self.origin[0]) / self.cell).ceil().max(0.0) as usize;
                let to = ((pair[1] - self.origin[0]) / self.cell).floor();
                if to < 0.0 {
                    continue;
                }
                let to = (to as usize).min(self.width - 1);
                for x in from..=to {
                    let v = &mut self.values[y * self.width + x];
                    *v = -*v;
                }
            }
        }
    }

    fn sample(&self, x: f32, y: f32) -> f32 {
        let fx = ((x as f64 - self.origin[0]) / self.cell).clamp(0.0, (self.width - 1) as f64);
        let fy = ((y as f64 - self.origin[1]) / self.cell).clamp(0.0, (self.height - 1) as f64);
        let x0 = (fx.floor() as usize).min(self.width - 2);
        let y0 = (fy.floor() as usize).min(self.height - 2);
        let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let v = |x: usize, y: usize| self.values[y * self.width + x];
        let top = v(x0, y0) * (1.0 - tx) + v(x0 + 1, y0) * tx;
        let bottom = v(x0, y0 + 1) * (1.0 - tx) + v(x0 + 1, y0 + 1) * tx;
        top * (1.0 - ty) + bottom * ty
    }
}

fn box_distance(s: &[[f64; 2]; 2], p: [f64; 2]) -> f64 {
    let lo = [s[0][0].min(s[1][0]), s[0][1].min(s[1][1])];
    let hi = [s[0][0].max(s[1][0]), s[0][1].max(s[1][1])];
    let dx = (lo[0] - p[0]).max(p[0] - hi[0]).max(0.0);
    let dy = (lo[1] - p[1]).max(p[1] - hi[1]).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let l2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if l2 <= 1e-300 {
        0.0
    } else {
        ((ap[0] * ab[0] + ap[1] * ab[1]) / l2).clamp(0.0, 1.0)
    };
    let d = [ap[0] - ab[0] * t, ap[1] - ab[1] * t];
    (d[0] * d[0] + d[1] * d[1]).sqrt()
}

fn edges(region: &[Vec<[f64; 2]>]) -> Vec<[[f64; 2]; 2]> {
    region
        .iter()
        .filter(|c| c.len() > 2)
        .flat_map(|c| (0..c.len()).map(move |i| [c[i], c[(i + 1) % c.len()]]))
        .collect()
}

fn bounds2(segments: &[[[f64; 2]; 2]]) -> ([f64; 2], [f64; 2]) {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for p in segments.iter().flatten() {
        for i in 0..2 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    (lo, hi)
}

fn signed(segments: &[[[f64; 2]; 2]], lo: [f64; 2], hi: [f64; 2], cell: f64, band: f64) -> Raster {
    let mut raster = Raster::new(lo, hi, cell);
    raster.distances(segments, band);
    raster.sign_inside(segments);
    raster
}

fn hole_raster(
    holes: &[Vec<[f64; 2]>],
    lo: [f64; 2],
    hi: [f64; 2],
    cell: f64,
    band: f64,
) -> Option<Raster> {
    if holes.is_empty() {
        return None;
    }
    let region = holes.simplify_shape_as::<i64>(OverlayFill::NonZero);
    let contours = region.into_iter().flatten().collect::<Vec<_>>();
    Some(signed(&edges(&contours), lo, hi, cell, band))
}

fn tube(segments: &[[[f64; 2]; 2]], r: f64, holes: &[Vec<[f64; 2]>], form: &Form) -> Mesh {
    let voxel = (r / 10.0).max(1.0 / VOXEL_SPAN as f64);
    let (lo, hi) = bounds2(segments);
    let pad = r + 4.0 * voxel;
    let lo = [lo[0] - pad, lo[1] - pad];
    let hi = [hi[0] + pad, hi[1] + pad];
    let cell = voxel * 0.5;
    let band = r + 6.0 * voxel;
    let mut raster = Raster::new(lo, hi, cell);
    raster.distances(segments, band);
    let cut = hole_raster(holes, lo, hi, cell, band);
    let r = r as f32;
    let field = |p: [f32; 3]| {
        let u = raster.sample(p[0], p[1]);
        let d = (u * u + p[2] * p[2]).sqrt() - r;
        match &cut {
            Some(h) => d.max(-h.sample(p[0], p[1])),
            None => d,
        }
    };
    let low = [
        (lo[0] + 3.0 * voxel) as f32,
        (lo[1] + 3.0 * voxel) as f32,
        -r,
    ];
    let high = [
        (hi[0] - 3.0 * voxel) as f32,
        (hi[1] - 3.0 * voxel) as f32,
        r,
    ];
    Surface::new(voxel as f32, form.relax).mesh(&field, low, high)
}

fn pillow(region: &Region, grow: f64, holes: &[Vec<[f64; 2]>], form: &Form) -> Mesh {
    let voxel = (grow / 10.0).max(1.0 / VOXEL_SPAN as f64);
    let contours = region.iter().flatten().cloned().collect::<Vec<_>>();
    let segments = edges(&contours);
    let (lo, hi) = bounds2(&segments);
    let pad = grow + 4.0 * voxel;
    let lo = [lo[0] - pad, lo[1] - pad];
    let hi = [hi[0] + pad, hi[1] + pad];
    let cell = voxel * 0.5;
    let band = grow + 6.0 * voxel;
    let raster = signed(&segments, lo, hi, cell, band);
    let cut = hole_raster(holes, lo, hi, cell, band);
    let core = (grow * CORE as f64) as f32;
    let reach = grow as f32 - core;
    let field = |p: [f32; 3]| {
        let w = [raster.sample(p[0], p[1]), p[2].abs() - core];
        let d = w[0].max(w[1]).min(0.0) + (w[0].max(0.0).powi(2) + w[1].max(0.0).powi(2)).sqrt()
            - reach;
        match &cut {
            Some(h) => d.max(-h.sample(p[0], p[1])),
            None => d,
        }
    };
    let g = grow as f32;
    let low = [
        (lo[0] + 3.0 * voxel) as f32,
        (lo[1] + 3.0 * voxel) as f32,
        -g,
    ];
    let high = [
        (hi[0] - 3.0 * voxel) as f32,
        (hi[1] - 3.0 * voxel) as f32,
        g,
    ];
    Surface::new(voxel as f32, form.relax).mesh(&field, low, high)
}
