use crate::lod::{NORMAL_WEIGHT, UV_WEIGHT};
use crate::mesh::Mesh;
use crate::shapes::Shape;
use crate::simplify::{Input, Options, Simplified, simplify};
use meshopt::{SimplifyOptions, VertexDataAdapter, simplify_with_attributes_and_locks};

const WEIGHTS: [f32; 5] = [
    NORMAL_WEIGHT,
    NORMAL_WEIGHT,
    NORMAL_WEIGHT,
    UV_WEIGHT,
    UV_WEIGHT,
];

const FUSED_TRIANGLES: f64 = 0.01;
const FUSED_ERROR: f64 = 1.25;

fn reference(input: &Input, target: usize, target_error: f32, options: Options) -> Simplified {
    let bytes: Vec<u8> = input
        .positions
        .iter()
        .flat_map(|p| p.iter().flat_map(|v| v.to_le_bytes()))
        .collect();
    let adapter = VertexDataAdapter::new(&bytes, 12, 0).unwrap();
    let locks = if input.locks.is_empty() {
        vec![false; input.positions.len()]
    } else {
        input.locks.to_vec()
    };
    let mut flags = SimplifyOptions::None;
    if options.lock_border {
        flags |= SimplifyOptions::LockBorder;
    }
    if options.error_absolute {
        flags |= SimplifyOptions::ErrorAbsolute;
    }
    let mut error = 0.0f32;
    let indices = simplify_with_attributes_and_locks(
        input.indices,
        &adapter,
        input.attributes,
        input.weights,
        std::mem::size_of_val(input.weights),
        &locks,
        target,
        target_error,
        flags,
        Some(&mut error),
    );
    Simplified { indices, error }
}

fn attributes(mesh: &Mesh) -> Vec<f32> {
    mesh.normals
        .iter()
        .zip(mesh.uvs.iter())
        .flat_map(|(n, uv)| [n[0], n[1], n[2], uv[0], uv[1]])
        .collect()
}

fn sheet(cells: usize) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let seam = cells / 2;
    let mut ids = vec![[0u32; 2]; (cells + 1) * (cells + 1)];
    for y in 0..=cells {
        for x in 0..=cells {
            let (u, v) = (x as f32 / cells as f32, y as f32 / cells as f32);
            let z = (u - 0.5) * (v - 0.3) * (u * u - v) * 0.4 + u * v * v * 0.05;
            let p = [u, v, z];
            let id = positions.len() as u32;
            positions.push(p);
            normals.push([0.0, 0.0, 1.0]);
            uvs.push([u, v]);
            ids[y * (cells + 1) + x] = [id, id];
            if x == seam {
                positions.push(p);
                normals.push([0.0, 0.0, 1.0]);
                uvs.push([u + 0.25, v]);
                ids[y * (cells + 1) + x][1] = id + 1;
            }
        }
    }
    let mut indices = Vec::new();
    for y in 0..cells {
        for x in 0..cells {
            let side = usize::from(x >= seam);
            let at = |x: usize, y: usize| ids[y * (cells + 1) + x][side];
            let (a, b, c, d) = (at(x, y), at(x + 1, y), at(x + 1, y + 1), at(x, y + 1));
            indices.extend([a, b, c, a, c, d]);
        }
    }
    let tangents = vec![[1.0, 0.0, 0.0, 1.0]; positions.len()];
    Mesh::new(positions, normals, tangents, uvs, indices)
}

fn ball(cells: usize) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for axis in 0..3 {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        for sign in [1.0f32, -1.0] {
            let base = positions.len() as u32;
            for j in 0..=cells {
                for i in 0..=cells {
                    let (s, t) = (i as f32 / cells as f32, j as f32 / cells as f32);
                    let mut c = [0.0f32; 3];
                    c[axis] = sign;
                    c[u] = s * 2.0 - 1.0;
                    c[v] = t * 2.0 - 1.0;
                    let length = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
                    let p = [c[0] / length, c[1] / length, c[2] / length];
                    positions.push(p);
                    normals.push(p);
                    uvs.push([s, t]);
                }
            }
            let at = |i: usize, j: usize| base + (j * (cells + 1) + i) as u32;
            for j in 0..cells {
                for i in 0..cells {
                    let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
                    if sign > 0.0 {
                        indices.extend([a, b, c, a, c, d]);
                    } else {
                        indices.extend([a, c, b, a, d, c]);
                    }
                }
            }
        }
    }
    let tangents = vec![[1.0, 0.0, 0.0, 1.0]; positions.len()];
    Mesh::new(positions, normals, tangents, uvs, indices)
}

fn plain_meshes() -> Vec<Mesh> {
    vec![ball(16), sheet(40)]
}

fn meshes() -> Vec<Mesh> {
    let mut out = plain_meshes();
    out.push(
        Shape::Ellipsoid {
            radius: 1.0,
            squash: 1.0,
        }
        .mesh(0.0005),
    );
    for error in [0.01f32, 0.002] {
        out.extend(crate::tests::shapes().iter().map(|s| s.mesh(error)));
    }
    out
}

fn each_case(mesh: &Mesh, m: usize, run: &mut impl FnMut(&Input, usize, f32, Options, &str)) {
    let variants = [
        Options {
            lock_border: true,
            error_absolute: true,
        },
        Options {
            lock_border: false,
            error_absolute: true,
        },
        Options {
            lock_border: false,
            error_absolute: false,
        },
    ];
    let attributes = attributes(mesh);
    let locks: Vec<bool> = (0..mesh.positions.len()).map(|i| i % 7 == 3).collect();
    let half = [NORMAL_WEIGHT, 0.0, NORMAL_WEIGHT, UV_WEIGHT, 0.0];
    let inputs = [
        (&WEIGHTS, &[][..]),
        (&WEIGHTS, &locks[..]),
        (&half, &[][..]),
    ];
    let dense = mesh.triangle_count() > 4000;
    for (w, (weights, locks)) in inputs
        .into_iter()
        .enumerate()
        .take(if dense { 1 } else { 3 })
    {
        let input = Input {
            indices: &mesh.indices,
            positions: &mesh.positions,
            attributes: &attributes,
            weights,
            locks,
        };
        for (o, &options) in variants.iter().enumerate().take(if dense { 1 } else { 3 }) {
            let mut target = mesh.triangle_count() / 2;
            while target >= 8 {
                let what = format!("mesh {m}, input {w}, options {o}, target {target}");
                run(&input, target * 3, f32::MAX, options, &what);
                target /= 2;
            }
            let what = format!("mesh {m}, input {w}, options {o}, error limit");
            run(&input, 0, 0.01, options, &what);
        }
    }
    let bare = Input {
        indices: &mesh.indices,
        positions: &mesh.positions,
        attributes: &[],
        weights: &[],
        locks: &[],
    };
    let target = mesh.triangle_count() / 4 * 3;
    run(
        &bare,
        target,
        f32::MAX,
        variants[0],
        &format!("mesh {m}, bare"),
    );
}

fn assert_same(
    input: &Input,
    target: usize,
    target_error: f32,
    options: Options,
    what: &str,
    tally: &mut Tally,
) {
    let ours = simplify(input, target, target_error, options);
    let theirs = reference(input, target, target_error, options);
    if cfg!(target_arch = "x86_64") {
        assert_eq!(ours.indices, theirs.indices, "{what}: indices");
        assert_eq!(
            ours.error.to_bits(),
            theirs.error.to_bits(),
            "{what}: error {} against {}",
            ours.error,
            theirs.error
        );
    }
    tally.ours += ours.indices.len() as f64;
    tally.theirs += theirs.indices.len() as f64;
    tally.log_error += ((f64::from(ours.error) + 1e-6) / (f64::from(theirs.error) + 1e-6)).ln();
    tally.runs += 1;
}

#[derive(Default)]
struct Tally {
    ours: f64,
    theirs: f64,
    log_error: f64,
    runs: u32,
}

impl Tally {
    fn check(&self, what: &str) {
        let triangles = self.ours / self.theirs;
        let error = (self.log_error / f64::from(self.runs)).exp();
        assert!(
            (triangles - 1.0).abs() <= FUSED_TRIANGLES,
            "{what}: {triangles} times the triangles"
        );
        assert!(
            (1.0 / FUSED_ERROR..=FUSED_ERROR).contains(&error),
            "{what}: {error} times the error"
        );
    }
}

#[test]
fn the_port_gives_meshoptimizers_indices_and_error() {
    for (m, mesh) in meshes().iter().enumerate() {
        let mut tally = Tally::default();
        each_case(mesh, m, &mut |input, target, error, options, what| {
            assert_same(input, target, error, options, what, &mut tally)
        });
        tally.check(&format!("mesh {m}"));
    }
}

#[test]
fn the_port_gives_the_same_chains_on_every_platform() {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |value: u32| {
        for b in value.to_le_bytes() {
            hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    let mut runs = 0;
    for (m, mesh) in plain_meshes().iter().enumerate() {
        each_case(mesh, m, &mut |input, target, error, options, what| {
            let out = simplify(input, target, error, options);
            assert!(out.indices.len() < input.indices.len(), "{what}");
            eat(out.indices.len() as u32);
            for &i in &out.indices {
                eat(i);
            }
            eat(out.error.to_bits());
            runs += 1;
        });
    }
    assert_eq!(runs, 164);
    assert_eq!(hash, 0x42a4_b9f7_f69c_7c6a, "{hash:#018x}");
}

#[test]
fn input_it_cannot_use_comes_back_unchanged() {
    let mesh = sheet(4);
    let attributes = attributes(&mesh);
    let n = mesh.positions.len();
    let mut out_of_range = mesh.indices.clone();
    out_of_range[4] = n as u32;
    let ragged = &mesh.indices[..mesh.indices.len() - 1];
    let short_locks = vec![false; n - 1];
    let cases = [
        (&out_of_range[..], &attributes[..], &[][..]),
        (ragged, &attributes[..], &[][..]),
        (
            &mesh.indices[..],
            &attributes[..attributes.len() - 1],
            &[][..],
        ),
        (&mesh.indices[..], &attributes[..], &short_locks[..]),
    ];
    for (case, (indices, attributes, locks)) in cases.into_iter().enumerate() {
        let input = Input {
            indices,
            positions: &mesh.positions,
            attributes,
            weights: &WEIGHTS,
            locks,
        };
        let out = simplify(&input, 0, f32::MAX, Options::default());
        assert_eq!(out.indices, indices, "case {case}");
        assert_eq!(out.error, 0.0, "case {case}");
    }
}
