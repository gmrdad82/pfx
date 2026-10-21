use crate::Error;
use crate::vector::single::{cross, dot, sub, unit};
use pfx_build::{Look, Part};
use pfx_geom::mesh::Mesh as Geometry;
use pfx_glb::{DMat4, Node};
use pfx_live::frame::Matrix;
use pfx_materials::{Blend, Family, Material, Wet};
use pfx_mesh::{Mesh, Slot};
use pfx_recipe::{Recipe, Setting, Settings, Shadow};
use std::collections::{BTreeMap, BTreeSet};

const DEFAULT_MATERIAL: &str = "default";
const DEGENERATE: f32 = 1e-14;

pub struct Piece {
    pub geometry: Geometry,
    pub material: String,
    pub object: usize,
    pub name: String,
    pub shadow: Shadow,
    pub two_sided: bool,
    pub look: Option<Look>,
}

pub fn pieces(parts: &[Part]) -> Vec<Piece> {
    let mut out = Vec::new();
    for part in parts.iter().filter(|part| !part.hidden) {
        push_pieces(&mut out, part, placed(part));
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Crowd {
    pub key: (u32, u32),
    pub worlds: Vec<DMat4>,
}

impl Crowd {
    pub fn models(&self) -> Vec<Matrix> {
        self.worlds
            .iter()
            .map(|world| {
                world
                    .to_cols_array_2d()
                    .map(|column| column.map(|v| v as f32))
            })
            .collect()
    }
}

pub struct Split {
    pub pieces: Vec<Piece>,
    pub crowd_of: Vec<Option<usize>>,
    pub crowds: Vec<Crowd>,
}

pub fn split(parts: &[Part], baked: &BTreeSet<(u32, u32)>) -> Split {
    let mut pieces = Vec::new();
    let mut groups: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    let mut firsts: Vec<&Part> = Vec::new();
    let mut crowds: Vec<Crowd> = Vec::new();
    for part in parts.iter().filter(|part| !part.hidden) {
        let world = placed(part);
        let key = part
            .scattered
            .map(|copy| (copy.scatter, copy.piece))
            .filter(|key| !baked.contains(key))
            .filter(|_| world.determinant() > 0.0 && world.is_finite());
        let Some(key) = key else {
            push_pieces(&mut pieces, part, world);
            continue;
        };
        let at = *groups.entry(key).or_insert_with(|| {
            firsts.push(part);
            crowds.push(Crowd {
                key,
                worlds: Vec::new(),
            });
            crowds.len() - 1
        });
        crowds[at].worlds.push(world);
    }
    let mut crowd_of = vec![None; pieces.len()];
    for (at, part) in firsts.into_iter().enumerate() {
        let before = pieces.len();
        push_pieces(&mut pieces, part, DMat4::IDENTITY);
        crowd_of.extend(std::iter::repeat_n(Some(at), pieces.len() - before));
    }
    Split {
        pieces,
        crowd_of,
        crowds,
    }
}

fn placed(part: &Part) -> DMat4 {
    if part.node.keep_transform {
        part.node.transform
    } else {
        DMat4::IDENTITY
    }
}

fn push_pieces(out: &mut Vec<Piece>, part: &Part, world: DMat4) {
    let node = &part.node;
    let normals = world.inverse().transpose();
    let flip = world.determinant() < 0.0;
    let positions: Vec<[f32; 3]> = node
        .mesh
        .positions
        .iter()
        .map(|p| {
            world
                .transform_point3([p[0], p[1], p[2]].map(f64::from).into())
                .to_array()
                .map(|v| v as f32)
        })
        .collect();
    let turned: Vec<[f32; 3]> = node
        .mesh
        .normals
        .iter()
        .map(|n| {
            let n = normals
                .transform_vector3([n[0], n[1], n[2]].map(f64::from).into())
                .normalize_or_zero();
            n.to_array().map(|v| v as f32)
        })
        .collect();
    let whole = [Slot {
        material: DEFAULT_MATERIAL.into(),
        start: 0,
        count: node.mesh.indices.len() as u32,
    }];
    let slots = if node.mesh.slots.is_empty() {
        &whole[..]
    } else {
        &node.mesh.slots[..]
    };
    let uvs = node.mesh.uvs.as_deref().unwrap_or_default();
    for slot in slots {
        let start = (slot.start as usize).min(node.mesh.indices.len());
        let end = (start + slot.count as usize).min(node.mesh.indices.len());
        let indices = kept(&positions, &node.mesh.indices[start..end], flip);
        if indices.is_empty() {
            continue;
        }
        let geometry = if turned.len() == positions.len() {
            compact(&positions, &turned, uvs, &indices)
        } else {
            flat(&positions, uvs, &indices)
        };
        out.push(Piece {
            geometry,
            material: slot.material.clone(),
            object: part.object,
            name: node.name.clone(),
            shadow: part.shadow,
            two_sided: part.two_sided,
            look: part.looks.get(&slot.material).cloned(),
        });
    }
}

fn kept(positions: &[[f32; 3]], indices: &[u32], flip: bool) -> Vec<u32> {
    let mut out = Vec::with_capacity(indices.len());
    for tri in indices.chunks_exact(3) {
        if tri.iter().any(|&i| i as usize >= positions.len()) {
            continue;
        }
        let p: [[f32; 3]; 3] = std::array::from_fn(|k| positions[tri[k] as usize]);
        let c = cross(sub(p[1], p[0]), sub(p[2], p[0]));
        if dot(c, c).sqrt() < DEGENERATE {
            continue;
        }
        if flip {
            out.extend_from_slice(&[tri[0], tri[2], tri[1]]);
        } else {
            out.extend_from_slice(tri);
        }
    }
    out
}

fn compact(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Geometry {
    let mut map = vec![u32::MAX; positions.len()];
    let mut p = Vec::new();
    let mut n = Vec::new();
    let mut t = Vec::new();
    let mut out = Vec::with_capacity(indices.len());
    for &index in indices {
        let slot = &mut map[index as usize];
        if *slot == u32::MAX {
            *slot = p.len() as u32;
            p.push(positions[index as usize]);
            n.push(normals[index as usize]);
            t.push(uvs.get(index as usize).copied().unwrap_or([0.0; 2]));
        }
        out.push(*slot);
    }
    Geometry::new(p, n, Vec::new(), t, out)
}

fn flat(positions: &[[f32; 3]], uvs: &[[f32; 2]], indices: &[u32]) -> Geometry {
    let mut p = Vec::with_capacity(indices.len());
    let mut n = Vec::with_capacity(indices.len());
    let mut t = Vec::with_capacity(indices.len());
    for tri in indices.chunks_exact(3) {
        let corners: [[f32; 3]; 3] = std::array::from_fn(|k| positions[tri[k] as usize]);
        let normal = unit(cross(
            sub(corners[1], corners[0]),
            sub(corners[2], corners[0]),
        ));
        for (corner, &index) in corners.iter().zip(tri) {
            p.push(*corner);
            n.push(normal);
            t.push(uvs.get(index as usize).copied().unwrap_or([0.0; 2]));
        }
    }
    let out = (0..p.len() as u32).collect();
    Geometry::new(p, n, Vec::new(), t, out)
}

pub fn material(recipe: &Recipe, name: &str) -> Result<Option<Material>, Error> {
    recipe
        .materials
        .get(name)
        .map(|value| {
            serde_json::from_value::<Material>(value.clone())
                .map_err(|e| Error(format!("materials.{name}: {e}")))
        })
        .transpose()
        .map(|found| {
            found.map(|mut material| {
                if let Some(settings) = recipe.effects.wet.get(name) {
                    material.wet = wet(settings);
                }
                material
            })
        })
}

pub fn wet(settings: &Settings) -> Wet {
    let mut wet = Wet::default();
    for (key, value) in settings {
        match (*key, value) {
            ("amount", Setting::Number(v)) => wet.amount = *v as f32,
            ("puddles", Setting::Number(v)) => wet.puddles = *v as f32,
            ("puddle_scale", Setting::Number(v)) => wet.puddle_scale = *v as f32,
            ("seed", Setting::Whole(v)) => wet.seed = *v,
            _ => {}
        }
    }
    wet
}

fn missing(recipe: &Recipe, piece: &Piece) -> Error {
    Error(format!(
        "object {}: {}",
        piece.name,
        pfx_recipe::missing_material_text(
            recipe.materials.keys().map(String::as_str),
            &piece.material
        )
    ))
}

pub struct Palette {
    pub names: Vec<String>,
    pub materials: Vec<Material>,
    pub slots: Vec<u32>,
}

pub fn palette(
    recipe: &Recipe,
    pieces: &[Piece],
    carded: impl IntoIterator<Item = bool>,
) -> Result<Palette, Error> {
    palette_with(
        recipe,
        pieces,
        carded.into_iter().map(|card| card.then_some(Blend::Over)),
    )
}

pub fn palette_with(
    recipe: &Recipe,
    pieces: &[Piece],
    layers: impl IntoIterator<Item = Option<Blend>>,
) -> Result<Palette, Error> {
    let mut keys: Vec<(&str, Option<Blend>)> = Vec::new();
    let mut names = Vec::new();
    let mut materials = Vec::new();
    let mut slots = Vec::with_capacity(pieces.len());
    for (piece, layer) in pieces.iter().zip(layers) {
        let key = (piece.material.as_str(), layer);
        let index = match keys.iter().position(|known| *known == key) {
            Some(index) => index,
            None => {
                let Some(material) = material(recipe, &piece.material)? else {
                    return Err(missing(recipe, piece));
                };
                keys.push(key);
                names.push(piece.material.clone());
                materials.push(match layer {
                    Some(blend) => crate::cards::layered_as(material, blend),
                    None => material,
                });
                keys.len() - 1
            }
        };
        slots.push(index as u32);
    }
    Ok(Palette {
        names,
        materials,
        slots,
    })
}

pub fn glassy(material: &Material) -> bool {
    material.refracts() || material.family == Family::Liquid
}

pub fn glb_parts(bytes: &[u8]) -> Result<Vec<Part>, Error> {
    let mesh = pfx_load::Mesh::parse(bytes).map_err(Error)?;
    let mut objects: Vec<String> = Vec::new();
    let mut parts = Vec::new();
    let level = |root: &u32| {
        mesh.nodes
            .get(*root as usize)
            .and_then(|node| node.extras.as_ref())
            .and_then(|extras| extras.get("lod"))
            .is_some_and(|lod| lod.get("of").is_some())
    };
    let mut stack: Vec<(u32, DMat4)> = mesh
        .all_roots()
        .into_iter()
        .rev()
        .filter(|root| !level(root))
        .map(|root| (root, DMat4::IDENTITY))
        .collect();
    let mut visits = 0usize;
    while let Some((index, parent)) = stack.pop() {
        visits += 1;
        if visits > mesh.nodes.len() * 4 + 16 {
            return Err(Error("glb: the node tree has a cycle".into()));
        }
        let Some(node) = mesh.nodes.get(index as usize) else {
            return Err(Error(format!("glb: node {index} does not exist")));
        };
        let local = DMat4::from_cols_array_2d(&node.transform.map(|c| c.map(f64::from)));
        let world = parent * local;
        if let Some(group) = node.group.and_then(|g| mesh.groups.get(g as usize)) {
            let name = if !node.name.is_empty() {
                node.name.clone()
            } else if !group.name.is_empty() {
                group.name.clone()
            } else {
                "object".to_string()
            };
            let object = match objects.iter().position(|o| *o == name) {
                Some(found) => found,
                None => {
                    objects.push(name.clone());
                    objects.len() - 1
                }
            };
            let mut built = Mesh {
                positions: Vec::new(),
                normals: Vec::new(),
                uvs: Some(Vec::new()),
                uvs1: None,
                indices: Vec::new(),
                slots: Vec::new(),
            };
            let mut smooth = true;
            for primitive in &group.primitives {
                let material = primitive
                    .material
                    .and_then(|m| mesh.materials.get(m as usize))
                    .map(|m| m.name.as_str())
                    .filter(|n| !n.is_empty())
                    .unwrap_or(DEFAULT_MATERIAL)
                    .to_string();
                let base = built.positions.len() as u32;
                let start = built.indices.len() as u32;
                smooth &= primitive.normals.len() == primitive.positions.len();
                built.positions.extend(&primitive.positions);
                built.normals.extend(&primitive.normals);
                if let Some(uvs) = &mut built.uvs {
                    uvs.extend(
                        (0..primitive.positions.len())
                            .map(|i| primitive.uvs.get(i).copied().unwrap_or([0.0; 2])),
                    );
                }
                built
                    .indices
                    .extend(primitive.indices.iter().map(|&i| i + base));
                built.slots.push(Slot {
                    material,
                    start,
                    count: primitive.indices.len() as u32,
                });
            }
            if !smooth {
                built.normals.clear();
            }
            parts.push(Part {
                node: Node {
                    name,
                    mesh: built,
                    transform: world,
                    keep_transform: true,
                    targets: None,
                },
                object,
                shadow: Shadow::Cast,
                two_sided: false,
                hidden: false,
                looks: Default::default(),
                scattered: None,
            });
        }
        for &child in node.children.iter().rev() {
            stack.push((child, world));
        }
    }
    Ok(parts)
}

pub fn placed_bounds(
    pieces: &[Piece],
    crowd_of: &[Option<usize>],
    crowds: &[Crowd],
    visible: bool,
) -> Option<[[f64; 3]; 2]> {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    let mut grow = |p: [f64; 3]| {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    };
    for (at, piece) in pieces.iter().enumerate() {
        if visible && piece.shadow == Shadow::Only {
            continue;
        }
        let Some(crowd) = crowd_of.get(at).copied().flatten() else {
            for p in &piece.geometry.positions {
                grow(p.map(f64::from));
            }
            continue;
        };
        for world in &crowds[crowd].worlds {
            for p in &piece.geometry.positions {
                let placed = world.transform_point3(p.map(f64::from).into());
                grow(placed.to_array().map(|v| f64::from(v as f32)));
            }
        }
    }
    lo[0].is_finite().then_some([lo, hi])
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_build::Scattered;
    use pfx_glb::{Order, write};
    use pfx_recipe::parse;

    fn quad(material: &str) -> Mesh {
        Mesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 5],
            uvs1: None,
            uvs: None,
            indices: vec![0, 1, 2, 0, 2, 3, 0, 4, 1],
            slots: vec![Slot {
                material: material.into(),
                start: 0,
                count: 9,
            }],
        }
    }

    fn part(name: &str, object: usize, mesh: Mesh, transform: DMat4, keep: bool) -> Part {
        Part {
            node: Node {
                name: name.into(),
                mesh,
                transform,
                keep_transform: keep,
                targets: None,
            },
            object,
            shadow: Shadow::Cast,
            two_sided: false,
            hidden: false,
            looks: Default::default(),
            scattered: None,
        }
    }

    #[test]
    fn pieces_drop_degenerate_triangles_and_hidden_parts() {
        let mut hidden = part("cutter", 1, quad("chrome"), DMat4::IDENTITY, false);
        hidden.hidden = true;
        let shown = part("plate", 0, quad("brass"), DMat4::IDENTITY, false);
        let out = pieces(&[shown, hidden]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].material, "brass");
        assert_eq!(out[0].geometry.indices.len(), 6);
        assert_eq!(out[0].geometry.positions.len(), 4);
        assert_eq!(out[0].geometry.tangents.len(), 4);
    }

    #[test]
    fn a_kept_transform_moves_and_turns_the_piece() {
        let mirror = DMat4::from_scale([-1.0, 1.0, 1.0].into());
        let moved = DMat4::from_translation([0.0, 2.0, 0.0].into());
        let out = pieces(&[
            part("a", 0, quad("m"), moved, true),
            part("b", 1, quad("m"), moved, false),
            part("c", 2, quad("m"), mirror, true),
        ]);
        assert_eq!(out[0].geometry.positions[0], [0.0, 2.0, 0.0]);
        assert_eq!(out[1].geometry.positions[0], [0.0, 0.0, 0.0]);
        let mirrored = &out[2].geometry;
        let corner = |k: usize| mirrored.positions[mirrored.indices[k] as usize];
        let face = cross(sub(corner(1), corner(0)), sub(corner(2), corner(0)));
        assert!(face[2] > 0.0, "{face:?}");
        assert_eq!(mirrored.normals[0], [0.0, 0.0, 1.0]);
        assert!(mirrored.positions.contains(&[-1.0, 0.0, 0.0]));
    }

    #[test]
    fn materials_resolve_by_name_through_the_recipe_only() {
        let recipe = parse(
            "recipe = 1\nname = \"t\"\n[materials.mine]\nbase = [0.1, 0.2, 0.3]\nroughness = 0.4\n",
        )
        .unwrap();
        for name in ["lacquer_orange", "chrome"] {
            assert_eq!(material(&recipe, name).unwrap(), None, "{name}");
        }
        let mine = material(&recipe, "mine").unwrap().unwrap();
        assert_eq!(mine.base, [0.1, 0.2, 0.3]);
        assert!(mine.wet.inactive());
        let wet = parse(
            "recipe = 1\nname = \"t\"\n[materials.mine]\nbase = [0.1, 0.2, 0.3]\n[materials.mine.wet]\namount = 0.6\npuddles = 0.25\n",
        )
        .unwrap();
        let soaked = material(&wet, "mine").unwrap().unwrap().wet;
        assert_eq!((soaked.amount, soaked.puddles), (0.6, 0.25));
        assert_eq!(soaked.puddle_scale, Wet::default().puddle_scale);
        assert_eq!(material(&recipe, "no_such_material").unwrap(), None);
    }

    #[test]
    fn a_material_in_neither_the_catalogue_nor_the_recipe_is_a_clear_error() {
        let recipe = parse(
            "recipe = 1\nname = \"t\"\n[materials.mine]\nbase = [0.1, 0.2, 0.3]\nroughness = 0.4\n",
        )
        .unwrap();
        let out = pieces(&[part("a", 0, quad("chrome"), DMat4::IDENTITY, false)]);
        let error = palette(&recipe, &out, [false]).err().unwrap();
        assert!(
            error.0.starts_with("object a: material \"chrome\""),
            "{error}"
        );
        assert!(error.0.contains("(it defines mine)"), "{error}");
        assert!(error.0.contains("[materials.chrome]"), "{error}");
        let bare = parse("recipe = 1\nname = \"t\"\n").unwrap();
        let error = palette(&bare, &out, [false]).err().unwrap();
        assert!(error.0.contains("(it defines none)"), "{error}");
        let mine = pieces(&[part("a", 0, quad("mine"), DMat4::IDENTITY, false)]);
        assert_eq!(palette(&recipe, &mine, [false]).unwrap().names, ["mine"]);
    }

    #[test]
    fn a_glb_becomes_parts_per_object_in_world_space() {
        let nodes = [
            Node {
                name: "shell".into(),
                mesh: quad("lacquer_orange"),
                transform: DMat4::from_translation([0.0, 0.0, 1.0].into()),
                keep_transform: true,
                targets: None,
            },
            Node {
                name: "knob".into(),
                mesh: quad("brass"),
                transform: DMat4::IDENTITY,
                keep_transform: false,
                targets: None,
            },
        ];
        let bytes = write(&nodes, Order::File, "pfx test");
        let parts = glb_parts(&bytes).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].node.name, "shell");
        assert_eq!(parts[0].object, 0);
        assert_eq!(parts[1].object, 1);
        assert_eq!(parts[0].node.mesh.slots[0].material, "lacquer_orange");
        let out = pieces(&parts);
        assert_eq!(out[0].geometry.positions[0], [0.0, 0.0, 1.0]);
        assert_eq!(out[1].material, "brass");
        assert!(glb_parts(b"not a glb").is_err());
    }

    fn copies(count: u32) -> Vec<Part> {
        (0..count)
            .map(|n| {
                let size = 0.5 + f64::from(n % 5) * 0.25;
                let world =
                    DMat4::from_translation([f64::from(n) * 1.3, 0.1, -f64::from(n) * 0.4].into())
                        * DMat4::from_rotation_y(f64::from(n) * 0.7)
                        * DMat4::from_scale([size; 3].into());
                let mut copy = part(&format!("leaves.{n}"), 1, quad("leaf"), world, true);
                copy.scattered = Some(Scattered {
                    scatter: 0,
                    piece: 0,
                    copy: n,
                });
                copy
            })
            .collect()
    }

    #[test]
    fn scattered_copies_become_one_crowd_whose_pieces_do_not_grow_with_the_copies() {
        let ground = part("ground", 0, quad("stone"), DMat4::IDENTITY, false);
        for count in [1, 10, 500] {
            let mut parts = vec![ground.clone()];
            parts.extend(copies(count));
            let split = split(&parts, &BTreeSet::new());
            assert_eq!(split.pieces.len(), 2);
            assert_eq!(split.crowd_of, [None, Some(0)]);
            assert_eq!(split.crowds.len(), 1);
            assert_eq!(split.crowds[0].worlds.len(), count as usize);
        }
    }

    #[test]
    fn a_crowd_places_exactly_the_vertices_and_bounds_the_baked_copies_held() {
        let parts = copies(20);
        let baked = pieces(&parts);
        let split = split(&parts, &BTreeSet::new());
        let template = &split.pieces[0];
        assert_eq!(baked.len(), 20);
        for (world, piece) in split.crowds[0].worlds.iter().zip(&baked) {
            let placed: Vec<[f32; 3]> = template
                .geometry
                .positions
                .iter()
                .map(|p| {
                    world
                        .transform_point3(p.map(f64::from).into())
                        .to_array()
                        .map(|v| v as f32)
                })
                .collect();
            assert_eq!(placed, piece.geometry.positions);
            assert_eq!(template.geometry.indices, piece.geometry.indices);
        }
        for visible in [false, true] {
            assert_eq!(
                placed_bounds(&split.pieces, &split.crowd_of, &split.crowds, visible),
                placed_bounds(&baked, &vec![None; baked.len()], &[], visible)
            );
        }
    }

    #[test]
    fn mirrored_or_refused_copies_stay_baked_pieces() {
        let mut parts = copies(4);
        parts[2].node.transform = DMat4::from_scale([-1.0, 1.0, 1.0].into());
        let mixed = split(&parts, &BTreeSet::new());
        assert_eq!(mixed.crowds[0].worlds.len(), 3);
        assert_eq!(mixed.crowd_of, [None, Some(0)]);
        let baked = split(&parts, &BTreeSet::from([(0, 0)]));
        assert!(baked.crowds.is_empty());
        assert_eq!(baked.pieces.len(), 4);
        assert_eq!(baked.crowd_of, [None; 4]);
    }
}
