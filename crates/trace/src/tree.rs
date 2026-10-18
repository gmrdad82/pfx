use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::lights::{GpuLight, LightShape, LocalLight};
use crate::math::{add, dot, length, normalize, scale, sub};

pub const LOOPED: usize = 4;
const LEAF: usize = 8;
const BUCKETS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Cone {
    axis: [f32; 3],
    theta_o: f32,
    theta_e: f32,
}

impl Cone {
    const EMPTY: Cone = Cone {
        axis: [0.0; 3],
        theta_o: f32::MIN_POSITIVE,
        theta_e: f32::MIN_POSITIVE,
    };

    fn is_empty(&self) -> bool {
        self.axis == [0.0; 3]
    }

    fn measure(&self) -> f32 {
        if self.is_empty() {
            return 0.0;
        }
        let theta_w = (self.theta_o + self.theta_e).min(PI);
        let cos_o = self.theta_o.cos();
        let sin_o = self.theta_o.sin();
        TAU * (1.0 - cos_o)
            + FRAC_PI_2
                * (2.0 * theta_w * sin_o
                    - (self.theta_o - 2.0 * theta_w).cos()
                    - 2.0 * self.theta_o * sin_o
                    + cos_o)
    }

    fn merge(self, other: Cone) -> Cone {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let (a, b) = if other.theta_o > self.theta_o {
            (other, self)
        } else {
            (self, other)
        };
        let cos_a_b = dot(a.axis, b.axis);
        let theta_d = cos_a_b.clamp(-1.0, 1.0).acos();
        let theta_e = a.theta_e.max(b.theta_e);
        if a.theta_o + 5e-4 >= PI.min(theta_d + b.theta_o) {
            return Cone { theta_e, ..a };
        }
        let theta_o = (theta_d + a.theta_o + b.theta_o) * 0.5;
        if theta_o >= PI {
            return Cone {
                axis: a.axis,
                theta_o: PI,
                theta_e,
            };
        }
        let axis = if cos_a_b < -0.9995 {
            orthogonal(a.axis)
        } else {
            let theta_r = theta_o - a.theta_o;
            let ortho = normalize(sub(b.axis, scale(a.axis, cos_a_b)));
            add(scale(a.axis, theta_r.cos()), scale(ortho, theta_r.sin()))
        };
        Cone {
            axis,
            theta_o,
            theta_e,
        }
    }
}

fn orthogonal(n: [f32; 3]) -> [f32; 3] {
    let helper = if n[0].abs() > 0.1 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    normalize(crate::math::cross(n, helper))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Measure {
    min: [f32; 3],
    max: [f32; 3],
    cone: Cone,
    energy: f32,
    range: f32,
}

impl Measure {
    const EMPTY: Measure = Measure {
        min: [f32::MAX; 3],
        max: [-f32::MAX; 3],
        cone: Cone::EMPTY,
        energy: 0.0,
        range: 0.0,
    };

    fn add(&mut self, other: &Measure) {
        if other.energy == 0.0 {
            return;
        }
        for k in 0..3 {
            self.min[k] = self.min[k].min(other.min[k]);
            self.max[k] = self.max[k].max(other.max[k]);
        }
        self.cone = self.cone.merge(other.cone);
        self.energy += other.energy;
        self.range = self.range.max(other.range);
    }

    fn calculate(&self) -> f32 {
        if self.energy == 0.0 {
            return 0.0;
        }
        let size = sub(self.max, self.min);
        let area = 2.0 * (size[0] * size[1] + size[1] * size[2] + size[2] * size[0]);
        let area_measure = if area == 0.0 { length(size) } else { area };
        self.energy * area_measure * self.cone.measure()
    }
}

#[derive(Clone, Copy, Debug)]
struct Emitter {
    index: usize,
    centroid: [f32; 3],
    measure: Measure,
}

impl Emitter {
    fn of(index: usize, light: &LocalLight) -> Emitter {
        let cone = match light.shape {
            LightShape::Spot {
                direction,
                outer_deg,
                ..
            } => Cone {
                axis: normalize(direction),
                theta_o: 0.0,
                theta_e: outer_deg.to_radians().min(FRAC_PI_2),
            },
            _ => Cone {
                axis: [0.0, 0.0, -1.0],
                theta_o: PI,
                theta_e: FRAC_PI_2,
            },
        };
        let strength = light
            .colour
            .map(|channel| (channel * light.intensity).abs());
        Emitter {
            index,
            centroid: light.position,
            measure: Measure {
                min: light.position.map(|v| v - light.radius),
                max: light.position.map(|v| v + light.radius),
                cone,
                energy: (strength[0] + strength[1] + strength[2]) / 3.0,
                range: light.range,
            },
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Bucket {
    measure: Option<Measure>,
    count: usize,
}

impl Bucket {
    fn add(&mut self, measure: &Measure) {
        self.measure.get_or_insert(Measure::EMPTY).add(measure);
        self.count += 1;
    }

    fn plus(&self, other: &Bucket) -> Bucket {
        let mut measure = self.measure.unwrap_or(Measure::EMPTY);
        if let Some(other) = &other.measure {
            measure.add(other);
        }
        Bucket {
            measure: Some(measure),
            count: self.count + other.count,
        }
    }

    fn cost(&self) -> f32 {
        self.measure.map_or(0.0, |measure| measure.calculate())
    }
}

pub(crate) fn treed(light: &LocalLight) -> bool {
    light.lit() && matches!(light.shape, LightShape::Point | LightShape::Spot { .. })
}

pub(crate) struct Tree {
    pub order: Vec<usize>,
    pub nodes: Vec<GpuLight>,
}

pub(crate) struct Laid {
    pub rows: Vec<GpuLight>,
    pub looped: usize,
    pub tree: usize,
}

pub(crate) fn lay(lights: &[LocalLight]) -> Laid {
    let treed_count = lights.iter().filter(|light| treed(light)).count();
    if treed_count <= LOOPED {
        return Laid {
            rows: lights.iter().map(LocalLight::pack).collect(),
            looped: lights.len(),
            tree: 0,
        };
    }
    let mut rows: Vec<GpuLight> = lights
        .iter()
        .filter(|light| !treed(light))
        .map(LocalLight::pack)
        .collect();
    let looped = rows.len();
    let tree = build(lights, looped);
    rows.extend(tree.order.iter().map(|&index| lights[index].pack()));
    rows.extend(tree.nodes);
    Laid {
        rows,
        looped,
        tree: treed_count,
    }
}

pub(crate) fn build(lights: &[LocalLight], base: usize) -> Tree {
    let mut emitters: Vec<Emitter> = lights
        .iter()
        .enumerate()
        .filter(|(_, light)| treed(light))
        .map(|(index, light)| Emitter::of(index, light))
        .collect();
    let count = emitters.len();
    let mut nodes = Vec::new();
    if count > 0 {
        grow(&mut emitters, 0, base, base + count, &mut nodes);
    }
    Tree {
        order: emitters.iter().map(|emitter| emitter.index).collect(),
        nodes,
    }
}

fn grow(
    emitters: &mut [Emitter],
    start: usize,
    base: usize,
    root: usize,
    nodes: &mut Vec<GpuLight>,
) {
    let at = nodes.len();
    nodes.push(GpuLight {
        a: [0.0; 4],
        b: [0.0; 4],
        c: [0.0; 4],
        d: [0.0; 4],
    });
    let (measure, split) = should_split(emitters);
    match split {
        Some((middle, dim)) => {
            if let Some(dim) = dim {
                emitters.select_nth_unstable_by(middle, |l, r| {
                    l.centroid[dim].total_cmp(&r.centroid[dim])
                });
            }
            let (left, right) = emitters.split_at_mut(middle);
            grow(left, start, base, root, nodes);
            let right_at = root + nodes.len();
            grow(right, start + middle, base, root, nodes);
            nodes[at] = row(&measure, 0, right_at);
        }
        None => nodes[at] = row(&measure, emitters.len(), base + start),
    }
}

fn row(measure: &Measure, count: usize, link: usize) -> GpuLight {
    GpuLight {
        a: [
            measure.min[0],
            measure.min[1],
            measure.min[2],
            measure.energy,
        ],
        b: [
            measure.max[0],
            measure.max[1],
            measure.max[2],
            measure.range,
        ],
        c: [
            measure.cone.axis[0],
            measure.cone.axis[1],
            measure.cone.axis[2],
            measure.cone.theta_o,
        ],
        d: [measure.cone.theta_e, count as f32, link as f32, 0.0],
    }
}

fn should_split(emitters: &[Emitter]) -> (Measure, Option<(usize, Option<usize>)>) {
    let count = emitters.len();
    if count < 2 {
        return (emitters[0].measure, None);
    }
    let mut middle = count / 2;
    let mut low = [f32::MAX; 3];
    let mut high = [-f32::MAX; 3];
    for emitter in emitters {
        for k in 0..3 {
            low[k] = low[k].min(emitter.centroid[k]);
            high[k] = high[k].max(emitter.centroid[k]);
        }
    }
    let extent = sub(high, low);
    let max_extent = extent[0].max(extent[1]).max(extent[2]).max(0.0);
    let mut measure = Measure::EMPTY;
    let mut total_cost = 0.0_f32;
    let mut min_cost = f32::MAX;
    let mut split_dim = None;
    for dim in 0..3 {
        let mut buckets = [Bucket::default(); BUCKETS];
        let inv_extent;
        if extent[dim] == 0.0 {
            if dim != 0 {
                continue;
            }
            inv_extent = f32::MAX;
            for emitter in emitters {
                buckets[0].add(&emitter.measure);
            }
        } else {
            inv_extent = 1.0 / extent[dim];
            for emitter in emitters {
                let at = (BUCKETS as f32 * (emitter.centroid[dim] - low[dim]) * inv_extent) as i64;
                buckets[at.clamp(0, BUCKETS as i64 - 1) as usize].add(&emitter.measure);
            }
        }
        let mut left = [Bucket::default(); BUCKETS - 1];
        left[0] = buckets[0];
        for i in 1..BUCKETS - 1 {
            left[i] = left[i - 1].plus(&buckets[i]);
        }
        if dim == 0 {
            measure = left[BUCKETS - 2]
                .plus(&buckets[BUCKETS - 1])
                .measure
                .unwrap_or(Measure::EMPTY);
            if extent == [0.0; 3] {
                break;
            }
            total_cost = measure.calculate();
            if total_cost == 0.0 {
                break;
            }
            if extent[dim] == 0.0 {
                continue;
            }
        }
        let mut right = [Bucket::default(); BUCKETS - 1];
        right[BUCKETS - 2] = buckets[BUCKETS - 1];
        for i in (0..BUCKETS - 2).rev() {
            right[i] = right[i + 1].plus(&buckets[i + 1]);
        }
        let regularization = max_extent * inv_extent;
        for split in 0..BUCKETS - 1 {
            let cost = regularization * (left[split].cost() + right[split].cost());
            if cost < total_cost && cost < min_cost && (1..count).contains(&left[split].count) {
                min_cost = cost;
                split_dim = Some(dim);
                middle = left[split].count;
            }
        }
    }
    let split = min_cost < total_cost || count > LEAF;
    (measure, split.then_some((middle, split_dim)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn street(count: usize) -> Vec<LocalLight> {
        (0..count)
            .map(|k| {
                let side = if k % 2 == 0 { -6.0 } else { 6.0 };
                let mut light =
                    LocalLight::point([side, 5.0, k as f32 * 4.0], [1.0, 0.8, 0.6], 40.0, 18.0);
                if k % 3 == 0 {
                    light.shape = LightShape::Spot {
                        direction: [0.0, -1.0, 0.0],
                        inner_deg: 30.0,
                        outer_deg: 50.0,
                    };
                }
                light
            })
            .collect()
    }

    fn leaves(tree: &Tree, base: usize, at: usize, seen: &mut Vec<usize>) {
        let root = base + tree.order.len();
        let node = tree.nodes[at - root];
        let count = node.d[1] as usize;
        let link = node.d[2] as usize;
        if count == 0 {
            leaves(tree, base, at + 1, seen);
            leaves(tree, base, link, seen);
        } else {
            assert!(count <= LEAF);
            seen.extend((link - base..link - base + count).map(|k| tree.order[k]));
        }
    }

    #[test]
    fn every_light_sits_in_exactly_one_leaf() {
        for count in [5, 32, 128, 1024] {
            let lights = street(count);
            let tree = build(&lights, 3);
            let mut seen = Vec::new();
            leaves(&tree, 3, 3 + count, &mut seen);
            seen.sort_unstable();
            assert_eq!(seen, (0..count).collect::<Vec<_>>(), "{count} lights");
            assert!(tree.nodes.len() < 2 * count);
        }
    }

    #[test]
    fn a_node_bounds_its_lights_energy_and_reach() {
        let lights = street(128);
        let tree = build(&lights, 0);
        let root = tree.nodes[0];
        let energy: f32 = lights
            .iter()
            .map(|light| light.colour.iter().sum::<f32>() * light.intensity / 3.0)
            .sum();
        assert!((root.a[3] - energy).abs() <= energy * 1e-5);
        assert_eq!(root.b[3], 18.0);
        for light in &lights {
            for k in 0..3 {
                assert!(root.a[k] <= light.position[k] && light.position[k] <= root.b[k]);
            }
        }
    }

    #[test]
    fn few_lights_keep_their_order_and_their_own_slots() {
        let lights = street(LOOPED);
        let laid = lay(&lights);
        assert_eq!((laid.looped, laid.tree), (LOOPED, 0));
        assert_eq!(laid.rows.len(), LOOPED);
        let many = lay(&street(LOOPED + 1));
        assert_eq!((many.looped, many.tree), (0, LOOPED + 1));
    }

    #[test]
    fn merged_cones_hold_both() {
        let a = Cone {
            axis: [0.0, -1.0, 0.0],
            theta_o: 0.0,
            theta_e: 0.8,
        };
        let b = Cone {
            axis: [1.0, 0.0, 0.0],
            theta_o: 0.0,
            theta_e: 0.4,
        };
        let merged = a.merge(b);
        for axis in [a.axis, b.axis] {
            let angle = dot(merged.axis, axis).clamp(-1.0, 1.0).acos();
            assert!(angle <= merged.theta_o + 1e-4);
        }
        assert_eq!(merged.theta_e, 0.8);
    }
}
