use crate::math::{cross, dot, sub};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Node {
    pub lo: [f32; 4],
    pub hi: [f32; 4],
    pub first: u32,
    pub count: u32,
    pub _pad: [u32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    pub vertices: [[f32; 3]; 3],
    pub material: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: [f32; 3],
    pub direction: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub distance: f32,
    pub normal: [f32; 3],
    pub material: u32,
    pub index: u32,
}

#[derive(Clone, Debug)]
pub struct Bvh {
    pub nodes: Vec<Node>,
    pub order: Vec<u32>,
}

#[derive(Clone, Copy)]
struct Bounds {
    lo: [f32; 3],
    hi: [f32; 3],
}

impl Bounds {
    fn empty() -> Self {
        Self {
            lo: [f32::INFINITY; 3],
            hi: [f32::NEG_INFINITY; 3],
        }
    }
    fn point(&mut self, p: [f32; 3]) {
        for (axis, value) in p.into_iter().enumerate() {
            self.lo[axis] = self.lo[axis].min(value);
            self.hi[axis] = self.hi[axis].max(value);
        }
    }
    fn grow(&mut self, other: Self) {
        self.point(other.lo);
        self.point(other.hi);
    }
    fn area(self) -> f32 {
        let d = sub(self.hi, self.lo);
        if d[0] < 0.0 {
            0.0
        } else {
            2.0 * (d[0] * d[1] + d[1] * d[2] + d[2] * d[0])
        }
    }
}

const BINS: usize = 16;
const LEAF: usize = 4;

impl Bvh {
    pub fn build(triangles: &[Triangle]) -> Self {
        let boxes: Vec<_> = triangles
            .iter()
            .map(|triangle| {
                let mut bounds = Bounds::empty();
                for &vertex in &triangle.vertices {
                    bounds.point(vertex);
                }
                bounds
            })
            .collect();
        let centers: Vec<_> = boxes
            .iter()
            .map(|b| [0, 1, 2].map(|axis| (b.lo[axis] + b.hi[axis]) * 0.5))
            .collect();
        let mut order: Vec<_> = (0..triangles.len() as u32).collect();
        let mut nodes = Vec::new();
        if !triangles.is_empty() {
            split(&boxes, &centers, &mut order, 0, triangles.len(), &mut nodes);
        }
        Self { nodes, order }
    }
    pub fn intersect(&self, triangles: &[Triangle], ray: Ray, limit: f32) -> Option<Hit> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut best = None;
        let mut stack = vec![0usize];
        while let Some(index) = stack.pop() {
            let node = self.nodes[index];
            let max = best.as_ref().map_or(limit, |hit: &Hit| hit.distance);
            if !box_hit(node, ray, max) {
                continue;
            }
            if node.count == 0 {
                stack.push(node.first as usize);
                stack.push(index + 1);
            } else {
                for &i in &self.order[node.first as usize..(node.first + node.count) as usize] {
                    if let Some(mut hit) = triangle_hit(
                        &triangles[i as usize],
                        ray,
                        best.as_ref().map_or(limit, |hit: &Hit| hit.distance),
                    ) {
                        hit.index = i;
                        best = Some(hit);
                    }
                }
            }
        }
        best
    }
}

fn split(
    boxes: &[Bounds],
    centers: &[[f32; 3]],
    order: &mut [u32],
    start: usize,
    end: usize,
    nodes: &mut Vec<Node>,
) {
    let mut bounds = Bounds::empty();
    let mut centroids = Bounds::empty();
    for &index in &order[start..end] {
        bounds.grow(boxes[index as usize]);
        centroids.point(centers[index as usize]);
    }
    let here = nodes.len();
    nodes.push(Node {
        lo: [bounds.lo[0], bounds.lo[1], bounds.lo[2], 0.0],
        hi: [bounds.hi[0], bounds.hi[1], bounds.hi[2], 0.0],
        first: start as u32,
        count: (end - start) as u32,
        _pad: [0; 2],
    });
    let len = end - start;
    if len <= LEAF {
        return;
    }
    let mut best = (f32::INFINITY, 0usize, 0usize);
    for (axis, (&lo, &hi)) in centroids.lo.iter().zip(&centroids.hi).enumerate() {
        let extent = hi - lo;
        if extent <= 1e-9 {
            continue;
        }
        let mut bins = [(Bounds::empty(), 0usize); BINS];
        for &index in &order[start..end] {
            let bin = bin(centers[index as usize][axis], centroids.lo[axis], extent);
            bins[bin].0.grow(boxes[index as usize]);
            bins[bin].1 += 1;
        }
        let mut areas = [0.0; BINS];
        let mut counts = [0; BINS];
        let mut bound = Bounds::empty();
        let mut count = 0;
        for index in 0..BINS {
            if bins[index].1 > 0 {
                bound.grow(bins[index].0);
            }
            count += bins[index].1;
            areas[index] = bound.area();
            counts[index] = count;
        }
        bound = Bounds::empty();
        count = 0;
        for index in (1..BINS).rev() {
            if bins[index].1 > 0 {
                bound.grow(bins[index].0);
            }
            count += bins[index].1;
            let cost = areas[index - 1] * counts[index - 1] as f32 + bound.area() * count as f32;
            if counts[index - 1] > 0 && count > 0 && cost < best.0 {
                best = (cost, axis, index);
            }
        }
    }
    if best.0 >= bounds.area() * len as f32 && len <= 16 {
        return;
    }
    let mid = if best.0.is_finite() {
        let axis = best.1;
        let extent = centroids.hi[axis] - centroids.lo[axis];
        let slice = &mut order[start..end];
        let mut left = 0;
        let mut right = len;
        while left < right {
            if bin(
                centers[slice[left] as usize][axis],
                centroids.lo[axis],
                extent,
            ) < best.2
            {
                left += 1;
            } else {
                right -= 1;
                slice.swap(left, right);
            }
        }
        if left == 0 || left == len {
            start + len / 2
        } else {
            start + left
        }
    } else {
        start + len / 2
    };
    split(boxes, centers, order, start, mid, nodes);
    let right = nodes.len();
    split(boxes, centers, order, mid, end, nodes);
    nodes[here].first = right as u32;
    nodes[here].count = 0;
}

fn bin(value: f32, lo: f32, extent: f32) -> usize {
    (((value - lo) / extent) * BINS as f32).clamp(0.0, (BINS - 1) as f32) as usize
}

fn box_hit(node: Node, ray: Ray, limit: f32) -> bool {
    let mut near = 0.0f32;
    let mut far = limit;
    for axis in 0..3 {
        let direction = ray.direction[axis];
        if direction.abs() < 1e-12 {
            if ray.origin[axis] < node.lo[axis] || ray.origin[axis] > node.hi[axis] {
                return false;
            }
        } else {
            let a = (node.lo[axis] - ray.origin[axis]) / direction;
            let b = (node.hi[axis] - ray.origin[axis]) / direction;
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return false;
            }
        }
    }
    true
}

pub fn triangle_hit(triangle: &Triangle, ray: Ray, limit: f32) -> Option<Hit> {
    let [a, b, c] = triangle.vertices;
    let e1 = sub(b, a);
    let e2 = sub(c, a);
    let p = cross(ray.direction, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let s = sub(ray.origin, a);
    let u = dot(s, p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(ray.direction, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = dot(e2, q) * inv;
    if distance <= 1e-5 || distance >= limit {
        return None;
    }
    Some(Hit {
        distance,
        normal: crate::math::normalize(cross(e1, e2)),
        material: triangle.material,
        index: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn random_rays_match_brute_force_and_build_is_stable() {
        let mut state = 17u32;
        let mut random = || {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 8) as f32 / (1u32 << 24) as f32
        };
        let triangles: Vec<_> = (0..120)
            .map(|_| {
                let x = random() * 8.0 - 4.0;
                let y = random() * 8.0 - 4.0;
                let z = random() * 8.0 - 4.0;
                Triangle {
                    vertices: [
                        [x, y, z],
                        [x + random() + 0.1, y, z],
                        [x, y + random() + 0.1, z],
                    ],
                    material: 0,
                }
            })
            .collect();
        let bvh = Bvh::build(&triangles);
        let other = Bvh::build(&triangles);
        assert_eq!(bvh.nodes, other.nodes);
        assert_eq!(bvh.order, other.order);
        for _ in 0..2000 {
            let ray = Ray {
                origin: [
                    random() * 12.0 - 6.0,
                    random() * 12.0 - 6.0,
                    random() * 12.0 - 6.0,
                ],
                direction: crate::math::normalize([
                    random() * 2.0 - 1.0,
                    random() * 2.0 - 1.0,
                    random() * 2.0 - 1.0,
                ]),
            };
            let brute = triangles
                .iter()
                .enumerate()
                .filter_map(|(i, t)| {
                    triangle_hit(t, ray, f32::INFINITY).map(|mut h| {
                        h.index = i as u32;
                        h
                    })
                })
                .min_by(|a, b| a.distance.total_cmp(&b.distance));
            let fast = bvh.intersect(&triangles, ray, f32::INFINITY);
            assert_eq!(fast.map(|h| h.index), brute.map(|h| h.index));
        }
    }
}
