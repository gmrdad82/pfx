use std::f32::consts::{PI, TAU};

use pfx_geom::mesh::Mesh;
use pfx_live::frame::{self, Matrix};
use pfx_materials::{
    Ageing, Blend, CoatWobble, Content, ContentLayer, Crinkle, Family, Fibre, Grime, Material,
    NoiseKind, NoiseLayer, Normal, NormalSource, PlankWood, Scratch, WallMottle,
};

pub const LEFT: f32 = -3.0;
pub const RIGHT: f32 = 3.0;
pub const FLOOR: f32 = 0.0;
pub const CEILING: f32 = 2.8;
pub const BACK: f32 = -2.2;
pub const FRONT: f32 = 3.2;
pub const WALL: f32 = 0.12;
pub const WINDOW_Z: [f32; 2] = [-1.7, 0.1];
pub const WINDOW_Y: [f32; 2] = [0.95, 2.25];
pub const TABLE_TOP: f32 = 0.76;
pub const TABLE_CENTRE: [f32; 2] = [-0.4, -0.85];
pub const TABLE_HALF: [f32; 2] = [1.8, 0.55];
pub const SHELF_Y: [f32; 3] = [1.15, 1.6, 2.05];
pub const SHELF_X: [f32; 2] = [-1.0, 2.6];
pub const SHELF_DEPTH: f32 = 0.28;
pub const LIP: f32 = 0.018;
pub const SLATS: usize = 10;
pub const TREE_ORIGIN: [f32; 3] = [-6.2, FLOOR, -0.9];
pub const BIRDS: [[f32; 3]; 3] = [[-4.0, 2.4, -0.8], [-4.3, 2.55, -1.05], [-3.8, 2.3, -0.45]];
pub const VESSEL: [f32; 3] = [0.3, TABLE_TOP, -1.2];
pub const LENS: [f32; 3] = [-1.35, TABLE_TOP + 0.06, -0.55];
pub const LENS_RADIUS: f32 = 0.06;
pub const SHEET: [f32; 3] = [-0.75, TABLE_TOP + 0.002, -0.85];
pub const SHEET_SIZE: [f32; 2] = [0.5, 0.36];

pub const CUBE: usize = 0;
pub const BALL: usize = 1;
pub const CAN: usize = 2;
pub const PAGE: usize = 3;
pub const SCREEN: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cast {
    Casts,
    Never,
    Only,
}

#[derive(Clone, Debug)]
pub struct Geometry {
    pub mesh: Mesh,
    pub uvs1: Option<Vec<[f32; 2]>>,
    pub alpha: Option<Vec<f32>>,
}

#[derive(Clone, Debug)]
pub struct Part {
    pub name: String,
    pub mesh: usize,
    pub model: Matrix,
    pub material: usize,
    pub cast: Cast,
    pub age: f32,
    pub cutout: bool,
}

pub struct Room {
    pub meshes: Vec<(&'static str, Geometry)>,
    pub parts: Vec<Part>,
    pub materials: Vec<(&'static str, Material)>,
}

pub fn identity() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

pub fn translate(at: [f32; 3]) -> Matrix {
    let mut m = identity();
    m[3] = [at[0], at[1], at[2], 1.0];
    m
}

pub fn scale(size: [f32; 3]) -> Matrix {
    let mut m = identity();
    m[0][0] = size[0];
    m[1][1] = size[1];
    m[2][2] = size[2];
    m
}

pub fn turn_x(angle: f32) -> Matrix {
    let (s, c) = angle.sin_cos();
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, c, s, 0.0],
        [0.0, -s, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

pub fn turn_y(angle: f32) -> Matrix {
    let (s, c) = angle.sin_cos();
    [
        [c, 0.0, -s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [s, 0.0, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

pub fn turn_z(angle: f32) -> Matrix {
    let (s, c) = angle.sin_cos();
    [
        [c, s, 0.0, 0.0],
        [-s, c, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

pub fn place(at: [f32; 3], turn: Matrix, size: [f32; 3]) -> Matrix {
    frame::multiply(translate(at), frame::multiply(turn, scale(size)))
}

pub fn block(lo: [f32; 3], hi: [f32; 3]) -> Matrix {
    place(
        std::array::from_fn(|k| (lo[k] + hi[k]) * 0.5),
        identity(),
        std::array::from_fn(|k| hi[k] - lo[k]),
    )
}

pub fn hash(mut value: u32) -> f32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    value as f32 / u32::MAX as f32
}

fn geometry(mesh: Mesh) -> Geometry {
    Geometry {
        mesh,
        uvs1: None,
        alpha: None,
    }
}

pub fn cube() -> Mesh {
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for (n, u, v) in faces {
        let base = positions.len() as u32;
        for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            positions.push(std::array::from_fn(|k| 0.5 * (n[k] + u[k] * a + v[k] * b)));
            normals.push(n);
            uvs.push([(a + 1.0) * 0.5, (1.0 - b) * 0.5]);
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(positions, normals, Vec::new(), uvs, indices)
}

pub fn ball(rings: u32, segments: u32) -> Mesh {
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    for ring in 0..=rings {
        let theta = PI * ring as f32 / rings as f32;
        for segment in 0..=segments {
            let phi = TAU * segment as f32 / segments as f32;
            positions.push([
                0.5 * theta.sin() * phi.cos(),
                0.5 * theta.cos(),
                -0.5 * theta.sin() * phi.sin(),
            ]);
            uvs.push([segment as f32 / segments as f32, ring as f32 / rings as f32]);
        }
    }
    let normals = positions.iter().map(|p| p.map(|v| v * 2.0)).collect();
    let row = segments + 1;
    let mut indices = Vec::new();
    for ring in 0..rings {
        for segment in 0..segments {
            let a = ring * row + segment;
            let b = a + row;
            if ring > 0 {
                indices.extend([a, b, a + 1]);
            }
            if ring + 1 < rings {
                indices.extend([a + 1, b, b + 1]);
            }
        }
    }
    Mesh::new(positions, normals, Vec::new(), uvs, indices)
}

pub fn can(segments: u32) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for segment in 0..=segments {
        let phi = TAU * segment as f32 / segments as f32;
        let (s, c) = phi.sin_cos();
        for y in [-0.5, 0.5] {
            positions.push([0.5 * c, y, -0.5 * s]);
            normals.push([c, 0.0, -s]);
            uvs.push([segment as f32 / segments as f32, 0.5 - y]);
        }
    }
    for segment in 0..segments {
        let a = segment * 2;
        indices.extend([a, a + 2, a + 1, a + 1, a + 2, a + 3]);
    }
    for (y, sign) in [(0.5f32, 1.0f32), (-0.5, -1.0)] {
        let centre = positions.len() as u32;
        positions.push([0.0, y, 0.0]);
        normals.push([0.0, sign, 0.0]);
        uvs.push([0.5, 0.5]);
        for segment in 0..=segments {
            let phi = TAU * segment as f32 / segments as f32;
            let (s, c) = phi.sin_cos();
            positions.push([0.5 * c, y, -0.5 * s]);
            normals.push([0.0, sign, 0.0]);
            uvs.push([0.5 + 0.5 * c, 0.5 + 0.5 * s]);
        }
        for segment in 0..segments {
            let a = centre + 1 + segment;
            if sign > 0.0 {
                indices.extend([centre, a, a + 1]);
            } else {
                indices.extend([centre, a + 1, a]);
            }
        }
    }
    Mesh::new(positions, normals, Vec::new(), uvs, indices)
}

pub fn grid(columns: u32, rows: u32) -> Mesh {
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    for row in 0..=rows {
        for column in 0..=columns {
            let u = column as f32 / columns as f32;
            let v = row as f32 / rows as f32;
            positions.push([u - 0.5, 0.0, v - 0.5]);
            uvs.push([u, v]);
        }
    }
    let mut indices = Vec::new();
    let stride = columns + 1;
    for row in 0..rows {
        for column in 0..columns {
            let a = row * stride + column;
            indices.extend([a, a + stride, a + 1, a + 1, a + stride, a + stride + 1]);
        }
    }
    let count = positions.len();
    Mesh::new(
        positions,
        vec![[0.0, 1.0, 0.0]; count],
        Vec::new(),
        uvs,
        indices,
    )
}

fn layers(list: &[(NoiseKind, f32, f32, u32)]) -> [NoiseLayer; 4] {
    std::array::from_fn(|i| {
        list.get(i).map_or_else(
            NoiseLayer::default,
            |&(kind, frequency, amplitude, seed)| NoiseLayer::new(kind, frequency, amplitude, seed),
        )
    })
}

fn stack(list: &[NoiseLayer]) -> [NoiseLayer; 4] {
    std::array::from_fn(|i| list.get(i).copied().unwrap_or_default())
}

pub fn library() -> Vec<(&'static str, Material)> {
    let plain = Material::default();
    vec![
        (
            "wall",
            Material {
                family: Family::Plaster,
                base: [0.71, 0.69, 0.65],
                roughness: 0.92,
                layers: stack(&[
                    WallMottle::SAMPLE.layer(0.35, 5),
                    NoiseLayer::new(NoiseKind::Fbm, 7.0, 0.06, 6),
                ]),
                ..plain
            },
        ),
        (
            "ceiling",
            Material {
                family: Family::Plaster,
                base: [0.8, 0.79, 0.76],
                roughness: 0.95,
                ..plain
            },
        ),
        (
            "floor",
            Material {
                family: Family::Wood,
                base: [0.34, 0.23, 0.15],
                roughness: 0.48,
                clearcoat: 0.25,
                clearcoat_roughness: 0.18,
                layers: stack(&[
                    PlankWood::SAMPLE.layer(1.0, 3),
                    Grime::SAMPLE.layer(0.45, 4),
                ]),
                ..plain
            },
        ),
        (
            "table",
            Material {
                family: Family::Wood,
                base: [0.45, 0.31, 0.19],
                roughness: 0.4,
                clearcoat: 0.5,
                clearcoat_roughness: 0.08,
                layers: stack(&[
                    PlankWood::SAMPLE.layer(0.8, 7),
                    Scratch::SAMPLE.layer(0.6, 8),
                ]),
                ..plain
            },
        ),
        (
            "lacquer",
            Material {
                family: Family::Lacquer,
                base: [0.5, 0.05, 0.035],
                roughness: 0.3,
                clearcoat: 1.0,
                clearcoat_roughness: 0.03,
                normal: Normal {
                    source: NormalSource::Bump,
                    strength: 0.6,
                },
                ..plain
            },
        ),
        (
            "lacquer_black",
            Material {
                family: Family::Lacquer,
                base: [0.02, 0.02, 0.025],
                roughness: 0.3,
                clearcoat: 1.0,
                clearcoat_roughness: 0.05,
                layers: stack(&[CoatWobble::SAMPLE.layer(0.5, 21)]),
                ..plain
            },
        ),
        (
            "sheet",
            Material {
                family: Family::Plain,
                base: [0.88, 0.85, 0.78],
                roughness: 0.85,
                specular: 0.35,
                subsurface: 0.2,
                subsurface_tint: [0.95, 0.85, 0.7],
                layers: stack(&[Fibre::SAMPLE.layer(1.0, 11)]),
                ageing: Ageing {
                    fade: 0.9,
                    yellow: 1.0,
                    ink: 0.0,
                    scratch: 0.6,
                    edge: 0.5,
                    dust: 1.0,
                    patina: 0.0,
                    seed: 119,
                },
                ..plain
            },
        ),
        (
            "crumpled",
            Material {
                family: Family::Plain,
                base: [0.84, 0.83, 0.8],
                roughness: 0.8,
                layers: stack(&[Crinkle::SAMPLE.layer(1.0, 12), Fibre::SAMPLE.layer(1.0, 13)]),
                ..plain
            },
        ),
        (
            "felt",
            Material {
                family: Family::Cloth,
                base: [0.16, 0.24, 0.34],
                roughness: 0.95,
                sheen: 0.6,
                layers: layers(&[(NoiseKind::Fbm, 60.0, 0.12, 14)]),
                ..plain
            },
        ),
        (
            "slate",
            Material {
                family: Family::Stone,
                base: [0.24, 0.25, 0.26],
                roughness: 0.7,
                layers: layers(&[
                    (NoiseKind::Value, 30.0, 0.2, 15),
                    (NoiseKind::Fbm, 9.0, 0.1, 16),
                ]),
                ..plain
            },
        ),
        (
            "brass",
            Material {
                family: Family::Metal,
                base: [0.78, 0.58, 0.3],
                roughness: 0.26,
                metalness: 1.0,
                ageing: Ageing {
                    patina: 0.7,
                    seed: 31,
                    ..Ageing::default()
                },
                ..plain
            },
        ),
        (
            "steel",
            Material {
                family: Family::Metal,
                base: [0.58, 0.58, 0.6],
                roughness: 0.38,
                metalness: 1.0,
                layers: stack(&[Scratch::SAMPLE.layer(0.8, 17)]),
                ..plain
            },
        ),
        (
            "chrome",
            Material {
                family: Family::Chrome,
                base: [0.92, 0.92, 0.93],
                roughness: 0.04,
                metalness: 1.0,
                ..plain
            },
        ),
        (
            "pages",
            Material {
                family: Family::Plain,
                base: [0.86, 0.82, 0.74],
                roughness: 0.9,
                layers: layers(&[(NoiseKind::Value, 300.0, 0.1, 18)]),
                ..plain
            },
        ),
        (
            "cover",
            Material {
                family: Family::Cloth,
                base: [0.12, 0.2, 0.14],
                roughness: 0.8,
                sheen: 0.3,
                layers: layers(&[(NoiseKind::Value, 40.0, 0.15, 19)]),
                ..plain
            },
        ),
        (
            "occluder",
            Material {
                family: Family::Occluder,
                base: [0.05, 0.05, 0.05],
                roughness: 1.0,
                ..plain
            },
        ),
        (
            "petal",
            Material {
                family: Family::Petal,
                base: [0.95, 0.74, 0.8],
                roughness: 0.6,
                subsurface: 0.45,
                subsurface_tint: [1.0, 0.8, 0.85],
                ..plain
            },
        ),
        (
            "leaf",
            Material {
                family: Family::Leaf,
                base: [0.2, 0.38, 0.12],
                roughness: 0.55,
                subsurface: 0.3,
                subsurface_tint: [0.6, 0.9, 0.3],
                layers: layers(&[(NoiseKind::Leaf, 1.0, 1.0, 22)]),
                ..plain
            },
        ),
        (
            "bark",
            Material {
                family: Family::Bark,
                base: [0.24, 0.17, 0.13],
                roughness: 0.92,
                layers: layers(&[(NoiseKind::Bark, 1.0, 1.0, 9)]),
                ..plain
            },
        ),
        (
            "cord",
            Material {
                family: Family::Cord,
                base: [0.62, 0.5, 0.34],
                roughness: 0.85,
                layers: stack(&[Fibre::SAMPLE.layer(0.8, 23)]),
                ..plain
            },
        ),
        (
            "frost",
            Material {
                family: Family::Plain,
                base: [0.95, 0.95, 0.96],
                roughness: 0.3,
                specular: 0.05,
                fresnel_power: 4.0,
                ..plain
            },
        ),
        (
            "lamp",
            Material {
                family: Family::Emissive,
                base: [0.9, 0.85, 0.75],
                emission: [3.2, 2.7, 2.0],
                ..plain
            },
        ),
        (
            "plain",
            Material {
                family: Family::Plain,
                base: [0.5, 0.5, 0.5],
                roughness: 0.6,
                ..plain
            },
        ),
        (
            "anodised",
            Material {
                family: Family::Metal,
                base: [0.56, 0.57, 0.58],
                roughness: 0.2,
                metalness: 1.0,
                thin_film: 160.0,
                thin_film_ior: 2.4,
                thin_film_amount: 1.0,
                ..plain
            },
        ),
        (
            "cards",
            Material {
                family: Family::Plain,
                base: [0.9, 0.89, 0.86],
                roughness: 0.75,
                ..plain
            },
        ),
        (
            "decal",
            Material {
                family: Family::Lacquer,
                base: [0.75, 0.73, 0.68],
                roughness: 0.35,
                clearcoat: 0.6,
                clearcoat_roughness: 0.06,
                ..plain
            },
        ),
        (
            "photo",
            Material {
                family: Family::Plain,
                base: [0.92, 0.91, 0.88],
                roughness: 0.4,
                ..plain
            },
        ),
        (
            "screen",
            Material {
                family: Family::Plain,
                base: [0.015, 0.015, 0.02],
                roughness: 0.15,
                ..plain
            },
        ),
        (
            "ink",
            Material {
                family: Family::Plain,
                base: [0.9, 0.88, 0.82],
                roughness: 0.85,
                subsurface: 0.15,
                layers: stack(&[Fibre::SAMPLE.layer(0.7, 24)]),
                ..plain
            },
        ),
    ]
}

pub fn content(name: &str) -> Option<(Content, ContentLayer)> {
    let layer = |slot: i32| ContentLayer {
        slot,
        ..ContentLayer::default()
    };
    match name {
        "cards" => Some((Content::Print, layer(0))),
        "decal" => Some((Content::Decal, layer(0))),
        "photo" => Some((
            Content::Photo,
            ContentLayer {
                blend: Blend::Multiply,
                ..layer(1)
            },
        )),
        "screen" => Some((
            Content::Screen,
            ContentLayer {
                blend: Blend::Emit,
                strength: 1.4,
                ..layer(2)
            },
        )),
        "ink" => Some((
            Content::Ink,
            ContentLayer {
                ink_roughness: 0.35,
                emboss: 0.3,
                ..layer(0)
            },
        )),
        _ => None,
    }
}

pub fn glass_library() -> Vec<(&'static str, Material)> {
    vec![
        (
            "glass",
            Material {
                family: Family::Glass,
                base: [1.0, 1.0, 1.0],
                roughness: 0.02,
                specular: 0.04,
                transmission: 1.0,
                ior: 1.5,

                ..Material::default()
            },
        ),
        (
            "tinted",
            Material {
                family: Family::Glass,
                base: [0.35, 0.75, 0.55],
                roughness: 0.03,
                specular: 0.04,
                transmission: 1.0,
                ior: 1.5,
                absorption: 4.0,
                ..Material::default()
            },
        ),
        (
            "water",
            Material {
                family: Family::Liquid,
                base: [1.0, 1.0, 1.0],
                roughness: 0.02,
                specular: 0.02,
                transmission: 1.0,
                ior: 1.333,
                ..Material::default()
            },
        ),
        (
            "milk",
            Material {
                family: Family::Glass,
                base: [0.96, 0.95, 0.93],
                roughness: 0.25,
                specular: 0.04,
                transmission: 1.0,
                ior: 1.47,
                subsurface: 0.8,

                ..Material::default()
            },
        ),
        (
            "soap",
            Material {
                family: Family::Glass,
                base: [1.0, 1.0, 1.0],
                roughness: 0.0,
                specular: 0.02,
                transmission: 1.0,
                ior: 1.0,
                thin_film: 420.0,
                thin_film_ior: 1.33,
                thin_film_amount: 1.0,

                ..Material::default()
            },
        ),
        (
            "film_slab",
            Material {
                family: Family::Glass,
                base: [1.0, 1.0, 1.0],
                roughness: 0.02,
                specular: 0.04,
                transmission: 1.0,
                ior: 1.5,
                thin_film: 300.0,
                thin_film_ior: 2.0,
                thin_film_amount: 0.8,
                ..Material::default()
            },
        ),
    ]
}

pub fn material_index(materials: &[(&str, Material)], name: &str) -> usize {
    materials
        .iter()
        .position(|(key, _)| *key == name)
        .unwrap_or_else(|| panic!("the bench library has no {name}"))
}

fn cube_geometry() -> Geometry {
    let mesh = cube();
    let pad = 0.02;
    let uvs1 = mesh
        .uvs
        .iter()
        .enumerate()
        .map(|(index, uv)| {
            let face = index / 4;
            let (column, row) = ((face % 3) as f32, (face / 3) as f32);
            [
                (column + pad + uv[0] * (1.0 - 2.0 * pad)) / 3.0,
                (row + pad + uv[1] * (1.0 - 2.0 * pad)) / 2.0,
            ]
        })
        .collect();
    Geometry {
        mesh,
        uvs1: Some(uvs1),
        alpha: None,
    }
}

fn can_geometry(segments: u32) -> Geometry {
    let mesh = can(segments);
    let side = (segments as usize + 1) * 2;
    let cap = segments as usize + 2;
    let pad = 0.02;
    let uvs1 = mesh
        .uvs
        .iter()
        .enumerate()
        .map(|(index, uv)| {
            if index < side {
                [
                    pad + uv[0] * (1.0 - 2.0 * pad),
                    pad + uv[1] * (0.5 - 2.0 * pad),
                ]
            } else {
                let left = if index < side + cap { 0.0 } else { 0.5 };
                [
                    left + pad + uv[0] * (0.5 - 2.0 * pad),
                    0.5 + pad + uv[1] * (0.5 - 2.0 * pad),
                ]
            }
        })
        .collect();
    Geometry {
        mesh,
        uvs1: Some(uvs1),
        alpha: None,
    }
}

fn page() -> Geometry {
    let mut geometry = geometry(grid(24, 16));
    geometry.uvs1 = Some(
        geometry
            .mesh
            .uvs
            .iter()
            .map(|uv| [uv[0] * 0.5 + 0.25, uv[1] * 0.5 + 0.25])
            .collect(),
    );
    geometry
}

fn screen_mesh() -> Geometry {
    let mut geometry = geometry(grid(32, 32));
    geometry.alpha = Some(
        geometry
            .mesh
            .uvs
            .iter()
            .map(|uv| {
                let cell = [(uv[0] * 8.0).fract() - 0.5, (uv[1] * 8.0).fract() - 0.5];
                if cell[0] * cell[0] + cell[1] * cell[1] < 0.09 {
                    0.0
                } else {
                    1.0
                }
            })
            .collect(),
    );
    geometry
}

struct Parts<'a> {
    materials: &'a [(&'static str, Material)],
    out: Vec<Part>,
}

impl Parts<'_> {
    fn add(&mut self, name: &str, mesh: usize, model: Matrix, material: &str) -> usize {
        self.out.push(Part {
            name: name.to_string(),
            mesh,
            model,
            material: material_index(self.materials, material),
            cast: Cast::Casts,
            age: 0.0,
            cutout: false,
        });
        self.out.len() - 1
    }
}

pub fn room() -> Room {
    let materials = library();
    let mut parts = Parts {
        materials: &materials,
        out: Vec::new(),
    };
    let w = WALL;
    parts.add(
        "floor",
        CUBE,
        block(
            [LEFT - w, FLOOR - w, BACK - w],
            [RIGHT + w, FLOOR, FRONT + w],
        ),
        "floor",
    );
    parts.add(
        "ceiling",
        CUBE,
        block(
            [LEFT - w, CEILING, BACK - w],
            [RIGHT + w, CEILING + w, FRONT + w],
        ),
        "ceiling",
    );
    parts.add(
        "back wall",
        CUBE,
        block([LEFT, FLOOR, BACK - w], [RIGHT, CEILING, BACK]),
        "wall",
    );
    parts.add(
        "right wall",
        CUBE,
        block([RIGHT, FLOOR, BACK], [RIGHT + w, CEILING, FRONT]),
        "wall",
    );
    parts.add(
        "front wall",
        CUBE,
        block([LEFT, FLOOR, FRONT], [RIGHT, CEILING, FRONT + w]),
        "wall",
    );
    parts.add(
        "window wall below",
        CUBE,
        block(
            [LEFT - w, FLOOR, WINDOW_Z[0]],
            [LEFT, WINDOW_Y[0], WINDOW_Z[1]],
        ),
        "wall",
    );
    parts.add(
        "window wall above",
        CUBE,
        block(
            [LEFT - w, WINDOW_Y[1], WINDOW_Z[0]],
            [LEFT, CEILING, WINDOW_Z[1]],
        ),
        "wall",
    );
    parts.add(
        "window wall back",
        CUBE,
        block([LEFT - w, FLOOR, BACK], [LEFT, CEILING, WINDOW_Z[0]]),
        "wall",
    );
    parts.add(
        "window wall front",
        CUBE,
        block([LEFT - w, FLOOR, WINDOW_Z[1]], [LEFT, CEILING, FRONT]),
        "wall",
    );
    parts.add(
        "sill",
        CUBE,
        block(
            [LEFT - w - 0.03, WINDOW_Y[0] - 0.03, WINDOW_Z[0] - 0.05],
            [LEFT + 0.09, WINDOW_Y[0], WINDOW_Z[1] + 0.05],
        ),
        "slate",
    );
    let pitch = (WINDOW_Y[1] - WINDOW_Y[0]) / SLATS as f32;
    for slat in 0..SLATS {
        let y = WINDOW_Y[0] + pitch * (slat as f32 + 0.5);
        parts.add(
            &format!("slat {slat}"),
            CUBE,
            place(
                [LEFT - w * 0.5, y, (WINDOW_Z[0] + WINDOW_Z[1]) * 0.5],
                turn_z(0.35),
                [0.075, 0.008, WINDOW_Z[1] - WINDOW_Z[0]],
            ),
            if slat % 2 == 0 { "steel" } else { "table" },
        );
    }
    let [cx, cz] = TABLE_CENTRE;
    let [hx, hz] = TABLE_HALF;
    parts.add(
        "table top",
        CUBE,
        block(
            [cx - hx, TABLE_TOP - 0.04, cz - hz],
            [cx + hx, TABLE_TOP, cz + hz],
        ),
        "table",
    );
    for (index, (sx, sz)) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
        .into_iter()
        .enumerate()
    {
        parts.add(
            &format!("table leg {index}"),
            CAN,
            place(
                [
                    cx + sx * (hx - 0.08),
                    (TABLE_TOP - 0.04) * 0.5,
                    cz + sz * (hz - 0.07),
                ],
                identity(),
                [0.05, TABLE_TOP - 0.04, 0.05],
            ),
            "steel",
        );
    }
    for (index, y) in SHELF_Y.into_iter().enumerate() {
        parts.add(
            &format!("shelf {index}"),
            CUBE,
            block(
                [SHELF_X[0], y - LIP, BACK],
                [SHELF_X[1], y, BACK + SHELF_DEPTH],
            ),
            if index == 1 { "lacquer_black" } else { "table" },
        );
        for (side, x) in [SHELF_X[0] + 0.15, SHELF_X[1] - 0.15]
            .into_iter()
            .enumerate()
        {
            parts.add(
                &format!("bracket {index} {side}"),
                CAN,
                place(
                    [x, y - LIP - 0.09, BACK + 0.05],
                    identity(),
                    [0.025, 0.18, 0.025],
                ),
                "brass",
            );
        }
    }
    let shelf_objects = [
        ("pages", CUBE, [0.05, 0.24, 0.17], "pages"),
        ("cover", CUBE, [0.035, 0.26, 0.19], "cover"),
        ("lacquer", BALL, [0.1, 0.1, 0.1], "lacquer"),
        ("brass", CAN, [0.07, 0.12, 0.07], "brass"),
        ("slate", CUBE, [0.12, 0.08, 0.12], "slate"),
        ("chrome", BALL, [0.09, 0.09, 0.09], "chrome"),
        ("felt", CUBE, [0.16, 0.05, 0.16], "felt"),
        ("cord", CAN, [0.09, 0.03, 0.09], "cord"),
        ("anodised", CAN, [0.06, 0.15, 0.06], "anodised"),
        ("plain", CUBE, [0.1, 0.1, 0.1], "plain"),
    ];
    for (row, y) in SHELF_Y.into_iter().enumerate() {
        let mut x = SHELF_X[0] + 0.32;
        let mut index = 0;
        while x < SHELF_X[1] - 0.3 {
            let pick = (row * 7 + index * 3) % shelf_objects.len();
            let (name, mesh, size, material) = shelf_objects[pick];
            let jitter = hash((row * 97 + index) as u32);
            let lean = if mesh == CUBE && size[1] > 0.2 {
                turn_z((jitter - 0.5) * 0.12)
            } else {
                turn_y(jitter * TAU)
            };
            let id = parts.add(
                &format!("shelf {row} {name} {index}"),
                mesh,
                place(
                    [
                        x,
                        y + size[1] * 0.5,
                        BACK + 0.04 + size[2] * 0.5 + jitter * 0.05,
                    ],
                    lean,
                    size,
                ),
                material,
            );
            if index % 4 == 1 {
                parts.out[id].age = 1.0;
            }
            x += size[0] + 0.06 + jitter * 0.08;
            index += 1;
        }
    }
    parts.add(
        "board",
        CUBE,
        place(
            [-1.75, TABLE_TOP + 0.12, -1.0],
            turn_x(-0.45),
            [0.6, 0.02, 0.45],
        ),
        "table",
    );
    parts.add(
        "board rest",
        CUBE,
        block([-2.05, TABLE_TOP, -1.24], [-1.45, TABLE_TOP + 0.05, -1.18]),
        "steel",
    );
    for (index, (at, size, mesh, material)) in [
        ([-1.85, 0.02, -0.6], [0.04, 0.04, 0.04], CUBE, "plain"),
        ([-1.74, 0.015, -0.55], [0.03, 0.03, 0.03], BALL, "brass"),
        ([-1.8, 0.01, -0.69], [0.06, 0.02, 0.025], CUBE, "slate"),
    ]
    .into_iter()
    .enumerate()
    {
        parts.add(
            &format!("contact {index}"),
            mesh,
            place([at[0], TABLE_TOP + at[1], at[2]], identity(), size),
            material,
        );
    }
    for (index, material) in ["chrome", "brass", "lacquer"].into_iter().enumerate() {
        parts.add(
            &format!("{material} sphere"),
            BALL,
            place(
                [-0.5 + index as f32 * 0.2, TABLE_TOP + 0.08, -1.2],
                identity(),
                [0.16, 0.16, 0.16],
            ),
            material,
        );
    }
    let cavity = [2.0, FLOOR, -1.6];
    for (name, lo, hi) in [
        ("cavity back", [0.0, 0.0, -0.3], [0.6, 0.5, -0.27]),
        ("cavity top", [0.0, 0.47, -0.3], [0.6, 0.5, 0.3]),
        ("cavity left", [0.0, 0.0, -0.3], [0.03, 0.5, 0.3]),
        ("cavity right", [0.57, 0.0, -0.3], [0.6, 0.5, 0.3]),
    ] {
        parts.add(
            name,
            CUBE,
            block(
                std::array::from_fn(|k| cavity[k] + lo[k]),
                std::array::from_fn(|k| cavity[k] + hi[k]),
            ),
            "lacquer_black",
        );
    }
    parts.add(
        "cavity ball",
        BALL,
        place([2.3, FLOOR + 0.1, -1.7], identity(), [0.2, 0.2, 0.2]),
        "plain",
    );
    parts.add(
        "felt mat",
        CUBE,
        block([0.1, TABLE_TOP, -0.65], [0.6, TABLE_TOP + 0.004, -0.35]),
        "felt",
    );
    parts.add(
        "branch",
        CAN,
        place(
            [1.14, TABLE_TOP + 0.16, -1.2],
            turn_z(0.25),
            [0.012, 0.3, 0.012],
        ),
        "bark",
    );
    for petal in 0..5 {
        let angle = petal as f32 * TAU / 5.0;
        parts.add(
            &format!("petal {petal}"),
            BALL,
            place(
                [
                    1.1 + angle.cos() * 0.025,
                    TABLE_TOP + 0.3,
                    -1.2 + angle.sin() * 0.025,
                ],
                turn_y(-angle),
                [0.045, 0.008, 0.025],
            ),
            "petal",
        );
    }
    for leaf in 0..3 {
        let angle = leaf as f32 * 2.1 + 0.4;
        parts.add(
            &format!("leaf {leaf}"),
            BALL,
            place(
                [
                    1.16 + angle.cos() * 0.04,
                    TABLE_TOP + 0.12 + leaf as f32 * 0.04,
                    -1.2 + angle.sin() * 0.04,
                ],
                turn_y(-angle),
                [0.07, 0.006, 0.03],
            ),
            "leaf",
        );
    }
    parts.add(
        "cord",
        CAN,
        place([1.2, 2.05, -0.5], identity(), [0.006, 1.5, 0.006]),
        "cord",
    );
    let lamp = parts.add(
        "lamp",
        BALL,
        place([1.2, 1.25, -0.5], identity(), [0.12, 0.12, 0.12]),
        "lamp",
    );
    parts.out[lamp].cast = Cast::Never;
    parts.add(
        "frost panel",
        CUBE,
        block([2.95, 0.9, -0.6], [RIGHT, 2.0, 0.6]),
        "frost",
    );
    parts.add(
        "crumpled",
        BALL,
        place(
            [1.15, TABLE_TOP + 0.03, -0.85],
            turn_y(0.7),
            [0.09, 0.06, 0.08],
        ),
        "crumpled",
    );
    let awning = parts.add(
        "awning",
        CUBE,
        block(
            [LEFT - 0.9, WINDOW_Y[1] + 0.25, WINDOW_Z[0] - 0.3],
            [LEFT - w, WINDOW_Y[1] + 0.28, WINDOW_Z[1] + 0.3],
        ),
        "occluder",
    );
    parts.out[awning].cast = Cast::Only;
    for (index, at) in [[2.4, 1.0, -2.15], [2.4, 1.0, -1.95]]
        .into_iter()
        .enumerate()
    {
        let quiet = parts.add(
            &format!("quiet {index}"),
            CUBE,
            place(at, identity(), [0.08, 0.08, 0.08]),
            "plain",
        );
        parts.out[quiet].cast = Cast::Never;
    }
    let fresh = parts.add(
        "sheet new",
        PAGE,
        place(SHEET, identity(), [SHEET_SIZE[0], 1.0, SHEET_SIZE[1]]),
        "ink",
    );
    parts.out[fresh].age = 0.0;
    let aged = parts.add(
        "sheet aged",
        PAGE,
        place(
            [SHEET[0] + 0.6, SHEET[1], SHEET[2] + 0.02],
            turn_y(0.08),
            [SHEET_SIZE[0], 1.0, SHEET_SIZE[1]],
        ),
        "sheet",
    );
    parts.out[aged].age = 1.0;
    parts.add(
        "photo",
        PAGE,
        place(
            [RIGHT - 0.01, 1.55, -1.3],
            frame::multiply(turn_z(PI * 0.5), turn_y(PI * 0.5)),
            [0.6, 1.0, 0.45],
        ),
        "photo",
    );
    let grille = parts.add(
        "grille",
        SCREEN,
        place(
            [RIGHT - 0.03, 1.55, 1.1],
            frame::multiply(turn_z(PI * 0.5), turn_y(PI * 0.5)),
            [0.5, 1.0, 0.5],
        ),
        "steel",
    );
    parts.out[grille].cutout = true;
    parts.add(
        "screen",
        PAGE,
        place(
            [-2.4, 1.45, BACK + 0.012],
            turn_x(PI * 0.5),
            [0.96, 1.0, 0.54],
        ),
        "screen",
    );
    parts.add(
        "decal box",
        CUBE,
        place(
            [0.35, TABLE_TOP + 0.064, -0.5],
            turn_y(-0.3),
            [0.12, 0.12, 0.12],
        ),
        "decal",
    );
    for card in 0..24 {
        let column = card % 8;
        let row = card / 8;
        parts.add(
            &format!("card {card}"),
            PAGE,
            place(
                [
                    -0.1 + column as f32 * 0.15,
                    2.5 - row as f32 * 0.12,
                    BACK + 0.004,
                ],
                turn_x(PI * 0.5),
                [0.12, 1.0, 0.09],
            ),
            "cards",
        );
    }
    let out = parts.out;
    Room {
        meshes: vec![
            ("cube", cube_geometry()),
            ("ball", geometry(ball(24, 48))),
            ("can", can_geometry(40)),
            ("page", page()),
            ("grille", screen_mesh()),
        ],
        parts: out,
        materials,
    }
}

pub fn cards(room: &Room) -> Vec<usize> {
    room.parts
        .iter()
        .enumerate()
        .filter(|(_, part)| part.name.starts_with("card "))
        .map(|(index, _)| index)
        .collect()
}

pub fn part(room: &Room, name: &str) -> usize {
    room.parts
        .iter()
        .position(|part| part.name == name)
        .unwrap_or_else(|| panic!("the room has no {name}"))
}
