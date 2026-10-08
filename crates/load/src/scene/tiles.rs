use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

pub use pfx_scene::types::{TileCell, TilePlane, Tiles};

use pfx_core::anim::math::transform_point;

use super::build::{self, Places, Step, inner};
use super::{IDENTITY, Matrix, Object, Scene, SceneError, multiply};

pub const EMPTY: char = Tiles::EMPTY;
pub const KEYS: &str = "#@%&*+=ABCDEFGHJKLMNPQRSTUVWXYZabcdefghjkmnpqrstuvwxyz0123456789";

pub fn axes(layer: &Tiles) -> [[f32; 3]; 2] {
    let (across, up) = layer.plane.unwrap_or_default().axes();
    [across, up]
}

pub fn cell_at(layer: &Tiles, point: [f64; 3]) -> [i32; 2] {
    let [across, up] = axes(layer);
    let [width, height] = layer.cell_sides();
    let d: [f64; 3] = std::array::from_fn(|k| point[k] - f64::from(layer.origin[k]));
    let along = |axis: [f32; 3]| (0..3).map(|k| d[k] * f64::from(axis[k])).sum::<f64>();
    [
        (along(across) / f64::from(width)).round() as i32,
        (along(up) / f64::from(height)).round() as i32,
    ]
}

pub fn corners(layer: &Tiles, cell: [i32; 2]) -> [[f32; 3]; 4] {
    let [across, up] = axes(layer);
    let [width, height] = layer.cell_sides();
    let centre = layer.position(cell);
    let at = |a: f32, b: f32| {
        std::array::from_fn(|k| centre[k] + across[k] * a * width + up[k] * b * height)
    };
    [at(-0.5, -0.5), at(0.5, -0.5), at(0.5, 0.5), at(-0.5, 0.5)]
}

pub fn get(layer: &Tiles, cell: [i32; 2]) -> Option<String> {
    layer
        .cells()
        .into_iter()
        .find(|(at, _)| *at == cell)
        .map(|(_, tile)| tile.to_string())
}

pub fn key(layer: &Tiles, prefab: &str) -> Option<String> {
    layer
        .palette
        .iter()
        .find(|(_, path)| path.as_str() == prefab)
        .map(|(key, _)| key.clone())
}

pub fn free_key(layer: &Tiles) -> Option<String> {
    KEYS.chars()
        .map(String::from)
        .find(|key| !layer.palette.contains_key(key))
}

#[derive(Clone, Debug, PartialEq)]
pub enum Painted {
    Rows { rows: Vec<String>, origin: [f32; 3] },
    Cells(Vec<TileCell>),
}

pub fn painted(layer: &Tiles, changes: &[([i32; 2], Option<&str>)]) -> Result<Painted, String> {
    if layer.cells.is_some() {
        let mut cells: BTreeMap<[i32; 2], String> = layer
            .cells()
            .into_iter()
            .map(|(at, tile)| (at, tile.to_string()))
            .collect();
        let mut order: Vec<[i32; 2]> = layer.cells.iter().flatten().map(|cell| cell.at).collect();
        for (at, tile) in changes {
            match tile {
                Some(tile) => {
                    if cells.insert(*at, tile.to_string()).is_none() {
                        order.push(*at);
                    }
                }
                None => {
                    cells.remove(at);
                }
            }
        }
        return Ok(Painted::Cells(
            order
                .into_iter()
                .filter_map(|at| {
                    Some(TileCell {
                        at,
                        tile: cells.get(&at)?.clone(),
                    })
                })
                .collect(),
        ));
    }
    let mut rows: Vec<Vec<char>> = layer
        .rows
        .iter()
        .flatten()
        .map(|row| row.chars().collect())
        .collect();
    let mut shift = [0i32; 2];
    for (cell, tile) in changes {
        let [i, j] = [cell[0] + shift[0], cell[1] + shift[1]];
        let Some(tile) = tile else {
            let height = rows.len() as i32;
            let r = height - 1 - j;
            if (0..height).contains(&r)
                && i >= 0
                && let Some(slot) = rows[r as usize].get_mut(i as usize)
            {
                *slot = EMPTY;
            }
            continue;
        };
        let mut chars = tile.chars();
        let (Some(character), None) = (chars.next(), chars.next()) else {
            return Err(format!(
                "tile layer {}: a rows layer takes one-character keys, not {tile:?}",
                layer.name
            ));
        };
        let (mut i, mut j) = (i, j);
        if i < 0 {
            let grow = -i as usize;
            for row in &mut rows {
                if !row.is_empty() {
                    row.splice(0..0, std::iter::repeat_n(EMPTY, grow));
                }
            }
            shift[0] += -i;
            i = 0;
        }
        if j < 0 {
            for _ in 0..-j {
                rows.push(Vec::new());
            }
            shift[1] += -j;
            j = 0;
        }
        let height = rows.len() as i32;
        if j >= height {
            for _ in 0..(j - height + 1) {
                rows.insert(0, Vec::new());
            }
        }
        let height = rows.len() as i32;
        let row = &mut rows[(height - 1 - j) as usize];
        if row.len() <= i as usize {
            row.resize(i as usize + 1, EMPTY);
        }
        row[i as usize] = character;
    }
    let [across, up] = axes(layer);
    let [width, height] = layer.cell_sides();
    let (a, b) = (shift[0] as f32 * width, shift[1] as f32 * height);
    Ok(Painted::Rows {
        rows: rows.iter().map(|row| row.iter().collect()).collect(),
        origin: std::array::from_fn(|k| layer.origin[k] - across[k] * a - up[k] * b),
    })
}

pub fn cell_key(layer: &str, cell: [i32; 2]) -> String {
    format!("{layer}[{},{}]", cell[0], cell[1])
}

pub fn cell_of(object: &str) -> Option<(String, [i32; 2])> {
    let head = object.split('/').next()?;
    let (layer, rest) = head.split_once('[')?;
    let inside = rest.strip_suffix(']')?;
    let (i, j) = inside.split_once(',')?;
    Some((layer.to_string(), [i.parse().ok()?, j.parse().ok()?]))
}

pub fn line(from: [i32; 2], to: [i32; 2]) -> Vec<[i32; 2]> {
    let (dx, dy) = ((to[0] - from[0]).abs(), -(to[1] - from[1]).abs());
    let (sx, sy) = (
        if from[0] < to[0] { 1 } else { -1 },
        if from[1] < to[1] { 1 } else { -1 },
    );
    let mut error = dx + dy;
    let mut at = from;
    let mut cells = vec![at];
    while at != to {
        let twice = 2 * error;
        if twice >= dy {
            error += dy;
            at[0] += sx;
        }
        if twice <= dx {
            error += dx;
            at[1] += sy;
        }
        cells.push(at);
    }
    cells
}

pub fn rectangle(from: [i32; 2], to: [i32; 2]) -> Vec<[i32; 2]> {
    let mut cells = Vec::new();
    for j in from[1].min(to[1])..=from[1].max(to[1]) {
        for i in from[0].min(to[0])..=from[0].max(to[0]) {
            cells.push([i, j]);
        }
    }
    cells
}

struct Template {
    objects: Vec<(String, Object)>,
    bodies: Vec<(String, super::Body)>,
    triggers: Vec<(String, super::Trigger)>,
    characters: Vec<(String, super::Character)>,
    animations: Vec<(String, super::anim::Animation)>,
    layers: Vec<(String, String)>,
}

fn translate(at: [f32; 3]) -> Matrix {
    let mut matrix = IDENTITY;
    matrix[3] = [at[0], at[1], at[2], 1.0];
    matrix
}

const PLACED: &str = "tile";

fn graft<T: Clone>(into: &mut BTreeMap<String, T>, from: &[(String, T)], base: &str) {
    for (name, value) in from {
        into.insert(format!("{base}/{name}"), value.clone());
    }
}

fn quoted(text: &str) -> String {
    toml::Value::String(text.to_string()).to_string()
}

fn synthetic(
    root: &Path,
    file: &Path,
    text: String,
    overlay: &BTreeMap<PathBuf, String>,
    refuse: &dyn Fn(String) -> SceneError,
) -> Result<Scene, SceneError> {
    let mut held = overlay.clone();
    held.insert(file.to_path_buf(), text);
    let project = build::project(root)?;
    let absolute: BTreeMap<PathBuf, String> = held
        .iter()
        .map(|(file, text)| (root.join(file), text.clone()))
        .collect();
    let resolved = project
        .scene_with(&root.join(file), absolute)
        .map_err(|found| refuse(build::refused(root, &found).message))?;
    let mut built = build::build(&root.join(file), root, &resolved, None, &held)?;
    built.files.remove(&root.join(file));
    Ok(built)
}

pub fn extent(root: &Path, asset: &str) -> Result<[[f32; 3]; 2], SceneError> {
    let file = PathBuf::from(format!("{asset}.extent.scene.toml"));
    let text = if asset.ends_with(".prefab.toml") {
        format!(
            "format = 1\n\n[[object]]\nname = \"{PLACED}\"\nprefab = {}\n",
            quoted(asset)
        )
    } else {
        format!(
            "format = 1\n\n[mesh.{PLACED}]\nfile = {}\n\n[[object]]\nname = \"{PLACED}\"\nmesh = \"{PLACED}\"\n",
            quoted(asset)
        )
    };
    let refuse = |message: String| SceneError::new(&root.join(asset), None, message);
    let built = synthetic(root, &file, text, &BTreeMap::new(), &refuse)?;
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for draw in built.draws().items {
        for p in &draw.geometry.positions {
            let world = transform_point(&draw.model, *p);
            for k in 0..3 {
                low[k] = low[k].min(world[k]);
                high[k] = high[k].max(world[k]);
            }
        }
    }
    if low.iter().chain(&high).all(|value| value.is_finite()) {
        Ok([low, high])
    } else {
        Ok([[0.0; 3]; 2])
    }
}

fn template(
    scene: &mut Scene,
    root: &Path,
    prefab: &str,
    overlay: &BTreeMap<PathBuf, String>,
    refuse: &dyn Fn(String) -> SceneError,
) -> Result<Template, SceneError> {
    let named = |message: String| refuse(format!("tile prefab {prefab}: {message}"));
    let project = build::project(root)?;
    let absolute: BTreeMap<PathBuf, String> = overlay
        .iter()
        .map(|(file, text)| (root.join(file), text.clone()))
        .collect();
    let resolved = project
        .scene_with(&root.join(prefab), absolute)
        .map_err(|found| named(build::refused(root, &found).message))?;
    let built = build::build(&root.join(prefab), root, &resolved, None, overlay)?;
    if let Some((name, _)) = built
        .animations
        .iter()
        .find(|(_, animation)| animation.sprite.is_some())
    {
        return Err(named(format!(
            "its object {name} is a sprite, and a tile places glTF clips only; place the sprite as an object"
        )));
    }
    for (kind, count) in [
        ("lights", built.lights.len()),
        ("emitters", built.emitters.len()),
        ("movers", built.movers.len()),
        ("text", built.texts.len()),
        ("content", built.contents.len()),
        ("sounds", built.sounds.len()),
    ] {
        if count > 0 {
            return Err(named(format!(
                "a tile places objects, bodies, characters, triggers and glTF clips only, and it has {kind}"
            )));
        }
    }
    for (name, material) in built.library.iter() {
        match scene.library.get(name) {
            Some(own) if own == material => {}
            Some(_) => {
                return Err(named(format!(
                    "its material {name} differs from the scene's material of that name"
                )));
            }
            None => {
                scene.library.insert(name, *material).map_err(named)?;
            }
        }
    }
    let mesh_key = |key: &str| format!("{prefab}:{key}");
    for (key, mesh) in &built.meshes {
        scene.meshes.insert(mesh_key(key), mesh.clone());
    }
    for (file, hash) in &built.files {
        scene.files.insert(file.clone(), *hash);
    }
    let objects = built
        .objects
        .iter()
        .map(|object| {
            let mut object = object.clone();
            if !object.mesh.is_empty() {
                object.mesh = mesh_key(&object.mesh);
            }
            (object.name.clone(), object)
        })
        .collect();
    Ok(Template {
        objects,
        bodies: built.bodies.into_iter().collect(),
        triggers: built.triggers.into_iter().collect(),
        characters: built.characters.into_iter().collect(),
        animations: built.animations.into_iter().collect(),
        layers: built.body_layers.into_iter().collect(),
    })
}

pub(super) fn expand(
    scene: &mut Scene,
    root: &Path,
    resolved: &pfx_scene::Scene,
    places: &Places,
    overlay: &BTreeMap<PathBuf, String>,
) -> Result<(), SceneError> {
    let mut templates: BTreeMap<String, Template> = BTreeMap::new();
    for entry in &resolved.tiles {
        let layer = &entry.value;
        let used: BTreeSet<&str> = layer.cells().into_iter().map(|(_, tile)| tile).collect();
        for (key, prefab) in &layer.palette {
            if templates.contains_key(prefab) || !used.contains(key.as_str()) {
                continue;
            }
            let steps = [
                Step::Named("tiles", inner(&entry.key)),
                Step::Key("palette"),
                Step::Key(key),
            ];
            let refuse = |message: String| places.refusal(&entry.file, &steps, message);
            let made = template(scene, root, prefab, overlay, &refuse)?;
            templates.insert(prefab.clone(), made);
        }
    }
    let mut taken: HashSet<String> = scene.objects.iter().map(|o| o.name.clone()).collect();
    let mut next = scene.objects.iter().map(|o| o.id).max().unwrap_or(0) + 1;
    for entry in &resolved.tiles {
        let layer = &entry.value;
        for (cell, tile) in layer.cells() {
            let Some(made) = layer.palette.get(tile).and_then(|p| templates.get(p)) else {
                continue;
            };
            let base = cell_key(&entry.key, cell);
            let at = layer.position(cell);
            let shift = translate(at);
            for (name, object) in &made.objects {
                let key = format!("{base}/{name}");
                if !taken.insert(key.clone()) {
                    return places.error(
                        &entry.file,
                        &[Step::Named("tiles", inner(&entry.key))],
                        format!(
                            "tile layer {}: the cell's object {key} is already an object of the scene",
                            entry.key
                        ),
                    );
                }
                let mut placed = object.clone();
                placed.name = key;
                placed.model = multiply(shift, object.model);
                match &object.parent {
                    Some(parent) => placed.parent = Some(format!("{base}/{parent}")),
                    None => placed.at = std::array::from_fn(|k| object.at[k] + at[k]),
                }
                placed.id = next;
                next += 1;
                scene.objects.push(placed);
            }
            graft(&mut scene.bodies, &made.bodies, &base);
            graft(&mut scene.triggers, &made.triggers, &base);
            graft(&mut scene.characters, &made.characters, &base);
            graft(&mut scene.animations, &made.animations, &base);
            graft(&mut scene.body_layers, &made.layers, &base);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tiles_tests.rs"]
mod tests;
