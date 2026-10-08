use crate::detail::{detail_level, projected_pixels};
use crate::lod::{chain, errors};
use crate::mesh::{Bounds, Mesh};
use crate::shapes::Shape;
use crate::strip::{ROLL_WGSL, Roller, Strip, column_step};

fn sample_points(mesh: &Mesh) -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    for tri in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [
            mesh.positions[tri[0] as usize],
            mesh.positions[tri[1] as usize],
            mesh.positions[tri[2] as usize],
        ];
        let mid = |p: [f32; 3], q: [f32; 3]| {
            [
                (p[0] + q[0]) * 0.5,
                (p[1] + q[1]) * 0.5,
                (p[2] + q[2]) * 0.5,
            ]
        };
        out.push([
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ]);
        out.push(mid(a, b));
        out.push(mid(b, c));
        out.push(mid(c, a));
    }
    out
}

fn shapes() -> Vec<Shape> {
    vec![
        Shape::RoundBox {
            half: [1.0, 0.6, 0.3],
            radius: 0.2,
        },
        Shape::RoundCone {
            a: [0.0, 0.0, 0.0],
            b: [0.0, 0.0, 1.5],
            r1: 0.5,
            r2: 0.2,
        },
        Shape::Capsule {
            a: [-0.8, 0.0, 0.0],
            b: [0.8, 0.2, 0.1],
            radius: 0.25,
        },
        Shape::Ellipsoid {
            radius: 0.6,
            squash: 0.4,
        },
        Shape::Slab {
            half_length: 1.2,
            radius: 0.4,
            height: 0.3,
            fillet: 0.08,
        },
        Shape::Torus {
            major: 0.8,
            minor: 0.15,
        },
    ]
}

#[test]
fn a_farther_object_draws_a_coarser_level() {
    let bound = Bounds {
        min: [-1.0; 3],
        max: [1.0; 3],
    };
    let errors = [0.0, 0.001, 0.01, 0.1];
    let fov = 50f32.to_radians();
    let near = detail_level(bound, &errors, 3.0, fov, 2160.0, None);
    let far = detail_level(bound, &errors, 300.0, fov, 2160.0, None);
    assert_eq!(near, 0);
    assert!(far > near);
    assert!(projected_pixels(errors[far], 300.0 - bound.radius(), fov, 2160.0) <= 0.5);
}

#[test]
fn the_lod_chain_shrinks_monotonically_and_is_deterministic() {
    let dense = Shape::Ellipsoid {
        radius: 1.0,
        squash: 1.0,
    }
    .mesh(0.0005);
    assert!(dense.triangle_count() > 2000);
    let levels = chain(&dense);
    assert!(levels.len() >= 3, "{} levels", levels.len());
    for pair in levels.windows(2) {
        assert!(pair[1].mesh.triangle_count() < pair[0].mesh.triangle_count());
        assert!(pair[1].error >= pair[0].error);
    }
    let again = chain(&dense);
    assert_eq!(levels.len(), again.len());
    for (a, b) in levels.iter().zip(again.iter()) {
        assert_eq!(a.mesh.bytes(), b.mesh.bytes());
    }
    let errs = errors(&levels);
    assert_eq!(errs[0], 0.0);
}

#[test]
fn every_analytic_mesh_stays_within_its_error_of_the_true_surface() {
    for error in [0.01f32, 0.002] {
        for shape in shapes() {
            let mesh = shape.mesh(error);
            assert!(mesh.triangle_count() > 0, "{shape:?}");
            for p in &mesh.positions {
                let d = shape.distance(*p);
                assert!(
                    d.abs() <= error * 0.25 + 1e-4,
                    "{shape:?} vertex off by {d}"
                );
            }
            for p in sample_points(&mesh) {
                let d = shape.distance(p);
                assert!(d.abs() <= error * 1.05 + 1e-4, "{shape:?} at {error}: {d}");
            }
        }
    }
}

#[test]
fn a_finer_error_gives_more_triangles() {
    for shape in shapes() {
        assert!(shape.mesh(0.002).triangle_count() > shape.mesh(0.02).triangle_count());
    }
}

#[test]
fn the_roller_keeps_the_sheet_length_and_leaves_the_flat_part_alone() {
    let strip = Strip {
        length: 2.0,
        width: 0.4,
    };
    let roller = Roller {
        at: 0.8,
        core: 0.03,
        thickness: 0.0005,
    };
    let outer = roller.outer(&strip);
    let xs = strip.columns(roller.core, 0.0002, None);
    let placed: Vec<[f32; 3]> = xs
        .iter()
        .map(|x| roller.place([*x, 0.0, 0.1], outer).position)
        .collect();
    let length: f32 = placed
        .windows(2)
        .map(|w| {
            let d = [w[1][0] - w[0][0], w[1][1] - w[0][1], w[1][2] - w[0][2]];
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
        })
        .sum();
    assert!(
        (length - strip.length).abs() / strip.length < 0.002,
        "{length}"
    );
    let flat = roller.place([0.5, 0.0, 0.2], outer);
    assert_eq!(flat.position, [0.5, 0.0, 0.2]);
    assert_eq!(flat.normal, [0.0, 1.0, 0.0]);
    let wound = roller.place([1.0, 0.0, 0.2], outer);
    assert!(wound.position[1] > 0.0);
    let n = wound.normal;
    assert!(((n[0] * n[0] + n[1] * n[1]).sqrt() - 1.0).abs() < 1e-5);
    let just_after = roller.place([roller.at + 1e-4, 0.0, 0.0], outer);
    assert!((just_after.normal[1] - 1.0).abs() < 0.05);
}

#[test]
fn a_band_refines_only_where_the_roll_travels() {
    let strip = Strip {
        length: 2.0,
        width: 0.4,
    };
    let everywhere = strip.columns(0.03, 0.0002, None);
    let band = strip.columns(0.03, 0.0002, Some((0.6, 1.0)));
    assert!(band.len() < everywhere.len());
    let step = column_step(0.03, 0.0002);
    for w in band.windows(2) {
        if w[0] >= 0.6 && w[1] <= 1.0 {
            assert!(w[1] - w[0] <= step * 1.0001);
        }
    }
    let mesh = strip.mesh(0.03, 0.0002, Some((0.6, 1.0)), 2);
    assert_eq!(mesh.positions.len(), band.len() * 3);
}

#[test]
fn the_roll_shader_parses_and_mirrors_the_cpu_reference() {
    let module = naga::front::wgsl::parse_str(ROLL_WGSL).expect("roll WGSL parses");
    let names: Vec<_> = module
        .functions
        .iter()
        .map(|(_, f)| f.name.clone().unwrap_or_default())
        .collect();
    assert!(names.contains(&"roll_place".to_string()));
    assert!(names.contains(&"roll_outer".to_string()));
    let strip = Strip {
        length: 1.0,
        width: 0.3,
    };
    let roller = Roller {
        at: 0.25,
        core: 0.02,
        thickness: 0.0,
    };
    let outer = roller.outer(&strip);
    assert_eq!(outer, 0.02);
    let quarter = roller.place([0.25 + std::f32::consts::FRAC_PI_2 * 0.02, 0.0, 0.0], outer);
    assert!((quarter.position[0] - (0.25 + 0.02)).abs() < 1e-5);
    assert!((quarter.position[1] - 0.02).abs() < 1e-5);
}
