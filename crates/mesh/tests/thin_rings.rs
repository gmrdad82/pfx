#[allow(dead_code)]
mod common;

use common::*;
use glam::DVec2;
use pfx_mesh::outline::{self, Outline, Shape, superellipse};
use pfx_mesh::*;

const SIZES: [f64; 7] = [0.001, 0.003, 0.01, 0.05, 0.2, 0.5, 1.0];

fn ring(size: f64, exponent: f64, segments: u32) -> Vec<Outline> {
    let half = [size * 0.5, size * 0.3];
    let wall = (0.004_f64).min(size * 0.04);
    let centre = [size * 0.7, -size * 0.2];
    vec![
        superellipse(centre, half, exponent, segments),
        superellipse(centre, [half[0] - wall, half[1] - wall], exponent, segments),
    ]
}

fn points(o: &Outline) -> Vec<DVec2> {
    o.0.iter().map(|&p| DVec2::from(p)).collect()
}

fn signed(p: &[DVec2], t: &[u32; 3]) -> f64 {
    let [a, b, c] = t.map(|i| p[i as usize]);
    (b - a).perp_dot(c - a) * 0.5
}

#[test]
fn thin_superellipse_rings_fill_outer_minus_hole_and_face_one_way() {
    for size in SIZES {
        for (exponent, segments) in [(4.0, 64), (2.5, 128), (8.0, 256)] {
            let rings = ring(size, exponent, segments);
            let want = rings[0].area().abs() - rings[1].area().abs();
            let found = outline::even_odd(&rings);
            assert_eq!(found.len(), 1, "size {size}");
            let shape = &found[0];
            assert_eq!(shape.holes.len(), 1, "size {size}");
            let mut pts = points(&shape.outer);
            pts.extend(points(&shape.holes[0]));
            let tris = outline::triangulate(shape);
            let areas: Vec<f64> = tris.iter().map(|t| signed(&pts, t)).collect();
            assert!(
                areas.iter().all(|&a| a > 0.0),
                "size {size} e {exponent}: a triangle faces the other way"
            );
            let sum: f64 = areas.iter().sum();
            assert!((shape.area() - want).abs() <= 1e-6 * want, "size {size}");
            assert!(
                (sum - shape.area()).abs() <= 1e-9 * want,
                "size {size} e {exponent}: {sum} vs {want}"
            );
            let direct = Shape {
                outer: rings[0].clone(),
                holes: vec![rings[1].clone()],
            };
            let depth = 0.0005_f64.min(size * 0.1);
            for s in [shape, &direct] {
                let m = extrude_shape(s, 0.0, depth, &Profile::NONE, UvMode::Local);
                assert_sane(&m);
                assert_closed(&m);
                assert_outward(&m);
                let (mut front, mut back) = (0.0, 0.0);
                for t in m.indices.chunks_exact(3) {
                    let p = [m.point(t[0]), m.point(t[1]), m.point(t[2])];
                    let n = (p[1] - p[0]).cross(p[2] - p[0]);
                    if p.iter().all(|q| q.z == 0.0) {
                        assert!(n.z > 0.0, "size {size}: a front triangle faces back");
                        front += n.length() * 0.5;
                    } else if p.iter().all(|q| (q.z + depth).abs() < 1e-9) {
                        assert!(n.z < 0.0, "size {size}: a back triangle faces front");
                        back += n.length() * 0.5;
                    }
                }
                for cap in [front, back] {
                    assert!(
                        (cap - want).abs() <= 1e-4 * want,
                        "size {size} e {exponent}: cap {cap} vs {want}"
                    );
                }
            }
        }
    }
}
