use std::borrow::Cow;

pub const MESH_SUFFIXES: [&str; 3] = [".glb", ".gltf", ".pitofx"];

pub fn is_mesh_file(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    MESH_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
}

pub fn is_pfx(bytes: &[u8]) -> bool {
    bytes.starts_with(&pfx_file::MAGIC)
}

pub fn open_gltf(bytes: &[u8]) -> Result<gltf::Gltf, String> {
    if is_pfx(bytes) {
        return document(&reader(bytes)?);
    }
    gltf::Gltf::from_slice(bytes).map_err(|e| format!("mesh: {e}"))
}

pub(crate) fn reader(bytes: &[u8]) -> Result<pfx_file::Reader<'_>, String> {
    pfx_file::Reader::from_bytes(bytes).map_err(|e| format!("mesh: {e}"))
}

pub(crate) fn document(reader: &pfx_file::Reader<'_>) -> Result<gltf::Gltf, String> {
    let Some(json) = reader
        .chunk(pfx_file::GLTF)
        .map_err(|e| format!("mesh: {e}"))?
    else {
        if reader.find(pfx_file::SCNE).is_some() {
            return Err("mesh: the .pitofx holds a scene and no GLTF; open it as a scene".into());
        }
        return Err("mesh: the .pitofx holds no GLTF chunk".into());
    };
    if json.starts_with(b"glTF") {
        return Err("mesh: the .pitofx GLTF chunk holds a GLB, not glTF JSON".into());
    }
    let mut gltf = gltf::Gltf::from_slice(&json).map_err(|e| format!("mesh: {e}"))?;
    gltf.blob = reader
        .chunk(pfx_file::GBIN)
        .map_err(|e| format!("mesh: {e}"))?
        .map(Cow::into_owned);
    Ok(gltf)
}

pub(crate) fn file(reader: &pfx_file::Reader<'_>, uri: &str) -> Result<Option<Vec<u8>>, String> {
    let key = crate::source::normalize(uri).unwrap_or_else(|_| uri.to_string());
    let found = match reader.named(pfx_file::FILE, &key) {
        Ok(None) if key != uri => reader.named(pfx_file::FILE, uri),
        other => other,
    };
    found
        .map(|chunk| chunk.map(Cow::into_owned))
        .map_err(|e| format!("mesh: {uri}: {e}"))
}
