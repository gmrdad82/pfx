use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pfx_materials::{Library, Material};
use serde_json::{Value, json};

use super::room::{self, Cast, Room};
use super::{ANCHORS, Lens, SAMPLES, SEED, reflections, volumes};

pub const RECIPE: &str = include_str!("bake.toml");
pub const LIBRARY: &str = include_str!("materials.toml");
pub const ROOM: &[u8] = include_bytes!("room.glb");
pub const DETAIL_PARTS: [&str; 6] = [
    "table top",
    "board",
    "sheet aged",
    "back wall",
    "floor",
    "shelf 1",
];

pub fn library_text(materials: &[(&str, Material)]) -> String {
    let mut library = Library::new();
    for (name, material) in materials {
        library.insert(name, *material).unwrap();
    }
    library.to_toml().unwrap()
}

pub fn library() -> Library {
    Library::from_toml(LIBRARY).unwrap()
}

fn floats(values: impl IntoIterator<Item = f32>) -> Vec<u8> {
    values.into_iter().flat_map(f32::to_le_bytes).collect()
}

pub fn glb(room: &Room) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let mut views: Vec<Value> = Vec::new();
    let mut accessors: Vec<Value> = Vec::new();
    let mut view = |bin: &mut Vec<u8>, bytes: Vec<u8>, target: u32| {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        views.push(json!({
            "buffer": 0,
            "byteOffset": bin.len(),
            "byteLength": bytes.len(),
            "target": target,
        }));
        bin.extend(bytes);
        views.len() - 1
    };
    let mut shared = Vec::new();
    for (_, geometry) in &room.meshes {
        let mesh = &geometry.mesh;
        let count = mesh.positions.len();
        let lo: [f32; 3] =
            std::array::from_fn(|k| mesh.positions.iter().map(|p| p[k]).fold(f32::MAX, f32::min));
        let hi: [f32; 3] =
            std::array::from_fn(|k| mesh.positions.iter().map(|p| p[k]).fold(f32::MIN, f32::max));
        let positions = view(
            &mut bin,
            floats(mesh.positions.iter().flatten().copied()),
            34962,
        );
        let normals = view(
            &mut bin,
            floats(mesh.normals.iter().flatten().copied()),
            34962,
        );
        let uvs = view(&mut bin, floats(mesh.uvs.iter().flatten().copied()), 34962);
        let uvs1 = geometry
            .uvs1
            .as_ref()
            .map(|uvs1| view(&mut bin, floats(uvs1.iter().flatten().copied()), 34962));
        let indices = view(
            &mut bin,
            mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect(),
            34963,
        );
        let first = accessors.len();
        accessors.push(json!({"bufferView": positions, "componentType": 5126, "count": count, "type": "VEC3", "min": lo, "max": hi}));
        accessors.push(
            json!({"bufferView": normals, "componentType": 5126, "count": count, "type": "VEC3"}),
        );
        accessors.push(
            json!({"bufferView": uvs, "componentType": 5126, "count": count, "type": "VEC2"}),
        );
        accessors.push(json!({"bufferView": indices, "componentType": 5125, "count": mesh.indices.len(), "type": "SCALAR"}));
        let second = uvs1.map(|uvs1| {
            accessors.push(
                json!({"bufferView": uvs1, "componentType": 5126, "count": count, "type": "VEC2"}),
            );
            accessors.len() - 1
        });
        shared.push((first, second));
    }
    let mut meshes: Vec<Value> = Vec::new();
    let mut made: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut nodes: Vec<Value> = Vec::new();
    for part in &room.parts {
        let key = (part.mesh, part.material);
        let mesh = *made.entry(key).or_insert_with(|| {
            let (first, second) = shared[part.mesh];
            let mut attributes =
                json!({"POSITION": first, "NORMAL": first + 1, "TEXCOORD_0": first + 2});
            if let Some(second) = second {
                attributes["TEXCOORD_1"] = json!(second);
            }
            meshes.push(json!({
                "name": format!("{}-{}", room.meshes[part.mesh].0, room.materials[part.material].0),
                "primitives": [{
                    "attributes": attributes,
                    "indices": first + 3,
                    "material": part.material,
                }],
            }));
            meshes.len() - 1
        });
        let matrix: Vec<f32> = part.model.iter().flatten().copied().collect();
        nodes.push(json!({"name": part.name, "mesh": mesh, "matrix": matrix}));
    }
    let materials: Vec<Value> = room
        .materials
        .iter()
        .map(|(name, m)| {
            json!({
                "name": name,
                "pbrMetallicRoughness": {
                    "baseColorFactor": [m.base[0], m.base[1], m.base[2], 1.0],
                    "metallicFactor": m.metalness,
                    "roughnessFactor": m.roughness,
                },
                "emissiveFactor": m.emission.map(|v| v.min(1.0)),
            })
        })
        .collect();
    let document = json!({
        "asset": {"version": "2.0", "generator": "pfx bench"},
        "scene": 0,
        "scenes": [{"nodes": (0..nodes.len()).collect::<Vec<_>>()}],
        "nodes": nodes,
        "meshes": meshes,
        "materials": materials,
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{"byteLength": bin.len()}],
    });
    let mut text = serde_json::to_vec(&document).unwrap();
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = 12 + 8 + text.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&text);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

fn list<T: std::fmt::Debug>(values: &[T]) -> String {
    format!("{values:?}")
}

fn quoted(values: &[&str]) -> String {
    let inner: Vec<String> = values.iter().map(|v| format!("\"{v}\"")).collect();
    format!("[{}]", inner.join(", "))
}

pub fn recipe_text(room: &Room) -> String {
    let shadow_only: Vec<&str> = room
        .parts
        .iter()
        .filter(|part| part.cast == Cast::Only)
        .map(|part| part.name.as_str())
        .collect();
    let lens = Lens::default();
    let mut out = format!(
        "app = \"bench\"\nscene = \"room.glb\"\nmaterials = \"materials.toml\"\nfallback = \"plain\"\nanchors = {}\nreference_hour = 12.0\nsamples = {SAMPLES}\nseed = {SEED}\nshadow_only_nodes = {}\n\n",
        list(&ANCHORS),
        quoted(&shadow_only),
    );
    out.push_str("[sky]\nkind = \"daylight\"\n\n[sun]\nmodel = \"solar\"\n\n");
    for (name, spec) in volumes() {
        out.push_str(&format!(
            "[[volumes]]\nname = \"{name}\"\nmin = {}\nmax = {}\nspacing = {:?}\n\n",
            list(&spec.min),
            list(&spec.max),
            spec.spacing,
        ));
    }
    for reflection in reflections() {
        out.push_str(&format!(
            "[[reflection]]\nname = \"{}\"\nposition = {}\nmin = {}\nmax = {}\nresolution = {}\nfade = {:?}\npriority = {:?}\n\n",
            reflection.name.unwrap_or_default(),
            list(&reflection.position),
            list(&reflection.min),
            list(&reflection.max),
            reflection.resolution,
            reflection.fade,
            reflection.priority,
        ));
    }
    out.push_str(&format!(
        "[detail]\nsize = 2048\npadding = 8\nparts = {}\n\n[detail.view]\nposition = {}\ntarget = {}\nfov_y_deg = {:?}\nshift = {}\n",
        quoted(&DETAIL_PARTS),
        list(&lens.position),
        list(&lens.target),
        lens.fov_y,
        list(&lens.shift),
    ));
    out
}

pub fn folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/bench")
}

pub fn write(folder: &Path) {
    let room = room::room();
    for (name, bytes) in [
        ("bake.toml", recipe_text(&room).into_bytes()),
        ("materials.toml", library_text(&room.materials).into_bytes()),
        ("room.glb", glb(&room)),
    ] {
        std::fs::write(folder.join(name), bytes).unwrap();
        println!("wrote {}", folder.join(name).display());
    }
}

pub fn stale() -> Vec<&'static str> {
    let room = room::room();
    let mut out = Vec::new();
    if RECIPE != recipe_text(&room) {
        out.push("bake.toml");
    }
    if LIBRARY != library_text(&room.materials) {
        out.push("materials.toml");
    }
    if ROOM != glb(&room).as_slice() {
        out.push("room.glb");
    }
    out
}

pub fn materials(room: &Room) -> Vec<Material> {
    let library = library();
    room.materials
        .iter()
        .map(|(name, _)| *library.get(name).unwrap())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_bake::SceneSource;
    use pfx_load::{Asset, Cache};

    #[test]
    #[ignore = "writes bench/bake.toml, materials.toml and room.glb; run by hand when the room changes"]
    fn writes_the_recipe_files() {
        write(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/bench"));
    }

    #[test]
    fn the_committed_recipe_files_match_the_room() {
        let room = room::room();
        assert_eq!(RECIPE, recipe_text(&room), "rerun writes_the_recipe_files");
        assert_eq!(
            LIBRARY,
            library_text(&room.materials),
            "rerun writes_the_recipe_files"
        );
        assert!(
            ROOM == glb(&room).as_slice(),
            "rerun writes_the_recipe_files"
        );
    }

    #[test]
    fn the_library_reads_back_every_material() {
        let room = room::room();
        let library = library();
        assert_eq!(library.len(), room.materials.len());
        for (name, expected) in &room.materials {
            assert_eq!(library.get(name), Some(expected), "{name}");
        }
    }

    #[test]
    fn pfx_bake_reads_the_recipe_as_the_bench_bakes_it() {
        let source = SceneSource::load(&folder()).unwrap();
        assert_eq!(source.recipe.anchors, ANCHORS.to_vec());
        let room = room::room();
        let (triangles, _) = super::super::triangles(&room);
        assert_eq!(source.scene.triangles.len(), triangles.len());
        for (name, material) in &room.materials {
            let index = room::material_index(&room.materials, name);
            assert_eq!(&source.scene.materials[index], material, "{name}");
        }
        assert_eq!(source.recipe.volumes.len(), volumes().len());
        assert_eq!(source.recipe.reflections, reflections());
    }

    #[test]
    fn the_room_file_loads_with_every_part() {
        let room = room::room();
        let mut cache = Cache::new();
        let handle = cache.load_mesh(ROOM).unwrap();
        let Some(Asset::Mesh(mesh)) = cache.get(handle) else {
            panic!("room.glb holds no mesh");
        };
        assert_eq!(mesh.nodes.len(), room.parts.len());
        assert_eq!(mesh.materials.len(), room.materials.len());
        for (node, part) in mesh.nodes.iter().zip(&room.parts) {
            assert_eq!(node.name, part.name);
            assert_eq!(node.transform, part.model);
        }
        assert!(ROOM.len() < 512 * 1024, "room.glb is {} bytes", ROOM.len());
    }

    #[test]
    fn the_recipe_names_its_own_app() {
        assert!(RECIPE.starts_with("app = \"bench\"\n"));
    }
}
