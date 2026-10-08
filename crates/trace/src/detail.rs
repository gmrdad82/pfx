use pfx_materials::{ContentLayer, ContentLook};

use crate::bvh::Triangle;
use crate::math::{cross, dot, normalize, sub};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriangleDetail {
    pub vertices: [[f32; 3]; 3],
    pub normals: [[f32; 3]; 3],
    pub uvs: [[f32; 2]; 3],
    pub material: u32,
}

impl TriangleDetail {
    pub fn flat(triangle: Triangle) -> Self {
        let [a, b, c] = triangle.vertices;
        let normal = normalize(cross(sub(b, a), sub(c, a)));
        Self {
            vertices: triangle.vertices,
            normals: [normal; 3],
            uvs: [[0.0; 2]; 3],
            material: triangle.material,
        }
    }

    pub fn interpolate(self, barycentric: [f32; 3]) -> ([f32; 3], [f32; 2]) {
        let normal = normalize(std::array::from_fn(|axis| {
            (0..3)
                .map(|corner| self.normals[corner][axis] * barycentric[corner])
                .sum()
        }));
        let uv = std::array::from_fn(|axis| {
            (0..3)
                .map(|corner| self.uvs[corner][axis] * barycentric[corner])
                .sum()
        });
        (normal, uv)
    }

    pub fn triangle(self) -> Triangle {
        Triangle {
            vertices: self.vertices,
            material: self.material,
        }
    }
}

pub type Transform = [[f32; 4]; 4];

#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    pub triangles: Vec<TriangleDetail>,
    pub transform: Transform,
    pub surface: InstanceSurface,
    pub shadow_only: bool,
    pub two_sided: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContentFace {
    #[default]
    Both,
    Front,
    Back,
}

impl ContentFace {
    pub fn code(self) -> u32 {
        match self {
            Self::Both => 0,
            Self::Front => 1,
            Self::Back => 2,
        }
    }

    pub fn shows(self, front: bool) -> bool {
        match self {
            Self::Both => true,
            Self::Front => front,
            Self::Back => !front,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstanceSurface {
    pub clip: [[f32; 4]; 2],
    pub uv_offset: [f32; 2],
    pub uv_scale: [f32; 2],
    pub crop: [f32; 4],
    pub face: ContentFace,
    pub layer: ContentLayer,
    pub cutout: bool,
    pub casts_shadow: bool,
    pub alpha_cutoff: f32,
}

impl Default for InstanceSurface {
    fn default() -> Self {
        Self {
            clip: [[0.0; 4]; 2],
            uv_offset: [0.0; 2],
            uv_scale: [1.0; 2],
            crop: [0.0, 0.0, 1.0, 1.0],
            face: ContentFace::Both,
            layer: ContentLayer::default(),
            cutout: false,
            casts_shadow: true,
            alpha_cutoff: 0.5,
        }
    }
}

impl InstanceSurface {
    pub fn valid(&self) -> bool {
        self.clip
            .iter()
            .flatten()
            .chain(self.uv_offset.iter())
            .chain(self.uv_scale.iter())
            .chain(self.crop.iter())
            .chain([
                &self.layer.ink_roughness,
                &self.layer.emboss,
                &self.layer.strength,
            ])
            .all(|value| value.is_finite())
            && (0.0..=1.0).contains(&self.alpha_cutoff)
    }

    pub fn cuts(&self, point: [f32; 3]) -> bool {
        self.clip.iter().any(|plane| {
            plane[..3].iter().any(|&value| value != 0.0)
                && plane[0] * point[0] + plane[1] * point[1] + plane[2] * point[2] > plane[3]
        })
    }

    pub fn content_uv(&self, uv: [f32; 2]) -> Option<[f32; 2]> {
        pfx_materials::content_uv(uv, self.uv_offset, self.uv_scale, self.crop)
    }
}

impl Instance {
    pub fn transformed(&self) -> Result<Vec<TriangleDetail>, String> {
        let m = self.transform;
        if m.into_iter().flatten().any(|value| !value.is_finite()) {
            return Err("instance transform is not finite".into());
        }
        if m[0][3] != 0.0 || m[1][3] != 0.0 || m[2][3] != 0.0 || m[3][3] != 1.0 {
            return Err("instance transform is not affine".into());
        }
        let a = |row: usize, column: usize| m[column][row];
        let cofactor = |row: usize, column: usize| {
            let (r0, r1) = ((row + 1) % 3, (row + 2) % 3);
            let (c0, c1) = ((column + 1) % 3, (column + 2) % 3);
            a(r0, c0) * a(r1, c1) - a(r0, c1) * a(r1, c0)
        };
        let determinant: f32 = (0..3)
            .map(|column| a(0, column) * cofactor(0, column))
            .sum();
        if determinant.abs() < 1e-12 {
            return Err("instance transform is singular".into());
        }
        let normal_matrix: [[f32; 3]; 3] = std::array::from_fn(|row| {
            std::array::from_fn(|column| cofactor(row, column) * determinant.signum())
        });
        let triangles = self
            .triangles
            .iter()
            .map(|triangle| TriangleDetail {
                vertices: triangle.vertices.map(|point| {
                    std::array::from_fn(|row| {
                        m[3][row]
                            + (0..3)
                                .map(|column| m[column][row] * point[column])
                                .sum::<f32>()
                    })
                }),
                normals: triangle
                    .normals
                    .map(|normal| normalize(normal_matrix.map(|row| dot(row, normal)))),
                uvs: triangle.uvs,
                material: triangle.material,
            })
            .collect();
        Ok(triangles)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    pub shift: [f32; 2],
    pub aperture: f32,
    pub focus_distance: f32,
}

impl Default for Lens {
    fn default() -> Self {
        Self {
            shift: [0.0; 2],
            aperture: 0.0,
            focus_distance: 1.0,
        }
    }
}

impl Lens {
    pub fn valid(self) -> bool {
        self.shift.into_iter().all(f32::is_finite)
            && self.aperture.is_finite()
            && self.aperture >= 0.0
            && self.focus_distance.is_finite()
            && self.focus_distance > 0.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContentImage {
    pub width: u32,
    pub height: u32,
    pub texels: Vec<[f32; 4]>,
}

impl ContentImage {
    pub fn valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && (self.width as usize)
                .checked_mul(self.height as usize)
                .is_some_and(|count| self.texels.len() == count)
            && self.texels.iter().flatten().all(|value| value.is_finite())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContentText {
    pub width: u32,
    pub height: u32,
    pub records: Vec<[f32; 4]>,
}

impl ContentText {
    pub fn valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.records.len() > 4
            && self.records.iter().flatten().all(|value| value.is_finite())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Detail {
    pub triangles: Vec<TriangleDetail>,
    pub instances: Vec<Instance>,
    pub lens: Lens,
    pub projection: crate::gpu::Projection,
    pub sun_radius_deg: f32,
    pub content: Vec<Option<ContentImage>>,
    pub content_slots: Vec<Option<ContentImage>>,
    pub text_slots: Vec<Option<ContentText>>,
    pub leaf: Option<LeafShade>,
    pub steam: Option<Steam>,
    pub lights: Vec<crate::lights::LocalLight>,
    pub content_look: ContentLook,
    pub bounces: Bounces,
    pub transmissive_shadows: bool,
    pub clamp_indirect: f32,
    pub filter_glossy: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounces {
    pub total: u32,
    pub diffuse: u32,
    pub glossy: u32,
    pub transmission: u32,
}

impl Bounces {
    pub const REFERENCE: Bounces = Bounces {
        total: 16,
        diffuse: 4,
        glossy: 8,
        transmission: 12,
    };
    pub const BLENDER: Bounces = Bounces {
        total: 12,
        diffuse: 4,
        glossy: 4,
        transmission: 12,
    };

    pub fn deep(count: u32) -> Bounces {
        Bounces {
            total: count,
            diffuse: count,
            glossy: count,
            transmission: count,
        }
    }

    pub fn valid(self) -> bool {
        self.total <= 64 && self.diffuse <= 64 && self.glossy <= 64 && self.transmission <= 64
    }
}

impl Default for Bounces {
    fn default() -> Self {
        Bounces::REFERENCE
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeafShade {
    pub center: [f32; 3],
    pub strength: f32,
    pub time: f32,
}

impl LeafShade {
    pub fn valid(self) -> bool {
        self.center.into_iter().all(f32::is_finite)
            && self.strength.is_finite()
            && (0.0..=1.0).contains(&self.strength)
            && self.time.is_finite()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlumeLook {
    pub spread: [f32; 2],
    pub drift: [[f32; 2]; 2],
    pub sway: [[f32; 5]; 2],
    pub fade: [f32; 3],
    pub grain: [f32; 2],
    pub lift: f32,
    pub warp: [f32; 2],
    pub churn: f32,
    pub threshold: [f32; 2],
}

impl Default for PlumeLook {
    fn default() -> Self {
        Self {
            spread: [0.7, 4.0],
            drift: [[-0.9, -0.12], [0.0, 0.02]],
            sway: [[17.0, 1.3, 0.012, 0.4, 6.0], [13.0, 0.9, 0.01, 0.4, 5.0]],
            fade: [0.02, 0.07, 0.2],
            grain: [70.0, 42.0],
            lift: 0.05,
            warp: [2.2, 0.2],
            churn: 0.7,
            threshold: [0.42, 0.9],
        }
    }
}

impl PlumeLook {
    pub const FLOATS: usize = 27;
    pub const NAMES: [&'static str; Self::FLOATS] = [
        "STEAM_SPREAD_BASE",
        "STEAM_SPREAD_GROWTH",
        "STEAM_DRIFT_X_QUAD",
        "STEAM_DRIFT_X_LIN",
        "STEAM_DRIFT_Z_QUAD",
        "STEAM_DRIFT_Z_LIN",
        "STEAM_SWAY_X_WAVE",
        "STEAM_SWAY_X_RATE",
        "STEAM_SWAY_X_AMP",
        "STEAM_SWAY_X_BASE",
        "STEAM_SWAY_X_GROWTH",
        "STEAM_SWAY_Z_WAVE",
        "STEAM_SWAY_Z_RATE",
        "STEAM_SWAY_Z_AMP",
        "STEAM_SWAY_Z_BASE",
        "STEAM_SWAY_Z_GROWTH",
        "STEAM_FADE_IN",
        "STEAM_FADE_OUT_START",
        "STEAM_FADE_OUT_END",
        "STEAM_GRAIN_H",
        "STEAM_GRAIN_V",
        "STEAM_LIFT_RATE",
        "STEAM_WARP_AMP",
        "STEAM_WARP_RATE",
        "STEAM_CHURN",
        "STEAM_THRESHOLD_LOW",
        "STEAM_THRESHOLD_HIGH",
    ];

    pub fn floats(self) -> [f32; Self::FLOATS] {
        let [sx, sz] = self.sway;
        [
            self.spread[0],
            self.spread[1],
            self.drift[0][0],
            self.drift[0][1],
            self.drift[1][0],
            self.drift[1][1],
            sx[0],
            sx[1],
            sx[2],
            sx[3],
            sx[4],
            sz[0],
            sz[1],
            sz[2],
            sz[3],
            sz[4],
            self.fade[0],
            self.fade[1],
            self.fade[2],
            self.grain[0],
            self.grain[1],
            self.lift,
            self.warp[0],
            self.warp[1],
            self.churn,
            self.threshold[0],
            self.threshold[1],
        ]
    }

    pub fn valid(self) -> bool {
        self.floats().into_iter().all(f32::is_finite)
            && self.spread[0] > 0.0
            && self.spread[0] + self.spread[1] * self.fade[2] > 0.0
            && self.fade[0] > 0.0
            && self.fade[1] < self.fade[2]
            && self.fade[2] > 0.0
            && self.threshold[0] < self.threshold[1]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Steam {
    pub source: [f32; 3],
    pub radius: f32,
    pub density: f32,
    pub ambient: f32,
    pub anisotropy: f32,
    pub box_lo: [f32; 3],
    pub box_hi: [f32; 3],
    pub time: f32,
    pub look: PlumeLook,
}

impl Steam {
    pub fn valid(self) -> bool {
        self.source
            .into_iter()
            .chain(self.box_lo)
            .chain(self.box_hi)
            .chain([
                self.radius,
                self.density,
                self.ambient,
                self.anisotropy,
                self.time,
            ])
            .all(f32::is_finite)
            && self.radius > 0.0
            && self.density >= 0.0
            && self.ambient >= 0.0
            && (-1.0..1.0).contains(&self.anisotropy)
            && (0..3).all(|axis| self.box_lo[axis] < self.box_hi[axis])
            && self.look.valid()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_budget_is_the_reference_budget() {
        assert_eq!(Detail::default().bounces, Bounces::REFERENCE);
        assert_eq!(
            Bounces::REFERENCE,
            Bounces {
                total: 16,
                diffuse: 4,
                glossy: 8,
                transmission: 12
            }
        );
        assert!(Bounces::deep(64).valid());
        assert!(!Bounces::deep(65).valid());
        assert!(
            !Bounces {
                glossy: 65,
                ..Bounces::BLENDER
            }
            .valid()
        );
    }

    #[test]
    fn interpolates_normals_and_uvs() {
        let triangle = TriangleDetail {
            vertices: [[0.0; 3]; 3],
            normals: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            uvs: [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            material: 0,
        };
        let (normal, uv) = triangle.interpolate([0.5, 0.25, 0.25]);
        assert_eq!(uv, [0.25, 0.25]);
        assert!((normal[0] - 0.8164966).abs() < 1e-6);
    }

    #[test]
    fn transforms_instance_positions_and_normals() {
        let triangle = TriangleDetail {
            vertices: [[0.0; 3]; 3],
            normals: [[1.0, 1.0, 0.0]; 3],
            uvs: [[0.0; 2]; 3],
            material: 0,
        };
        let instance = Instance {
            triangles: vec![triangle],
            surface: InstanceSurface::default(),
            shadow_only: false,
            two_sided: true,
            transform: [
                [2.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [3.0, 4.0, 5.0, 1.0],
            ],
        };
        let transformed = instance.transformed().unwrap();
        assert_eq!(transformed[0].vertices[0], [3.0, 4.0, 5.0]);
        assert!((transformed[0].normals[0][0] - 0.4472136).abs() < 1e-6);
    }

    #[test]
    fn instance_surface_matches_live_clips_and_uvs() {
        let surface = InstanceSurface {
            clip: [[1.0, 0.0, 0.0, 0.2], [0.0, -1.0, 0.0, 0.1]],
            uv_offset: [0.25, 0.0],
            uv_scale: [0.5, 1.0],
            crop: [0.25, 0.1, 0.75, 0.9],
            ..InstanceSurface::default()
        };
        assert!(surface.valid());
        assert!(
            InstanceSurface {
                alpha_cutoff: 0.0,
                ..surface
            }
            .valid()
        );
        assert!(
            InstanceSurface {
                alpha_cutoff: 1.0,
                ..surface
            }
            .valid()
        );
        assert!(
            !InstanceSurface {
                alpha_cutoff: 1.1,
                ..surface
            }
            .valid()
        );
        assert!(
            !InstanceSurface {
                alpha_cutoff: f32::NAN,
                ..surface
            }
            .valid()
        );
        assert!(!surface.cuts([0.1, 0.0, 0.0]));
        assert!(surface.cuts([0.3, 0.0, 0.0]));
        assert!(surface.cuts([0.0, -0.2, 0.0]));
        assert_eq!(surface.content_uv([0.5, 0.5]), Some([0.5, 0.5]));
        assert_eq!(surface.content_uv([0.5, 1.0]), None);
    }
}
