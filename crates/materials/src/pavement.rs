use crate::kinds::{Crack, Track};
use crate::math::{clamp01, fract, smoothstep};
use crate::noise::{Hash, hash2, octave_weight, value_noise3};

pub const POINT_SEED: u32 = 0x9e37_79b9;
pub const CRACK_SPREAD: f32 = 0.12;
pub const WARP_SHIFT: [f32; 3] = [17.3, 5.1, 11.7];
pub const MASK_SHIFT: [f32; 3] = [3.7, 1.3, 8.9];
pub const TRACK_SLICE: f32 = 0.37;
pub const TRACK_SLICES: u32 = 4096;

fn noise(p: [f32; 3]) -> f32 {
    value_noise3(p, Hash::Xor)
}

fn point(cell: [i32; 2], seed: u32, randomness: f32) -> [f32; 2] {
    [
        hash2(cell, seed) * randomness,
        hash2(cell, seed ^ POINT_SEED) * randomness,
    ]
}

pub fn voronoi_edge2(p: [f32; 2], randomness: f32, seed: u32) -> f32 {
    let cell = [p[0].floor(), p[1].floor()];
    let local = [p[0] - cell[0], p[1] - cell[1]];
    let at = [cell[0] as i32, cell[1] as i32];
    let to = |i: i32, j: i32| {
        let q = point([at[0] + i, at[1] + j], seed, randomness);
        [i as f32 + q[0] - local[0], j as f32 + q[1] - local[1]]
    };
    let mut closest = [0.0; 2];
    let mut nearest = f32::MAX;
    for j in -1..=1 {
        for i in -1..=1 {
            let v = to(i, j);
            let d = v[0] * v[0] + v[1] * v[1];
            if d < nearest {
                nearest = d;
                closest = v;
            }
        }
    }
    let mut edge = f32::MAX;
    for j in -1..=1 {
        for i in -1..=1 {
            let v = to(i, j);
            let across = [v[0] - closest[0], v[1] - closest[1]];
            let length2 = across[0] * across[0] + across[1] * across[1];
            if length2 > 0.0001 {
                let length = length2.sqrt();
                let mid = [(closest[0] + v[0]) * 0.5, (closest[1] + v[1]) * 0.5];
                edge = edge.min((mid[0] * across[0] + mid[1] * across[1]) / length);
            }
        }
    }
    edge
}

pub fn plane(p: [f32; 3], n: [f32; 3]) -> [f32; 2] {
    let a = n.map(f32::abs);
    if a[1] >= a[0] && a[1] >= a[2] {
        [p[0], p[2]]
    } else if a[0] >= a[2] {
        [p[2], p[1]]
    } else {
        [p[0], p[1]]
    }
}

pub fn crack(p: [f32; 3], n: [f32; 3], c: Crack, seed: u32, pixel: f32) -> f32 {
    if c.cell <= 0.0 || c.width <= 0.0 {
        return 0.0;
    }
    let w = p.map(|v| v * c.warp_frequency);
    let warp = [
        noise(w) - 0.5,
        noise([
            w[0] + WARP_SHIFT[0],
            w[1] + WARP_SHIFT[1],
            w[2] + WARP_SHIFT[2],
        ]) - 0.5,
    ];
    let flat = plane(p, n);
    let q = [
        (flat[0] + warp[0] * 2.0 * c.warp) / c.cell,
        (flat[1] + warp[1] * 2.0 * c.warp) / c.cell,
    ];
    let edge = voronoi_edge2(q, c.randomness, seed) * c.cell;
    let wide = c.width.max(pixel);
    let line = (1.0 - smoothstep(0.0, wide, edge)) * (c.width / wide);
    let m = p.map(|v| v * c.spread);
    let open = noise([
        m[0] + MASK_SHIFT[0],
        m[1] + MASK_SHIFT[1],
        m[2] + MASK_SHIFT[2],
    ]);
    let threshold = 1.0 - c.density;
    line * smoothstep(threshold - CRACK_SPREAD, threshold + CRACK_SPREAD, open)
}

pub fn track(uv: [f32; 2], t: Track, seed: u32, pixel: f32) -> f32 {
    if t.period <= 0.0 || t.band <= 0.0 {
        return 0.0;
    }
    let slice = (seed % TRACK_SLICES) as f32 * TRACK_SLICE;
    let wander = (noise([uv[1] * t.wander_frequency, slice, 0.0]) - 0.5) * 2.0 * t.wander;
    let across = (fract((uv[0] - t.offset + wander) / t.period + 0.5) - 0.5) * t.period;
    let off = (across.abs() - t.gauge).abs();
    let band = 1.0 - smoothstep(0.0, t.band, off);
    let streak = 0.6 + 0.8 * noise([uv[0] * t.streak[0], uv[1] * t.streak[1], slice + 5.0]);
    let weight = octave_weight(pixel * t.streak[0].max(t.streak[1]));
    clamp01(band * (1.0 + (streak - 1.0) * weight))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_edge_distance_is_zero_on_a_bisector_and_grows_inside_a_cell() {
        let mut inside = 0.0f32;
        let mut least = f32::MAX;
        for y in 0..64 {
            for x in 0..64 {
                let p = [x as f32 * 0.071 + 0.013, y as f32 * 0.067 + 0.029];
                let e = voronoi_edge2(p, 1.0, 5);
                assert!(e >= -1e-4, "{p:?} {e}");
                inside = inside.max(e);
                least = least.min(e);
            }
        }
        assert!(least < 0.02, "{least}");
        assert!(inside > 0.25, "{inside}");
        assert_ne!(
            voronoi_edge2([0.4, 0.6], 1.0, 5),
            voronoi_edge2([0.4, 0.6], 1.0, 6)
        );
    }

    #[test]
    fn a_wider_footprint_keeps_a_crack_s_share_of_the_ground() {
        let c = Crack {
            density: 1.0,
            ..Crack::SAMPLE
        };
        let share = |pixel: f32| {
            let mut sum = 0.0;
            let side = 400;
            for y in 0..side {
                for x in 0..side {
                    let p = [x as f32 * 0.0237, 0.0, y as f32 * 0.0241];
                    sum += crack(p, [0.0, 1.0, 0.0], c, 3, pixel);
                }
            }
            sum / (side * side) as f32
        };
        let sharp = share(0.0);
        let blurred = share(0.05);
        assert!(sharp > 0.0005, "{sharp}");
        assert!((blurred / sharp - 1.0).abs() < 0.5, "{sharp} {blurred}");
    }

    #[test]
    fn the_tracks_lie_a_gauge_either_side_of_each_lane_s_centre() {
        let t = Track {
            wander: 0.0,
            ..Track::SAMPLE
        };
        let at = |u: f32| track([u, 12.0], t, 0, 0.0);
        let centre = t.offset;
        assert!(at(centre - t.gauge) > 0.5);
        assert!(at(centre + t.gauge) > 0.5);
        assert!(at(centre + t.period - t.gauge) > 0.5);
        assert_eq!(at(centre), 0.0);
        assert_eq!(at(centre + t.period * 0.5), 0.0);
    }
}
