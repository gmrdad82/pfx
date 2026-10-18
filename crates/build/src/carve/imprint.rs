use glam::{DVec2, DVec3, Vec2, Vec3};
use pfx_mesh::{Mesh, Slot, triangle_normal};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const VIEWS: [&str; 4] = ["side", "top", "front", "rear"];

const FACE: f64 = 1e-6;
const EPS: f64 = 1e-9;
const PAD: f64 = 1e-6;
const GRAIN: f64 = 1e-12;
const SIDE: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Side,
    Top,
    Front,
    Rear,
}

impl View {
    pub fn parse(name: &str) -> Option<View> {
        match name {
            "side" => Some(View::Side),
            "top" => Some(View::Top),
            "front" => Some(View::Front),
            "rear" => Some(View::Rear),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub along: DVec3,
    pub up: DVec3,
    pub across: DVec3,
}

impl Frame {
    pub const WORLD: Frame = Frame {
        along: DVec3::Z,
        up: DVec3::Y,
        across: DVec3::X,
    };

    pub fn new(along: DVec3, up: DVec3) -> Option<Frame> {
        let along = along.normalize_or_zero();
        let up = (up - along * up.dot(along)).normalize_or_zero();
        (along != DVec3::ZERO && up != DVec3::ZERO).then(|| Frame {
            along,
            up,
            across: up.cross(along),
        })
    }

    pub fn along(axis: DVec3) -> Option<Frame> {
        let up = if axis.normalize_or_zero().y.abs() < 0.9 {
            DVec3::Y
        } else {
            DVec3::Z
        };
        Frame::new(axis, up)
    }

    pub fn project(&self, view: View, p: DVec3) -> DVec2 {
        match view {
            View::Side => DVec2::new(p.dot(self.along), p.dot(self.up)),
            View::Top => DVec2::new(p.dot(self.along), p.dot(self.across)),
            View::Front | View::Rear => DVec2::new(p.dot(self.across), p.dot(self.up)),
        }
    }

    fn toward(&self, view: View) -> DVec3 {
        match view {
            View::Side => self.across,
            View::Top => self.up,
            View::Front => self.along,
            View::Rear => -self.along,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Facing {
    pub view: View,
    pub frame: Frame,
    pub flank: f64,
}

impl Facing {
    pub fn all(view: View, frame: Frame) -> Facing {
        Facing {
            view,
            frame,
            flank: 0.0,
        }
    }

    pub fn project(&self, p: DVec3) -> DVec2 {
        self.frame.project(self.view, p)
    }

    pub fn faces(&self, n: DVec3) -> bool {
        let d = n.dot(self.frame.toward(self.view));
        match self.view {
            View::Side if self.flank == 0.0 => d.abs() > FACE,
            View::Side => d * self.flank > FACE,
            _ => d > FACE,
        }
    }

    pub fn inward(&self, n: DVec3) -> DVec3 {
        let toward = self.frame.toward(self.view);
        if n.dot(toward) < 0.0 { toward } else { -toward }
    }

    pub fn depth(&self, p: DVec3) -> f64 {
        p.dot(self.frame.toward(self.view))
    }
}

pub(crate) type Key = [u64; 3];
pub(crate) type Edge = (Key, Key);

pub(crate) fn key(p: DVec3) -> Key {
    [
        (p.x + 0.0).to_bits(),
        (p.y + 0.0).to_bits(),
        (p.z + 0.0).to_bits(),
    ]
}

pub(crate) fn edge(a: Key, b: Key) -> Edge {
    if a <= b { (a, b) } else { (b, a) }
}

pub(crate) struct Work {
    pub pos: Vec<DVec3>,
    pub nrm: Vec<DVec3>,
    pub uv: Option<Vec<DVec2>>,
    pub uv1: Option<Vec<DVec2>>,
    pub tris: Vec<[u32; 3]>,
    pub mat: Vec<usize>,
    pub materials: Vec<Option<String>>,
}

fn uvs(u: &Option<Vec<[f32; 2]>>) -> Option<Vec<DVec2>> {
    u.as_ref()
        .map(|u| u.iter().map(|&t| Vec2::from(t).as_dvec2()).collect())
}

impl Work {
    pub fn from_mesh(m: &Mesh) -> Work {
        let count = m.indices.len() / 3;
        let mut w = Work {
            pos: m
                .positions
                .iter()
                .map(|&p| Vec3::from(p).as_dvec3())
                .collect(),
            nrm: (0..m.positions.len())
                .map(|i| {
                    m.normals
                        .get(i)
                        .map_or(DVec3::ZERO, |&n| Vec3::from(n).as_dvec3())
                })
                .collect(),
            uv: uvs(&m.uvs),
            uv1: uvs(&m.uvs1),
            tris: m
                .indices
                .chunks_exact(3)
                .map(|t| [t[0], t[1], t[2]])
                .collect(),
            mat: vec![usize::MAX; count],
            materials: Vec::new(),
        };
        for s in &m.slots {
            let id = w.intern(Some(s.material.clone()));
            let end = ((s.start + s.count) as usize / 3).min(count);
            for t in (s.start as usize / 3)..end {
                w.mat[t] = id;
            }
        }
        if w.mat.contains(&usize::MAX) {
            let id = w.intern(None);
            for m in w.mat.iter_mut().filter(|m| **m == usize::MAX) {
                *m = id;
            }
        }
        w
    }

    pub fn intern(&mut self, material: Option<String>) -> usize {
        match self.materials.iter().position(|m| *m == material) {
            Some(i) => i,
            None => {
                self.materials.push(material);
                self.materials.len() - 1
            }
        }
    }

    pub fn corners(&self, t: usize) -> [DVec3; 3] {
        self.tris[t].map(|i| self.pos[i as usize])
    }

    pub fn face(&self, t: usize) -> DVec3 {
        triangle_normal(&self.corners(t))
    }

    pub fn centroid(&self, t: usize) -> DVec3 {
        let [a, b, c] = self.corners(t);
        (a + b + c) / 3.0
    }

    pub fn push_vertex(&mut self, from: &[(u32, f64)], p: DVec3, n: Option<DVec3>) -> u32 {
        let blend = |v: &[DVec2]| from.iter().map(|&(i, w)| v[i as usize] * w).sum();
        let normal = n.unwrap_or_else(|| {
            from.iter()
                .map(|&(i, w)| self.nrm[i as usize] * w)
                .sum::<DVec3>()
                .normalize_or_zero()
        });
        let uv = self.uv.as_ref().map(|v| blend(v));
        let uv1 = self.uv1.as_ref().map(|v| blend(v));
        self.pos.push(p);
        self.nrm.push(normal);
        if let (Some(v), Some(u)) = (self.uv.as_mut(), uv) {
            v.push(u);
        }
        if let (Some(v), Some(u)) = (self.uv1.as_mut(), uv1) {
            v.push(u);
        }
        self.pos.len() as u32 - 1
    }

    pub fn select(&self, facing: Facing, polygon: &[DVec2]) -> Vec<bool> {
        let outline = Outline::new(polygon);
        (0..self.tris.len())
            .map(|t| {
                facing.faces(self.face(t)) && outline.contains(facing.project(self.centroid(t)))
            })
            .collect()
    }

    pub fn retain(&mut self, keep: &[bool]) {
        let mut tris = Vec::with_capacity(self.tris.len());
        let mut mat = Vec::with_capacity(self.tris.len());
        for ((tri, m), _) in self
            .tris
            .iter()
            .zip(&self.mat)
            .zip(keep)
            .filter(|(_, k)| **k)
        {
            tris.push(*tri);
            mat.push(*m);
        }
        self.tris = tris;
        self.mat = mat;
    }

    pub fn cut(&mut self, removed: &[bool], lip: f64) -> BTreeMap<Key, DVec3> {
        let mut gone = BTreeSet::new();
        let mut uses: BTreeMap<Edge, usize> = BTreeMap::new();
        for (t, &gone_here) in removed.iter().enumerate() {
            for (a, b) in self.edges(t) {
                let e = edge(key(self.pos[a as usize]), key(self.pos[b as usize]));
                if gone_here {
                    gone.insert(e);
                } else {
                    *uses.entry(e).or_default() += 1;
                }
            }
        }
        let keep: Vec<bool> = removed.iter().map(|r| !r).collect();
        self.retain(&keep);
        if lip <= 0.0 {
            return BTreeMap::new();
        }
        let mut rim = Vec::new();
        for t in 0..self.tris.len() {
            for (a, b) in self.edges(t) {
                let e = edge(key(self.pos[a as usize]), key(self.pos[b as usize]));
                if gone.contains(&e) && uses.get(&e) == Some(&1) {
                    rim.push((a, b, self.mat[t]));
                }
            }
        }
        let mut normal: BTreeMap<Key, DVec3> = BTreeMap::new();
        for &(a, b, _) in &rim {
            for v in [a, b] {
                *normal.entry(key(self.pos[v as usize])).or_default() += self.nrm[v as usize];
            }
        }
        let below = |p: DVec3, k: &Key| p - normal[k].normalize_or_zero() * lip;
        let mut lowered = BTreeMap::new();
        for &(a, b, _) in &rim {
            for v in [a, b] {
                let p = self.pos[v as usize];
                let k = key(p);
                lowered.insert(k, below(p, &k));
            }
        }
        let mut wall: BTreeMap<Key, DVec3> = BTreeMap::new();
        for &(a, b, _) in &rim {
            let (pa, pb) = (self.pos[a as usize], self.pos[b as usize]);
            let ka = key(pa);
            let n = triangle_normal(&[pb, pa, below(pa, &ka)]);
            *wall.entry(ka).or_default() += n;
            *wall.entry(key(pb)).or_default() += n;
        }
        let mut ring: BTreeMap<(u32, bool), u32> = BTreeMap::new();
        let mut lip_vertex = |w: &mut Work, v: u32, low: bool| -> u32 {
            if let Some(&i) = ring.get(&(v, low)) {
                return i;
            }
            let p = w.pos[v as usize];
            let k = key(p);
            let at = if low { below(p, &k) } else { p };
            let i = w.push_vertex(&[(v, 1.0)], at, Some(wall[&k].normalize_or_zero()));
            ring.insert((v, low), i);
            i
        };
        for (a, b, m) in rim {
            let (ta, tb) = (lip_vertex(self, a, false), lip_vertex(self, b, false));
            let (ba, bb) = (lip_vertex(self, a, true), lip_vertex(self, b, true));
            self.tris.push([tb, ta, ba]);
            self.tris.push([tb, ba, bb]);
            self.mat.extend([m, m]);
        }
        lowered
    }

    fn edges(&self, t: usize) -> [(u32, u32); 3] {
        let [a, b, c] = self.tris[t];
        [(a, b), (b, c), (c, a)]
    }

    pub fn to_mesh(&self, keep: &[bool]) -> Mesh {
        let mut out = Mesh {
            uvs: self.uv.as_ref().map(|_| Vec::new()),
            uvs1: self.uv1.as_ref().map(|_| Vec::new()),
            ..Mesh::default()
        };
        let mut map = vec![u32::MAX; self.pos.len()];
        for (id, material) in self.materials.iter().enumerate() {
            let start = out.indices.len() as u32;
            for t in (0..self.tris.len()).filter(|&t| keep[t] && self.mat[t] == id) {
                for v in self.tris[t] {
                    let v = v as usize;
                    if map[v] == u32::MAX {
                        map[v] = out.positions.len() as u32;
                        out.positions.push(self.pos[v].as_vec3().to_array());
                        out.normals.push(self.nrm[v].as_vec3().to_array());
                        if let (Some(o), Some(u)) = (out.uvs.as_mut(), self.uv.as_ref()) {
                            o.push(u[v].as_vec2().to_array());
                        }
                        if let (Some(o), Some(u)) = (out.uvs1.as_mut(), self.uv1.as_ref()) {
                            o.push(u[v].as_vec2().to_array());
                        }
                    }
                    out.indices.push(map[v]);
                }
            }
            let count = out.indices.len() as u32 - start;
            if let (Some(material), true) = (material, count > 0) {
                out.slots.push(Slot {
                    material: material.clone(),
                    start,
                    count,
                });
            }
        }
        out
    }
}

struct Grid {
    facing: Facing,
    lo: DVec2,
    cell: DVec2,
    dims: [usize; 2],
    pad: f64,
    grain: f64,
    cells: Vec<Vec<u32>>,
}

impl Grid {
    fn new(w: &Work, facing: Facing) -> Grid {
        let mut lo = DVec2::splat(f64::INFINITY);
        let mut hi = DVec2::splat(f64::NEG_INFINITY);
        for p in w
            .tris
            .iter()
            .flatten()
            .map(|&i| facing.project(w.pos[i as usize]))
        {
            lo = lo.min(p);
            hi = hi.max(p);
        }
        if lo.x > hi.x || lo.y > hi.y {
            (lo, hi) = (DVec2::ZERO, DVec2::ZERO);
        }
        let extent = hi - lo;
        let n = w.tris.len().max(1) as f64;
        let area = extent.x * extent.y;
        let side = if area > 0.0 {
            (area / n).sqrt()
        } else {
            extent.max_element() / n
        };
        let count = |e: f64| {
            if side > 0.0 && e > 0.0 {
                ((e / side).ceil() as usize).clamp(1, SIDE)
            } else {
                1
            }
        };
        let dims = [count(extent.x), count(extent.y)];
        let size = |e: f64, d: usize| if e > 0.0 { e / d as f64 } else { 1.0 };
        Grid {
            facing,
            lo,
            cell: DVec2::new(size(extent.x, dims[0]), size(extent.y, dims[1])),
            dims,
            pad: PAD * (extent.length() + lo.abs().max(hi.abs()).max_element()),
            grain: GRAIN * (extent.length() + lo.abs().max(hi.abs()).max_element()),
            cells: vec![Vec::new(); dims[0] * dims[1]],
        }
    }

    fn at(&self, p: DVec2) -> [usize; 2] {
        let f = ((p - self.lo) / self.cell).floor();
        [
            (f.x as usize).min(self.dims[0] - 1),
            (f.y as usize).min(self.dims[1] - 1),
        ]
    }

    fn add(&mut self, w: &Work, t: u32) {
        let q = w.tris[t as usize].map(|i| self.facing.project(w.pos[i as usize]));
        let pad = DVec2::splat(self.pad);
        let [x0, y0] = self.at(q[0].min(q[1]).min(q[2]) - pad);
        let [x1, y1] = self.at(q[0].max(q[1]).max(q[2]) + pad);
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.cells[y * self.dims[0] + x].push(t);
            }
        }
    }

    fn near(&self, p: DVec2, from: u32) -> Vec<u32> {
        let [x, y] = self.at(p);
        let mut found: Vec<u32> = self.cells[y * self.dims[0] + x]
            .iter()
            .copied()
            .filter(|&t| t >= from)
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    fn along(&self, a: DVec2, b: DVec2) -> Vec<u32> {
        let m = self.pad + PAD * a.distance(b);
        let d = b - a;
        let [_, y0] = self.at(DVec2::splat(a.y.min(b.y) - m));
        let [_, y1] = self.at(DVec2::splat(a.y.max(b.y) + m));
        let mut found = Vec::new();
        for y in y0..=y1 {
            let low = if y == 0 {
                f64::NEG_INFINITY
            } else {
                self.lo.y + y as f64 * self.cell.y - m
            };
            let high = if y + 1 == self.dims[1] {
                f64::INFINITY
            } else {
                self.lo.y + (y + 1) as f64 * self.cell.y + m
            };
            let (s0, s1) = if d.y == 0.0 {
                (0.0, 1.0)
            } else {
                let (u, v) = ((low - a.y) / d.y, (high - a.y) / d.y);
                (u.min(v).max(0.0), u.max(v).min(1.0))
            };
            if s0 > s1 {
                continue;
            }
            let (u, v) = (a.x + d.x * s0, a.x + d.x * s1);
            let [x0, _] = self.at(DVec2::splat(u.min(v) - m));
            let [x1, _] = self.at(DVec2::splat(u.max(v) + m));
            for x in x0..=x1 {
                found.extend_from_slice(&self.cells[y * self.dims[0] + x]);
            }
        }
        found.sort_unstable();
        found.dedup();
        found
    }
}

struct Index {
    edges: BTreeMap<Edge, Vec<u32>>,
    grid: Grid,
}

impl Index {
    fn new(w: &Work, facing: Facing) -> Index {
        let mut index = Index {
            edges: BTreeMap::new(),
            grid: Grid::new(w, facing),
        };
        for t in 0..w.tris.len() {
            index.add(w, t as u32);
        }
        index
    }

    fn keys(w: &Work, t: u32) -> [Edge; 3] {
        w.edges(t as usize)
            .map(|(a, b)| edge(key(w.pos[a as usize]), key(w.pos[b as usize])))
    }

    fn add(&mut self, w: &Work, t: u32) {
        for e in Index::keys(w, t) {
            self.edges.entry(e).or_default().push(t);
        }
        self.grid.add(w, t);
    }

    fn remove(&mut self, w: &Work, t: u32) {
        for e in Index::keys(w, t) {
            if let Some(list) = self.edges.get_mut(&e) {
                list.retain(|&x| x != t);
                if list.is_empty() {
                    self.edges.remove(&e);
                }
            }
        }
    }
}

fn split_edge(w: &mut Work, index: &mut Index, e: Edge, s: f64) {
    let Some(list) = index.edges.get(&e).cloned() else {
        return;
    };
    let mut made: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let mut point = None;
    for t in list {
        let tri = w.tris[t as usize];
        let Some(k) = (0..3).find(|&k| {
            edge(
                key(w.pos[tri[k] as usize]),
                key(w.pos[tri[(k + 1) % 3] as usize]),
            ) == e
        }) else {
            continue;
        };
        let (u, v, c) = (tri[k], tri[(k + 1) % 3], tri[(k + 2) % 3]);
        let (pu, pv) = (w.pos[u as usize], w.pos[v as usize]);
        let forward = key(pu) == e.0;
        let along = if forward { s } else { 1.0 - s };
        let x = *point.get_or_insert_with(|| {
            let (a, b) = if forward { (pu, pv) } else { (pv, pu) };
            a + (b - a) * s
        });
        let pair = (u.min(v), u.max(v));
        let m = match made.get(&pair) {
            Some(&m) => m,
            None => {
                let m = w.push_vertex(&[(u, 1.0 - along), (v, along)], x, None);
                made.insert(pair, m);
                m
            }
        };
        index.remove(w, t);
        w.tris[t as usize] = [u, m, c];
        index.add(w, t);
        let j = w.tris.len() as u32;
        w.tris.push([m, v, c]);
        w.mat.push(w.mat[t as usize]);
        index.add(w, j);
    }
}

fn split_triangle(w: &mut Work, index: &mut Index, t: u32, b: [f64; 3]) {
    let [p0, p1, p2] = w.tris[t as usize];
    let x = w.pos[p0 as usize] * b[0] + w.pos[p1 as usize] * b[1] + w.pos[p2 as usize] * b[2];
    let m = w.push_vertex(&[(p0, b[0]), (p1, b[1]), (p2, b[2])], x, None);
    let material = w.mat[t as usize];
    index.remove(w, t);
    w.tris[t as usize] = [p0, p1, m];
    index.add(w, t);
    for tri in [[p1, p2, m], [p2, p0, m]] {
        let j = w.tris.len() as u32;
        w.tris.push(tri);
        w.mat.push(material);
        index.add(w, j);
    }
}

fn barycentric(q: [DVec2; 3], p: DVec2) -> Option<[f64; 3]> {
    let d = (q[1] - q[0]).perp_dot(q[2] - q[0]);
    if d.abs() < 1e-18 {
        return None;
    }
    let b1 = (p - q[0]).perp_dot(q[2] - q[0]) / d;
    let b2 = (q[1] - q[0]).perp_dot(p - q[0]) / d;
    let b = [1.0 - b1 - b2, b1, b2];
    b.iter().all(|&x| x >= -EPS).then_some(b)
}

fn insert(w: &mut Work, index: &mut Index, facing: Facing, p: DVec2) {
    let mut from = 0;
    loop {
        let near = index.grid.near(p, from);
        if near.is_empty() {
            return;
        }
        for t in near {
            visit(w, index, facing, p, t as usize);
            from = t + 1;
        }
    }
}

fn visit(w: &mut Work, index: &mut Index, facing: Facing, p: DVec2, t: usize) {
    if !facing.faces(w.face(t)) {
        return;
    }
    let tri = w.tris[t];
    let q = tri.map(|i| facing.project(w.pos[i as usize]));
    let long = (0..3)
        .map(|k| q[k].distance(q[(k + 1) % 3]))
        .fold(0.0, f64::max);
    if q.iter()
        .any(|c| c.distance(p) <= 2.0 * EPS * long + index.grid.grain)
    {
        return;
    }
    let Some(b) = barycentric(q, p).filter(|b| b.iter().all(|&x| x <= 1.0 - EPS)) else {
        return;
    };
    match (0..3).find(|&k| b[k] < EPS) {
        Some(k) => {
            let (u, v) = (tri[(k + 1) % 3], tri[(k + 2) % 3]);
            let (ku, kv) = (key(w.pos[u as usize]), key(w.pos[v as usize]));
            let e = edge(ku, kv);
            let s = if ku == e.0 {
                b[(k + 2) % 3]
            } else {
                b[(k + 1) % 3]
            };
            split_edge(w, index, e, s);
        }
        None => split_triangle(w, index, t as u32, b),
    }
}

fn crossing(p: DVec2, q: DVec2, a: DVec2, b: DVec2) -> Option<f64> {
    let (r, d) = (q - p, b - a);
    let den = r.perp_dot(d);
    if den.abs() <= f64::EPSILON * r.length() * d.length() {
        return None;
    }
    let s = (a - p).perp_dot(d) / den;
    let u = (a - p).perp_dot(r) / den;
    (s > EPS && s < 1.0 - EPS && (-EPS..=1.0 + EPS).contains(&u)).then_some(s)
}

fn cross(w: &mut Work, index: &mut Index, facing: Facing, a: DVec2, b: DVec2) {
    let (lo, hi) = (a.min(b), a.max(b));
    let mut found: BTreeMap<Edge, f64> = BTreeMap::new();
    for t in index.grid.along(a, b) {
        let t = t as usize;
        let q = w.tris[t].map(|i| facing.project(w.pos[i as usize]));
        let (qlo, qhi) = (q[0].min(q[1]).min(q[2]), q[0].max(q[1]).max(q[2]));
        if qhi.x < lo.x || qhi.y < lo.y || qlo.x > hi.x || qlo.y > hi.y || !facing.faces(w.face(t))
        {
            continue;
        }
        for (u, v) in w.edges(t) {
            let (pu, pv) = (w.pos[u as usize], w.pos[v as usize]);
            let (ku, kv) = (key(pu), key(pv));
            let e = edge(ku, kv);
            if found.contains_key(&e) {
                continue;
            }
            let (pa, pb) = if ku == e.0 { (pu, pv) } else { (pv, pu) };
            if let Some(s) = crossing(facing.project(pa), facing.project(pb), a, b) {
                found.insert(e, s);
            }
        }
    }
    for (e, s) in found {
        split_edge(w, index, e, s);
    }
}

pub(crate) fn imprint(w: &mut Work, facing: Facing, lines: &[(&[DVec2], bool)]) {
    let mut index = Index::new(w, facing);
    for &(line, closed) in lines {
        for &p in line {
            insert(w, &mut index, facing, p);
        }
        let n = line.len();
        let count = if closed { n } else { n.saturating_sub(1) };
        for i in 0..count {
            cross(w, &mut index, facing, line[i], line[(i + 1) % n]);
        }
    }
}

pub(crate) struct Outline<'a> {
    polygon: &'a [DVec2],
    lo: f64,
    step: f64,
    bands: Vec<Vec<u32>>,
}

impl<'a> Outline<'a> {
    pub fn new(polygon: &'a [DVec2]) -> Outline<'a> {
        let n = polygon.len();
        let lo = polygon.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let hi = polygon
            .iter()
            .map(|p| p.y)
            .fold(f64::NEG_INFINITY, f64::max);
        let step = if hi > lo { (hi - lo) / n as f64 } else { 1.0 };
        let mut outline = Outline {
            polygon,
            lo,
            step,
            bands: vec![Vec::new(); n.max(1)],
        };
        for i in 0..n {
            let (a, b) = (polygon[i], polygon[(i + n - 1) % n]);
            for band in outline.band(a.y.min(b.y))..=outline.band(a.y.max(b.y)) {
                outline.bands[band].push(i as u32);
            }
        }
        outline
    }

    fn band(&self, y: f64) -> usize {
        (((y - self.lo) / self.step).floor() as usize).min(self.bands.len() - 1)
    }

    pub fn contains(&self, p: DVec2) -> bool {
        let n = self.polygon.len();
        if n == 0 {
            return false;
        }
        let mut odd = false;
        for &i in &self.bands[self.band(p.y)] {
            let (a, b) = (
                self.polygon[i as usize],
                self.polygon[(i as usize + n - 1) % n],
            );
            if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
                odd = !odd;
            }
        }
        odd
    }
}

pub(crate) fn distance(line: &[DVec2], closed: bool, p: DVec2) -> f64 {
    let n = line.len();
    if n == 1 {
        return p.distance(line[0]);
    }
    let count = if closed { n } else { n - 1 };
    (0..count)
        .map(|i| {
            let (a, b) = (line[i], line[(i + 1) % n]);
            let d = b - a;
            let len = d.length_squared();
            let s = if len > 0.0 {
                ((p - a).dot(d) / len).clamp(0.0, 1.0)
            } else {
                0.0
            };
            p.distance(a + d * s)
        })
        .fold(f64::INFINITY, f64::min)
}

pub(crate) fn polygon(curve: &[DVec2]) -> Vec<DVec2> {
    let mut points = curve.to_vec();
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    points
}

pub(crate) fn list<'a>(
    value: Option<&'a Value>,
    at: &str,
) -> Result<Vec<&'a Map<String, Value>>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value
        .as_array()
        .ok_or_else(|| format!("{at}: expected a list of tables"))?;
    items
        .iter()
        .enumerate()
        .map(|(i, v)| {
            v.as_object()
                .ok_or_else(|| format!("{at}.{i}: expected a table"))
        })
        .collect()
}

pub(crate) fn known(table: &Map<String, Value>, at: &str, fields: &[&str]) -> Result<(), String> {
    match table.keys().find(|k| !fields.contains(&k.as_str())) {
        Some(k) => Err(format!(
            "{at}.{k}: unknown field, expected one of: {}",
            fields.join(", ")
        )),
        None => Ok(()),
    }
}

pub(crate) fn text(
    table: &Map<String, Value>,
    at: &str,
    field: &str,
) -> Result<Option<String>, String> {
    match table.get(field) {
        None => Ok(None),
        Some(v) => v
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| Some(s.to_string()))
            .ok_or_else(|| format!("{at}.{field}: expected a name")),
    }
}

pub(crate) fn name(table: &Map<String, Value>, at: &str) -> Result<String, String> {
    text(table, at, "name")?.ok_or_else(|| format!("{at}.name: required"))
}

pub(crate) fn view(table: &Map<String, Value>, at: &str) -> Result<View, String> {
    let bad = || format!("{at}.view: expected one of: {}", VIEWS.join(", "));
    table
        .get("view")
        .ok_or_else(bad)?
        .as_str()
        .and_then(View::parse)
        .ok_or_else(bad)
}

pub(crate) fn number(
    table: &Map<String, Value>,
    at: &str,
    field: &str,
    fallback: Option<f64>,
    min: f64,
) -> Result<f64, String> {
    match table.get(field) {
        None => fallback.ok_or_else(|| format!("{at}.{field}: required")),
        Some(v) => v
            .as_f64()
            .filter(|x| x.is_finite() && *x >= min)
            .ok_or_else(|| format!("{at}.{field}: expected a number from {min}")),
    }
}

pub(crate) fn curve(
    table: &Map<String, Value>,
    at: &str,
    min: usize,
) -> Result<Vec<DVec2>, String> {
    let bad = || format!("{at}.curve: expected at least {min} points [a, b]");
    let points = table
        .get("curve")
        .and_then(Value::as_array)
        .ok_or_else(bad)?
        .iter()
        .map(|p| {
            let p = p.as_array().filter(|p| p.len() == 2)?;
            let a = p[0].as_f64().filter(|x| x.is_finite())?;
            let b = p[1].as_f64().filter(|x| x.is_finite())?;
            Some(DVec2::new(a, b))
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(bad)?;
    if points.len() < min {
        return Err(bad());
    }
    Ok(points)
}

pub(crate) fn unique<'a>(names: impl Iterator<Item = &'a str>, at: &str) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for (i, n) in names.enumerate() {
        if !seen.insert(n) {
            return Err(format!("{at}.{i}.name: `{n}` is used twice"));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod test {
    use super::{Key, key};
    use glam::{DVec3, Vec3};
    use pfx_mesh::Mesh;
    use std::collections::BTreeMap;

    pub fn body() -> Mesh {
        pfx_mesh::rounded_box([2.0, 1.2, 4.0], 0.15, 4)
    }

    pub fn point(m: &Mesh, i: u32) -> DVec3 {
        Vec3::from(m.positions[i as usize]).as_dvec3()
    }

    pub fn open_edges(meshes: &[&Mesh]) -> Vec<(Key, Key)> {
        let mut count: BTreeMap<(Key, Key), i64> = BTreeMap::new();
        for m in meshes {
            for t in m.indices.chunks_exact(3) {
                for k in 0..3 {
                    let (a, b) = (key(point(m, t[k])), key(point(m, t[(k + 1) % 3])));
                    if a != b {
                        *count.entry((a, b)).or_default() += 1;
                        *count.entry((b, a)).or_default() -= 1;
                    }
                }
            }
        }
        count
            .into_iter()
            .filter(|(_, c)| *c > 0)
            .map(|(e, _)| e)
            .collect()
    }

    pub fn area(m: &Mesh) -> f64 {
        pfx_mesh::area(m)
    }
}
