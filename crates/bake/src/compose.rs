use std::collections::{BTreeMap, BTreeSet};
use std::f32::consts::{PI, TAU};
use std::path::{Path, PathBuf};

use pfx_load::{Mesh, Node};
use pfx_materials::{Library, Material};
use pfx_trace::bvh::Triangle;

use crate::recipe::{Recipe, materials as factor_materials, read_inside};

pub(crate) type Matrix = [[f32; 4]; 4];

const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

const EMITTER_BASE: u32 = 0x8000_0000;

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn unit(v: [f32; 3]) -> [f32; 3] {
    let n = dot(v, v).sqrt().max(1e-20);
    v.map(|x| x / n)
}

pub(crate) fn multiply(a: Matrix, b: Matrix) -> Matrix {
    let mut result = [[0.0; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            result[column][row] = (0..4).map(|k| a[k][row] * b[column][k]).sum();
        }
    }
    result
}

pub(crate) fn transform(matrix: &Matrix, p: [f32; 3]) -> [f32; 3] {
    let point = [p[0], p[1], p[2], 1.0];
    std::array::from_fn(|row| {
        (0..4)
            .map(|column| matrix[column][row] * point[column])
            .sum()
    })
}

pub(crate) fn translated(by: [f32; 3]) -> Matrix {
    let mut matrix = IDENTITY;
    matrix[3] = [by[0], by[1], by[2], 1.0];
    matrix
}

pub(crate) fn posed(yaw: f32, scale: [f32; 3], origin: [f32; 3]) -> Matrix {
    let (s, c) = yaw.sin_cos();
    let rotation = [[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]];
    let column = |j: usize| {
        [
            rotation[j][0] * scale[j],
            rotation[j][1] * scale[j],
            rotation[j][2] * scale[j],
            0.0,
        ]
    };
    [
        column(0),
        column(1),
        column(2),
        [origin[0], origin[1], origin[2], 1.0],
    ]
}

pub(crate) struct Palette<'a> {
    pub library: &'a Library,
    pub fallback: Option<&'a str>,
    pub tint: &'a BTreeMap<String, [f32; 3]>,
}

impl Palette<'_> {
    pub(crate) fn listed(&self, name: &str) -> Option<Material> {
        let mut material = *self.library.get(name)?;
        if let Some(tint) = self.tint.get(name) {
            material.base = [0, 1, 2].map(|k| material.base[k] * tint[k]);
        }
        Some(material)
    }

    pub(crate) fn named(&self, name: &str) -> Material {
        let key = name.split('.').next().unwrap_or(name);
        self.listed(name)
            .or_else(|| self.listed(key))
            .unwrap_or_else(|| self.fallback())
    }

    pub(crate) fn fallback(&self) -> Material {
        self.fallback
            .and_then(|name| self.listed(name))
            .unwrap_or_default()
    }
}

pub(crate) struct Files {
    root: PathBuf,
    pub(crate) sources: BTreeMap<String, Vec<u8>>,
    pub(crate) external: BTreeMap<String, Vec<u8>>,
    meshes: BTreeMap<String, Mesh>,
    factors: BTreeMap<String, Vec<Material>>,
}

impl Files {
    pub(crate) fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            sources: BTreeMap::new(),
            external: BTreeMap::new(),
            meshes: BTreeMap::new(),
            factors: BTreeMap::new(),
        }
    }

    pub(crate) fn load(&mut self, name: &str) -> Result<(), String> {
        if self.meshes.contains_key(name) {
            return Ok(());
        }
        let bytes = read_inside(&self.root, name)?;
        let base = Path::new(name).parent().unwrap_or(Path::new(""));
        let buffers = std::cell::RefCell::new(BTreeMap::<String, Vec<u8>>::new());
        let mesh = Mesh::parse_with(&bytes, |uri| {
            let joined = base.join(uri).to_string_lossy().into_owned();
            let data = read_inside(&self.root, &joined)?;
            buffers.borrow_mut().insert(joined, data.clone());
            Ok(data)
        })?;
        let mut factors = factor_materials(&bytes, None)?;
        factors.pop();
        self.external.extend(buffers.into_inner());
        self.sources.insert(name.to_owned(), bytes);
        self.meshes.insert(name.to_owned(), mesh);
        self.factors.insert(name.to_owned(), factors);
        Ok(())
    }

    pub(crate) fn mesh(&self, name: &str) -> Result<&Mesh, String> {
        self.meshes
            .get(name)
            .ok_or_else(|| format!("{name} was not loaded"))
    }

    fn get(&self, name: &str) -> Result<(&Mesh, &[Material]), String> {
        Ok((self.mesh(name)?, &self.factors[name]))
    }
}

pub(crate) struct Composed {
    pub triangles: Vec<Triangle>,
    pub materials: Vec<Material>,
    pub emitters: Vec<usize>,
}

struct Pose<'a> {
    index: usize,
    matrix: Matrix,
    overrides: &'a BTreeMap<String, String>,
}

struct Build<'a> {
    recipe: &'a Recipe,
    palette: Option<&'a Palette<'a>>,
    names: Vec<String>,
    materials: Vec<Material>,
    triangles: Vec<Triangle>,
}

impl Build<'_> {
    fn material(
        &mut self,
        mesh: &Mesh,
        factors: &[Material],
        file: &str,
        id: Option<u32>,
        pose: Option<&Pose>,
    ) -> (u32, String) {
        let name = id
            .map(|m| mesh.materials[m as usize].name.as_str())
            .filter(|n| !n.is_empty())
            .unwrap_or("default")
            .to_owned();
        let key = match (self.palette.is_some(), pose) {
            (true, Some(pose)) => format!("pose{}/{name}", pose.index),
            (true, None) => name.clone(),
            (false, _) => format!("{file}#{}", id.map_or("none".to_owned(), |i| i.to_string())),
        };
        if let Some(index) = self.names.iter().position(|n| *n == key) {
            return (index as u32, name);
        }
        let material = match self.palette {
            Some(palette) => match pose {
                Some(pose) => palette.named(pose.overrides.get(&name).unwrap_or(&name)),
                None => palette.named(&name),
            },
            None => id
                .and_then(|i| factors.get(i as usize))
                .copied()
                .unwrap_or_default(),
        };
        self.names.push(key);
        self.materials.push(material);
        ((self.names.len() - 1) as u32, name)
    }

    fn group(
        &mut self,
        mesh: &Mesh,
        factors: &[Material],
        file: &str,
        node: &Node,
        world: &Matrix,
        pose: Option<&Pose>,
    ) -> Result<(), String> {
        let Some(group) = node.group else {
            return Ok(());
        };
        let emitter = self
            .recipe
            .emitters
            .iter()
            .position(|e| e.node.as_deref() == Some(node.name.as_str()));
        let hidden = self.recipe.shadow_only_nodes.contains(&node.name);
        for primitive in &mesh.groups[group as usize].primitives {
            let (material, name) = match emitter {
                Some(index) => (EMITTER_BASE + index as u32, String::new()),
                None => self.material(mesh, factors, file, primitive.material, pose),
            };
            if hidden || self.recipe.shadow_only_materials.contains(&name) {
                continue;
            }
            let positions: Vec<[f32; 3]> = primitive
                .positions
                .iter()
                .map(|&p| transform(world, p))
                .collect();
            for indices in primitive.indices.chunks_exact(3) {
                let looked = [0, 1, 2].map(|k| positions.get(indices[k] as usize).copied());
                let [Some(a), Some(b), Some(c)] = looked else {
                    return Err("mesh has an index outside its vertices".into());
                };
                let mut vertices = [a, b, c];
                if self.recipe.drop_degenerate {
                    let c = cross(sub(vertices[1], vertices[0]), sub(vertices[2], vertices[0]));
                    if dot(c, c).sqrt() < 1e-14 {
                        continue;
                    }
                }
                if let Some(pose) = pose {
                    vertices = vertices.map(|v| transform(&pose.matrix, v));
                }
                self.triangles.push(Triangle { vertices, material });
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn visit(
        &mut self,
        mesh: &Mesh,
        factors: &[Material],
        file: &str,
        index: u32,
        parent: Matrix,
        pose: Option<&Pose>,
        seen: &mut BTreeSet<u32>,
    ) -> Result<(), String> {
        if !seen.insert(index) {
            return Err("mesh scene has repeated nodes".into());
        }
        let node = mesh
            .nodes
            .get(index as usize)
            .ok_or("mesh scene has a missing node")?;
        if pose.is_none()
            && self.recipe.poses.iter().any(|p| {
                p.file.as_deref().unwrap_or(&self.recipe.scene) == file && p.node == node.name
            })
        {
            return Ok(());
        }
        let world = multiply(parent, node.transform);
        self.group(mesh, factors, file, node, &world, pose)?;
        for &child in &node.children {
            self.visit(mesh, factors, file, child, world, pose, seen)?;
        }
        Ok(())
    }
}

fn lift(recipe: &Recipe, file: &str, root_name: &str) -> Matrix {
    recipe
        .files
        .iter()
        .find(|f| f.path == file)
        .and_then(|f| {
            f.translation.filter(|_| {
                !f.keep_prefix
                    .as_deref()
                    .is_some_and(|prefix| root_name.starts_with(prefix))
            })
        })
        .map_or(IDENTITY, translated)
}

fn find(recipe: &Recipe, mesh: &Mesh, file: &str, name: &str) -> Result<(u32, Matrix), String> {
    let mut stack: Vec<(u32, Matrix)> = mesh
        .roots
        .iter()
        .rev()
        .map(|&root| (root, lift(recipe, file, &mesh.nodes[root as usize].name)))
        .collect();
    let mut seen = BTreeSet::new();
    while let Some((index, parent)) = stack.pop() {
        if !seen.insert(index) {
            return Err("mesh scene has repeated nodes".into());
        }
        let node = mesh
            .nodes
            .get(index as usize)
            .ok_or("mesh scene has a missing node")?;
        if node.name == name {
            return Ok((index, parent));
        }
        let world = multiply(parent, node.transform);
        stack.extend(node.children.iter().rev().map(|&child| (child, world)));
    }
    Err(format!("pose names {name}, which {file} does not hold"))
}

pub(crate) fn sphere(centre: [f32; 3], radius: f32, material: u32) -> Vec<Triangle> {
    let rings = 12;
    let segments = 24;
    let at = |i: u32, j: u32| {
        let theta = PI * i as f32 / rings as f32;
        let phi = TAU * j as f32 / segments as f32;
        [
            centre[0] + radius * theta.sin() * phi.cos(),
            centre[1] + radius * theta.cos(),
            centre[2] + radius * theta.sin() * phi.sin(),
        ]
    };
    let mut out = Vec::new();
    for i in 0..rings {
        for j in 0..segments {
            let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
            if i > 0 {
                out.push(Triangle {
                    vertices: [a, c, d],
                    material,
                });
            }
            if i + 1 < rings {
                out.push(Triangle {
                    vertices: [a, b, c],
                    material,
                });
            }
        }
    }
    out
}

pub(crate) fn compose(
    recipe: &Recipe,
    files: &Files,
    palette: Option<&Palette>,
) -> Result<Composed, String> {
    let mut build = Build {
        recipe,
        palette,
        names: Vec::new(),
        materials: Vec::new(),
        triangles: Vec::new(),
    };
    let order = std::iter::once(recipe.scene.as_str())
        .chain(recipe.files.iter().map(|file| file.path.as_str()));
    for path in order {
        let (mesh, factors) = files.get(path)?;
        let mut seen = BTreeSet::new();
        for &root in &mesh.roots {
            let parent = lift(recipe, path, &mesh.nodes[root as usize].name);
            build.visit(mesh, factors, path, root, parent, None, &mut seen)?;
        }
    }
    for (index, entry) in recipe.poses.iter().enumerate() {
        let path = entry.file.as_deref().unwrap_or(&recipe.scene);
        let (mesh, factors) = files.get(path)?;
        let (node, parent) = find(recipe, mesh, path, &entry.node)?;
        let pose = Pose {
            index,
            matrix: posed(entry.yaw, entry.scale, entry.translation),
            overrides: &entry.materials,
        };
        build.visit(
            mesh,
            factors,
            path,
            node,
            parent,
            Some(&pose),
            &mut BTreeSet::new(),
        )?;
    }
    let mut slots = Vec::new();
    for emitter in &recipe.emitters {
        let radiance = match emitter.radius {
            Some(radius) => emitter.intensity / (PI * radius * radius),
            None => emitter.intensity,
        };
        let base = recipe.emitter_base(emitter.preset.as_deref(), palette);
        let index = build.materials.len();
        build.materials.push(Material {
            base: [0.0; 3],
            roughness: 1.0,
            emission: emitter.color.map(|c| c * radiance),
            ..base
        });
        slots.push(index);
        if let (Some(position), Some(radius)) = (emitter.position, emitter.radius) {
            build
                .triangles
                .extend(sphere(position, radius, index as u32));
        }
    }
    for triangle in &mut build.triangles {
        if triangle.material >= EMITTER_BASE {
            triangle.material = slots[(triangle.material - EMITTER_BASE) as usize] as u32;
        }
    }
    if build.triangles.is_empty() {
        return Err("scene has no triangles".into());
    }
    Ok(Composed {
        triangles: build.triangles,
        materials: build.materials,
        emitters: slots,
    })
}
