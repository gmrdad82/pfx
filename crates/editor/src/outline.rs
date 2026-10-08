use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pfx_load::scene::tiles::{Tiles, cell_of};
use pfx_load::scene::{Scene, Target};
use pfx_play::Tunables;
use pfx_project::Project;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Item {
    Object(String),
    Mesh(String),
    Node { mesh: String, node: String },
    Light(String),
    Emitter(String),
    Material(String),
    Sound(String),
    Tunables(String),
    File(PathBuf),
    Scene(PathBuf),
    Folder(PathBuf),
    Asset(PathBuf),
    Layer(String),
    Sun,
    Sky,
    Haze,
    Camera,
    Finish,
}

impl Item {
    pub fn target(&self) -> Option<Target> {
        Some(match self {
            Item::Object(name) => Target::Object(name.clone()),
            Item::Mesh(name) => Target::Mesh(name.clone()),
            Item::Node { mesh, node } => Target::Node {
                mesh: mesh.clone(),
                node: node.clone(),
            },
            Item::Light(name) => Target::Light(name.clone()),
            Item::Emitter(name) => Target::Emitter(name.clone()),
            Item::Material(name) => Target::Material(name.clone()),
            Item::Sound(name) => Target::Sound(name.clone()),
            Item::Sun => Target::Sun,
            Item::Sky => Target::Sky,
            Item::Haze => Target::Haze,
            Item::Camera => Target::Camera,
            Item::Finish => Target::Finish,
            Item::File(_)
            | Item::Tunables(_)
            | Item::Scene(_)
            | Item::Folder(_)
            | Item::Asset(_)
            | Item::Layer(_) => return None,
        })
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Item::Object(_) => "object",
            Item::Mesh(_) => "mesh",
            Item::Node { .. } => "node",
            Item::Light(_) => "light",
            Item::Emitter(_) => "emitter",
            Item::Material(_) => "material",
            Item::Sound(_) => "sound",
            Item::Tunables(_) => "tunables",
            Item::File(_) => "file",
            Item::Scene(_) => "scene",
            Item::Folder(_) => "folder",
            Item::Asset(_) => "asset",
            Item::Layer(_) => "layer",
            Item::Sun => "sun",
            Item::Sky => "sky",
            Item::Haze => "haze",
            Item::Camera => "camera",
            Item::Finish => "finish",
        }
    }

    pub fn name(&self) -> String {
        match self {
            Item::Object(name)
            | Item::Mesh(name)
            | Item::Light(name)
            | Item::Emitter(name)
            | Item::Material(name)
            | Item::Sound(name)
            | Item::Tunables(name)
            | Item::Layer(name) => name.clone(),
            Item::Node { mesh, node } => format!("{mesh}/{node}"),
            Item::File(path) | Item::Scene(path) | Item::Folder(path) | Item::Asset(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Item::Sun | Item::Sky | Item::Haze | Item::Camera | Item::Finish => String::new(),
        }
    }

    pub fn key(&self) -> String {
        let name = self.name();
        if name.is_empty() {
            self.kind().to_string()
        } else {
            format!("{}:{name}", self.kind())
        }
    }

    pub fn exists(&self, scene: &Scene) -> bool {
        match self {
            Item::Object(name) => scene.object(name).is_some(),
            Item::Mesh(name) => scene.meshes.contains_key(name),
            Item::Node { mesh, node } => scene
                .meshes
                .get(mesh)
                .is_some_and(|found| found.parts.iter().any(|part| &part.node == node)),
            Item::Light(name) => scene.lights.contains_key(name),
            Item::Emitter(name) => scene.emitters.contains_key(name),
            Item::Material(name) => scene.library.contains(name),
            Item::Sound(name) => scene.sounds.contains_key(name),
            Item::Tunables(_) => true,
            Item::File(path) => scene.files.contains_key(path),
            Item::Scene(_) | Item::Folder(_) | Item::Asset(_) | Item::Layer(_) => true,
            Item::Sun | Item::Sky | Item::Haze | Item::Camera | Item::Finish => true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub item: Item,
    pub label: String,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub title: &'static str,
    pub nodes: Vec<Node>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outline {
    pub sections: Vec<Section>,
}

fn leaf(item: Item, label: impl Into<String>) -> Node {
    Node {
        item,
        label: label.into(),
        children: Vec::new(),
    }
}

fn object_tree(scene: &Scene) -> Vec<Node> {
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut roots = Vec::new();
    for object in &scene.objects {
        match &object.parent {
            Some(parent) if scene.object(parent).is_some() => {
                children.entry(parent).or_default().push(&object.name)
            }
            _ => roots.push(object.name.as_str()),
        }
    }
    fn grow(name: &str, children: &BTreeMap<&str, Vec<&str>>) -> Node {
        Node {
            item: Item::Object(name.to_string()),
            label: name.to_string(),
            children: children
                .get(name)
                .map(|names| names.iter().map(|child| grow(child, children)).collect())
                .unwrap_or_default(),
        }
    }
    roots
        .into_iter()
        .map(|name| grow(name, &children))
        .collect()
}

fn mesh_nodes(scene: &Scene) -> Vec<Node> {
    scene
        .meshes
        .iter()
        .map(|(name, mesh)| {
            let mut nodes: Vec<&str> = Vec::new();
            for part in &mesh.parts {
                if !nodes.contains(&part.node.as_str()) {
                    nodes.push(&part.node);
                }
            }
            Node {
                item: Item::Mesh(name.clone()),
                label: name.clone(),
                children: nodes
                    .into_iter()
                    .map(|node| {
                        leaf(
                            Item::Node {
                                mesh: name.clone(),
                                node: node.to_string(),
                            },
                            node,
                        )
                    })
                    .collect(),
            }
        })
        .collect()
}

pub fn relative(root: &Path, file: &Path) -> String {
    let base = root.parent().unwrap_or(Path::new(""));
    file.strip_prefix(base)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

impl Outline {
    pub fn of(scene: &Scene, files: &[PathBuf], tunables: &Tunables) -> Outline {
        let lights = scene
            .lights
            .keys()
            .map(|name| leaf(Item::Light(name.clone()), name))
            .chain(
                scene
                    .emitters
                    .keys()
                    .map(|name| leaf(Item::Emitter(name.clone()), name)),
            )
            .collect();
        let materials = scene
            .library
            .names()
            .map(|name| leaf(Item::Material(name.to_string()), name))
            .collect();
        let world = vec![
            leaf(Item::Sun, "sun"),
            leaf(Item::Sky, "sky"),
            leaf(Item::Haze, "haze"),
            leaf(Item::Camera, "camera"),
            leaf(Item::Finish, "finish"),
        ];
        let files = files
            .iter()
            .map(|file| leaf(Item::File(file.clone()), relative(&scene.path, file)))
            .collect();
        let sounds: Vec<Node> = scene
            .sounds
            .keys()
            .map(|name| leaf(Item::Sound(name.clone()), name))
            .collect();
        let groups: Vec<Node> = tunables
            .groups()
            .into_iter()
            .map(|(group, _)| {
                let label = if group.is_empty() { "general" } else { group };
                leaf(Item::Tunables(group.to_string()), label)
            })
            .collect();
        let mut outline = Outline {
            sections: vec![
                Section {
                    title: "objects",
                    nodes: object_tree(scene),
                },
                Section {
                    title: "meshes",
                    nodes: mesh_nodes(scene),
                },
                Section {
                    title: "lights",
                    nodes: lights,
                },
                Section {
                    title: "materials",
                    nodes: materials,
                },
                Section {
                    title: "world",
                    nodes: world,
                },
                Section {
                    title: "files",
                    nodes: files,
                },
            ],
        };
        for (after, title, nodes) in [("lights", "sounds", sounds), ("world", "tunables", groups)] {
            if nodes.is_empty() {
                continue;
            }
            let at = outline
                .sections
                .iter()
                .position(|section| section.title == after)
                .map_or(outline.sections.len(), |at| at + 1);
            outline.sections.insert(at, Section { title, nodes });
        }
        outline
    }

    pub fn add_project(&mut self, project: &Project, open: &Path) {
        let scenes = project
            .scenes()
            .into_iter()
            .map(|scene| {
                let mut label = project.relative(&scene);
                if scene == open {
                    label.push_str("  ·  open");
                }
                leaf(Item::Scene(scene), label)
            })
            .collect();
        let content = project
            .folders()
            .into_iter()
            .map(|folder| Node {
                label: match project.relative(&folder.path) {
                    relative if relative.is_empty() => ".".to_string(),
                    relative => relative,
                },
                children: folder
                    .files
                    .iter()
                    .map(|file| {
                        let name = file
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        leaf(Item::Asset(file.clone()), name)
                    })
                    .collect(),
                item: Item::Folder(folder.path),
            })
            .collect();
        self.sections.splice(
            0..0,
            [
                Section {
                    title: "scenes",
                    nodes: scenes,
                },
                Section {
                    title: "content",
                    nodes: content,
                },
            ],
        );
    }

    pub fn add_tiles(&mut self, tiles: &[Tiles]) {
        let layers: Vec<&str> = tiles.iter().map(|layer| layer.name.as_str()).collect();
        if let Some(objects) = self
            .sections
            .iter_mut()
            .find(|section| section.title == "objects")
        {
            objects.nodes.retain(|node| match &node.item {
                Item::Object(name) => {
                    cell_of(name).is_none_or(|(layer, _)| !layers.contains(&layer.as_str()))
                }
                _ => true,
            });
        }
        if tiles.is_empty() {
            return;
        }
        let nodes = tiles
            .iter()
            .map(|layer| {
                let count = layer.cells().len();
                leaf(
                    Item::Layer(layer.name.clone()),
                    format!(
                        "{}  ·  {count} {}",
                        layer.name,
                        if count == 1 { "cell" } else { "cells" }
                    ),
                )
            })
            .collect();
        let at = self
            .sections
            .iter()
            .position(|section| section.title == "objects")
            .map_or(0, |at| at + 1);
        self.sections.insert(
            at,
            Section {
                title: "tiles",
                nodes,
            },
        );
    }

    pub fn items(&self) -> Vec<&Item> {
        fn walk<'a>(nodes: &'a [Node], into: &mut Vec<&'a Item>) {
            for node in nodes {
                into.push(&node.item);
                walk(&node.children, into);
            }
        }
        let mut items = Vec::new();
        for section in &self.sections {
            walk(&section.nodes, &mut items);
        }
        items
    }

    pub fn contains(&self, item: &Item) -> bool {
        self.items().contains(&item)
    }

    pub fn section(&self, title: &str) -> Option<&Section> {
        self.sections.iter().find(|section| section.title == title)
    }

    pub fn path_to(&self, item: &Item) -> Vec<Item> {
        fn find(nodes: &[Node], item: &Item, trail: &mut Vec<Item>) -> bool {
            for node in nodes {
                trail.push(node.item.clone());
                if &node.item == item || find(&node.children, item, trail) {
                    return true;
                }
                trail.pop();
            }
            false
        }
        for section in &self.sections {
            let mut trail = Vec::new();
            if find(&section.nodes, item, &mut trail) {
                return trail;
            }
        }
        Vec::new()
    }

    pub fn step(&self, from: Option<&Item>, by: isize) -> Option<Item> {
        let items = self.items();
        if items.is_empty() {
            return None;
        }
        let at = from
            .and_then(|item| items.iter().position(|candidate| *candidate == item))
            .map(|at| at as isize + by)
            .unwrap_or(if by < 0 { items.len() as isize - 1 } else { 0 });
        let at = at.clamp(0, items.len() as isize - 1) as usize;
        Some(items[at].clone())
    }
}
