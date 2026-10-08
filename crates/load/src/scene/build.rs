use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pfx_core::anim::normalize_weights;
use pfx_core::camera::{DEG, Depth, Hush, Preset};
use pfx_core::daylight::{Authored, Daylight, REFERENCE_HOUR, Reading};
use pfx_core::sky::AnalyticSky;
use pfx_materials::{Library, Material};
use pfx_scene::types as file;
use pfx_scene::{Diagnostic, Entry, Project};
use sha2::{Digest, Sha256};

use super::{
    ALPHA_CUTOFF, Camera, Content, DepthOfField, Emitter, Environment, Finish, Geometry,
    GeometrySkin, Haze, IDENTITY, Light, Location, Matrix, Motion, Mover, MoverKind, NodeOverride,
    Object, Part, Projection, Scene, SceneError, SceneMesh, SceneSky, Shadow, Sun, Text,
    TraceSettings, model, multiply,
};
use crate::{
    ColorSpace, Font, Image, Mesh, Pixels, Primitive, RoomLamp, RoomSky, SkinRig, Sky, sha256,
};

pub(super) struct Reader<'a> {
    pub(super) files: BTreeMap<PathBuf, [u8; 32]>,
    root: &'a Path,
    overlay: &'a BTreeMap<PathBuf, String>,
}

impl Reader<'_> {
    pub(super) fn read(&mut self, path: &Path) -> Result<Vec<u8>, String> {
        let held = path
            .strip_prefix(self.root)
            .ok()
            .and_then(|relative| self.overlay.get(relative));
        let bytes = match held {
            Some(text) => text.clone().into_bytes(),
            None => {
                std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?
            }
        };
        self.files.insert(path.to_path_buf(), sha256(&bytes));
        Ok(bytes)
    }
}

pub(super) fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

fn column_of(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    let start = text[..offset].rfind('\n').map_or(0, |at| at + 1);
    text[start..offset].chars().count() + 1
}

#[derive(Clone, Copy)]
pub(super) enum Step<'a> {
    Key(&'a str),
    Named(&'a str, &'a str),
    Index(&'a str, usize),
}

pub(super) struct Places<'a> {
    pub(super) root: PathBuf,
    texts: RefCell<BTreeMap<PathBuf, Option<String>>>,
    overlay: &'a BTreeMap<PathBuf, String>,
}

fn descend<'t, 'i>(
    value: &'t toml::Spanned<toml::de::DeValue<'i>>,
    step: Step<'_>,
) -> Option<&'t toml::Spanned<toml::de::DeValue<'i>>> {
    let table = match value.get_ref() {
        toml::de::DeValue::Table(table) => table,
        _ => return None,
    };
    match step {
        Step::Key(key) => table.get(key),
        Step::Named(list, name) => match table.get(list)?.get_ref() {
            toml::de::DeValue::Array(items) => items.iter().find(|item| {
                match item.get_ref() {
                toml::de::DeValue::Table(entry) => entry.get("name").is_some_and(|value| {
                    matches!(value.get_ref(), toml::de::DeValue::String(text) if text == name)
                }),
                _ => false,
            }
            }),
            _ => None,
        },
        Step::Index(list, place) => match table.get(list)?.get_ref() {
            toml::de::DeValue::Array(items) => items.get(place),
            _ => None,
        },
    }
}

impl<'a> Places<'a> {
    pub(super) fn new(root: &Path, overlay: &'a BTreeMap<PathBuf, String>) -> Self {
        Self {
            root: root.to_path_buf(),
            texts: RefCell::new(BTreeMap::new()),
            overlay,
        }
    }

    fn text(&self, file: &Path) -> Option<String> {
        if let Some(text) = self.overlay.get(file) {
            return Some(text.clone());
        }
        self.texts
            .borrow_mut()
            .entry(file.to_path_buf())
            .or_insert_with(|| std::fs::read_to_string(self.root.join(file)).ok())
            .clone()
    }

    pub(super) fn locate(&self, file: &Path, steps: &[Step<'_>]) -> (Option<usize>, Option<usize>) {
        let Some(text) = self.text(file) else {
            return (None, None);
        };
        let Ok(top) = toml::de::DeTable::parse(&text) else {
            return (None, None);
        };
        let top = toml::Spanned::new(top.span(), toml::de::DeValue::Table(top.into_inner()));
        let mut at = &top;
        let mut found = None;
        for step in steps {
            match descend(at, *step) {
                Some(next) => {
                    at = next;
                    found = Some(next.span().start);
                }
                None => break,
            }
        }
        match found {
            Some(offset) => (Some(line_of(&text, offset)), Some(column_of(&text, offset))),
            None => (None, None),
        }
    }

    pub(super) fn error<T>(
        &self,
        file: &Path,
        steps: &[Step<'_>],
        message: impl Into<String>,
    ) -> Result<T, SceneError> {
        Err(self.refusal(file, steps, message))
    }

    pub(super) fn refusal(
        &self,
        file: &Path,
        steps: &[Step<'_>],
        message: impl Into<String>,
    ) -> SceneError {
        let (line, column) = self.locate(file, steps);
        SceneError {
            file: self.root.join(file),
            line,
            column,
            message: message.into(),
            related: Vec::new(),
        }
    }

    pub(super) fn holder(&self, files: &[PathBuf], key: &str) -> PathBuf {
        files
            .iter()
            .find(|file| {
                self.text(file).is_some_and(|text| {
                    toml::de::DeTable::parse(&text)
                        .is_ok_and(|table| table.get_ref().get(key).is_some())
                })
            })
            .cloned()
            .unwrap_or_default()
    }
}

pub(super) fn inner(key: &str) -> &str {
    key.rsplit('/').next().unwrap_or(key)
}

pub(super) fn diagnostic(root: &Path, diagnostic: &Diagnostic) -> SceneError {
    SceneError {
        file: root.join(&diagnostic.file),
        line: Some(diagnostic.line as usize),
        column: Some(diagnostic.column as usize),
        message: diagnostic.message.clone(),
        related: diagnostic
            .related
            .iter()
            .map(|place| Location {
                file: root.join(&place.file),
                line: place.line as usize,
                column: place.column as usize,
            })
            .collect(),
    }
}

pub(super) fn refused(root: &Path, diagnostics: &[Diagnostic]) -> SceneError {
    diagnostics
        .iter()
        .find(|found| found.is_error())
        .or_else(|| diagnostics.first())
        .map(|found| diagnostic(root, found))
        .unwrap_or_else(|| SceneError::new(root, None, "the scene does not load"))
}

pub(super) fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

pub fn project_root(path: &Path) -> PathBuf {
    Project::root_of(path)
}

pub(super) fn project(root: &Path) -> Result<Project, SceneError> {
    let folder = if root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        root
    };
    Project::open(folder).map_err(|found| diagnostic(root, &found))
}

pub(super) fn open(path: &Path, hour: Option<f64>) -> Result<Scene, SceneError> {
    let root = project_root(path);
    let project = project(&root)?;
    let absolute = absolute(path);
    let resolved = project
        .scene(&absolute)
        .map_err(|found| refused(&root, &found))?;
    build(path, &root, &resolved, hour, &BTreeMap::new())
}

pub(super) fn build(
    path: &Path,
    root: &Path,
    resolved: &pfx_scene::Scene,
    hour: Option<f64>,
    overlay: &BTreeMap<PathBuf, String>,
) -> Result<Scene, SceneError> {
    let places = Places::new(root, overlay);
    let mut reader = Reader {
        files: BTreeMap::new(),
        root,
        overlay,
    };
    for file in &resolved.files {
        reader
            .read(&root.join(file))
            .map_err(|message| SceneError::new(&root.join(file), None, message))?;
    }
    let library = build_library(&places, &resolved.materials)?;
    let mut meshes = BTreeMap::new();
    for entry in &resolved.meshes {
        meshes.insert(entry.key.clone(), build_mesh(&places, entry, &mut reader)?);
    }
    let sun = match &resolved.sun {
        Some(sun) => {
            let holder = places.holder(&resolved.files, "sun");
            Some(build_sun(&places, &holder, sun, hour)?)
        }
        None => None,
    };
    let sky = match &resolved.sky {
        Some(sky) => {
            let holder = places.holder(&resolved.files, "sky");
            Some(build_sky(&places, &holder, sky, sun.as_ref(), &mut reader)?)
        }
        None => None,
    };
    let mut contents = BTreeMap::new();
    for entry in &resolved.contents {
        contents.insert(
            entry.key.clone(),
            build_content(&places, entry, &mut reader)?,
        );
    }
    let mut texts = BTreeMap::new();
    for entry in &resolved.texts {
        texts.insert(entry.key.clone(), build_text(&places, entry, &mut reader)?);
    }
    let mut objects = build_objects(&places, &resolved.objects, &meshes)?;
    let animations = super::anim::build_animations(
        &places,
        &resolved.objects,
        &mut objects,
        &meshes,
        &mut contents,
        &mut reader,
    )?;
    place_texts(&mut texts, &objects);
    let movers = resolved
        .movers
        .iter()
        .map(|entry| (entry.key.clone(), build_mover(&entry.value)))
        .collect();
    let lights = resolved
        .lights
        .iter()
        .map(|entry| (entry.key.clone(), build_light(&entry.value)))
        .collect();
    let emitters = resolved
        .emitters
        .iter()
        .map(|entry| (entry.key.clone(), build_emitter(&entry.value)))
        .collect();
    let camera = match &resolved.camera {
        Some(camera) => {
            let holder = places.holder(&resolved.files, "camera");
            Some(build_camera(&places, &holder, camera)?)
        }
        None => None,
    };
    let finish = match &resolved.finish {
        Some(finish) => {
            let holder = places.holder(&resolved.files, "finish");
            Some(build_finish(&places, &holder, finish, &mut reader)?)
        }
        None => None,
    };
    let plates = match &resolved.plates {
        Some(plates) => {
            let holder = places.holder(&resolved.files, "plates");
            Some(super::plates::build(
                &places,
                &holder,
                plates,
                &resolved.proxies,
                &objects,
                &mut reader,
            )?)
        }
        None => None,
    };
    let bodies = super::play::build_bodies(&places, &resolved.objects, &objects, &meshes)?;
    let triggers = super::play::build_triggers(&places, &resolved.objects, &objects, &meshes)?;
    let body_layers = super::play::body_layers(&resolved.objects);
    let characters = super::play::build_characters(&places, &resolved.objects, &objects, &meshes)?;
    let mut sounds = BTreeMap::new();
    for entry in &resolved.sounds {
        sounds.insert(
            entry.key.clone(),
            super::play::build_sound(&places, entry, &mut reader)?,
        );
    }
    let mut scene = Scene {
        path: path.to_path_buf(),
        files: BTreeMap::new(),
        library,
        fallback: resolved.fallback.clone(),
        meshes,
        objects,
        sun,
        sky,
        haze: resolved.haze.as_ref().map(build_haze),
        movers,
        lights,
        emitters,
        camera,
        finish,
        trace: resolved
            .trace
            .map(|trace| TraceSettings {
                transmissive_shadows: trace.transmissive_shadows,
                clamp_indirect: trace.clamp_indirect.unwrap_or(file::Trace::CLAMP_INDIRECT),
                filter_glossy: trace.filter_glossy.unwrap_or(file::Trace::FILTER_GLOSSY),
            })
            .unwrap_or_default(),
        contents,
        texts,
        plates,
        physics: resolved.physics.as_ref().map(super::play::build_physics),
        bodies,
        body_layers,
        characters,
        animations,
        triggers,
        layers: resolved.layers.clone(),
        sounds,
        warnings: resolved
            .warnings
            .iter()
            .map(|found| diagnostic(root, found))
            .collect(),
    };
    super::tiles::expand(&mut scene, root, resolved, &places, overlay)?;
    scene.files.extend(reader.files);
    Ok(scene)
}

fn build_library(
    places: &Places,
    entries: &[Entry<file::Material>],
) -> Result<Library, SceneError> {
    let mut library = Library::new();
    for entry in entries {
        let steps = [Step::Key("materials"), Step::Key(inner(&entry.key))];
        let mut value = toml::Value::try_from(&entry.value)
            .map_err(|error| places.refusal(&entry.file, &steps, error.to_string()))?;
        if let Some(table) = value.as_table_mut() {
            table.remove("id");
            table.remove("authoring");
        }
        let text = toml::to_string(&value)
            .map_err(|error| places.refusal(&entry.file, &steps, error.to_string()))?;
        let material = Material::from_toml(&text).map_err(|message| {
            places.refusal(
                &entry.file,
                &steps,
                format!("material {}: {message}", entry.key),
            )
        })?;
        library
            .insert(&entry.key, material)
            .map_err(|message| places.refusal(&entry.file, &steps, message))?;
    }
    Ok(library)
}

fn build_haze(haze: &file::Haze) -> Haze {
    let amount = haze.amount.unwrap_or(0.0);
    Haze {
        lo: haze.lo,
        hi: haze.hi,
        fog: haze.fog.unwrap_or(amount / 6.0),
        smoke: haze.smoke.unwrap_or(amount * 20.0),
        mist: haze.mist.unwrap_or(amount * 0.2),
        floor: haze.floor.unwrap_or(amount * 2.0),
        phase: haze.phase.unwrap_or(0.3),
        back: haze.back.unwrap_or(0.5),
        gold: haze.gold.unwrap_or([1.0; 3]),
        ambient: haze.ambient.unwrap_or([0.03; 3]),
        reach: haze.reach.unwrap_or(3.0),
        unmapped: haze.unmapped.unwrap_or(0.0),
        seed: haze.seed.unwrap_or(0),
    }
}

fn build_mover(mover: &file::Mover) -> Mover {
    Mover {
        objects: mover.objects.clone(),
        kind: match mover.kind {
            file::MoverKind::Turn => MoverKind::Turn,
            file::MoverKind::Slide => MoverKind::Slide,
        },
        pivot: mover.pivot.unwrap_or([0.0; 3]),
        axis: super::unit(mover.axis),
        travel: mover.travel,
        period: mover.period.unwrap_or(file::Mover::PERIOD),
        motion: match mover.motion.unwrap_or_default() {
            file::Motion::Swing => Motion::Swing,
            file::Motion::Loop => Motion::Loop,
            file::Motion::Once => Motion::Once,
        },
        offset: mover.offset.unwrap_or(0.0),
        clip: mover.clip.clone(),
    }
}

pub(super) fn finite(values: &[f32]) -> bool {
    values.iter().all(|value| value.is_finite())
}

fn shadow(shadow: file::Shadow) -> Shadow {
    match shadow {
        file::Shadow::Cast => Shadow::Cast,
        file::Shadow::Only => Shadow::Only,
        file::Shadow::None => Shadow::None,
    }
}

fn hasher_add(hasher: &mut Sha256, values: impl IntoIterator<Item = f32>) {
    for value in values {
        hasher.update(value.to_le_bytes());
    }
}

fn build_mesh(
    places: &Places,
    entry: &Entry<file::Mesh>,
    reader: &mut Reader,
) -> Result<SceneMesh, SceneError> {
    let name = entry.key.as_str();
    let steps = [Step::Key("mesh"), Step::Key(inner(name))];
    let error = |message: String| places.refusal(&entry.file, &steps, message);
    let mesh = &entry.value;
    let file = places.root.join(&mesh.file);
    let bytes = reader.read(&file).map_err(error)?;
    let buffers: RefCell<Vec<(PathBuf, Vec<u8>)>> = RefCell::new(Vec::new());
    let directory = file.parent().unwrap_or_else(|| Path::new("")).to_path_buf();
    let parsed = Mesh::parse_with(&bytes, |uri| {
        let path = directory.join(uri);
        let data =
            std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        buffers.borrow_mut().push((path, data.clone()));
        Ok(data)
    })
    .map_err(|message| SceneError::new(&file, None, message))?;
    for (path, data) in buffers.into_inner() {
        reader.files.insert(path, sha256(&data));
    }
    let roots: Vec<(u32, bool)> = match &mesh.node {
        Some(node) => {
            let found: Vec<u32> = parsed
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| &n.name == node)
                .map(|(index, _)| index as u32)
                .collect();
            match found.as_slice() {
                [one] => vec![(*one, false)],
                [] => {
                    return Err(error(format!(
                        "mesh {name}: {} has no node {node}",
                        mesh.file
                    )));
                }
                _ => {
                    return Err(error(format!(
                        "mesh {name}: {} has {} nodes named {node}",
                        mesh.file,
                        found.len()
                    )));
                }
            }
        }
        None => parsed.roots.iter().map(|&root| (root, true)).collect(),
    };
    let overrides: BTreeMap<String, NodeOverride> = mesh
        .nodes
        .iter()
        .map(|(node, file)| {
            (
                node.clone(),
                NodeOverride {
                    material: file.material.clone(),
                    hidden: file.hidden,
                    shadow: file.shadow.map(shadow),
                    two_sided: file.two_sided,
                },
            )
        })
        .collect();
    let mut parts = Vec::new();
    let mut visited = BTreeSet::new();
    let mut rigs = Rigging::default();
    for (root, own) in roots {
        walk(
            &parsed,
            root,
            IDENTITY,
            own,
            &overrides,
            name,
            &mut parts,
            &mut visited,
            &mut rigs,
        )
        .map_err(error)?;
    }
    if rigs.skins.len() > 1 {
        return Err(error(format!(
            "mesh {name}: {} holds {} skins; a scene mesh takes one",
            mesh.file,
            rigs.skins.len()
        )));
    }
    let rig = match rigs.skins.into_values().next() {
        Some(found) => {
            for part in &mut parts {
                part.joint = None;
            }
            Some(Arc::new(found.rig))
        }
        None => {
            let found = parsed
                .node_rig(&rigs.nodes)
                .map_err(|message| error(format!("mesh {name}: {message}")))?;
            if found.is_none() {
                for part in &mut parts {
                    part.joint = None;
                }
            }
            found.map(Arc::new)
        }
    };
    for node in overrides.keys() {
        if !visited.contains(node) {
            return Err(places.refusal(
                &entry.file,
                &[
                    Step::Key("mesh"),
                    Step::Key(inner(name)),
                    Step::Key("nodes"),
                    Step::Key(node),
                ],
                format!("mesh {name}: the override names node {node}, which is not in this mesh"),
            ));
        }
    }
    if parts.is_empty() {
        return Err(error(format!(
            "mesh {name}: {} holds no triangles",
            mesh.file
        )));
    }
    let mut hasher = Sha256::new();
    for part in &parts {
        hasher.update(part.node.as_bytes());
        hasher.update([0]);
        hasher.update(part.geometry.hash);
        hasher_add(&mut hasher, part.transform.iter().flatten().copied());
        hasher.update(format!("{:?}{:?}", part.material, part.overrides).as_bytes());
    }
    if let Some(rig) = &rig {
        hasher.update(format!("{rig:?}").as_bytes());
    }
    Ok(SceneMesh {
        file,
        node: mesh.node.clone(),
        parts,
        rig,
        hash: hasher.finalize().into(),
    })
}

#[derive(Default)]
struct Rigging {
    skins: BTreeMap<u32, SkinRig>,
    nodes: Vec<(u32, bool)>,
}

#[allow(clippy::too_many_arguments)]
fn walk(
    mesh: &Mesh,
    index: u32,
    parent: Matrix,
    own: bool,
    overrides: &BTreeMap<String, NodeOverride>,
    name: &str,
    parts: &mut Vec<Part>,
    visited: &mut BTreeSet<String>,
    rigs: &mut Rigging,
) -> Result<(), String> {
    let node = &mesh.nodes[index as usize];
    visited.insert(node.name.clone());
    rigs.nodes.push((index, own));
    let joint = rigs.nodes.len() - 1;
    let transform = if own {
        multiply(parent, node.transform)
    } else {
        parent
    };
    let node_override = overrides.get(&node.name).cloned().unwrap_or_default();
    if let Some(group) = node.group {
        for primitive in &mesh.groups[group as usize].primitives {
            let mut geometry = geometry(primitive)
                .map_err(|message| format!("mesh {name}, node {}: {message}", node.name))?;
            let mut placed = transform;
            if let (Some(skin), Some(joints), Some(weights)) =
                (node.skin, &primitive.joints, &primitive.weights)
            {
                if let std::collections::btree_map::Entry::Vacant(entry) = rigs.skins.entry(skin) {
                    entry.insert(mesh.rig(skin).map_err(|message| {
                        format!("mesh {name}, node {}: {message}", node.name)
                    })?);
                }
                let mut weights = weights.clone();
                normalize_weights(&mut weights);
                let joints = rigs.skins[&skin]
                    .joints(joints, &weights)
                    .map_err(|message| format!("mesh {name}, node {}: {message}", node.name))?;
                geometry = geometry.with_skin(GeometrySkin { joints, weights });
                placed = IDENTITY;
            }
            parts.push(Part {
                node: node.name.clone(),
                joint: Some(joint),
                transform: placed,
                geometry: Arc::new(geometry),
                material: primitive
                    .material
                    .map(|material| mesh.materials[material as usize].name.clone()),
                overrides: node_override.clone(),
            });
        }
    }
    for &child in &node.children {
        walk(
            mesh, child, transform, true, overrides, name, parts, visited, rigs,
        )?;
    }
    Ok(())
}

pub(super) fn geometry(primitive: &Primitive) -> Result<Geometry, String> {
    let count = primitive.positions.len();
    if !primitive.indices.len().is_multiple_of(3) {
        return Err("the index count is not a multiple of three".into());
    }
    if primitive
        .indices
        .iter()
        .any(|&index| index as usize >= count)
    {
        return Err("an index is past the last vertex".into());
    }
    if !finite(&primitive.positions.concat()) {
        return Err("a position is not finite".into());
    }
    let normals = if primitive.normals.is_empty() {
        smooth_normals(&primitive.positions, &primitive.indices)
    } else {
        primitive.normals.clone()
    };
    let tangents = if primitive.tangents.is_empty() {
        vec![[1.0, 0.0, 0.0, 1.0]; count]
    } else {
        primitive.tangents.clone()
    };
    let uvs = if primitive.uvs.is_empty() {
        vec![[0.0; 2]; count]
    } else {
        primitive.uvs.clone()
    };
    Ok(Geometry::new(
        primitive.positions.clone(),
        normals,
        tangents,
        uvs,
        primitive.uvs1.clone(),
        primitive.indices.clone(),
    ))
}

impl Geometry {
    pub fn new(
        positions: Vec<[f32; 3]>,
        normals: Vec<[f32; 3]>,
        tangents: Vec<[f32; 4]>,
        uvs: Vec<[f32; 2]>,
        uvs1: Option<Vec<[f32; 2]>>,
        indices: Vec<u32>,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update((positions.len() as u64).to_le_bytes());
        hasher_add(&mut hasher, positions.iter().flatten().copied());
        hasher_add(&mut hasher, normals.iter().flatten().copied());
        hasher_add(&mut hasher, tangents.iter().flatten().copied());
        hasher_add(&mut hasher, uvs.iter().flatten().copied());
        if let Some(uvs1) = &uvs1 {
            hasher.update([1]);
            hasher_add(&mut hasher, uvs1.iter().flatten().copied());
        }
        for index in &indices {
            hasher.update(index.to_le_bytes());
        }
        Self {
            positions,
            normals,
            tangents,
            uvs,
            uvs1,
            indices,
            skin: None,
            hash: hasher.finalize().into(),
        }
    }

    pub fn with_skin(mut self, skin: GeometrySkin) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(self.hash);
        hasher.update([2]);
        for joints in &skin.joints {
            for joint in joints {
                hasher.update(joint.to_le_bytes());
            }
        }
        hasher_add(&mut hasher, skin.weights.iter().flatten().copied());
        self.hash = hasher.finalize().into();
        self.skin = Some(Arc::new(skin));
        self
    }
}

fn smooth_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut normals = vec![[0.0_f32; 3]; positions.len()];
    for triangle in indices.chunks_exact(3) {
        let [a, b, c] = [0, 1, 2].map(|k| positions[triangle[k] as usize]);
        let face = super::cross(super::sub(b, a), super::sub(c, a));
        for &index in triangle {
            for axis in 0..3 {
                normals[index as usize][axis] += face[axis];
            }
        }
    }
    normals
        .into_iter()
        .map(|normal| {
            let unit = super::unit(normal);
            if super::dot(unit, unit) > 0.5 {
                unit
            } else {
                [0.0, 1.0, 0.0]
            }
        })
        .collect()
}

fn build_objects(
    places: &Places,
    entries: &[Entry<file::Object>],
    meshes: &BTreeMap<String, SceneMesh>,
) -> Result<Vec<Object>, SceneError> {
    let mut objects = Vec::new();
    let mut local = Vec::new();
    for (place, entry) in entries.iter().enumerate() {
        let value = &entry.value;
        let name = entry.key.as_str();
        let steps = [Step::Named("object", inner(name))];
        let mesh = value.mesh.clone().unwrap_or_default();
        if let Some(found) = meshes.get(&mesh) {
            for node in value.materials.keys() {
                if !found.parts.iter().any(|part| &part.node == node) {
                    return places.error(
                        &entry.file,
                        &[
                            Step::Named("object", inner(name)),
                            Step::Key("materials"),
                            Step::Key(node),
                        ],
                        format!(
                            "object {name}: materials names node {node}, which mesh {mesh} has not"
                        ),
                    );
                }
            }
        } else if value.prefab.is_none() {
            return places.error(
                &entry.file,
                &steps,
                format!("object {name} names mesh {mesh}, which the scene has not"),
            );
        }
        let scale = value.scale_axes();
        local.push(model(value.at, value.rotate, scale));
        objects.push(Object {
            name: entry.key.clone(),
            mesh,
            at: value.at,
            rotate: value.rotate,
            scale,
            parent: value.parent.clone(),
            model: IDENTITY,
            material: value.material.clone(),
            materials: value.materials.clone(),
            shadow: shadow(value.shadow.unwrap_or_default()),
            two_sided: value.two_sided,
            hidden: value.hidden,
            clip: value.clip.clone(),
            content: value.content.clone(),
            id: value.pick.unwrap_or(place as u32 + 1),
            alpha_cutoff: value.alpha_cutoff.unwrap_or(ALPHA_CUTOFF),
            face_camera: value.face_camera,
            dynamic: value.dynamic,
        });
    }
    let index: BTreeMap<String, usize> = objects
        .iter()
        .enumerate()
        .map(|(place, object)| (object.name.clone(), place))
        .collect();
    for place in 0..objects.len() {
        let mut matrix = local[place];
        let mut at = place;
        let mut steps = 0;
        while let Some(parent) = &objects[at].parent {
            steps += 1;
            let Some(&above) = index.get(parent) else {
                break;
            };
            if steps > objects.len() {
                let entry = &entries[place];
                return places.error(
                    &entry.file,
                    &[Step::Named("object", inner(&entry.key))],
                    format!("object {}: its parents form a cycle", entry.key),
                );
            }
            at = above;
            matrix = multiply(local[at], matrix);
        }
        objects[place].model = matrix;
    }
    Ok(objects)
}

fn build_light(light: &file::Light) -> Light {
    Light {
        position: light.position,
        color: light.color.unwrap_or(file::Light::COLOR),
        intensity: light.intensity,
        radius: light.radius.unwrap_or(file::Light::RADIUS),
        range: light.range.unwrap_or(file::Light::RANGE),
        shadow: light.shadow.unwrap_or(file::Light::SHADOW),
    }
}

fn build_emitter(emitter: &file::Emitter) -> Emitter {
    Emitter {
        position: emitter.position,
        radius: emitter.radius,
        color: emitter.color.unwrap_or([1.0; 3]),
        intensity: emitter.intensity,
    }
}

fn build_sun(
    places: &Places,
    holder: &Path,
    sun: &file::Sun,
    hour: Option<f64>,
) -> Result<Sun, SceneError> {
    let steps = [Step::Key("sun")];
    let hour = hour.or(sun.hour);
    let radius = sun.radius.unwrap_or(0.0);
    let reference_hour = sun.reference_hour.unwrap_or(REFERENCE_HOUR);
    let daylight = |hour: Option<f64>| {
        let fields = [
            ("hour", hour),
            ("day", sun.day),
            ("latitude", sun.latitude),
            ("heading", sun.heading),
        ]
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, Reading::Number(value))));
        Daylight::parse(fields).map_err(|message| places.refusal(holder, &steps, message))
    };
    match sun.model {
        file::SunModel::Daylight => {
            let daylight = daylight(hour)?;
            let light = daylight.light(reference_hour);
            Ok(Sun {
                direction: daylight.sun().y_up.map(|value| value as f32),
                color: light.colour.map(|value| value as f32),
                intensity: light.intensity as f32,
                hour: daylight.hour as f32,
                daylight: Some(daylight),
                reference_hour,
                radius,
            })
        }
        file::SunModel::Authored => {
            let (Some(toward), Some(irradiance)) = (sun.toward, sun.irradiance) else {
                return places.error(
                    holder,
                    &steps,
                    "an authored sun needs toward and irradiance",
                );
            };
            let color = sun.color.unwrap_or([1.0; 3]);
            let daylight = daylight(Some(hour.unwrap_or(reference_hour)))?;
            if daylight.hour == reference_hour {
                return Ok(Sun {
                    direction: super::unit(toward),
                    color,
                    intensity: irradiance,
                    hour: daylight.hour as f32,
                    daylight: None,
                    reference_hour,
                    radius,
                });
            }
            let peak = color.into_iter().fold(0.0_f32, f32::max);
            let followed = daylight.authored(
                &Authored {
                    toward: toward.map(f64::from),
                    colour: color.map(f64::from),
                    sky_fill: 0.0,
                },
                reference_hour,
            );
            Ok(Sun {
                direction: super::unit(followed.toward.map(|value| value as f32)),
                color: followed.colour.map(|value| value as f32 * peak),
                intensity: irradiance * followed.intensity as f32,
                hour: daylight.hour as f32,
                daylight: None,
                reference_hour,
                radius,
            })
        }
    }
}

fn kind_name(kind: file::SkyKind) -> &'static str {
    match kind {
        file::SkyKind::Analytic => "analytic",
        file::SkyKind::Hdr => "hdr",
        file::SkyKind::Room => "room",
        file::SkyKind::Mix => "mix",
    }
}

fn room_of(room: &file::Room) -> RoomSky {
    RoomSky {
        width: room.width,
        floor: room.floor,
        wall: room.wall,
        ceiling: room.ceiling,
        lights: room
            .lights
            .iter()
            .map(|lamp| RoomLamp {
                name: lamp.name.clone(),
                az: lamp.az,
                el: lamp.el,
                power: lamp.power,
                color: lamp.color,
                width: lamp.width,
                height: lamp.height,
                soft: lamp.soft,
                slats: lamp.slats,
                open: lamp.open,
            })
            .collect(),
    }
}

fn build_sky(
    places: &Places,
    holder: &Path,
    sky: &file::Sky,
    sun: Option<&Sun>,
    reader: &mut Reader,
) -> Result<SceneSky, SceneError> {
    let steps = [Step::Key("sky")];
    let mut hasher = Sha256::new();
    hasher.update(kind_name(sky.kind).as_bytes());
    let rotation = sky.rotation_deg.unwrap_or(0.0);
    let intensity = sky.intensity.unwrap_or(file::Sky::INTENSITY);
    let environment = match sky.kind {
        file::SkyKind::Analytic => {
            let turbidity = sky.turbidity.unwrap_or(file::Sky::TURBIDITY);
            let albedo = sky.ground_albedo.unwrap_or(file::Sky::GROUND_ALBEDO);
            let mut analytic = match sun
                .and_then(|sun| sun.daylight.map(|daylight| (daylight, sun.reference_hour)))
            {
                Some((daylight, reference)) => {
                    AnalyticSky::new(daylight, reference, turbidity, albedo)
                }
                None => {
                    let sun = sun.copied().unwrap_or_else(Sun::dark);
                    AnalyticSky {
                        sun: sun.direction,
                        sun_colour: sun.color,
                        sun_intensity: sun.intensity,
                        ambient: 1.0,
                        turbidity,
                        ground_albedo: albedo.map(|v| v.clamp(0.0, 1.0)),
                    }
                }
            };
            if let Some(ambient) = sky.ambient {
                analytic.ambient = ambient;
            }
            hasher_add(
                &mut hasher,
                analytic
                    .sun
                    .into_iter()
                    .chain(analytic.sun_colour)
                    .chain([analytic.sun_intensity, analytic.ambient, analytic.turbidity])
                    .chain(analytic.ground_albedo),
            );
            Environment::Analytic(analytic)
        }
        file::SkyKind::Hdr => {
            let Some(relative) = &sky.path else {
                return places.error(holder, &steps, "an hdr sky needs a path");
            };
            let path = places.root.join(relative);
            let bytes = reader
                .read(&path)
                .map_err(|message| places.refusal(holder, &steps, message))?;
            hasher.update(sha256(&bytes));
            let mut parsed =
                Sky::parse(&bytes).map_err(|message| SceneError::new(&path, None, message))?;
            if let Some(prepare) = sky.prepare {
                hasher.update(
                    format!(
                        "PrepareFile {{ cap: {:?}, balance: {:?}, mean: {:?} }}",
                        prepare.cap, prepare.balance, prepare.mean
                    )
                    .as_bytes(),
                );
                if let Some(cap) = prepare.cap {
                    parsed = parsed.capped_luminance(cap);
                }
                if let Some(balance) = prepare.balance {
                    parsed = parsed.balanced_to(balance);
                }
                if let Some(mean) = prepare.mean {
                    parsed = parsed.scaled_to_mean_luminance(mean);
                }
            }
            hasher_add(&mut hasher, [rotation, intensity]);
            Environment::Hdr(Arc::new(
                parsed.rotated(-rotation.to_radians()).exposed(intensity),
            ))
        }
        file::SkyKind::Room => {
            let Some(room) = &sky.room else {
                return places.error(holder, &steps, "a room sky needs a [sky.room] table");
            };
            let room = room_of(room);
            room.check()
                .map_err(|message| places.refusal(holder, &steps, message))?;
            hasher.update(format!("{room:?}").as_bytes());
            hasher_add(&mut hasher, [rotation, intensity]);
            Environment::Hdr(Arc::new(
                Sky::room(&room)
                    .rotated(-rotation.to_radians())
                    .exposed(intensity),
            ))
        }
        file::SkyKind::Mix => return build_mix(places, holder, sky, sun, reader),
    };
    Ok(SceneSky {
        kind: kind_name(sky.kind).into(),
        environment,
        hash: hasher.finalize().into(),
    })
}

fn build_mix(
    places: &Places,
    holder: &Path,
    sky: &file::Sky,
    sun: Option<&Sun>,
    reader: &mut Reader,
) -> Result<SceneSky, SceneError> {
    let mut hasher = Sha256::new();
    hasher.update(b"mix");
    let mut layers = Vec::new();
    for layer in &sky.layer {
        let weight = layer.weight.unwrap_or(file::Sky::WEIGHT);
        let built = build_sky(places, holder, layer, sun, reader)?;
        hasher.update(built.hash);
        hasher_add(&mut hasher, [weight]);
        layers.push((weight, built.environment));
    }
    Ok(SceneSky {
        kind: "mix".into(),
        environment: Environment::Hdr(Arc::new(super::mix::bake(&layers))),
        hash: hasher.finalize().into(),
    })
}

fn build_camera(
    places: &Places,
    holder: &Path,
    camera: &file::Camera,
) -> Result<Camera, SceneError> {
    let steps = [Step::Key("camera")];
    let base = Camera::default();
    let projection = match camera.projection.unwrap_or_default() {
        file::Projection::Perspective => {
            let fov = match (camera.fov, camera.focal) {
                (Some(fov), _) => fov,
                (None, Some(focal)) => {
                    let sensor = camera.sensor.unwrap_or(file::Camera::SENSOR);
                    2.0 * (sensor / (2.0 * focal)).atan().to_degrees()
                }
                (None, None) => file::Camera::FOV,
            };
            Projection::Perspective { fov }
        }
        file::Projection::Orthographic => {
            let Some(height) = camera.height else {
                return places.error(
                    holder,
                    &steps,
                    "an orthographic camera needs a positive height",
                );
            };
            Projection::Orthographic { height }
        }
    };
    let mut built = Camera {
        projection,
        at: camera.at.unwrap_or(base.at),
        look_at: camera.look_at.unwrap_or(base.look_at),
        up: camera.up.unwrap_or(base.up),
        near: camera.near.unwrap_or(base.near),
        far: camera.far.unwrap_or(base.far),
        shift: camera.shift.unwrap_or([0.0; 2]),
        depth_of_field: None,
        preset: preset(camera.preset.as_ref()),
    };
    if let (Projection::Perspective { fov }, Some(fstop)) = (built.projection, camera.fstop) {
        let distance = camera.focus.unwrap_or_else(|| {
            super::dot(
                super::sub(built.look_at, built.at),
                super::sub(built.look_at, built.at),
            )
            .sqrt()
        });
        let sensor = camera.sensor.unwrap_or(file::Camera::SENSOR);
        let focal = camera
            .focal
            .unwrap_or_else(|| sensor / (2.0 * (fov.to_radians() * 0.5).tan()))
            / 1000.0;
        if !(distance.is_finite() && distance > focal) {
            return places.error(
                holder,
                &steps,
                "camera fstop must be positive and focus beyond the focal length",
            );
        }
        built.depth_of_field = Some(DepthOfField {
            fstop,
            distance,
            focal,
        });
    }
    Ok(built)
}

fn preset(file: Option<&file::Preset>) -> Preset {
    let mut preset = Preset::default();
    let Some(file) = file else {
        return preset;
    };
    let set = |target: &mut f32, value: Option<f32>, scale: f32| {
        if let Some(value) = value {
            *target = value * scale;
        }
    };
    set(&mut preset.gain, file.gain, 1.0);
    if let Some(orbit) = file.orbit {
        set(&mut preset.orbit.stiffness, orbit.stiffness, 1.0);
        set(&mut preset.orbit.yaw, orbit.yaw, DEG);
        set(&mut preset.orbit.pitch, orbit.pitch, DEG);
        set(&mut preset.orbit.hold, orbit.hold, 1.0);
    }
    if let Some(look) = file.look {
        set(&mut preset.look.stiffness, look.stiffness, 1.0);
        set(&mut preset.look.lean, look.lean, 1.0);
        set(&mut preset.look.yaw, look.yaw, DEG);
        set(&mut preset.look.pitch, look.pitch, DEG);
        set(&mut preset.look.sign, look.sign, 1.0);
    }
    if let Some(zoom) = file.zoom {
        set(&mut preset.zoom.stiffness, zoom.stiffness, 1.0);
        set(&mut preset.zoom.hold, zoom.hold, 1.0);
        set(&mut preset.zoom.max, zoom.max, 1.0);
        set(&mut preset.zoom.rate, zoom.rate, 1.0);
    }
    if let Some(wander) = file.wander {
        set(&mut preset.wander.idle, wander.idle, 1.0);
        set(&mut preset.wander.every, wander.every, 1.0);
        set(&mut preset.wander.hold, wander.hold, 1.0);
    }
    if let Some(nudge) = file.nudge {
        set(&mut preset.nudge.stiffness, nudge.stiffness, 1.0);
        set(&mut preset.nudge.pitch, nudge.pitch, DEG);
        set(&mut preset.nudge.hold, nudge.hold, 1.0);
        set(&mut preset.nudge.floor, nudge.floor, 1.0);
    }
    if let Some(file::Hush {
        floor: Some(floor),
        stiffness: Some(stiffness),
        blocks_cursor: Some(blocks_cursor),
    }) = file.hush
    {
        preset.hush = Some(Hush {
            floor,
            stiffness,
            blocks_cursor,
        });
    }
    if let Some(file::Depth {
        focus: Some(focus),
        blur: Some(blur),
        floor: Some(floor),
    }) = file.depth
    {
        preset.depth = Some(Depth { focus, blur, floor });
    }
    preset
}

fn pass_line(text: &str, place: usize) -> Option<usize> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| {
            line.split('#')
                .next()
                .unwrap_or("")
                .replace([' ', '\t'], "")
                == "[[finish.pass]]"
        })
        .nth(place)
        .map(|(number, _)| number + 1)
}

fn build_finish(
    places: &Places,
    holder: &Path,
    finish: &file::Finish,
    reader: &mut Reader,
) -> Result<Finish, SceneError> {
    let steps = [Step::Key("finish")];
    let text = places.text(holder).unwrap_or_default();
    let raw: toml::Table = toml::from_str::<toml::Table>(&text)
        .ok()
        .and_then(|mut top| top.remove("finish"))
        .and_then(|value| value.as_table().cloned())
        .unwrap_or_default();
    let mut hasher = Sha256::new();
    let mut base = raw.clone();
    let passes = match base.remove("pass") {
        Some(toml::Value::Array(items)) => items,
        _ => Vec::new(),
    };
    let mut chain = match &finish.file {
        Some(relative) => {
            let path = places.root.join(relative);
            let bytes = reader
                .read(&path)
                .map_err(|message| places.refusal(holder, &steps, message))?;
            hasher.update(sha256(&bytes));
            let text = String::from_utf8(bytes)
                .map_err(|_| SceneError::new(&path, None, "the finish is not UTF-8"))?;
            pfx_post::parse_str(&text)
                .map_err(|error| SceneError::new(&path, None, error.to_string()))?
        }
        None => {
            hasher.update(base.to_string().as_bytes());
            pfx_post::parse(&base)
                .map_err(|error| places.refusal(holder, &steps, format!("[finish]: {error}")))?
        }
    };
    for (place, item) in passes.iter().enumerate() {
        let refuse = |message: String| {
            let mut error = places.refusal(holder, &steps, message);
            if let Some(line) = pass_line(&text, place) {
                error.line = Some(line);
                error.column = Some(1);
            }
            error
        };
        let Some(pass) = item.as_table() else {
            return Err(refuse("a [[finish.pass]] is a table of finish keys".into()));
        };
        let built =
            pfx_post::parse(pass).map_err(|error| refuse(format!("[[finish.pass]]: {error}")))?;
        if built.passes.is_empty() {
            return Err(refuse("a [[finish.pass]] builds no pass".into()));
        }
        hasher.update(b"pass");
        hasher.update(pass.to_string().as_bytes());
        chain.passes.extend(built.passes);
    }
    Ok(Finish {
        chain,
        hash: hasher.finalize().into(),
    })
}

fn build_content(
    places: &Places,
    entry: &Entry<file::Content>,
    reader: &mut Reader,
) -> Result<Content, SceneError> {
    let steps = [Step::Key("content"), Step::Key(inner(&entry.key))];
    let path = places.root.join(&entry.value.image);
    let bytes = reader
        .read(&path)
        .map_err(|message| places.refusal(&entry.file, &steps, message))?;
    content_from(path.clone(), &bytes).map_err(|message| SceneError::new(&path, None, message))
}

pub(super) fn content_from(path: PathBuf, bytes: &[u8]) -> Result<Content, String> {
    let image = Image::png(bytes)?;
    let rgba = match image.pixels {
        Pixels::Eight(bytes) => bytes,
        Pixels::Sixteen(values) => values.into_iter().map(|value| (value >> 8) as u8).collect(),
    };
    Ok(Content {
        image: path,
        width: image.width,
        height: image.height,
        srgb: image.space == ColorSpace::Srgb,
        rgba: Arc::new(rgba),
        hash: sha256(bytes),
    })
}

fn place_texts(texts: &mut BTreeMap<String, Text>, objects: &[Object]) {
    for (key, text) in texts.iter_mut() {
        let Some((placement, _)) = key.rsplit_once('/') else {
            continue;
        };
        if let Some(object) = objects.iter().find(|object| object.name == placement) {
            text.place = object.model;
        }
    }
}

fn build_text(
    places: &Places,
    entry: &Entry<file::Text>,
    reader: &mut Reader,
) -> Result<Text, SceneError> {
    let steps = [Step::Key("text"), Step::Key(inner(&entry.key))];
    let text = &entry.value;
    let path = places.root.join(&text.font);
    let bytes = reader
        .read(&path)
        .map_err(|message| places.refusal(&entry.file, &steps, message))?;
    let font = Font::parse(&bytes).map_err(|message| SceneError::new(&path, None, message))?;
    let Some(family) = font.families.first().cloned() else {
        return Err(SceneError::new(&path, None, "the font names no family"));
    };
    Ok(Text {
        text: super::filled(&text.text),
        font: path,
        font_bytes: Arc::new(bytes),
        family,
        lit: text.lit,
        size: text.size,
        color: text.color.unwrap_or(file::Text::COLOR),
        at: text.at,
        rotate: text.rotate,
        place: IDENTITY,
        dynamic: text.dynamic,
    })
}
