use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::obj::{self, LoadOptions, Material, Model};
use crate::uri;

pub const SUFFIXES: [&str; 3] = ["glb", "gltf", "obj"];

pub fn is_model(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|e| SUFFIXES.contains(&e.as_str()))
}

pub fn model(path: &Path) -> Result<Vec<u8>, String> {
    let shown = path.display();
    let bytes = std::fs::read(path).map_err(|e| format!("{shown}: {e}"))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let folder = crate::Source::folder(path.parent().unwrap_or(Path::new("")));
    let read = |inside: &str| folder.read(inside);
    let suffix = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match suffix.as_str() {
        "glb" | "gltf" => gltf(&bytes, read),
        "obj" => from_obj(&bytes, &name, read),
        _ => Err("not a model pfx imports (.glb, .gltf or .obj)".into()),
    }
    .map_err(|e| format!("{shown}: {e}"))
}

fn outside(doc: &Value) -> bool {
    ["buffers", "images"].iter().any(|list| {
        doc.get(list)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("uri").and_then(Value::as_str))
            .any(|link| !link.starts_with("data:"))
    })
}

pub fn gltf<F>(bytes: &[u8], read: F) -> Result<Vec<u8>, String>
where
    F: Fn(&str) -> Result<Vec<u8>, String>,
{
    let (text, mut blob) = if bytes.starts_with(b"glTF") {
        let glb = gltf::binary::Glb::from_slice(bytes).map_err(|e| e.to_string())?;
        (glb.json.into_owned(), glb.bin.map(|bin| bin.into_owned()))
    } else {
        (bytes.to_vec(), None)
    };
    let mut doc: Value =
        serde_json::from_slice(&text).map_err(|e| format!("the JSON does not parse: {e}"))?;
    if bytes.starts_with(b"glTF") && !outside(&doc) {
        return Ok(bytes.to_vec());
    }
    let mut bin = Vec::new();
    let mut starts = Vec::new();
    let buffers = doc
        .get("buffers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (index, buffer) in buffers.iter().enumerate() {
        let length = buffer
            .get("byteLength")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("buffers[{index}] has no byteLength"))?
            as usize;
        let data = match buffer.get("uri").and_then(Value::as_str) {
            Some(link) => uri::load(link, &read).map_err(|e| format!("buffers[{index}]: {e}"))?,
            None => blob
                .take()
                .ok_or_else(|| format!("buffers[{index}] has no uri and no BIN chunk"))?,
        };
        if data.len() < length {
            return Err(format!(
                "buffers[{index}] is {} bytes, the file says {length}",
                data.len()
            ));
        }
        align(&mut bin);
        starts.push(bin.len() as u64);
        bin.extend_from_slice(&data[..length]);
    }
    let mut views = doc
        .get("bufferViews")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (index, view) in views.iter_mut().enumerate() {
        let buffer = view.get("buffer").and_then(Value::as_u64).unwrap_or(0) as usize;
        let start = *starts
            .get(buffer)
            .ok_or_else(|| format!("bufferViews[{index}] names a missing buffer"))?;
        let offset = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0);
        view["buffer"] = json!(0);
        view["byteOffset"] = json!(start + offset);
    }
    if let Some(images) = doc.get_mut("images").and_then(Value::as_array_mut) {
        for (index, image) in images.iter_mut().enumerate() {
            let Some(link) = image.get("uri").and_then(Value::as_str).map(str::to_string) else {
                continue;
            };
            let data = uri::load(&link, &read).map_err(|e| format!("images[{index}]: {e}"))?;
            let declared = image.get("mimeType").and_then(Value::as_str);
            let mime = mime_type(&link, declared, &data)
                .ok_or_else(|| format!("images[{index}]: {link} is not a PNG or JPEG image"))?;
            let image = image
                .as_object_mut()
                .ok_or_else(|| format!("images[{index}] is not an object"))?;
            image.remove("uri");
            image.insert("mimeType".into(), json!(mime));
            image.insert("bufferView".into(), json!(views.len()));
            views.push(embed(&mut bin, &data));
        }
    }
    let object = doc.as_object_mut().ok_or("the glTF is not a JSON object")?;
    if views.is_empty() {
        object.remove("bufferViews");
    } else {
        object.insert("bufferViews".into(), Value::Array(views));
    }
    if bin.is_empty() {
        object.remove("buffers");
    } else {
        object.insert("buffers".into(), json!([{ "byteLength": bin.len() }]));
    }
    glb(&doc, bin)
}

fn align(bin: &mut Vec<u8>) {
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
}

fn embed(bin: &mut Vec<u8>, data: &[u8]) -> Value {
    align(bin);
    let start = bin.len();
    bin.extend_from_slice(data);
    json!({ "buffer": 0, "byteOffset": start, "byteLength": data.len() })
}

fn sniffed(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else {
        None
    }
}

fn mime_type(link: &str, declared: Option<&str>, data: &[u8]) -> Option<String> {
    if let Some(found) = sniffed(data) {
        return Some(found.into());
    }
    if let Some(declared) = declared {
        return Some(declared.into());
    }
    if let Ok(uri::Uri::Data {
        mime: Some(mime), ..
    }) = uri::parse(link)
    {
        return Some(mime.into());
    }
    None
}

fn glb(doc: &Value, mut bin: Vec<u8>) -> Result<Vec<u8>, String> {
    let mut text = serde_json::to_vec(doc).map_err(|e| e.to_string())?;
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    align(&mut bin);
    let total = 12 + 8 + text.len() + if bin.is_empty() { 0 } else { 8 + bin.len() };
    let total = u32::try_from(total).map_err(|_| "the model is over 4 GiB".to_string())?;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&text);
    if !bin.is_empty() {
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
    }
    Ok(out)
}

struct Writer {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
    images: Vec<Value>,
    textures: Vec<Value>,
    by_path: BTreeMap<String, usize>,
}

impl Writer {
    fn accessor(&mut self, bytes: &[u8], count: usize, kind: &str, component: u32) -> usize {
        let view = embed(&mut self.bin, bytes);
        self.views.push(view);
        self.accessors.push(json!({
            "bufferView": self.views.len() - 1,
            "componentType": component,
            "count": count,
            "type": kind,
        }));
        self.accessors.len() - 1
    }

    fn floats(&mut self, values: &[f32], width: usize, kind: &str) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.accessor(&bytes, values.len() / width, kind, 5126)
    }

    fn texture<F>(&mut self, path: &str, read: &F) -> Result<usize, String>
    where
        F: Fn(&str) -> Result<Vec<u8>, String>,
    {
        if let Some(&found) = self.by_path.get(path) {
            return Ok(found);
        }
        let data = read(path)?;
        let mime = sniffed(&data).ok_or_else(|| format!("{path} is not a PNG or JPEG image"))?;
        let view = embed(&mut self.bin, &data);
        self.views.push(view);
        self.images.push(json!({
            "name": path,
            "mimeType": mime,
            "bufferView": self.views.len() - 1,
        }));
        self.textures
            .push(json!({ "source": self.images.len() - 1 }));
        let index = self.textures.len() - 1;
        self.by_path.insert(path.to_string(), index);
        Ok(index)
    }
}

const OPTIONS: LoadOptions = LoadOptions {
    triangulate: true,
    ignore_points: true,
    ignore_lines: true,
};

pub fn from_obj<F>(bytes: &[u8], name: &str, read: F) -> Result<Vec<u8>, String>
where
    F: Fn(&str) -> Result<Vec<u8>, String>,
{
    let text = obj::text(bytes).map_err(|e| e.to_string())?;
    let libraries = RefCell::new(Vec::<(String, usize)>::new());
    let missing = RefCell::new(None::<String>);
    let (models, materials) = obj::load_obj_text(text, &OPTIONS, |library| {
        let path = uri::inside(&uri::beside(name, library));
        let loaded = path.and_then(|path| {
            let bytes = read(&path)?;
            let text = obj::text(&bytes).map_err(|e| format!("{path}: {e}"))?;
            let loaded = obj::load_mtl_text(text).map_err(|e| format!("{path}: {e}"))?;
            Ok((path, loaded))
        });
        match loaded {
            Ok((path, (materials, map))) => {
                libraries.borrow_mut().push((path, materials.len()));
                Ok((materials, map))
            }
            Err(text) => {
                missing
                    .borrow_mut()
                    .get_or_insert(format!("mtllib {library}: {text}"));
                Err(obj::LoadError::ReadError)
            }
        }
    })
    .map_err(|e| e.to_string())?;
    if let Some(text) = missing.into_inner() {
        return Err(text);
    }
    let materials = materials.map_err(|e| e.to_string())?;
    let mut owners = Vec::with_capacity(materials.len());
    for (path, count) in libraries.into_inner() {
        owners.extend(std::iter::repeat_n(path, count));
    }
    let mut writer = Writer {
        bin: Vec::new(),
        views: Vec::new(),
        accessors: Vec::new(),
        images: Vec::new(),
        textures: Vec::new(),
        by_path: BTreeMap::new(),
    };
    let mut gltf_materials = Vec::with_capacity(materials.len());
    let mut used = Vec::new();
    for (material, owner) in materials.iter().zip(&owners) {
        let beside = |texture: &str| uri::inside(&uri::beside(owner, texture));
        let (value, extensions) = look(material, &mut writer, &beside, &read)
            .map_err(|e| format!("material {}: {e}", material.name))?;
        for extension in extensions {
            if !used.contains(&extension) {
                used.push(extension);
            }
        }
        gltf_materials.push(value);
    }
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    let mut names = BTreeMap::<String, usize>::new();
    for group in groups(&models) {
        let mut primitives = Vec::new();
        for model in group {
            if let Some(primitive) = primitive(model, &mut writer) {
                primitives.push(primitive);
            }
        }
        if primitives.is_empty() {
            continue;
        }
        let seen = names.entry(group[0].name.clone()).or_insert(0);
        *seen += 1;
        let label = match *seen {
            1 => group[0].name.clone(),
            n => format!("{}_{n}", group[0].name),
        };
        meshes.push(json!({ "name": label, "primitives": primitives }));
        nodes.push(json!({ "name": label, "mesh": meshes.len() - 1 }));
    }
    if nodes.is_empty() {
        return Err("the OBJ has no faces".into());
    }
    let mut doc = Map::new();
    doc.insert(
        "asset".into(),
        json!({ "version": "2.0", "generator": "pfx obj import" }),
    );
    doc.insert("scene".into(), json!(0));
    doc.insert(
        "scenes".into(),
        json!([{ "nodes": (0..nodes.len()).collect::<Vec<_>>() }]),
    );
    doc.insert("nodes".into(), Value::Array(nodes));
    doc.insert("meshes".into(), Value::Array(meshes));
    if !gltf_materials.is_empty() {
        doc.insert("materials".into(), Value::Array(gltf_materials));
    }
    if !writer.textures.is_empty() {
        doc.insert("textures".into(), Value::Array(writer.textures));
        doc.insert("images".into(), Value::Array(writer.images));
    }
    doc.insert("accessors".into(), Value::Array(writer.accessors));
    doc.insert("bufferViews".into(), Value::Array(writer.views));
    doc.insert(
        "buffers".into(),
        json!([{ "byteLength": writer.bin.len() }]),
    );
    if !used.is_empty() {
        doc.insert("extensionsUsed".into(), json!(used));
    }
    glb(&Value::Object(doc), writer.bin)
}

fn groups(models: &[Model]) -> Vec<&[Model]> {
    let mut out = Vec::new();
    let mut start = 0;
    for index in 1..=models.len() {
        if index == models.len() || models[index].name != models[start].name {
            out.push(&models[start..index]);
            start = index;
        }
    }
    out
}

fn primitive(model: &Model, writer: &mut Writer) -> Option<Value> {
    let mesh = &model.mesh;
    if mesh.indices.is_empty() {
        return None;
    }
    let count = mesh.positions.len() / 3;
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for point in mesh.positions.chunks_exact(3) {
        for axis in 0..3 {
            low[axis] = low[axis].min(point[axis]);
            high[axis] = high[axis].max(point[axis]);
        }
    }
    let position = writer.floats(&mesh.positions, 3, "VEC3");
    writer.accessors[position]["min"] = json!(low);
    writer.accessors[position]["max"] = json!(high);
    let mut attributes = Map::new();
    attributes.insert("POSITION".into(), json!(position));
    if mesh.normals.len() == count * 3 {
        let normal = writer.floats(&mesh.normals, 3, "VEC3");
        attributes.insert("NORMAL".into(), json!(normal));
    }
    if mesh.texcoords.len() == count * 2 {
        let flipped: Vec<f32> = mesh
            .texcoords
            .chunks_exact(2)
            .flat_map(|uv| [uv[0], 1.0 - uv[1]])
            .collect();
        let uv = writer.floats(&flipped, 2, "VEC2");
        attributes.insert("TEXCOORD_0".into(), json!(uv));
    }
    if mesh.vertex_color.len() == count * 3 {
        let colour = writer.floats(&mesh.vertex_color, 3, "VEC3");
        attributes.insert("COLOR_0".into(), json!(colour));
    }
    let bytes: Vec<u8> = mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
    let indices = writer.accessor(&bytes, mesh.indices.len(), "SCALAR", 5125);
    let mut primitive = json!({ "attributes": attributes, "indices": indices });
    if let Some(material) = mesh.material_id {
        primitive["material"] = json!(material);
    }
    Some(primitive)
}

pub fn roughness(material: &Material) -> f32 {
    if let Some(roughness) = material.roughness {
        return roughness.clamp(0.0, 1.0);
    }
    if material
        .specular
        .is_some_and(|ks| ks.iter().all(|&k| k <= 0.0))
    {
        return 1.0;
    }
    let shininess = material.shininess.unwrap_or(0.0).max(0.0);
    (2.0 / (shininess + 2.0)).powf(0.25)
}

fn look<F, R>(
    material: &Material,
    writer: &mut Writer,
    beside: &F,
    read: &R,
) -> Result<(Value, Vec<&'static str>), String>
where
    F: Fn(&str) -> Result<String, String>,
    R: Fn(&str) -> Result<Vec<u8>, String>,
{
    let mut extensions = Vec::new();
    let slot = |raw: &Option<String>,
                writer: &mut Writer,
                extensions: &mut Vec<&'static str>|
     -> Result<Option<(Value, Option<f32>)>, String> {
        let Some(raw) = raw else {
            return Ok(None);
        };
        let options = obj::texture_options(raw).ok_or_else(|| format!("{raw}: no file name"))?;
        let path = beside(&options.name)?;
        let index = writer.texture(&path, read)?;
        let mut info = json!({ "index": index });
        if options.scale.is_some() || options.offset.is_some() {
            let [su, sv, _] = options.scale.unwrap_or([1.0; 3]);
            let [ou, ov, _] = options.offset.unwrap_or([0.0; 3]);
            info["extensions"] = json!({
                "KHR_texture_transform": { "offset": [ou, 1.0 - sv - ov], "scale": [su, sv] }
            });
            if !extensions.contains(&"KHR_texture_transform") {
                extensions.push("KHR_texture_transform");
            }
        }
        Ok(Some((info, options.bump_multiplier)))
    };
    let diffuse = material.diffuse.unwrap_or([1.0; 3]);
    let alpha = material.dissolve.unwrap_or(1.0).clamp(0.0, 1.0);
    let mut pbr = json!({
        "baseColorFactor": [diffuse[0], diffuse[1], diffuse[2], alpha],
        "metallicFactor": material.metallic.unwrap_or(0.0).clamp(0.0, 1.0),
        "roughnessFactor": roughness(material),
    });
    if let Some((info, _)) = slot(&material.diffuse_texture, writer, &mut extensions)? {
        pbr["baseColorTexture"] = info;
    }
    let mut out = json!({ "name": material.name, "pbrMetallicRoughness": pbr });
    if alpha < 1.0 {
        out["alphaMode"] = json!("BLEND");
    }
    let normal = material
        .norm_texture
        .clone()
        .or(material.normal_texture.clone());
    if let Some((mut info, scale)) = slot(&normal, writer, &mut extensions)? {
        if let Some(scale) = scale {
            info["scale"] = json!(scale);
        }
        out["normalTexture"] = info;
    }
    let emissive_map = slot(&material.emissive_texture, writer, &mut extensions)?;
    let emissive = material.emissive.unwrap_or(if emissive_map.is_some() {
        [1.0; 3]
    } else {
        [0.0; 3]
    });
    let strength = emissive.iter().copied().fold(0.0_f32, f32::max);
    if strength > 0.0 {
        let scale = strength.max(1.0);
        out["emissiveFactor"] = json!(emissive.map(|e| e.max(0.0) / scale));
        if strength > 1.0 {
            out["extensions"]["KHR_materials_emissive_strength"] =
                json!({ "emissiveStrength": strength });
            extensions.push("KHR_materials_emissive_strength");
        }
    }
    if let Some((info, _)) = emissive_map {
        out["emissiveTexture"] = info;
    }
    if let Some(clearcoat) = material.clearcoat.filter(|&c| c > 0.0) {
        out["extensions"]["KHR_materials_clearcoat"] = json!({
            "clearcoatFactor": clearcoat.min(1.0),
            "clearcoatRoughnessFactor": material.clearcoat_roughness.unwrap_or(0.0).clamp(0.0, 1.0),
        });
        extensions.push("KHR_materials_clearcoat");
    }
    if material.roughness.is_none()
        && material.metallic.is_none()
        && let Some(specular) = material.specular
    {
        out["extensions"]["KHR_materials_specular"] =
            json!({ "specularColorFactor": specular.map(|k| k.max(0.0)) });
        extensions.push("KHR_materials_specular");
    }
    Ok((out, extensions))
}
