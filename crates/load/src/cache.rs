use crate::font::Font;
use crate::image::{ColorSpace, Image};
use crate::mesh::Mesh;
use crate::sky::Sky;
use crate::source::Source;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Handle(u32);

#[derive(Clone, Debug)]
pub enum Asset {
    Mesh(Mesh),
    Image(Image),
    Sky(Sky),
    Font(Font),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Mesh,
    Png,
    Rgba16 {
        width: u32,
        height: u32,
        space: ColorSpace,
    },
    Sky,
    Font,
}

struct Entry {
    hash: [u8; 32],
    kind: Kind,
    asset: Asset,
}

pub struct Cache {
    index: BTreeMap<[u8; 32], u32>,
    entries: Vec<Entry>,
}

impl Cache {
    pub fn new() -> Self {
        Cache {
            index: BTreeMap::new(),
            entries: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, handle: Handle) -> Option<&Asset> {
        self.entries
            .get(handle.0 as usize)
            .map(|entry| &entry.asset)
    }

    pub fn digest(&self, handle: Handle) -> Option<[u8; 32]> {
        self.entries.get(handle.0 as usize).map(|entry| entry.hash)
    }

    pub fn load_mesh(&mut self, bytes: &[u8]) -> Result<Handle, String> {
        self.insert(bytes, Kind::Mesh, || Ok(Asset::Mesh(Mesh::parse(bytes)?)))
    }

    pub fn load_mesh_from(&mut self, source: &Source, name: &str) -> Result<Handle, String> {
        let bytes = source.read(name)?;
        self.insert(&bytes, Kind::Mesh, || {
            Ok(Asset::Mesh(Mesh::parse_with(&bytes, |uri| {
                source.read_beside(name, uri)
            })?))
        })
    }

    pub fn load_png(&mut self, bytes: &[u8]) -> Result<Handle, String> {
        self.insert(bytes, Kind::Png, || Ok(Asset::Image(Image::png(bytes)?)))
    }

    pub fn load_rgba16(
        &mut self,
        bytes: &[u8],
        width: u32,
        height: u32,
        space: ColorSpace,
    ) -> Result<Handle, String> {
        self.insert(
            bytes,
            Kind::Rgba16 {
                width,
                height,
                space,
            },
            || Ok(Asset::Image(Image::rgba16(bytes, width, height, space)?)),
        )
    }

    pub fn load_sky(&mut self, bytes: &[u8]) -> Result<Handle, String> {
        self.insert(bytes, Kind::Sky, || Ok(Asset::Sky(Sky::parse(bytes)?)))
    }

    pub fn load_font(&mut self, bytes: &[u8]) -> Result<Handle, String> {
        self.insert(bytes, Kind::Font, || Ok(Asset::Font(Font::parse(bytes)?)))
    }

    fn insert(
        &mut self,
        bytes: &[u8],
        kind: Kind,
        build: impl FnOnce() -> Result<Asset, String>,
    ) -> Result<Handle, String> {
        let hash = sha256(bytes);
        if let Some(&index) = self.index.get(&hash) {
            return if self.entries[index as usize].kind == kind {
                Ok(Handle(index))
            } else {
                Err("the same bytes are already cached as a different asset".into())
            };
        }
        let asset = build()?;
        let index =
            u32::try_from(self.entries.len()).map_err(|_| "the cache is full".to_string())?;
        self.index.insert(hash, index);
        self.entries.push(Entry { hash, kind, asset });
        Ok(Handle(index))
    }
}

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&digest);
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Asset;
    use std::path::Path;

    const FILES: &[(&str, &[u8])] = &[
        ("triangle.glb", include_bytes!("../tests/data/triangle.glb")),
        (
            "triangle.gltf",
            include_bytes!("../tests/data/triangle.gltf"),
        ),
        ("swatch8.png", include_bytes!("../tests/data/swatch8.png")),
        ("sky.hdr", include_bytes!("../tests/data/sky.hdr")),
        ("family.ttf", include_bytes!("../tests/data/family.ttf")),
    ];

    #[test]
    fn the_same_bytes_return_the_same_handle() {
        let glb = include_bytes!("../tests/data/triangle.glb");
        let mut cache = Cache::new();
        assert!(cache.load_mesh(b"nope").is_err());
        assert!(cache.is_empty());
        let first = cache.load_mesh(glb).unwrap();
        let second = cache.load_mesh(glb).unwrap();
        assert_eq!(first, second);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.digest(first), Some(sha256(glb)));
        match cache.get(first) {
            Some(Asset::Mesh(mesh)) => assert_eq!(mesh.materials[0].name, "ink"),
            other => panic!("mesh, got {other:?}"),
        }

        let png = include_bytes!("../tests/data/swatch8.png");
        let image = cache.load_png(png).unwrap();
        assert_ne!(image, first);
        assert!(cache.load_mesh(png).is_err());
        assert_eq!(cache.len(), 2);

        let embedded = Source::embedded(FILES);
        let folder = Source::folder(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data"));
        let from_binary = cache.load_mesh_from(&embedded, "triangle.glb").unwrap();
        let from_folder = cache.load_mesh_from(&folder, "triangle.glb").unwrap();
        assert_eq!(from_binary, first);
        assert_eq!(from_folder, first);

        let sky = cache
            .load_sky(include_bytes!("../tests/data/sky.hdr"))
            .unwrap();
        let font = cache
            .load_font(include_bytes!("../tests/data/family.ttf"))
            .unwrap();
        let raw = cache
            .load_rgba16(
                include_bytes!("../tests/data/swatch.rgba16"),
                2,
                2,
                ColorSpace::Linear,
            )
            .unwrap();
        assert_eq!(
            cache
                .load_sky(include_bytes!("../tests/data/sky.hdr"))
                .unwrap(),
            sky
        );
        assert_eq!(
            cache
                .load_font(include_bytes!("../tests/data/family.ttf"))
                .unwrap(),
            font
        );
        assert_eq!(
            cache
                .load_rgba16(
                    include_bytes!("../tests/data/swatch.rgba16"),
                    2,
                    2,
                    ColorSpace::Linear
                )
                .unwrap(),
            raw
        );
        assert!(
            cache
                .load_rgba16(
                    include_bytes!("../tests/data/swatch.rgba16"),
                    4,
                    1,
                    ColorSpace::Linear
                )
                .is_err()
        );
        let sidecar = cache.load_mesh_from(&folder, "triangle.gltf").unwrap();
        match cache.get(sidecar) {
            Some(Asset::Mesh(mesh)) => assert_eq!(mesh.groups[0].name, "sheet"),
            other => panic!("mesh, got {other:?}"),
        }
    }
}
