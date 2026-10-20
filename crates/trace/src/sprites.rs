use std::sync::Arc;

use pfx_core::sim::particles::{Blend, Sprite};
use pfx_materials::Material;

use crate::detail::{Instance, InstanceSurface, SpriteLook, Transform, TriangleDetail};
use crate::gpu::{Camera, Trace};
use crate::math::{add, cross, dot, length, normalize, scale, sub};
use crate::motion::InstanceMotion;
use crate::stage::Staged;
use pfx_gpu::Gpu;

pub const SOFT_FLOOR: f32 = 0.02;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Puff {
    pub sprite: Sprite,
    pub stretch: f32,
    pub blend: Blend,
    pub lit: bool,
}

#[derive(Clone, Debug)]
pub struct Spray<'a> {
    pub puffs: &'a [Puff],
    pub camera: Camera,
    pub soft: bool,
    pub steps: &'a [f32],
}

impl<'a> Spray<'a> {
    pub fn new(puffs: &'a [Puff], camera: Camera) -> Self {
        Self {
            puffs,
            camera,
            soft: true,
            steps: &[],
        }
    }

    pub fn shown(&self) -> impl Iterator<Item = &'a Puff> + use<'a> {
        self.puffs.iter().filter(|puff| {
            let sprite = &puff.sprite;
            sprite.size > 0.0
                && sprite.size.is_finite()
                && sprite.alpha > 0.0
                && sprite.position.iter().all(|v| v.is_finite())
        })
    }

    pub fn instances(&self, triangles: &Arc<[TriangleDetail]>, moving: bool) -> Vec<Instance> {
        self.shown()
            .map(|puff| {
                let sprite = &puff.sprite;
                let additive = puff.blend == Blend::Add;
                let look = SpriteLook {
                    colour: sprite.colour.map(|c| c.max(0.0)),
                    alpha: sprite.alpha.clamp(0.0, 1.0),
                    lit: puff.lit,
                    additive,
                    soft: if self.soft {
                        (0.5 * sprite.size).max(SOFT_FLOOR)
                    } else {
                        0.0
                    },
                };
                let motion = moving.then(|| InstanceMotion {
                    transforms: self
                        .steps
                        .iter()
                        .map(|&dt| {
                            transform(
                                &self.camera,
                                puff,
                                add(sprite.position, scale(sprite.velocity, dt)),
                            )
                        })
                        .collect(),
                    shapes: Vec::new(),
                });
                Instance {
                    triangles: triangles.clone(),
                    transform: transform(&self.camera, puff, sprite.position),
                    surface: InstanceSurface {
                        casts_shadow: !additive,
                        sprite: Some(look),
                        ..InstanceSurface::default()
                    },
                    shadow_only: false,
                    two_sided: true,
                    motion,
                }
            })
            .collect()
    }

    pub fn moves(&self) -> bool {
        self.shown().any(|puff| {
            let sprite = &puff.sprite;
            let still = transform(&self.camera, puff, sprite.position);
            self.steps.iter().any(|&dt| {
                transform(
                    &self.camera,
                    puff,
                    add(sprite.position, scale(sprite.velocity, dt)),
                ) != still
            })
        })
    }
}

pub fn quad(material: u32) -> [TriangleDetail; 2] {
    let corner = |x: f32, y: f32| ([x, y, 0.0], [(x + 1.0) * 0.5, (y + 1.0) * 0.5]);
    let corners = [
        corner(-1.0, -1.0),
        corner(1.0, -1.0),
        corner(1.0, 1.0),
        corner(-1.0, 1.0),
    ];
    let triangle = |a: usize, b: usize, c: usize| TriangleDetail {
        vertices: [corners[a].0, corners[b].0, corners[c].0],
        normals: [[0.0, 0.0, 1.0]; 3],
        uvs: [corners[a].1, corners[b].1, corners[c].1],
        uvs1: [corners[a].1, corners[b].1, corners[c].1],
        material,
    };
    [triangle(0, 1, 2), triangle(0, 2, 3)]
}

pub fn material() -> Material {
    Material {
        base: [1.0; 3],
        roughness: 1.0,
        specular: 0.0,
        ..Material::default()
    }
}

pub fn facing(camera: &Camera, puff: &Puff) -> ([f32; 3], [f32; 3], f32, f32) {
    let sprite = &puff.sprite;
    let half = 0.5 * sprite.size;
    let right = normalize(camera.right);
    let up = normalize(camera.up);
    let (sin, cos) = sprite.angle.sin_cos();
    let mut axis_x = add(scale(right, cos), scale(up, sin));
    let mut axis_y = sub(scale(up, cos), scale(right, sin));
    let mut along = half;
    if puff.stretch > 0.0 {
        let to_eye = sub(sprite.position, camera.origin);
        let ray = if dot(to_eye, to_eye) > 1e-10 {
            normalize(to_eye)
        } else {
            normalize(camera.forward)
        };
        let v = sprite.velocity;
        let across = sub(v, scale(ray, dot(v, ray)));
        let speed = length(across);
        if speed > 1e-4 {
            axis_y = scale(across, 1.0 / speed);
            axis_x = normalize(cross(axis_y, ray));
            along = half + 0.5 * puff.stretch * speed;
        }
    }
    (axis_x, axis_y, half, along)
}

pub fn transform(camera: &Camera, puff: &Puff, at: [f32; 3]) -> Transform {
    let (axis_x, axis_y, half, along) = facing(camera, puff);
    let axis_z = normalize(cross(axis_x, axis_y));
    let column = |v: [f32; 3], k: f32| [v[0] * k, v[1] * k, v[2] * k, 0.0];
    [
        column(axis_x, half),
        column(axis_y, along),
        column(axis_z, half),
        [at[0], at[1], at[2], 1.0],
    ]
}

impl<E> Staged<E> {
    pub fn spray(&mut self, spray: &Spray<'_>) -> Result<usize, String> {
        if spray.shown().next().is_none() {
            return Ok(0);
        }
        let moving = match &self.detail.motion {
            Some(motion) if !spray.steps.is_empty() => {
                if spray.steps.len() != motion.steps {
                    return Err("particle motion needs one time per shutter step".into());
                }
                true
            }
            _ => false,
        };
        let slot = self.scene.materials.len() as u32;
        self.scene.materials.push(material());
        let triangles: Arc<[TriangleDetail]> = Arc::new(quad(slot));
        self.detail
            .instances
            .extend(spray.instances(&triangles, moving));
        Ok(self.detail.instances.len())
    }
}

impl Trace {
    pub fn set_spray(&mut self, gpu: &Gpu, spray: &Spray<'_>) -> Result<(), String> {
        let sprites = self.spray_instances(spray)?;
        if self.place_sprites(&sprites, gpu)? {
            self.finish_tops(gpu, true);
        }
        Ok(())
    }

    pub fn set_frame(
        &mut self,
        gpu: &Gpu,
        transforms: &[Transform],
        spray: &Spray<'_>,
        motion: Option<(&[Vec<Transform>], &[Camera])>,
    ) -> Result<(), String> {
        let sprites = self.spray_instances(spray)?;
        let moved = self.move_instances(transforms, gpu)?;
        let sprayed = self.place_sprites(&sprites, gpu)?;
        let posed = match motion {
            Some((transforms, cameras)) => {
                self.pose_motion(transforms, cameras)?;
                true
            }
            None => false,
        };
        if moved || sprayed || posed {
            self.finish_tops(gpu, sprayed || posed);
        }
        Ok(())
    }

    fn spray_instances(&self, spray: &Spray<'_>) -> Result<Vec<Instance>, String> {
        let Some(quad) = self.sprite_quad() else {
            return match spray.shown().next() {
                None => Ok(Vec::new()),
                Some(_) => Err("this trace was staged without sprites; stage it again".into()),
            };
        };
        if let Some(steps) = self.motion_steps()
            && !spray.steps.is_empty()
            && spray.steps.len() != steps
        {
            return Err("particle motion needs one time per shutter step".into());
        }
        Ok(spray.instances(&quad, !spray.steps.is_empty()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn puff(position: [f32; 3], velocity: [f32; 3], stretch: f32) -> Puff {
        Puff {
            sprite: Sprite {
                position,
                previous: position,
                velocity,
                size: 2.0,
                angle: 0.0,
                colour: [1.0; 3],
                alpha: 0.5,
            },
            stretch,
            blend: Blend::Alpha,
            lit: true,
        }
    }

    fn camera() -> Camera {
        Camera {
            origin: [0.0, 0.0, 5.0],
            forward: [0.0, 0.0, -1.0],
            right: [0.5, 0.0, 0.0],
            up: [0.0, 0.25, 0.0],
        }
    }

    #[test]
    fn a_sprite_faces_the_camera_at_its_size_and_streaks_along_its_motion() {
        let m = transform(
            &camera(),
            &puff([1.0, 2.0, 0.0], [0.0; 3], 0.0),
            [1.0, 2.0, 0.0],
        );
        assert_eq!(m[0], [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(m[1], [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(m[2], [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(m[3], [1.0, 2.0, 0.0, 1.0]);
        let streak = transform(&camera(), &puff([0.0; 3], [4.0, 0.0, 0.0], 0.5), [0.0; 3]);
        assert!((length([streak[1][0], streak[1][1], streak[1][2]]) - 2.0).abs() < 1e-6);
        assert!((streak[1][0] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn the_quad_spans_the_unit_disc_with_its_normal_toward_plus_z() {
        let [a, b] = quad(3);
        assert_eq!(a.material, 3);
        let e1 = sub(a.vertices[1], a.vertices[0]);
        let e2 = sub(a.vertices[2], a.vertices[0]);
        assert_eq!(normalize(cross(e1, e2)), [0.0, 0.0, 1.0]);
        assert_eq!(b.uvs[2], [0.0, 1.0]);
        assert_eq!(a.uvs[0], [0.0, 0.0]);
    }
}
