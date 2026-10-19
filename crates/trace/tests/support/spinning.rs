#![allow(dead_code)]

use pfx_gpu::Gpu;
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::detail::{Lens, Transform};
use pfx_trace::motion::Motion;
use pfx_trace::sobol::Sampler;
use pfx_trace::stage::{Mesh, Placement, PlacementMotion, Stage};
use pfx_trace::{Camera, Projection, Sun, Trace};

pub const TAN: f32 = 0.5;
pub const EYE: [f32; 3] = [0.0, 0.3, 6.0];
pub const BODY: [f32; 2] = [0.0, -0.2];
pub const BODY_SIZE: [f32; 2] = [3.0, 0.8];
pub const WHEEL: f32 = 0.55;
pub const WALL: f32 = -1.0;
pub const FLAG: [f32; 2] = [-3.6, -0.6];
pub const FLAG_HALF: f32 = 0.3;
pub const BALL: f32 = 0.35;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Moves {
    pub steps: usize,
    pub travel: f32,
    pub spin: f32,
    pub pan: f32,
    pub rise: f32,
    pub dense: usize,
}

pub struct Arrays {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl Arrays {
    fn new() -> Self {
        Self {
            positions: Vec::new(),
            normals: Vec::new(),
            tangents: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
        }
    }

    fn mesh(&self) -> Mesh<'_> {
        Mesh {
            positions: &self.positions,
            normals: &self.normals,
            tangents: &self.tangents,
            uvs: &self.uvs,
            uvs1: None,
            alpha: None,
            indices: &self.indices,
        }
    }

    fn corners(&mut self, corners: [[f32; 2]; 4]) {
        let first = self.positions.len() as u32;
        for [x, y] in corners {
            self.positions.push([x, y, 0.0]);
            self.normals.push([0.0, 0.0, 1.0]);
            self.tangents.push([1.0, 0.0, 0.0, 1.0]);
            self.uvs.push([x, y]);
        }
        self.indices
            .extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    }

    fn rect(lo: [f32; 2], hi: [f32; 2]) -> Self {
        let mut out = Self::new();
        out.corners([
            [lo[0], lo[1]],
            [hi[0], lo[1]],
            [hi[0], hi[1]],
            [lo[0], hi[1]],
        ]);
        out
    }

    fn ball(triangles: usize, radius: f32) -> Self {
        let rings = ((triangles as f64 / 4.0).sqrt() as u32).max(4);
        let segments = rings * 2;
        let mut out = Self::new();
        for ring in 0..=rings {
            let theta = std::f32::consts::PI * ring as f32 / rings as f32;
            for segment in 0..=segments {
                let phi = std::f32::consts::TAU * segment as f32 / segments as f32;
                let n = [
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ];
                let r = radius * (1.0 + 0.08 * (7.0 * phi).sin() * (5.0 * theta).sin());
                out.positions.push(n.map(|v| v * r));
                out.normals.push(n);
                out.tangents.push([1.0, 0.0, 0.0, 1.0]);
                out.uvs
                    .push([segment as f32 / segments as f32, ring as f32 / rings as f32]);
            }
        }
        let row = segments + 1;
        for ring in 0..rings {
            for segment in 0..segments {
                let a = ring * row + segment;
                let b = a + row;
                out.indices.extend([a, a + 1, b, a + 1, b + 1, b]);
            }
        }
        out
    }

    fn wheel() -> Self {
        let mut out = Self::new();
        let segments = 96;
        let ring = |out: &mut Self, inner: f32, outer: f32| {
            for k in 0..segments {
                let a = std::f32::consts::TAU * k as f32 / segments as f32;
                let b = std::f32::consts::TAU * (k + 1) as f32 / segments as f32;
                let at = |r: f32, t: f32| [r * t.cos(), r * t.sin()];
                out.corners([at(inner, a), at(outer, a), at(outer, b), at(inner, b)]);
            }
        };
        ring(&mut out, 0.45, WHEEL);
        ring(&mut out, 0.0, 0.08);
        for spoke in 0..8 {
            let a = std::f32::consts::TAU * spoke as f32 / 8.0;
            let (s, c) = a.sin_cos();
            let along = |r: f32, side: f32| [r * c - side * s, r * s + side * c];
            out.corners([
                along(0.07, -0.03),
                along(0.46, -0.03),
                along(0.46, 0.03),
                along(0.07, 0.03),
            ]);
        }
        out
    }
}

pub fn placed(at: [f32; 3], turn: f32) -> Transform {
    let (s, c) = turn.sin_cos();
    [
        [c, s, 0.0, 0.0],
        [-s, c, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [at[0], at[1], at[2], 1.0],
    ]
}

pub fn camera(width: u32, height: u32, shift: f32) -> Camera {
    let aspect = width as f32 / height as f32;
    Camera {
        origin: [EYE[0] + shift, EYE[1], EYE[2]],
        forward: [0.0, 0.0, -1.0],
        right: [TAN * aspect, 0.0, 0.0],
        up: [0.0, TAN, 0.0],
    }
}

pub fn pixel(width: u32, height: u32, p: [f32; 3]) -> [f32; 2] {
    let aspect = width as f32 / height as f32;
    let depth = EYE[2] - p[2];
    let ndc_x = (p[0] - EYE[0]) / (depth * TAN * aspect);
    let ndc_y = -(p[1] - EYE[1]) / (depth * TAN);
    [
        (ndc_x + 1.0) * 0.5 * width as f32,
        (ndc_y + 1.0) * 0.5 * height as f32,
    ]
}

fn materials(lit: bool) -> Vec<Material> {
    let paint = |colour: [f32; 3]| {
        if lit {
            Material {
                base: colour,
                roughness: 0.6,
                specular: 0.3,
                ..Material::default()
            }
        } else {
            Material {
                base: [0.0; 3],
                emission: colour,
                roughness: 1.0,
                specular: 0.0,
                ..Material::default()
            }
        }
    };
    vec![
        paint([0.1, 0.1, 0.1]),
        paint([1.0, 1.0, 1.0]),
        paint([1.0, 0.1, 0.1]),
        paint([0.9, 0.9, 0.9]),
        paint([0.1, 0.9, 0.2]),
        paint([0.2, 0.3, 0.9]),
    ]
}

pub struct Rig {
    arrays: Vec<Arrays>,
    flags: Vec<u32>,
}

impl Rig {
    pub fn new(moves: Moves) -> Self {
        let mut arrays = vec![
            Arrays::rect([-7.0, -4.0], [7.0, 4.0]),
            Arrays::rect([-0.06, 1.6], [0.06, 2.4]),
            Arrays::rect(
                [-BODY_SIZE[0] * 0.5, -BODY_SIZE[1] * 0.5],
                [BODY_SIZE[0] * 0.5, BODY_SIZE[1] * 0.5],
            ),
            Arrays::wheel(),
            Arrays::rect([-FLAG_HALF, -FLAG_HALF], [FLAG_HALF, FLAG_HALF]),
            Arrays::ball(moves.dense.max(16), BALL),
        ];
        let mut flags = Vec::new();
        if moves.steps > 0 && moves.rise != 0.0 {
            for k in 0..moves.steps {
                let u = time(k, moves.steps) - 0.5;
                flags.push(arrays.len() as u32);
                let mut flag = Arrays::rect([-FLAG_HALF, -FLAG_HALF], [FLAG_HALF, FLAG_HALF]);
                for at in [2, 3] {
                    flag.positions[at][1] += moves.rise * u;
                }
                arrays.push(flag);
            }
        }
        Self { arrays, flags }
    }
}

fn time(k: usize, steps: usize) -> f32 {
    k as f32 / (steps - 1).max(1) as f32
}

pub fn body(u: f32, moves: Moves) -> Transform {
    placed([BODY[0] + moves.travel * u, BODY[1], 0.0], 0.0)
}

pub fn wheel(u: f32, moves: Moves) -> Transform {
    placed([BODY[0] + moves.travel * u, BODY[1], 0.05], moves.spin * u)
}

pub fn stripes() -> Vec<Transform> {
    (0..16)
        .map(|k| placed([-4.5 + 0.6 * k as f32, 0.0, WALL + 0.02], 0.0))
        .collect()
}

pub struct Posed {
    pub models: Vec<Transform>,
    pub motion: Vec<Vec<Transform>>,
    pub cameras: Vec<Camera>,
}

pub fn posed(moves: Moves, width: u32, height: u32) -> Posed {
    let at = |u: f32| {
        let mut models = vec![placed([0.0, 0.0, WALL], 0.0)];
        models.extend(stripes());
        models.extend([
            body(u, moves),
            wheel(u, moves),
            placed([FLAG[0], FLAG[1], 0.0], 0.0),
        ]);
        if moves.dense > 0 {
            models.push(placed(
                [BODY[0] - 1.0 + moves.travel * u, BODY[1], 0.4],
                moves.spin * u,
            ));
        }
        models
    };
    let models = at(0.0);
    let mut motion = vec![Vec::new(); models.len()];
    let mut cameras = Vec::new();
    if moves.steps > 0 {
        let steps: Vec<Vec<Transform>> = (0..moves.steps)
            .map(|k| at(time(k, moves.steps) - 0.5))
            .collect();
        for (index, moving) in motion.iter_mut().enumerate() {
            *moving = steps.iter().map(|step| step[index]).collect();
        }
        cameras = (0..moves.steps)
            .map(|k| camera(width, height, moves.pan * (time(k, moves.steps) - 0.5)))
            .collect();
    }
    Posed {
        models,
        motion,
        cameras,
    }
}

pub fn stage(
    gpu: &Gpu,
    moves: Moves,
    lit: bool,
    size: [u32; 2],
    samples: u32,
) -> Result<Trace, String> {
    let rig = Rig::new(moves);
    let meshes: Vec<Mesh<'_>> = rig.arrays.iter().map(Arrays::mesh).collect();
    let posed = posed(moves, size[0], size[1]);
    let mut kinds = vec![0_u32];
    kinds.extend(std::iter::repeat_n(1, stripes().len()));
    kinds.extend([2, 3, 4]);
    if moves.dense > 0 {
        kinds.push(5);
    }
    let materials = materials(lit);
    let placements: Vec<Placement<'_>> = kinds
        .iter()
        .zip(&posed.models)
        .zip(&posed.motion)
        .map(|((&kind, &model), motion)| Placement {
            motion: (moves.steps > 0).then_some(PlacementMotion {
                transforms: motion,
                meshes: if kind == 4 { &rig.flags } else { &[] },
            }),
            ..Placement::new(kind, model, kind)
        })
        .collect();
    let mut staged = Stage {
        meshes: &meshes,
        instances: &placements,
        materials: &materials,
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![if lit {
                [0.3, 0.32, 0.35, 1.0]
            } else {
                [0.0; 4]
            }],
        },
        sun: Sun {
            direction: normalize([0.3, 0.6, 1.0]),
            color: [1.0, 0.95, 0.9],
            intensity: if lit { 3.0 } else { 0.0 },
        },
        camera: camera(size[0], size[1], 0.0),
        projection: Projection::Perspective,
        lens: Lens::default(),
    }
    .build()?;
    staged.detail.sampler = Sampler::Sobol { samples };
    if moves.steps > 0 {
        staged.detail.motion = Some(Motion {
            steps: moves.steps,
            cameras: posed.cameras,
            curve: Vec::new(),
        });
    }
    Ok(staged.trace(gpu, size[0], size[1])?)
}

pub fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|x| x / l)
}

pub fn luminance(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(16)
        .map(|texel| {
            let lane = |k: usize| {
                f32::from_le_bytes([
                    texel[4 * k],
                    texel[4 * k + 1],
                    texel[4 * k + 2],
                    texel[4 * k + 3],
                ])
            };
            0.2126 * lane(0) + 0.7152 * lane(1) + 0.0722 * lane(2)
        })
        .collect()
}
