pub fn manf_from_glb(glb: &[u8], files: &[(&str, &[u8])], compress: bool) -> Vec<u8> {
    let codec = if compress {
        manf::Codec::Auto
    } else {
        manf::Codec::Store
    };
    let split = gltf::Glb::from_slice(glb).expect("a GLB");
    let mut writer = manf::Writer::new().chunk(manf::GLTF, split.json.into_owned(), codec);
    if let Some(bin) = split.bin {
        writer = writer.chunk(manf::GBIN, bin.into_owned(), codec);
    }
    for (name, bytes) in files {
        writer = writer.named(manf::FILE, *name, bytes.to_vec(), codec);
    }
    writer.to_vec().expect("a .manf")
}

pub fn manf_from_gltf(json: &[u8], files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = manf::Writer::new().chunk(manf::GLTF, json.to_vec(), manf::Codec::Auto);
    for (name, bytes) in files {
        writer = writer.named(manf::FILE, *name, bytes.to_vec(), manf::Codec::Auto);
    }
    writer.to_vec().expect("a .manf")
}
