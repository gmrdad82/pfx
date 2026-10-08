use crate::math::{add, arc_segments, basis, dot, length, normalize, scale, sub};
use crate::mesh::{Builder, Mesh, weld};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    RoundBox {
        half: [f32; 3],
        radius: f32,
    },
    RoundCone {
        a: [f32; 3],
        b: [f32; 3],
        r1: f32,
        r2: f32,
    },
    Capsule {
        a: [f32; 3],
        b: [f32; 3],
        radius: f32,
    },
    Ellipsoid {
        radius: f32,
        squash: f32,
    },
    Slab {
        half_length: f32,
        radius: f32,
        height: f32,
        fillet: f32,
    },
    Torus {
        major: f32,
        minor: f32,
    },
}

impl Shape {
    pub fn distance(&self, p: [f32; 3]) -> f32 {
        match *self {
            Shape::RoundBox { half, radius } => round_box(p, half, radius),
            Shape::RoundCone { a, b, r1, r2 } => round_cone(p, a, b, r1, r2),
            Shape::Capsule { a, b, radius } => round_cone(p, a, b, radius, radius),
            Shape::Ellipsoid { radius, squash } => ellipsoid(p, [radius, radius, radius * squash]),
            Shape::Slab {
                half_length,
                radius,
                height,
                fillet,
            } => slab(p, half_length, radius, height, fillet),
            Shape::Torus { major, minor } => {
                let q = [(p[0] * p[0] + p[1] * p[1]).sqrt() - major, p[2]];
                (q[0] * q[0] + q[1] * q[1]).sqrt() - minor
            }
        }
    }

    pub fn mesh(&self, error: f32) -> Mesh {
        let step = (error * 0.5).max(1e-6);
        match *self {
            Shape::RoundBox { half, radius } => round_box_mesh(half, radius, step, error),
            Shape::RoundCone { a, b, r1, r2 } => round_cone_mesh(a, b, r1, r2, step, error),
            Shape::Capsule { a, b, radius } => round_cone_mesh(a, b, radius, radius, step, error),
            Shape::Ellipsoid { radius, squash } => {
                ellipsoid_mesh([radius, radius, radius * squash], step, error)
            }
            Shape::Slab {
                half_length,
                radius,
                height,
                fillet,
            } => slab_mesh(half_length, radius, height, fillet, step, error),
            Shape::Torus { major, minor } => torus_mesh(major, minor, step, error),
        }
    }
}

fn round_box(p: [f32; 3], half: [f32; 3], radius: f32) -> f32 {
    let q = [
        p[0].abs() - half[0] + radius,
        p[1].abs() - half[1] + radius,
        p[2].abs() - half[2] + radius,
    ];
    let outside = length([q[0].max(0.0), q[1].max(0.0), q[2].max(0.0)]);
    outside + q[0].max(q[1]).max(q[2]).min(0.0) - radius
}

pub fn round_cone(p: [f32; 3], a: [f32; 3], b: [f32; 3], r1: f32, r2: f32) -> f32 {
    let ba = sub(b, a);
    let l2 = dot(ba, ba).max(1e-4);
    let rr = r1 - r2;
    let a2 = l2 - rr * rr;
    let il2 = 1.0 / l2;
    let pa = sub(p, a);
    let y = dot(pa, ba);
    let z = y - l2;
    let xv = sub(scale(pa, l2), scale(ba, y));
    let x2 = dot(xv, xv);
    let y2 = y * y * l2;
    let z2 = z * z * l2;
    let k = crate::math::sign(rr) * rr * rr * x2;
    if crate::math::sign(z) * a2 * z2 > k {
        return (x2 + z2).sqrt() * il2 - r2;
    }
    if crate::math::sign(y) * a2 * y2 < k {
        return (x2 + y2).sqrt() * il2 - r1;
    }
    ((x2 * a2 * il2).sqrt() + y * rr) * il2 - r1
}

fn ellipsoid(p: [f32; 3], radii: [f32; 3]) -> f32 {
    let k0 = length([p[0] / radii[0], p[1] / radii[1], p[2] / radii[2]]);
    let k1 = length([
        p[0] / (radii[0] * radii[0]),
        p[1] / (radii[1] * radii[1]),
        p[2] / (radii[2] * radii[2]),
    ]);
    k0 * (k0 - 1.0) / k1.max(1e-5)
}

fn slab(p: [f32; 3], half_length: f32, radius: f32, height: f32, fillet: f32) -> f32 {
    let flat = length([(p[0].abs() - (half_length - radius)).max(0.0), p[1], 0.0]) - radius;
    let e = fillet.min(height * 0.5).min(radius);
    let w = [flat + e, (p[2] - height * 0.5).abs() - height * 0.5 + e];
    w[0].max(w[1]).min(0.0) + (w[0].max(0.0).powi(2) + w[1].max(0.0).powi(2)).sqrt() - e
}

fn segments(radius: f32, step: f32, span: f32, least: usize) -> usize {
    arc_segments(radius, step, span).max(least)
}

fn round_box_mesh(half: [f32; 3], radius: f32, step: f32, error: f32) -> Mesh {
    let radius = radius.clamp(0.0, half[0].min(half[1]).min(half[2]));
    let inner = [half[0] - radius, half[1] - radius, half[2] - radius];
    let k = if radius > 0.0 {
        segments(radius, step * 0.6, FRAC_PI_4, 1)
    } else {
        0
    };
    let axis_samples = |axis: usize| -> Vec<f32> {
        let mut out = Vec::new();
        for i in (0..=k).rev() {
            let t = (FRAC_PI_4 * i as f32 / k.max(1) as f32).tan();
            out.push(-inner[axis] - radius * t);
        }
        for i in 0..=k {
            let t = (FRAC_PI_4 * i as f32 / k.max(1) as f32).tan();
            out.push(inner[axis] + radius * t);
        }
        out.dedup_by(|a, b| (*a - *b).abs() <= 1e-7);
        out
    };
    let mut builder = Builder::new();
    for face in 0..6 {
        let normal_axis = face / 2;
        let sign = if face % 2 == 0 { 1.0 } else { -1.0 };
        let u_axis = (normal_axis + 1) % 3;
        let v_axis = (normal_axis + 2) % 3;
        let us = axis_samples(u_axis);
        let vs = axis_samples(v_axis);
        let base = |i: usize, j: usize, builder: &mut Builder| -> u32 {
            let mut p = [0.0f32; 3];
            p[normal_axis] = sign * half[normal_axis];
            p[u_axis] = us[i];
            p[v_axis] = vs[j];
            let q = [
                p[0].clamp(-inner[0], inner[0]),
                p[1].clamp(-inner[1], inner[1]),
                p[2].clamp(-inner[2], inner[2]),
            ];
            let mut n = [0.0f32; 3];
            n[normal_axis] = sign;
            let d = sub(p, q);
            let normal = if length(d) > 1e-9 { normalize(d) } else { n };
            let position = add(q, scale(normal, radius));
            let uv = [
                (us[i] / half[u_axis].max(1e-6)) * 0.5 + 0.5,
                (vs[j] / half[v_axis].max(1e-6)) * 0.5 + 0.5,
            ];
            builder.vertex(position, normal, uv)
        };
        let mut grid = vec![vec![0u32; vs.len()]; us.len()];
        for (i, row) in grid.iter_mut().enumerate() {
            for (j, slot) in row.iter_mut().enumerate() {
                *slot = base(i, j, &mut builder);
            }
        }
        for i in 0..us.len() - 1 {
            for j in 0..vs.len() - 1 {
                let (a, b, c, d) = (
                    grid[i][j],
                    grid[i + 1][j],
                    grid[i + 1][j + 1],
                    grid[i][j + 1],
                );
                builder.tri(a, b, c);
                builder.tri(a, c, d);
            }
        }
    }
    weld(builder.build(), (error * 0.01).max(1e-6))
}

fn revolve(
    a: [f32; 3],
    axis: [f32; 3],
    profile: &[([f32; 2], [f32; 2])],
    around: usize,
    error: f32,
) -> Mesh {
    let (x, y) = basis(axis);
    let mut builder = Builder::new();
    let mut rings = Vec::with_capacity(profile.len());
    for (k, ((s, rho), (ns, nrho))) in profile
        .iter()
        .map(|(p, n)| ((p[0], p[1]), (n[0], n[1])))
        .enumerate()
    {
        let mut ring = Vec::with_capacity(around + 1);
        for i in 0..=around {
            let phi = TAU * i as f32 / around as f32;
            let radial = add(scale(x, phi.cos()), scale(y, phi.sin()));
            let position = add(add(a, scale(axis, s)), scale(radial, rho));
            let normal = add(scale(axis, ns), scale(radial, nrho));
            let uv = [
                i as f32 / around as f32,
                k as f32 / (profile.len() - 1) as f32,
            ];
            ring.push(builder.vertex(position, normal, uv));
        }
        rings.push(ring);
    }
    for k in 0..rings.len() - 1 {
        for i in 0..around {
            let (p, q, r, t) = (
                rings[k][i],
                rings[k][i + 1],
                rings[k + 1][i + 1],
                rings[k + 1][i],
            );
            builder.tri(p, q, r);
            builder.tri(p, r, t);
        }
    }
    weld(builder.build(), (error * 0.01).max(1e-6))
}

fn round_cone_mesh(a: [f32; 3], b: [f32; 3], r1: f32, r2: f32, step: f32, error: f32) -> Mesh {
    let ab = sub(b, a);
    let h = length(ab);
    if h <= 1e-6 || (r1 - r2).abs() >= h {
        let (center, radius) = if r1 >= r2 { (a, r1) } else { (b, r2) };
        let mut mesh = ellipsoid_mesh([radius; 3], step, error);
        for p in &mut mesh.positions {
            *p = add(*p, center);
        }
        return Mesh::new(
            mesh.positions,
            mesh.normals,
            mesh.tangents,
            mesh.uvs,
            mesh.indices,
        );
    }
    let axis = scale(ab, 1.0 / h);
    let beta = ((r1 - r2) / h).asin();
    let tangent = FRAC_PI_2 - beta;
    let mut profile = Vec::new();
    let south = segments(r1, step, PI - tangent, 2);
    for i in 0..=south {
        let g = PI - (PI - tangent) * i as f32 / south as f32;
        profile.push(([r1 * g.cos(), r1 * g.sin()], [g.cos(), g.sin()]));
    }
    let north = segments(r2, step, tangent, 2);
    for i in 0..=north {
        let g = tangent - tangent * i as f32 / north as f32;
        profile.push(([h + r2 * g.cos(), r2 * g.sin()], [g.cos(), g.sin()]));
    }
    let around = segments(r1.max(r2), step, TAU, 8);
    revolve(a, axis, &profile, around, error)
}

fn ellipsoid_mesh(radii: [f32; 3], step: f32, error: f32) -> Mesh {
    let big = radii[0].max(radii[1]).max(radii[2]);
    let around = segments(big, step, TAU, 8);
    let rings = segments(big, step, PI, 4);
    let mut builder = Builder::new();
    let mut grid = Vec::with_capacity(rings + 1);
    for k in 0..=rings {
        let theta = PI * k as f32 / rings as f32;
        let mut row = Vec::with_capacity(around + 1);
        for i in 0..=around {
            let phi = TAU * i as f32 / around as f32;
            let unit = [
                theta.sin() * phi.cos(),
                theta.sin() * phi.sin(),
                theta.cos(),
            ];
            let position = [unit[0] * radii[0], unit[1] * radii[1], unit[2] * radii[2]];
            let normal = [
                position[0] / (radii[0] * radii[0]),
                position[1] / (radii[1] * radii[1]),
                position[2] / (radii[2] * radii[2]),
            ];
            let uv = [i as f32 / around as f32, k as f32 / rings as f32];
            row.push(builder.vertex(position, normal, uv));
        }
        grid.push(row);
    }
    for k in 0..rings {
        for i in 0..around {
            let (p, q, r, t) = (
                grid[k][i],
                grid[k][i + 1],
                grid[k + 1][i + 1],
                grid[k + 1][i],
            );
            builder.tri(p, q, r);
            builder.tri(p, r, t);
        }
    }
    weld(builder.build(), (error * 0.01).max(1e-6))
}

fn slab_mesh(
    half_length: f32,
    radius: f32,
    height: f32,
    fillet: f32,
    step: f32,
    error: f32,
) -> Mesh {
    let fillet = fillet.min(height * 0.5).min(radius).max(0.0);
    let core = radius - fillet;
    let straight = (half_length - radius).max(0.0);
    let end = segments(radius, step, PI, 4);
    let mut outline: Vec<([f32; 2], [f32; 2])> = Vec::new();
    for (cx, from) in [(straight, -FRAC_PI_2), (-straight, FRAC_PI_2)] {
        for i in 0..=end {
            let angle = from + PI * i as f32 / end as f32;
            let n = [angle.cos(), angle.sin()];
            outline.push(([cx + core * n[0], core * n[1]], n));
        }
    }
    let bend = if fillet > 0.0 {
        segments(fillet, step, FRAC_PI_2, 1)
    } else {
        0
    };
    let mut profile: Vec<(f32, f32, f32, f32)> = Vec::new();
    for i in 0..=bend {
        let g = FRAC_PI_2 * i as f32 / bend.max(1) as f32;
        profile.push((
            fillet * g.sin(),
            height - fillet + fillet * g.cos(),
            g.sin(),
            g.cos(),
        ));
    }
    for i in 0..=bend {
        let g = FRAC_PI_2 * i as f32 / bend.max(1) as f32;
        profile.push((
            fillet * g.cos(),
            fillet - fillet * g.sin(),
            g.cos(),
            -g.sin(),
        ));
    }
    let mut builder = Builder::new();
    let count = outline.len();
    let mut grid = Vec::with_capacity(profile.len());
    for (k, &(o, z, nn, nz)) in profile.iter().enumerate() {
        let mut row = Vec::with_capacity(count);
        for (j, (p, n)) in outline.iter().enumerate() {
            let position = [p[0] + n[0] * o, p[1] + n[1] * o, z];
            let normal = [n[0] * nn, n[1] * nn, nz];
            let uv = [
                j as f32 / count as f32,
                k as f32 / (profile.len() - 1) as f32,
            ];
            row.push(builder.vertex(position, normal, uv));
        }
        grid.push(row);
    }
    for k in 0..grid.len() - 1 {
        for j in 0..count {
            let jn = (j + 1) % count;
            let (p, q, r, t) = (grid[k][j], grid[k][jn], grid[k + 1][jn], grid[k + 1][j]);
            builder.tri(p, q, r);
            builder.tri(p, r, t);
        }
    }
    for (z, nz, row) in [(height, 1.0f32, 0usize), (0.0, -1.0, grid.len() - 1)] {
        let center = builder.vertex([0.0, 0.0, z], [0.0, 0.0, nz], [0.5, 0.5]);
        for j in 0..count {
            let jn = (j + 1) % count;
            builder.tri(center, grid[row][j], grid[row][jn]);
        }
    }
    weld(builder.build(), (error * 0.01).max(1e-6))
}

fn torus_mesh(major: f32, minor: f32, step: f32, error: f32) -> Mesh {
    let around = segments(major + minor, step, TAU, 8);
    let tube = segments(minor, step, TAU, 6);
    let mut builder = Builder::new();
    let mut grid = Vec::with_capacity(around + 1);
    for i in 0..=around {
        let phi = TAU * i as f32 / around as f32;
        let mut row = Vec::with_capacity(tube + 1);
        for j in 0..=tube {
            let theta = TAU * j as f32 / tube as f32;
            let ring = major + minor * theta.cos();
            let position = [ring * phi.cos(), ring * phi.sin(), minor * theta.sin()];
            let normal = [
                theta.cos() * phi.cos(),
                theta.cos() * phi.sin(),
                theta.sin(),
            ];
            let uv = [i as f32 / around as f32, j as f32 / tube as f32];
            row.push(builder.vertex(position, normal, uv));
        }
        grid.push(row);
    }
    for i in 0..around {
        for j in 0..tube {
            let (p, q, r, t) = (
                grid[i][j],
                grid[i + 1][j],
                grid[i + 1][j + 1],
                grid[i][j + 1],
            );
            builder.tri(p, q, r);
            builder.tri(p, r, t);
        }
    }
    weld(builder.build(), (error * 0.01).max(1e-6))
}
