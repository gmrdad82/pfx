use std::collections::BTreeMap;

use pfx_live::frame::{
    Instance, InstanceSurface, Matrix, Scene as FrameScene, SceneWind, multiply,
};
use pfx_live::glass;
use pfx_live::keyed::keyed_light;
use pfx_live::morph::InstanceWeights;
use pfx_live::particles::{SprayInstance, Sprays};
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
    pub weights: Vec<MorphWeights<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MorphWeights<'a> {
    pub instance: usize,
    pub current: &'a [f32],
    pub previous: &'a [f32],
}

#[derive(Default)]
pub(crate) struct Presenter {
    instances: Vec<Instance>,
    glass: Vec<glass::Surface>,
    materials: Vec<Material>,
    previous: Vec<Matrix>,
    previous_palettes: BTreeMap<usize, Vec<Matrix>>,
    palettes: BTreeMap<usize, Vec<Matrix>>,
    palette_key: Option<(u64, u64)>,
    weights: BTreeMap<usize, Vec<Vec<f32>>>,
    previous_weights: BTreeMap<usize, Vec<Vec<f32>>>,
    surfaces: Vec<InstanceSurface>,
    last_view_projection: Option<Matrix>,
    lens_camera: Option<Camera>,
    cut: bool,
    lit: u64,
    placed: Vec<pfx_live::lights::LocalLight>,
    exposed: u64,
    touched: Touched,
    sprays: Option<Sprays>,
    spray_parents: Vec<Matrix>,
    spray_next: Vec<Matrix>,
    spray_time: Option<f32>,
    spray_instances: Vec<SprayInstance>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Touched {
    pub lights: bool,
    pub finish: bool,
    pub posed: bool,
    pub morphed: bool,
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
            self.placed.clear();
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
        let keyer = staged.keyer();
        let keyed_at = world.time() as f32;
        let camera = world.fx.camera(&world.camera);
        let camera = keyer.map_or(camera.clone(), |keyer| keyer.camera(&camera, keyed_at));
        if self.lens_camera.as_ref() != Some(&camera) {
            renderer.set_lens(live_scene::lens(&camera, aspect));
            self.lens_camera = Some(camera.clone());
        }
        let posed = world.faced_models(&camera);
        let draws = staged.draws();
        let key = world.palette_key();
        let fresh = self.palette_key != Some(key);
        if fresh {
            self.previous_palettes = std::mem::replace(&mut self.palettes, world.palettes());
            self.previous_weights = std::mem::replace(&mut self.weights, world.weights());
            self.palette_key = Some(key);
        }
        let palettes = &self.palettes;
        let keyed = keyer.is_some_and(|keyer| !keyer.keys().lights.is_empty());
        if keyed || world.lights().values().any(|light| light.attach.is_some()) {
            let lights = live_scene::posed_lights(world.lights().values(), &posed, palettes);
            let lights = match keyer.filter(|_| keyed) {
                Some(keyer) => lights
                    .into_iter()
                    .zip(world.lights().keys())
                    .map(|(light, name)| keyed_light(keyer.keys(), name, light, keyed_at))
                    .collect(),
                None => lights,
            };
            if self.placed != lights {
                renderer.set_lights(&lights).map_err(PlayError::Render)?;
                self.placed = lights;
                self.touched.lights = true;
            }
        }
        let frames = world.sprite_frames();
        let models = draws.models_posed(&posed, palettes);
        let mut skinned: Vec<(usize, usize)> = Vec::new();
        let mut morphed: Vec<(usize, usize, usize)> = Vec::new();
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
                if let Some(morph) = &draw.geometry.morph
                    && self
                        .weights
                        .get(&owner)
                        .is_some_and(|sets| morph.morph < sets.len())
                {
                    morphed.push((opaque, owner, morph.morph));
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
                    previous: if fresh {
                        self.previous_palettes
                            .get(owner)
                            .filter(|previous| previous.len() == joints.len())
                            .unwrap_or(joints)
                    } else {
                        joints
                    },
                }
            })
            .collect();
        if !poses.is_empty() || self.touched.posed {
            renderer.set_poses(&poses).map_err(PlayError::Render)?;
            self.touched.posed = true;
        }
        drop(poses);
        self.spray(world, &posed, time);
        if !frames.is_empty() {
            renderer
                .set_surfaces(&self.surfaces)
                .map_err(PlayError::Render)?;
            self.touched.sprites = true;
        }
        if let Some(keyer) = keyer {
            keyer.apply(renderer, keyed_at).map_err(PlayError::Render)?;
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
        let weights = morphed
            .into_iter()
            .map(|(instance, owner, morph)| {
                let current = self.weights[&owner][morph].as_slice();
                let previous = if fresh {
                    self.previous_weights
                        .get(&owner)
                        .and_then(|sets| sets.get(morph))
                        .filter(|previous| previous.len() == current.len())
                        .map_or(current, Vec::as_slice)
                } else {
                    current
                };
                MorphWeights {
                    instance,
                    current,
                    previous,
                }
            })
            .collect::<Vec<_>>();
        if !weights.is_empty() || self.touched.morphed {
            let handed: Vec<InstanceWeights<'_>> = weights
                .iter()
                .map(|set| InstanceWeights {
                    instance: set.instance,
                    current: set.current,
                    previous: set.previous,
                })
                .collect();
            renderer.set_weights(&handed).map_err(PlayError::Render)?;
            self.touched.morphed = true;
        }
        Ok(PlayFrame {
            weights,
            scene: FrameScene {
                camera: frame_camera,
                time,
                seed,
                sun: live_scene::sun(
                    &keyer.map_or(staged.sun(), |keyer| keyer.sun(staged.sun(), keyed_at)),
                ),
                instances: &self.instances,
                materials: &self.materials,
                deformers: &[],
                wind: SceneWind::default(),
            },
            text,
            effects: Effects {
                glass: &self.glass,
                sprays: &self.spray_instances,
                ..Effects::default()
            },
        })
    }

    fn spray(&mut self, world: &World, posed: &[Matrix], time: f32) {
        let scene = world.scene();
        if self
            .sprays
            .as_ref()
            .is_none_or(|sprays| !sprays.same(scene))
        {
            self.sprays = Some(Sprays::of(scene));
            self.spray_time = None;
        }
        let Some(sprays) = self.sprays.as_mut().filter(|sprays| !sprays.is_empty()) else {
            self.spray_instances.clear();
            return;
        };
        sprays.parents(Some(posed), &self.palettes, &mut self.spray_next);
        let from = match self.spray_time {
            Some(last) if last <= time => last,
            _ => {
                self.spray_parents.clone_from(&self.spray_next);
                time
            }
        };
        sprays.seek_between([from, time], &self.spray_parents, &self.spray_next);
        pfx_live::particles::pack(sprays, &mut self.spray_instances);
        std::mem::swap(&mut self.spray_parents, &mut self.spray_next);
        self.spray_time = Some(time);
    }
}
