use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use egui::Pos2;
use glam::DVec3;
use pfx_editor_viewport::pick::matrix;
use pfx_editor_viewport::{Eye, Lens};
use pfx_load::scene::tiles::{self, Tiles};
use pfx_load::scene::{EditError, Id, Kind, PatchGroup, Scene, Target, Value};

use crate::editor::Editor;
use crate::outline::Item;
use crate::snap::{self, Snapping, Surfaces, VERTEX_POINTS};

pub const OFFSET: [f32; 3] = [1.0, 0.0, 0.0];
const TRIES: u32 = 16;
const PLACES: f64 = 1e4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Paint,
    Erase,
    Pick,
}

impl Tool {
    pub fn word(self) -> &'static str {
        match self {
            Tool::Paint => "paint",
            Tool::Erase => "erase",
            Tool::Pick => "pick",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Free,
    Line,
    Rectangle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stroke {
    pub shape: Shape,
    pub from: [i32; 2],
    pub to: [i32; 2],
    pub visited: Vec<[i32; 2]>,
}

impl Stroke {
    pub fn new(shape: Shape, at: [i32; 2]) -> Stroke {
        Stroke {
            shape,
            from: at,
            to: at,
            visited: vec![at],
        }
    }

    pub fn reach(&mut self, at: [i32; 2]) {
        if self.shape == Shape::Free && at != self.to {
            for cell in tiles::line(self.to, at).into_iter().skip(1) {
                if !self.visited.contains(&cell) {
                    self.visited.push(cell);
                }
            }
        }
        self.to = at;
    }

    pub fn cells(&self) -> Vec<[i32; 2]> {
        match self.shape {
            Shape::Free => self.visited.clone(),
            Shape::Line => tiles::line(self.from, self.to),
            Shape::Rectangle => tiles::rectangle(self.from, self.to),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Brush {
    pub on: bool,
    pub tool: Tool,
    pub layer: Option<String>,
    pub prefab: Option<String>,
    pub stroke: Option<Stroke>,
    pub grab: Option<(Eye, Pos2, bool)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aim {
    pub point: DVec3,
    pub normal: DVec3,
    pub at: [f32; 3],
    pub rotate: [f32; 3],
}

#[derive(Default)]
pub struct Level {
    pub snapping: Snapping,
    pub stamp: Option<PathBuf>,
    pub picked: Vec<String>,
    pub offset: [f32; 3],
    pub brush: Brush,
    pub shift: bool,
    pub aim: Option<Aim>,
    surfaces: Option<(BTreeMap<PathBuf, [u8; 32]>, usize, Surfaces)>,
    extents: BTreeMap<PathBuf, Result<[[f32; 3]; 2], String>>,
}

impl Level {
    pub fn new() -> Level {
        Level {
            offset: OFFSET,
            ..Level::default()
        }
    }

    pub fn shown(&self) -> bool {
        self.brush.on
            || self.stamp.is_some()
            || self.picked.len() > 1
            || self.snapping != Snapping::default()
    }
}

pub fn placeable(path: &Path) -> bool {
    let name = path.to_string_lossy().to_lowercase();
    name.ends_with(".prefab.toml") || name.ends_with(".gltf") || name.ends_with(".glb")
}

pub fn stem(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lower = name.to_lowercase();
    for suffix in [".prefab.toml", ".gltf", ".glb"] {
        if lower.ends_with(suffix) {
            return name[..name.len() - suffix.len()].to_string();
        }
    }
    name
}

pub fn unique(taken: impl Fn(&str) -> bool, stem: &str) -> String {
    let stem = if stem.trim().is_empty() {
        "object"
    } else {
        stem.trim()
    };
    if !taken(stem) {
        return stem.to_string();
    }
    (2..)
        .map(|count| format!("{stem} {count}"))
        .find(|name| !taken(name))
        .unwrap_or_default()
}

pub fn base(name: &str) -> &str {
    match name.rsplit_once(' ') {
        Some((base, count)) if !base.is_empty() && count.parse::<u32>().is_ok() => base,
        _ => name,
    }
}

fn round(value: f64) -> f32 {
    let rounded = (value * PLACES).round() / PLACES;
    if rounded == 0.0 { 0.0 } else { rounded as f32 }
}

fn rounded(point: DVec3) -> [f32; 3] {
    point.to_array().map(round)
}

fn taken(error: &EditError) -> bool {
    error.message.contains("is taken")
}

fn world(scene: &Scene, name: &str) -> Option<DVec3> {
    let model = scene.object(name)?.model;
    Some(DVec3::new(
        f64::from(model[3][0]),
        f64::from(model[3][1]),
        f64::from(model[3][2]),
    ))
}

fn local(scene: &Scene, name: &str, point: DVec3) -> Option<DVec3> {
    let object = scene.object(name)?;
    Some(
        match object.parent.as_deref().and_then(|p| scene.object(p)) {
            Some(parent) => matrix(parent.model).inverse().transform_point3(point),
            None => point,
        },
    )
}

impl Editor {
    pub fn relative(&self, file: &Path) -> String {
        let root = self.project_root();
        let root = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
        let file = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
        file.strip_prefix(&root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn derived(&self, kind: &str, name: &str, salt: u32) -> Id {
        let scene = self.relative(self.path());
        Id::derive(format!("{scene}\n{kind}\n{name}\n{salt}").as_bytes())
    }

    pub fn layers(&self) -> Vec<Tiles> {
        self.edit
            .resolved()
            .tiles
            .iter()
            .map(|entry| entry.value.clone())
            .collect()
    }

    pub fn is_cell(&self, object: &str) -> bool {
        let Some((layer, _)) = tiles::cell_of(object) else {
            return false;
        };
        self.edit
            .resolved()
            .tiles
            .iter()
            .any(|entry| entry.key == layer)
    }

    pub fn surfaces(&mut self) -> Option<&Surfaces> {
        let scene = self.viewport.scene()?;
        let fresh = self
            .level
            .surfaces
            .as_ref()
            .is_none_or(|(files, count, _)| files != &scene.files || *count != scene.objects.len());
        if fresh {
            self.level.surfaces = Some((
                scene.files.clone(),
                scene.objects.len(),
                Surfaces::of(scene),
            ));
        }
        self.level
            .surfaces
            .as_ref()
            .map(|(_, _, surfaces)| surfaces)
    }

    pub fn lens(&self) -> Option<Lens> {
        Some(Lens::new(&self.viewport.eye()?, self.viewport.image()?))
    }

    pub fn extent(&mut self, asset: &Path) -> Result<[[f32; 3]; 2], String> {
        if let Some(found) = self.level.extents.get(asset) {
            return found.clone();
        }
        let relative = self.relative(asset);
        let found = tiles::extent(self.project_root(), &relative).map_err(|e| e.to_string());
        self.level
            .extents
            .insert(asset.to_path_buf(), found.clone());
        found
    }

    pub fn aim(&mut self, asset: &Path, at: Pos2) -> Result<Aim, String> {
        let lens = self.lens().ok_or("the viewport has not been laid out")?;
        let ray = lens.ray(at);
        let snapping = self.level.snapping;
        let bounds = self.extent(asset)?;
        let surfaces = self.surfaces();
        let hit = surfaces
            .as_ref()
            .filter(|_| snapping.surface)
            .and_then(|surfaces| surfaces.cast(&ray));
        let (mut point, normal) = match hit {
            Some(hit) => (hit.point, hit.normal),
            None => (
                snap::ground(&ray).ok_or("nothing under the pointer to place on")?,
                DVec3::Y,
            ),
        };
        if snapping.vertex
            && let Some(vertex) = surfaces
                .as_ref()
                .and_then(|surfaces| surfaces.vertex(&lens, at, VERTEX_POINTS, &[]))
        {
            point = vertex;
        }
        if snapping.grid {
            point = snap::grid(point, snapping.step, snap::across(normal));
        }
        let rotate = if snapping.align {
            snap::upright(normal)
        } else {
            DVec3::ZERO
        };
        let lifted = point + snap::lift(bounds, rotate, normal);
        Ok(Aim {
            point,
            normal,
            at: rounded(lifted),
            rotate: rounded(rotate),
        })
    }

    pub fn place(&mut self, asset: &Path, at: Pos2) -> Option<String> {
        if !self.editable() {
            return None;
        }
        let aim = match self.aim(asset, at) {
            Ok(aim) => aim,
            Err(error) => {
                self.log.warn(format!("place {}: {error}", stem(asset)));
                return None;
            }
        };
        match self.place_at(asset, aim.at, aim.rotate) {
            Ok(name) => Some(name),
            Err(error) => {
                self.log.warn(error);
                None
            }
        }
    }

    pub fn place_at(
        &mut self,
        asset: &Path,
        at: [f32; 3],
        rotate: [f32; 3],
    ) -> Result<String, String> {
        if !placeable(asset) {
            return Err(format!(
                "{}: only a *.prefab.toml or a glTF can be placed",
                self.relative(asset)
            ));
        }
        self.end_group();
        let relative = self.relative(asset);
        let prefab = relative.to_lowercase().ends_with(".prefab.toml");
        let scene = self.scene().clone();
        let name = unique(|name| scene.object(name).is_some(), &stem(asset));
        let file = self.project_root().join(&relative);
        let mesh = scene
            .meshes
            .iter()
            .find(|(key, mesh)| {
                mesh.file == file && mesh.node.is_none() && !key.contains(['/', ':'])
            })
            .map(|(key, _)| key.clone());
        let mesh_name = mesh
            .clone()
            .unwrap_or_else(|| unique(|key| scene.meshes.contains_key(key), &stem(asset)));
        let mut fields: Vec<(&str, Value)> = Vec::new();
        if prefab {
            fields.push(("prefab", Value::from(relative.as_str())));
        } else {
            fields.push(("mesh", Value::from(mesh_name.as_str())));
        }
        fields.push(("at", Value::from(at)));
        if rotate != [0.0; 3] {
            fields.push(("rotate", Value::from(rotate)));
        }
        let group = self.salted(format!("place {name}: no free id"), |editor, salt| {
            let id = editor.derived("object", &name, salt);
            let mesh_id = editor.derived("mesh", &mesh_name, salt);
            editor.edit.dry_run(|edit| {
                if !prefab && mesh.is_none() {
                    edit.add_mesh(mesh_id, &mesh_name, &relative, None)?;
                }
                edit.add(Kind::Object, id, &name, &fields, None)
            })
        })?;
        self.land(labelled(group, format!("place {name}")));
        if self.scene().object(&name).is_none() {
            return Err(format!("place {name}: the scene did not take it"));
        }
        self.level.stamp = Some(asset.to_path_buf());
        self.select(Some(Item::Object(name.clone())));
        Ok(name)
    }

    pub fn picked(&self) -> Vec<String> {
        if self.level.picked.is_empty() {
            return self
                .selected_object()
                .map(str::to_string)
                .into_iter()
                .collect();
        }
        self.level.picked.clone()
    }

    pub fn pick_more(&mut self, name: &str) {
        let mut picked = self.picked();
        match picked.iter().position(|found| found == name) {
            Some(at) if picked.len() > 1 => {
                picked.remove(at);
            }
            Some(_) => {}
            None => picked.push(name.to_string()),
        }
        let primary = picked.last().cloned();
        self.select(primary.map(Item::Object));
        self.level.picked = picked;
    }

    fn movable(&mut self, verb: &str) -> Vec<String> {
        let picked = self.picked();
        let mut kept = Vec::new();
        for name in picked {
            if self.is_cell(&name) {
                self.log.warn(format!(
                    "{verb}: {name} is a tile; paint its layer with the brush instead"
                ));
            } else if self.scene().object(&name).is_some() {
                kept.push(name);
            }
        }
        kept
    }

    pub fn duplicate(&mut self) -> Vec<String> {
        if !self.editable() {
            return Vec::new();
        }
        self.end_group();
        let names = self.movable("duplicate");
        if names.is_empty() {
            return Vec::new();
        }
        let scene = self.scene().clone();
        let offset = self.level.offset;
        let mut made: Vec<String> = Vec::new();
        let mut plan = Vec::new();
        for name in &names {
            if name.contains('/') {
                self.log.warn(format!(
                    "duplicate: {name} is placed by a prefab; duplicate its placement"
                ));
                continue;
            }
            let Some(object) = scene.object(name) else {
                continue;
            };
            let new = unique(
                |candidate| {
                    scene.object(candidate).is_some() || made.iter().any(|m| m == candidate)
                },
                base(name),
            );
            let at: [f32; 3] = std::array::from_fn(|k| object.at[k] + offset[k]);
            made.push(new.clone());
            plan.push((name.clone(), new, at));
        }
        if plan.is_empty() {
            return Vec::new();
        }
        let label = match plan.len() {
            1 => format!("duplicate {}", plan[0].0),
            count => format!("duplicate {count} objects"),
        };
        let outcome = self.salted(format!("{label}: no free id"), |editor, salt| {
            let ids: Vec<Id> = plan
                .iter()
                .map(|(_, new, _)| editor.derived("object", new, salt))
                .collect();
            editor.edit.dry_run(|edit| {
                for ((name, new, at), id) in plan.iter().zip(&ids) {
                    edit.duplicate_object(name, new, *id)?;
                    edit.set_at(new, *at)?;
                }
                Ok(())
            })
        });
        match outcome {
            Ok(group) => {
                self.land(labelled(group, label));
                let primary = made.last().cloned();
                self.select(primary.map(Item::Object));
                self.level.picked = made.clone();
                made
            }
            Err(error) => {
                self.log.warn(error);
                Vec::new()
            }
        }
    }

    fn moves(&mut self, label: String, moves: Vec<(String, DVec3)>) -> bool {
        let scene = self.scene().clone();
        let mut writes = Vec::new();
        for (name, point) in moves {
            let Some(local) = local(&scene, &name, point) else {
                continue;
            };
            writes.push((name, rounded(local)));
        }
        let found = self.edit.dry_run(|edit| {
            for (name, at) in &writes {
                edit.set_at(name, *at)?;
            }
            Ok(())
        });
        match found {
            Ok(group) if group.is_empty() => {
                self.log.info(format!("{label}: already in place"));
                false
            }
            Ok(group) => {
                self.land(labelled(group, label));
                true
            }
            Err(error) => {
                self.log.edit_error(&error);
                false
            }
        }
    }

    pub fn align(&mut self, axis: usize, to_last: bool) -> bool {
        if !self.editable() || axis > 2 {
            return false;
        }
        self.end_group();
        let names = self.movable("align");
        if names.len() < 2 {
            self.log
                .info("align: select two objects or more (shift-click)");
            return false;
        }
        let scene = self.scene().clone();
        let anchor = if to_last {
            &names[names.len() - 1]
        } else {
            &names[0]
        };
        let Some(target) = world(&scene, anchor) else {
            return false;
        };
        let moves = names
            .iter()
            .filter(|name| *name != anchor)
            .filter_map(|name| {
                let mut point = world(&scene, name)?;
                point[axis] = target[axis];
                Some((name.clone(), point))
            })
            .collect();
        let label = format!(
            "align {} objects in {} to {anchor}",
            names.len(),
            ["x", "y", "z"][axis]
        );
        self.moves(label, moves)
    }

    pub fn distribute(&mut self, axis: usize) -> bool {
        if !self.editable() || axis > 2 {
            return false;
        }
        self.end_group();
        let names = self.movable("distribute");
        if names.len() < 3 {
            self.log
                .info("distribute: select three objects or more (shift-click)");
            return false;
        }
        let scene = self.scene().clone();
        let mut placed: Vec<(String, DVec3)> = names
            .iter()
            .filter_map(|name| Some((name.clone(), world(&scene, name)?)))
            .collect();
        placed.sort_by(|a, b| a.1[axis].total_cmp(&b.1[axis]).then_with(|| a.0.cmp(&b.0)));
        let (first, last) = (placed[0].1[axis], placed[placed.len() - 1].1[axis]);
        let gap = (last - first) / (placed.len() - 1) as f64;
        let moves = placed
            .iter()
            .enumerate()
            .skip(1)
            .take(placed.len() - 2)
            .map(|(place, (name, point))| {
                let mut point = *point;
                point[axis] = first + gap * place as f64;
                (name.clone(), point)
            })
            .collect();
        let label = format!(
            "distribute {} objects along {}",
            names.len(),
            ["x", "y", "z"][axis]
        );
        self.moves(label, moves)
    }

    pub fn brush_layer(&self) -> Option<Tiles> {
        let layers = self.layers();
        let wanted = self.level.brush.layer.as_deref();
        layers
            .iter()
            .find(|layer| Some(layer.name.as_str()) == wanted)
            .or(layers.first())
            .cloned()
    }

    pub fn brush_prefab(&mut self) -> Option<String> {
        if let Some(prefab) = &self.level.brush.prefab {
            return Some(prefab.clone());
        }
        if let Some(stamp) = self.level.stamp.clone()
            && stamp.to_string_lossy().ends_with(".prefab.toml")
        {
            return Some(self.relative(&stamp));
        }
        self.brush_layer()?.palette.values().next().cloned()
    }

    pub fn cell_under(&mut self, at: Pos2) -> Option<[i32; 2]> {
        let layer = self.brush_layer()?;
        let lens = self.lens()?;
        let ray = lens.ray(at);
        let [across, up] = tiles::axes(&layer);
        let normal =
            DVec3::from_array(across.map(f64::from)).cross(DVec3::from_array(up.map(f64::from)));
        let point = snap::plane(&ray, DVec3::from_array(layer.origin.map(f64::from)), normal)?;
        Some(tiles::cell_at(&layer, point.to_array()))
    }

    pub fn paint(&mut self, cells: &[[i32; 2]], erase: bool) -> Result<usize, String> {
        if !self.editable() {
            return Err("paint: the scene is playing".to_string());
        }
        self.end_group();
        let mut layer = self
            .brush_layer()
            .ok_or("paint: this scene has no tile layer ([[tiles]])")?;
        let mut joined = None;
        let key = if erase {
            None
        } else {
            let prefab = self
                .brush_prefab()
                .ok_or("paint: choose a prefab for the brush first")?;
            match tiles::key(&layer, &prefab) {
                Some(key) => Some(key),
                None => {
                    let key = tiles::free_key(&layer).ok_or_else(|| {
                        format!("paint: layer {}: its palette is full", layer.name)
                    })?;
                    layer.palette.insert(key.clone(), prefab.clone());
                    joined = Some((key.clone(), prefab));
                    Some(key)
                }
            }
        };
        let changes: Vec<([i32; 2], Option<&str>)> = cells
            .iter()
            .filter(|cell| tiles::get(&layer, **cell) != key)
            .map(|cell| (*cell, key.as_deref()))
            .collect();
        if changes.is_empty() {
            return Ok(0);
        }
        let painted = tiles::painted(&layer, &changes)?;
        let count = changes.len();
        let verb = if erase { "erase" } else { "paint" };
        let label = format!(
            "{verb} {count} {} in {}",
            if count == 1 { "tile" } else { "tiles" },
            layer.name
        );
        let target = Target::Tiles(layer.name.clone());
        let old = layer.rows.clone().unwrap_or_default();
        let found = self.edit.dry_run(|edit| {
            if let Some((key, prefab)) = &joined {
                edit.set(&target, &["palette", key], prefab.as_str())?;
            }
            match &painted {
                tiles::Painted::Rows { rows, origin } => {
                    if *origin != layer.origin {
                        edit.set(&target, &["origin"], *origin)?;
                    }
                    if rows.len() == old.len() {
                        for (place, row) in rows.iter().enumerate() {
                            if row != &old[place] {
                                let place = place.to_string();
                                edit.set(&target, &["rows", &place], row.as_str())?;
                            }
                        }
                    } else {
                        let rows = rows.iter().map(|row| Value::from(row.as_str())).collect();
                        edit.set(&target, &["rows"], Value::Array(rows))?;
                    }
                }
                tiles::Painted::Cells(cells) => {
                    let cells = cells
                        .iter()
                        .map(|cell| {
                            Value::Table(vec![
                                (
                                    "at".to_string(),
                                    Value::Array(vec![
                                        Value::Int(i64::from(cell.at[0])),
                                        Value::Int(i64::from(cell.at[1])),
                                    ]),
                                ),
                                ("tile".to_string(), Value::from(cell.tile.as_str())),
                            ])
                        })
                        .collect();
                    edit.set(&target, &["cells"], Value::Array(cells))?;
                }
            }
            Ok(())
        });
        match found {
            Ok(group) if group.is_empty() => Ok(0),
            Ok(group) => {
                self.land(labelled(group, label));
                Ok(count)
            }
            Err(error) => {
                self.log.edit_error(&error);
                Err(error.to_string())
            }
        }
    }

    pub fn pick_cell(&mut self, cell: [i32; 2]) -> Option<String> {
        let layer = self.brush_layer()?;
        let key = tiles::get(&layer, cell)?;
        let prefab = layer.palette.get(&key)?.clone();
        self.level.brush.prefab = Some(prefab.clone());
        self.level.brush.tool = Tool::Paint;
        self.log.info(format!("brush: {prefab}"));
        Some(prefab)
    }

    pub fn finish_stroke(&mut self) {
        let Some(stroke) = self.level.brush.stroke.take() else {
            return;
        };
        let cells = stroke.cells();
        let outcome = match self.level.brush.tool {
            Tool::Pick => {
                self.pick_cell(stroke.to);
                return;
            }
            Tool::Paint => self.paint(&cells, false),
            Tool::Erase => self.paint(&cells, true),
        };
        if let Err(error) = outcome {
            self.log.warn(error);
        }
    }

    pub fn toggle_brush(&mut self) {
        let brush = &mut self.level.brush;
        brush.on = !brush.on;
        brush.stroke = None;
        brush.grab = None;
        if brush.on && self.edit.resolved().tiles.is_empty() {
            self.log
                .warn("brush: this scene has no tile layer; add a [[tiles]] table to it");
        }
    }
}

impl Editor {
    fn salted(
        &mut self,
        none: String,
        mut attempt: impl FnMut(&mut Self, u32) -> Result<PatchGroup, EditError>,
    ) -> Result<PatchGroup, String> {
        for salt in 0..TRIES {
            match attempt(self, salt) {
                Err(error) if taken(&error) => continue,
                other => return other.map_err(|error| error.to_string()),
            }
        }
        Err(none)
    }
}

pub fn labelled(mut group: PatchGroup, label: String) -> PatchGroup {
    for patch in &mut group.patches {
        patch.label = label.clone();
    }
    group.label = label;
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_stems_and_strokes() {
        assert_eq!(stem(Path::new("a/block.prefab.toml")), "block");
        assert_eq!(stem(Path::new("Rock.GLB")), "Rock");
        assert!(placeable(Path::new("x/rock.gltf")));
        assert!(!placeable(Path::new("x/room.scene.toml")));
        let taken = ["block", "block 2"];
        assert_eq!(unique(|name| taken.contains(&name), "block"), "block 3");
        assert_eq!(unique(|_| false, " "), "object");
        assert_eq!(base("block 12"), "block");
        assert_eq!(base("block two"), "block two");
        assert_eq!(base(" 2"), " 2");
        let mut free = Stroke::new(Shape::Free, [0, 0]);
        free.reach([3, 0]);
        free.reach([3, 0]);
        free.reach([2, 0]);
        assert_eq!(free.cells(), [[0, 0], [1, 0], [2, 0], [3, 0]]);
        let mut line = Stroke::new(Shape::Line, [0, 0]);
        line.reach([2, 2]);
        assert_eq!(line.cells(), [[0, 0], [1, 1], [2, 2]]);
        let mut rectangle = Stroke::new(Shape::Rectangle, [0, 0]);
        rectangle.reach([1, 1]);
        assert_eq!(rectangle.cells().len(), 4);
        assert_eq!(round(0.123456), 0.1235);
        assert_eq!(round(-0.00001), 0.0);
        assert!(!Level::new().shown());
    }
}
