use crate::pieces::{self, Rules, Step};
use crate::skin::Skin;
use crate::{Error, holes, maps, rotation};
use glam::{DMat4, DVec3};
use pfx_mesh::outline::{self, Outline as Ring, Shape};
use pfx_mesh::{
    self as mesh, Bend, Mesh, Morph, Pocket, Profile, ProfileKind, Sheet, Slot, UvMode,
};
use pfx_recipe::{self as recipe, BendForm, Caps, Kind, Object, Outline, Uv};
use std::path::Path;

const CAPSULE_SEGMENTS: u32 = 32;
const CAPSULE_RINGS: u32 = 8;
const CAP_NORMAL: f64 = std::f64::consts::FRAC_1_SQRT_2;

pub(crate) fn fail(object: &Object, key: &str, text: impl Into<String>) -> Error {
    Error::new(format!("object[{}].{key}", object.name), text)
}

const EDGE: f64 = 1e-6;

const HOLE: &str = "the outline has a hole, and a sweep or a pocket follows one contour";

fn profile(p: &recipe::Profile) -> Profile {
    Profile {
        kind: match p.kind {
            recipe::ProfileKind::Round => ProfileKind::Round,
            recipe::ProfileKind::Chamfer => ProfileKind::Chamfer,
            recipe::ProfileKind::None => ProfileKind::None,
        },
        width: p.width,
        segments: p.segments,
    }
}

pub(crate) fn shapes(o: &Outline) -> Vec<Shape> {
    match o {
        Outline::Rect {
            rect,
            radius,
            turned,
            segments,
        } => {
            let ring = if *turned == 0.0 {
                outline::rounded_rect(*rect, *radius, *segments)
            } else {
                outline::turned(*rect, *radius, *turned, *segments)
            };
            vec![Shape::new(ring)]
        }
        Outline::Circle {
            centre,
            radius,
            segments,
        } => vec![Shape::new(outline::circle(*centre, *radius, *segments))],
        Outline::Points { points, segments } => {
            vec![Shape::new(outline::fillet(points, *segments))]
        }
        Outline::Union(parts) => {
            let parts: Vec<Vec<Shape>> = parts.iter().map(shapes).collect();
            let outers: Vec<Ring> = parts.iter().flatten().map(|s| s.outer.clone()).collect();
            let mut out = outline::union(&outers);
            for (i, part) in parts.iter().enumerate() {
                if part.iter().all(|s| s.holes.is_empty()) {
                    continue;
                }
                let others: Vec<Ring> = parts
                    .iter()
                    .enumerate()
                    .filter(|&(j, _)| j != i)
                    .flat_map(|(_, p)| p.iter().map(|s| s.outer.clone()))
                    .collect();
                for hole in part.iter().flat_map(|s| s.holes.iter()) {
                    let open = outline::difference(&Shape::new(hole.clone()), &others);
                    if open.iter().any(|s| !s.holes.is_empty()) {
                        let every: Vec<Shape> = parts.iter().flatten().cloned().collect();
                        return outline::merge(&every);
                    }
                    let open: Vec<Ring> = open.into_iter().map(|s| s.outer).collect();
                    if !open.is_empty() {
                        out = out
                            .iter()
                            .flat_map(|s| outline::difference(s, &open))
                            .collect();
                    }
                }
            }
            out
        }
        Outline::Difference(a, b) => {
            let cut: Vec<Ring> = shapes(b).into_iter().map(|s| s.outer).collect();
            shapes(a)
                .iter()
                .flat_map(|s| outline::difference(s, &cut))
                .collect()
        }
    }
}

fn single(object: &Object, o: &Outline) -> Result<Ring, Error> {
    let shape = one(object, shapes(o))?;
    if !shape.holes.is_empty() {
        return Err(fail(object, "outline", HOLE));
    }
    Ok(shape.outer)
}

fn one(object: &Object, found: Vec<Shape>) -> Result<Shape, Error> {
    let [shape] = <[Shape; 1]>::try_from(found).map_err(|found| {
        fail(
            object,
            "outline",
            format!(
                "the outline makes {} separate shapes, expected one",
                found.len()
            ),
        )
    })?;
    Ok(shape)
}

fn cut_ring(pocket: &Object, cut: &Outline, into: &str, target: &Shape) -> Result<Ring, Error> {
    let ring = single(pocket, cut)?;
    let area = ring.area().abs();
    let mine = Shape::new(ring.clone());
    let mut outside: f64 = outline::difference(&mine, std::slice::from_ref(&target.outer))
        .iter()
        .map(Shape::area)
        .sum();
    for hole in &target.holes {
        outside += outline::intersection(&mine, &Shape::new(hole.clone()))
            .iter()
            .map(Shape::area)
            .sum::<f64>();
    }
    if outside > area * EDGE {
        return Err(fail(
            pocket,
            "outline",
            format!(
                "the pocket reaches past the edge of `{into}`, with {:.1}% of its area outside it",
                100.0 * outside / area
            ),
        ));
    }
    Ok(ring)
}

pub(crate) fn refusals(object: &Object, pockets: &[&Object]) -> Vec<Error> {
    match &object.kind {
        Kind::Outline {
            outline: o,
            holes,
            profile: p,
            ..
        } => {
            let found = match holes::solid(object, o, holes, p) {
                Ok(found) => found,
                Err(e) => return vec![e],
            };
            if pockets.is_empty() {
                return Vec::new();
            }
            match one(object, found) {
                Err(e) => vec![e],
                Ok(shape) => pockets
                    .iter()
                    .filter_map(|p| match &p.kind {
                        Kind::Pocket { outline: cut, .. } => {
                            cut_ring(p, cut, &object.name, &shape).err()
                        }
                        _ => None,
                    })
                    .collect(),
            }
        }
        Kind::Sweep {
            along: Some(along), ..
        } => single(object, &along.outline).err().into_iter().collect(),
        _ => Vec::new(),
    }
}

fn uv_mode(object: &Object) -> UvMode {
    match object.uv {
        Some(Uv::None) => UvMode::None,
        _ => UvMode::Local,
    }
}

fn card(size: [f64; 2]) -> Mesh {
    let mut m = mesh::grid(size, [1, 1], None);
    if let Some(uvs) = &mut m.uvs {
        for uv in uvs {
            uv[1] = 1.0 - uv[1];
        }
    }
    mesh::transform(&mut m, rotation([90.0, 0.0, 0.0]));
    m
}

fn target_names(m: &gltf::Mesh) -> Vec<String> {
    m.extras()
        .as_ref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw.get()).ok())
        .and_then(|extras| {
            extras.get("targetNames").and_then(|names| {
                names.as_array().map(|list| {
                    list.iter()
                        .map(|n| n.as_str().unwrap_or("").to_string())
                        .collect()
                })
            })
        })
        .unwrap_or_default()
}

type Target = (Vec<[f32; 3]>, Vec<[f32; 3]>);

fn join(
    morph: &mut Morph,
    names: &[String],
    weights: Option<&[f32]>,
    targets: Vec<Target>,
    matrix: DMat4,
    before: usize,
    count: usize,
) {
    let (positions, normals) = targets.into_iter().unzip();
    let mut part = Morph {
        positions,
        normals,
        ..Morph::default()
    };
    part.transform(matrix);
    for (i, (deltas, turns)) in part.positions.into_iter().zip(part.normals).enumerate() {
        let name = names
            .get(i)
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| i.to_string());
        let at = match morph.names.iter().position(|n| *n == name) {
            Some(at) => at,
            None => {
                morph.names.push(name);
                morph
                    .weights
                    .push(weights.and_then(|w| w.get(i)).copied().unwrap_or(0.0));
                morph.positions.push(Vec::new());
                morph.normals.push(Vec::new());
                morph.names.len() - 1
            }
        };
        let list = &mut morph.positions[at];
        if list.len() <= before {
            list.resize(before, [0.0; 3]);
            list.extend(deltas);
            let list = &mut morph.normals[at];
            list.resize(before, [0.0; 3]);
            list.extend(turns);
        }
    }
    for list in morph.positions.iter_mut().chain(morph.normals.iter_mut()) {
        list.resize(before + count, [0.0; 3]);
    }
}

pub(crate) struct Source<'a> {
    pub object: &'a Object,
    pub path: &'a str,
    pub folder: &'a Path,
    pub buffers: &'a [gltf::buffer::Data],
}

pub(crate) fn read_mesh(
    source: &Source,
    m: &gltf::Mesh,
    (matrix, chosen): (DMat4, Option<&str>),
    out: &mut Mesh,
    morph: &mut Morph,
    looks: &mut pfx_glb::Looks,
) -> Result<(), Error> {
    let Source {
        object,
        path,
        folder,
        buffers,
    } = *source;
    let names = target_names(m);
    for primitive in m.primitives() {
        if primitive.mode() != gltf::mesh::Mode::Triangles {
            continue;
        }
        let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
        let Some(positions) = reader.read_positions() else {
            continue;
        };
        let positions: Vec<[f32; 3]> = positions.collect();
        let count = positions.len();
        let indices: Vec<u32> = match reader.read_indices() {
            Some(i) => i
                .into_u32()
                .collect::<Vec<u32>>()
                .chunks_exact(3)
                .filter(|t| t.iter().all(|&v| (v as usize) < count))
                .flatten()
                .copied()
                .collect(),
            None => (0..count as u32).collect(),
        };
        if indices.is_empty() {
            continue;
        }
        let uvs = reader
            .read_tex_coords(0)
            .map(|t| t.into_f32().collect::<Vec<[f32; 2]>>())
            .filter(|uvs| uvs.len() == count);
        let sized = |list: Option<Vec<[f32; 3]>>| {
            list.filter(|d| d.len() == count)
                .unwrap_or_else(|| vec![[0.0; 3]; count])
        };
        let targets: Vec<Target> = reader
            .read_morph_targets()
            .map(|(deltas, turns, _)| {
                (
                    sized(deltas.map(Iterator::collect)),
                    sized(turns.map(Iterator::collect)),
                )
            })
            .collect();
        let mut part = Mesh {
            normals: sized(reader.read_normals().map(Iterator::collect)),
            positions,
            uvs,
            uvs1: None,
            indices,
            slots: Vec::new(),
        };
        let material = primitive.material();
        let name = match chosen {
            Some(name) => name.to_string(),
            None => crate::library::material_name(material.name(), material.index()),
        };
        if chosen.is_none() && material.index().is_some() && !looks.contains_key(&name) {
            let look = maps::look(&material, buffers, folder)
                .map_err(|e| fail(object, "path", format!("{path}: material `{name}`: {e}")))?;
            looks.insert(name.clone(), look);
        }
        part.set_material(&name);
        mesh::transform(&mut part, matrix);
        let before = out.positions.len();
        out.append(&part);
        if !targets.is_empty() || !morph.is_empty() {
            join(morph, &names, m.weights(), targets, matrix, before, count);
        }
    }
    Ok(())
}

pub(crate) struct Imported {
    pub mesh: Mesh,
    pub morph: Morph,
    pub looks: pfx_glb::Looks,
    pub materials: std::collections::BTreeMap<String, serde_json::Value>,
    pub skin: Option<(Skin, pfx_load::SkinRig)>,
    pub pieces: pieces::Found,
}

pub(crate) fn imported(
    object: &Object,
    recipe: &recipe::Recipe,
    base: &Path,
    path: &str,
    node: Option<&str>,
) -> Result<Imported, Error> {
    let file = recipe
        .asset(base, path)
        .map_err(|e| fail(object, "path", e))?;
    let bytes = pfx_recipe::files::read(&file)
        .map_err(|e| fail(object, "path", format!("cannot read {path}: {e}")))?;
    let parsed = gltf::Gltf::from_slice(&bytes)
        .map_err(|e| fail(object, "path", format!("{path} is not glTF: {e}")))?;
    let folder = file.parent().unwrap_or(base);
    let buffers = gltf::import_buffers(&parsed.document, Some(folder), parsed.blob.clone())
        .map_err(|e| fail(object, "path", format!("{path}: buffers: {e}")))?;
    let document = &parsed.document;
    let mut out = Mesh::default();
    let mut morph = Morph::default();
    let mut looks = pfx_glb::Looks::new();
    let mut found = false;
    let mut rig: Option<(usize, pfx_load::SkinRig, Option<Skin>)> = None;
    let roots: Vec<gltf::Node> = match document
        .default_scene()
        .or_else(|| document.scenes().next())
    {
        Some(scene) => scene.nodes().collect(),
        None => document.nodes().collect(),
    };
    let mut stack: Vec<(gltf::Node, DMat4, bool, Option<usize>)> = roots
        .into_iter()
        .rev()
        .map(|n| (n, DMat4::IDENTITY, false, None))
        .collect();
    let mut visited = vec![false; document.nodes().len()];
    let mut rules = Rules::new(object);
    let mut steps = Vec::new();
    let mut taken = std::collections::BTreeSet::new();
    while let Some((n, parent, inside, up)) = stack.pop() {
        if visited
            .get_mut(n.index())
            .is_none_or(|done| std::mem::replace(done, true))
        {
            continue;
        }
        let name = n.name().map(str::to_string);
        let label = name.clone().unwrap_or_else(|| format!("node{}", n.index()));
        steps.push(Step {
            index: n.index(),
            unique: pieces::unique(&mut taken, label, n.index()),
            name,
            parent: up,
        });
        let at = steps.len() - 1;
        let choice = rules.choose(&steps, at);
        let local = DMat4::from_cols_array_2d(&n.transform().matrix().map(|c| c.map(f64::from)));
        let chosen = inside || node.is_none_or(|want| n.name() == Some(want));
        let matrix = if node.is_some() && !inside && chosen {
            DMat4::IDENTITY
        } else {
            parent * local
        };
        if chosen && let Some(m) = n.mesh() {
            found = true;
            if !choice.hidden {
                let source = Source {
                    object,
                    path,
                    folder,
                    buffers: &buffers,
                };
                let before = out.positions.len();
                match n.skin() {
                    Some(skin) => {
                        let placed = (DMat4::IDENTITY, choice.material);
                        read_mesh(&source, &m, placed, &mut out, &mut morph, &mut looks)?;
                        let index = skin.index();
                        if rig.as_ref().is_some_and(|(at, _, _)| *at != index) {
                            return Err(fail(
                                object,
                                "path",
                                format!("{path}: the file has two skins, and an object takes one"),
                            ));
                        }
                        if rig.is_none() {
                            let found = crate::skin::rigged(&bytes, folder, index)
                                .map_err(|e| fail(object, "path", format!("{path}: {e}")))?;
                            rig = Some((index, found, None));
                        }
                        let (raw, weights) = crate::skin::influences(&m, &buffers);
                        if let Some((_, found, kept)) = rig.as_mut() {
                            let made =
                                crate::skin::skinned(&mut out, before, &raw, &weights, found)
                                    .map_err(|e| fail(object, "path", format!("{path}: {e}")))?;
                            match kept {
                                Some(skin) => {
                                    skin.extend_static(before);
                                    skin.join(made);
                                }
                                None => *kept = Some(made),
                            }
                        }
                    }
                    None => {
                        let placed = (matrix, choice.material);
                        read_mesh(&source, &m, placed, &mut out, &mut morph, &mut looks)?;
                    }
                }
            }
        }
        for child in n.children().collect::<Vec<_>>().into_iter().rev() {
            stack.push((child, matrix, chosen, Some(at)));
        }
    }
    let found_pieces = rules.finish(path, &steps)?;
    match node {
        Some(want) if !found => Err(fail(
            object,
            "node",
            format!("{path} has no node `{want}` with a mesh"),
        )),
        _ => {
            looks.retain(|_, look| look.has_maps());
            let skin = rig.and_then(|(_, found, skin)| {
                let mut skin = skin?;
                skin.extend_static(out.positions.len());
                Some((skin, found))
            });
            let mut materials = crate::library::file_materials(&bytes);
            materials.retain(|name, _| out.slots.iter().any(|slot| slot.material == *name));
            Ok(Imported {
                mesh: out,
                morph,
                looks,
                materials,
                skin,
                pieces: found_pieces,
            })
        }
    }
}

pub(crate) fn weights(object: &Object, morph: &mut Morph) -> Result<(), Error> {
    let Kind::Mesh {
        path, morph: set, ..
    } = &object.kind
    else {
        return Ok(());
    };
    for (name, &w) in set {
        let Some(at) = morph.names.iter().position(|n| n == name) else {
            let text = if morph.is_empty() {
                format!("{path} has no morph targets")
            } else {
                format!(
                    "{path} has no morph target `{name}`, expected one of: {}",
                    morph.names.join(", ")
                )
            };
            return Err(fail(object, &format!("morph.{name}"), text));
        };
        morph.weights[at] = w as f32;
    }
    Ok(())
}

pub(crate) fn local(
    object: &Object,
    pockets: &[&Object],
    recipe: &recipe::Recipe,
    base: &Path,
) -> Result<Mesh, Error> {
    let uv = uv_mode(object);
    Ok(match &object.kind {
        Kind::Box { size } => match &object.bevel {
            Some(b) => mesh::rounded_box(*size, b.width, b.segments),
            None => mesh::rounded_box(*size, 0.0, 1),
        },
        Kind::Cylinder {
            radius,
            depth,
            segments,
            caps,
        } => match &object.bevel {
            Some(b) => {
                let ring = outline::circle([0.0, 0.0], *radius, *segments);
                let edge = Profile::round(b.width, b.segments);
                let mut m = mesh::extrude(&ring, -depth * 0.5, depth * 0.5, &edge, UvMode::None);
                mesh::transform(&mut m, rotation([-90.0, 0.0, 0.0]));
                m
            }
            None => mesh::cylinder(*radius, *depth, *segments, *caps == Caps::Ngon),
        },
        Kind::Sphere {
            radius,
            segments,
            rings,
        } => mesh::sphere(*radius, *segments, *rings),
        Kind::Plane {
            size,
            grid,
            bend,
            curl,
            cup,
            jag,
        } => mesh::sheet(&Sheet {
            size: *size,
            cells: grid.unwrap_or([1, 1]),
            bend: bend.as_ref().map(|b| match b.form {
                BendForm::Wrap => Bend::Wrap(b.radius),
                BendForm::Lift => Bend::Lift(b.radius),
            }),
            curl: curl.as_ref().map(|c| [c.width, c.depth]),
            cup: cup.as_ref().map(|c| [c.from, c.depth]),
            jag: jag.clone(),
        }),
        Kind::Disc { radius, segments } => mesh::disc(*radius, *segments),
        Kind::Torus {
            major,
            minor,
            segments,
            rings,
        } => mesh::torus(*major, *minor, *segments, *rings),
        Kind::Capsule { radius, length } => {
            mesh::capsule(*radius, *length, CAPSULE_SEGMENTS, CAPSULE_RINGS)
        }
        Kind::Outline {
            outline: o,
            holes,
            front,
            back,
            profile: p,
        } => {
            let edge = profile(p);
            let found = holes::solid(object, o, holes, p)?;
            if pockets.is_empty() {
                let mut m = Mesh::default();
                for shape in &found {
                    m.append(&mesh::extrude_shape(shape, *front, *back, &edge, uv));
                }
                m
            } else {
                let shape = one(object, found)?;
                let mut cuts = Vec::new();
                for p in pockets {
                    let Kind::Pocket {
                        outline: cut,
                        depth,
                        rim,
                        floor,
                        ..
                    } = &p.kind
                    else {
                        continue;
                    };
                    cuts.push(Pocket {
                        outline: cut_ring(p, cut, &object.name, &shape)?,
                        depth: *depth,
                        rim: rim.as_ref().map_or(Profile::NONE, profile),
                        floor: floor.as_ref().map_or(Profile::NONE, profile),
                    });
                }
                mesh::pocket_shape(&shape, *front, *back, &edge, &cuts, uv)
            }
        }
        Kind::Pocket { .. } => Mesh::default(),
        Kind::Lathe {
            profile: p,
            segments,
            weld,
        } => mesh::lathe(p, *segments, *weld),
        Kind::Sweep {
            radii,
            ring,
            along: Some(along),
            ..
        } => {
            let [radius] = radii[..] else {
                return Err(fail(
                    object,
                    "radius",
                    format!(
                        "a sweep along an outline takes one radius, found {}",
                        radii.len()
                    ),
                ));
            };
            let outline = single(object, &along.outline)?;
            mesh::bead(&outline, radius, along.depth, along.offset, *ring)
        }
        Kind::Sweep {
            path,
            radii,
            ring,
            closed,
            caps,
            along: None,
        } => mesh::sweep(path, radii, *ring, *closed, *caps),
        Kind::Mesh { path, node, .. } => {
            imported(object, recipe, base, path, node.as_deref())?.mesh
        }
        Kind::Generator { generator, .. } => {
            return Err(fail(
                object,
                "generator",
                format!(
                    "`{generator}` is built by the app's own build binary, which registers it through the pfx library"
                ),
            ));
        }
        Kind::Library { object: name, .. } => {
            return Err(fail(
                object,
                "object",
                format!("`{name}` is a library object, imported with its own pieces"),
            ));
        }
        Kind::Card { size, .. } => card(*size),
        Kind::Text { .. } | Kind::Empty => Mesh::default(),
    })
}

fn unweld(m: &mut Mesh) -> Vec<u32> {
    let sources = m.indices.clone();
    let mut positions = Vec::with_capacity(m.indices.len());
    for &i in &m.indices {
        positions.push(m.positions[i as usize]);
    }
    if let Some(uvs1) = &m.uvs1 {
        m.uvs1 = Some(m.indices.iter().map(|&i| uvs1[i as usize]).collect());
    }
    m.normals = m
        .indices
        .iter()
        .map(|&i| m.normals.get(i as usize).copied().unwrap_or([0.0; 3]))
        .collect();
    m.indices = (0..positions.len() as u32).collect();
    m.positions = positions;
    sources
}

fn box_uvs(m: &mut Mesh) -> Option<Vec<u32>> {
    let [lo, hi] = mesh::bounds(m)?;
    let (lo, hi) = (DVec3::from(lo), DVec3::from(hi));
    let span = (hi - lo).max(DVec3::splat(1e-12));
    let sources = unweld(m);
    let mut uvs = vec![[0.0f32; 2]; m.positions.len()];
    for t in m.indices.chunks_exact(3) {
        let n = m.face_normal(t);
        let a = n.abs();
        let (u, v) = if a.x >= a.y && a.x >= a.z {
            (2, 1)
        } else if a.y >= a.z {
            (0, 2)
        } else {
            (0, 1)
        };
        for &i in t {
            let q = (m.point(i) - lo) / span;
            uvs[i as usize] = [q[u] as f32, (1.0 - q[v]) as f32];
        }
    }
    m.uvs = Some(uvs);
    Some(sources)
}

pub(crate) fn uvs(
    recipe: &recipe::Recipe,
    object: &Object,
    matrix: DMat4,
    m: &mut Mesh,
) -> Result<Option<Vec<u32>>, Error> {
    Ok(match &object.uv {
        _ if matches!(object.kind, Kind::Card { .. }) => None,
        Some(Uv::None) => {
            m.uvs = None;
            None
        }
        None if generated(object) => None,
        Some(Uv::Local) | None => {
            if m.uvs.is_none() && !m.indices.is_empty() {
                box_uvs(m)
            } else {
                None
            }
        }
        Some(_) => crate::uv::apply(recipe, object, matrix, m)?,
    })
}

fn groups(object: &Object) -> Option<&'static [&'static str]> {
    match object.kind {
        Kind::Cylinder { .. } => Some(&["side", "caps"]),
        _ => None,
    }
}

fn cylinder_slots(m: &mut Mesh, names: &[String]) {
    let mut side = Vec::new();
    let mut caps = Vec::new();
    for t in m.indices.chunks_exact(3) {
        if m.face_normal(t).y.abs() > CAP_NORMAL {
            caps.extend_from_slice(t);
        } else {
            side.extend_from_slice(t);
        }
    }
    let split = side.len() as u32;
    side.extend(caps);
    m.slots = vec![
        Slot {
            material: names[0].clone(),
            start: 0,
            count: split,
        },
        Slot {
            material: names[1].clone(),
            start: split,
            count: side.len() as u32 - split,
        },
    ];
    m.indices = side;
}

fn generated(object: &Object) -> bool {
    matches!(object.kind, Kind::Generator { .. } | Kind::Library { .. })
}

pub(crate) fn materials(object: &Object, m: &mut Mesh) -> Result<(), Error> {
    let names = &object.materials;
    match (&object.material, names.len()) {
        (Some(_), _) if Rules::overrides(object) => {}
        (Some(name), _) => m.set_material(name),
        (None, 0) if generated(object) || matches!(object.kind, Kind::Mesh { .. }) => {}
        (None, 0) => m.slots.clear(),
        (None, 1) => m.set_material(&names[0]),
        (None, count) => match groups(object) {
            Some(groups) if groups.len() == count => cylinder_slots(m, names),
            Some(groups) => {
                return Err(fail(
                    object,
                    "materials",
                    format!(
                        "a {} has {} face groups ({}), expected that many names, found {count}",
                        object.kind.name(),
                        groups.len(),
                        groups.join(", ")
                    ),
                ));
            }
            None => {
                return Err(fail(
                    object,
                    "materials",
                    format!(
                        "a {} defines no face groups, expected one name or `material`",
                        object.kind.name()
                    ),
                ));
            }
        },
    }
    Ok(())
}

pub(crate) fn normals(object: &Object, m: &mut Mesh, morph: Option<&mut Morph>) {
    if m.indices.is_empty() || keeps_normals(object, m) {
        return;
    }
    if let Some(morph) = morph {
        morph.normals.clear();
        let (degrees, flat_faces) = match (object.smooth_angle, object.smooth) {
            (Some(angle), _) => (angle, object.flat_faces),
            (None, true) => (180.0, false),
            (None, false) => (0.01, false),
        };
        mesh::smooth_morphed(m, morph, degrees, flat_faces);
        return;
    }
    match (object.smooth_angle, object.smooth) {
        (Some(angle), _) if object.flat_faces => mesh::smooth_by_angle_flat_faces(m, angle),
        (Some(angle), _) => mesh::smooth_by_angle(m, angle),
        (None, true) => mesh::smooth(m),
        (None, false) if matches!(object.kind, Kind::Generator { .. }) && carries_normals(m) => {}
        (None, false) => mesh::flat(m),
    }
}

pub(crate) fn normals_traced(object: &Object, m: &mut Mesh, morph: Option<&mut Morph>) -> Vec<u32> {
    if m.indices.is_empty() || keeps_normals(object, m) {
        return (0..m.positions.len() as u32).collect();
    }
    let (degrees, flat_faces) = match (object.smooth_angle, object.smooth) {
        (Some(angle), _) => (angle, object.flat_faces),
        (None, true) => (180.0, false),
        (None, false) => (0.01, false),
    };
    mesh::smooth_traced(m, degrees, flat_faces, morph)
}

fn keeps_normals(object: &Object, m: &Mesh) -> bool {
    let imported = matches!(object.kind, Kind::Mesh { .. } | Kind::Library { .. });
    imported
        && object.keep_normals != Some(false)
        && !object.smooth
        && object.smooth_angle.is_none()
        && m.normals.len() == m.positions.len()
        && m.normals.iter().all(|n| *n != [0.0; 3])
}

fn carries_normals(m: &Mesh) -> bool {
    m.normals.len() == m.positions.len() && m.normals.iter().any(|n| *n != [0.0; 3])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card_object(uv: &str) -> Object {
        let text = format!(
            "recipe = 1\nname = \"t\"\n\n[[object]]\nname = \"c\"\nkind = \"card\"\nsize = [2.0, 1.0]\nimage = \"a.png\"\n{uv}"
        );
        recipe::parse(&text).unwrap().objects.remove(0)
    }

    #[test]
    fn a_card_quad_spans_its_image_from_the_top_left() {
        let m = card([2.0, 1.0]);
        let uvs = m.uvs.as_ref().unwrap();
        assert_eq!(m.positions.len(), uvs.len());
        assert_eq!(m.indices.len(), 6);
        for (p, uv) in m.positions.iter().zip(uvs) {
            assert!(uv.iter().all(|&v| v == 0.0 || v == 1.0), "{uv:?}");
            let expected = [(uv[0] - 0.5) * 2.0, 0.5 - uv[1], 0.0];
            for k in 0..3 {
                assert!((p[k] - expected[k]).abs() < 1e-6, "{p:?} at {uv:?}");
            }
        }
        let corners: Vec<[f32; 2]> = uvs.clone();
        for want in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]] {
            assert!(corners.contains(&want), "{want:?}");
        }
        for n in &m.normals {
            assert!((n[2] - 1.0).abs() < 1e-6, "{n:?}");
        }
        let t = &m.indices[..3];
        assert!(m.face_normal(t).z > 0.0);
    }

    #[test]
    fn a_card_keeps_its_own_uvs_whatever_its_uv_key_says() {
        for uv in ["", "uv = \"none\"\n", "uv = \"local\"\n"] {
            let object = card_object(uv);
            let recipe = recipe::parse("recipe = 1\nname = \"t\"\n").unwrap();
            let mut m = local(&object, &[], &recipe, Path::new("")).unwrap();
            let before = m.uvs.clone();
            uvs(&recipe, &object, DMat4::IDENTITY, &mut m).unwrap();
            assert_eq!(m.uvs, before, "{uv}");
            assert!(m.uvs.is_some());
        }
    }
}
