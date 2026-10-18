use serde::{Deserialize, Serialize};

pub const FLAKE_RANDOMNESS: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Flakes {
    pub amount: f32,
    pub size: f32,
    pub spread: f32,
    pub color: [f32; 3],
    pub roughness: f32,
}

impl Default for Flakes {
    fn default() -> Self {
        Self {
            amount: 0.0,
            size: 0.0005,
            spread: 0.3,
            color: [0.95, 0.95, 0.95],
            roughness: 0.25,
        }
    }
}

impl Flakes {
    pub fn off(&self) -> bool {
        self.amount <= 0.0 || self.size <= 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub distance: f32,
    pub color: [f32; 3],
    pub cell: [i32; 3],
}

pub fn hash_pcg3d_i(k: [i32; 3]) -> [i32; 3] {
    let mut v = k.map(|x| x.wrapping_mul(1664525).wrapping_add(1013904223));
    v[0] = v[0].wrapping_add(v[1].wrapping_mul(v[2]));
    v[1] = v[1].wrapping_add(v[2].wrapping_mul(v[0]));
    v[2] = v[2].wrapping_add(v[0].wrapping_mul(v[1]));
    v = v.map(|x| x ^ (x >> 16));
    v[0] = v[0].wrapping_add(v[1].wrapping_mul(v[2]));
    v[1] = v[1].wrapping_add(v[2].wrapping_mul(v[0]));
    v[2] = v[2].wrapping_add(v[0].wrapping_mul(v[1]));
    v.map(|x| x & 0x7FFF_FFFF)
}

pub fn hash_int3_to_float3(k: [i32; 3]) -> [f32; 3] {
    hash_pcg3d_i(k).map(|h| h as f32 * (1.0 / 0x7FFF_FFFF as f32))
}

pub fn voronoi_f1(coord: [f32; 3], randomness: f32) -> Cell {
    let cell_f = coord.map(f32::floor);
    let local: [f32; 3] = std::array::from_fn(|i| coord[i] - cell_f[i]);
    let cell = cell_f.map(|c| c as i32);
    let mut min_distance = f32::MAX;
    let mut target = [0; 3];
    let mut target_position = [0.0; 3];
    for k in -1..=1 {
        for j in -1..=1 {
            for i in -1..=1 {
                let offset = [i, j, k];
                let hash = hash_int3_to_float3(std::array::from_fn(|a| cell[a] + offset[a]));
                let point: [f32; 3] =
                    std::array::from_fn(|a| offset[a] as f32 + hash[a] * randomness);
                let distance = (0..3).map(|a| (point[a] - local[a]).powi(2)).sum::<f32>();
                if distance < min_distance {
                    target = offset;
                    min_distance = distance;
                    target_position = point;
                }
            }
        }
    }
    let found = std::array::from_fn(|a| cell[a] + target[a]);
    Cell {
        distance: (0..3)
            .map(|a| (target_position[a] - local[a]).powi(2))
            .sum::<f32>()
            .sqrt(),
        color: hash_int3_to_float3(found),
        cell: found,
    }
}

pub fn flake_frame(normal: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let axis = if normal[1].abs() > 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let t = unit(cross(axis, normal));
    (t, cross(normal, t))
}

pub fn flake_normal(flakes: &Flakes, position: [f32; 3], normal: [f32; 3]) -> Option<[f32; 3]> {
    if flakes.off() {
        return None;
    }
    let cell = voronoi_f1(position.map(|p| p / flakes.size), FLAKE_RANDOMNESS);
    if cell.color[0] >= flakes.amount {
        return None;
    }
    let (t, b) = flake_frame(normal);
    let u = flakes.spread * (2.0 * cell.color[1] - 1.0);
    let v = flakes.spread * (2.0 * cell.color[2] - 1.0);
    Some(unit(std::array::from_fn(|a| {
        normal[a] + u * t[a] + v * b[a]
    })))
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|value| value / length)
}
