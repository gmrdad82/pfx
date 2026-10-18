use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pfx_file::{Codec, FILE, GLTF, Reader, SCNE, SMET, Writer};

use crate::paths::{folder, join, rooted, slashed};
use crate::{Diagnostic, FORMAT, Project, Severity};

pub const GLTF_PATH: &str = "@gltf";

#[derive(Clone)]
pub(crate) struct Mount {
    pub file: PathBuf,
    pub entry: PathBuf,
    pub texts: BTreeMap<PathBuf, String>,
    pub files: BTreeSet<String>,
    pub gltf: bool,
    pub memory: bool,
    bytes: Arc<[u8]>,
}

impl fmt::Debug for Mount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mount")
            .field("file", &self.file)
            .field("entry", &self.entry)
            .field("texts", &self.texts.keys().collect::<Vec<_>>())
            .field("files", &self.files)
            .field("gltf", &self.gltf)
            .field("memory", &self.memory)
            .finish()
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn container(error: pfx_file::Error) -> io::Error {
    match error {
        pfx_file::Error::Io(error) => error,
        other => invalid(other.to_string()),
    }
}

pub(crate) fn inner_name(name: &str) -> Result<PathBuf, String> {
    match rooted(name) {
        Ok(joined) if joined == name => Ok(PathBuf::from(joined)),
        _ => Err(format!(
            "holds a chunk named {name}, which is not a path inside the project"
        )),
    }
}

#[derive(serde::Deserialize)]
struct Meta {
    format: u32,
    entry: String,
}

impl Mount {
    pub fn read(root: &Path, file: &Path) -> Result<Mount, String> {
        let bytes: Arc<[u8]> = std::fs::read(root.join(file))
            .map_err(|error| format!("cannot read: {error}"))?
            .into();
        Mount::from_bytes(file.to_path_buf(), bytes, false)
    }

    pub fn from_bytes(file: PathBuf, bytes: Arc<[u8]>, memory: bool) -> Result<Mount, String> {
        let reader = Reader::from_bytes(&bytes).map_err(|error| error.to_string())?;
        let meta = reader
            .chunk(SMET)
            .map_err(|error| error.to_string())?
            .ok_or("holds no scene: it has no SMET chunk")?;
        let meta = std::str::from_utf8(&meta)
            .ok()
            .and_then(|text| toml::from_str::<Meta>(text).ok())
            .ok_or("has an SMET chunk that is not a format and an entry")?;
        if meta.format > FORMAT {
            return Err(format!(
                "holds a scene of format {}, newer than this reader's {FORMAT}",
                meta.format
            ));
        }
        let mut texts = BTreeMap::new();
        let mut files = BTreeSet::new();
        for (index, entry) in reader.entries().iter().enumerate() {
            if entry.tag == SCNE {
                let name = inner_name(&entry.name)?;
                let raw = reader.read(index).map_err(|error| error.to_string())?;
                let text = String::from_utf8(raw.into_owned())
                    .map_err(|_| format!("holds {} as text that is not UTF-8", entry.name))?;
                texts.insert(name, text);
            } else if entry.tag == FILE {
                inner_name(&entry.name)?;
                files.insert(entry.name.clone());
            }
        }
        let entry = inner_name(&meta.entry)?;
        if !texts.contains_key(&entry) {
            return Err(format!(
                "names {} as its scene, which it does not hold",
                meta.entry
            ));
        }
        let gltf = reader.find(GLTF).is_some();
        Ok(Mount {
            file,
            entry,
            texts,
            files,
            gltf,
            memory,
            bytes,
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn reader(&self) -> Option<Reader<'_>> {
        Reader::from_bytes(&self.bytes).ok()
    }

    pub fn holds(&self, path: &str) -> bool {
        self.files.contains(path) || self.texts.contains_key(Path::new(path))
    }

    pub fn file_bytes(&self, name: &str) -> Option<io::Result<Cow<'_, [u8]>>> {
        if !self.files.contains(name) {
            return None;
        }
        let reader = Reader::from_bytes(&self.bytes).map_err(container);
        Some(reader.and_then(|reader| {
            reader
                .named(FILE, name)
                .map_err(container)?
                .ok_or_else(|| invalid(format!("{name} is not in the file")))
        }))
    }
}

pub(crate) fn pfx_of(path: &Path) -> Option<(PathBuf, String)> {
    for ancestor in path.ancestors().skip(1) {
        if pfx_file::is_file_path(ancestor) && ancestor.is_file() {
            let inner = path.strip_prefix(ancestor).ok()?;
            return Some((ancestor.to_path_buf(), slashed(inner)));
        }
    }
    None
}

pub(crate) fn current(path: &Path) -> io::Result<String> {
    let Some((pfx, inner)) = pfx_of(path) else {
        return std::fs::read_to_string(path);
    };
    let reader = Reader::open(&pfx).map_err(container)?;
    let raw = reader
        .named(SCNE, &inner)
        .map_err(container)?
        .ok_or_else(|| invalid(format!("{inner} is not in {}", pfx.display())))?;
    String::from_utf8(raw.into_owned()).map_err(|_| invalid(format!("{inner} is not UTF-8")))
}

fn rewrite(pfx: &Path, changes: &BTreeMap<String, &str>) -> io::Result<()> {
    let bytes = std::fs::read(pfx)?;
    let reader = Reader::from_bytes(&bytes).map_err(container)?;
    let mut writer = Writer::new();
    let mut done = BTreeSet::new();
    for (index, entry) in reader.entries().iter().enumerate() {
        match changes.get(&entry.name) {
            Some(text) if entry.tag == SCNE => {
                writer = writer.named(SCNE, entry.name.clone(), text.as_bytes(), Codec::Auto);
                done.insert(entry.name.as_str());
            }
            _ => writer = writer.copy(&reader, index).map_err(container)?,
        }
    }
    for (name, text) in changes {
        if !done.contains(name.as_str()) {
            writer = writer.named(SCNE, name.clone(), text.as_bytes(), Codec::Auto);
        }
    }
    writer.write(pfx).map_err(container)
}

type Texts<'a> = BTreeMap<String, &'a str>;

pub(crate) fn store(
    changes: &BTreeMap<&Path, (&str, &str)>,
    write: impl Fn(&Path, &str) -> io::Result<()>,
) -> Result<(), (PathBuf, io::Error)> {
    let mut pfx_files: BTreeMap<PathBuf, (Texts, Texts)> = BTreeMap::new();
    let mut disk = Vec::new();
    for (&file, &(before, after)) in changes {
        match pfx_of(file) {
            Some((pfx, inner)) => {
                let (afters, befores) = pfx_files.entry(pfx).or_default();
                afters.insert(inner.clone(), after);
                befores.insert(inner, before);
            }
            None => disk.push((file, before, after)),
        }
    }
    let mut undo_pfx_files: Vec<(&Path, &BTreeMap<String, &str>)> = Vec::new();
    let mut undo_disk: Vec<(&Path, &str)> = Vec::new();
    let mut failed = None;
    for (pfx, (afters, befores)) in &pfx_files {
        if let Err(error) = rewrite(pfx, afters) {
            failed = Some((pfx.clone(), error));
            break;
        }
        undo_pfx_files.push((pfx, befores));
    }
    if failed.is_none() {
        for (file, before, after) in disk {
            if let Err(error) = write(file, after) {
                failed = Some((file.to_path_buf(), error));
                break;
            }
            undo_disk.push((file, before));
        }
    }
    let Some(failed) = failed else {
        return Ok(());
    };
    for (file, text) in undo_disk {
        let _ = write(file, text);
    }
    for (pfx, befores) in undo_pfx_files {
        let _ = rewrite(pfx, befores);
    }
    Err(failed)
}

fn refusal(file: &Path, message: String) -> Diagnostic {
    Diagnostic {
        file: file.to_path_buf(),
        line: 1,
        column: 1,
        end_line: 1,
        end_column: 1,
        severity: Severity::Error,
        code: crate::code::UNREADABLE,
        key: String::new(),
        message,
        related: Vec::new(),
    }
}

pub fn pack(project: &Project, scene: &Path) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let (resolved, found) = project.closure(scene)?;
    let entry = project.relative(scene);
    let mut tomls: BTreeSet<PathBuf> = resolved.files.iter().cloned().collect();
    let mut assets: BTreeSet<PathBuf> = BTreeSet::new();
    for path in found {
        if path.extension().is_some_and(|end| end == "toml") {
            tomls.insert(path);
        } else {
            assets.insert(path);
        }
    }
    tomls.remove(&entry);
    let order: Vec<PathBuf> = std::iter::once(entry.clone()).chain(tomls).collect();
    let mut texts = Vec::with_capacity(order.len());
    for file in &order {
        let text = project.source(file).map_err(|error| {
            vec![refusal(
                file,
                format!("cannot read {}: {error}", file.display()),
            )]
        })?;
        texts.push((slashed(file), text));
    }
    let beside: Vec<PathBuf> = assets
        .iter()
        .filter(|path| path.extension().is_some_and(|end| end == "gltf"))
        .filter_map(|path| {
            let bytes = project.asset(&slashed(path)).ok()?;
            Some(gltf_files(path, &bytes))
        })
        .flatten()
        .collect();
    assets.extend(beside);
    let mut files = Vec::new();
    for path in &assets {
        let name = slashed(path);
        let mounted = project.pfx_path().is_some() && project.asset(&name).is_ok();
        if !mounted && !project.root().join(path).is_file() {
            continue;
        }
        let bytes = project
            .asset(&name)
            .map_err(|error| vec![refusal(path, format!("cannot read {name}: {error}"))])?;
        files.push((name, bytes));
    }
    let borrowed: Vec<(String, &str)> = texts
        .iter()
        .map(|(name, text)| (name.clone(), text.as_str()))
        .collect();
    let meta = meta(&entry, &scene_hash(&borrowed));
    let mut writer = Writer::new();
    let original = project.pfx();
    if let Some(reader) = &original {
        for (index, chunk) in reader.entries().iter().enumerate() {
            if ![SCNE, SMET, FILE].contains(&chunk.tag) {
                writer = writer
                    .copy(reader, index)
                    .map_err(|error| vec![refusal(&entry, error.to_string())])?;
            }
        }
    }
    writer = writer.chunk(SMET, meta.into_bytes(), Codec::Store);
    for (name, text) in &texts {
        writer = writer.named(SCNE, name.clone(), text.as_bytes(), Codec::Auto);
    }
    for (name, bytes) in &files {
        writer = writer.named(FILE, name.clone(), bytes.as_ref(), Codec::Auto);
    }
    writer
        .to_vec()
        .map_err(|error| vec![refusal(&entry, error.to_string())])
}

fn gltf_files(gltf: &Path, bytes: &[u8]) -> Vec<PathBuf> {
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Vec::new();
    };
    let base = folder(gltf);
    ["buffers", "images"]
        .iter()
        .filter_map(|key| json.get(key)?.as_array())
        .flatten()
        .filter_map(|item| item.get("uri")?.as_str())
        .filter(|uri| !uri.starts_with("data:"))
        .filter_map(|uri| join(&base, uri).ok())
        .map(PathBuf::from)
        .collect()
}

fn linked(dir: &Path, name: &Path) -> io::Result<()> {
    let mut at = dir.to_path_buf();
    for part in name.components() {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(invalid(format!(
                    "{} is a link; unpack does not write through links",
                    at.display()
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn unpack(bytes: &[u8], dir: &Path) -> io::Result<Vec<PathBuf>> {
    let reader = Reader::from_bytes(bytes).map_err(container)?;
    let mut seen = BTreeSet::new();
    let mut chosen = Vec::new();
    for (index, chunk) in reader.entries().iter().enumerate() {
        if chunk.tag != SCNE && chunk.tag != FILE {
            continue;
        }
        let name =
            inner_name(&chunk.name).map_err(|message| invalid(format!("the file {message}")))?;
        if !seen.insert(name.clone()) {
            return Err(invalid(format!("the file holds {} twice", chunk.name)));
        }
        linked(dir, &name)?;
        chosen.push((index, name));
    }
    let mut written = Vec::with_capacity(chosen.len());
    for (index, name) in chosen {
        let raw = reader.read(index).map_err(container)?;
        let target = dir.join(&name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, &raw)?;
        written.push(target);
    }
    Ok(written)
}

pub(crate) fn meta(entry: &Path, scene: &str) -> String {
    format!(
        "format = {FORMAT}\nentry = {}\nscene = {}\n",
        toml::Value::String(slashed(entry)),
        toml::Value::String(scene.to_string())
    )
}

pub(crate) fn scene_hash(texts: &[(String, &str)]) -> String {
    let mut message = Vec::new();
    for (name, text) in texts {
        message.extend_from_slice(name.as_bytes());
        message.push(0);
        message.extend_from_slice(text.as_bytes());
    }
    pfx_file::sha256::hex(&pfx_file::sha256::sha256(&message))
}
