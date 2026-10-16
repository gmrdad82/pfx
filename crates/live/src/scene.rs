use std::collections::{BTreeMap, HashMap};

use pfx_geom::mesh::Mesh;
use pfx_load::scene::{
    Camera as SceneCamera, Content, Draws, Environment, Geometry, Haze, Scene, SceneDiff,
    Sun as SceneSun,
};
use pfx_materials::{Family, Material};
use pfx_post::dof;

use crate::effects::RoomHaze;
use crate::frame::{
    Camera, Instance, InstanceSurface, Matrix, MeshData, MeshHandle, Scene as FrameScene,
    SceneWind, Sun, multiply,
};
use crate::glass;
use crate::height_fog::HeightFog;
use crate::lights::{LocalLight, local_light};
use crate::maps::ContentFormat;
use crate::morph::{MorphData, MorphTargetData};
use crate::renderer::{Effects, Finish, Renderer, Text, TextQuadItem};
use crate::skin::SkinData;
use crate::sky::SkySource;
use crate::text::{GpuAtlas, RichQuad, TextSpace, rich_quads};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub uploaded: usize,
    pub replaced: usize,
    pub released: usize,
    pub contents: usize,
    pub texts: usize,
    pub draws: bool,
    pub lights: bool,
    pub sky: bool,
    pub sky_request: Option<u64>,
    pub haze: bool,
    pub sun: bool,
    pub lens: bool,
    pub finish: bool,
    pub motion_blur: bool,
}

impl Applied {
    pub fn touched_device(&self) -> bool {
        self.uploaded + self.replaced + self.released + self.contents + self.texts > 0
            || self.draws
            || self.lights
            || self.sky
            || self.haze
            || self.sun
            || self.lens
            || self.finish
            || self.motion_blur
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Override {
    pub hidden: bool,
    pub color: Option<[f32; 3]>,
    pub highlight: Option<[f32; 3]>,
    pub transform: Option<Matrix>,
}

impl Override {
    fn tints(&self) -> bool {
        self.color.is_some() || self.highlight.is_some()
    }

    fn moved(&self, model: Matrix) -> Matrix {
        match self.transform {
            Some(transform) => multiply(transform, model),
            None => model,
        }
    }

    fn material(&self, material: Material) -> Material {
        let mut out = material;
        if let Some(color) = self.color {
            out.base = color;
        }
        if let Some(highlight) = self.highlight {
            for (channel, add) in out.emission.iter_mut().zip(highlight) {
                *channel += add;
            }
        }
        out
    }
}

pub struct StagedFrame<'a> {
    pub scene: FrameScene<'a>,
    pub text: Text<'a>,
    pub effects: Effects<'a>,
}

struct StagedText {
    quads: Vec<RichQuad>,
    atlas: GpuAtlas,
    model: Matrix,
    lit: bool,
    dynamic: bool,
}

#[derive(Clone, Copy)]
enum Slot {
    Opaque(usize),
    Glass(usize),
    Plate,
}

pub struct Staged {
    meshes: HashMap<[u8; 32], MeshHandle>,
    glass_meshes: HashMap<[u8; 32], glass::MeshHandle>,
    draws: Draws,
    slots: Vec<Slot>,
    base: Vec<Instance>,
    base_glass: Vec<glass::Surface>,
    models: Vec<Matrix>,
    previous: Vec<Matrix>,
    instances: Vec<Instance>,
    surfaces: Vec<InstanceSurface>,
    glass: Vec<glass::Surface>,
    materials: Vec<Material>,
    overrides: BTreeMap<String, Override>,
    content_slots: BTreeMap<usize, [u8; 32]>,
    texts: Vec<StagedText>,
    posing: Option<Scene>,
    camera: SceneCamera,
    sun: SceneSun,
    plate: crate::plates::PlateMode,
    blurred: bool,
    shown: Option<Matrix>,
    sprays: crate::particles::Sprays,
    spray_instances: Vec<crate::particles::SprayInstance>,
    rig: Vec<pfx_load::scene::Light>,
    lights: Option<Vec<LocalLight>>,
    leveled: Vec<MeshHandle>,
}

fn mesh_data(geometry: &Geometry) -> MeshData<'_> {
    MeshData {
        positions: &geometry.positions,
        normals: &geometry.normals,
        tangents: &geometry.tangents,
        uvs: &geometry.uvs,
        uvs1: geometry.uvs1.as_deref(),
        alpha: None,
        indices: &geometry.indices,
    }
}

fn upload_geometry(renderer: &mut Renderer, geometry: &Geometry) -> Result<MeshHandle, String> {
    let skin = geometry.skin.as_ref().map(|skin| SkinData {
        joints: &skin.joints,
        weights: &skin.weights,
    });
    match (&geometry.morph, skin) {
        (Some(morph), skin) => {
            let targets: Vec<MorphTargetData<'_>> = morph
                .targets
                .iter()
                .map(|target| MorphTargetData {
                    positions: &target.positions,
                    normals: target.normals.as_deref(),
                })
                .collect();
            renderer.upload_morphed_mesh(mesh_data(geometry), skin, MorphData { targets: &targets })
        }
        (None, Some(skin)) => renderer.upload_skinned_mesh(mesh_data(geometry), skin),
        (None, None) => renderer.upload_mesh(mesh_data(geometry)),
    }
}

fn sprite_surface(scene: &Scene, draw: &pfx_load::scene::Draw, surface: &mut InstanceSurface) {
    let Some(frame) = draw
        .owner
        .and_then(|owner| scene.objects.get(owner))
        .and_then(|object| scene.animations.get(&object.name))
        .and_then(|animation| animation.still_frame())
    else {
        return;
    };
    let (offset, scale, crop) = pfx_load::scene::surface(frame);
    surface.uv_offset = offset;
    surface.uv_scale = scale;
    surface.crop = crop;
}

fn glass_mesh(geometry: &Geometry) -> Mesh {
    Mesh::new(
        geometry.positions.clone(),
        geometry.normals.clone(),
        geometry.tangents.clone(),
        geometry.uvs.clone(),
        geometry.indices.clone(),
    )
}

pub fn sky_source(environment: Environment) -> SkySource {
    match environment {
        Environment::Analytic(sky) => SkySource::Analytic(sky),
        Environment::Hdr(sky) => SkySource::Hdr(std::sync::Arc::unwrap_or_clone(sky)),
    }
}

pub fn local_lights(scene: &Scene) -> Vec<LocalLight> {
    scene.lights.values().map(local_light).collect()
}

pub fn posed_lights(lights: &[pfx_load::scene::Light], posed: &[Matrix]) -> Vec<LocalLight> {
    lights
        .iter()
        .map(|light| local_light(&light.placed(posed)))
        .collect()
}

fn light_rig(scene: &Scene) -> Vec<pfx_load::scene::Light> {
    if scene.animated() && scene.lights.values().any(|light| light.attach.is_some()) {
        scene.lights.values().copied().collect()
    } else {
        Vec::new()
    }
}

pub fn room_haze(haze: &Haze) -> RoomHaze {
    RoomHaze {
        fog: haze.fog,
        smoke: haze.smoke,
        mist: haze.mist,
        floor: haze.floor,
        phase: haze.phase,
        back: haze.back,
        gold: haze.gold,
        ambient: haze.ambient,
        reach: haze.reach,
        unmapped: haze.unmapped,
        seed: haze.seed,
        ..RoomHaze::new(haze.lo, haze.hi)
    }
}

pub fn height_fog(haze: &Haze) -> Option<HeightFog> {
    let outdoor = haze.outdoor?;
    Some(HeightFog {
        height: outdoor.height,
        falloff: outdoor.falloff,
        density: outdoor.density,
        reach: outdoor.reach,
        shafts: outdoor.shafts,
        steps: outdoor.steps,
        color: outdoor.color,
        phase: outdoor.phase,
        ambient: outdoor.ambient,
    })
}

fn apply_haze(renderer: &mut Renderer, haze: Option<&Haze>) {
    renderer.set_haze(haze.filter(|haze| haze.outdoor.is_none()).map(room_haze));
    renderer.set_height_fog(haze.and_then(height_fog));
}

pub fn lens(camera: &SceneCamera, aspect: f32) -> Option<dof::Lens> {
    let depth = camera.depth_of_field?;
    let tan_half_width = camera.tan_half_height()? * aspect;
    Some(dof::Lens {
        focal_m: depth.focal,
        sensor_m: dof::Lens::sensor_for_view(depth.focal, depth.distance, tan_half_width),
        fstop: depth.fstop,
        aperture: 1.0,
        focus_m: depth.distance,
        near_m: 0.0,
        near_blur: 0.0,
        far_m: 0.0,
        far_blur: 0.0,
        ground_m: None,
    })
}

pub fn camera(camera: &SceneCamera, aspect: f32) -> Camera {
    let view = camera.view();
    let projection = camera.projection(aspect);
    Camera {
        view,
        projection,
        previous_view_projection: multiply(projection, view),
        position: camera.at,
    }
}

fn blurred(scene: &Scene) -> bool {
    scene.motion_blur.is_some_and(|blur| blur.active())
}

pub fn sun(sun: &SceneSun) -> Sun {
    Sun {
        direction: sun.direction,
        colour: sun.color,
        intensity: sun.intensity,
    }
}

fn aspect_of(renderer: &Renderer) -> f32 {
    let (width, height) = renderer.size();
    width as f32 / height.max(1) as f32
}

fn content_slots(scene: &Scene, draws: &Draws) -> Result<BTreeMap<usize, String>, String> {
    let mut slots: BTreeMap<usize, String> = BTreeMap::new();
    for draw in &draws.items {
        let Some(content) = &draw.content else {
            continue;
        };
        let material = &draws.materials[draw.material as usize];
        if !material.content_layer.active() {
            continue;
        }
        let slot = material.content_layer.slot as usize;
        match slots.get(&slot) {
            Some(other) if other != content => {
                return Err(format!(
                    "{}: objects show content {other} and {content} through slot {slot} of material {}",
                    scene.path.display(),
                    draws.names[draw.material as usize]
                ));
            }
            _ => {
                slots.insert(slot, content.clone());
            }
        }
    }
    Ok(slots)
}

fn content_bytes(content: &Content) -> (ContentFormat, Vec<u8>) {
    if content.srgb {
        (ContentFormat::Srgb8, content.rgba.to_vec())
    } else {
        (
            ContentFormat::Linear16,
            content
                .rgba
                .iter()
                .flat_map(|&byte| (u16::from(byte) * 257).to_le_bytes())
                .collect(),
        )
    }
}

fn is_glass(draws: &Draws, index: usize) -> bool {
    let material = &draws.materials[draws.items[index].material as usize];
    material.family == Family::Glass || material.transmission > 0.0
}

fn set_sun_radius(renderer: &mut Renderer, radius: f32) -> bool {
    let quality = renderer.shadows().quality();
    if quality.sun_radius_deg == radius {
        return false;
    }
    renderer.set_shadow_quality(crate::shadow::Quality {
        sun_radius_deg: radius,
        ..quality
    });
    true
}

fn stage_texts(renderer: &Renderer, scene: &Scene) -> Result<Vec<StagedText>, String> {
    let gpu = renderer.gpu();
    let mut texts = Vec::new();
    for text in scene.texts.values() {
        let paragraph = text.paragraph()?;
        let quads = rich_quads(&paragraph);
        if quads.is_empty() {
            continue;
        }
        let atlas = GpuAtlas::new(&gpu.device, &gpu.queue, &paragraph.atlas)
            .map_err(|error| format!("{}: {error:?}", text.font.display()))?;
        texts.push(StagedText {
            quads,
            atlas,
            model: text.model(),
            lit: text.lit,
            dynamic: text.dynamic,
        });
    }
    Ok(texts)
}

impl Renderer {
    pub fn stage(&mut self, scene: &Scene) -> Result<Staged, String> {
        let draws = scene.draws();
        let wanted = content_slots(scene, &draws)?;
        let mut meshes = HashMap::new();
        let mut glass_meshes = HashMap::new();
        for (index, draw) in draws.items.iter().enumerate() {
            if is_glass(&draws, index) {
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    glass_meshes.entry(draw.geometry.hash)
                {
                    entry.insert(self.upload_glass(&glass_mesh(&draw.geometry))?);
                }
            } else {
                for geometry in crate::lod_levels::with_levels(&draw.geometry) {
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        meshes.entry(geometry.hash)
                    {
                        entry.insert(upload_geometry(self, geometry)?);
                    }
                }
            }
        }
        let mut staged = Staged {
            meshes,
            glass_meshes,
            draws: Draws::default(),
            slots: Vec::new(),
            base: Vec::new(),
            base_glass: Vec::new(),
            models: Vec::new(),
            previous: Vec::new(),
            instances: Vec::new(),
            surfaces: Vec::new(),
            glass: Vec::new(),
            materials: Vec::new(),
            overrides: BTreeMap::new(),
            content_slots: BTreeMap::new(),
            texts: stage_texts(self, scene)?,
            posing: None,
            camera: scene.camera_or_default(),
            sun: scene.sun_or_dark(),
            plate: crate::plates::PlateMode::default(),
            blurred: blurred(scene),
            shown: None,
            sprays: crate::particles::Sprays::of(scene),
            spray_instances: Vec::new(),
            rig: Vec::new(),
            lights: None,
            leveled: Vec::new(),
        };
        staged.plate.load(self, scene, true)?;
        staged.place(self, scene, draws, wanted)?;
        self.set_plate_hour(staged.sun.hour);
        self.set_lights(&local_lights(scene))?;
        self.set_sky(sky_source(scene.environment()), staged.sun.hour)?;
        apply_haze(self, scene.haze.as_ref());
        set_sun_radius(self, staged.sun.radius);
        self.set_lens(lens(&staged.camera, aspect_of(self)));
        if let Some(finish) = &scene.finish {
            self.set_finish(finish.chain.clone());
        }
        self.set_motion_blur(scene.motion_blur);
        Ok(staged)
    }
}

impl Staged {
    fn place(
        &mut self,
        renderer: &mut Renderer,
        scene: &Scene,
        draws: Draws,
        wanted: BTreeMap<usize, String>,
    ) -> Result<usize, String> {
        let mut slots = Vec::with_capacity(draws.items.len());
        let mut base = Vec::new();
        let mut base_glass = Vec::new();
        let mut surfaces = Vec::new();
        let mut stand_ins = Vec::new();
        for (index, draw) in draws.items.iter().enumerate() {
            if self.plate.baked(draw) {
                slots.push(Slot::Plate);
                if !is_glass(&draws, index)
                    && draw.shadow.casts()
                    && let Some(&mesh) = self.meshes.get(&draw.geometry.hash)
                {
                    stand_ins.push(Instance {
                        casts_shadow: true,
                        two_sided: draw.two_sided,
                        alpha_cutoff: draw.alpha_cutoff,
                        ..Instance::new(mesh, draw.model, draw.material, draw.id)
                    });
                }
                continue;
            }
            if is_glass(&draws, index) {
                slots.push(Slot::Glass(base_glass.len()));
                base_glass.push(glass::Surface {
                    mesh: self.glass_meshes[&draw.geometry.hash],
                    model: draw.model,
                    material: draws.materials[draw.material as usize],
                    liquid: false,
                    fluid_height: 0.0,
                    ripple_height: 0.0,
                    caustic_strength: 0.0,
                    tinted: None,
                    id: draw.id,
                    casts_shadow: draw.shadow.casts(),
                    shadow_only: !draw.shadow.drawn(),
                    clip: draw.clip,
                });
            } else {
                slots.push(Slot::Opaque(base.len()));
                base.push(Instance {
                    shadow_only: !draw.shadow.drawn(),
                    casts_shadow: draw.shadow.casts(),
                    two_sided: draw.two_sided,
                    alpha_cutoff: draw.alpha_cutoff,
                    ..Instance::new(
                        self.meshes[&draw.geometry.hash],
                        draw.model,
                        draw.material,
                        draw.id,
                    )
                });
                let mut surface = InstanceSurface {
                    clip: draw.clip,
                    ..InstanceSurface::default()
                };
                sprite_surface(scene, draw, &mut surface);
                surfaces.push(surface);
            }
        }
        renderer.frame().set_surfaces(&surfaces)?;
        self.leveled = crate::lod_levels::register(
            renderer,
            &draws,
            &self.meshes,
            |index| is_glass(&draws, index) || self.plate.baked(&draws.items[index]),
            &self.leveled,
        )?;
        self.plate.cast(renderer, &stand_ins)?;
        let mut written = 0;
        for (slot, name) in &wanted {
            let content = &scene.contents[name];
            if self.content_slots.get(slot) == Some(&content.hash) {
                continue;
            }
            let (format, bytes) = content_bytes(content);
            renderer
                .frame()
                .set_content(*slot, content.width, content.height, format)?;
            renderer
                .frame()
                .write_content(*slot, [0, 0, content.width, content.height], &bytes)?;
            self.content_slots.insert(*slot, content.hash);
            written += 1;
        }
        self.content_slots
            .retain(|slot, _| wanted.contains_key(slot));
        self.models = draws.items.iter().map(|draw| draw.model).collect();
        self.previous = self.models.clone();
        self.posing = scene.animated().then(|| scene.clone());
        self.rig = light_rig(scene);
        self.draws = draws;
        self.slots = slots;
        self.base = base;
        self.base_glass = base_glass;
        self.surfaces = surfaces;
        if let Some(posing) = &self.posing {
            let posed = posing.posed(&self.camera, 0.0);
            self.models = self.draws.models(&posed);
            self.previous = self.models.clone();
            self.place_lights(&posed);
        }
        self.refresh();
        Ok(written)
    }

    fn refresh(&mut self) {
        self.materials = self.draws.materials.clone();
        if self.materials.is_empty() {
            self.materials.push(Material::default());
        }
        self.instances = self.base.clone();
        self.glass.clear();
        let mut tinted: BTreeMap<(String, u32), u32> = BTreeMap::new();
        for (index, slot) in self.slots.iter().enumerate() {
            let draw = &self.draws.items[index];
            let shown = self.overrides.get(&draw.object).copied();
            let mut material = draw.material;
            if let Some(change) = shown.filter(Override::tints) {
                let key = (draw.object.clone(), draw.material);
                material = match tinted.get(&key) {
                    Some(&made) => made,
                    None => {
                        let made = self.materials.len() as u32;
                        self.materials
                            .push(change.material(self.draws.materials[draw.material as usize]));
                        tinted.insert(key, made);
                        made
                    }
                };
            }
            let hidden = shown.is_some_and(|change| change.hidden);
            let change = shown.unwrap_or_default();
            match *slot {
                Slot::Opaque(at) => {
                    let instance = &mut self.instances[at];
                    instance.model = change.moved(self.models[index]);
                    instance.previous_model = change.moved(self.previous[index]);
                    instance.material = material;
                    if hidden {
                        instance.shadow_only = true;
                        instance.casts_shadow = false;
                    }
                }
                Slot::Plate => {}
                Slot::Glass(at) => {
                    if hidden {
                        continue;
                    }
                    self.glass.push(glass::Surface {
                        model: change.moved(self.models[index]),
                        material: self.materials[material as usize],
                        ..self.base_glass[at]
                    });
                }
            }
        }
    }

    pub fn apply(
        &mut self,
        renderer: &mut Renderer,
        scene: &Scene,
        diff: &SceneDiff,
    ) -> Result<Applied, String> {
        let mut applied = Applied::default();
        if diff.is_empty() {
            return Ok(applied);
        }
        if !self.sprays.same(scene) {
            self.sprays = crate::particles::Sprays::of(scene);
        }
        if diff.camera {
            self.camera = scene.camera_or_default();
            renderer.set_lens(lens(&self.camera, aspect_of(renderer)));
            applied.lens = true;
        }
        if diff.plates || diff.draws() {
            self.plate.load(renderer, scene, diff.plates)?;
            let draws = scene.draws();
            let wanted = content_slots(scene, &draws)?;
            let mut needed: Vec<&Geometry> = Vec::new();
            let mut needed_glass: Vec<&Geometry> = Vec::new();
            for (index, draw) in draws.items.iter().enumerate() {
                let list = if is_glass(&draws, index) {
                    &mut needed_glass
                } else {
                    &mut needed
                };
                let glass = is_glass(&draws, index);
                for geometry in crate::lod_levels::with_levels(&draw.geometry).take(if glass {
                    1
                } else {
                    usize::MAX
                }) {
                    if !list.iter().any(|known| known.hash == geometry.hash) {
                        list.push(geometry);
                    }
                }
            }
            let mut spare: Vec<[u8; 32]> = self
                .meshes
                .keys()
                .filter(|hash| !needed.iter().any(|geometry| &geometry.hash == *hash))
                .copied()
                .collect();
            spare.sort();
            for geometry in needed {
                if self.meshes.contains_key(&geometry.hash) {
                    continue;
                }
                let reuse = if geometry.skin.is_none() && geometry.morph.is_none() {
                    spare
                        .iter()
                        .rposition(|old| {
                            self.meshes.get(old).is_some_and(|handle| {
                                !renderer.frame().skinned(*handle)
                                    && !renderer.frame().morphed(*handle)
                            })
                        })
                        .map(|at| spare.remove(at))
                } else {
                    None
                };
                match reuse {
                    Some(old) => {
                        let handle = self.meshes.remove(&old).ok_or("a staged mesh vanished")?;
                        renderer.replace_mesh(handle, mesh_data(geometry))?;
                        self.meshes.insert(geometry.hash, handle);
                        applied.replaced += 1;
                    }
                    None => {
                        let handle = upload_geometry(renderer, geometry)?;
                        self.meshes.insert(geometry.hash, handle);
                        applied.uploaded += 1;
                    }
                }
            }
            for old in spare {
                if let Some(handle) = self.meshes.remove(&old) {
                    renderer.release_mesh(handle)?;
                    applied.released += 1;
                }
            }
            let mut spare_glass: Vec<[u8; 32]> = self
                .glass_meshes
                .keys()
                .filter(|hash| !needed_glass.iter().any(|geometry| &geometry.hash == *hash))
                .copied()
                .collect();
            spare_glass.sort();
            for geometry in needed_glass {
                if self.glass_meshes.contains_key(&geometry.hash) {
                    continue;
                }
                match spare_glass.pop() {
                    Some(old) => {
                        let handle = self
                            .glass_meshes
                            .remove(&old)
                            .ok_or("a staged glass mesh vanished")?;
                        renderer.replace_glass(handle, &glass_mesh(geometry))?;
                        self.glass_meshes.insert(geometry.hash, handle);
                        applied.replaced += 1;
                    }
                    None => {
                        let handle = renderer.upload_glass(&glass_mesh(geometry))?;
                        self.glass_meshes.insert(geometry.hash, handle);
                        applied.uploaded += 1;
                    }
                }
            }
            for old in spare_glass {
                if let Some(handle) = self.glass_meshes.remove(&old) {
                    renderer.release_glass(handle)?;
                    applied.released += 1;
                }
            }
            applied.contents = self.place(renderer, scene, draws, wanted)?;
            applied.draws = true;
        } else if diff.camera && self.posing.is_some() {
            if let Some(posing) = &self.posing {
                self.models = self.draws.models(&posing.posed(&self.camera, 0.0));
                self.previous = self.models.clone();
            }
            self.refresh();
        }
        if !diff.texts.is_empty() {
            self.texts = stage_texts(renderer, scene)?;
            applied.texts = self.texts.len().max(1);
        }
        if !diff.lights.is_empty() {
            renderer.set_lights(&local_lights(scene))?;
            self.rig = light_rig(scene);
            self.lights = None;
            applied.lights = true;
        }
        if diff.sun {
            self.sun = scene.sun_or_dark();
            applied.sun = set_sun_radius(renderer, self.sun.radius);
            renderer.set_plate_hour(self.sun.hour);
        }
        if diff.sky {
            applied.sky_request =
                Some(renderer.request_sky(sky_source(scene.environment()), self.sun.hour)?);
            applied.sky = true;
        }
        if diff.haze {
            apply_haze(renderer, scene.haze.as_ref());
            applied.haze = true;
        }
        if diff.finish {
            match &scene.finish {
                Some(finish) => renderer.set_finish(finish.chain.clone()),
                None => renderer.clear_finish(),
            }
            applied.finish = true;
        }
        if diff.motion_blur {
            self.blurred = blurred(scene);
            renderer.set_motion_blur(scene.motion_blur);
            applied.motion_blur = true;
        }
        Ok(applied)
    }

    pub fn pose(&mut self, time: f32) {
        if let Some(posing) = &self.posing {
            let posed = posing.posed(&self.camera, time);
            let models = self.draws.models(&posed);
            self.previous = std::mem::replace(&mut self.models, models);
            self.place_lights(&posed);
            self.repose();
        }
    }

    pub fn pose_at(&mut self, time: f32, interval: f32) {
        if let Some(posing) = &self.posing {
            self.previous = self
                .draws
                .models(&posing.posed(&self.camera, time - interval));
            let posed = posing.posed(&self.camera, time);
            self.models = self.draws.models(&posed);
            self.place_lights(&posed);
            self.repose();
        }
    }

    fn place_lights(&mut self, posed: &[Matrix]) {
        self.lights = (!self.rig.is_empty()).then(|| posed_lights(&self.rig, posed));
    }

    fn repose(&mut self) {
        let mut glass = self.glass.iter_mut();
        for (index, slot) in self.slots.iter().enumerate() {
            let change = if self.overrides.is_empty() {
                Override::default()
            } else {
                self.overrides
                    .get(&self.draws.items[index].object)
                    .copied()
                    .unwrap_or_default()
            };
            match *slot {
                Slot::Opaque(at) => {
                    let instance = &mut self.instances[at];
                    instance.model = change.moved(self.models[index]);
                    instance.previous_model = change.moved(self.previous[index]);
                }
                Slot::Plate => {}
                Slot::Glass(_) => {
                    if change.hidden {
                        continue;
                    }
                    if let Some(surface) = glass.next() {
                        surface.model = change.moved(self.models[index]);
                    }
                }
            }
        }
    }

    pub fn scene(&self, aspect: f32, time: f32, seed: u32) -> FrameScene<'_> {
        FrameScene {
            camera: camera(&self.camera, aspect),
            time,
            seed,
            sun: sun(&self.sun),
            instances: &self.instances,
            materials: &self.materials,
            deformers: &[],
            wind: SceneWind::default(),
        }
    }

    pub fn frame(&mut self, aspect: f32, time: f32, seed: u32) -> StagedFrame<'_> {
        self.pose(time);
        self.staged_frame(aspect, time, seed)
    }

    pub fn frame_at(
        &mut self,
        aspect: f32,
        time: f32,
        interval: f32,
        seed: u32,
    ) -> StagedFrame<'_> {
        self.pose_at(time, interval);
        self.staged_frame(aspect, time, seed)
    }

    pub fn cut(&mut self) {
        self.shown = None;
    }

    fn staged_frame(&mut self, aspect: f32, time: f32, seed: u32) -> StagedFrame<'_> {
        self.spray(time);
        let shown = camera(&self.camera, aspect).previous_view_projection;
        let previous = self.shown.filter(|_| self.blurred).unwrap_or(shown);
        self.shown = Some(shown);
        let mut scene = self.scene(aspect, time, seed);
        scene.camera.previous_view_projection = previous;
        StagedFrame {
            scene,
            text: self.text(aspect),
            effects: self.effects(),
        }
    }

    pub fn text(&self, aspect: f32) -> Text<'_> {
        let view_projection = multiply(self.camera.projection(aspect), self.camera.view());
        Text {
            surface_quads: self
                .texts
                .iter()
                .filter(|text| text.dynamic || !self.plate.plated())
                .map(|text| TextQuadItem {
                    atlas: &text.atlas,
                    quads: &text.quads,
                    space: if text.lit {
                        TextSpace::LitSurface {
                            model_view_projection: multiply(view_projection, text.model),
                            model: text.model,
                        }
                    } else {
                        TextSpace::Surface {
                            model_view_projection: multiply(view_projection, text.model),
                        }
                    },
                    icons: false,
                    id: 0,
                })
                .collect(),
            ..Text::default()
        }
    }

    pub fn spray(&mut self, time: f32) {
        if self.sprays.is_empty() {
            return;
        }
        let camera = self.camera;
        let posing = self.posing.as_ref();
        self.sprays
            .seek_posed(time, |t| posing.map(|scene| scene.posed(&camera, t)));
        self.sprays.pack(&mut self.spray_instances);
    }

    pub fn sprays(&self) -> &crate::particles::Sprays {
        &self.sprays
    }

    pub fn effects(&self) -> Effects<'_> {
        Effects {
            glass: &self.glass,
            sprays: &self.spray_instances,
            lights: self.lights.as_deref(),
            ..Effects::default()
        }
    }

    pub fn lens(&self, aspect: f32) -> Option<dof::Lens> {
        lens(&self.camera, aspect)
    }

    pub fn set_override(&mut self, object: &str, change: Override) -> bool {
        if !self.draws.items.iter().any(|draw| draw.object == object) {
            return false;
        }
        if change == Override::default() {
            self.overrides.remove(object);
        } else {
            self.overrides.insert(object.to_string(), change);
        }
        self.refresh();
        true
    }

    pub fn clear_override(&mut self, object: &str) {
        if self.overrides.remove(object).is_some() {
            self.refresh();
        }
    }

    pub fn clear_overrides(&mut self) {
        if !self.overrides.is_empty() {
            self.overrides.clear();
            self.refresh();
        }
    }

    pub fn overrides(&self) -> &BTreeMap<String, Override> {
        &self.overrides
    }

    pub fn finish(&self) -> Finish {
        Finish::Standard
    }

    pub fn draws(&self) -> &Draws {
        &self.draws
    }

    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }

    pub fn glass(&self) -> &[glass::Surface] {
        &self.glass
    }

    pub fn materials(&self) -> &[Material] {
        &self.materials
    }

    pub fn surfaces(&self) -> &[InstanceSurface] {
        &self.surfaces
    }

    pub fn meshes(&self) -> usize {
        self.meshes.len() + self.glass_meshes.len()
    }

    pub fn texts(&self) -> usize {
        self.texts.len()
    }

    pub fn camera(&self) -> SceneCamera {
        self.camera
    }

    pub fn sun(&self) -> SceneSun {
        self.sun
    }

    pub fn plated(&self) -> bool {
        self.plate.plated()
    }

    pub fn release(self, renderer: &mut Renderer) -> Result<(), String> {
        for base in &self.leveled {
            renderer.lod_mut().clear_levels(*base);
        }
        self.plate.release(renderer)?;
        for handle in self.meshes.into_values() {
            renderer.release_mesh(handle)?;
        }
        for handle in self.glass_meshes.into_values() {
            renderer.release_glass(handle)?;
        }
        Ok(())
    }
}
