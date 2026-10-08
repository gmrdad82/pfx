use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug)]
pub enum Source {
    Embedded(&'static [(&'static str, &'static [u8])]),
    Folder(PathBuf),
}

impl Source {
    pub fn embedded(files: &'static [(&'static str, &'static [u8])]) -> Self {
        Source::Embedded(files)
    }

    pub fn folder(path: impl Into<PathBuf>) -> Self {
        Source::Folder(path.into())
    }

    pub fn beside_executable(relative: impl AsRef<Path>) -> Result<Self, String> {
        let relative = relative.as_ref();
        if relative.is_absolute() || relative.has_root() {
            return Err("the asset folder must be relative to the executable".into());
        }
        let exe = std::env::current_exe().map_err(|e| format!("executable: {e}"))?;
        let dir = exe
            .parent()
            .ok_or("the executable has no directory")?
            .join(relative);
        Ok(Source::Folder(dir))
    }

    pub fn directory(&self) -> Option<&Path> {
        match self {
            Source::Folder(path) => Some(path),
            Source::Embedded(_) => None,
        }
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        let key = normalize(name)?;
        match self {
            Source::Embedded(files) => files
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, bytes)| bytes.to_vec())
                .ok_or_else(|| format!("{key}: not in the binary")),
            Source::Folder(root) => {
                let path = root.join(&key);
                std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
            }
        }
    }

    pub fn read_beside(&self, file: &str, uri: &str) -> Result<Vec<u8>, String> {
        let file = normalize(file)?;
        let side = normalize(uri)?;
        let joined = match file.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/{side}"),
            None => side,
        };
        self.read(&joined)
    }
}

fn normalize(name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("asset name is empty".into());
    }
    let name = name.replace('\\', "/");
    let path = Path::new(&name);
    if path.is_absolute() {
        return Err(format!("{name} is outside the asset folder"));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| format!("{name} is outside the asset folder"))?;
                parts.push(part);
            }
            Component::CurDir => {}
            _ => return Err(format!("{name} is outside the asset folder")),
        }
    }
    if parts.is_empty() {
        return Err("asset name is empty".into());
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILES: &[(&str, &[u8])] = &[
        ("triangle.glb", include_bytes!("../tests/data/triangle.glb")),
        (
            "triangle.gltf",
            include_bytes!("../tests/data/triangle.gltf"),
        ),
        ("triangle.bin", include_bytes!("../tests/data/triangle.bin")),
        ("swatch8.png", include_bytes!("../tests/data/swatch8.png")),
        ("swatch16.png", include_bytes!("../tests/data/swatch16.png")),
        (
            "swatch.rgba16",
            include_bytes!("../tests/data/swatch.rgba16"),
        ),
        ("sky.hdr", include_bytes!("../tests/data/sky.hdr")),
        ("flat.hdr", include_bytes!("../tests/data/flat.hdr")),
        ("family.ttf", include_bytes!("../tests/data/family.ttf")),
    ];

    #[test]
    fn embedded_and_folder_sources_return_the_same_bytes() {
        let embedded = Source::embedded(FILES);
        let folder = Source::folder(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data"));
        for (name, bytes) in FILES {
            assert_eq!(embedded.read(name).unwrap(), *bytes);
            assert_eq!(folder.read(name).unwrap(), *bytes);
            assert_eq!(folder.read_beside("scene.gltf", name).unwrap(), *bytes);
        }
        assert!(folder.read("../Cargo.toml").is_err());
        assert!(folder.read("/etc/passwd").is_err());
        assert!(embedded.read("missing.png").is_err());
    }

    #[test]
    fn the_installed_folder_is_beside_the_executable() {
        let source = Source::beside_executable("assets").unwrap();
        let exe = std::env::current_exe().unwrap();
        let expected = exe.parent().unwrap().join("assets");
        assert_eq!(source.directory(), Some(expected.as_path()));
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        assert_ne!(source.directory(), Some(manifest.as_path()));
        assert!(Source::beside_executable("/assets").is_err());
    }
}
