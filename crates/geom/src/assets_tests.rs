use std::collections::BTreeMap;
use std::f64::consts::PI;

use crate::field::{Surface, gradient};
use crate::icon::{Icon, Part, PartShape};
use crate::mark::{Each, Form, Mark, Profile};
use crate::mesh::Mesh;
use crate::svg::{Drawing, Paint};

const SAMPLE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><circle cx="32" cy="32" r="24" fill="#808080"/><rect x="26" y="14" width="12" height="36" fill="#404040"/></svg>"##;

fn volume(mesh: &Mesh) -> f64 {
    mesh.indices
        .chunks_exact(3)
        .map(|t| {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize].map(f64::from));
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0
        })
        .sum()
}

fn closed(mesh: &Mesh) -> Result<(), String> {
    let mut ids: BTreeMap<[u32; 3], u32> = BTreeMap::new();
    let welded = mesh
        .positions
        .iter()
        .map(|p| {
            let key = p.map(f32::to_bits);
            let next = ids.len() as u32;
            *ids.entry(key).or_insert(next)
        })
        .collect::<Vec<_>>();
    let mut edges: BTreeMap<(u32, u32), i64> = BTreeMap::new();
    for t in mesh.indices.chunks_exact(3) {
        for i in 0..3 {
            let a = welded[t[i] as usize];
            let b = welded[t[(i + 1) % 3] as usize];
            if a == b {
                return Err(format!("degenerate triangle {t:?}"));
            }
            *edges.entry((a.min(b), a.max(b))).or_default() += if a < b { 1 } else { -1 };
        }
    }
    match edges.iter().find(|(_, count)| **count != 0) {
        Some((edge, count)) => Err(format!("edge {edge:?} is open by {count}")),
        None => Ok(()),
    }
}

fn silhouette(mesh: &Mesh, lo: [f64; 2], hi: [f64; 2], n: usize) -> f64 {
    let cell = [(hi[0] - lo[0]) / n as f64, (hi[1] - lo[1]) / n as f64];
    let mut covered = vec![false; n * n];
    for t in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| {
            let p = mesh.positions[i as usize];
            [p[0] as f64, p[1] as f64]
        });
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if area.abs() < 1e-14 {
            continue;
        }
        let x0 = (((a[0].min(b[0]).min(c[0]) - lo[0]) / cell[0])
            .floor()
            .max(0.0)) as usize;
        let x1 = (((a[0].max(b[0]).max(c[0]) - lo[0]) / cell[0]).ceil() as usize).min(n);
        let y0 = (((a[1].min(b[1]).min(c[1]) - lo[1]) / cell[1])
            .floor()
            .max(0.0)) as usize;
        let y1 = (((a[1].max(b[1]).max(c[1]) - lo[1]) / cell[1]).ceil() as usize).min(n);
        for y in y0..y1 {
            for x in x0..x1 {
                let p = [
                    lo[0] + (x as f64 + 0.5) * cell[0],
                    lo[1] + (y as f64 + 0.5) * cell[1],
                ];
                let e = |u: [f64; 2], v: [f64; 2]| {
                    (v[0] - u[0]) * (p[1] - u[1]) - (v[1] - u[1]) * (p[0] - u[0])
                };
                let (e0, e1, e2) = (e(a, b), e(b, c), e(c, a));
                if (e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0) || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0) {
                    covered[y * n + x] = true;
                }
            }
        }
    }
    covered.iter().filter(|c| **c).count() as f64 * cell[0] * cell[1]
}

fn facing(mesh: &Mesh) -> f64 {
    let mut good = 0usize;
    let mut total = 0usize;
    for t in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize]);
        let face = cross(sub(b, a), sub(c, a));
        if dot(face, face) < 1e-20 {
            continue;
        }
        let n = [t[0], t[1], t[2]]
            .map(|i| mesh.normals[i as usize])
            .iter()
            .fold([0.0; 3], |s, n| [s[0] + n[0], s[1] + n[1], s[2] + n[2]]);
        total += 1;
        if dot(face, n) > 0.0 {
            good += 1;
        }
    }
    good as f64 / total.max(1) as f64
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit_normals(mesh: &Mesh) {
    for n in &mesh.normals {
        let len = dot(*n, *n).sqrt();
        assert!((len - 1.0).abs() < 1e-4, "normal {n:?} has length {len}");
    }
}

fn svg(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">{body}</svg>"#)
}

fn faceted_volume(area: impl Fn(f64) -> f64, depth: f64, bevel: f64, segments: u32) -> f64 {
    let steps = segments + 1;
    let mut points = Vec::new();
    for k in 0..=steps {
        let g = PI * 0.5 * k as f64 / steps as f64;
        points.push((bevel * g.cos(), depth + bevel * g.sin()));
    }
    let mut v = 2.0 * depth * area(bevel);
    for w in points.windows(2) {
        let ((o0, z0), (o1, z1)) = (w[0], w[1]);
        let mid = area((o0 + o1) * 0.5);
        v += 2.0 * (z1 - z0) * (area(o0) + 4.0 * mid + area(o1)) / 6.0;
    }
    v
}

#[test]
fn a_rectangle_extrudes_to_the_exact_bevelled_volume() {
    let mark = Mark::build(
        &svg(r##"<rect x="20" y="30" width="60" height="40" fill="#808080"/>"##),
        &Form::default(),
    )
    .unwrap();
    assert_eq!(mark.solids.len(), 1);
    let mesh = &mark.solids[0].mesh;
    let (w, h) = (0.6, 0.4);
    let area = |o: f64| (w + 2.0 * o) * (h + 2.0 * o);
    let expected = faceted_volume(area, 0.12, 0.05, 8);
    let got = volume(mesh);
    assert!(
        (got - expected).abs() / expected < 1e-4,
        "volume {got} against {expected}"
    );
    closed(mesh).unwrap();
    unit_normals(mesh);
    assert_eq!(facing(mesh), 1.0);
    let b = mesh.bounds;
    for (got, want) in [
        (b.min[0], -0.35),
        (b.max[0], 0.35),
        (b.min[1], -0.25),
        (b.max[1], 0.25),
        (b.min[2], -0.17),
        (b.max[2], 0.17),
    ] {
        assert!((got - want).abs() < 1e-5, "bound {got} against {want}");
    }
}

#[test]
fn faces_are_flat_and_the_bevel_is_smooth() {
    let mark = Mark::build(
        &svg(r##"<rect x="20" y="30" width="60" height="40" fill="#808080"/>"##),
        &Form::default(),
    )
    .unwrap();
    let mesh = &mark.solids[0].mesh;
    let top = 0.17f32;
    let mut on_top = 0;
    for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
        if (p[2] - top).abs() < 1e-6 {
            on_top += 1;
            assert_eq!(*n, [0.0, 0.0, 1.0]);
        }
        if (p[2] + top).abs() < 1e-6 {
            assert_eq!(*n, [0.0, 0.0, -1.0]);
        }
        if p[2].abs() < 1e-6 {
            assert!(n[2].abs() < 1e-6);
        }
    }
    assert_eq!(on_top, 4);
    let mut at_corner: BTreeMap<[u32; 3], Vec<[f32; 3]>> = BTreeMap::new();
    for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
        at_corner.entry(p.map(f32::to_bits)).or_default().push(*n);
    }
    let split = at_corner.values().filter(|n| n.len() > 1).count();
    assert_eq!(
        split,
        4 * 18,
        "rectangle corners split their normals along the whole profile but the caps"
    );
    let chamfer = Form {
        profile: Profile::Chamfer,
        ..Form::default()
    };
    let flat = Mark::build(
        &svg(r##"<rect x="20" y="30" width="60" height="40" fill="#808080"/>"##),
        &chamfer,
    )
    .unwrap();
    let mesh = &flat.solids[0].mesh;
    closed(mesh).unwrap();
    let slanted = mesh
        .normals
        .iter()
        .filter(|n| (n[2] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4)
        .count();
    assert_eq!(slanted, 16);
}

#[test]
fn a_disc_matches_the_analytic_solid_and_silhouette() {
    let mark = Mark::build(
        SAMPLE
            .replace(
                r##"<rect x="26" y="14" width="12" height="36" fill="#404040"/>"##,
                "",
            )
            .as_str(),
        &Form::default(),
    )
    .unwrap();
    let mesh = &mark.solids[0].mesh;
    let r = 24.0 / 64.0;
    let area = |o: f64| PI * (r + o) * (r + o);
    let expected = faceted_volume(area, 0.12, 0.05, 8);
    let got = volume(mesh);
    assert!(
        (got - expected).abs() / expected < 2e-3,
        "volume {got} against {expected}"
    );
    let shadow = silhouette(mesh, [-0.5, -0.5], [0.5, 0.5], 500);
    let disc = PI * (r + 0.05) * (r + 0.05);
    assert!(
        (shadow - disc).abs() / disc < 5e-3,
        "silhouette {shadow} against {disc}"
    );
    closed(mesh).unwrap();
    assert_eq!(facing(mesh), 1.0);
}

#[test]
fn holes_by_fill_rule_and_mask_stay_open() {
    let ring = svg(
        r##"<path fill-rule="evenodd" fill="#808080" d="M10 10 H90 V90 H10 Z M30 30 H70 V70 H30 Z"/>"##,
    );
    let mark = Mark::build(&ring, &Form::default()).unwrap();
    let mesh = &mark.solids[0].mesh;
    closed(mesh).unwrap();
    let (outer, inner) = (0.8, 0.4);
    let area =
        |o: f64| (outer + 2.0 * o) * (outer + 2.0 * o) - (inner - 2.0 * o) * (inner - 2.0 * o);
    let expected = faceted_volume(area, 0.12, 0.05, 8);
    let got = volume(mesh);
    assert!(
        (got - expected).abs() / expected < 1e-4,
        "volume {got} against {expected}"
    );
    let masked = svg(
        r##"<mask id="m"><rect x="0" y="0" width="100" height="100" fill="white"/><rect x="30" y="30" width="40" height="40" fill="black"/></mask><rect x="10" y="10" width="80" height="80" fill="#808080" mask="url(#m)"/>"##,
    );
    let mark = Mark::build(&masked, &Form::default()).unwrap();
    let mesh = &mark.solids[0].mesh;
    closed(mesh).unwrap();
    let got = volume(mesh);
    assert!(
        (got - expected).abs() / expected < 1e-4,
        "masked volume {got} against {expected}"
    );
}

#[test]
fn the_sample_mark_layers_its_paints_and_counts_its_triangles() {
    let mark = Mark::build(SAMPLE, &Form::default()).unwrap();
    assert_eq!(mark.solids.len(), 2);
    assert_eq!(mark.solids[0].paint, Paint::Color([0x80, 0x80, 0x80]));
    assert_eq!(mark.solids[1].paint, Paint::Color([0x40, 0x40, 0x40]));
    let lift = mark.solids[1].mesh.bounds.max[2] - mark.solids[0].mesh.bounds.max[2];
    assert!((lift - 0.25 * 0.12).abs() < 1e-5, "layer lift {lift}");
    for solid in &mark.solids {
        closed(&solid.mesh).unwrap();
        unit_normals(&solid.mesh);
        assert_eq!(facing(&solid.mesh), 1.0);
    }
    let counts = mark
        .solids
        .iter()
        .map(|s| s.mesh.triangle_count())
        .collect::<Vec<_>>();
    assert_eq!(counts, [3_876, 156]);
}

#[test]
fn same_paint_merges_into_one_solid_unless_told_not_to() {
    let two = svg(
        r##"<rect x="10" y="10" width="50" height="50" fill="#808080"/><rect x="40" y="40" width="50" height="50" fill="#808080"/>"##,
    );
    let mark = Mark::build(&two, &Form::default()).unwrap();
    assert_eq!(mark.solids.len(), 1);
    assert_eq!(mark.solids[0].elements, [0, 1]);
    closed(&mark.solids[0].mesh).unwrap();
    let apart = Form {
        merge: false,
        ..Form::default()
    };
    let mark = Mark::build(&two, &apart).unwrap();
    assert_eq!(mark.solids.len(), 2);
    let mut materials = Form::default();
    materials.element_material.insert(1, "glyph".to_string());
    let mark = Mark::build(&two, &materials).unwrap();
    assert_eq!(mark.solids.len(), 2);
    assert_eq!(mark.solids[1].material.as_deref(), Some("glyph"));
}

#[test]
fn a_merged_fill_keeps_each_elements_own_bevel() {
    let mixed = svg(
        r##"<circle cx="30" cy="50" r="15" fill="#808080"/><rect x="60" y="48" width="4" height="4" fill="#808080"/><rect x="64" y="48" width="4" height="4" fill="#808080"/>"##,
    );
    let mark = Mark::build(&mixed, &Form::default()).unwrap();
    assert_eq!(mark.solids.len(), 1);
    assert_eq!(mark.solids[0].elements, [0, 1, 2]);
    let mesh = &mark.solids[0].mesh;
    closed(mesh).unwrap();
    unit_normals(mesh);
    let top = |lo: f32, hi: f32| {
        mesh.positions
            .iter()
            .filter(|p| p[0] > lo && p[0] < hi)
            .map(|p| p[2])
            .fold(f32::MIN, f32::max)
    };
    assert!((top(-0.4, 0.0) - 0.17).abs() < 1e-5, "{}", top(-0.4, 0.0));
    assert!(
        (top(0.05, 0.25) - 0.128).abs() < 1e-5,
        "{}",
        top(0.05, 0.25)
    );
    let pair = svg(
        r##"<rect x="60" y="48" width="4" height="4" fill="#808080"/><rect x="64" y="48" width="4" height="4" fill="#808080"/>"##,
    );
    let mark = Mark::build(&pair, &Form::default()).unwrap();
    let area = |o: f64| (0.08 + 2.0 * o) * (0.04 + 2.0 * o);
    let expected = faceted_volume(area, 0.12, 0.008, 8);
    let got = volume(&mark.solids[0].mesh);
    assert!(
        (got - expected).abs() / expected < 1e-4,
        "touching elements of one bevel join in 2D: {got} against {expected}"
    );
}

#[test]
fn element_depth_lift_and_inset_follow_the_form() {
    let one = svg(r##"<rect x="20" y="20" width="60" height="60" fill="#808080"/>"##);
    let mut form = Form::default();
    form.element_depth.by_element.insert(0, 2.0);
    form.element_z.by_element.insert(0, 0.5);
    let mark = Mark::build(&one, &form).unwrap();
    let b = mark.solids[0].mesh.bounds;
    assert!((b.max[2] - (0.17 * 2.0 + 0.06)).abs() < 1e-5);
    assert!((b.min[2] - (-0.17 * 2.0 + 0.06)).abs() < 1e-5);
    let inset = Form {
        inset: Each::new(5.0),
        ..Form::default()
    };
    let mark = Mark::build(&one, &inset).unwrap();
    let b = mark.solids[0].mesh.bounds;
    assert!(
        (b.max[0] - (0.25 + 0.05)).abs() < 1e-4,
        "inset bound {}",
        b.max[0]
    );
}

#[test]
fn a_ground_rect_is_the_ground_not_a_solid() {
    let tiled = svg(
        r##"<rect width="100" height="100" fill="#202020"/><circle cx="50" cy="50" r="20" fill="#808080"/>"##,
    );
    let mark = Mark::build(&tiled, &Form::default()).unwrap();
    assert_eq!(mark.ground, Some(Paint::Color([0x20, 0x20, 0x20])));
    assert_eq!(mark.solids.len(), 1);
    assert_eq!(mark.solids[0].elements, [0]);
}

#[test]
fn a_stroke_becomes_a_round_tube() {
    let ring =
        svg(r##"<circle cx="50" cy="50" r="30" fill="none" stroke="#808080" stroke-width="8"/>"##);
    let mark = Mark::build(&ring, &Form::default()).unwrap();
    let mesh = &mark.solids[0].mesh;
    closed(mesh).unwrap();
    unit_normals(mesh);
    assert!(facing(mesh) > 0.999);
    let (big, small) = (0.3, 0.04);
    let expected = 2.0 * PI * PI * big * small * small;
    let got = volume(mesh);
    assert!(
        (got - expected).abs() / expected < 0.01,
        "tube volume {got} against {expected}"
    );
    let b = mesh.bounds;
    assert!((b.max[2] - small as f32).abs() < 2e-3 && (b.max[0] - 0.34).abs() < 2e-3);
}

#[test]
fn a_fill_stroked_in_its_own_paint_inflates_into_a_pillow() {
    let pillow = svg(
        r##"<rect x="20" y="20" width="60" height="60" fill="#808080" stroke="#808080" stroke-width="10"/>"##,
    );
    let mark = Mark::build(&pillow, &Form::default()).unwrap();
    assert_eq!(mark.solids.len(), 1);
    let mesh = &mark.solids[0].mesh;
    closed(mesh).unwrap();
    assert!(facing(mesh) > 0.999);
    let (side, grow) = (0.6, 0.05);
    let core = grow * 0.15;
    let reach = grow - core;
    let steiner = |a: f64, perimeter: f64, r: f64| a + perimeter * r + PI * r * r;
    let slab = 2.0 * core * steiner(side * side, 4.0 * side, reach);
    let rounded = side * side * 2.0 * reach
        + 4.0 * side * PI * reach * reach * 0.5
        + 4.0 / 3.0 * PI * reach.powi(3);
    let expected = slab + rounded;
    let got = volume(mesh);
    assert!(
        (got - expected).abs() / expected < 0.01,
        "pillow volume {got} against {expected}"
    );
}

#[test]
fn curves_flatten_within_the_tolerance() {
    let curve = svg(
        r##"<path fill="#808080" d="M20 50 C20 20 80 20 80 50 Q80 80 50 80 A30 30 0 0 1 20 50 Z"/>"##,
    );
    let drawing = Drawing::parse(&curve, 2e-4).unwrap();
    assert_eq!(drawing.elements.len(), 1);
    assert!(drawing.elements[0].paths[0].points.len() > 40);
    let mark = Mark::build(&curve, &Form::default()).unwrap();
    closed(&mark.solids[0].mesh).unwrap();
}

#[test]
fn refuses_what_it_cannot_build() {
    assert!(Mark::build("<!DOCTYPE svg><svg/>", &Form::default()).is_err());
    assert!(
        Mark::build(
            &svg(r##"<rect width="10" height="10" fill="url(#missing)"/>"##),
            &Form::default()
        )
        .is_err()
    );
    let flat = Form {
        depth: 0.0,
        bevel: 0.0,
        ..Form::default()
    };
    assert!(Mark::build(SAMPLE, &flat).is_err());
    assert!(
        Icon::new(vec![Part::new(PartShape::Ball {
            at: [0.0, 0.0],
            z: 0.0,
            r: 0.0
        })])
        .build()
        .is_err()
    );
}

fn sphere_error(mesh: &Mesh, center: [f32; 3], r: f32) -> f32 {
    mesh.positions
        .iter()
        .map(|p| (dot(sub(*p, center), sub(*p, center)).sqrt() - r).abs())
        .fold(0.0, f32::max)
}

#[test]
fn the_field_mesher_puts_its_vertices_on_the_surface() {
    let (major, minor) = (0.5f32, 0.15f32);
    let torus = |p: [f32; 3]| {
        let ring = (p[0] * p[0] + p[1] * p[1]).sqrt() - major;
        (ring * ring + p[2] * p[2]).sqrt() - minor
    };
    let voxel = 0.01;
    let mesh = Surface::new(voxel, 8).mesh(&torus, [-0.65, -0.65, -0.15], [0.65, 0.65, 0.15]);
    closed(&mesh).unwrap();
    unit_normals(&mesh);
    assert_eq!(facing(&mesh), 1.0);
    for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
        assert!(
            torus(*p).abs() < voxel * 0.02,
            "vertex off the surface by {}",
            torus(*p)
        );
        let g = gradient(&torus, *p, 1e-4);
        let len = dot(g, g).sqrt();
        assert!(dot(*n, g) / len > 0.9995);
    }
    let expected = 2.0 * PI * PI * major as f64 * (minor as f64).powi(2);
    let got = volume(&mesh);
    assert!(
        (got - expected).abs() / expected < 2e-3,
        "torus volume {got} against {expected}"
    );
    let shadow = silhouette(&mesh, [-0.7, -0.7], [0.7, 0.7], 500);
    let annulus = PI * ((major + minor) as f64).powi(2) - PI * ((major - minor) as f64).powi(2);
    assert!(
        (shadow - annulus).abs() / annulus < 5e-3,
        "torus silhouette {shadow} against {annulus}"
    );
}

#[test]
fn fused_balls_make_the_union_of_two_spheres() {
    let r = 0.3f32;
    let mut icon = Icon::new(vec![
        Part::new(PartShape::Ball {
            at: [-0.15, 0.0],
            z: 0.0,
            r,
        }),
        Part::new(PartShape::Ball {
            at: [0.15, 0.0],
            z: 0.0,
            r,
        }),
    ]);
    icon.blend = Some(0.0);
    let pieces = icon.build().unwrap();
    assert_eq!(pieces.len(), 1);
    let mesh = &pieces[0].mesh;
    closed(mesh).unwrap();
    assert!(facing(mesh) > 0.999);
    let (r, d) = (r as f64, 0.3);
    let lens = PI * (4.0 * r + d) * (2.0 * r - d).powi(2) / 12.0;
    let expected = 2.0 * 4.0 / 3.0 * PI * r.powi(3) - lens;
    let got = volume(mesh);
    assert!(
        (got - expected).abs() / expected < 3e-3,
        "union volume {got} against {expected}"
    );
    let blended = Icon::new(icon.parts.clone()).build().unwrap();
    assert!(
        volume(&blended[0].mesh) > got,
        "a blend only adds to the crease"
    );
}

#[test]
fn a_capsule_alone_is_fused_like_render_py_and_matches_its_volume() {
    let icon = Icon::new(vec![Part::new(PartShape::capsule([-0.4, 0.0], [0.4, 0.0]))]);
    let pieces = icon.build().unwrap();
    assert_eq!(pieces.len(), 1);
    let mesh = &pieces[0].mesh;
    closed(mesh).unwrap();
    let r = 0.08f64;
    let expected = PI * r * r * 0.8 + 4.0 / 3.0 * PI * r.powi(3);
    let got = volume(mesh);
    assert!(
        (got - expected).abs() / expected < 0.01,
        "capsule volume {got} against {expected}"
    );
}

#[test]
fn a_rounded_box_follows_steiner() {
    let shape = PartShape::Box {
        at: [0.1, -0.1],
        z: 0.05,
        half: [0.3, 0.2, 0.1],
        round: 0.05,
        angle: 30.0,
    };
    let field = |p: [f32; 3]| shape.distance(p);
    let (lo, hi) = shape.bounds();
    let mesh = Surface::new(0.008, 8).mesh(&field, lo, hi);
    closed(&mesh).unwrap();
    let rho = 0.05f64;
    let (a, b, c) = (0.3 - rho, 0.2 - rho, 0.1 - rho);
    let expected = 8.0 * a * b * c
        + 8.0 * (a * b + b * c + c * a) * rho
        + 2.0 * PI * (a + b + c) * rho * rho
        + 4.0 / 3.0 * PI * rho.powi(3);
    let got = volume(&mesh);
    assert!(
        (got - expected).abs() / expected < 5e-3,
        "box volume {got} against {expected}"
    );
    let analytic = shape.mesh(0.001);
    let got = volume(&analytic);
    assert!(
        (got - expected).abs() / expected < 5e-3,
        "analytic box volume {got} against {expected}"
    );
}

#[test]
fn unfused_parts_stay_separate_analytic_meshes() {
    let mut icon = Icon::new(vec![
        Part::new(PartShape::Ball {
            at: [0.0, 0.0],
            z: 0.0,
            r: 0.4,
        }),
        Part::new(PartShape::torus()).with_material("rim"),
        Part::new(PartShape::Drop {
            at: [0.0, 0.0],
            z: 0.0,
            r: 0.5,
            squash: 0.45,
            stretch: 1.5,
        })
        .with_material("rim"),
    ]);
    icon.fuse = false;
    let pieces = icon.build().unwrap();
    assert_eq!(pieces.len(), 3);
    assert_eq!(pieces[0].material, None);
    assert_eq!(pieces[1].material.as_deref(), Some("rim"));
    assert!(sphere_error(&pieces[0].mesh, [0.0; 3], 0.4) < 1e-5);
    let drop = &pieces[2].mesh;
    let b = drop.bounds;
    for (got, want) in [(b.max[0], 0.5), (b.max[1], 0.75), (b.max[2], 0.225)] {
        assert!(
            got <= want + 1e-5 && got > want - 2e-3,
            "drop bound {got} against {want}"
        );
    }
    let expected = 4.0 / 3.0 * PI * 0.5 * 0.75 * 0.225;
    let got = volume(drop);
    assert!(
        (got - expected).abs() / expected < 5e-3,
        "drop volume {got} against {expected}"
    );
    for piece in &pieces {
        closed(&piece.mesh).unwrap();
        assert!(facing(&piece.mesh) > 0.999);
    }
}

#[test]
fn a_large_voxel_gives_the_blocky_look_and_the_cross_limit_holds() {
    let parts = vec![
        Part::new(PartShape::Ball {
            at: [0.0, 0.0],
            z: 0.0,
            r: 0.4,
        }),
        Part::new(PartShape::capsule([0.0, 0.0], [0.6, 0.3])),
    ];
    let mut icon = Icon::new(parts.clone());
    icon.voxel = 0.1;
    let blocky = icon.build().unwrap();
    let fine = Icon::new(parts.clone()).build().unwrap();
    assert!(blocky[0].mesh.triangle_count() * 50 < fine[0].mesh.triangle_count());
    closed(&blocky[0].mesh).unwrap();
    let mut tiny = Icon::new(parts);
    tiny.voxel = 1e-6;
    let shapes = tiny.parts.iter().map(|p| &p.shape).collect::<Vec<_>>();
    assert!((tiny.voxel_for(&shapes) - 1.08 / 900.0).abs() < 1e-6);
}

#[test]
fn the_sample_icons_count_their_triangles() {
    let dot = Icon::new(vec![Part::new(PartShape::Ball {
        at: [0.0, 0.0],
        z: 0.0,
        r: 0.4,
    })]);
    let ring = Icon::new(vec![Part::new(PartShape::Torus {
        at: [0.0, 0.0],
        z: 0.0,
        major: 0.5,
        minor: 0.1,
    })]);
    let pair = Icon::new(vec![
        Part::new(PartShape::Ball {
            at: [-0.2, 0.0],
            z: 0.0,
            r: 0.3,
        }),
        Part::new(PartShape::Ball {
            at: [0.2, 0.0],
            z: 0.0,
            r: 0.3,
        }),
    ]);
    let counts = [dot, ring, pair].map(|icon| {
        icon.build()
            .unwrap()
            .iter()
            .map(|p| p.mesh.triangle_count())
            .sum::<usize>()
    });
    assert_eq!(counts, [3_906, 4_928, 55_996]);
}

#[test]
fn the_same_input_gives_the_same_bytes() {
    let stroked = format!(
        "{}{}",
        SAMPLE.trim_end_matches("</svg>"),
        r##"<path d="M10 54 Q32 40 54 54" fill="none" stroke="#202020" stroke-width="3"/></svg>"##
    );
    let once = Mark::build(&stroked, &Form::default()).unwrap();
    let twice = Mark::build(&stroked, &Form::default()).unwrap();
    assert_eq!(once.solids.len(), twice.solids.len());
    for (a, b) in once.solids.iter().zip(&twice.solids) {
        assert_eq!(a.mesh.bytes(), b.mesh.bytes());
    }
    let icon = Icon::new(vec![
        Part::new(PartShape::Ball {
            at: [0.0, 0.0],
            z: 0.0,
            r: 0.3,
        }),
        Part::new(PartShape::tube(vec![[0.0, 0.0], [0.4, 0.2], [0.6, -0.2]])),
        Part::new(PartShape::cube()),
    ]);
    let a = icon.build().unwrap();
    let b = std::thread::spawn(move || icon.build().unwrap())
        .join()
        .unwrap();
    assert_eq!(a[0].mesh.bytes(), b[0].mesh.bytes());
}
