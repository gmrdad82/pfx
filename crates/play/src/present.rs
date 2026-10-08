use std::collections::BTreeMap;

use pfx_live::frame::{
    Instance, InstanceSurface, Matrix, Scene as FrameScene, SceneWind, multiply,
};
use pfx_live::glass;
use pfx_live::renderer::{Effects, Renderer, Text, invert};
use pfx_live::scene::{self as live_scene, Staged};
use pfx_live::skin::InstancePose;
use pfx_live::text::TextSpace;
use pfx_load::scene::{Camera, Scene, surface as sprite_surface};
use pfx_materials::{Family, Material};
use pfx_post::{Bloom, Chain, Pass, Tone};

use crate::PlayError;
use crate::world::{Look, ObjectId, World};

pub struct PlayFrame<'a> {
    pub scene: FrameScene<'a>,
    pub text: Text<'a>,
    pub effects: Effects<'a>,
}

#[derive(Default)]
pub(crate) struct Presenter {
    instances: Vec<Instance>,
    glass: Vec<glass::Surface>,
    materials: Vec<Material>,
    previous: Vec<Matrix>,
    previous_palettes: BTreeMap<usize, Vec<Matrix>>,
    surfaces: Vec<InstanceSurface>,
    last_view_projection: Option<Matrix>,
    lens_camera: Option<Camera>,
    cut: bool,
    lit: u64,
    exposed: u64,
    touched: Touched,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Touched {
    pub lights: bool,
    pub finish: bool,
    pub posed: bool,
    pub sprites: bool,
}

pub(crate) fn cut(renderer: &mut Renderer) -> Result<(), PlayError> {
    let (width, height) = renderer.size();
    renderer.resize(width, height).map_err(PlayError::Render)
}

pub(crate) fn exposed(scene: &Scene, exposure: f32) -> Chain {
    let mut chain = match &scene.finish {
        Some(finish) => finish.chain.clone(),
        None => Chain {
            passes: vec![Pass::Bloom(Bloom::neutral(0.0)), Pass::Tone(Tone::aces())],
            seed: 0,
            frame: 0,
        },
    };
    match chain.passes.first_mut() {
        Some(Pass::Exposure(value)) => *value = exposure,
        _ => chain.passes.insert(0, Pass::Exposure(exposure)),
    }
    chain
}

pub(crate) fn untouch(
    scene: &Scene,
    touched: Touched,
    renderer: &mut Renderer,
) -> Result<(), PlayError> {
    if touched.lights {
        renderer
            .set_lights(&live_scene::local_lights(scene))
            .map_err(PlayError::Render)?;
    }
    if touched.finish {
        match &scene.finish {
            Some(finish) => renderer.set_finish(finish.chain.clone()),
            None => renderer.clear_finish(),
        }
    }
    Ok(())
}

fn looked(world: &World, base: Material, look: &Look) -> Material {
    let mut material = look
        .material
        .as_deref()
        .and_then(|name| world.material(name))
        .unwrap_or(base);
    if let Some(color) = look.color {
        material.base = color;
    }
    if let Some(emission) = look.emission {
        material.emission = emission;
    }
    material
}

impl Presenter {
    pub(crate) fn touched(&self) -> Touched {
        self.touched
    }

    fn sync(&mut self, world: &World, renderer: &mut Renderer) -> Result<(), PlayError> {
        if self.lit != world.lights_revision() {
            let lights: Vec<_> = world
                .lights()
                .values()
                .map(pfx_live::lights::local_light)
                .collect();
            renderer.set_lights(&lights).map_err(PlayError::Render)?;
            self.lit = world.lights_revision();
            self.touched.lights = true;
        }
        if self.exposed != world.exposure_revision() {
            renderer.set_finish(exposed(world.scene(), world.exposure()));
            self.exposed = world.exposure_revision();
            self.touched.finish = true;
        }
        Ok(())
    }

    pub(crate) fn present<'a>(
        &'a mut self,
        world: &World,
        staged: &'a Staged,
        renderer: &mut Renderer,
        aspect: f32,
        time: f32,
        seed: u32,
    ) -> Result<PlayFrame<'a>, PlayError> {
        if !self.cut {
            cut(renderer)?;
            self.cut = true;
        }
        self.sync(world, renderer)?;
        let camera = world.fx.camera(&world.camera);
        if self.lens_camera != Some(camera) {
            renderer.set_lens(live_scene::lens(&camera, aspect));
            self.lens_camera = Some(camera);
        }
        let posed = world.faced_models(&camera);
        let draws = staged.draws();
        let palettes = world.palettes();
        let frames = world.sprite_frames();
        let models = draws.models_posed(&posed, &palettes);
        let mut skinned: Vec<(usize, usize)> = Vec::new();
        self.surfaces.clear();
        self.surfaces.extend_from_slice(staged.surfaces());
        self.instances.clear();
        self.instances.extend_from_slice(staged.instances());
        self.materials.clear();
        self.materials.extend_from_slice(staged.materials());
        let edited = world.edited_materials();
        if !edited.is_empty() {
            for (slot, name) in self.materials.iter_mut().zip(&draws.names) {
                if let Some(material) = edited.get(name) {
                    *slot = *material;
                }
            }
        }
        self.glass.clear();
        let staged_glass = staged.glass();
        let mut opaque = 0;
        let mut glass_at = 0;
        let mut made: BTreeMap<(usize, u32), u32> = BTreeMap::new();
        for (index, draw) in draws.items.iter().enumerate() {
            let model = models[index];
            let previous = self.previous.get(index).copied().unwrap_or(model);
            let object = draw
                .owner
                .filter(|&place| place < world.len())
                .map(ObjectId::from_index);
            let hidden = object.is_some_and(|id| world.hidden(id));
            let look = object
                .map(|id| world.look(id))
                .filter(|look| !look.is_plain());
            if draws.materials[draw.material as usize].family == Family::Glass {
                if staged
                    .overrides()
                    .get(&draw.object)
                    .is_some_and(|change| change.hidden)
                {
                    continue;
                }
                let Some(surface) = staged_glass.get(glass_at) else {
                    continue;
                };
                glass_at += 1;
                if hidden {
                    continue;
                }
                let base = draws
                    .names
                    .get(draw.material as usize)
                    .and_then(|name| edited.get(name))
                    .copied()
                    .unwrap_or(surface.material);
                let material = match look {
                    Some(look) => looked(world, base, look),
                    None => base,
                };
                self.glass.push(glass::Surface {
                    model,
                    material,
                    ..*surface
                });
                continue;
            }
            let Some(instance) = self.instances.get_mut(opaque) else {
                continue;
            };
            if let Some(owner) = draw.owner {
                if draw.geometry.skin.is_some() && palettes.contains_key(&owner) {
                    skinned.push((opaque, owner));
                }
                if let (Some(frame), Some(surface)) =
                    (frames.get(&owner), self.surfaces.get_mut(opaque))
                {
                    let (offset, scale, crop) = sprite_surface(*frame);
                    surface.uv_offset = offset;
                    surface.uv_scale = scale;
                    surface.crop = crop;
                }
            }
            opaque += 1;
            instance.model = model;
            instance.previous_model = previous;
            if hidden {
                instance.shadow_only = true;
                instance.casts_shadow = false;
            }
            if let (Some(look), Some(id)) = (look, object) {
                let key = (id.index(), instance.material);
                instance.material = match made.get(&key) {
                    Some(&material) => material,
                    None => {
                        let base = self.materials[instance.material as usize];
                        let material = self.materials.len() as u32;
                        self.materials.push(looked(world, base, look));
                        made.insert(key, material);
                        material
                    }
                };
            }
        }
        self.previous = models;
        let poses: Vec<InstancePose<'_>> = skinned
            .iter()
            .map(|(instance, owner)| {
                let joints = &palettes[owner];
                InstancePose {
                    instance: *instance,
                    joints,
                    previous: self
                        .previous_palettes
                        .get(owner)
                        .filter(|previous| previous.len() == joints.len())
                        .unwrap_or(joints),
                }
            })
            .collect();
        if !poses.is_empty() || self.touched.posed {
            renderer.set_poses(&poses).map_err(PlayError::Render)?;
            self.touched.posed = true;
        }
        drop(poses);
        self.previous_palettes = palettes;
        if !frames.is_empty() {
            renderer
                .set_surfaces(&self.surfaces)
                .map_err(PlayError::Render)?;
            self.touched.sprites = true;
        }
        let mut frame_camera = live_scene::camera(&camera, aspect);
        let view_projection = multiply(frame_camera.projection, frame_camera.view);
        if let Some(last) = self.last_view_projection {
            frame_camera.previous_view_projection = last;
        }
        self.last_view_projection = Some(view_projection);
        let mut text = staged.text(aspect);
        let rest = staged.camera();
        if rest != camera {
            let before = multiply(rest.projection(aspect), rest.view());
            if let Some(undo) = invert(before) {
                let fix = multiply(view_projection, undo);
                for item in &mut text.surface_quads {
                    match &mut item.space {
                        TextSpace::Surface {
                            model_view_projection,
                        } => *model_view_projection = multiply(fix, *model_view_projection),
                        TextSpace::LitSurface {
                            model_view_projection,
                            model,
                        } => *model_view_projection = multiply(view_projection, *model),
                        _ => {}
                    }
                }
            }
        }
        Ok(PlayFrame {
            scene: FrameScene {
                camera: frame_camera,
                time,
                seed,
                sun: live_scene::sun(&staged.sun()),
                instances: &self.instances,
                materials: &self.materials,
                deformers: &[],
                wind: SceneWind::default(),
            },
            text,
            effects: Effects {
                glass: &self.glass,
                ..Effects::default()
            },
        })
    }
}
