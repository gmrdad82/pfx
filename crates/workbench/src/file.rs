use std::path::{Path, PathBuf};

use pfx_build::{Document, Registry, gltf_io};
use pfx_recipe::{Key, Loaded, Patch};
use toml_edit::{ArrayOfTables, DocumentMut, Item};

pub const MARK: &str = "\u{25cf}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Recipe,
    Scene,
    Pfx,
    Glb,
}

pub fn kind(path: &Path) -> Option<Kind> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(pfx_file::EXTENSION) if crate::scenes::is_scene(path) => Some(Kind::Scene),
        Some(pfx_file::EXTENSION) => Some(Kind::Pfx),
        Some("glb") => Some(Kind::Glb),
        Some("toml") if crate::scenes::is_scene(path) => Some(Kind::Scene),
        Some("toml") => Some(Kind::Recipe),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Open(PathBuf),
    Save,
    SaveAs(PathBuf),
    Import(PathBuf),
    Export { path: PathBuf, selected: bool },
    Drop { path: PathBuf, shift: bool },
}

pub fn step(command: &str, rest: &str) -> Option<Result<Step, String>> {
    let rest = rest.trim();
    let path = |text: &str, why: &str| {
        let text = text.trim();
        if text.is_empty() {
            Err(why.to_string())
        } else {
            Ok(PathBuf::from(text))
        }
    };
    let flagged = |flag: &str| match rest.strip_suffix(flag) {
        Some(head) if head.ends_with(char::is_whitespace) => (head, true),
        _ => (rest, false),
    };
    Some(match command {
        "save" if rest.is_empty() => Ok(Step::Save),
        "save" => Err("save takes nothing; save-as <path> saves elsewhere".to_string()),
        "open" => path(rest, "open wants a path").map(Step::Open),
        "save-as" => path(rest, "save-as wants a path").map(Step::SaveAs),
        "import" => path(rest, "import wants the path of a .glb").map(Step::Import),
        "export" => {
            let (head, selected) = flagged("--selected");
            path(head, "export wants a path and an optional --selected")
                .map(|path| Step::Export { path, selected })
        }
        "drop" => {
            let (head, shift) = flagged("shift");
            path(head, "drop wants a path and an optional shift")
                .map(|path| Step::Drop { path, shift })
        }
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Held {
    pub kind: Kind,
    pub source: PathBuf,
    pub target: Option<PathBuf>,
    pub work: PathBuf,
    pub saved: usize,
    pub edited: bool,
    pub kept: Vec<String>,
}

impl Held {
    pub fn shown(&self) -> &Path {
        self.target.as_deref().unwrap_or(&self.source)
    }
}

#[cfg(not(test))]
fn work_root() -> PathBuf {
    crate::open::state_dir().join("documents")
}

#[cfg(test)]
fn work_root() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/ui-tests/documents")
}

fn work_dir(source: &Path) -> PathBuf {
    let whole = std::path::absolute(source).unwrap_or_else(|_| source.to_path_buf());
    let stem = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".to_string());
    work_root().join(format!(
        "{stem}-{:016x}",
        crate::follow::hash(&whole.display().to_string())
    ))
}

fn fresh_dir(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    }
    std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))
}

pub fn hold(path: &Path) -> Result<(Held, PathBuf), String> {
    let work = work_dir(path);
    match kind(path) {
        Some(Kind::Pfx) => {
            let document = Document::open(path)?;
            fresh_dir(&work)?;
            let recipe = document.unpack(&work)?;
            let edited = document.edited();
            let kept = bring_beside(document, path, &recipe).unwrap_or_default();
            Ok((
                Held {
                    kind: Kind::Pfx,
                    source: path.to_path_buf(),
                    target: Some(path.to_path_buf()),
                    work,
                    saved: 0,
                    edited,
                    kept,
                },
                recipe,
            ))
        }
        Some(Kind::Glb) => {
            if !is_glb(path) {
                return Err(format!("{}: not a binary glTF 2.0 file", path.display()));
            }
            fresh_dir(&work)?;
            let stem = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "imported".to_string());
            let file = path.file_name().map(PathBuf::from).unwrap_or_default();
            std::fs::copy(path, work.join(&file))
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let head = format!(
                "recipe = 1\nname = {}\n",
                toml_edit::Value::from(stem.as_str())
            );
            let text = gltf_io::import_glb(&head, &file, &object_name(&stem, &[]))?;
            let recipe = work.join(format!("{stem}.toml"));
            std::fs::write(&recipe, text)
                .map_err(|error| format!("{}: {error}", recipe.display()))?;
            Ok((
                Held {
                    kind: Kind::Glb,
                    source: path.to_path_buf(),
                    target: None,
                    work,
                    saved: 0,
                    edited: false,
                    kept: Vec::new(),
                },
                recipe,
            ))
        }
        _ => Err(format!("{}: not a .pitofx or a .glb", path.display())),
    }
}

fn bring_beside(
    mut document: Document,
    source: &Path,
    recipe: &Path,
) -> Result<Vec<String>, String> {
    document.recipe_text = std::fs::read_to_string(recipe)
        .map_err(|error| format!("{}: {error}", recipe.display()))?;
    let dir = recipe.parent().unwrap_or(Path::new(""));
    let mut kept = Vec::new();
    for (name, file) in document.beside(source)? {
        let copy = dir.join(&name);
        if !same_file_bytes(&file, &copy) {
            if let Some(parent) = copy.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("{}: {error}", parent.display()))?;
            }
            std::fs::copy(&file, &copy).map_err(|error| format!("{}: {error}", file.display()))?;
        }
        kept.push(name);
    }
    Ok(kept)
}

fn same_file_bytes(file: &Path, other: &Path) -> bool {
    let len = |path: &Path| std::fs::metadata(path).map(|meta| meta.len()).ok();
    len(file).is_some()
        && len(file) == len(other)
        && std::fs::read(file).ok() == std::fs::read(other).ok()
}

pub fn keep_beside(held: &mut Held, recipe: &Path) -> Result<(), String> {
    if held.kind != Kind::Pfx || !held.source.is_file() {
        return Ok(());
    }
    let document = Document::open(&held.source)?;
    for name in bring_beside(document, &held.source, recipe)? {
        if !held.kept.contains(&name) {
            held.kept.push(name);
        }
    }
    Ok(())
}

pub fn with_extension(path: &Path, ext: &str) -> PathBuf {
    if path.extension().is_some_and(|found| found == ext) {
        return path.to_path_buf();
    }
    let mut text = path.as_os_str().to_os_string();
    text.push(".");
    text.push(ext);
    PathBuf::from(text)
}

pub fn imported(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy().to_string();
    let lower = name.to_lowercase();
    let stem = if lower.ends_with(".scene.toml") {
        &name[..name.len() - ".scene.toml".len()]
    } else if lower.ends_with(".toml") {
        &name[..name.len() - ".toml".len()]
    } else {
        return None;
    };
    Some(path.with_file_name(format!("{stem}.pitofx")))
}

pub fn import_beside(path: &Path, registry: &Registry) -> Result<Option<PathBuf>, String> {
    let Some(target) = imported(path).filter(|_| path.is_file()) else {
        return Ok(None);
    };
    if target.is_file() {
        return Ok(Some(target));
    }
    match kind(path) {
        Some(Kind::Scene) => {
            pfx_load::scene::pack(path, &target).map_err(|error| error.to_string())?;
        }
        Some(Kind::Recipe) => {
            pack(path, registry, &target, true, None)?;
        }
        _ => return Ok(None),
    }
    Ok(Some(target))
}

pub fn pack(
    recipe: &Path,
    registry: &Registry,
    out: &Path,
    whole: bool,
    keep: Option<&[String]>,
) -> Result<PathBuf, String> {
    let out = with_extension(out, pfx_file::EXTENSION);
    let mut document = Document::pack_for(recipe, registry, &out)?;
    if keep.is_some_and(|keep| document.referenced.iter().any(|name| !keep.contains(name))) {
        document = Document::pack(recipe, registry)?;
    }
    if let Ok(old) = Document::open(&out) {
        document.overrides = old.overrides;
        if document.thumb.is_none() {
            document.thumb = old.thumb;
        }
    }
    if whole {
        document.write_whole(&out)?;
    } else {
        document.write(&out)?;
    }
    Ok(out)
}

pub fn export(
    recipe: &Path,
    registry: &Registry,
    only: Option<&str>,
    out: &Path,
) -> Result<PathBuf, String> {
    let out = with_extension(out, "glb");
    let bytes = gltf_io::export_glb(recipe, registry, only)?;
    crate::open::write_atomic(&out, &bytes)
        .map_err(|error| format!("{}: {error}", out.display()))?;
    Ok(out)
}

pub fn is_glb(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 8];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut head))
        .is_ok()
        && &head[..4] == b"glTF"
        && u32::from_le_bytes([head[4], head[5], head[6], head[7]]) == 2
}

pub fn object_name(stem: &str, known: &[&str]) -> String {
    let mut base: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if base.is_empty() || base.starts_with(|c: char| c.is_ascii_digit()) {
        base.insert_str(0, "mesh_");
    }
    if !known.contains(&base.as_str()) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|name| !known.contains(&name.as_str()))
        .unwrap_or(base)
}

pub fn copy_in(glb: &Path, work: &Path) -> Result<PathBuf, String> {
    let name = glb
        .file_name()
        .map(PathBuf::from)
        .ok_or_else(|| format!("{}: not a file", glb.display()))?;
    let stem = glb
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut file = name.clone();
    let mut n = 2;
    while work.join(&file).exists() {
        if std::fs::read(work.join(&file)).ok() == std::fs::read(glb).ok() {
            return Ok(file);
        }
        file = PathBuf::from(format!("{stem}_{n}.glb"));
        n += 1;
    }
    std::fs::copy(glb, work.join(&file)).map_err(|error| format!("{}: {error}", glb.display()))?;
    Ok(file)
}

pub fn relative(path: &Path, base: &Path) -> PathBuf {
    let whole = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let base = std::path::absolute(base).unwrap_or_else(|_| base.to_path_buf());
    whole
        .strip_prefix(&base)
        .map(Path::to_path_buf)
        .unwrap_or(whole)
}

pub fn import_patches(loaded: &Loaded, glb: &Path, name: &str) -> Result<Vec<Patch>, String> {
    let text = gltf_io::import_glb(&loaded.doc.to_string(), glb, name)?;
    let doc: DocumentMut = text
        .parse()
        .map_err(|error| format!("import: the recipe would not read: {error}"))?;
    let mut table = doc
        .get("object")
        .and_then(Item::as_array_of_tables)
        .and_then(|objects| objects.iter().last())
        .cloned()
        .ok_or_else(|| "import: no object was added".to_string())?;
    table.decor_mut().set_prefix("\n");
    let had = loaded
        .doc
        .get("object")
        .and_then(Item::as_array_of_tables)
        .is_some_and(|objects| !objects.is_empty());
    Ok(vec![if had {
        Patch {
            path: vec![
                Key::Table("object".to_string()),
                Key::Object(name.to_string()),
            ],
            value: Some(Item::Table(table)),
        }
    } else {
        let mut tables = ArrayOfTables::new();
        tables.push(table);
        Patch {
            path: vec![Key::Table("object".to_string())],
            value: Some(Item::ArrayOfTables(tables)),
        }
    }])
}

pub fn thumb(path: &Path) -> Option<Vec<u8>> {
    let reader = pfx_file::Reader::open(path).ok()?;
    let thumb = reader.chunk(pfx_file::THMB).ok()??;
    Some(thumb.into_owned())
}

pub fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let size = [info.width as usize, info.height as usize];
    let pixels = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => pixels
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some(egui::ColorImage::from_rgba_unmultiplied(size, &rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stock_glb_and_its_packed_document_draw_their_own_materials() {
        let glb = crate::scratch("stock-glb").with_file_name("model.glb");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/data/head.glb"),
            &glb,
        )
        .unwrap();
        let registry = Registry::new();
        let drawn = |recipe: &Path| {
            let loaded = pfx_recipe::load(recipe).unwrap();
            let base = recipe.parent().unwrap();
            let built = pfx_build::build_with(&loaded.recipe, base, &registry).unwrap();
            pfx_recipe_scene::Stage::prepare(&loaded.recipe, &built)
                .map(|_| ())
                .map_err(|error| error.to_string())
        };
        let (_, recipe) = hold(&glb).unwrap();
        assert_eq!(drawn(&recipe), Ok(()));
        let packed = pack(&recipe, &registry, &glb.with_file_name("model"), true, None).unwrap();
        let (_, recipe) = hold(&packed).unwrap();
        assert_eq!(drawn(&recipe), Ok(()));
    }

    #[test]
    fn a_toml_is_imported_into_a_pfx_beside_it_once_and_never_written() {
        let recipe = crate::scratch("import-once").with_file_name("desk.toml");
        let text = "recipe = 1\nname = \"desk\"\n\n[[object]]\nname = \"top\"\nkind = \"box\"\nsize = [1.0, 0.1, 0.5]\n";
        std::fs::write(&recipe, text).unwrap();
        let target = recipe.with_file_name("desk.pitofx");
        let _ = std::fs::remove_file(&target);
        let registry = Registry::new();
        assert_eq!(
            import_beside(&recipe, &registry).unwrap(),
            Some(target.clone())
        );
        let packed = std::fs::read(&target).unwrap();
        assert!(packed.starts_with(&pfx_file::MAGIC));
        std::fs::write(&recipe, text.replace("0.5]", "0.6]")).unwrap();
        assert_eq!(
            import_beside(&recipe, &registry).unwrap(),
            Some(target.clone())
        );
        assert_eq!(std::fs::read(&target).unwrap(), packed);
        assert_eq!(import_beside(&target, &registry).unwrap(), None);
        let scene = crate::scenes::fixture("import-scene");
        let packed_scene = import_beside(&scene, &registry).unwrap().unwrap();
        assert_eq!(packed_scene, scene.with_file_name("stand.pitofx"));
        assert!(crate::scenes::is_scene(&packed_scene));
        assert_eq!(kind(&packed_scene), Some(Kind::Scene));
        assert_eq!(kind(&target), Some(Kind::Pfx));
    }

    #[test]
    fn a_held_document_keeps_its_hdri_beside_when_saved_back_and_packs_it_when_saved_elsewhere() {
        let dir = crate::scratch("held-sky").with_file_name("source");
        let other = dir.with_file_name("other");
        for folder in [&dir, &other] {
            let _ = std::fs::remove_dir_all(folder);
            std::fs::create_dir_all(folder).unwrap();
        }
        std::fs::create_dir_all(dir.join("assets/sky")).unwrap();
        let mut sky = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 2\n".to_vec();
        sky.extend([128, 128, 128, 129, 128, 128, 128, 129]);
        std::fs::write(dir.join("assets/sky/softbox.hdr"), &sky).unwrap();
        let toml = dir.join("plaque.toml");
        std::fs::write(
            &toml,
            "recipe = 1\nname = \"plaque\"\n\n[sky]\nkind = \"hdr\"\npath = \"assets/sky/softbox.hdr\"\nsource = \"generated\"\n\n[[object]]\nname = \"b\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\n",
        )
        .unwrap();
        let registry = Registry::new();
        let kept_pfx = pack(&toml, &registry, &dir.join("plaque"), true, None).unwrap();
        let packed_pfx = pack(&toml, &registry, &dir.join("packed"), true, Some(&[][..])).unwrap();
        std::fs::remove_file(&toml).unwrap();
        let packs_sky = |pfx: &Path| {
            Document::open(pfx)
                .unwrap()
                .files
                .keys()
                .any(|name| name.to_string_lossy().ends_with("softbox.hdr"))
        };
        let lit = |pfx: &Path| -> Result<(), String> {
            let document = Document::open(pfx)?;
            let (recipe, _mount) = document.mount(pfx)?;
            let loaded = pfx_recipe::load(&recipe).map_err(|error| error.to_string())?;
            pfx_recipe_scene::Lighting::of(&loaded.recipe)
                .map(|_| ())
                .map_err(|error| error.0)
        };
        assert!(!packs_sky(&kept_pfx));
        assert!(packs_sky(&packed_pfx));

        let (mut held, recipe) = hold(&packed_pfx).unwrap();
        assert!(held.kept.is_empty());
        keep_beside(&mut held, &recipe).unwrap();
        pack(&recipe, &registry, &packed_pfx, false, Some(&held.kept)).unwrap();
        assert!(packs_sky(&packed_pfx));

        let (mut held, recipe) = hold(&kept_pfx).unwrap();
        assert_eq!(held.kept, ["assets/sky/softbox.hdr"]);
        let copy = recipe.parent().unwrap().join("assets/sky/softbox.hdr");
        assert_eq!(std::fs::read(&copy).unwrap(), sky);
        keep_beside(&mut held, &recipe).unwrap();
        pack(&recipe, &registry, &kept_pfx, false, Some(&held.kept)).unwrap();
        assert!(!packs_sky(&kept_pfx));
        lit(&kept_pfx).unwrap();

        std::fs::remove_file(&copy).unwrap();
        held.kept.clear();
        keep_beside(&mut held, &recipe).unwrap();
        assert_eq!(held.kept, ["assets/sky/softbox.hdr"]);
        assert_eq!(std::fs::read(&copy).unwrap(), sky);
        let away = pack(
            &recipe,
            &registry,
            &other.join("plaque"),
            true,
            Some(&held.kept),
        )
        .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(packs_sky(&away));
        lit(&away).unwrap();
        let (held, _) = hold(&away).unwrap();
        assert!(held.kept.is_empty());
    }
}
