pub fn pfx_from_glb(glb: &[u8], files: &[(&str, &[u8])], compress: bool) -> Vec<u8> {
    let codec = if compress {
        pfx_file::Codec::Auto
    } else {
        pfx_file::Codec::Store
    };
    let split = gltf::Glb::from_slice(glb).expect("a GLB");
    let mut writer = pfx_file::Writer::new().chunk(pfx_file::GLTF, split.json.into_owned(), codec);
    if let Some(bin) = split.bin {
        writer = writer.chunk(pfx_file::GBIN, bin.into_owned(), codec);
    }
    for (name, bytes) in files {
        writer = writer.named(pfx_file::FILE, *name, bytes.to_vec(), codec);
    }
    writer.to_vec().expect("a .pfx")
}

pub fn pfx_from_gltf(json: &[u8], files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer =
        pfx_file::Writer::new().chunk(pfx_file::GLTF, json.to_vec(), pfx_file::Codec::Auto);
    for (name, bytes) in files {
        writer = writer.named(pfx_file::FILE, *name, bytes.to_vec(), pfx_file::Codec::Auto);
    }
    writer.to_vec().expect("a .pfx")
}
