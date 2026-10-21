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
        input.weights.len() * std::mem::size_of::<f32>(),
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
            let p = [u, v, (u * 7.0).sin() * (v * 5.0).cos() * 0.1];
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

fn meshes() -> Vec<Mesh> {
    let mut out = vec![
        Shape::Ellipsoid {
            radius: 1.0,
            squash: 1.0,
        }
        .mesh(0.0005),
        sheet(40),
    ];
    for error in [0.01f32, 0.002] {
        out.extend(crate::tests::shapes().iter().map(|s| s.mesh(error)));
    }
    out
}

fn assert_same(input: &Input, target: usize, target_error: f32, options: Options, what: &str) {
    let ours = simplify(input, target, target_error, options);
    let theirs = reference(input, target, target_error, options);
    assert_eq!(ours.indices, theirs.indices, "{what}: indices");
    assert_eq!(
        ours.error.to_bits(),
        theirs.error.to_bits(),
        "{what}: error {} against {}",
        ours.error,
        theirs.error
    );
}

#[test]
fn the_port_gives_meshoptimizers_indices_and_error_bit_for_bit() {
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
    for (m, mesh) in meshes().iter().enumerate() {
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
                    assert_same(&input, target * 3, f32::MAX, options, &what);
                    target /= 2;
                }
                let what = format!("mesh {m}, input {w}, options {o}, error limit");
                assert_same(&input, 0, 0.01, options, &what);
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
        assert_same(
            &bare,
            target,
            f32::MAX,
            variants[0],
            &format!("mesh {m}, bare"),
        );
    }
}

#[test]
fn the_port_is_deterministic() {
    let mesh = sheet(40);
    let attributes = attributes(&mesh);
    let input = Input {
        indices: &mesh.indices,
        positions: &mesh.positions,
        attributes: &attributes,
        weights: &WEIGHTS,
        locks: &[],
    };
    let options = Options {
        lock_border: false,
        error_absolute: true,
    };
    let first = simplify(&input, mesh.indices.len() / 6, f32::MAX, options);
    assert!(first.indices.len() < mesh.indices.len() / 2);
    for _ in 0..3 {
        assert_eq!(
            simplify(&input, mesh.indices.len() / 6, f32::MAX, options),
            first
        );
    }
}
