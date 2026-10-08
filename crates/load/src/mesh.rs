use pfx_core::anim::{Interpolation, Property, Trs};

#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub nodes: Vec<Node>,
    pub groups: Vec<Group>,
    pub materials: Vec<Material>,
    pub roots: Vec<u32>,
    pub scenes: Vec<Scene>,
    pub skins: Vec<Skin>,
    pub animations: Vec<Animation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scene {
    pub name: Option<String>,
    pub roots: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub name: String,
    pub parent: Option<u32>,
    pub children: Vec<u32>,
    pub transform: [[f32; 4]; 4],
    pub trs: Trs,
    pub group: Option<u32>,
    pub skin: Option<u32>,
    pub extras: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skin {
    pub name: String,
    pub joints: Vec<u32>,
    pub inverse_binds: Vec<[[f32; 4]; 4]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Animation {
    pub name: String,
    pub channels: Vec<AnimationChannel>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnimationChannel {
    pub node: u32,
    pub property: Property,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    pub values: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub name: String,
    pub primitives: Vec<Primitive>,
    pub extras: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Primitive {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    pub uvs1: Option<Vec<[f32; 2]>>,
    pub indices: Vec<u32>,
    pub material: Option<u32>,
    pub joints: Option<Vec<[u16; 4]>>,
    pub weights: Option<Vec<[f32; 4]>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Material {
    pub name: String,
    pub extras: Option<serde_json::Value>,
}

macro_rules! extras_helpers {
    ($($kind:ty),*) => {$(
        impl $kind {
            pub fn extra(&self, key: &str) -> Option<&serde_json::Value> {
                self.extras.as_ref()?.get(key)
            }

            pub fn extra_str(&self, key: &str) -> Option<&str> {
                self.extra(key)?.as_str()
            }

            pub fn extra_f64(&self, key: &str) -> Option<f64> {
                self.extra(key)?.as_f64()
            }

            pub fn extra_bool(&self, key: &str) -> Option<bool> {
                self.extra(key)?.as_bool()
            }

            pub fn extra_array(&self, key: &str) -> Option<&[serde_json::Value]> {
                self.extra(key)?.as_array().map(Vec::as_slice)
            }

            pub fn extra_object(
                &self,
                key: &str,
            ) -> Option<&serde_json::Map<String, serde_json::Value>> {
                self.extra(key)?.as_object()
            }

            pub fn extra_f64s(&self, key: &str) -> Option<Vec<f64>> {
                self.extra_array(key)?
                    .iter()
                    .map(serde_json::Value::as_f64)
                    .collect()
            }
        }
    )*};
}

extras_helpers!(Node, Group, Material);

fn read_extras(extras: &gltf::json::Extras) -> Result<Option<serde_json::Value>, String> {
    match extras {
        Some(raw) => serde_json::from_str(raw.get())
            .map(Some)
            .map_err(|e| format!("mesh: bad extras: {e}")),
        None => Ok(None),
    }
}

const IDENTITY: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

impl Mesh {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        Self::parse_with(bytes, |uri| Err(format!("mesh: external buffer {uri}")))
    }

    pub fn parse_with<F>(bytes: &[u8], read: F) -> Result<Self, String>
    where
        F: Fn(&str) -> Result<Vec<u8>, String>,
    {
        let gltf = gltf::Gltf::from_slice(bytes).map_err(|e| format!("mesh: {e}"))?;
        let buffers = buffers(&gltf, &read)?;
        let nodes = nodes(&gltf)?;
        let groups = groups(&gltf, &buffers)?;
        let materials = materials(&gltf)?;
        let skins = skins(&gltf, &buffers)?;
        let animations = animations(&gltf, &buffers)?;
        for node in &nodes {
            if let Some(group) = node.group
                && groups.get(group as usize).is_none()
            {
                return Err("mesh: a node names a missing mesh".into());
            }
        }
        for group in &groups {
            for primitive in &group.primitives {
                if let Some(material) = primitive.material
                    && materials.get(material as usize).is_none()
                {
                    return Err("mesh: a triangle names a missing material".into());
                }
            }
        }
        let roots = gltf
            .default_scene()
            .or_else(|| gltf.scenes().next())
            .map(|scene| scene.nodes().map(|node| node.index() as u32).collect())
            .unwrap_or_default();
        let scenes = gltf
            .scenes()
            .map(|scene| Scene {
                name: scene.name().map(str::to_string),
                roots: scene.nodes().map(|node| node.index() as u32).collect(),
            })
            .collect();
        Ok(Mesh {
            nodes,
            groups,
            materials,
            roots,
            scenes,
            skins,
            animations,
        })
    }

    pub fn all_roots(&self) -> Vec<u32> {
        let mut seen = std::collections::HashSet::new();
        self.scenes
            .iter()
            .flat_map(|scene| scene.roots.iter().copied())
            .filter(|root| seen.insert(*root))
            .collect()
    }

    pub fn world(&self, node: u32) -> Option<[[f32; 4]; 4]> {
        let mut chain = Vec::new();
        let mut at = node;
        for _ in 0..=self.nodes.len() {
            chain.push(at);
            match self.nodes.get(at as usize)?.parent {
                Some(parent) => at = parent,
                None => {
                    let mut matrix = IDENTITY;
                    for id in chain.iter().rev() {
                        matrix = crate::scene::multiply(matrix, self.nodes[*id as usize].transform);
                    }
                    return Some(matrix);
                }
            }
        }
        None
    }
}

fn buffers<F>(gltf: &gltf::Gltf, read: &F) -> Result<Vec<Vec<u8>>, String>
where
    F: Fn(&str) -> Result<Vec<u8>, String>,
{
    let mut blob = gltf.blob.clone();
    let mut out = Vec::new();
    for buffer in gltf.buffers() {
        let mut data = match buffer.source() {
            gltf::buffer::Source::Bin => blob
                .take()
                .ok_or_else(|| "mesh: missing BIN chunk".to_string())?,
            gltf::buffer::Source::Uri(uri) => load_uri(uri, read)?,
        };
        if data.len() < buffer.length() {
            return Err(format!(
                "mesh: buffer {} is {} bytes, the file says {}",
                buffer.index(),
                data.len(),
                buffer.length()
            ));
        }
        while !data.len().is_multiple_of(4) {
            data.push(0);
        }
        out.push(data);
    }
    Ok(out)
}

fn load_uri<F>(uri: &str, read: &F) -> Result<Vec<u8>, String>
where
    F: Fn(&str) -> Result<Vec<u8>, String>,
{
    if let Some(rest) = uri.strip_prefix("data:") {
        let (meta, payload) = rest
            .split_once(',')
            .ok_or_else(|| "mesh: bad data uri".to_string())?;
        if meta.split(';').any(|part| part == "base64") {
            return decode_base64(payload).map_err(|e| format!("mesh: {e}"));
        }
        return percent_decode(payload).map(|text| text.into_bytes());
    }
    if uri.contains(':') {
        return Err(format!("mesh: unsupported buffer uri {uri}"));
    }
    read(&percent_decode(uri)?)
}

fn nodes(gltf: &gltf::Gltf) -> Result<Vec<Node>, String> {
    let mut nodes = Vec::new();
    for (index, node) in gltf.nodes().enumerate() {
        if node.index() != index {
            return Err("mesh: node order is not packed".into());
        }
        nodes.push(Node {
            name: node.name().unwrap_or("").to_string(),
            parent: None,
            children: node.children().map(|child| child.index() as u32).collect(),
            transform: node.transform().matrix(),
            trs: {
                let (translation, rotation, scale) = node.transform().decomposed();
                Trs {
                    translation,
                    rotation,
                    scale,
                }
            },
            group: node.mesh().map(|mesh| mesh.index() as u32),
            skin: node.skin().map(|skin| skin.index() as u32),
            extras: read_extras(node.extras())?,
        });
    }
    let children: Vec<Vec<u32>> = nodes.iter().map(|node| node.children.clone()).collect();
    for (index, child_list) in children.iter().enumerate() {
        for &child in child_list {
            let node = nodes
                .get_mut(child as usize)
                .ok_or("mesh: a node names a missing child")?;
            if node.parent.is_some() {
                return Err("mesh: a node has two parents".into());
            }
            node.parent = Some(index as u32);
        }
    }
    Ok(nodes)
}

fn groups(gltf: &gltf::Gltf, buffers: &[Vec<u8>]) -> Result<Vec<Group>, String> {
    let mut groups = Vec::new();
    for (index, mesh) in gltf.meshes().enumerate() {
        if mesh.index() != index {
            return Err("mesh: mesh order is not packed".into());
        }
        let mut primitives = Vec::new();
        for primitive in mesh.primitives() {
            if let Some(primitive) = read_primitive(&primitive, buffers)? {
                primitives.push(primitive);
            }
        }
        groups.push(Group {
            name: mesh.name().unwrap_or("").to_string(),
            primitives,
            extras: read_extras(mesh.extras())?,
        });
    }
    Ok(groups)
}

fn materials(gltf: &gltf::Gltf) -> Result<Vec<Material>, String> {
    let mut materials = Vec::new();
    for (index, material) in gltf.materials().enumerate() {
        if material.index() != Some(index) {
            return Err("mesh: material order is not packed".into());
        }
        materials.push(Material {
            name: material.name().unwrap_or("").to_string(),
            extras: read_extras(material.extras())?,
        });
    }
    Ok(materials)
}

fn skins(gltf: &gltf::Gltf, buffers: &[Vec<u8>]) -> Result<Vec<Skin>, String> {
    let mut skins = Vec::new();
    for (index, skin) in gltf.skins().enumerate() {
        if skin.index() != index {
            return Err("mesh: skin order is not packed".into());
        }
        let joints: Vec<u32> = skin.joints().map(|joint| joint.index() as u32).collect();
        let reader = skin.reader(|buffer| buffers.get(buffer.index()).map(Vec::as_slice));
        let inverse_binds: Vec<[[f32; 4]; 4]> = match reader.read_inverse_bind_matrices() {
            Some(matrices) => matrices.collect(),
            None => vec![IDENTITY; joints.len()],
        };
        if inverse_binds.len() != joints.len() {
            return Err(format!(
                "mesh: skin {index} has {} inverse bind matrices for {} joints",
                inverse_binds.len(),
                joints.len()
            ));
        }
        skins.push(Skin {
            name: skin.name().unwrap_or("").to_string(),
            joints,
            inverse_binds,
        });
    }
    Ok(skins)
}

fn animations(gltf: &gltf::Gltf, buffers: &[Vec<u8>]) -> Result<Vec<Animation>, String> {
    use gltf::animation::util::ReadOutputs;
    let mut out = Vec::new();
    for (index, animation) in gltf.animations().enumerate() {
        let mut channels = Vec::new();
        for channel in animation.channels() {
            let property = match channel.target().property() {
                gltf::animation::Property::Translation => Property::Translation,
                gltf::animation::Property::Rotation => Property::Rotation,
                gltf::animation::Property::Scale => Property::Scale,
                gltf::animation::Property::MorphTargetWeights => continue,
            };
            let interpolation = match channel.sampler().interpolation() {
                gltf::animation::Interpolation::Linear => Interpolation::Linear,
                gltf::animation::Interpolation::Step => Interpolation::Step,
                gltf::animation::Interpolation::CubicSpline => Interpolation::CubicSpline,
            };
            let reader = channel.reader(|buffer| buffers.get(buffer.index()).map(Vec::as_slice));
            let times: Vec<f32> = reader
                .read_inputs()
                .ok_or_else(|| format!("mesh: animation {index} has a channel without key times"))?
                .collect();
            let values: Vec<f32> = match reader.read_outputs() {
                Some(ReadOutputs::Translations(values)) => values.flatten().collect(),
                Some(ReadOutputs::Scales(values)) => values.flatten().collect(),
                Some(ReadOutputs::Rotations(values)) => values.into_f32().flatten().collect(),
                Some(ReadOutputs::MorphTargetWeights(_)) | None => {
                    return Err(format!(
                        "mesh: animation {index} has a channel without key values"
                    ));
                }
            };
            channels.push(AnimationChannel {
                node: channel.target().node().index() as u32,
                property,
                interpolation,
                times,
                values,
            });
        }
        out.push(Animation {
            name: animation
                .name()
                .map_or_else(|| format!("animation{index}"), str::to_string),
            channels,
        });
    }
    Ok(out)
}

fn read_primitive(
    primitive: &gltf::Primitive<'_>,
    buffers: &[Vec<u8>],
) -> Result<Option<Primitive>, String> {
    if primitive.mode() != gltf::mesh::Mode::Triangles {
        return Err("mesh: only triangles are supported".into());
    }
    let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(Vec::as_slice));
    let Some(positions) = reader.read_positions() else {
        return Ok(None);
    };
    let positions: Vec<[f32; 3]> = positions.collect();
    let normals = fit(
        reader
            .read_normals()
            .map(|values| values.collect())
            .unwrap_or_default(),
        positions.len(),
        "normals",
    )?;
    let tangents = fit(
        reader
            .read_tangents()
            .map(|values| values.collect())
            .unwrap_or_default(),
        positions.len(),
        "tangents",
    )?;
    let uvs = fit(
        reader
            .read_tex_coords(0)
            .map(|values| values.into_f32().collect())
            .unwrap_or_default(),
        positions.len(),
        "uvs",
    )?;
    let uvs1 = reader
        .read_tex_coords(1)
        .map(|values| fit(values.into_f32().collect(), positions.len(), "uvs1"))
        .transpose()?
        .filter(|values| !values.is_empty());
    let indices: Vec<u32> = match reader.read_indices() {
        Some(indices) => indices.into_u32().collect(),
        None => (0..positions.len() as u32).collect(),
    };
    if !indices.len().is_multiple_of(3) {
        return Err("mesh: indices are not a list of triangles".into());
    }
    if indices
        .iter()
        .any(|index| *index as usize >= positions.len())
    {
        return Err("mesh: an index is outside the vertex list".into());
    }
    let joints = reader
        .read_joints(0)
        .map(|values| fit(values.into_u16().collect(), positions.len(), "joints"))
        .transpose()?
        .filter(|values| !values.is_empty());
    let weights = reader
        .read_weights(0)
        .map(|values| fit(values.into_f32().collect(), positions.len(), "weights"))
        .transpose()?
        .filter(|values| !values.is_empty());
    if joints.is_some() != weights.is_some() {
        return Err(
            "mesh: a primitive has joints without weights or weights without joints".into(),
        );
    }
    Ok(Some(Primitive {
        positions,
        normals,
        tangents,
        uvs,
        uvs1,
        indices,
        material: primitive.material().index().map(|index| index as u32),
        joints,
        weights,
    }))
}

fn fit<T>(values: Vec<T>, len: usize, what: &str) -> Result<Vec<T>, String> {
    if values.is_empty() || values.len() == len {
        Ok(values)
    } else {
        Err(format!(
            "mesh: {what} has {} values for {len} vertices",
            values.len()
        ))
    }
}

fn percent_decode(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err("mesh: bad percent escape".into());
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3])
                .map_err(|_| "mesh: bad percent escape")?;
            let value = u8::from_str_radix(hex, 16).map_err(|_| "mesh: bad percent escape")?;
            out.push(value);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "mesh: buffer uri is not utf-8".into())
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if !bytes.len().is_multiple_of(4) {
        return Err("bad base64".into());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks_exact(4) {
        let pad = chunk.iter().rev().take_while(|byte| **byte == b'=').count();
        if pad > 2 {
            return Err("bad base64".into());
        }
        let mut packed = 0u32;
        for (index, &byte) in chunk.iter().enumerate() {
            if byte == b'=' {
                if index < 4 - pad {
                    return Err("bad base64".into());
                }
                continue;
            }
            let value = value(byte).ok_or("bad base64")?;
            packed |= u32::from(value) << (18 - 6 * index);
        }
        out.push((packed >> 16) as u8);
        if pad < 2 {
            out.push((packed >> 8) as u8);
        }
        if pad < 1 {
            out.push(packed as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Source;
    use std::path::Path;

    fn triangle(mesh: &Mesh) {
        assert_eq!(mesh.roots, vec![0]);
        assert_eq!(mesh.nodes[0].name, "root");
        assert_eq!(mesh.nodes[0].children, vec![1]);
        assert_eq!(mesh.nodes[0].parent, None);
        assert_eq!(mesh.nodes[1].name, "triangle");
        assert_eq!(mesh.nodes[1].parent, Some(0));
        assert_eq!(mesh.nodes[1].group, Some(0));
        assert_eq!(mesh.groups[0].name, "sheet");
        assert_eq!(
            mesh.materials,
            vec![Material {
                name: "ink".into(),
                extras: None
            }]
        );
        let primitive = &mesh.groups[0].primitives[0];
        assert_eq!(
            primitive.positions,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        );
        assert_eq!(primitive.normals, vec![[0.0, 0.0, 1.0]; 3]);
        assert_eq!(primitive.tangents, vec![[1.0, 0.0, 0.0, 1.0]; 3]);
        assert_eq!(primitive.uvs, vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        assert_eq!(primitive.indices, vec![0, 1, 2]);
        assert_eq!(primitive.material, Some(0));
        let world = mesh.world(1).unwrap();
        assert_eq!(world[3], [1.0, 2.0, 3.0, 1.0]);
        assert_eq!(mesh.world(0).unwrap()[0], [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_hand_made_glb_triangle_keeps_its_names_and_local_vertices() {
        let glb = Mesh::parse(include_bytes!("../tests/data/triangle.glb")).unwrap();
        triangle(&glb);
        let folder = Source::folder(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data"));
        let json = String::from_utf8(folder.read("triangle.gltf").unwrap()).unwrap();
        let gltf = Mesh::parse_with(json.as_bytes(), |uri| {
            folder.read_beside("triangle.gltf", uri)
        })
        .unwrap();
        assert_eq!(gltf, glb);
        let encoded = base64_encode(include_bytes!("../tests/data/triangle.bin"));
        let inline = json.replace(
            "\"uri\":\"triangle.bin\"",
            &format!("\"uri\":\"data:application/octet-stream;base64,{encoded}\""),
        );
        assert_eq!(Mesh::parse(inline.as_bytes()).unwrap(), glb);
        assert_eq!(decode_base64("TQ==").unwrap(), b"M");
        assert_eq!(decode_base64("TWE=").unwrap(), b"Ma");
        assert_eq!(decode_base64("TWFu").unwrap(), b"Man");
        assert_eq!(percent_decode("a%2Fb.bin").unwrap(), "a/b.bin");
    }

    fn scenes_json(scenes: &str, default: &str) -> String {
        format!(
            "{{\"asset\":{{\"version\":\"2.0\"}}{scenes}{default},\"nodes\":[\
             {{\"name\":\"a\"}},{{\"name\":\"b\"}},{{\"name\":\"c\"}},{{\"name\":\"d\"}}]}}"
        )
    }

    #[test]
    fn every_scene_keeps_its_roots_and_all_roots_lists_each_node_once() {
        let json = scenes_json(
            ",\"scenes\":[{\"name\":\"first\",\"nodes\":[0,1]},\
             {\"name\":\"second\",\"nodes\":[1,2]},{\"nodes\":[3]}]",
            "",
        );
        let mesh = Mesh::parse(json.as_bytes()).unwrap();
        assert_eq!(
            mesh.scenes,
            vec![
                Scene {
                    name: Some("first".into()),
                    roots: vec![0, 1]
                },
                Scene {
                    name: Some("second".into()),
                    roots: vec![1, 2]
                },
                Scene {
                    name: None,
                    roots: vec![3]
                },
            ]
        );
        assert_eq!(mesh.roots, vec![0, 1]);
        assert_eq!(mesh.all_roots(), vec![0, 1, 2, 3]);

        let json = scenes_json(
            ",\"scenes\":[{\"nodes\":[0,1]},{\"nodes\":[1,2]},{\"nodes\":[3,0]}]",
            ",\"scene\":1",
        );
        let mesh = Mesh::parse(json.as_bytes()).unwrap();
        assert_eq!(mesh.roots, vec![1, 2]);
        assert_eq!(mesh.scenes.len(), 3);
        assert_eq!(mesh.all_roots(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn a_file_without_scenes_has_no_roots() {
        let json = scenes_json("", "");
        let mesh = Mesh::parse(json.as_bytes()).unwrap();
        assert_eq!(mesh.nodes.len(), 4);
        assert!(mesh.roots.is_empty());
        assert!(mesh.scenes.is_empty());
        assert!(mesh.all_roots().is_empty());
    }

    const EXTRAS_NODE_A: &str = "\"extras\":{\"mover\":true,\"kind\":\"door\",\"speed\":2.5,\"axis\":[0,1,0],\"limits\":{\"min\":-1,\"max\":1.5,\"tags\":[\"a\",\"b\"]},\"none\":null}";

    fn with_extras(json: &str) -> String {
        json.replace(
            "{\"name\":\"root\",\"children\":[1]}",
            &format!("{{\"name\":\"root\",\"children\":[1],{EXTRAS_NODE_A}}}"),
        )
        .replace(
            "\"translation\":[1,2,3]}",
            "\"translation\":[1,2,3],\"extras\":{\"flag\":false,\"id\":7}}",
        )
        .replace(
            "\"name\":\"sheet\",",
            "\"name\":\"sheet\",\"extras\":{\"shadow\":\"only\"},",
        )
        .replace(
            "{\"name\":\"ink\"}",
            "{\"name\":\"ink\",\"extras\":{\"wet\":1.5,\"layers\":[1,2,3]}}",
        )
    }

    fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
        let mut json = json.as_bytes().to_vec();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut bin = bin.to_vec();
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let total = 12 + 8 + json.len() + 8 + bin.len();
        let mut out = Vec::new();
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&json);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
        out
    }

    #[test]
    fn extras_of_every_json_kind_are_read_from_nodes_a_mesh_and_a_material() {
        let json = include_str!("../tests/data/triangle.gltf");
        let bin = include_bytes!("../tests/data/triangle.bin");
        let embedded = with_extras(json).replace("{\"uri\":\"triangle.bin\",", "{");
        let mesh = Mesh::parse(&glb(&embedded, bin)).unwrap();

        let root = &mesh.nodes[0];
        assert_eq!(root.extra_bool("mover"), Some(true));
        assert_eq!(root.extra_str("kind"), Some("door"));
        assert_eq!(root.extra_f64("speed"), Some(2.5));
        assert_eq!(root.extra_f64s("axis"), Some(vec![0.0, 1.0, 0.0]));
        assert_eq!(root.extra_array("axis").unwrap().len(), 3);
        let limits = root.extra_object("limits").unwrap();
        assert_eq!(limits["min"], serde_json::json!(-1));
        assert_eq!(limits["tags"], serde_json::json!(["a", "b"]));
        assert_eq!(root.extra("none"), Some(&serde_json::Value::Null));
        assert_eq!(root.extra_str("none"), None);

        let triangle = &mesh.nodes[1];
        assert_eq!(triangle.extra_bool("flag"), Some(false));
        assert_eq!(triangle.extra_f64("id"), Some(7.0));

        assert_eq!(mesh.groups[0].extra_str("shadow"), Some("only"));
        assert_eq!(mesh.materials[0].extra_f64("wet"), Some(1.5));
        assert_eq!(
            mesh.materials[0].extra_f64s("layers"),
            Some(vec![1.0, 2.0, 3.0])
        );

        let text = Mesh::parse_with(with_extras(json).as_bytes(), |uri| {
            if uri == "triangle.bin" {
                Ok(include_bytes!("../tests/data/triangle.bin").to_vec())
            } else {
                Err(format!("unexpected {uri}"))
            }
        })
        .unwrap();
        assert_eq!(text, mesh);
    }

    #[test]
    fn extra_helpers_never_panic_on_missing_or_mistyped_keys() {
        let json = include_str!("../tests/data/triangle.gltf");
        let bin = include_bytes!("../tests/data/triangle.bin");
        let embedded = with_extras(json).replace("{\"uri\":\"triangle.bin\",", "{");
        let mesh = Mesh::parse(&glb(&embedded, bin)).unwrap();
        let root = &mesh.nodes[0];
        assert_eq!(root.extra("missing"), None);
        assert_eq!(root.extra_str("mover"), None);
        assert_eq!(root.extra_bool("kind"), None);
        assert_eq!(root.extra_f64("kind"), None);
        assert_eq!(root.extra_array("kind"), None);
        assert_eq!(root.extra_object("axis"), None);
        assert_eq!(root.extra_f64s("limits"), None);
        let mixed = Node {
            extras: Some(serde_json::json!({"mixed": [1, "x"], "list": 4})),
            ..mesh.nodes[0].clone()
        };
        assert_eq!(mixed.extra_f64s("mixed"), None);
        let scalar = Node {
            extras: Some(serde_json::json!(5)),
            ..mesh.nodes[0].clone()
        };
        assert_eq!(scalar.extra("anything"), None);
        let none = Node {
            extras: None,
            ..mesh.nodes[0].clone()
        };
        assert_eq!(none.extra_str("kind"), None);
    }

    #[test]
    fn a_file_without_extras_loads_with_none_and_the_same_geometry() {
        let plain = Mesh::parse(include_bytes!("../tests/data/triangle.glb")).unwrap();
        assert!(plain.nodes.iter().all(|node| node.extras.is_none()));
        assert!(plain.groups.iter().all(|group| group.extras.is_none()));
        assert!(
            plain
                .materials
                .iter()
                .all(|material| material.extras.is_none())
        );
        let json = include_str!("../tests/data/triangle.gltf");
        let bin = include_bytes!("../tests/data/triangle.bin");
        let embedded = with_extras(json).replace("{\"uri\":\"triangle.bin\",", "{");
        let extra = Mesh::parse(&glb(&embedded, bin)).unwrap();
        assert_eq!(extra.groups[0].primitives, plain.groups[0].primitives);
        assert_eq!(extra.nodes[1].transform, plain.nodes[1].transform);
        assert_eq!(extra.roots, plain.roots);
    }

    fn with_second_uvs(uvs1: &[[f32; 2]]) -> Vec<u8> {
        let json = include_str!("../tests/data/triangle.gltf");
        let mut bin = include_bytes!("../tests/data/triangle.bin").to_vec();
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let offset = bin.len();
        for uv in uvs1 {
            for value in uv {
                bin.extend_from_slice(&value.to_le_bytes());
            }
        }
        let json = json
            .replace("{\"uri\":\"triangle.bin\",\"byteLength\":150}", &format!("{{\"byteLength\":{}}}", bin.len()))
            .replace(
                "\"TEXCOORD_0\":3}",
                "\"TEXCOORD_0\":3,\"TEXCOORD_1\":5}",
            )
            .replace(
                "\"type\":\"SCALAR\"}]",
                &format!(
                    "\"type\":\"SCALAR\"}},{{\"bufferView\":5,\"componentType\":5126,\"count\":{},\"type\":\"VEC2\"}}]",
                    uvs1.len()
                ),
            )
            .replace(
                "\"byteLength\":6}]",
                &format!(
                    "\"byteLength\":6}},{{\"buffer\":0,\"byteOffset\":{offset},\"byteLength\":{}}}]",
                    uvs1.len() * 8
                ),
            );
        glb(&json, &bin)
    }

    #[test]
    fn a_second_uv_set_loads_beside_the_first() {
        let second = [[0.25, 0.5], [0.75, 0.5], [0.25, 1.0]];
        let mesh = Mesh::parse(&with_second_uvs(&second)).unwrap();
        let primitive = &mesh.groups[0].primitives[0];
        assert_eq!(primitive.uvs1.as_deref(), Some(&second[..]));
        assert_eq!(primitive.uvs, vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        let plain = Mesh::parse(include_bytes!("../tests/data/triangle.glb")).unwrap();
        assert_eq!(plain.groups[0].primitives[0].uvs1, None);
        let without = Primitive {
            uvs1: None,
            ..primitive.clone()
        };
        assert_eq!(without, plain.groups[0].primitives[0]);
        assert!(
            Mesh::parse(&with_second_uvs(&second[..2]))
                .unwrap_err()
                .contains("uvs1 has 2 values for 3 vertices")
        );
    }

    fn base64_encode(bytes: &[u8]) -> String {
        const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        let mut index = 0;
        while index + 3 <= bytes.len() {
            let packed = (u32::from(bytes[index]) << 16)
                | (u32::from(bytes[index + 1]) << 8)
                | u32::from(bytes[index + 2]);
            for shift in [18, 12, 6, 0] {
                out.push(ALPHA[((packed >> shift) & 63) as usize] as char);
            }
            index += 3;
        }
        out
    }
}
