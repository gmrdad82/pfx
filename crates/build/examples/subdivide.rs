use pfx_build::subdiv::{Cage, Crease, Options};
use std::f64::consts::TAU;
use std::time::Instant;

fn torus(around: u32, along: u32) -> Cage {
    let mut points = Vec::new();
    let mut uvs = Vec::new();
    for j in 0..along {
        for i in 0..around {
            let (u, v) = (
                i as f64 / around as f64 * TAU,
                j as f64 / along as f64 * TAU,
            );
            let r = 2.0 + 0.6 * v.cos();
            points.push([r * u.cos(), 0.6 * v.sin(), r * u.sin()]);
        }
    }
    let at = |i: u32, j: u32| (j % along) * around + i % around;
    let mut faces = Vec::new();
    for j in 0..along {
        for i in 0..around {
            faces.push(vec![at(i, j), at(i, j + 1), at(i + 1, j + 1), at(i + 1, j)]);
            let (u0, u1) = (i as f64 / around as f64, (i + 1) as f64 / around as f64);
            let (v0, v1) = (j as f64 / along as f64, (j + 1) as f64 / along as f64);
            uvs.push(vec![[u0, v0], [u0, v1], [u1, v1], [u1, v0]]);
        }
    }
    let creases = (0..around)
        .map(|i| Crease {
            edge: [at(i, 0), at(i + 1, 0)],
            sharpness: 2.5,
        })
        .collect();
    Cage {
        points,
        faces,
        creases,
        corners: Vec::new(),
        uvs: Some(uvs),
    }
}

fn main() {
    let cage = torus(40, 25);
    let options = Options::default();
    let mut times = Vec::new();
    let mut triangles = 0;
    for _ in 0..5 {
        let start = Instant::now();
        let mesh = pfx_build::subdivide(&cage, 3, &options).unwrap();
        times.push(start.elapsed().as_secs_f64() * 1e3);
        triangles = mesh.triangle_count();
    }
    times.sort_by(f64::total_cmp);
    println!(
        "{} faces at level 3: {} triangles, median {:.1} ms (min {:.1}, max {:.1})",
        cage.faces.len(),
        triangles,
        times[2],
        times[0],
        times[4]
    );
}
