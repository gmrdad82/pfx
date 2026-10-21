use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use pfx_file::{Codec, Reader, Tag, Writer};
use pfx_recipe::toml_edit::{self, DocumentMut, Item};
use pfx_recipe::{Key, Kind, Patch, Recipe, apply_variant, files};
use sha2::{Digest, Sha256};

use crate::{Built, Registry, build_with, glb_path};

pub const GLTF_NAME: &str = "@gltf";
pub const ENTRY: &str = "main.toml";
pub const ASSET_RECORD: &str = "asset.pitofx";
const RECORD_DEPTH: usize = 4;

const OWNED: [Tag; 10] = [
    pfx_file::GLTF,
    pfx_file::GBIN,
    pfx_file::SCNE,
    pfx_file::SMET,
    pfx_file::RCPE,
    pfx_file::FILE,
    pfx_file::THMB,
    pfx_file::FACT,
    pfx_file::NODE,
    pfx_file::OVRD,
];

const LIGHT_KEYS: [&str; 8] = [
    "intensity",
    "color",
    "position",
    "radius",
    "range",
    "direction",
    "cone",
    "shadow",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Override {
    pub file: String,
    pub path: String,
    pub value: toml::Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneChange {
    pub file: String,
    pub path: String,
    pub was: Option<toml::Value>,
    pub now: toml::Value,
    pub recipe: Option<String>,
}

#[derive(Debug, Default)]
pub struct Document {
    pub recipe_name: String,
    pub recipe_text: String,
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    pub outputs: BTreeSet<PathBuf>,
    pub scene: Vec<(String, Vec<u8>)>,
    pub gltf: Vec<u8>,
    pub gbin: Vec<u8>,
    pub facts: String,
    pub nodes: String,
    pub thumb: Option<Vec<u8>>,
    pub overrides: Vec<Override>,
    pub built_from: Option<String>,
    pub scene_hash: Option<String>,
    pub outside: Vec<String>,
    pub unread: Vec<String>,
    pub referenced: Vec<String>,
    pub notes: Vec<String>,
    foreign: Vec<(Tag, String, Vec<u8>)>,
}

pub fn is_pfx(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(pfx_file::EXTENSION))
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub fn scene_hash_of(scene: &[(String, Vec<u8>)]) -> String {
    let mut hash = Sha256::new();
    for (name, bytes) in scene {
        hash.update(name.as_bytes());
        hash.update([0]);
        hash.update(bytes);
    }
    hex(&hash.finalize())
}

fn safe_name(name: &str) -> Result<PathBuf, String> {
    let path = Path::new(name);
    let plain = !name.is_empty()
        && name.len() <= 1024
        && !name.contains('\\')
        && path.components().all(|c| matches!(c, Component::Normal(_)));
    if plain {
        Ok(path.to_path_buf())
    } else {
        Err(format!(
            "`{name}` is not a relative path inside the document"
        ))
    }
}

fn name_of(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn relative(root: &Path, file: &Path) -> Option<PathBuf> {
    file.strip_prefix(root).ok().map(Path::to_path_buf)
}

pub(crate) fn split_glb(glb: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(glb.get(at..at + 4)?.try_into().ok()?))
    };
    if glb.get(..4) != Some(b"glTF") {
        return Err("the build is not a GLB".into());
    }
    let mut at = 12;
    let (mut json, mut bin) = (Vec::new(), Vec::new());
    while let (Some(length), Some(kind)) = (word(at), word(at + 4)) {
        let start = at + 8;
        let end = start + length as usize;
        let data = glb.get(start..end).ok_or("a GLB chunk runs past its end")?;
        match kind {
            0x4E4F_534A => json = data.to_vec(),
            0x004E_4942 => bin = data.to_vec(),
            _ => {}
        }
        at = end;
    }
    Ok((json, bin))
}

pub fn join_glb(json: &[u8], bin: &[u8]) -> Vec<u8> {
    let chunks = 8 + json.len() + if bin.is_empty() { 0 } else { 8 + bin.len() };
    let mut out = Vec::with_capacity(12 + chunks);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&((12 + chunks) as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(json);
    if !bin.is_empty() {
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
        out.extend_from_slice(bin);
    }
    out
}

fn toml_of(value: &serde_json::Value) -> Option<toml::Value> {
    Some(match value {
        serde_json::Value::Null => return None,
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => toml::Value::Integer(i),
            None => toml::Value::Float(n.as_f64()?),
        },
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Array(items) => {
            toml::Value::Array(items.iter().filter_map(toml_of).collect())
        }
        serde_json::Value::Object(map) => toml::Value::Table(
            map.iter()
                .filter_map(|(k, v)| Some((k.clone(), toml_of(v)?)))
                .collect(),
        ),
    })
}

fn node_map(json: &[u8]) -> Result<String, String> {
    let doc: serde_json::Value =
        serde_json::from_slice(json).map_err(|e| format!("the built glTF is not JSON: {e}"))?;
    let mut table = toml::Table::new();
    for (index, node) in doc["nodes"].as_array().into_iter().flatten().enumerate() {
        if let Some(name) = node["name"].as_str() {
            table.insert(name.to_string(), toml::Value::Integer(index as i64));
        }
    }
    toml::to_string(&table).map_err(|e| e.to_string())
}

#[derive(Clone, Debug, PartialEq)]
enum Seg {
    Key(String),
    Named(String),
    Index(usize),
}

fn path_text(segs: &[Seg]) -> String {
    let mut out = String::new();
    for seg in segs {
        match seg {
            Seg::Key(k) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(k);
            }
            Seg::Named(n) => {
                out.push('[');
                out.push_str(n);
                out.push(']');
            }
            Seg::Index(i) => {
                out.push('[');
                out.push('#');
                out.push_str(&i.to_string());
                out.push(']');
            }
        }
    }
    out
}

fn segs_of(text: &str) -> Result<Vec<Seg>, String> {
    let mut segs = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(inner) = rest.strip_prefix('[') {
            let end = inner
                .find(']')
                .ok_or_else(|| format!("`{text}`: an unclosed ["))?;
            let name = &inner[..end];
            segs.push(match name.strip_prefix('#') {
                Some(i) => Seg::Index(i.parse().map_err(|_| format!("`{text}`: a bad index"))?),
                None => Seg::Named(name.to_string()),
            });
            rest = &inner[end + 1..];
        } else {
            let rest_key = rest.strip_prefix('.').unwrap_or(rest);
            let end = rest_key.find(['.', '[']).unwrap_or(rest_key.len());
            if end == 0 {
                return Err(format!("`{text}`: an empty key"));
            }
            segs.push(Seg::Key(rest_key[..end].to_string()));
            rest = &rest_key[end..];
        }
    }
    Ok(segs)
}

fn named_in(array: &[toml::Value]) -> bool {
    let mut names = BTreeSet::new();
    array.iter().all(|v| {
        v.as_table()
            .and_then(|t| t.get("name"))
            .and_then(toml::Value::as_str)
            .is_some_and(|n| names.insert(n.to_string()))
    })
}

fn diff(
    at: &mut Vec<Seg>,
    was: Option<&toml::Value>,
    now: &toml::Value,
    out: &mut Vec<(Vec<Seg>, Option<toml::Value>, toml::Value)>,
) {
    match (was, now) {
        (Some(toml::Value::Table(a)), toml::Value::Table(b)) => {
            for (key, value) in b {
                at.push(Seg::Key(key.clone()));
                diff(at, a.get(key), value, out);
                at.pop();
            }
        }
        (Some(toml::Value::Array(a)), toml::Value::Array(b))
            if named_in(a) && named_in(b) && b.iter().all(toml::Value::is_table) =>
        {
            for item in b {
                let name = item["name"].as_str().unwrap_or_default().to_string();
                let old = a.iter().find(|v| v["name"].as_str() == Some(name.as_str()));
                at.push(Seg::Named(name));
                diff(at, old, item, out);
                at.pop();
            }
        }
        (Some(toml::Value::Array(a)), toml::Value::Array(b))
            if a.len() == b.len() && b.iter().all(toml::Value::is_table) =>
        {
            for (i, (old, item)) in a.iter().zip(b).enumerate() {
                at.push(Seg::Index(i));
                diff(at, Some(old), item, out);
                at.pop();
            }
        }
        (was, now) if was != Some(now) => out.push((at.clone(), was.cloned(), now.clone())),
        _ => {}
    }
}

fn item_of(value: &toml::Value) -> Result<Item, String> {
    let mut table = toml::Table::new();
    table.insert("v".into(), value.clone());
    let text = toml::to_string(&table).map_err(|e| e.to_string())?;
    let doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    doc.get("v")
        .cloned()
        .ok_or_else(|| "a value TOML cannot hold".to_string())
}

fn put_override(doc: &mut DocumentMut, segs: &[Seg], value: &toml::Value) -> Result<(), String> {
    let missing = || format!("the scene has no `{}`", path_text(segs));
    let (last, parents) = segs.split_last().ok_or_else(missing)?;
    let mut item: &mut Item = doc.as_item_mut();
    for seg in parents {
        item = match seg {
            Seg::Key(k) => item
                .get_mut(k.as_str())
                .filter(|i| !i.is_none())
                .ok_or_else(missing)?,
            Seg::Index(i) => item.get_mut(*i).ok_or_else(missing)?,
            Seg::Named(n) => {
                let at = match &*item {
                    Item::ArrayOfTables(array) => array
                        .iter()
                        .position(|t| t.get("name").and_then(Item::as_str) == Some(n.as_str())),
                    Item::Value(toml_edit::Value::Array(array)) => array.iter().position(|v| {
                        v.as_inline_table()
                            .and_then(|t| t.get("name"))
                            .and_then(toml_edit::Value::as_str)
                            == Some(n.as_str())
                    }),
                    _ => None,
                }
                .ok_or_else(missing)?;
                item.get_mut(at).ok_or_else(missing)?
            }
        };
    }
    let Seg::Key(key) = last else {
        return Err(format!("`{}` does not end in a key", path_text(segs)));
    };
    let new = item_of(value)?;
    let table = item.as_table_like_mut().ok_or_else(missing)?;
    table.insert(key, new);
    Ok(())
}

fn apply_overrides(scene: &mut [(String, Vec<u8>)], overrides: &[Override]) -> Result<(), String> {
    for o in overrides {
        let Some((_, bytes)) = scene.iter_mut().find(|(name, _)| *name == o.file) else {
            return Err(format!("override {}: the scene has no {}", o.path, o.file));
        };
        let text = String::from_utf8_lossy(bytes).into_owned();
        let mut doc: DocumentMut = text
            .parse()
            .map_err(|e: toml_edit::TomlError| format!("{}: {}", o.file, e.message()))?;
        put_override(&mut doc, &segs_of(&o.path)?, &o.value)
            .map_err(|e| format!("override {}: {e}", o.path))?;
        *bytes = doc.to_string().into_bytes();
    }
    Ok(())
}

fn overrides_text(overrides: &[Override]) -> Result<String, String> {
    let list: Vec<toml::Value> = overrides
        .iter()
        .map(|o| {
            let mut t = toml::Table::new();
            t.insert("file".into(), toml::Value::String(o.file.clone()));
            t.insert("path".into(), toml::Value::String(o.path.clone()));
            t.insert("value".into(), o.value.clone());
            toml::Value::Table(t)
        })
        .collect();
    let mut table = toml::Table::new();
    table.insert("override".into(), toml::Value::Array(list));
    toml::to_string(&table).map_err(|e| e.to_string())
}

fn overrides_of(text: &str) -> Result<Vec<Override>, String> {
    let table: toml::Table = toml::from_str(text).map_err(|e| format!("OVRD: {e}"))?;
    let mut out = Vec::new();
    for item in table
        .get("override")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        let field = |k: &str| {
            item.get(k)
                .and_then(toml::Value::as_str)
                .map(str::to_string)
        };
        match (field("file"), field("path"), item.get("value")) {
            (Some(file), Some(path), Some(value)) => out.push(Override {
                file,
                path,
                value: value.clone(),
            }),
            _ => return Err("OVRD: an override without file, path and value".into()),
        }
    }
    Ok(out)
}

fn stored(bytes: &[u8]) -> Codec {
    if bytes.starts_with(b"\x89PNG") || bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Codec::Store
    } else {
        Codec::Auto
    }
}

fn text_of(bytes: &[u8], what: &str) -> Result<String, String> {
    String::from_utf8(bytes.to_vec()).map_err(|_| format!("{what} is not UTF-8"))
}

fn asset_records(seen: &BTreeSet<PathBuf>) -> Vec<PathBuf> {
    seen.iter()
        .filter_map(|file| {
            file.ancestors()
                .skip(1)
                .take(RECORD_DEPTH)
                .map(|dir| dir.join(ASSET_RECORD))
                .find(|record| record.is_file())
        })
        .collect()
}

fn strings_in(value: &toml::Value, out: &mut Vec<String>) {
    match value {
        toml::Value::String(text) => out.push(text.clone()),
        toml::Value::Array(items) => items.iter().for_each(|item| strings_in(item, out)),
        toml::Value::Table(table) => table.values().for_each(|item| strings_in(item, out)),
        _ => {}
    }
}

fn same_bytes(file: &Path, other: &Path) -> bool {
    let length = |path: &Path| std::fs::metadata(path).map(|meta| meta.len()).ok();
    file == other
        || (length(file).is_some()
            && length(file) == length(other)
            && matches!((std::fs::read(file), std::fs::read(other)), (Ok(a), Ok(b)) if a == b))
}

fn path_between(folder: &Path, file: &Path) -> String {
    let from: Vec<_> = folder.components().collect();
    let to: Vec<_> = file.components().collect();
    let shared = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts = vec!["..".to_string(); from.len() - shared];
    parts.extend(
        to[shared..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

fn outside_paths(
    main: &Path,
    seen: &BTreeSet<PathBuf>,
    folder: &Path,
) -> (Vec<PathBuf>, Vec<String>) {
    let mut found = Vec::new();
    let mut unread = Vec::new();
    let texts = std::iter::once(main).chain(
        seen.iter()
            .map(PathBuf::as_path)
            .filter(|path| path.extension().is_some_and(|e| e == "toml")),
    );
    for text_file in texts {
        let Ok(text) = files::read_to_string(text_file) else {
            continue;
        };
        let Ok(table) = toml::from_str::<toml::Table>(&text) else {
            continue;
        };
        let mut strings = Vec::new();
        strings_in(&toml::Value::Table(table), &mut strings);
        let here = text_file.parent().unwrap_or(folder);
        for reference in strings {
            if !reference.starts_with("../") || reference.contains('\\') {
                continue;
            }
            let candidates = [here, folder].map(|base| files::clean(&base.join(&reference)));
            if candidates.iter().any(|path| path.is_dir()) {
                continue;
            }
            match candidates.iter().find(|path| files::is_file(path)) {
                Some(path) if !seen.contains(path) && !found.contains(path) => {
                    found.push(path.clone());
                }
                Some(_) => {}
                None => {
                    let shown = format!("{reference} (named in {})", text_file.display());
                    if !unread.contains(&shown) {
                        unread.push(shown);
                    }
                }
            }
        }
    }
    (found, unread)
}

fn sky_reads(recipe: &Recipe, base: &Path) -> Vec<PathBuf> {
    assets(
        recipe,
        base,
        recipe.sky.iter().filter_map(|sky| sky.path.as_deref()),
    )
    .into_iter()
    .filter(|file| reads_as_sky(file))
    .collect()
}

fn reads_as_sky(file: &Path) -> bool {
    use std::io::Read;
    let mut head = Vec::new();
    std::fs::File::open(file)
        .and_then(|opened| opened.take(1 << 16).read_to_end(&mut head))
        .is_ok_and(|_| pfx_load::Sky::size(&head).is_ok())
}

fn assets<'a>(recipe: &Recipe, base: &Path, named: impl Iterator<Item = &'a str>) -> Vec<PathBuf> {
    named
        .filter_map(|path| recipe.asset(base, path).ok())
        .map(|found| files::clean(&found))
        .filter(|file| file.is_file())
        .collect()
}

fn render_reads(recipe: &Recipe, base: &Path) -> Vec<PathBuf> {
    let mut named: Vec<&str> = Vec::new();
    if let Some(path) = recipe.sky.as_ref().and_then(|sky| sky.path.as_deref()) {
        named.push(path);
    }
    for object in &recipe.objects {
        match &object.kind {
            Kind::Card {
                image: Some(image), ..
            } => named.push(image),
            Kind::Text { spans, .. } => named.extend(
                spans
                    .iter()
                    .filter_map(|span| span.family.as_deref())
                    .filter(|family| *family != pfx_recipe::MONO),
            ),
            _ => {}
        }
    }
    assets(recipe, base, named.into_iter())
}

fn real_folder(pfx: &Path) -> PathBuf {
    match pfx.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

struct Placed {
    recipe: PathBuf,
    folder: PathBuf,
    dir: Vec<String>,
}

fn parts(path: &Path) -> Vec<String> {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect()
}

impl Placed {
    fn of(pfx: &Path, recipe_name: &str) -> Result<Placed, String> {
        let name = safe_name(recipe_name)?;
        let folder = real_folder(pfx);
        let dir = name.parent().map(parts).unwrap_or_default();
        let file = name.file_name().map(PathBuf::from).unwrap_or(name);
        Ok(Placed {
            recipe: folder.join(file),
            folder: files::clean(&folder),
            dir,
        })
    }

    fn path(&self, name: &Path) -> PathBuf {
        let name = parts(name);
        let shared = self
            .dir
            .iter()
            .zip(&name)
            .take_while(|(a, b)| a == b)
            .count();
        let mut out = self.folder.clone();
        for _ in shared..self.dir.len() {
            out.pop();
        }
        out.extend(&name[shared..]);
        out
    }

    fn top(&self) -> PathBuf {
        self.path(Path::new(""))
    }

    fn name(&self, path: &Path) -> Option<PathBuf> {
        let mut out = self.dir.clone();
        for part in path_between(&self.folder, path).split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    out.pop()?;
                }
                other => out.push(other.to_string()),
            }
        }
        (!out.is_empty()).then(|| out.iter().collect())
    }
}

fn smet_text(
    recipe: &str,
    scene_hash: Option<&str>,
    scene: &[(String, Vec<u8>)],
) -> Result<String, String> {
    let mut smet = toml::Table::new();
    smet.insert("format".into(), toml::Value::Integer(1));
    smet.insert("entry".into(), toml::Value::String(ENTRY.into()));
    smet.insert(
        "recipe".into(),
        toml::Value::String(sha256(recipe.as_bytes())),
    );
    smet.insert(
        "scene".into(),
        toml::Value::String(scene_hash.map_or_else(|| scene_hash_of(scene), str::to_string)),
    );
    toml::to_string(&smet).map_err(|e| e.to_string())
}

struct Chunks<'a> {
    writer: Writer<'a>,
    old: Option<&'a Reader<'a>>,
}

impl<'a> Chunks<'a> {
    fn put(&mut self, tag: Tag, name: &str, bytes: std::borrow::Cow<'a, [u8]>, codec: Codec) {
        let writer = std::mem::take(&mut self.writer);
        let kept = self
            .old
            .and_then(|old| same(old, tag, name, &bytes).map(|index| (old, index)));
        self.writer = match kept {
            Some((old, index)) => writer.keep(old, index),
            None => writer.named(tag, name, bytes, codec),
        };
    }
}

fn same(old: &Reader<'_>, tag: Tag, name: &str, bytes: &[u8]) -> Option<usize> {
    let index = old.find_named(tag, name)?;
    let entry = &old.entries()[index];
    (entry.raw == bytes.len() as u64
        && pfx_file::crc32::hash(bytes) == entry.crc
        && old.read(index).ok()?.as_ref() == bytes)
        .then_some(index)
}

impl Document {
    pub fn pack(recipe: &Path, registry: &Registry) -> Result<Document, String> {
        Document::packed(recipe, registry, None)
    }

    pub fn pack_for(recipe: &Path, registry: &Registry, out: &Path) -> Result<Document, String> {
        Document::packed(recipe, registry, Some(out))
    }

    fn packed(recipe: &Path, registry: &Registry, out: Option<&Path>) -> Result<Document, String> {
        let shown = recipe.display().to_string();
        let recorder = files::record();
        let loaded = pfx_recipe::load(recipe).map_err(|e| format!("{shown}: {e}"))?;
        let base = real_folder(recipe);
        let built =
            build_with(&loaded.recipe, &base, registry).map_err(|e| format!("{shown}: {e}"))?;
        let mut doc = Document::default();
        let filled = doc.fill(&built, &loaded.recipe, &base, &base);
        let variants: Vec<Recipe> = loaded
            .recipe
            .variants
            .keys()
            .filter_map(|name| apply_variant(&loaded.recipe, name).ok())
            .collect();
        for variant in &variants {
            let _ = build_with(variant, &base, registry);
        }
        let mut seen = recorder.seen();
        drop(recorder);
        for recipe in std::iter::once(&loaded.recipe).chain(&variants) {
            seen.extend(render_reads(recipe, &base));
        }
        let main = files::clean(recipe);
        seen.remove(&main);
        let folder = main.parent().map(Path::to_path_buf).unwrap_or_default();
        let there = out.and_then(|out| files::clean(out).parent().map(Path::to_path_buf));
        let skies: BTreeSet<PathBuf> = std::iter::once(&loaded.recipe)
            .chain(&variants)
            .flat_map(|recipe| sky_reads(recipe, &base))
            .filter(|file| file.starts_with(&folder) && !files::mounted(file))
            .filter(|file| {
                there
                    .as_ref()
                    .is_some_and(|there| same_bytes(file, &there.join(path_between(&folder, file))))
            })
            .collect();
        seen.retain(|file| !skies.contains(file));
        doc.referenced = skies
            .iter()
            .map(|file| path_between(&folder, file))
            .collect();
        let records = asset_records(&seen);
        seen.extend(records);
        let (found, unread) = outside_paths(&main, &seen, &folder);
        seen.extend(found);
        doc.unread = unread;
        let mut root = main.parent().map(Path::to_path_buf).unwrap_or_default();
        for file in &seen {
            while !file.starts_with(&root) {
                if !root.pop() {
                    break;
                }
            }
        }
        if root.parent().is_none() {
            return Err(format!(
                "{shown}: its files share no folder below the file system's root, expected them beside the recipe or in folders near it"
            ));
        }
        if root == folder {
            filled?;
        } else {
            doc.fill(&built, &loaded.recipe, &base, &root)?;
        }
        doc.recipe_name = relative(&root, &main)
            .map(|p| name_of(&p))
            .unwrap_or_default();
        doc.recipe_text = files::read_to_string(recipe).map_err(|e| format!("{shown}: {e}"))?;
        for file in seen {
            let Some(name) = relative(&root, &file) else {
                continue;
            };
            if !file.starts_with(&folder) {
                doc.outside.push(path_between(&folder, &file));
            }
            let bytes = files::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            doc.files.insert(name, bytes);
        }
        let beside = std::mem::take(&mut doc.foreign);
        for (_, name, bytes) in beside {
            doc.files.entry(PathBuf::from(name)).or_insert(bytes);
        }
        doc.built_from = Some(doc.hash());
        doc.notes.clone_from(&built.notes);
        doc.facts = doc.facts_text(&built)?;
        Ok(doc)
    }

    fn fill(
        &mut self,
        built: &Built,
        recipe: &Recipe,
        base: &Path,
        top: &Path,
    ) -> Result<(), String> {
        let glb = built.try_glb().map_err(|e| e.to_string())?;
        let (json, bin) = split_glb(&glb)?;
        self.nodes = node_map(&json)?;
        self.gltf = json;
        self.gbin = bin;
        let scene_file = top.join(ENTRY);
        let glb_file = glb_path(recipe, base, None);
        self.scene.clear();
        self.outputs.clear();
        let mut beside = Vec::new();
        match crate::scene::text_named(
            built,
            recipe,
            (&scene_file, &glb_file, Some(GLTF_NAME)),
            base,
        ) {
            Ok(written) => {
                self.scene
                    .push((ENTRY.to_string(), written.text.into_bytes()));
                let folder = files::clean(top);
                if let Some((path, text)) = written.library
                    && let Some(name) = relative(&folder, &files::clean(&path))
                {
                    self.scene.push((name_of(&name), text.into_bytes()));
                }
                for (path, bytes) in written.files {
                    if let Some(name) = relative(&folder, &files::clean(&path)) {
                        self.outputs.insert(name.clone());
                        beside.push((pfx_file::FILE, name_of(&name), bytes.into_owned()));
                    }
                }
            }
            Err(e) if recipe.output.scene.is_some() => return Err(e.to_string()),
            Err(_) => {}
        }
        apply_overrides(&mut self.scene, &self.overrides)?;
        self.scene_hash = Some(scene_hash_of(&self.scene));
        self.foreign.retain(|(tag, _, _)| *tag != pfx_file::FILE);
        self.foreign.extend(beside);
        Ok(())
    }

    fn facts_text(&self, built: &Built) -> Result<String, String> {
        let mut table = toml::Table::new();
        if let Ok(before) = toml::from_str::<toml::Table>(&self.facts) {
            for key in ["licence", "credit", "author", "source"] {
                if let Some(value) = before.get(key).and_then(toml::Value::as_str) {
                    table.insert(key.into(), toml::Value::String(value.into()));
                }
            }
        }
        table.insert(
            "pfx".into(),
            toml::Value::String(env!("CARGO_PKG_VERSION").into()),
        );
        let mut build = toml::Table::new();
        if let Some(from) = &self.built_from {
            build.insert("from".into(), toml::Value::String(from.clone()));
        }
        build.insert(
            "outputs".into(),
            toml::Value::Array(
                self.outputs
                    .iter()
                    .map(|p| toml::Value::String(name_of(p)))
                    .collect(),
            ),
        );
        if !self.notes.is_empty() {
            build.insert(
                "notes".into(),
                toml::Value::Array(
                    self.notes
                        .iter()
                        .map(|note| toml::Value::String(note.clone()))
                        .collect(),
                ),
            );
        }
        table.insert("build".into(), toml::Value::Table(build));
        let objects: toml::Table = built
            .object_facts()
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), toml_of(v)?)))
            .collect();
        table.insert("objects".into(), toml::Value::Table(objects));
        let provenance: toml::Table = built
            .object_provenance()
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), toml_of(v)?)))
            .collect();
        if !provenance.is_empty() {
            table.insert("provenance".into(), toml::Value::Table(provenance));
        }
        toml::to_string(&table).map_err(|e| format!("the facts: {e}"))
    }

    pub fn hash(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(self.recipe_name.as_bytes());
        hash.update([0]);
        hash.update(self.recipe_text.as_bytes());
        for (name, bytes) in self.inputs() {
            hash.update([0]);
            hash.update(name_of(name).as_bytes());
            hash.update([0]);
            hash.update(bytes);
        }
        hex(&hash.finalize())
    }

    pub fn inputs(&self) -> impl Iterator<Item = (&PathBuf, &Vec<u8>)> {
        self.files
            .iter()
            .filter(|(name, _)| !self.outputs.contains(*name))
    }

    pub fn fresh(&self) -> bool {
        !self.gltf.is_empty() && self.built_from.as_deref() == Some(self.hash().as_str())
    }

    pub fn edited(&self) -> bool {
        self.scene_hash
            .as_deref()
            .is_some_and(|recorded| recorded != scene_hash_of(&self.scene))
    }

    pub fn glb(&self) -> Vec<u8> {
        join_glb(&self.gltf, &self.gbin)
    }

    pub fn open(path: &Path) -> Result<Document, String> {
        let reader = Reader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Document::read(&reader).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn read(reader: &Reader<'_>) -> Result<Document, String> {
        let mut doc = Document::default();
        let mut recipe = None;
        for (index, entry) in reader.entries().iter().enumerate() {
            let bytes = reader.read(index).map_err(|e| e.to_string())?.into_owned();
            match entry.tag {
                pfx_file::RCPE => {
                    let name = if entry.name.is_empty() {
                        "recipe.toml".to_string()
                    } else {
                        entry.name.clone()
                    };
                    safe_name(&name)?;
                    recipe = Some((name, text_of(&bytes, "RCPE")?));
                }
                pfx_file::FILE => {
                    doc.files.insert(safe_name(&entry.name)?, bytes);
                }
                pfx_file::SCNE => {
                    safe_name(&entry.name)?;
                    doc.scene.push((entry.name.clone(), bytes));
                }
                pfx_file::GLTF => doc.gltf = bytes,
                pfx_file::GBIN => doc.gbin = bytes,
                pfx_file::FACT => doc.facts = text_of(&bytes, "FACT")?,
                pfx_file::NODE => doc.nodes = text_of(&bytes, "NODE")?,
                pfx_file::THMB => doc.thumb = Some(bytes),
                pfx_file::OVRD => doc.overrides = overrides_of(&text_of(&bytes, "OVRD")?)?,
                pfx_file::SMET => {
                    let table: toml::Table = toml::from_str(&text_of(&bytes, "SMET")?)
                        .map_err(|e| format!("SMET: {e}"))?;
                    doc.scene_hash = table
                        .get("scene")
                        .and_then(toml::Value::as_str)
                        .map(str::to_string);
                }
                tag => doc.foreign.push((tag, entry.name.clone(), bytes)),
            }
        }
        let (name, text) = recipe.ok_or("the document has no recipe (RCPE)")?;
        let own = Path::new(&name)
            .parent()
            .map_or_else(|| PathBuf::from(GLTF_NAME), |dir| dir.join(GLTF_NAME));
        if !doc.gltf.is_empty()
            && text.contains(&format!("\"{GLTF_NAME}\""))
            && !doc.files.contains_key(&own)
        {
            doc.files.insert(own, join_glb(&doc.gltf, &doc.gbin));
        }
        doc.recipe_name = name;
        doc.recipe_text = text;
        if let Ok(facts) = toml::from_str::<toml::Table>(&doc.facts)
            && let Some(build) = facts.get("build").and_then(toml::Value::as_table)
        {
            doc.built_from = build
                .get("from")
                .and_then(toml::Value::as_str)
                .map(str::to_string);
            doc.outputs = build
                .get("outputs")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(toml::Value::as_str)
                .map(PathBuf::from)
                .collect();
            doc.notes = build
                .get("notes")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(toml::Value::as_str)
                .map(str::to_string)
                .collect();
        }
        Ok(doc)
    }

    pub fn to_vec(&self) -> Result<Vec<u8>, String> {
        self.writer(None)?.to_vec().map_err(|e| e.to_string())
    }

    fn writer<'a>(&'a self, old: Option<&'a Reader<'a>>) -> Result<Writer<'a>, String> {
        let smet = smet_text(&self.recipe_text, self.scene_hash.as_deref(), &self.scene)?;
        let mut w = Chunks {
            writer: Writer::new(),
            old,
        };
        w.put(pfx_file::SMET, "", smet.into_bytes().into(), Codec::Store);
        w.put(
            pfx_file::RCPE,
            &self.recipe_name,
            self.recipe_text.as_bytes().into(),
            Codec::Auto,
        );
        for (name, bytes) in &self.files {
            w.put(
                pfx_file::FILE,
                &name_of(name),
                bytes[..].into(),
                stored(bytes),
            );
        }
        for (name, bytes) in &self.scene {
            w.put(pfx_file::SCNE, name, bytes[..].into(), Codec::Auto);
        }
        if !self.gltf.is_empty() {
            w.put(pfx_file::GLTF, "", self.gltf[..].into(), Codec::Auto);
            w.put(pfx_file::GBIN, "", self.gbin[..].into(), Codec::Auto);
        }
        w.put(
            pfx_file::FACT,
            "",
            self.facts.as_bytes().into(),
            Codec::Auto,
        );
        w.put(
            pfx_file::NODE,
            "",
            self.nodes.as_bytes().into(),
            Codec::Auto,
        );
        if let Some(thumb) = &self.thumb {
            w.put(pfx_file::THMB, "", thumb[..].into(), Codec::Store);
        }
        if !self.overrides.is_empty() {
            w.put(
                pfx_file::OVRD,
                "",
                overrides_text(&self.overrides)?.into_bytes().into(),
                Codec::Auto,
            );
        }
        for (tag, name, bytes) in &self.foreign {
            if !OWNED.contains(tag) {
                w.put(*tag, name, bytes[..].into(), Codec::Auto);
            }
        }
        Ok(w.writer)
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        let shown = |e: pfx_file::Error| format!("{}: {e}", path.display());
        let old = Reader::open(path).ok();
        self.writer(old.as_ref())?
            .update(path)
            .map(drop)
            .map_err(shown)
    }

    pub fn write_whole(&self, path: &Path) -> Result<(), String> {
        self.writer(None)?
            .write(path)
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn unpack(&self, dir: &Path) -> Result<PathBuf, String> {
        let fail = |path: &Path, e: std::io::Error| format!("{}: {e}", path.display());
        let recipe = dir.join(safe_name(&self.recipe_name)?);
        let mut writes: Vec<(PathBuf, &[u8])> = vec![(recipe.clone(), self.recipe_text.as_bytes())];
        for (name, bytes) in self.inputs() {
            writes.push((dir.join(safe_name(&name_of(name))?), bytes));
        }
        for (path, bytes) in writes {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| fail(parent, e))?;
            }
            std::fs::write(&path, bytes).map_err(|e| fail(&path, e))?;
        }
        Ok(recipe)
    }

    pub fn mount(&self, pfx: &Path) -> Result<(PathBuf, files::Mount), String> {
        let placed = Placed::of(pfx, &self.recipe_name)?;
        let mut mounted = BTreeMap::new();
        mounted.insert(placed.recipe.clone(), self.recipe_text.clone().into_bytes());
        for (name, bytes) in &self.files {
            mounted.insert(placed.path(&safe_name(&name_of(name))?), bytes.clone());
        }
        Ok((placed.recipe, files::mount(mounted)))
    }

    pub fn beside(&self, pfx: &Path) -> Result<Vec<(String, PathBuf)>, String> {
        let (recipe_path, _mount) = self.mount(pfx)?;
        let loaded =
            pfx_recipe::load(&recipe_path).map_err(|e| format!("{}: {e}", pfx.display()))?;
        let base = real_folder(&recipe_path);
        let folder = files::clean(&base);
        let variants: Vec<Recipe> = loaded
            .recipe
            .variants
            .keys()
            .filter_map(|name| apply_variant(&loaded.recipe, name).ok())
            .collect();
        let found: BTreeSet<PathBuf> = std::iter::once(&loaded.recipe)
            .chain(&variants)
            .flat_map(|recipe| sky_reads(recipe, &base))
            .filter(|file| file.starts_with(&folder) && !files::mounted(file))
            .collect();
        Ok(found
            .into_iter()
            .map(|file| (path_between(&folder, &file), file))
            .collect())
    }

    pub fn take_changes(&mut self, pfx: &Path, mount: &files::Mount) -> bool {
        let Ok(placed) = Placed::of(pfx, &self.recipe_name) else {
            return false;
        };
        let changed = mount.changed();
        for (path, bytes) in &changed {
            let Some(name) = placed.name(path) else {
                continue;
            };
            if name_of(&name) == self.recipe_name {
                self.recipe_text = String::from_utf8_lossy(bytes).into_owned();
            } else {
                self.files.insert(name, bytes.clone());
            }
        }
        mount.settle();
        !changed.is_empty()
    }

    pub fn rebuild(&mut self, pfx: &Path, registry: &Registry) -> Result<(), String> {
        let (recipe_path, _mount) = self.mount(pfx)?;
        let shown = pfx.display().to_string();
        let loaded = pfx_recipe::load(&recipe_path).map_err(|e| format!("{shown}: {e}"))?;
        let base = real_folder(&recipe_path);
        let built =
            build_with(&loaded.recipe, &base, registry).map_err(|e| format!("{shown}: {e}"))?;
        let outputs: Vec<PathBuf> = self.outputs.iter().cloned().collect();
        for name in outputs {
            self.files.remove(&name);
        }
        let top = Placed::of(pfx, &self.recipe_name)?.top();
        self.fill(&built, &loaded.recipe, &base, &top)?;
        let beside = std::mem::take(&mut self.foreign);
        for (tag, name, bytes) in beside {
            if tag == pfx_file::FILE {
                self.files.entry(PathBuf::from(name)).or_insert(bytes);
            } else {
                self.foreign.push((tag, name, bytes));
            }
        }
        self.built_from = Some(self.hash());
        self.notes.clone_from(&built.notes);
        self.facts = self.facts_text(&built)?;
        Ok(())
    }

    pub fn scene_changes(
        &self,
        pfx: &Path,
        registry: &Registry,
    ) -> Result<Vec<SceneChange>, String> {
        let mut fresh = Document {
            recipe_name: self.recipe_name.clone(),
            recipe_text: self.recipe_text.clone(),
            files: self.files.clone(),
            outputs: self.outputs.clone(),
            overrides: self.overrides.clone(),
            ..Document::default()
        };
        fresh.rebuild(pfx, registry)?;
        let lights: BTreeSet<String> = {
            let (path, _mount) = self.mount(pfx)?;
            pfx_recipe::load(&path)
                .map(|l| l.recipe.lights.iter().map(|l| l.name.clone()).collect())
                .unwrap_or_default()
        };
        let mut out = Vec::new();
        for (file, bytes) in &self.scene {
            let parse = |b: &[u8]| -> Result<toml::Value, String> {
                toml::from_str::<toml::Table>(&String::from_utf8_lossy(b))
                    .map(toml::Value::Table)
                    .map_err(|e| format!("{file}: {e}"))
            };
            let now = parse(bytes)?;
            let was = match fresh.scene.iter().find(|(name, _)| name == file) {
                Some((_, b)) => Some(parse(b)?),
                None => None,
            };
            let mut found = Vec::new();
            diff(&mut Vec::new(), was.as_ref(), &now, &mut found);
            for (segs, was, now) in found {
                let recipe = match (file.as_str(), segs.as_slice()) {
                    (ENTRY, [Seg::Key(light), Seg::Named(name), Seg::Key(key)])
                        if light == "light"
                            && lights.contains(name)
                            && LIGHT_KEYS.contains(&key.as_str()) =>
                    {
                        Some(format!("light.{name}.{key}"))
                    }
                    _ => None,
                };
                out.push(SceneChange {
                    file: file.clone(),
                    path: path_text(&segs),
                    was,
                    now,
                    recipe,
                });
            }
        }
        Ok(out)
    }

    pub fn keep(&mut self, changes: &[SceneChange]) {
        for change in changes {
            self.overrides
                .retain(|o| !(o.file == change.file && o.path == change.path));
            self.overrides.push(Override {
                file: change.file.clone(),
                path: change.path.clone(),
                value: change.now.clone(),
            });
        }
        self.scene_hash = Some(scene_hash_of(&self.scene));
    }

    pub fn fold(
        &mut self,
        pfx: &Path,
        changes: &[SceneChange],
    ) -> Result<Vec<SceneChange>, String> {
        let (path, mount) = self.mount(pfx)?;
        let mut loaded = pfx_recipe::load(&path).map_err(|e| e.to_string())?;
        let mut kept = Vec::new();
        for change in changes {
            let Some(target) = &change.recipe else {
                kept.push(change.clone());
                continue;
            };
            let parts: Vec<&str> = target.splitn(3, '.').collect();
            let [table, name, key] = parts[..] else {
                kept.push(change.clone());
                continue;
            };
            let patch = Patch {
                path: vec![
                    Key::Table(table.to_string()),
                    Key::Object(name.to_string()),
                    Key::Table(key.to_string()),
                ],
                value: Some(item_of(&change.now)?),
            };
            pfx_recipe::apply(&mut loaded, &patch).map_err(|e| format!("{target}: {e}"))?;
        }
        pfx_recipe::save(&loaded).map_err(|e| e.to_string())?;
        self.take_changes(pfx, &mount);
        drop(mount);
        self.keep(&kept);
        Ok(kept)
    }
}

pub struct Draft {
    reader: std::sync::Arc<Reader<'static>>,
    pub recipe_name: String,
    pub recipe_text: String,
    recipe_changed: bool,
    files: BTreeMap<PathBuf, usize>,
    own_gltf: Option<PathBuf>,
    changed: BTreeMap<PathBuf, Vec<u8>>,
    scene_hash: Option<String>,
    scene: Vec<(String, Vec<u8>)>,
}

impl Draft {
    pub fn open(path: &Path) -> Result<Draft, String> {
        let shown = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
        let reader = Reader::open(path).map_err(|e| shown(&e))?;
        let mut recipe = None;
        let mut files = BTreeMap::new();
        let mut scene = Vec::new();
        let mut scene_hash = None;
        let mut gltf = false;
        for (index, entry) in reader.entries().iter().enumerate() {
            match entry.tag {
                pfx_file::RCPE => {
                    let name = if entry.name.is_empty() {
                        "recipe.toml".to_string()
                    } else {
                        entry.name.clone()
                    };
                    safe_name(&name).map_err(|e| shown(&e))?;
                    let bytes = reader.read(index).map_err(|e| shown(&e))?;
                    recipe = Some((name, text_of(&bytes, "RCPE").map_err(|e| shown(&e))?));
                }
                pfx_file::FILE => {
                    files.insert(safe_name(&entry.name).map_err(|e| shown(&e))?, index);
                }
                pfx_file::SCNE => {
                    safe_name(&entry.name).map_err(|e| shown(&e))?;
                    let bytes = reader.read(index).map_err(|e| shown(&e))?;
                    scene.push((entry.name.clone(), bytes.into_owned()));
                }
                pfx_file::GLTF => gltf = true,
                pfx_file::SMET => {
                    let bytes = reader.read(index).map_err(|e| shown(&e))?;
                    let table: toml::Table =
                        toml::from_str(&text_of(&bytes, "SMET").map_err(|e| shown(&e))?)
                            .map_err(|e| shown(&format!("SMET: {e}")))?;
                    scene_hash = table
                        .get("scene")
                        .and_then(toml::Value::as_str)
                        .map(str::to_string);
                }
                _ => {}
            }
        }
        let (recipe_name, recipe_text) =
            recipe.ok_or_else(|| shown(&"the document has no recipe (RCPE)"))?;
        let own = Path::new(&recipe_name)
            .parent()
            .map_or_else(|| PathBuf::from(GLTF_NAME), |dir| dir.join(GLTF_NAME));
        let own_gltf = (gltf
            && recipe_text.contains(&format!("\"{GLTF_NAME}\""))
            && !files.contains_key(&own))
        .then_some(own);
        Ok(Draft {
            reader: std::sync::Arc::new(reader),
            recipe_name,
            recipe_text,
            recipe_changed: false,
            files,
            own_gltf,
            changed: BTreeMap::new(),
            scene_hash,
            scene,
        })
    }

    pub fn edited(&self) -> bool {
        self.scene_hash
            .as_deref()
            .is_some_and(|recorded| recorded != scene_hash_of(&self.scene))
    }

    pub fn holds(&self, name: &Path) -> bool {
        self.files.contains_key(name)
    }

    pub fn path_of(&self, pfx: &Path, name: &str) -> Result<PathBuf, String> {
        let placed = Placed::of(pfx, &self.recipe_name)?;
        if name == self.recipe_name {
            return Ok(placed.recipe);
        }
        Ok(placed.path(&safe_name(name)?))
    }

    pub fn mount(&self, pfx: &Path, label: &str) -> Result<(PathBuf, files::Mount), String> {
        let placed = Placed::of(pfx, &self.recipe_name)?;
        let recipe = placed.recipe.clone();
        let mut mounted = BTreeMap::new();
        let mut names = BTreeMap::new();
        mounted.insert(
            recipe.clone(),
            files::Content::Bytes(self.recipe_text.clone().into_bytes()),
        );
        names.insert(recipe.clone(), label.to_string());
        for (name, &index) in &self.files {
            let path = placed.path(name);
            let reader = self.reader.clone();
            let load: files::Load = std::sync::Arc::new(move || {
                reader
                    .read(index)
                    .map(std::borrow::Cow::into_owned)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
            });
            let inside = Path::new(&self.recipe_name)
                .parent()
                .and_then(|dir| name.strip_prefix(dir).ok())
                .unwrap_or(name);
            names.insert(path.clone(), format!("{label}:{}", name_of(inside)));
            mounted.insert(path, files::Content::Lazy(load));
        }
        if let Some(own) = &self.own_gltf {
            let reader = self.reader.clone();
            let load: files::Load = std::sync::Arc::new(move || {
                let chunk = |tag| {
                    reader
                        .chunk(tag)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
                        .map(|c| c.map(std::borrow::Cow::into_owned).unwrap_or_default())
                };
                Ok(join_glb(&chunk(pfx_file::GLTF)?, &chunk(pfx_file::GBIN)?))
            });
            mounted.insert(placed.path(own), files::Content::Lazy(load));
        }
        Ok((recipe, files::mount_named(mounted, names)))
    }

    pub fn take_changes(&mut self, pfx: &Path, mount: &files::Mount) -> bool {
        let Ok(placed) = Placed::of(pfx, &self.recipe_name) else {
            return false;
        };
        let changed = mount.changed();
        for (path, bytes) in &changed {
            let Some(name) = placed.name(path) else {
                continue;
            };
            if name_of(&name) == self.recipe_name {
                self.recipe_text = String::from_utf8_lossy(bytes).into_owned();
                self.recipe_changed = true;
            } else if self.own_gltf.as_ref() != Some(&name) {
                self.changed.insert(name, bytes.clone());
            }
        }
        mount.settle();
        !changed.is_empty()
    }

    pub fn write(&self, pfx: &Path) -> Result<pfx_file::Update, String> {
        let reader: &Reader<'_> = &self.reader;
        let smet = smet_text(&self.recipe_text, self.scene_hash.as_deref(), &self.scene)?;
        let mut writer = Writer::new();
        let mut written = BTreeSet::new();
        for (index, entry) in reader.entries().iter().enumerate() {
            let name = Path::new(&entry.name);
            writer = match entry.tag {
                pfx_file::SMET => writer.chunk(pfx_file::SMET, smet.as_bytes(), Codec::Store),
                pfx_file::RCPE if self.recipe_changed => writer.named(
                    pfx_file::RCPE,
                    entry.name.as_str(),
                    self.recipe_text.as_bytes(),
                    Codec::Auto,
                ),
                pfx_file::FILE if self.changed.contains_key(name) => {
                    written.insert(name.to_path_buf());
                    let bytes = &self.changed[name];
                    writer.named(
                        pfx_file::FILE,
                        entry.name.as_str(),
                        &bytes[..],
                        stored(bytes),
                    )
                }
                _ => writer.keep(reader, index),
            };
        }
        for (name, bytes) in &self.changed {
            if !written.contains(name) {
                writer = writer.named(pfx_file::FILE, name_of(name), &bytes[..], stored(bytes));
            }
        }
        writer
            .update(pfx)
            .map_err(|e| format!("{}: {e}", pfx.display()))
    }
}
