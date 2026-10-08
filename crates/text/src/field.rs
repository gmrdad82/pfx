use crate::{GlyphImage, MSDF_SPREAD as SPREAD};

const CORNER: f32 = 0.75;
const CLASH: f32 = 1.001 / SPREAD;
const COLORS: [u8; 3] = [0b011, 0b110, 0b101];
const CHUNK: usize = 8;
const GROUP: usize = 8;
const MARGIN: f32 = 0.01;
const SLACK: f32 = 0.05;
const GAP: f32 = 1e-3;

#[derive(Clone, Copy)]
pub(crate) struct Edge {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub mask: u8,
}

pub(crate) fn crossing(p: [f32; 2], edge: &Edge) -> i32 {
    let (a, b) = (edge.a, edge.b);
    if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
    {
        if b[1] > a[1] { 1 } else { -1 }
    } else {
        0
    }
}

pub(crate) struct Rows {
    base: f32,
    buckets: Vec<Vec<usize>>,
}

impl Rows {
    pub(crate) fn new(edges: &[Edge]) -> Self {
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for edge in edges {
            lo = lo.min(edge.a[1]).min(edge.b[1]);
            hi = hi.max(edge.a[1]).max(edge.b[1]);
        }
        if !lo.is_finite() || !hi.is_finite() {
            return Self {
                base: 0.0,
                buckets: Vec::new(),
            };
        }
        let base = lo.floor();
        let mut buckets = vec![Vec::new(); (hi.floor() - base) as usize + 1];
        for (index, edge) in edges.iter().enumerate() {
            let first = (edge.a[1].min(edge.b[1]).floor() - base) as usize;
            let last = (edge.a[1].max(edge.b[1]).floor() - base) as usize;
            for bucket in &mut buckets[first..=last] {
                bucket.push(index);
            }
        }
        Self { base, buckets }
    }

    fn bucket(&self, y: f32) -> &[usize] {
        let row = (y.floor() - self.base) as isize;
        usize::try_from(row)
            .ok()
            .and_then(|row| self.buckets.get(row))
            .map_or(&[], Vec::as_slice)
    }

    pub(crate) fn winding(&self, edges: &[Edge], p: [f32; 2]) -> i32 {
        self.bucket(p[1])
            .iter()
            .map(|&index| crossing(p, &edges[index]))
            .sum()
    }

    fn crossings(&self, edges: &[Edge], y: f32, out: &mut Vec<(f32, i32)>) {
        out.clear();
        for &index in self.bucket(y) {
            let Edge { a, b, .. } = edges[index];
            if (a[1] > y) != (b[1] > y) {
                let at = (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0];
                out.push((at, if b[1] > a[1] { 1 } else { -1 }));
            }
        }
        out.sort_by(|x, y| x.0.total_cmp(&y.0));
    }
}

fn boundary_edges(edges: &[Edge], rows: &Rows) -> Vec<(Edge, f32)> {
    let winding = |p: [f32; 2]| rows.winding(edges, p);
    let mut kept = Vec::with_capacity(edges.len());
    for edge in edges {
        let v = [edge.b[0] - edge.a[0], edge.b[1] - edge.a[1]];
        let length = v[0].hypot(v[1]);
        if length <= 0.0 {
            continue;
        }
        let pieces = length.ceil().max(1.0) as usize;
        let normal = [-v[1] / length * 0.05, v[0] / length * 0.05];
        let at = |t: f32| [edge.a[0] + v[0] * t, edge.a[1] + v[1] * t];
        let sides = (0..pieces)
            .map(|i| {
                let m = at((i as f32 + 0.5) / pieces as f32);
                let left = winding([m[0] + normal[0], m[1] + normal[1]]) != 0;
                let right = winding([m[0] - normal[0], m[1] - normal[1]]) != 0;
                (left != right).then_some(if left { 1.0 } else { -1.0 })
            })
            .collect::<Vec<_>>();
        if let Some(side) = sides
            .iter()
            .all(Option::is_some)
            .then(|| sides[pieces / 2])
            .flatten()
        {
            kept.push((*edge, side));
            continue;
        }
        let mut i = 0;
        while i < pieces {
            let Some(side) = sides[i] else {
                i += 1;
                continue;
            };
            let start = i;
            while i < pieces && sides[i] == Some(side) {
                i += 1;
            }
            kept.push((
                Edge {
                    a: at(start as f32 / pieces as f32),
                    b: at(i as f32 / pieces as f32),
                    mask: edge.mask,
                },
                side,
            ));
        }
    }
    kept
}

fn colored(contours: &[Vec<[f32; 2]>]) -> Vec<Edge> {
    let mut edges = Vec::new();
    for contour in contours {
        let mut points = contour.clone();
        points.dedup();
        while points.len() > 1 && points.first() == points.last() {
            points.pop();
        }
        let n = points.len();
        if n < 2 {
            continue;
        }
        let corners = (0..n)
            .filter(|&i| {
                let prev = points[(i + n - 1) % n];
                let a = points[i];
                let b = points[(i + 1) % n];
                let into = [a[0] - prev[0], a[1] - prev[1]];
                let out = [b[0] - a[0], b[1] - a[1]];
                let lens = into[0].hypot(into[1]) * out[0].hypot(out[1]);
                lens > 0.0 && (into[0] * out[0] + into[1] * out[1]) / lens < CORNER
            })
            .collect::<Vec<_>>();
        let k = corners.len();
        let first = corners.first().copied().unwrap_or(0);
        for step in 0..n {
            let i = (first + step) % n;
            let mask = match k {
                0 => 0b111,
                1 => COLORS[(step * 3 / n).min(2)],
                _ => {
                    let run = corners.partition_point(|&c| c <= i);
                    let run = if run == 0 { k - 1 } else { run - 1 };
                    if k % 3 == 1 && run == k - 1 {
                        0b110
                    } else {
                        COLORS[run % 3]
                    }
                }
            };
            edges.push(Edge {
                a: points[i],
                b: points[(i + 1) % n],
                mask,
            });
        }
    }
    edges
}

struct Prepared {
    a: [f32; 2],
    b: [f32; 2],
    unit: [f32; 2],
    length: f32,
    side: f32,
    mask: u8,
}

struct Chunk {
    lo: [f32; 2],
    hi: [f32; 2],
    mask: u8,
    start: usize,
    end: usize,
}

type Slot = (f32, f32, f32);

fn measure(edge: &Prepared, p: [f32; 2]) -> Slot {
    let w = [p[0] - edge.a[0], p[1] - edge.a[1]];
    let t = w[0] * edge.unit[0] + w[1] * edge.unit[1];
    let perp = (w[1] * edge.unit[0] - w[0] * edge.unit[1]) * edge.side;
    if t < 0.0 {
        let d = w[0].hypot(w[1]);
        (d, if d > 0.0 { -t / d } else { 0.0 }, perp)
    } else if t > edge.length {
        let e = [p[0] - edge.b[0], p[1] - edge.b[1]];
        let d = e[0].hypot(e[1]);
        (d, if d > 0.0 { (t - edge.length) / d } else { 0.0 }, perp)
    } else {
        (perp.abs(), 0.0, perp)
    }
}

fn update(slot: &mut Slot, measured: Slot) {
    let (distance, dot, _) = measured;
    let closer = distance < slot.0 - 1e-4 || ((distance - slot.0).abs() <= 1e-4 && dot < slot.1);
    if closer {
        *slot = measured;
    }
}

fn sequential(boundary: &[Prepared], p: [f32; 2]) -> ([Slot; 3], f32) {
    let mut best = [(f32::INFINITY, f32::INFINITY, 0.0f32); 3];
    let mut global = f32::INFINITY;
    for edge in boundary {
        let measured = measure(edge, p);
        global = global.min(measured.0);
        for (channel, slot) in best.iter_mut().enumerate() {
            if edge.mask & (1 << channel) != 0 {
                update(slot, measured);
            }
        }
    }
    (best, global)
}

fn far(chunk: &Chunk, low: &[f32; 3], p: [f32; 2]) -> bool {
    let mut reach = f32::NEG_INFINITY;
    for (channel, value) in low.iter().enumerate() {
        if chunk.mask & (1 << channel) != 0 {
            reach = reach.max(*value);
        }
    }
    if !reach.is_finite() {
        return false;
    }
    let dx = (chunk.lo[0] - p[0]).max(p[0] - chunk.hi[0]).max(0.0);
    let dy = (chunk.lo[1] - p[1]).max(p[1] - chunk.hi[1]).max(0.0);
    let reach = reach + SLACK + MARGIN;
    dx * dx + dy * dy > reach * reach
}

fn nearest(
    boundary: &[Prepared],
    chunks: &[Chunk],
    groups: &[Chunk],
    p: [f32; 2],
    hint: &mut usize,
    found: &mut Vec<(usize, Slot)>,
    near: &mut Vec<f32>,
) -> Option<([Slot; 3], f32)> {
    found.clear();
    let mut low = [f32::INFINITY; 3];
    let mut closest = (f32::INFINITY, *hint);
    let first = (*hint / GROUP).min(groups.len().saturating_sub(1));
    for group in std::iter::once(first).chain((0..groups.len()).filter(|&g| g != first)) {
        let members = &chunks[group * GROUP..(group * GROUP + GROUP).min(chunks.len())];
        if far(&groups[group], &low, p) {
            continue;
        }
        for (offset, chunk) in members.iter().enumerate() {
            if far(chunk, &low, p) {
                continue;
            }
            for (step, edge) in boundary[chunk.start..chunk.end].iter().enumerate() {
                let at = chunk.start + step;
                let measured = measure(edge, p);
                let mut keep = false;
                for (channel, value) in low.iter_mut().enumerate() {
                    if edge.mask & (1 << channel) != 0 {
                        keep |= measured.0 <= *value + SLACK;
                        *value = value.min(measured.0);
                    }
                }
                if keep {
                    found.push((at, measured));
                }
                if measured.0 < closest.0 {
                    closest = (measured.0, group * GROUP + offset);
                }
            }
        }
    }
    *hint = closest.1;
    found.sort_unstable_by_key(|entry| entry.0);
    let mut best = [(f32::INFINITY, f32::INFINITY, 0.0f32); 3];
    for (channel, slot) in best.iter_mut().enumerate() {
        if !low[channel].is_finite() {
            continue;
        }
        let limit = low[channel] + SLACK;
        near.clear();
        near.extend(
            found
                .iter()
                .filter(|(at, measured)| {
                    boundary[*at].mask & (1 << channel) != 0 && measured.0 <= limit
                })
                .map(|(_, measured)| measured.0),
        );
        near.sort_unstable_by(f32::total_cmp);
        let mut top = low[channel];
        for &distance in near.iter() {
            if distance > top + GAP {
                break;
            }
            top = top.max(distance);
        }
        if top + GAP >= limit - MARGIN {
            return None;
        }
        for (at, measured) in found.iter() {
            if boundary[*at].mask & (1 << channel) != 0 && measured.0 <= limit {
                update(slot, *measured);
            }
        }
    }
    let global = low.into_iter().fold(f32::INFINITY, f32::min);
    Some((best, global))
}

fn median(rgb: [f32; 3]) -> f32 {
    rgb[0].min(rgb[1]).max(rgb[0].max(rgb[1]).min(rgb[2]))
}

fn clash(a: [f32; 3], b: [f32; 3]) -> bool {
    let mut pairs = [(a[0], b[0]), (a[1], b[1]), (a[2], b[2])];
    pairs.sort_by(|x, y| (y.1 - y.0).abs().total_cmp(&(x.1 - x.0).abs()));
    let [(_, b0), (a1, b1), (a2, b2)] = pairs;
    (b1 - a1).abs() >= CLASH && !(b0 == b1 && b0 == b2) && (a2 - 0.5).abs() >= (b2 - 0.5).abs()
}

pub(crate) fn soft(
    contours: &[Vec<[f32; 2]>],
    corner: [f32; 2],
    size: [u32; 2],
) -> Option<GlyphImage> {
    let ([left, top], [width, height]) = (corner, size);
    let mut edges = Vec::new();
    for contour in contours {
        let n = contour.len();
        let mut color = 0usize;
        for i in 0..n {
            let a = contour[i];
            let b = contour[(i + 1) % n];
            let prev = contour[(i + n - 1) % n];
            let in_dir = [a[0] - prev[0], a[1] - prev[1]];
            let out_dir = [b[0] - a[0], b[1] - a[1]];
            let dot = in_dir[0] * out_dir[0] + in_dir[1] * out_dir[1];
            let lens = in_dir[0].hypot(in_dir[1]) * out_dir[0].hypot(out_dir[1]);
            if lens > 0.0 && dot / lens < 0.75 {
                color += 1;
            }
            if a != b {
                edges.push(Edge {
                    a,
                    b,
                    mask: [0b011, 0b110, 0b101][color % 3],
                });
            }
        }
    }
    let rows = Rows::new(&edges);
    let boundary = boundary_edges(&edges, &rows)
        .into_iter()
        .map(|(edge, _)| edge)
        .collect::<Vec<_>>();
    if boundary.is_empty() {
        return None;
    }
    let mut bytes = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            let p = [left + x as f32 + 0.5, top + y as f32 + 0.5];
            let winding = rows.winding(&edges, p);
            let mut distances = [f32::INFINITY; 3];
            let mut global = f32::INFINITY;
            for edge in &boundary {
                let d = segment_distance(p, edge.a, edge.b);
                global = global.min(d);
                for (channel, value) in distances.iter_mut().enumerate() {
                    if edge.mask & (1 << channel) != 0 {
                        *value = value.min(d);
                    }
                }
            }
            let inside = winding != 0;
            for d in distances {
                let signed = if inside {
                    d.min(global + 1.0)
                } else {
                    -d.min(global + 1.0)
                };
                bytes.push(((0.5 + signed / 8.0).clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
    }
    Some(GlyphImage {
        left,
        top,
        width,
        height,
        bytes,
    })
}

fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let v = [b[0] - a[0], b[1] - a[1]];
    let len2 = v[0] * v[0] + v[1] * v[1];
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * v[0] + (p[1] - a[1]) * v[1]) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - t * v[0]).hypot(p[1] - a[1] - t * v[1])
}

pub(crate) fn sharp(
    contours: &[Vec<[f32; 2]>],
    corner: [f32; 2],
    size: [u32; 2],
) -> Option<GlyphImage> {
    let edges = colored(contours);
    let rows = Rows::new(&edges);
    let boundary = boundary_edges(&edges, &rows)
        .into_iter()
        .filter_map(|(edge, side)| {
            let v = [edge.b[0] - edge.a[0], edge.b[1] - edge.a[1]];
            let length = v[0].hypot(v[1]);
            (length > 0.0).then_some(Prepared {
                a: edge.a,
                b: edge.b,
                unit: [v[0] / length, v[1] / length],
                length,
                side,
                mask: edge.mask,
            })
        })
        .collect::<Vec<_>>();
    if boundary.is_empty() {
        return None;
    }
    let chunks = boundary
        .chunks(CHUNK)
        .enumerate()
        .map(|(index, chunk)| {
            let mut lo = [f32::INFINITY; 2];
            let mut hi = [f32::NEG_INFINITY; 2];
            let mut mask = 0;
            for edge in chunk {
                for k in 0..2 {
                    lo[k] = lo[k].min(edge.a[k]).min(edge.b[k]);
                    hi[k] = hi[k].max(edge.a[k]).max(edge.b[k]);
                }
                mask |= edge.mask;
            }
            Chunk {
                lo,
                hi,
                mask,
                start: index * CHUNK,
                end: index * CHUNK + chunk.len(),
            }
        })
        .collect::<Vec<_>>();
    let groups = chunks
        .chunks(GROUP)
        .map(|members| {
            let mut bounds = Chunk {
                lo: [f32::INFINITY; 2],
                hi: [f32::NEG_INFINITY; 2],
                mask: 0,
                start: members[0].start,
                end: members[members.len() - 1].end,
            };
            for chunk in members {
                for k in 0..2 {
                    bounds.lo[k] = bounds.lo[k].min(chunk.lo[k]);
                    bounds.hi[k] = bounds.hi[k].max(chunk.hi[k]);
                }
                bounds.mask |= chunk.mask;
            }
            bounds
        })
        .collect::<Vec<_>>();
    let [width, height] = size;
    let mut values = Vec::with_capacity((width * height) as usize);
    let mut crossings = Vec::new();
    let mut hint = 0;
    let mut found = Vec::new();
    let mut near = Vec::new();
    for y in 0..height {
        let row_y = corner[1] + y as f32 + 0.5;
        rows.crossings(&edges, row_y, &mut crossings);
        let mut next = 0;
        let mut winding = crossings.iter().map(|&(_, dir)| dir).sum::<i32>();
        for x in 0..width {
            let p = [corner[0] + x as f32 + 0.5, row_y];
            while let Some(&(at, dir)) = crossings.get(next)
                && at <= p[0]
            {
                winding -= dir;
                next += 1;
            }
            let inside = winding != 0;
            let (best, global) = nearest(
                &boundary, &chunks, &groups, p, &mut hint, &mut found, &mut near,
            )
            .unwrap_or_else(|| sequential(&boundary, p));
            let truth = if inside { global } else { -global };
            let mut rgb = best.map(|slot| {
                if slot.0.is_finite() {
                    0.5 + slot.2 / SPREAD
                } else {
                    0.5 + truth / SPREAD
                }
            });
            if (median(rgb) - 0.5) * truth < 0.0 || (median(rgb) - 0.5 == 0.0 && truth != 0.0) {
                rgb = [0.5 + truth / SPREAD; 3];
            }
            values.push(rgb);
        }
    }
    let at = |x: u32, y: u32| (y * width + x) as usize;
    let mut flagged = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let here = values[at(x, y)];
            let neighbours = [
                (x > 0).then(|| at(x - 1, y)),
                (x + 1 < width).then(|| at(x + 1, y)),
                (y > 0).then(|| at(x, y - 1)),
                (y + 1 < height).then(|| at(x, y + 1)),
            ];
            if neighbours
                .into_iter()
                .flatten()
                .any(|other| clash(here, values[other]))
            {
                flagged.push(at(x, y));
            }
        }
    }
    for index in flagged {
        let m = median(values[index]);
        values[index] = [m; 3];
    }
    let bytes = values
        .into_iter()
        .flat_map(|rgb| rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
        .collect();
    Some(GlyphImage {
        left: corner[0],
        top: corner[1],
        width,
        height,
        bytes,
    })
}
