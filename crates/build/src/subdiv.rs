use glam::{DVec2, DVec3};
use pfx_mesh::{Mesh, Slot};
use std::collections::{BTreeMap, HashMap};
use std::f64::consts::PI;
use std::iter::Sum;
use std::ops::{Add, Div, Mul};

pub const INFINITE: f64 = 10.0;
pub const MAX_LEVELS: u32 = 4;
pub const MATERIAL: &str = "cage";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cage {
    pub points: Vec<[f64; 3]>,
    pub faces: Vec<Vec<u32>>,
    pub creases: Vec<Crease>,
    pub corners: Vec<Corner>,
    pub uvs: Option<Vec<Vec<[f64; 2]>>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crease {
    pub edge: [u32; 2],
    pub sharpness: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corner {
    pub point: u32,
    pub sharpness: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Boundary {
    #[default]
    Edges,
    EdgesAndCorners,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Creasing {
    #[default]
    Chaikin,
    Uniform,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UvInterpolation {
    #[default]
    Smooth,
    Linear,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub boundary: Boundary,
    pub creasing: Creasing,
    pub uvs: UvInterpolation,
}

pub fn subdivide(cage: &Cage, levels: u32, options: &Options) -> Result<Mesh, String> {
    if levels > MAX_LEVELS {
        return Err(format!(
            "subdivide: expected a whole number from 0 to {MAX_LEVELS}"
        ));
    }
    let mut level = Level::from_cage(cage, options)?;
    for _ in 0..levels {
        let topo = topology(&level).map_err(|_| "faces: an edge joins more than two faces")?;
        level = refine(&level, &topo, options);
    }
    let topo = topology(&level).map_err(|_| "faces: an edge joins more than two faces")?;
    Ok(surface(&level, &topo, options, levels > 0))
}

pub(crate) fn key(a: u32, b: u32) -> (u32, u32) {
    if a < b { (a, b) } else { (b, a) }
}

pub(crate) fn decrement(s: f64) -> f64 {
    if s <= 0.0 {
        0.0
    } else if s >= INFINITE {
        INFINITE
    } else if s > 1.0 {
        s - 1.0
    } else {
        0.0
    }
}

pub(crate) struct Level {
    pub(crate) points: Vec<DVec3>,
    pub(crate) starts: Vec<usize>,
    pub(crate) verts: Vec<u32>,
    pub(crate) sharp: HashMap<(u32, u32), f64>,
    pub(crate) corner: Vec<f64>,
    pub(crate) values: Vec<DVec2>,
    pub(crate) corner_values: Vec<u32>,
}

impl Level {
    pub(crate) fn new(
        points: Vec<DVec3>,
        starts: Vec<usize>,
        verts: Vec<u32>,
        sharp: HashMap<(u32, u32), f64>,
    ) -> Level {
        Level {
            points,
            starts,
            verts,
            sharp,
            corner: Vec::new(),
            values: Vec::new(),
            corner_values: Vec::new(),
        }
    }

    pub(crate) fn faces(&self) -> usize {
        self.starts.len()
    }

    pub(crate) fn face(&self, f: usize) -> &[u32] {
        let end = self.starts.get(f + 1).copied().unwrap_or(self.verts.len());
        &self.verts[self.starts[f]..end]
    }

    fn vertex_sharp(&self, v: usize) -> f64 {
        self.corner.get(v).copied().unwrap_or(0.0)
    }

    fn has_uvs(&self) -> bool {
        !self.corner_values.is_empty()
    }

    fn value(&self, f: usize, k: usize) -> u32 {
        let n = self.face(f).len();
        self.corner_values[self.starts[f] + k % n]
    }

    fn from_cage(cage: &Cage, options: &Options) -> Result<Level, String> {
        let count = cage.points.len();
        if cage.faces.is_empty() {
            return Err("faces: expected at least one face".into());
        }
        for (i, p) in cage.points.iter().enumerate() {
            if !p.iter().all(|c| c.is_finite()) {
                return Err(format!("points[{i}]: expected [x, y, z], finite numbers"));
            }
        }
        let mut used = vec![false; count];
        let mut directed: BTreeMap<(u32, u32), usize> = BTreeMap::new();
        let mut undirected: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
        for (f, face) in cage.faces.iter().enumerate() {
            if face.len() < 3 {
                return Err(format!("faces[{f}]: expected at least 3 point indices"));
            }
            for (k, &i) in face.iter().enumerate() {
                if i as usize >= count {
                    return Err(format!(
                        "faces[{f}]: point {i} is out of range; the cage has {count} points"
                    ));
                }
                if face[..k].contains(&i) {
                    return Err(format!("faces[{f}]: point {i} appears twice"));
                }
                used[i as usize] = true;
            }
            let n = face.len();
            for k in 0..n {
                let (a, b) = (face[k], face[(k + 1) % n]);
                let joined = undirected.entry(key(a, b)).or_default();
                if joined.len() == 2 {
                    return Err(format!(
                        "faces[{f}]: the edge {a}-{b} already joins faces[{}] and faces[{}]; an edge joins at most two faces",
                        joined[0], joined[1]
                    ));
                }
                joined.push(f);
                if let Some(&other) = directed.get(&(a, b)) {
                    return Err(format!(
                        "faces[{f}]: the edge {a}-{b} runs the same way in faces[{other}]; every face must wind the same way round"
                    ));
                }
                directed.insert((a, b), f);
            }
        }
        if let Some(i) = used.iter().position(|u| !u) {
            return Err(format!("points[{i}]: no face uses it"));
        }
        let mut sharp = HashMap::new();
        for (c, crease) in cage.creases.iter().enumerate() {
            let [a, b] = crease.edge;
            if !undirected.contains_key(&key(a, b)) {
                return Err(format!(
                    "creases[{c}].edge: {a}-{b} is not an edge of any face"
                ));
            }
            if !(crease.sharpness.is_finite() && (0.0..=INFINITE).contains(&crease.sharpness)) {
                return Err(format!(
                    "creases[{c}].sharpness: expected a number from 0 to {INFINITE}"
                ));
            }
            if sharp.insert(key(a, b), crease.sharpness).is_some() {
                return Err(format!("creases[{c}].edge: {a}-{b} is creased twice"));
            }
        }
        for (e, joined) in &undirected {
            if joined.len() == 1 {
                sharp.insert(*e, INFINITE);
            }
        }
        sharp.retain(|_, s| *s > 0.0);
        let mut corner = vec![0.0; count];
        for (c, at) in cage.corners.iter().enumerate() {
            if at.point as usize >= count {
                return Err(format!(
                    "corners[{c}].point: point {} is out of range; the cage has {count} points",
                    at.point
                ));
            }
            if !(at.sharpness.is_finite() && (0.0..=INFINITE).contains(&at.sharpness)) {
                return Err(format!(
                    "corners[{c}].sharpness: expected a number from 0 to {INFINITE}"
                ));
            }
            corner[at.point as usize] = at.sharpness;
        }
        let mut starts = Vec::with_capacity(cage.faces.len());
        let mut verts = Vec::new();
        for face in &cage.faces {
            starts.push(verts.len());
            verts.extend_from_slice(face);
        }
        let points = cage.points.iter().map(|&p| DVec3::from_array(p)).collect();
        let mut level = Level::new(points, starts, verts, sharp);
        let topo = topology(&level).map_err(|_| "faces: an edge joins more than two faces")?;
        for (v, slot) in corner.iter_mut().enumerate() {
            if ring(&level, &topo, v).is_none() {
                return Err(format!(
                    "points[{v}]: its faces make more than one fan; a point joins one fan of faces"
                ));
            }
            if options.boundary == Boundary::EdgesAndCorners
                && topo.vertex_faces[v].len() == 1
                && topo.vertex_edges[v].len() == 2
            {
                *slot = INFINITE;
            }
        }
        level.corner = corner;
        if let Some(uvs) = &cage.uvs {
            if uvs.len() != cage.faces.len() {
                return Err(format!(
                    "uvs: expected one list per face ({}), got {}",
                    cage.faces.len(),
                    uvs.len()
                ));
            }
            let mut found: Vec<Vec<(u64, u64, u32)>> = vec![Vec::new(); count];
            for (f, (face, list)) in cage.faces.iter().zip(uvs).enumerate() {
                if list.len() != face.len() {
                    return Err(format!(
                        "uvs[{f}]: expected {} [u, v] pairs, one per point of faces[{f}]",
                        face.len()
                    ));
                }
                for (&v, uv) in face.iter().zip(list) {
                    if !uv.iter().all(|c| c.is_finite()) {
                        return Err(format!("uvs[{f}]: expected [u, v] pairs of finite numbers"));
                    }
                    let bits = (uv[0].to_bits(), uv[1].to_bits());
                    let seen = &mut found[v as usize];
                    let id = match seen.iter().find(|s| (s.0, s.1) == bits) {
                        Some(s) => s.2,
                        None => {
                            let id = level.values.len() as u32;
                            level.values.push(DVec2::new(uv[0], uv[1]));
                            seen.push((bits.0, bits.1, id));
                            id
                        }
                    };
                    level.corner_values.push(id);
                }
            }
        }
        Ok(level)
    }
}

pub(crate) struct Topology {
    pub(crate) edges: Vec<[u32; 2]>,
    pub(crate) halves: Vec<[(u32, u32); 2]>,
    pub(crate) sharp: Vec<f64>,
    pub(crate) face_edges: Vec<u32>,
    pub(crate) vertex_edges: Vec<Vec<u32>>,
    pub(crate) vertex_faces: Vec<Vec<u32>>,
}

impl Topology {
    pub(crate) fn open(&self, id: usize) -> bool {
        self.halves[id][1].0 == u32::MAX
    }
}

pub(crate) struct Crowded;

pub(crate) fn topology(cage: &Level) -> Result<Topology, Crowded> {
    let mut index: HashMap<(u32, u32), u32> = HashMap::with_capacity(cage.verts.len());
    let mut edges = Vec::with_capacity(cage.verts.len() / 2);
    let mut halves: Vec<[(u32, u32); 2]> = Vec::with_capacity(cage.verts.len() / 2);
    let mut face_edges = vec![0u32; cage.verts.len()];
    let mut vertex_faces = vec![Vec::new(); cage.points.len()];
    for f in 0..cage.faces() {
        let face = cage.face(f);
        let n = face.len();
        for k in 0..n {
            let (a, b) = (face[k], face[(k + 1) % n]);
            vertex_faces[a as usize].push(f as u32);
            let id = *index.entry(key(a, b)).or_insert_with(|| {
                edges.push([a, b]);
                halves.push([(f as u32, k as u32), (u32::MAX, u32::MAX)]);
                (edges.len() - 1) as u32
            });
            if halves[id as usize][0] != (f as u32, k as u32) {
                if halves[id as usize][1].0 != u32::MAX {
                    return Err(Crowded);
                }
                halves[id as usize][1] = (f as u32, k as u32);
            }
            face_edges[cage.starts[f] + k] = id;
        }
    }
    let mut vertex_edges = vec![Vec::new(); cage.points.len()];
    for (i, e) in edges.iter().enumerate() {
        vertex_edges[e[0] as usize].push(i as u32);
        vertex_edges[e[1] as usize].push(i as u32);
    }
    let sharp = edges
        .iter()
        .map(|e| cage.sharp.get(&key(e[0], e[1])).copied().unwrap_or(0.0))
        .collect();
    Ok(Topology {
        edges,
        halves,
        sharp,
        face_edges,
        vertex_edges,
        vertex_faces,
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Rule {
    Smooth,
    Crease,
    Corner,
}

fn rule(count: usize) -> Rule {
    match count {
        0 | 1 => Rule::Smooth,
        2 => Rule::Crease,
        _ => Rule::Corner,
    }
}

fn vertex_rule(vertex: f64, count: usize) -> Rule {
    if vertex > 0.0 {
        Rule::Corner
    } else {
        rule(count)
    }
}

fn child_sharpness(topo: &Topology, creasing: Creasing) -> Vec<[f64; 2]> {
    match creasing {
        Creasing::Uniform => topo
            .sharp
            .iter()
            .map(|&s| [decrement(s), decrement(s)])
            .collect(),
        Creasing::Chaikin => {
            let around: Vec<(f64, usize)> = topo
                .vertex_edges
                .iter()
                .map(|edges| {
                    edges
                        .iter()
                        .map(|&id| topo.sharp[id as usize])
                        .filter(|&s| s > 0.0 && s < INFINITE)
                        .fold((0.0, 0), |(sum, n), s| (sum + s, n + 1))
                })
                .collect();
            let at = |s: f64, v: u32| {
                let (sum, count) = around[v as usize];
                if topo.vertex_edges[v as usize].len() < 2 {
                    return decrement(s);
                }
                if s <= 0.0 {
                    return 0.0;
                }
                if s >= INFINITE {
                    return INFINITE;
                }
                let mut s = s;
                if count > 1 {
                    let others = (sum - s) / (count - 1) as f64;
                    s = 0.75 * s + 0.25 * others;
                }
                s -= 1.0;
                if s > 0.0 { s } else { 0.0 }
            };
            topo.edges
                .iter()
                .zip(&topo.sharp)
                .map(|(e, &s)| [at(s, e[0]), at(s, e[1])])
                .collect()
        }
    }
}

trait Point:
    Copy + Add<Output = Self> + Mul<f64, Output = Self> + Div<f64, Output = Self> + Sum
{
}

impl Point for DVec3 {}
impl Point for DVec2 {}

struct VertexRules {
    parent: Vec<u32>,
    child: Vec<u32>,
    pr: Rule,
    cr: Rule,
    weight: f64,
}

fn vertex_rules(
    cage: &Level,
    topo: &Topology,
    child_sharp: &[[f64; 2]],
    creasing: Creasing,
    i: usize,
) -> VertexRules {
    let around = &topo.vertex_edges[i];
    let end = |id: u32| usize::from(topo.edges[id as usize][0] as usize != i);
    let parent: Vec<u32> = around
        .iter()
        .copied()
        .filter(|&id| topo.sharp[id as usize] > 0.0)
        .collect();
    let child: Vec<u32> = parent
        .iter()
        .copied()
        .filter(|&id| child_sharp[id as usize][end(id)] > 0.0)
        .collect();
    let pv = cage.vertex_sharp(i);
    let cv = decrement(pv);
    let (pr, cr) = (vertex_rule(pv, parent.len()), vertex_rule(cv, child.len()));
    let weight = if pr == cr {
        0.0
    } else {
        let mut fractional: Vec<f64> = Vec::new();
        if pv > 0.0 && cv <= 0.0 {
            fractional.push(pv);
        }
        fractional.extend(
            parent
                .iter()
                .filter(|&&id| match creasing {
                    Creasing::Uniform => topo.sharp[id as usize] <= 1.0,
                    Creasing::Chaikin => child_sharp[id as usize][end(id)] <= 0.0,
                })
                .map(|&id| topo.sharp[id as usize]),
        );
        if fractional.is_empty() {
            0.0
        } else {
            (fractional.iter().sum::<f64>() / fractional.len() as f64).min(1.0)
        }
    };
    VertexRules {
        parent,
        child,
        pr,
        cr,
        weight,
    }
}

fn vertex_point<P: Point>(
    rules: &VertexRules,
    p: P,
    edges: &[u32],
    faces: &[u32],
    other: impl Fn(u32) -> P,
    centre: impl Fn(u32) -> P,
) -> P {
    let mask = |r: Rule, sharp: &[u32]| match r {
        Rule::Corner => p,
        Rule::Crease => (p * 6.0 + other(sharp[0]) + other(sharp[1])) / 8.0,
        Rule::Smooth => {
            let n = edges.len() as f64;
            let e: P = edges.iter().map(|&id| other(id)).sum();
            let fs: P = faces.iter().map(|&f| centre(f)).sum();
            p * ((n - 2.0) / n) + (e + fs) / (n * n)
        }
    };
    if rules.pr == rules.cr {
        mask(rules.pr, &rules.parent)
    } else {
        let w = rules.weight;
        mask(rules.pr, &rules.parent) * w + mask(rules.cr, &rules.child) * (1.0 - w)
    }
}

fn edge_point<P: Point>(s: f64, child: [f64; 2], a: P, b: P, faces: impl Fn() -> (P, P)) -> P {
    let crease = (a + b) * 0.5;
    let smooth = || {
        let (f0, f1) = faces();
        (a + b + f0 + f1) * 0.25
    };
    if s <= 0.0 {
        smooth()
    } else if child[0] > 0.0 && child[1] > 0.0 {
        crease
    } else {
        crease * s + smooth() * (1.0 - s)
    }
}

pub(crate) fn refine(cage: &Level, topo: &Topology, options: &Options) -> Level {
    let v = cage.points.len();
    let e = topo.edges.len();
    let child_sharp = child_sharpness(topo, options.creasing);
    let faces: Vec<DVec3> = (0..cage.faces())
        .map(|f| {
            let face = cage.face(f);
            face.iter().map(|&i| cage.points[i as usize]).sum::<DVec3>() / face.len() as f64
        })
        .collect();
    let rules: Vec<VertexRules> = (0..v)
        .map(|i| vertex_rules(cage, topo, &child_sharp, options.creasing, i))
        .collect();
    let mut points = Vec::with_capacity(v + e + faces.len());
    for (i, rule) in rules.iter().enumerate() {
        let other = |id: u32| {
            let edge = topo.edges[id as usize];
            cage.points[if edge[0] as usize == i {
                edge[1]
            } else {
                edge[0]
            } as usize]
        };
        points.push(vertex_point(
            rule,
            cage.points[i],
            &topo.vertex_edges[i],
            &topo.vertex_faces[i],
            other,
            |f| faces[f as usize],
        ));
    }
    for (id, edge) in topo.edges.iter().enumerate() {
        let a = cage.points[edge[0] as usize];
        let b = cage.points[edge[1] as usize];
        let [h0, h1] = topo.halves[id];
        points.push(edge_point(topo.sharp[id], child_sharp[id], a, b, || {
            (faces[h0.0 as usize], faces[h1.0 as usize])
        }));
    }
    points.extend(&faces);
    let mut starts = Vec::with_capacity(cage.verts.len());
    let mut verts = Vec::with_capacity(cage.verts.len() * 4);
    for f in 0..cage.faces() {
        let face = cage.face(f);
        let n = face.len();
        let s = cage.starts[f];
        let centre = (v + e + f) as u32;
        for (k, &corner) in face.iter().enumerate() {
            starts.push(verts.len());
            verts.extend([
                corner,
                v as u32 + topo.face_edges[s + k],
                centre,
                v as u32 + topo.face_edges[s + (k + n - 1) % n],
            ]);
        }
    }
    let mut sharp = HashMap::new();
    for (id, edge) in topo.edges.iter().enumerate() {
        let [d0, d1] = child_sharp[id];
        let mid = (v + id) as u32;
        if d0 > 0.0 {
            sharp.insert(key(edge[0], mid), d0);
        }
        if d1 > 0.0 {
            sharp.insert(key(mid, edge[1]), d1);
        }
    }
    let mut out = Level::new(points, starts, verts, sharp);
    if !cage.corner.is_empty() {
        out.corner = cage.corner.iter().map(|&s| decrement(s)).collect();
        out.corner.resize(v + e + cage.faces(), 0.0);
    }
    if cage.has_uvs() {
        refine_values(cage, topo, options, &child_sharp, &rules, &mut out);
    }
    out
}

struct Ring {
    corners: Vec<(u32, u32)>,
    edges: Vec<u32>,
}

impl Ring {
    fn boundary(&self) -> bool {
        self.edges.len() > self.corners.len()
    }
}

fn ring(cage: &Level, topo: &Topology, v: usize) -> Option<Ring> {
    let corner_of = |f: u32| -> u32 {
        cage.face(f as usize)
            .iter()
            .position(|&x| x as usize == v)
            .unwrap_or(0) as u32
    };
    let all = &topo.vertex_faces[v];
    let leading = |(f, k): (u32, u32)| topo.face_edges[cage.starts[f as usize] + k as usize];
    let trailing = |(f, k): (u32, u32)| {
        let n = cage.face(f as usize).len() as u32;
        topo.face_edges[cage.starts[f as usize] + ((k + n - 1) % n) as usize]
    };
    let first = all
        .iter()
        .map(|&f| (f, corner_of(f)))
        .find(|&c| {
            let id = leading(c) as usize;
            topo.open(id)
        })
        .or_else(|| all.first().map(|&f| (f, corner_of(f))))?;
    let mut corners = vec![first];
    let mut edges = vec![leading(first)];
    let mut at = first;
    loop {
        let t = trailing(at) as usize;
        if topo.open(t) {
            edges.push(t as u32);
            break;
        }
        let [h0, h1] = topo.halves[t];
        let n = cage.face(at.0 as usize).len() as u32;
        let back = (at.0, (at.1 + n - 1) % n);
        let next = if h0 == back { h1 } else { h0 };
        if next == first {
            break;
        }
        if corners.len() > all.len() {
            return None;
        }
        corners.push(next);
        edges.push(leading(next));
        at = next;
    }
    (corners.len() == all.len()).then_some(Ring { corners, edges })
}

fn far(cage: &Level, ring: &Ring, i: usize) -> (usize, usize) {
    if i < ring.corners.len() {
        let (f, k) = ring.corners[i];
        (f as usize, k as usize + 1)
    } else {
        let (f, k) = ring.corners[i - 1];
        let n = cage.face(f as usize).len();
        (f as usize, k as usize + n - 1)
    }
}

fn far_point(cage: &Level, ring: &Ring, i: usize) -> DVec3 {
    let (f, k) = far(cage, ring, i);
    let face = cage.face(f);
    cage.points[face[k % face.len()] as usize]
}

fn far_value(cage: &Level, ring: &Ring, i: usize) -> u32 {
    let (f, k) = far(cage, ring, i);
    cage.value(f, k)
}

fn seams(cage: &Level, topo: &Topology) -> Vec<bool> {
    topo.halves
        .iter()
        .map(|&[(f0, k0), (f1, k1)]| {
            if f1 == u32::MAX {
                return false;
            }
            let (f0, k0, f1, k1) = (f0 as usize, k0 as usize, f1 as usize, k1 as usize);
            cage.value(f0, k0) != cage.value(f1, k1 + 1)
                || cage.value(f0, k0 + 1) != cage.value(f1, k1)
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Tag {
    Corner,
    Semi,
    Crease,
}

struct Span {
    value: u32,
    start: usize,
    size: usize,
    broken: bool,
}

impl Span {
    fn inner(&self, ring: &Ring) -> impl Iterator<Item = u32> + '_ {
        let m = ring.corners.len();
        let edges: Vec<u32> = (1..self.size)
            .map(|j| ring.edges[(self.start + j) % m])
            .collect();
        edges.into_iter()
    }

    fn ends(&self, cage: &Level, ring: &Ring) -> (u32, u32) {
        let m = ring.corners.len();
        let last = (self.start + self.size - 1) % m;
        let (f, k) = ring.corners[last];
        let n = cage.face(f as usize).len();
        (
            far_value(cage, ring, self.start),
            cage.value(f as usize, k as usize + n - 1),
        )
    }
}

fn spans(cage: &Level, ring: &Ring, seam: &[bool]) -> Vec<Span> {
    let m = ring.corners.len();
    let vals: Vec<u32> = ring
        .corners
        .iter()
        .map(|&(f, k)| cage.value(f as usize, k as usize))
        .collect();
    let single = vals.iter().all(|&x| x == vals[0]);
    if single && !ring.boundary() {
        let cuts: Vec<usize> = (0..m).filter(|&i| seam[ring.edges[i] as usize]).collect();
        return vec![Span {
            value: vals[0],
            start: cuts.first().copied().unwrap_or(0),
            size: m,
            broken: cuts.len() > 1,
        }];
    }
    let offset = if ring.boundary() {
        0
    } else {
        (0..m)
            .find(|&i| vals[i] != vals[(i + m - 1) % m])
            .unwrap_or(0)
    };
    let mut out: Vec<Span> = Vec::new();
    for j in 0..m {
        let i = (offset + j) % m;
        let joined = j > 0 && vals[i] == vals[(i + m - 1) % m];
        if joined {
            let last = out.last_mut().unwrap_or_else(|| unreachable!());
            last.size += 1;
            if seam[ring.edges[i] as usize] {
                last.broken = true;
            }
        } else {
            out.push(Span {
                value: vals[i],
                start: i,
                size: 1,
                broken: false,
            });
        }
    }
    let mut seen: HashMap<u32, usize> = HashMap::new();
    for s in &out {
        *seen.entry(s.value).or_default() += 1;
    }
    for s in &mut out {
        if seen[&s.value] > 1 {
            s.broken = true;
        }
    }
    out
}

fn tag(span: &Span, ring: &Ring, edge_sharp: impl Fn(u32) -> f64, vertex: f64) -> Tag {
    let (mut inf, mut semi) = (0, 0);
    for id in span.inner(ring) {
        let s = edge_sharp(id);
        if s >= INFINITE {
            inf += 1;
        } else if s > 0.0 {
            semi += 1;
        }
    }
    if span.broken || vertex >= INFINITE || span.size == 1 || inf > 0 {
        Tag::Corner
    } else if semi > 0 || vertex > 0.0 {
        Tag::Semi
    } else {
        Tag::Crease
    }
}

fn mismatched(topo: &Topology, options: &Options, seam: &[bool], v: usize) -> bool {
    topo.vertex_edges[v].iter().any(|&id| seam[id as usize])
        || (options.boundary == Boundary::Edges
            && topo.vertex_faces[v].len() == 1
            && topo.vertex_edges[v].len() == 2)
}

fn refine_values(
    cage: &Level,
    topo: &Topology,
    options: &Options,
    child_sharp: &[[f64; 2]],
    rules: &[VertexRules],
    out: &mut Level,
) {
    let v = cage.points.len();
    let e = topo.edges.len();
    let linear = options.uvs == UvInterpolation::Linear;
    let seam = seams(cage, topo);
    let val = |id: u32| cage.values[id as usize];
    let mut values: Vec<DVec2> = Vec::new();
    let face_values: Vec<u32> = (0..cage.faces())
        .map(|f| {
            let n = cage.face(f).len();
            let sum: DVec2 = (0..n).map(|k| val(cage.value(f, k))).sum();
            values.push(sum / n as f64);
            (values.len() - 1) as u32
        })
        .collect();
    let mut edge_values: Vec<[u32; 2]> = Vec::with_capacity(e);
    for (id, &[(f0, k0), (f1, k1)]) in topo.halves.iter().enumerate() {
        let (f0, k0) = (f0 as usize, k0 as usize);
        let a = val(cage.value(f0, k0));
        let b = val(cage.value(f0, k0 + 1));
        if seam[id] {
            let (f1, k1) = (f1 as usize, k1 as usize);
            values.push((a + b) * 0.5);
            values.push((val(cage.value(f1, k1)) + val(cage.value(f1, k1 + 1))) * 0.5);
            let n = values.len() as u32;
            edge_values.push([n - 2, n - 1]);
        } else {
            let point = if linear {
                (a + b) * 0.5
            } else {
                edge_point(topo.sharp[id], child_sharp[id], a, b, || {
                    (
                        values[face_values[f0] as usize],
                        values[face_values[f1 as usize] as usize],
                    )
                })
            };
            values.push(point);
            let n = values.len() as u32 - 1;
            edge_values.push([n, n]);
        }
    }
    let mut vertex_values: Vec<Vec<(u32, u32)>> = vec![Vec::new(); v];
    for i in 0..v {
        let own: Vec<u32> = {
            let mut list = Vec::new();
            for &f in &topo.vertex_faces[i] {
                let k = cage.face(f as usize).iter().position(|&x| x as usize == i);
                let value = cage.value(f as usize, k.unwrap_or(0));
                if !list.contains(&value) {
                    list.push(value);
                }
            }
            list
        };
        if linear {
            for value in own {
                values.push(val(value));
                vertex_values[i].push((value, (values.len() - 1) as u32));
            }
            continue;
        }
        if !mismatched(topo, options, &seam, i) {
            let value = own[0];
            let other = |id: u32| {
                let [(f0, k0), _] = topo.halves[id as usize];
                let (f0, k0) = (f0 as usize, k0 as usize);
                let face = cage.face(f0);
                let k = if face[k0] as usize == i { k0 + 1 } else { k0 };
                val(cage.value(f0, k))
            };
            values.push(vertex_point(
                &rules[i],
                val(value),
                &topo.vertex_edges[i],
                &topo.vertex_faces[i],
                other,
                |f| values[face_values[f as usize] as usize],
            ));
            vertex_values[i].push((value, (values.len() - 1) as u32));
            continue;
        }
        let Some(r) = ring(cage, topo, i) else {
            continue;
        };
        let end = |id: u32| usize::from(topo.edges[id as usize][0] as usize != i);
        let pv = cage.vertex_sharp(i);
        let cv = decrement(pv);
        for span in spans(cage, &r, &seam) {
            if vertex_values[i].iter().any(|&(p, _)| p == span.value) {
                continue;
            }
            let parent = tag(&span, &r, |id| topo.sharp[id as usize], pv);
            let child = tag(&span, &r, |id| child_sharp[id as usize][end(id)], cv);
            let here = val(span.value);
            let point = if parent == Tag::Corner || child != Tag::Crease {
                here
            } else {
                let (a, b) = span.ends(cage, &r);
                let (a, b) = (val(a), val(b));
                let corner = if parent == Tag::Semi {
                    let mut fractional: Vec<f64> = Vec::new();
                    if pv > 0.0 && cv <= 0.0 {
                        fractional.push(pv);
                    }
                    for id in span.inner(&r) {
                        let s = topo.sharp[id as usize];
                        let c = child_sharp[id as usize][end(id)];
                        let moves = match options.creasing {
                            Creasing::Uniform => s <= 1.0,
                            Creasing::Chaikin => c <= 0.0,
                        };
                        if s > 0.0 && moves {
                            fractional.push(s);
                        }
                    }
                    if fractional.is_empty() {
                        0.0
                    } else {
                        (fractional.iter().sum::<f64>() / fractional.len() as f64).min(1.0)
                    }
                } else {
                    0.0
                };
                let crease = 1.0 - corner;
                here * (crease * 0.75 + corner) + (a + b) * (crease * 0.125)
            };
            values.push(point);
            vertex_values[i].push((span.value, (values.len() - 1) as u32));
        }
    }
    let child_of = |i: u32, value: u32| -> u32 {
        vertex_values[i as usize]
            .iter()
            .find(|&&(p, _)| p == value)
            .map(|&(_, c)| c)
            .unwrap_or(0)
    };
    let side = |id: u32, f: usize, k: usize| -> u32 {
        let [h0, _] = topo.halves[id as usize];
        let pair = edge_values[id as usize];
        if h0 == (f as u32, k as u32) {
            pair[0]
        } else {
            pair[1]
        }
    };
    let mut corner_values = Vec::with_capacity(out.verts.len());
    for (f, &centre) in face_values.iter().enumerate() {
        let face = cage.face(f);
        let n = face.len();
        let s = cage.starts[f];
        for (k, &corner) in face.iter().enumerate() {
            let back = (k + n - 1) % n;
            corner_values.extend([
                child_of(corner, cage.value(f, k)),
                side(topo.face_edges[s + k], f, k),
                centre,
                side(topo.face_edges[s + back], f, back),
            ]);
        }
    }
    out.values = values;
    out.corner_values = corner_values;
}

struct Masks {
    vertex: f64,
    edges: Vec<f64>,
    faces: Vec<f64>,
}

impl Masks {
    fn zero(edges: usize, faces: usize) -> Masks {
        Masks {
            vertex: 0.0,
            edges: vec![0.0; edges],
            faces: vec![0.0; faces],
        }
    }
}

fn smooth_limit(valence: usize) -> Masks {
    let mut m = Masks::zero(valence, valence);
    if valence == 2 {
        m.vertex = 1.0;
        return m;
    }
    let (v, e, f) = if valence == 4 {
        (4.0 / 9.0, 1.0 / 9.0, 1.0 / 36.0)
    } else {
        let n = valence as f64;
        let f = 1.0 / (n * (n + 5.0));
        let e = 4.0 * f;
        (1.0 - n * (e + f), e, f)
    };
    m.vertex = v;
    m.edges.fill(e);
    m.faces.fill(f);
    m
}

fn crease_limit(edges: usize, faces: usize, ends: [usize; 2]) -> Masks {
    let mut m = Masks::zero(edges, faces);
    m.vertex = 2.0 / 3.0;
    m.edges[ends[0]] = 1.0 / 6.0;
    m.edges[ends[1]] = 1.0 / 6.0;
    m
}

fn smooth_tangents(valence: usize) -> (Masks, Masks) {
    let mut t1 = Masks::zero(valence, valence);
    let mut t2 = Masks::zero(valence, valence);
    if valence == 2 {
        t1.vertex = -1.0;
        t1.edges[0] = 1.0;
        t2.vertex = -1.0;
        t2.edges[1] = 1.0;
        return (t1, t2);
    }
    if valence == 4 {
        t1.edges.copy_from_slice(&[4.0, 0.0, -4.0, 0.0]);
        t1.faces.copy_from_slice(&[1.0, -1.0, -1.0, 1.0]);
        t2.edges.copy_from_slice(&[0.0, 4.0, 0.0, -4.0]);
        t2.faces.copy_from_slice(&[1.0, 1.0, -1.0, -1.0]);
        return (t1, t2);
    }
    let theta = 2.0 * PI / valence as f64;
    let cos_theta = theta.cos();
    let cos_half = (theta * 0.5).cos();
    let lambda = 5.0 / 16.0 + (cos_theta + cos_half * (2.0 * (9.0 + cos_theta)).sqrt()) / 16.0;
    let face_scale = 1.0 / (4.0 * lambda - 1.0);
    for i in 0..valence {
        let c0 = (i as f64 * theta).cos();
        let c1 = ((i + 1) as f64 * theta).cos();
        t1.edges[i] = 4.0 * c0;
        t1.faces[i] = face_scale * (c0 + c1);
    }
    t2.edges[0] = t1.edges[valence - 1];
    t2.faces[0] = t1.faces[valence - 1];
    for i in 1..valence {
        t2.edges[i] = t1.edges[i - 1];
        t2.faces[i] = t1.faces[i - 1];
    }
    (t1, t2)
}

fn crease_tangents(edges: usize, faces: usize, ends: [usize; 2]) -> (Masks, Masks) {
    let mut t1 = Masks::zero(edges, faces);
    let mut t2 = Masks::zero(edges, faces);
    t1.edges[ends[0]] = 0.5;
    t1.edges[ends[1]] = -0.5;
    let interior = ends[1] - ends[0] - 1;
    if interior == 1 {
        t2.vertex = -4.0 / 6.0;
        t2.edges[ends[0]] = -1.0 / 6.0;
        t2.edges[ends[0] + 1] = 4.0 / 6.0;
        t2.edges[ends[1]] = -1.0 / 6.0;
        t2.faces[ends[0]] = 1.0 / 6.0;
        t2.faces[ends[0] + 1] = 1.0 / 6.0;
    } else if interior > 1 {
        let k = (interior + 1) as f64;
        let theta = PI / k;
        let (sin_theta, cos_theta) = theta.sin_cos();
        let common = 1.0 / (k * (3.0 + cos_theta));
        let r = (cos_theta + 1.0) / sin_theta;
        t2.vertex = 4.0 * r * (cos_theta - 1.0) * common;
        let crease = -r * (1.0 + 2.0 * cos_theta) * common;
        t2.edges[ends[0]] = crease;
        t2.edges[ends[1]] = crease;
        t2.faces[ends[0]] = sin_theta * common;
        let mut next = sin_theta;
        for i in 1..=interior {
            let this = next;
            next = ((i + 1) as f64 * theta).sin();
            t2.edges[ends[0] + i] = 4.0 * this * common;
            t2.faces[ends[0] + i] = (this + next) * common;
        }
    } else {
        t2.vertex = -6.0;
        t2.edges[ends[0]] = 3.0;
        t2.edges[ends[1]] = 3.0;
    }
    (t1, t2)
}

struct Local<P> {
    centre: P,
    edges: Vec<P>,
    faces: Vec<P>,
}

impl<P: Point> Local<P> {
    fn apply(&self, m: &Masks) -> P {
        let mut out = self.centre * m.vertex;
        for (p, w) in self.edges.iter().zip(&m.edges) {
            out = out + *p * *w;
        }
        for (p, w) in self.faces.iter().zip(&m.faces) {
            out = out + *p * *w;
        }
        out
    }

    fn sector(&self, sector: Sector) -> Local<P> {
        let (e, m) = (self.edges.len(), self.faces.len());
        Local {
            centre: self.centre,
            edges: (0..=sector.count)
                .map(|i| self.edges[(sector.start + i) % e])
                .collect(),
            faces: (0..sector.count)
                .map(|i| self.faces[(sector.start + i) % m])
                .collect(),
        }
    }
}

impl Local<DVec3> {
    fn wedge(&self) -> DVec3 {
        (0..self.faces.len())
            .map(|i| (self.edges[i] - self.centre).cross(self.edges[i + 1] - self.centre))
            .sum()
    }
}

fn opposite(ring: &Ring, i: usize) -> (usize, usize) {
    let (f, k) = ring.corners[i];
    (f as usize, k as usize + 2)
}

fn local_points(cage: &Level, ring: &Ring, v: usize) -> Local<DVec3> {
    Local {
        centre: cage.points[v],
        edges: (0..ring.edges.len())
            .map(|i| far_point(cage, ring, i))
            .collect(),
        faces: (0..ring.corners.len())
            .map(|i| {
                let (f, k) = opposite(ring, i);
                let face = cage.face(f);
                cage.points[face[k % face.len()] as usize]
            })
            .collect(),
    }
}

fn local_values(cage: &Level, ring: &Ring, value: u32) -> Local<DVec2> {
    Local {
        centre: cage.values[value as usize],
        edges: (0..ring.edges.len())
            .map(|i| cage.values[far_value(cage, ring, i) as usize])
            .collect(),
        faces: (0..ring.corners.len())
            .map(|i| {
                let (f, k) = opposite(ring, i);
                cage.values[cage.value(f, k) as usize]
            })
            .collect(),
    }
}

#[derive(Clone, Copy)]
struct Sector {
    start: usize,
    count: usize,
}

fn sectors(ring: &Ring, sharp: &[usize], whole: bool) -> Vec<Sector> {
    let m = ring.corners.len();
    if whole || sharp.is_empty() {
        return vec![Sector { start: 0, count: m }];
    }
    if ring.boundary() {
        return sharp
            .windows(2)
            .map(|w| Sector {
                start: w[0],
                count: w[1] - w[0],
            })
            .collect();
    }
    (0..sharp.len())
        .map(|j| {
            let (a, b) = (sharp[j], sharp[(j + 1) % sharp.len()]);
            Sector {
                start: a,
                count: (b + m - a - 1) % m + 1,
            }
        })
        .collect()
}

struct Evaluated {
    positions: Vec<DVec3>,
    normals: Vec<DVec3>,
    corner_sector: Vec<u32>,
    corner_value: Vec<u32>,
    values: Vec<DVec2>,
}

fn surface(cage: &Level, topo: &Topology, options: &Options, limit: bool) -> Mesh {
    let Evaluated {
        positions,
        normals,
        corner_sector,
        corner_value,
        values,
    } = evaluate(cage, topo, options, limit);
    let mut out = Mesh::default();
    let mut uvs = Vec::new();
    let mut made: HashMap<(u32, u32, u32), u32> = HashMap::new();
    let mut index = vec![0u32; cage.verts.len()];
    for f in 0..cage.faces() {
        let face = cage.face(f);
        let start = cage.starts[f];
        for (k, &vi) in face.iter().enumerate() {
            let c = start + k;
            let sector = corner_sector[c];
            let value = corner_value[c];
            let next = out.positions.len() as u32;
            index[c] = *made.entry((vi, sector, value)).or_insert_with(|| {
                let p = positions[vi as usize];
                out.positions.push([p.x as f32, p.y as f32, p.z as f32]);
                let n = normals[sector as usize];
                out.normals.push([n.x as f32, n.y as f32, n.z as f32]);
                if cage.has_uvs() {
                    let uv = values[value as usize];
                    uvs.push([uv.x as f32, uv.y as f32]);
                }
                next
            });
        }
        for k in 1..face.len() - 1 {
            out.indices
                .extend([index[start], index[start + k], index[start + k + 1]]);
        }
    }
    if cage.has_uvs() {
        out.uvs = Some(uvs);
    }
    out.slots = vec![Slot {
        material: MATERIAL.to_string(),
        start: 0,
        count: out.indices.len() as u32,
    }];
    out
}

fn evaluate(cage: &Level, topo: &Topology, options: &Options, limit: bool) -> Evaluated {
    let seam = if cage.has_uvs() {
        seams(cage, topo)
    } else {
        Vec::new()
    };
    let linear = options.uvs == UvInterpolation::Linear;
    let mut normals: Vec<DVec3> = Vec::new();
    let mut corner_sector = vec![0u32; cage.verts.len()];
    let mut corner_value = vec![0u32; cage.verts.len()];
    let mut limit_values: Vec<DVec2> = Vec::new();
    let mut positions = cage.points.clone();
    for (v, place) in positions.iter_mut().enumerate() {
        let Some(r) = ring(cage, topo, v) else {
            continue;
        };
        let m = r.corners.len();
        let local = local_points(cage, &r, v);
        let sharp: Vec<usize> = (0..r.edges.len())
            .filter(|&i| topo.sharp[r.edges[i] as usize] > 0.0)
            .collect();
        let pv = cage.vertex_sharp(v);
        let rl = vertex_rule(pv, sharp.len());
        let position = |local: &Local<DVec3>| match rl {
            Rule::Corner => local.centre,
            Rule::Crease => local.apply(&crease_limit(r.edges.len(), m, [sharp[0], sharp[1]])),
            Rule::Smooth => local.apply(&smooth_limit(m)),
        };
        if limit {
            *place = position(&local);
        }
        for sector in sectors(&r, &sharp, rl == Rule::Smooth) {
            let part = local.sector(sector);
            let normal = match rl {
                Rule::Smooth if limit => {
                    let (t1, t2) = smooth_tangents(m);
                    local.apply(&t1).cross(local.apply(&t2))
                }
                Rule::Crease if limit => {
                    let (t1, t2) =
                        crease_tangents(sector.count + 1, sector.count, [0, sector.count]);
                    part.apply(&t1).cross(part.apply(&t2))
                }
                _ => part.wedge(),
            };
            let normal = if normal.length_squared() > 1e-30 {
                normal
            } else {
                part.wedge()
            };
            let id = normals.len() as u32;
            normals.push(normal.normalize_or_zero());
            for j in 0..sector.count {
                let (f, k) = r.corners[(sector.start + j) % m];
                corner_sector[cage.starts[f as usize] + k as usize] = id;
            }
        }
        if !cage.has_uvs() {
            continue;
        }
        let mut done: Vec<u32> = Vec::new();
        let mut put = |value: u32, uv: DVec2, corner_value: &mut [u32]| {
            limit_values.push(uv);
            let at = (limit_values.len() - 1) as u32;
            for &(f, k) in &r.corners {
                if cage.value(f as usize, k as usize) == value {
                    corner_value[cage.starts[f as usize] + k as usize] = at;
                }
            }
        };
        if !limit || linear {
            for &(f, k) in &r.corners {
                let value = cage.value(f as usize, k as usize);
                if !done.contains(&value) {
                    done.push(value);
                    put(value, cage.values[value as usize], &mut corner_value);
                }
            }
        } else if !mismatched(topo, options, &seam, v) {
            let (f, k) = r.corners[0];
            let value = cage.value(f as usize, k as usize);
            let values = local_values(cage, &r, value);
            let uv = match rl {
                Rule::Corner => values.centre,
                Rule::Crease => values.apply(&crease_limit(r.edges.len(), m, [sharp[0], sharp[1]])),
                Rule::Smooth => values.apply(&smooth_limit(m)),
            };
            put(value, uv, &mut corner_value);
        } else {
            for span in spans(cage, &r, &seam) {
                if done.contains(&span.value) {
                    continue;
                }
                done.push(span.value);
                let here = cage.values[span.value as usize];
                let uv = match tag(&span, &r, |id| topo.sharp[id as usize], pv) {
                    Tag::Crease => {
                        let (a, b) = span.ends(cage, &r);
                        here * (2.0 / 3.0)
                            + (cage.values[a as usize] + cage.values[b as usize]) * (1.0 / 6.0)
                    }
                    _ => here,
                };
                put(span.value, uv, &mut corner_value);
            }
        }
    }
    Evaluated {
        positions,
        normals,
        corner_sector,
        corner_value,
        values: limit_values,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn cube() -> Cage {
        let points = (0..8)
            .map(|i| [i & 1, (i >> 1) & 1, (i >> 2) & 1].map(|b| if b == 1 { 1.0 } else { -1.0 }))
            .collect();
        let faces = vec![
            vec![0, 2, 3, 1],
            vec![4, 5, 7, 6],
            vec![0, 1, 5, 4],
            vec![2, 6, 7, 3],
            vec![0, 4, 6, 2],
            vec![1, 3, 7, 5],
        ];
        Cage {
            points,
            faces,
            ..Cage::default()
        }
    }

    fn top_loop(sharpness: f64) -> Vec<Crease> {
        [[4, 5], [5, 7], [7, 6], [6, 4]]
            .map(|edge| Crease { edge, sharpness })
            .to_vec()
    }

    fn levels(cage: &Cage, options: &Options, n: u32) -> Level {
        let mut level = Level::from_cage(cage, options).unwrap();
        for _ in 0..n {
            let topo = topology(&level).ok().unwrap();
            level = refine(&level, &topo, options);
        }
        level
    }

    fn near(m: &Mesh, at: [f64; 3]) -> Vec<usize> {
        (0..m.positions.len())
            .filter(|&i| {
                let p = m.positions[i];
                (0..3).all(|k| (p[k] as f64 - at[k]).abs() < 1e-4)
            })
            .collect()
    }

    fn welded_edges(m: &Mesh) -> BTreeMap<(u32, u32), u32> {
        let mut ids: BTreeMap<[u32; 3], u32> = BTreeMap::new();
        let weld: Vec<u32> = m
            .positions
            .iter()
            .map(|p| {
                let next = ids.len() as u32;
                *ids.entry(p.map(f32::to_bits)).or_insert(next)
            })
            .collect();
        let mut edges = BTreeMap::new();
        for t in m.indices.chunks_exact(3) {
            for k in 0..3 {
                let (a, b) = (weld[t[k] as usize], weld[t[(k + 1) % 3] as usize]);
                *edges.entry(key(a, b)).or_insert(0) += 1;
            }
        }
        edges
    }

    #[test]
    fn a_cube_corner_refines_to_five_ninths_and_its_limit_is_one_half() {
        let options = Options::default();
        let one = levels(&cube(), &options, 1);
        assert!((one.points[7] - DVec3::splat(5.0 / 9.0)).length() < 1e-12);
        let mut gap = f64::INFINITY;
        for n in 1..=8 {
            let next = (levels(&cube(), &options, n).points[7] - DVec3::splat(0.5)).length();
            assert!(next < gap, "level {n}: {next}");
            gap = next;
        }
        assert!(gap < 1e-3, "{gap}");
        for n in 1..=4 {
            let m = subdivide(&cube(), n, &options).unwrap();
            assert_eq!(m.triangle_count(), 6 * 4usize.pow(n) * 2);
            assert_eq!(near(&m, [0.5; 3]).len(), 1, "level {n}");
            let corner = DVec3::from_array(m.positions[near(&m, [0.5; 3])[0]].map(f64::from));
            let normal = DVec3::from_array(m.normals[near(&m, [0.5; 3])[0]].map(f64::from));
            assert!((normal - DVec3::splat(1.0).normalize()).length() < 1e-5);
            assert!((corner - DVec3::splat(0.5)).length() < 1e-6);
            assert!(welded_edges(&m).values().all(|&c| c == 2));
        }
    }

    #[test]
    fn the_limit_matches_deep_refinement_at_every_vertex() {
        let mut cage = cube();
        cage.creases = top_loop(2.5);
        cage.creases.push(Crease {
            edge: [0, 1],
            sharpness: 10.0,
        });
        cage.points[7] = [1.4, 1.2, 1.3];
        cage.corners = vec![Corner {
            point: 2,
            sharpness: 1.5,
        }];
        let options = Options::default();
        let two = levels(&cage, &options, 3);
        let deep = levels(&cage, &options, 7);
        let topo = topology(&two).ok().unwrap();
        let limit = evaluate(&two, &topo, &options, true).positions;
        let dart = |v: usize| {
            topo.vertex_edges[v]
                .iter()
                .filter(|&&e| topo.sharp[e as usize] > 0.0)
                .count()
                == 1
        };
        let worst = (0..two.points.len())
            .filter(|&v| !dart(v))
            .map(|v| (limit[v] - deep.points[v]).length())
            .fold(0.0, f64::max);
        assert!(worst < 3e-4, "{worst}");
    }

    fn edge_middle(m: &Mesh) -> DVec3 {
        m.positions
            .iter()
            .filter(|p| p[1].abs() < 1e-6 && p[0] > 0.0)
            .map(|p| DVec3::from_array(p.map(f64::from)))
            .max_by(|a, b| (a.x + a.z).total_cmp(&(b.x + b.z)))
            .unwrap()
    }

    #[test]
    fn a_crease_loop_sharpens_the_edge_monotonically_with_its_sharpness() {
        let at = |sharpness: f64| {
            let mut cage = cube();
            cage.creases = top_loop(sharpness);
            edge_middle(&subdivide(&cage, 4, &Options::default()).unwrap())
        };
        let (smooth, two, sharp) = (at(0.0), at(2.0), at(10.0));
        assert!(
            smooth.z < two.z && two.z < sharp.z,
            "{smooth} {two} {sharp}"
        );
        assert!(
            smooth.x < two.x && two.x < sharp.x,
            "{smooth} {two} {sharp}"
        );
        assert!(
            (sharp - DVec3::new(11.0 / 12.0, 0.0, 1.0)).length() < 1e-6,
            "{sharp}"
        );
        let mut cage = cube();
        cage.creases = top_loop(10.0);
        let m = subdivide(&cage, 3, &Options::default()).unwrap();
        let normals: BTreeMap<[i64; 3], usize> = m
            .positions
            .iter()
            .zip(&m.normals)
            .filter(|(p, _)| (p[2] - 1.0).abs() < 1e-6)
            .fold(BTreeMap::new(), |mut seen, (p, _)| {
                *seen
                    .entry(p.map(|c| (c as f64 * 1e5).round() as i64))
                    .or_insert(0) += 1;
                seen
            });
        assert!(normals.values().any(|&n| n == 2));
    }

    #[test]
    fn chaikin_and_uniform_creasing_differ_only_where_sharpness_varies_along_a_crease() {
        let build = |creasing: Creasing, sharpness: [f64; 4]| {
            let mut cage = cube();
            cage.creases = top_loop(0.0)
                .into_iter()
                .zip(sharpness)
                .map(|(c, s)| Crease { sharpness: s, ..c })
                .collect();
            subdivide(
                &cage,
                3,
                &Options {
                    creasing,
                    ..Options::default()
                },
            )
            .unwrap()
        };
        assert_eq!(
            build(Creasing::Chaikin, [3.0; 4]),
            build(Creasing::Uniform, [3.0; 4])
        );
        assert_ne!(
            build(Creasing::Chaikin, [1.0, 3.0, 1.0, 3.0]),
            build(Creasing::Uniform, [1.0, 3.0, 1.0, 3.0])
        );
    }

    #[test]
    fn an_ngon_face_splits_into_quads_round_its_centre() {
        let ring = |z: f64| -> Vec<[f64; 3]> {
            (0..5)
                .map(|i| {
                    let a = i as f64 * 2.0 * PI / 5.0;
                    [a.cos(), a.sin(), z]
                })
                .collect()
        };
        let mut points = ring(-1.0);
        points.extend(ring(1.0));
        let mut faces = vec![(0..5).rev().collect::<Vec<u32>>(), (5..10).collect()];
        for i in 0..5u32 {
            let j = (i + 1) % 5;
            faces.push(vec![i, j, j + 5, i + 5]);
        }
        let cage = Cage {
            points,
            faces,
            ..Cage::default()
        };
        let m = subdivide(&cage, 1, &Options::default()).unwrap();
        assert_eq!(m.triangle_count(), (5 + 5 + 5 * 4) * 2);
        assert!(welded_edges(&m).values().all(|&c| c == 2));
        let top = m
            .positions
            .iter()
            .zip(&m.normals)
            .find(|(p, _)| p[0].abs() < 1e-6 && p[1].abs() < 1e-6 && p[2] > 0.0)
            .unwrap();
        assert!((top.1[2] - 1.0).abs() < 1e-6);
        let zero = subdivide(&cage, 0, &Options::default()).unwrap();
        assert_eq!(zero.triangle_count(), 3 + 3 + 5 * 2);
    }

    fn grid(n: u32) -> Cage {
        let points = (0..=n)
            .flat_map(|y| (0..=n).map(move |x| [x as f64, y as f64, 0.0]))
            .collect();
        let at = |x: u32, y: u32| y * (n + 1) + x;
        let faces = (0..n)
            .flat_map(|y| {
                (0..n).map(move |x| vec![at(x, y), at(x + 1, y), at(x + 1, y + 1), at(x, y + 1)])
            })
            .collect();
        Cage {
            points,
            faces,
            ..Cage::default()
        }
    }

    #[test]
    fn an_open_cage_keeps_its_boundary_and_sharpens_corners_on_request() {
        let edges = subdivide(&grid(3), 3, &Options::default()).unwrap();
        assert!(welded_edges(&edges).values().any(|&c| c == 1));
        assert!(edges.positions.iter().all(|p| p[2] == 0.0));
        assert!(edges.normals.iter().all(|n| (n[2] - 1.0).abs() < 1e-6));
        assert!(near(&edges, [0.0; 3]).is_empty());
        let boundary: Vec<[f32; 3]> = edges
            .positions
            .iter()
            .copied()
            .filter(|p| p[1] == 0.0)
            .collect();
        assert!(boundary.len() > 8);
        let corners = Options {
            boundary: Boundary::EdgesAndCorners,
            ..Options::default()
        };
        let sharp = subdivide(&grid(3), 3, &corners).unwrap();
        assert_eq!(near(&sharp, [0.0; 3]).len(), 1);
        assert_eq!(near(&sharp, [3.0, 3.0, 0.0]).len(), 1);
        assert_eq!(near(&sharp, [1.5, 0.0, 0.0]).len(), 1);
    }

    fn uv_cube() -> Cage {
        let mut cage = cube();
        cage.uvs = Some(
            (0..6)
                .map(|_| vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
                .collect(),
        );
        cage
    }

    #[test]
    fn linear_uvs_interpolate_each_face_bilinearly() {
        let options = Options {
            uvs: UvInterpolation::Linear,
            ..Options::default()
        };
        let m = subdivide(&uv_cube(), 2, &options).unwrap();
        let uvs = m.uvs.as_ref().unwrap();
        assert_eq!(uvs.len(), m.positions.len());
        let set: BTreeSet<[i64; 2]> = uvs
            .iter()
            .map(|t| t.map(|c| (c as f64 * 4.0).round() as i64))
            .collect();
        assert_eq!(set.len(), 25);
        assert!(
            uvs.iter()
                .all(|t| t.iter().all(|c| (c * 4.0).fract() == 0.0))
        );
    }

    #[test]
    fn smooth_uvs_hold_island_corners_and_follow_the_seams() {
        let m = subdivide(&uv_cube(), 2, &Options::default()).unwrap();
        let uvs = m.uvs.as_ref().unwrap();
        let corners: Vec<usize> = (0..m.positions.len())
            .filter(|&i| m.positions[i].iter().all(|c| (c.abs() - 0.5).abs() < 1e-6))
            .collect();
        assert!(corners.len() >= 8);
        assert!(
            corners
                .iter()
                .all(|&i| uvs[i].iter().all(|&c| c == 0.0 || c == 1.0))
        );
        assert!(
            uvs.iter()
                .all(|t| t.iter().all(|c| (0.0..=1.0).contains(c)))
        );
        assert!(uvs.iter().any(|t| t[1] == 0.0 && t[0] > 0.0 && t[0] < 1.0));
        let mut flat = grid(3);
        flat.uvs = Some(
            flat.faces
                .iter()
                .map(|f| {
                    f.iter()
                        .map(|&i| [flat.points[i as usize][0], flat.points[i as usize][1]])
                        .collect()
                })
                .collect(),
        );
        let corners = Options {
            boundary: Boundary::EdgesAndCorners,
            ..Options::default()
        };
        let m = subdivide(&flat, 3, &corners).unwrap();
        for (p, t) in m.positions.iter().zip(m.uvs.as_ref().unwrap()) {
            assert!(
                (p[0] - t[0]).abs() < 1e-5 && (p[1] - t[1]).abs() < 1e-5,
                "{p:?} {t:?}"
            );
        }
    }

    #[test]
    fn the_same_cage_builds_the_same_bytes() {
        let mut cage = uv_cube();
        cage.creases = top_loop(1.7);
        cage.corners = vec![Corner {
            point: 0,
            sharpness: 2.0,
        }];
        let a = subdivide(&cage, 3, &Options::default()).unwrap();
        let b = subdivide(&cage, 3, &Options::default()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn a_broken_cage_is_refused_in_one_line_naming_the_key() {
        let refuse =
            |cage: Cage, levels: u32| subdivide(&cage, levels, &Options::default()).unwrap_err();
        let mut c = cube();
        c.faces[2][1] = 9;
        assert_eq!(
            refuse(c, 1),
            "faces[2]: point 9 is out of range; the cage has 8 points"
        );
        let mut c = cube();
        c.faces[1] = vec![4, 5, 4, 6];
        assert_eq!(refuse(c, 1), "faces[1]: point 4 appears twice");
        let mut c = cube();
        c.faces.push(vec![0, 2, 6]);
        assert!(refuse(c, 1).starts_with("faces[6]: the edge 0-2 "),);
        let mut c = cube();
        c.faces[0].reverse();
        assert!(refuse(c, 1).contains("every face must wind the same way"));
        let mut c = cube();
        c.points.push([0.0; 3]);
        assert_eq!(refuse(c, 1), "points[8]: no face uses it");
        let mut c = cube();
        c.creases = vec![Crease {
            edge: [0, 7],
            sharpness: 2.0,
        }];
        assert_eq!(
            refuse(c, 1),
            "creases[0].edge: 0-7 is not an edge of any face"
        );
        assert_eq!(
            refuse(cube(), 5),
            "subdivide: expected a whole number from 0 to 4"
        );
        let mut c = uv_cube();
        c.uvs.as_mut().unwrap()[3].pop();
        assert_eq!(
            refuse(c, 1),
            "uvs[3]: expected 4 [u, v] pairs, one per point of faces[3]"
        );
    }
}
