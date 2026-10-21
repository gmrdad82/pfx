type V3 = [f32; 3];

const MANIFOLD: u8 = 0;
const BORDER: u8 = 1;
const SEAM: u8 = 2;
const COMPLEX: u8 = 3;
const LOCKED: u8 = 4;

const NONE: u32 = u32::MAX;
const EDGE_WEIGHT_SEAM: f32 = 0.5;
const EDGE_WEIGHT_BORDER: f32 = 10.0;
const VERTEX_FACTOR: f32 = 1e-7;
const SORT_BITS: u32 = 12;
const SORT_BINS: usize = 2048 + 512;

const CAN_COLLAPSE: [[u8; 5]; 5] = [
    [1, 1, 1, 1, 1],
    [0, 1, 0, 0, 1],
    [0, 0, 1, 0, 1],
    [0, 0, 0, 1, 1],
    [0, 0, 0, 0, 0],
];

const HAS_OPPOSITE: [[u8; 5]; 5] = [
    [1, 1, 1, 1, 1],
    [1, 0, 1, 0, 0],
    [1, 1, 1, 0, 1],
    [1, 0, 0, 0, 0],
    [1, 0, 1, 0, 0],
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub lock_border: bool,
    pub error_absolute: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Simplified {
    pub indices: Vec<u32>,
    pub error: f32,
}

pub struct Input<'a> {
    pub indices: &'a [u32],
    pub positions: &'a [[f32; 3]],
    pub attributes: &'a [f32],
    pub weights: &'a [f32],
    pub locks: &'a [bool],
}

pub fn simplify(
    input: &Input,
    target_index_count: usize,
    target_error: f32,
    options: Options,
) -> Simplified {
    let mut result = input.indices.to_vec();
    let n = input.positions.len();
    let stride = input.weights.len();
    let valid = result.len() % 3 == 0
        && result.iter().all(|&i| (i as usize) < n)
        && input.attributes.len() == n * stride
        && (input.locks.is_empty() || input.locks.len() == n)
        && n < NONE as usize;
    if !valid {
        return Simplified {
            indices: result,
            error: 0.0,
        };
    }
    let mut adjacency = Adjacency::new(n, result.len());
    adjacency.update(&result, None);
    let (remap, wedge) = position_remap(input.positions);
    let (kind, mut lp, mut lb) = classify(&adjacency, &remap, &wedge, input.locks, options);
    let (points, scale) = rescale(input.positions);
    let used: Vec<usize> = (0..stride).filter(|&k| input.weights[k] > 0.0).collect();
    let count = used.len();
    let mut attributes = Vec::with_capacity(n * count);
    for i in 0..n {
        for &k in &used {
            attributes.push(input.attributes[i * stride + k] * input.weights[k]);
        }
    }
    let mut q = Quadrics {
        vertex: vec![Quadric::default(); n],
        attribute: vec![Quadric::default(); if count > 0 { n } else { 0 }],
        grads: vec![Grad::default(); n * count],
        count,
    };
    fill_face(&mut q.vertex, &result, &points, &remap);
    fill_vertex(&mut q.vertex, &points, &remap);
    fill_edge(
        &mut q.vertex,
        &result,
        &points,
        &Topology {
            remap: &remap,
            wedge: &wedge,
            kind: &kind,
            lp: &lp,
            lb: &lb,
        },
    );
    if count > 0 {
        fill_attribute(&mut q, &result, &points, &attributes);
    }
    let capacity = bound(&adjacency, &kind, result.len());
    let mut collapses = Vec::with_capacity(capacity);
    let mut order = Vec::with_capacity(capacity);
    let mut collapse_remap = vec![0u32; n];
    let mut locked = vec![false; n];
    let mut result_error = 0.0f32;
    let error_scale = if options.error_absolute { scale } else { 1.0 };
    let error_limit = (target_error * target_error) / (error_scale * error_scale);
    while result.len() > target_index_count {
        adjacency.update(&result, Some(&remap));
        let topo = Topology {
            remap: &remap,
            wedge: &wedge,
            kind: &kind,
            lp: &lp,
            lb: &lb,
        };
        pick(&mut collapses, capacity, &result, &topo);
        if collapses.is_empty() {
            break;
        }
        rank(&mut collapses, &points, &attributes, &q, &topo);
        sort(&mut order, &collapses);
        let goal = (result.len() - target_index_count) / 3;
        for (i, c) in collapse_remap.iter_mut().enumerate() {
            *c = i as u32;
        }
        locked.fill(false);
        let done = perform(
            Pass {
                collapse_remap: &mut collapse_remap,
                locked: &mut locked,
                result_error: &mut result_error,
                error_limit,
            },
            &collapses,
            &order,
            &topo,
            &points,
            &adjacency,
            goal,
        );
        if done == 0 {
            break;
        }
        update_quadrics(&collapse_remap, &mut q, &remap);
        remap_edge_loops(&mut lp, &collapse_remap);
        remap_edge_loops(&mut lb, &collapse_remap);
        remap_index_buffer(&mut result, &collapse_remap, &remap);
    }
    Simplified {
        indices: result,
        error: result_error.sqrt() * error_scale,
    }
}

struct Adjacency {
    offsets: Vec<u32>,
    next: Vec<u32>,
    prev: Vec<u32>,
}

impl Adjacency {
    fn new(vertex_count: usize, index_count: usize) -> Adjacency {
        Adjacency {
            offsets: vec![0; vertex_count + 1],
            next: vec![0; index_count],
            prev: vec![0; index_count],
        }
    }

    fn update(&mut self, indices: &[u32], remap: Option<&[u32]>) {
        let vertex_count = self.offsets.len() - 1;
        let map = |i: u32| remap.map_or(i, |r| r[i as usize]);
        self.offsets[1..].fill(0);
        for &i in indices {
            self.offsets[map(i) as usize + 1] += 1;
        }
        let mut offset = 0;
        for i in 1..=vertex_count {
            let count = self.offsets[i];
            self.offsets[i] = offset;
            offset += count;
        }
        for t in indices.chunks_exact(3) {
            let (a, b, c) = (map(t[0]), map(t[1]), map(t[2]));
            for (v, next, prev) in [(a, b, c), (b, c, a), (c, a, b)] {
                let slot = &mut self.offsets[v as usize + 1];
                self.next[*slot as usize] = next;
                self.prev[*slot as usize] = prev;
                *slot += 1;
            }
        }
        self.offsets[0] = 0;
    }

    fn edges(&self, v: u32) -> std::ops::Range<usize> {
        self.offsets[v as usize] as usize..self.offsets[v as usize + 1] as usize
    }

    fn has_edge(&self, a: u32, b: u32) -> bool {
        self.edges(a).any(|e| self.next[e] == b)
    }
}

fn position_hash(p: V3) -> u32 {
    let key = p.map(|v| {
        let bits = v.to_bits();
        let bits = if bits == 0x8000_0000 { 0 } else { bits };
        bits ^ (bits >> 17)
    });
    key[0].wrapping_mul(73856093) ^ key[1].wrapping_mul(19349663) ^ key[2].wrapping_mul(83492791)
}

fn hash_buckets(count: usize) -> usize {
    let mut buckets = 1;
    while buckets < count + count / 4 {
        buckets *= 2;
    }
    buckets
}

fn position_remap(positions: &[V3]) -> (Vec<u32>, Vec<u32>) {
    let n = positions.len();
    let size = hash_buckets(n);
    let mask = size - 1;
    let mut table = vec![NONE; size];
    let mut remap = vec![0u32; n];
    for (i, &p) in positions.iter().enumerate() {
        let mut bucket = position_hash(p) as usize & mask;
        let mut probe = 0;
        remap[i] = loop {
            let item = table[bucket];
            if item == NONE {
                table[bucket] = i as u32;
                break i as u32;
            }
            if positions[item as usize] == p {
                break item;
            }
            bucket = (bucket + probe + 1) & mask;
            probe += 1;
        };
    }
    let mut wedge: Vec<u32> = (0..n as u32).collect();
    for i in 0..n {
        let r = remap[i] as usize;
        if r != i {
            wedge[i] = wedge[r];
            wedge[r] = i as u32;
        }
    }
    (remap, wedge)
}

fn classify(
    adjacency: &Adjacency,
    remap: &[u32],
    wedge: &[u32],
    locks: &[bool],
    options: Options,
) -> (Vec<u8>, Vec<u32>, Vec<u32>) {
    let n = remap.len();
    let mut openout = vec![NONE; n];
    let mut openinc = vec![NONE; n];
    for vertex in 0..n as u32 {
        for e in adjacency.edges(vertex) {
            let target = adjacency.next[e];
            if target == vertex {
                openinc[vertex as usize] = vertex;
                openout[vertex as usize] = vertex;
            } else if !adjacency.has_edge(target, vertex) {
                let inc = &mut openinc[target as usize];
                *inc = if *inc == NONE { vertex } else { target };
                let out = &mut openout[vertex as usize];
                *out = if *out == NONE { target } else { vertex };
            }
        }
    }
    let mut kind = vec![MANIFOLD; n];
    for i in 0..n {
        let iu = i as u32;
        if remap[i] != iu {
            kind[i] = kind[remap[i] as usize];
            continue;
        }
        kind[i] = if wedge[i] == iu {
            let (openi, openo) = (openinc[i], openout[i]);
            if openi == NONE && openo == NONE {
                MANIFOLD
            } else if openi != NONE
                && openo != NONE
                && remap[openi as usize] == remap[openo as usize]
                && openi != iu
            {
                SEAM
            } else if openi != iu && openo != iu {
                BORDER
            } else {
                LOCKED
            }
        } else if wedge[wedge[i] as usize] == iu {
            let w = wedge[i];
            let (openiv, openov) = (openinc[i], openout[i]);
            let (openiw, openow) = (openinc[w as usize], openout[w as usize]);
            if openiv != NONE
                && openiv != iu
                && openov != NONE
                && openov != iu
                && openiw != NONE
                && openiw != w
                && openow != NONE
                && openow != w
            {
                let r = |v: u32| remap[v as usize];
                if r(openiv) == r(openow) && r(openov) == r(openiw) && r(openiv) != r(openov) {
                    SEAM
                } else {
                    LOCKED
                }
            } else {
                LOCKED
            }
        } else {
            LOCKED
        };
    }
    if !locks.is_empty() {
        for i in 0..n {
            if locks[i] {
                kind[remap[i] as usize] = LOCKED;
            }
        }
        for i in 0..n {
            if kind[remap[i] as usize] == LOCKED {
                kind[i] = LOCKED;
            }
        }
    }
    if options.lock_border {
        for k in kind.iter_mut() {
            if *k == BORDER {
                *k = LOCKED;
            }
        }
    }
    (kind, openout, openinc)
}

fn rescale(positions: &[V3]) -> (Vec<V3>, f32) {
    let mut minv = [f32::MAX; 3];
    let mut maxv = [-f32::MAX; 3];
    for p in positions {
        for j in 0..3 {
            let v = p[j];
            minv[j] = if minv[j] > v { v } else { minv[j] };
            maxv[j] = if maxv[j] < v { v } else { maxv[j] };
        }
    }
    let mut extent = 0.0f32;
    for j in 0..3 {
        let d = maxv[j] - minv[j];
        extent = if d < extent { extent } else { d };
    }
    let scale = if extent == 0.0 { 0.0 } else { 1.0 / extent };
    let points = positions
        .iter()
        .map(|p| {
            [
                (p[0] - minv[0]) * scale,
                (p[1] - minv[1]) * scale,
                (p[2] - minv[2]) * scale,
            ]
        })
        .collect();
    (points, extent)
}

#[derive(Clone, Copy, Debug, Default)]
struct Quadric {
    a00: f32,
    a11: f32,
    a22: f32,
    a10: f32,
    a20: f32,
    a21: f32,
    b0: f32,
    b1: f32,
    b2: f32,
    c: f32,
    w: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Grad {
    gx: f32,
    gy: f32,
    gz: f32,
    gw: f32,
}

#[derive(Clone, Copy, Debug)]
struct Collapse {
    v0: u32,
    v1: u32,
    bidi: bool,
    error: f32,
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: &mut V3) -> f32 {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length > 0.0 {
        v[0] /= length;
        v[1] /= length;
        v[2] /= length;
    }
    length
}

impl Quadric {
    fn add(&mut self, r: &Quadric) {
        self.a00 += r.a00;
        self.a11 += r.a11;
        self.a22 += r.a22;
        self.a10 += r.a10;
        self.a20 += r.a20;
        self.a21 += r.a21;
        self.b0 += r.b0;
        self.b1 += r.b1;
        self.b2 += r.b2;
        self.c += r.c;
        self.w += r.w;
    }

    fn eval(&self, v: V3) -> f32 {
        let mut rx = self.b0;
        let mut ry = self.b1;
        let mut rz = self.b2;
        rx += self.a10 * v[1];
        ry += self.a21 * v[2];
        rz += self.a20 * v[0];
        rx *= 2.0;
        ry *= 2.0;
        rz *= 2.0;
        rx += self.a00 * v[0];
        ry += self.a11 * v[1];
        rz += self.a22 * v[2];
        let mut r = self.c;
        r += rx * v[0];
        r += ry * v[1];
        r += rz * v[2];
        r
    }

    fn error(&self, v: V3) -> f32 {
        let r = self.eval(v);
        let s = if self.w == 0.0 { 0.0 } else { 1.0 / self.w };
        r.abs() * s
    }

    fn attribute_error(&self, g: &[Grad], v: V3, va: &[f32]) -> f32 {
        let mut r = self.eval(v);
        for (g, &a) in g.iter().zip(va) {
            let d = v[0] * g.gx + v[1] * g.gy + v[2] * g.gz + g.gw;
            r += a * (a * self.w - 2.0 * d);
        }
        r.abs()
    }

    fn from_plane(a: f32, b: f32, c: f32, d: f32, w: f32) -> Quadric {
        let aw = a * w;
        let bw = b * w;
        let cw = c * w;
        let dw = d * w;
        Quadric {
            a00: a * aw,
            a11: b * bw,
            a22: c * cw,
            a10: a * bw,
            a20: a * cw,
            a21: b * cw,
            b0: a * dw,
            b1: b * dw,
            b2: c * dw,
            c: d * dw,
            w,
        }
    }

    fn from_point(x: f32, y: f32, z: f32, w: f32) -> Quadric {
        Quadric {
            a00: w,
            a11: w,
            a22: w,
            a10: 0.0,
            a20: 0.0,
            a21: 0.0,
            b0: -x * w,
            b1: -y * w,
            b2: -z * w,
            c: (x * x + y * y + z * z) * w,
            w,
        }
    }

    fn from_triangle(p0: V3, p1: V3, p2: V3, weight: f32) -> Quadric {
        let mut normal = cross(sub(p1, p0), sub(p2, p0));
        let area = normalize(&mut normal);
        let distance = normal[0] * p0[0] + normal[1] * p0[1] + normal[2] * p0[2];
        Quadric::from_plane(
            normal[0],
            normal[1],
            normal[2],
            -distance,
            area.sqrt() * weight,
        )
    }

    fn from_triangle_edge(p0: V3, p1: V3, p2: V3, weight: f32) -> Quadric {
        let p10 = sub(p1, p0);
        let lengthsq = p10[0] * p10[0] + p10[1] * p10[1] + p10[2] * p10[2];
        let length = lengthsq.sqrt();
        let p20 = sub(p2, p0);
        let p20p = p20[0] * p10[0] + p20[1] * p10[1] + p20[2] * p10[2];
        let mut perp = [
            p20[0] * lengthsq - p10[0] * p20p,
            p20[1] * lengthsq - p10[1] * p20p,
            p20[2] * lengthsq - p10[2] * p20p,
        ];
        normalize(&mut perp);
        let distance = perp[0] * p0[0] + perp[1] * p0[1] + perp[2] * p0[2];
        Quadric::from_plane(perp[0], perp[1], perp[2], -distance, length * weight)
    }

    fn from_attributes(
        g: &mut [Grad],
        p: [V3; 3],
        va0: &[f32],
        va1: &[f32],
        va2: &[f32],
    ) -> Quadric {
        let [p0, p1, p2] = p;
        let v0 = sub(p1, p0);
        let v1 = sub(p2, p0);
        let normal = cross(v0, v1);
        let area =
            (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt() * 0.5;
        let w = area;
        let d00 = v0[0] * v0[0] + v0[1] * v0[1] + v0[2] * v0[2];
        let d01 = v0[0] * v1[0] + v0[1] * v1[1] + v0[2] * v1[2];
        let d11 = v1[0] * v1[0] + v1[1] * v1[1] + v1[2] * v1[2];
        let denom = d00 * d11 - d01 * d01;
        let denomr = if denom == 0.0 { 0.0 } else { 1.0 / denom };
        let gx1 = (d11 * v0[0] - d01 * v1[0]) * denomr;
        let gx2 = (d00 * v1[0] - d01 * v0[0]) * denomr;
        let gy1 = (d11 * v0[1] - d01 * v1[1]) * denomr;
        let gy2 = (d00 * v1[1] - d01 * v0[1]) * denomr;
        let gz1 = (d11 * v0[2] - d01 * v1[2]) * denomr;
        let gz2 = (d00 * v1[2] - d01 * v0[2]) * denomr;
        let mut q = Quadric {
            w,
            ..Quadric::default()
        };
        for (k, grad) in g.iter_mut().enumerate() {
            let (a0, a1, a2) = (va0[k], va1[k], va2[k]);
            let gx = gx1 * (a1 - a0) + gx2 * (a2 - a0);
            let gy = gy1 * (a1 - a0) + gy2 * (a2 - a0);
            let gz = gz1 * (a1 - a0) + gz2 * (a2 - a0);
            let gw = a0 - p0[0] * gx - p0[1] * gy - p0[2] * gz;
            q.a00 += w * (gx * gx);
            q.a11 += w * (gy * gy);
            q.a22 += w * (gz * gz);
            q.a10 += w * (gy * gx);
            q.a20 += w * (gz * gx);
            q.a21 += w * (gz * gy);
            q.b0 += w * (gx * gw);
            q.b1 += w * (gy * gw);
            q.b2 += w * (gz * gw);
            q.c += w * (gw * gw);
            *grad = Grad {
                gx: w * gx,
                gy: w * gy,
                gz: w * gz,
                gw: w * gw,
            };
        }
        q
    }
}

fn add_grads(g: &mut [Grad], r: &[Grad]) {
    for (g, r) in g.iter_mut().zip(r) {
        g.gx += r.gx;
        g.gy += r.gy;
        g.gz += r.gz;
        g.gw += r.gw;
    }
}

struct Quadrics {
    vertex: Vec<Quadric>,
    attribute: Vec<Quadric>,
    grads: Vec<Grad>,
    count: usize,
}

impl Quadrics {
    fn attribute_error(&self, i: u32, v: V3, va: &[f32]) -> f32 {
        let (i, k) = (i as usize, self.count);
        self.attribute[i].attribute_error(&self.grads[i * k..(i + 1) * k], v, va)
    }
}

struct Topology<'a> {
    remap: &'a [u32],
    wedge: &'a [u32],
    kind: &'a [u8],
    lp: &'a [u32],
    lb: &'a [u32],
}

impl Topology<'_> {
    fn seam_pair(&self, i0: u32, i1: u32) -> (u32, u32) {
        let s0 = self.wedge[i0 as usize];
        let s1 = if self.lp[i0 as usize] == i1 {
            self.lb[s0 as usize]
        } else {
            self.lp[s0 as usize]
        };
        let s1 = if s1 != NONE {
            s1
        } else {
            self.wedge[i1 as usize]
        };
        (s0, s1)
    }
}

fn fill_face(q: &mut [Quadric], indices: &[u32], points: &[V3], remap: &[u32]) {
    for t in indices.chunks_exact(3) {
        let (i0, i1, i2) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let face = Quadric::from_triangle(points[i0], points[i1], points[i2], 1.0);
        q[remap[i0] as usize].add(&face);
        q[remap[i1] as usize].add(&face);
        q[remap[i2] as usize].add(&face);
    }
}

fn fill_vertex(q: &mut [Quadric], points: &[V3], remap: &[u32]) {
    for (i, p) in points.iter().enumerate() {
        if remap[i] as usize != i {
            continue;
        }
        let w = q[i].w * VERTEX_FACTOR;
        let point = Quadric::from_point(p[0], p[1], p[2], w);
        q[i].add(&point);
    }
}

fn fill_edge(q: &mut [Quadric], indices: &[u32], points: &[V3], topo: &Topology) {
    const NEXT: [usize; 4] = [1, 2, 0, 1];
    let open = |k: u8| k == BORDER || k == SEAM;
    for t in indices.chunks_exact(3) {
        for e in 0..3 {
            let i0 = t[e];
            let i1 = t[NEXT[e]];
            let k0 = topo.kind[i0 as usize];
            let k1 = topo.kind[i1 as usize];
            if !open(k0) && !open(k1) {
                continue;
            }
            if open(k0) && topo.lp[i0 as usize] != i1 {
                continue;
            }
            if open(k1) && topo.lb[i1 as usize] != i0 {
                continue;
            }
            let i2 = t[NEXT[e + 1]];
            let weight = if k0 == BORDER || k1 == BORDER {
                EDGE_WEIGHT_BORDER
            } else {
                EDGE_WEIGHT_SEAM
            };
            let (p0, p1, p2) = (
                points[i0 as usize],
                points[i1 as usize],
                points[i2 as usize],
            );
            let mut edge = Quadric::from_triangle_edge(p0, p1, p2, weight);
            let mut face = Quadric::from_triangle(p0, p1, p2, weight);
            face.w = 0.0;
            edge.add(&face);
            q[topo.remap[i0 as usize] as usize].add(&edge);
            q[topo.remap[i1 as usize] as usize].add(&edge);
        }
    }
}

fn fill_attribute(q: &mut Quadrics, indices: &[u32], points: &[V3], attributes: &[f32]) {
    let k = q.count;
    let mut g = vec![Grad::default(); k];
    for t in indices.chunks_exact(3) {
        let (i0, i1, i2) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let qa = Quadric::from_attributes(
            &mut g,
            [points[i0], points[i1], points[i2]],
            &attributes[i0 * k..(i0 + 1) * k],
            &attributes[i1 * k..(i1 + 1) * k],
            &attributes[i2 * k..(i2 + 1) * k],
        );
        for i in [i0, i1, i2] {
            q.attribute[i].add(&qa);
            add_grads(&mut q.grads[i * k..(i + 1) * k], &g);
        }
    }
}

fn has_flip(a: V3, b: V3, c: V3, d: V3) -> bool {
    let eb = sub(b, a);
    let ec = sub(c, a);
    let ed = sub(d, a);
    let nbc = cross(eb, ec);
    let nbd = cross(eb, ed);
    let ndp = nbc[0] * nbd[0] + nbc[1] * nbd[1] + nbc[2] * nbd[2];
    let abc = nbc[0] * nbc[0] + nbc[1] * nbc[1] + nbc[2] * nbc[2];
    let abd = nbd[0] * nbd[0] + nbd[1] * nbd[1] + nbd[2] * nbd[2];
    ndp <= 0.25 * (abc * abd).sqrt()
}

fn has_flips(
    adjacency: &Adjacency,
    points: &[V3],
    collapse_remap: &[u32],
    i0: u32,
    i1: u32,
) -> bool {
    let v0 = points[i0 as usize];
    let v1 = points[i1 as usize];
    for e in adjacency.edges(i0) {
        let a = collapse_remap[adjacency.next[e] as usize];
        let b = collapse_remap[adjacency.prev[e] as usize];
        if a == i1 || b == i1 || a == b {
            continue;
        }
        if has_flip(points[a as usize], points[b as usize], v0, v1) {
            return true;
        }
    }
    false
}

fn bound(adjacency: &Adjacency, kind: &[u8], index_count: usize) -> usize {
    let dual: usize = kind
        .iter()
        .enumerate()
        .filter(|&(_, &k)| k == MANIFOLD || k == SEAM)
        .map(|(i, _)| adjacency.edges(i as u32).len())
        .sum();
    (index_count - dual / 2) + 3
}

fn pick(collapses: &mut Vec<Collapse>, capacity: usize, indices: &[u32], topo: &Topology) {
    const NEXT: [usize; 3] = [1, 2, 0];
    collapses.clear();
    for t in indices.chunks_exact(3) {
        if collapses.len() + 3 > capacity {
            break;
        }
        for e in 0..3 {
            let i0 = t[e];
            let i1 = t[NEXT[e]];
            let (r0, r1) = (topo.remap[i0 as usize], topo.remap[i1 as usize]);
            if r0 == r1 {
                continue;
            }
            let k0 = topo.kind[i0 as usize];
            let k1 = topo.kind[i1 as usize];
            let (k0u, k1u) = (k0 as usize, k1 as usize);
            if CAN_COLLAPSE[k0u][k1u] | CAN_COLLAPSE[k1u][k0u] == 0 {
                continue;
            }
            if HAS_OPPOSITE[k0u][k1u] != 0 && r1 > r0 {
                continue;
            }
            if (k0 == BORDER || k0 == SEAM) && k1 != MANIFOLD && topo.lp[i0 as usize] != i1 {
                continue;
            }
            if (k1 == BORDER || k1 == SEAM) && k0 != MANIFOLD && topo.lb[i1 as usize] != i0 {
                continue;
            }
            if CAN_COLLAPSE[k0u][k1u] & CAN_COLLAPSE[k1u][k0u] != 0 {
                collapses.push(Collapse {
                    v0: i0,
                    v1: i1,
                    bidi: true,
                    error: 0.0,
                });
            } else {
                let forward = CAN_COLLAPSE[k0u][k1u] != 0;
                collapses.push(Collapse {
                    v0: if forward { i0 } else { i1 },
                    v1: if forward { i1 } else { i0 },
                    bidi: false,
                    error: 0.0,
                });
            }
        }
    }
}

fn rank(
    collapses: &mut [Collapse],
    points: &[V3],
    attributes: &[f32],
    q: &Quadrics,
    topo: &Topology,
) {
    let k = q.count;
    let va = |i: u32| &attributes[i as usize * k..(i as usize + 1) * k];
    for c in collapses.iter_mut() {
        let (i0, i1, bidi) = (c.v0, c.v1, c.bidi);
        let (p0, p1) = (points[i0 as usize], points[i1 as usize]);
        let mut ei = q.vertex[topo.remap[i0 as usize] as usize].error(p1);
        let mut ej = if bidi {
            q.vertex[topo.remap[i1 as usize] as usize].error(p0)
        } else {
            f32::MAX
        };
        if k > 0 {
            ei += q.attribute_error(i0, p1, va(i1));
            ej += if bidi {
                q.attribute_error(i1, p0, va(i0))
            } else {
                0.0
            };
            if topo.kind[i0 as usize] == SEAM {
                let (s0, s1) = topo.seam_pair(i0, i1);
                ei += q.attribute_error(s0, points[s1 as usize], va(s1));
                ej += if bidi {
                    q.attribute_error(s1, points[s0 as usize], va(s0))
                } else {
                    0.0
                };
            } else {
                if topo.kind[i0 as usize] == COMPLEX {
                    let mut v = topo.wedge[i0 as usize];
                    while v != i0 {
                        ei += q.attribute_error(v, p1, va(i1));
                        v = topo.wedge[v as usize];
                    }
                }
                if topo.kind[i1 as usize] == COMPLEX && bidi {
                    let mut v = topo.wedge[i1 as usize];
                    while v != i1 {
                        ej += q.attribute_error(v, p0, va(i0));
                        v = topo.wedge[v as usize];
                    }
                }
            }
        }
        let rev = bidi & (ej < ei);
        c.v0 = if rev { i1 } else { i0 };
        c.v1 = if rev { i0 } else { i1 };
        c.error = if ej < ei { ej } else { ei };
    }
}

fn sort_key(c: &Collapse) -> usize {
    let key = ((c.error.to_bits() << 1) >> (32 - SORT_BITS)) as usize;
    key.min(SORT_BINS - 1)
}

fn sort(order: &mut Vec<u32>, collapses: &[Collapse]) {
    let mut histogram = vec![0u32; SORT_BINS];
    for c in collapses {
        histogram[sort_key(c)] += 1;
    }
    let mut sum = 0u32;
    for h in histogram.iter_mut() {
        let count = *h;
        *h = sum;
        sum += count;
    }
    order.clear();
    order.resize(collapses.len(), 0);
    for (i, c) in collapses.iter().enumerate() {
        let slot = &mut histogram[sort_key(c)];
        order[*slot as usize] = i as u32;
        *slot += 1;
    }
}

struct Pass<'a> {
    collapse_remap: &'a mut [u32],
    locked: &'a mut [bool],
    result_error: &'a mut f32,
    error_limit: f32,
}

fn perform(
    pass: Pass,
    collapses: &[Collapse],
    order: &[u32],
    topo: &Topology,
    points: &[V3],
    adjacency: &Adjacency,
    triangle_goal: usize,
) -> usize {
    let Pass {
        collapse_remap,
        locked,
        result_error,
        error_limit,
    } = pass;
    let mut edge_collapses = 0;
    let mut triangle_collapses = 0;
    let mut edge_goal = triangle_goal / 2;
    for &index in order {
        let c = &collapses[index as usize];
        if c.error > error_limit {
            break;
        }
        if triangle_collapses >= triangle_goal {
            break;
        }
        let error_goal = if edge_goal < collapses.len() {
            1.5 * collapses[order[edge_goal] as usize].error
        } else {
            f32::MAX
        };
        if c.error > error_goal && c.error > *result_error && triangle_collapses > triangle_goal / 6
        {
            break;
        }
        let (i0, i1) = (c.v0, c.v1);
        let (r0, r1) = (topo.remap[i0 as usize], topo.remap[i1 as usize]);
        let kind = topo.kind[i0 as usize];
        if locked[r0 as usize] | locked[r1 as usize] {
            continue;
        }
        if has_flips(adjacency, points, collapse_remap, r0, r1) {
            edge_goal += 1;
            continue;
        }
        if kind == COMPLEX {
            let mut v = i0;
            loop {
                collapse_remap[v as usize] = i1;
                v = topo.wedge[v as usize];
                if v == i0 {
                    break;
                }
            }
        } else if kind == SEAM {
            let (s0, s1) = topo.seam_pair(i0, i1);
            collapse_remap[i0 as usize] = i1;
            collapse_remap[s0 as usize] = s1;
        } else {
            collapse_remap[i0 as usize] = i1;
        }
        locked[r0 as usize] = true;
        locked[r1 as usize] = true;
        triangle_collapses += if kind == BORDER { 1 } else { 2 };
        edge_collapses += 1;
        *result_error = if *result_error < c.error {
            c.error
        } else {
            *result_error
        };
    }
    edge_collapses
}

fn update_quadrics(collapse_remap: &[u32], q: &mut Quadrics, remap: &[u32]) {
    let k = q.count;
    for (i, &target) in collapse_remap.iter().enumerate() {
        if target as usize == i {
            continue;
        }
        let i1 = target as usize;
        let r0 = remap[i] as usize;
        let r1 = remap[i1] as usize;
        if i == r0 {
            let moved = q.vertex[r0];
            q.vertex[r1].add(&moved);
        }
        if k > 0 {
            let moved = q.attribute[i];
            q.attribute[i1].add(&moved);
            let (lo, hi) = (i.min(i1), i.max(i1));
            let (head, tail) = q.grads.split_at_mut(hi * k);
            let (from, to) = if i < i1 {
                (&head[lo * k..(lo + 1) * k], &mut tail[..k])
            } else {
                (&tail[..k], &mut head[lo * k..(lo + 1) * k])
            };
            add_grads(to, from);
        }
    }
}

fn remap_edge_loops(lp: &mut [u32], collapse_remap: &[u32]) {
    for i in 0..lp.len() {
        if lp[i] != NONE {
            let l = lp[i];
            let r = collapse_remap[l as usize];
            lp[i] = if i as u32 == r {
                if lp[l as usize] != NONE {
                    collapse_remap[lp[l as usize] as usize]
                } else {
                    NONE
                }
            } else {
                r
            };
        }
    }
}

fn remap_index_buffer(indices: &mut Vec<u32>, collapse_remap: &[u32], remap: &[u32]) {
    let mut write = 0;
    for i in (0..indices.len()).step_by(3) {
        let v0 = collapse_remap[indices[i] as usize];
        let v1 = collapse_remap[indices[i + 1] as usize];
        let v2 = collapse_remap[indices[i + 2] as usize];
        let (r0, r1, r2) = (remap[v0 as usize], remap[v1 as usize], remap[v2 as usize]);
        if r0 != r1 && r0 != r2 && r1 != r2 {
            indices[write] = v0;
            indices[write + 1] = v1;
            indices[write + 2] = v2;
            write += 3;
        }
    }
    indices.truncate(write);
}
