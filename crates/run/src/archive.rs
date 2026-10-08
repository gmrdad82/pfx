use chrono::{DateTime, FixedOffset, Local};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Archived,
    Queued,
    Conflict,
    Refused,
    Skipped,
}

#[derive(Debug)]
enum Fail {
    Conflict(String),
    Retry(String),
}

impl Fail {
    fn message(&self) -> &str {
        match self {
            Self::Conflict(message) | Self::Retry(message) => message,
        }
    }
}

impl From<String> for Fail {
    fn from(message: String) -> Self {
        Self::Retry(message)
    }
}

impl From<&str> for Fail {
    fn from(message: &str) -> Self {
        Self::Retry(message.into())
    }
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            Self::Archived => "archived",
            Self::Queued => "queued",
            Self::Conflict => "conflict",
            Self::Refused => "refused",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Pending {
    manifest: PathBuf,
    folder: PathBuf,
    root: PathBuf,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
struct CatalogFile {
    name: String,
    bytes: u64,
    sha256: String,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
struct Catalog {
    v: u8,
    tool: String,
    product: String,
    shot: String,
    version: String,
    caller: String,
    made: String,
    path: String,
    files: Vec<CatalogFile>,
}

pub fn local_made() -> String {
    Local::now().to_rfc3339()
}

fn hash_io(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn hash(path: &Path) -> Result<String, String> {
    hash_io(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn safe(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

const BUILT_ROOT: Option<&str> = option_env!("PFX_ARCHIVE_ROOT");

fn root_online(root: &Path) -> bool {
    if root.as_os_str().is_empty() || !root.exists() {
        return false;
    }
    let Some(built) = BUILT_ROOT.filter(|built| root == Path::new(built)) else {
        return true;
    };
    if !cfg!(target_os = "linux") {
        return true;
    }
    fs::read_to_string("/proc/self/mountinfo").is_ok_and(|text| {
        text.lines()
            .any(|line| line.split_whitespace().nth(4) == Some(built))
    })
}

pub fn folder(manifest: &Value) -> Result<PathBuf, String> {
    let root = archive_root();
    folder_at(manifest, &root)
}

fn root_in(env: &impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    env("PITO_ARCHIVE")
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
        .or_else(|| BUILT_ROOT.map(PathBuf::from))
}

fn archive_root() -> PathBuf {
    root_in(&|key| std::env::var_os(key)).unwrap_or_default()
}

const TOOL: &str = "pfx";

fn masters(root: &Path) -> PathBuf {
    root.join("Masters")
}

fn catalog_file(root: &Path) -> PathBuf {
    masters(root).join("catalog.jsonl")
}

pub const SAMPLES: &[&str] = &["checker", "sample"];

pub fn is_tag(version: &str) -> bool {
    version.strip_prefix('v').is_some_and(|numbers| {
        let parts: Vec<&str> = numbers.split('.').collect();
        parts.len() == 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
    })
}

pub fn pinned(manifest: &Value) -> bool {
    manifest["pinned"]
        .as_bool()
        .unwrap_or_else(|| manifest["version"].as_str().is_some_and(is_tag))
}

fn has_master(manifest: &Value) -> bool {
    manifest["files"].as_array().is_some_and(|files| {
        files.iter().any(|item| {
            item["file"].as_str().is_some_and(|name| {
                name.ends_with(".master.mp4")
                    || name.ends_with(".master.png")
                    || name.ends_with(".alpha.master.mov")
            })
        })
    })
}

fn under_tmp(root: &Path) -> bool {
    root.components().any(|part| part.as_os_str() == "tmp")
}

pub fn skip_reason(manifest: &Value, no_archive: bool) -> Option<&'static str> {
    skip_reason_in(manifest, no_archive, |key| std::env::var_os(key))
}

fn run_reason(env: &impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<&'static str> {
    if env("PFX_DIRECT").is_some_and(|value| value == "1")
        || env("PITO_ARCHIVE").is_some_and(|root| under_tmp(Path::new(&root)))
    {
        return Some("test");
    }
    if env("RIDE_APP").is_some_and(|value| !value.is_empty()) {
        return Some("ride");
    }
    None
}

fn skip_reason_in(
    manifest: &Value,
    no_archive: bool,
    env: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Option<&'static str> {
    if no_archive {
        return Some("no-archive");
    }
    if let Some(reason) = run_reason(&env) {
        return Some(reason);
    }
    if root_in(&env).is_none() {
        return Some("no-root");
    }
    if manifest["product"]
        .as_str()
        .is_some_and(|product| SAMPLES.contains(&product))
    {
        return Some("sample");
    }
    if !has_master(manifest) {
        return Some("frames-only");
    }
    if !pinned(manifest) {
        return Some("unpinned");
    }
    None
}

pub fn reason(manifest_path: &Path, no_archive: bool) -> Option<&'static str> {
    let manifest: Value = fs::read(manifest_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null);
    skip_reason(&manifest, no_archive)
}

fn caller(value: Option<&str>) -> String {
    let mut clean = String::new();
    for character in value.unwrap_or("").chars() {
        let lower = character.to_ascii_lowercase();
        if lower.is_ascii_lowercase() || lower.is_ascii_digit() {
            clean.push(lower);
        } else if !clean.is_empty() && !clean.ends_with('-') {
            clean.push('-');
        }
    }
    let clean = clean.trim_end_matches('-');
    if clean.is_empty() {
        "manual".into()
    } else {
        clean.into()
    }
}

#[cfg(not(test))]
fn invoker() -> Option<String> {
    std::env::var("PITO_INVOKER").ok()
}

#[cfg(test)]
fn invoker() -> Option<String> {
    None
}

fn folder_at(manifest: &Value, root: &Path) -> Result<PathBuf, String> {
    folder_with_caller(manifest, root, invoker().as_deref())
}

fn folder_with_caller(
    manifest: &Value,
    root: &Path,
    invoker: Option<&str>,
) -> Result<PathBuf, String> {
    let product = manifest["product"]
        .as_str()
        .ok_or("archive: missing product")?;
    let kind = manifest["kind"].as_str().ok_or("archive: missing kind")?;
    let shot = manifest["shot"].as_str().unwrap_or(kind);
    let version = manifest["version"]
        .as_str()
        .ok_or("archive: missing version")?;
    let made = manifest["made"].as_str().ok_or("archive: missing made")?;
    leaf_under(root, product, shot, version, made, &caller(invoker))
}

fn leaf_under(
    root: &Path,
    product: &str,
    shot: &str,
    version: &str,
    made: &str,
    caller: &str,
) -> Result<PathBuf, String> {
    for (what, part) in [
        ("product", product),
        ("shot", shot),
        ("version", version),
        ("caller", caller),
    ] {
        if !safe(part) {
            return Err(format!("the {what} {part:?} is not a safe folder name"));
        }
    }
    let date = DateTime::parse_from_rfc3339(made).map_err(|e| format!("archive made: {e}"))?;
    Ok(masters(root)
        .join(TOOL)
        .join(product)
        .join(shot)
        .join(format!(
            "{}_{}_{}_{}",
            date.format("%Y-%m-%d"),
            date.format("%H%M%S"),
            version,
            caller
        )))
}

fn selected(name: &str, still: bool) -> bool {
    name.ends_with(".master.mp4")
        || name.ends_with(".alpha.master.mov")
        || name.ends_with(".master.png")
        || name.ends_with(".poster.png")
        || name.contains(".cutouts/")
        || (still && !name.contains('/') && name.ends_with(".png"))
}

fn files(manifest: &Value) -> Result<Vec<(PathBuf, String)>, String> {
    let mut out = Vec::new();
    let listed = manifest["files"]
        .as_array()
        .ok_or("archive: missing files")?;
    let still = listed.iter().any(|item| {
        item["file"]
            .as_str()
            .is_some_and(|name| !name.contains('/') && name.ends_with(".master.png"))
    });
    for item in listed {
        let name = item["file"].as_str().ok_or("archive: file has no name")?;
        if !selected(name, still) {
            continue;
        }
        let path = Path::new(name);
        if path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(format!("archive: unsafe file {name}"));
        }
        let checksum = item["sha256"]
            .as_str()
            .ok_or("archive: file has no sha256")?;
        out.push((path.to_path_buf(), checksum.to_owned()));
    }
    Ok(out)
}

fn write_manifest(path: &Path, manifest: &Value) -> Result<(), String> {
    fs::write(
        path,
        serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("{}: {e}", path.display()))
}

fn leaf_name(root: &Path, folder: &Path) -> String {
    folder
        .strip_prefix(masters(root))
        .unwrap_or(folder)
        .to_string_lossy()
        .into_owned()
}

fn catalog_of(manifest: &Value) -> Result<Option<Catalog>, String> {
    match &manifest["archive"]["catalog"] {
        Value::Null => Ok(None),
        Value::String(line) => serde_json::from_str(line)
            .map(Some)
            .map_err(|e| format!("archive catalog: {e}")),
        object => serde_json::from_value(object.clone())
            .map(Some)
            .map_err(|e| format!("archive catalog: {e}")),
    }
}

fn set_failure(manifest: &mut Value, root: &Path, folder: &Path, state: State, error: &str) {
    set_state(manifest, root, folder, state);
    manifest["archive"]["error"] = json!(error);
}

fn set_state(manifest: &mut Value, root: &Path, folder: &Path, state: State) {
    let catalog = match catalog_of(manifest) {
        Ok(Some(catalog)) => serde_json::to_value(catalog).unwrap_or(Value::Null),
        _ => manifest["archive"]["catalog"].clone(),
    };
    manifest["archive"] =
        json!({"leaf": leaf_name(root, folder), "state": state.name(), "catalog": catalog});
}

fn instant(value: &Value) -> Result<DateTime<FixedOffset>, String> {
    let text = value.as_str().ok_or("archive: missing made")?;
    DateTime::parse_from_rfc3339(text).map_err(|e| format!("archive made: {e}"))
}

fn numbered(base: &Path, number: usize) -> Result<PathBuf, String> {
    if number < 2 {
        return Ok(base.to_path_buf());
    }
    Ok(base.with_file_name(format!(
        "{}-{number}",
        base.file_name()
            .ok_or("archive: invalid leaf")?
            .to_string_lossy()
    )))
}

fn same_render(leaf: &Path, manifest: &Value) -> Result<bool, Fail> {
    let at = |why: &dyn std::fmt::Display| format!("{}: {why}", leaf.display());
    let conflict = |why: &dyn std::fmt::Display| {
        Fail::Conflict(format!("archive conflict at {}: {why}", leaf.display()))
    };
    let bytes = match fs::read(leaf.join("manifest.json")) {
        Ok(bytes) => bytes,
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            return Ok(false);
        }
        Err(e) => return Err(Fail::Retry(at(&e))),
    };
    let old: Value = serde_json::from_slice(&bytes).map_err(|e| conflict(&e))?;
    let theirs = instant(&old["made"]).map_err(|e| conflict(&e))?;
    if theirs != instant(&manifest["made"]).map_err(|e| conflict(&e))? {
        return Ok(false);
    }
    let ours = files(manifest)?;
    for (name, _) in &ours {
        match fs::metadata(leaf.join(name)) {
            Ok(meta) if !meta.is_file() => return Ok(false),
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(Fail::Retry(at(&e))),
        }
    }
    for (name, checksum) in &ours {
        match hash_io(&leaf.join(name)) {
            Ok(found) if found == *checksum => {}
            Ok(_) => {
                return Err(conflict(&format!(
                    "{} differs from this render's manifest",
                    name.display()
                )));
            }
            Err(e) => return Err(Fail::Retry(at(&e))),
        }
    }
    Ok(true)
}

fn resolve_leaf(base: &Path, manifest: &Value) -> Result<(PathBuf, bool), Fail> {
    let mut number = 1;
    loop {
        let candidate = numbered(base, number)?;
        if !candidate
            .try_exists()
            .map_err(|e| format!("{}: {e}", candidate.display()))?
        {
            return Ok((candidate, false));
        }
        if same_render(&candidate, manifest)? {
            return Ok((candidate, true));
        }
        number += 1;
    }
}

fn held_or_conflict(destination: &Path, checksum: &str) -> Result<bool, Fail> {
    if hash(destination)? == checksum {
        Ok(false)
    } else {
        Err(Fail::Conflict(format!(
            "archive conflict at {}",
            destination.display()
        )))
    }
}

fn put(source: &Path, destination: &Path, checksum: &str) -> Result<bool, Fail> {
    if destination.exists() {
        return held_or_conflict(destination, checksum);
    }
    let parent = destination.parent().ok_or("archive: no parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let name = destination
        .file_name()
        .ok_or("archive: no file name")?
        .to_string_lossy();
    let partial = parent.join(format!(".{name}.partial"));
    if partial.exists() {
        fs::remove_file(&partial).map_err(|e| e.to_string())?;
    }
    fs::copy(source, &partial).map_err(|e| format!("{}: {e}", partial.display()))?;
    if hash(&partial)? != checksum {
        fs::remove_file(&partial).ok();
        return Err(format!("archive checksum mismatch for {}", source.display()).into());
    }
    place(&partial, destination, checksum)
}

fn link_unsupported(error: &std::io::Error) -> bool {
    if error.kind() == ErrorKind::Unsupported {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        matches!(error.raw_os_error(), Some(libc::ENOSYS | libc::EOPNOTSUPP))
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

fn place(partial: &Path, destination: &Path, checksum: &str) -> Result<bool, Fail> {
    match fs::hard_link(partial, destination) {
        Ok(()) => {
            fs::remove_file(partial).map_err(|e| format!("{}: {e}", partial.display()))?;
            return Ok(true);
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            fs::remove_file(partial).ok();
            return held_or_conflict(destination, checksum);
        }
        Err(error) if link_unsupported(&error) => {}
        Err(error) => {
            fs::remove_file(partial).ok();
            return Err(format!("{}: {error}", destination.display()).into());
        }
    }
    if destination.exists() {
        fs::remove_file(partial).ok();
        return held_or_conflict(destination, checksum);
    }
    fs::rename(partial, destination).map_err(|e| format!("{}: {e}", destination.display()))?;
    Ok(true)
}

fn copy_files(source: &Path, destination: &Path, manifest: &Value) -> Result<(), Fail> {
    for (name, checksum) in files(manifest)? {
        let from = source.join(&name);
        if hash(&from)? != checksum {
            return Err(format!("archive source checksum mismatch for {}", from.display()).into());
        }
        put(&from, &destination.join(&name), &checksum)?;
    }
    Ok(())
}

fn catalog_for(
    manifest: &Value,
    root: &Path,
    folder: &Path,
    source: &Path,
) -> Result<Catalog, String> {
    let mut entries = Vec::new();
    for (name, sha256) in files(manifest)? {
        let from = source.join(&name);
        if hash(&from)? != sha256 {
            return Err(format!(
                "archive source checksum mismatch for {}",
                from.display()
            ));
        }
        entries.push(CatalogFile {
            name: name.to_string_lossy().into_owned(),
            bytes: fs::metadata(&from).map_err(|e| e.to_string())?.len(),
            sha256,
        });
    }
    let path = folder
        .strip_prefix(masters(root))
        .map_err(|e| e.to_string())?;
    let parts: Vec<_> = path.components().collect();
    Ok(Catalog {
        v: 1,
        tool: TOOL.into(),
        product: manifest["product"]
            .as_str()
            .ok_or("archive: missing product")?
            .into(),
        shot: parts
            .get(2)
            .ok_or("archive: missing shot")?
            .as_os_str()
            .to_string_lossy()
            .into_owned(),
        version: manifest["version"]
            .as_str()
            .ok_or("archive: missing version")?
            .into(),
        caller: parts
            .get(3)
            .and_then(|_| folder.file_name())
            .and_then(|n| n.to_str())
            .and_then(|n| n.rsplit('_').next())
            .ok_or("archive: missing caller")?
            .into(),
        made: manifest["made"]
            .as_str()
            .ok_or("archive: missing made")?
            .into(),
        path: path.to_string_lossy().into_owned(),
        files: entries,
    })
}

fn lock_path() -> Result<PathBuf, String> {
    let runtime = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => {
            use std::os::unix::fs::MetadataExt;
            let uid = fs::metadata("/proc/self")
                .map_err(|e| format!("archive: no XDG_RUNTIME_DIR and no /proc/self: {e}"))?
                .uid();
            PathBuf::from(format!("/run/user/{uid}"))
        }
    };
    Ok(runtime.join("pito-archive/catalog.lock"))
}

fn with_lock<T>(path: &Path, action: impl FnOnce() -> Result<T, Fail>) -> Result<T, Fail> {
    fs::create_dir_all(path.parent().ok_or("archive: lock has no parent")?)
        .map_err(|e| e.to_string())?;
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.lock_exclusive().map_err(|e| e.to_string())?;
    let result = action();
    FileExt::unlock(&file).map_err(|e| e.to_string())?;
    result
}

fn append_catalog_unlocked(root: &Path, catalog: &Catalog) -> Result<(), String> {
    let mut line = serde_json::to_vec(catalog).map_err(|e| e.to_string())?;
    line.push(b'\n');
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(catalog_file(root))
        .map_err(|e| e.to_string())?;
    let written = file.write(&line).map_err(|e| e.to_string())?;
    if written != line.len() {
        return Err("archive: short catalog write".into());
    }
    Ok(())
}

fn catalog_present(root: &Path, catalog: &Catalog) -> Result<bool, String> {
    let text = match fs::read_to_string(catalog_file(root)) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    let line = serde_json::to_string(catalog).map_err(|e| e.to_string())?;
    Ok(text.lines().any(|item| item == line))
}

fn partial_folder(folder: &Path) -> Result<PathBuf, String> {
    Ok(folder.with_file_name(format!(
        "{}.partial",
        folder
            .file_name()
            .ok_or("archive: invalid leaf")?
            .to_string_lossy()
    )))
}

fn stage_leaf(source: &Path, folder: &Path, manifest: &Value) -> Result<PathBuf, Fail> {
    let partial = partial_folder(folder)?;
    if partial.exists() {
        fs::remove_dir_all(&partial).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    let result = (|| -> Result<PathBuf, Fail> {
        copy_files(source, &partial, manifest)?;
        let manifest_bytes = serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?;
        fs::write(partial.join("manifest.json"), &manifest_bytes).map_err(|e| e.to_string())?;
        let manifest_sha = hex::encode(Sha256::digest(&manifest_bytes));
        if hash(&partial.join("manifest.json"))? != manifest_sha {
            return Err("archive manifest checksum mismatch".into());
        }
        let saved: Value = serde_json::from_slice(
            &fs::read(partial.join("manifest.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if saved != *manifest {
            return Err("archive manifest verification failed".into());
        }
        Ok(partial.clone())
    })();
    if result.is_err() && partial.exists() {
        fs::remove_dir_all(partial).map_err(|e| e.to_string())?;
    }
    result
}

fn taken(folder: &Path) -> Fail {
    Fail::Retry(format!(
        "{} was taken by another render meanwhile",
        folder.display()
    ))
}

fn finalize_leaf(partial: &Path, folder: &Path, manifest: &Value) -> Result<bool, Fail> {
    if folder
        .try_exists()
        .map_err(|e| format!("{}: {e}", folder.display()))?
    {
        return if same_render(folder, manifest)? {
            Ok(false)
        } else {
            Err(taken(folder))
        };
    }
    fs::rename(partial, folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    Ok(true)
}

fn publish_leaf(source: &Path, folder: &Path, manifest: &Value) -> Result<bool, Fail> {
    if folder
        .try_exists()
        .map_err(|e| format!("{}: {e}", folder.display()))?
    {
        return if same_render(folder, manifest)? {
            Ok(false)
        } else {
            Err(taken(folder))
        };
    }
    let partial = stage_leaf(source, folder, manifest)?;
    let result = finalize_leaf(&partial, folder, manifest);
    if partial.exists() {
        fs::remove_dir_all(&partial).map_err(|e| e.to_string())?;
    }
    result
}

fn publish(
    entry: &Path,
    base: &Path,
    root: &Path,
    manifest: &Value,
) -> Result<(Value, bool), Fail> {
    let mut catalog = catalog_of(manifest)?.ok_or("archive: missing catalog")?;
    let (leaf, held) = resolve_leaf(base, manifest)?;
    catalog.path = leaf_name(root, &leaf);
    let mut staged = manifest.clone();
    staged["archive"]["catalog"] = serde_json::to_value(&catalog).map_err(|e| e.to_string())?;
    set_state(&mut staged, root, &leaf, State::Archived);
    let added = if held {
        false
    } else {
        publish_leaf(entry, &leaf, &staged)?
    };
    if added || !catalog_present(root, &catalog)? {
        append_catalog_unlocked(root, &catalog)?;
    }
    Ok((staged, added))
}

fn clone_or_copy(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let source = fs::File::open(from)?;
        let target = fs::File::create(to)?;
        if rustix::fs::ioctl_ficlone(&target, &source).is_ok() {
            return Ok(());
        }
    }
    fs::copy(from, to).map(|_| ())
}

fn queue_path(manifest: &Path, folder: &Path, cache: &Path) -> PathBuf {
    let key = hex::encode(Sha256::digest(
        format!("{}:{}", manifest.display(), folder.display()).as_bytes(),
    ));
    cache.join("archive-queue").join(&key[..24])
}

fn queue(
    manifest_path: &Path,
    folder: &Path,
    root: &Path,
    manifest: &Value,
    cache: &Path,
) -> Result<(), String> {
    let directory = queue_path(manifest_path, folder, cache);
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let source = manifest_path
        .parent()
        .ok_or("archive: manifest has no folder")?;
    for (name, checksum) in files(manifest)? {
        let from = source.join(&name);
        let to = directory.join(&name);
        if to.is_file() && hash(&to)? == checksum {
            continue;
        }
        fs::create_dir_all(to.parent().ok_or("archive: no parent")?).map_err(|e| e.to_string())?;
        if to.exists() {
            fs::remove_file(&to).map_err(|e| e.to_string())?;
        }
        clone_or_copy(&from, &to).map_err(|e| format!("{}: {e}", to.display()))?;
        if hash(&to)? != checksum {
            return Err(format!(
                "archive queue checksum mismatch for {}",
                to.display()
            ));
        }
    }
    write_manifest(&directory.join("manifest.json"), manifest)?;
    let pending = Pending {
        manifest: manifest_path.to_path_buf(),
        folder: folder.to_path_buf(),
        root: root.to_path_buf(),
    };
    fs::write(
        directory.join("pending.json"),
        serde_json::to_vec(&pending).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

pub fn archive(manifest_path: &Path, no_archive: bool) -> State {
    let root = archive_root();
    if let Some(reason) = reason(manifest_path, no_archive) {
        record_skip(manifest_path, &root, reason);
        return State::Skipped;
    }
    let cache = match crate::ffmpeg::cache() {
        Ok(cache) => cache,
        Err(message) => {
            eprintln!("pfx: archive cache unavailable: {message}");
            return State::Queued;
        }
    };
    let lock = match lock_path() {
        Ok(lock) => lock,
        Err(message) => {
            eprintln!("pfx: archive lock unavailable: {message}");
            return State::Queued;
        }
    };
    archive_in(manifest_path, no_archive, &root, &cache, &lock)
}

fn record_skip(manifest_path: &Path, root: &Path, reason: &str) {
    let attempt = || -> Result<(), String> {
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(manifest_path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let folder = folder_at(&manifest, root)
            .ok()
            .filter(|_| !root.as_os_str().is_empty());
        manifest["archive"] = match folder {
            Some(folder) => {
                json!({"leaf": leaf_name(root, &folder), "state": State::Skipped.name(), "reason": reason})
            }
            None => json!({"state": State::Skipped.name(), "reason": reason}),
        };
        write_manifest(manifest_path, &manifest)
    };
    if let Err(message) = attempt() {
        eprintln!("pfx: could not record skipped archive: {message}");
    }
}

fn archive_in(
    manifest_path: &Path,
    no_archive: bool,
    root: &Path,
    cache: &Path,
    lock: &Path,
) -> State {
    if no_archive {
        record_skip(manifest_path, root, "no-archive");
        return State::Skipped;
    }
    let attempt = || -> Result<State, String> {
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(manifest_path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if manifest["made"].is_null() {
            manifest["made"] = json!(local_made());
        }
        let folder = match folder_at(&manifest, root) {
            Ok(folder) => folder,
            Err(error) => {
                manifest["archive"] = json!({"state": State::Refused.name(), "error": error});
                write_manifest(manifest_path, &manifest)?;
                eprintln!("pfx: archive refused: {error}");
                return Ok(State::Refused);
            }
        };
        let source = manifest_path
            .parent()
            .ok_or("archive: manifest has no folder")?;
        let catalog = catalog_for(&manifest, root, &folder, source)?;
        manifest["archive"]["catalog"] =
            serde_json::to_value(&catalog).map_err(|e| e.to_string())?;
        set_state(&mut manifest, root, &folder, State::Queued);
        write_manifest(manifest_path, &manifest)?;
        queue(manifest_path, &folder, root, &manifest, cache)?;
        if !root_online(root) {
            return Ok(State::Queued);
        }
        let entry = queue_path(manifest_path, &folder, cache);
        match with_lock(lock, || publish(&entry, &folder, root, &manifest)) {
            Ok((archived, added)) => {
                write_manifest(manifest_path, &archived)?;
                fs::remove_dir_all(&entry).map_err(|e| e.to_string())?;
                Ok(if added {
                    State::Archived
                } else {
                    State::Skipped
                })
            }
            Err(Fail::Conflict(message)) => {
                set_failure(&mut manifest, root, &folder, State::Conflict, &message);
                write_manifest(manifest_path, &manifest)?;
                fs::remove_dir_all(&entry).map_err(|e| e.to_string())?;
                eprintln!("pfx: {message}");
                Ok(State::Conflict)
            }
            Err(Fail::Retry(message)) => {
                set_state(&mut manifest, root, &folder, State::Queued);
                write_manifest(manifest_path, &manifest)?;
                eprintln!("pfx: archive queued: {message}");
                Ok(State::Queued)
            }
        }
    };
    match attempt() {
        Ok(state) => state,
        Err(message) => {
            eprintln!("pfx: archive queued, but could not save queue: {message}");
            State::Queued
        }
    }
}

pub fn retry() -> Result<(usize, usize), String> {
    retry_in(&crate::ffmpeg::cache()?, &lock_path()?)
}

pub fn retry_at_startup() -> Result<(usize, usize), String> {
    startup_in(|key| std::env::var_os(key), retry)
}

fn startup_in(
    env: impl Fn(&str) -> Option<std::ffi::OsString>,
    retry: impl FnOnce() -> Result<(usize, usize), String>,
) -> Result<(usize, usize), String> {
    if run_reason(&env).is_some() {
        return Ok((0, 0));
    }
    retry()
}

fn rewrite_local(path: &Path, queued: &Value, manifest: &Value) -> Result<(), String> {
    let Ok(bytes) = fs::read(path) else {
        return Ok(());
    };
    let Ok(local) = serde_json::from_slice::<Value>(&bytes) else {
        return Ok(());
    };
    let (Ok(theirs), Ok(ours)) = (instant(&local["made"]), instant(&queued["made"])) else {
        return Ok(());
    };
    if theirs == ours {
        write_manifest(path, manifest)?;
    }
    Ok(())
}

fn read_json(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn retry_in(cache: &Path, lock: &Path) -> Result<(usize, usize), String> {
    let queue_root = cache.join("archive-queue");
    if !queue_root.exists() {
        return Ok((0, 0));
    }
    let mut completed = 0;
    let mut pending_count = 0;
    for entry in fs::read_dir(&queue_root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.path().is_dir() {
            continue;
        }
        let result = (|| -> Result<(), Fail> {
            let pending: Pending = serde_json::from_slice(
                &fs::read(entry.path().join("pending.json")).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            let manifest = read_json(&entry.path().join("manifest.json"))?;
            if !root_online(&pending.root) {
                return Err(format!("{} is offline", pending.root.display()).into());
            }
            let (archived, _) = with_lock(lock, || {
                publish(&entry.path(), &pending.folder, &pending.root, &manifest)
            })?;
            Ok(rewrite_local(&pending.manifest, &manifest, &archived)?)
        })();
        match result {
            Ok(()) => {
                fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
                completed += 1;
            }
            Err(Fail::Conflict(message)) => {
                eprintln!("pfx: archive retry: {message}");
                let pending: Pending = serde_json::from_slice(
                    &fs::read(entry.path().join("pending.json")).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                let manifest = read_json(&entry.path().join("manifest.json"))?;
                let mut conflicted = manifest.clone();
                set_failure(
                    &mut conflicted,
                    &pending.root,
                    &pending.folder,
                    State::Conflict,
                    &message,
                );
                rewrite_local(&pending.manifest, &manifest, &conflicted)?;
                fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
                completed += 1;
            }
            Err(fail) => {
                eprintln!("pfx: archive retry: {}", fail.message());
                pending_count += 1;
            }
        }
    }
    Ok((completed, pending_count))
}

fn filed(catalog: &mut Catalog, masters: &Path, manifest: &Path) {
    let Some(leaf) = manifest
        .parent()
        .and_then(|leaf| leaf.strip_prefix(masters).ok())
    else {
        return;
    };
    if let Some(tool) = leaf.components().next() {
        catalog.tool = tool.as_os_str().to_string_lossy().into_owned();
    }
    catalog.path = leaf.to_string_lossy().into_owned();
}

fn collect_catalogs(
    masters: &Path,
    path: &Path,
    entries: &mut Vec<(DateTime<FixedOffset>, Catalog)>,
) -> Result<(), String> {
    for item in fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))? {
        let item = item.map_err(|e| e.to_string())?;
        let item_path = item.path();
        if item.file_type().map_err(|e| e.to_string())?.is_dir() {
            if !item.file_name().to_string_lossy().ends_with(".partial") {
                collect_catalogs(masters, &item_path, entries)?;
            }
        } else if item.file_name().to_string_lossy() == "manifest.json" {
            let manifest = read_json(&item_path)?;
            let named = |why: String| format!("{}: {why}", item_path.display());
            if let Some(mut catalog) = catalog_of(&manifest).map_err(&named)? {
                let made = instant(&json!(catalog.made)).map_err(&named)?;
                filed(&mut catalog, masters, &item_path);
                entries.push((made, catalog));
            }
        }
    }
    Ok(())
}

pub fn reindex() -> Result<usize, String> {
    let root = archive_root();
    if root.as_os_str().is_empty() {
        return Err("archive: no root; set PITO_ARCHIVE".into());
    }
    reindex_in(&root, &lock_path()?)
}

fn reindex_in(root: &Path, lock: &Path) -> Result<usize, String> {
    if !root_online(root) {
        return Err(format!("archive: {} is offline", root.display()));
    }
    with_lock(lock, || {
        let masters = masters(root);
        fs::create_dir_all(&masters).map_err(|e| format!("{}: {e}", masters.display()))?;
        let mut entries = Vec::new();
        collect_catalogs(&masters, &masters, &mut entries)?;
        entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.path.cmp(&b.1.path)));
        let partial = masters.join("catalog.jsonl.partial");
        let mut file = fs::File::create(&partial).map_err(|e| e.to_string())?;
        for (_, entry) in &entries {
            serde_json::to_writer(&mut file, entry).map_err(|e| e.to_string())?;
            file.write_all(b"\n").map_err(|e| e.to_string())?;
        }
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&partial, catalog_file(root)).map_err(|e| e.to_string())?;
        Ok(entries.len())
    })
    .map_err(|fail| fail.message().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reason_with(
        manifest: &Value,
        no_archive: bool,
        pairs: &[(&str, &str)],
    ) -> Option<&'static str> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        skip_reason_in(manifest, no_archive, move |key| {
            owned
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| std::ffi::OsString::from(value))
        })
    }

    fn pinned_master(product: &str) -> Value {
        json!({
            "product": product,
            "version": "aabbccddeeff",
            "pinned": true,
            "files": [{"file": "alpha-clip-a-1280x720-60fps.master.mp4", "sha256": "00"}],
        })
    }

    #[test]
    fn only_a_pinned_master_outside_tests_and_rides_is_archived() {
        let master = pinned_master("alpha");
        let root = ("PITO_ARCHIVE", "/srv/archive");
        assert_eq!(reason_with(&master, false, &[root]), None);
        assert_eq!(
            reason_with(&master, false, &[]),
            BUILT_ROOT.map_or(Some("no-root"), |_| None)
        );
        assert_eq!(reason_with(&master, true, &[root]), Some("no-archive"));
        assert_eq!(
            reason_with(&master, false, &[("PFX_DIRECT", "1"), root]),
            Some("test")
        );
        assert_eq!(
            reason_with(&master, false, &[("PITO_ARCHIVE", "/work/app/tmp/archive")]),
            Some("test")
        );
        assert_eq!(
            reason_with(&master, false, &[("RIDE_APP", "beta"), root]),
            Some("ride")
        );
        assert_eq!(reason_with(&master, false, &[("RIDE_APP", ""), root]), None);
        assert_eq!(
            reason_with(&pinned_master("checker"), false, &[root]),
            Some("sample")
        );
        assert_eq!(
            reason_with(&pinned_master("sample"), false, &[root]),
            Some("sample")
        );
        let mut unpinned = master.clone();
        unpinned["pinned"] = json!(false);
        assert_eq!(reason_with(&unpinned, false, &[root]), Some("unpinned"));
        let mut frames = master.clone();
        frames["files"] = json!([{"file": "frames/00000.png", "sha256": "00"}]);
        assert_eq!(reason_with(&frames, false, &[root]), Some("frames-only"));
    }

    #[test]
    fn an_older_manifest_is_pinned_only_by_a_tag() {
        assert!(pinned(&json!({"version": "v1.2.3"})));
        assert!(!pinned(&json!({"version": "aabbccddeeff"})));
        assert!(!pinned(&json!({"version": "aabbccddeeff-dirty"})));
        assert!(!pinned(&json!({"version": "vpreview"})));
        assert!(pinned(&json!({"version": "aabbccddeeff", "pinned": true})));
    }

    #[test]
    fn verified_partial_is_linked_without_clobbering_an_existing_file() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("archive-link-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let partial = dir.join(".master.mp4.partial");
        let final_path = dir.join("master.mp4");
        fs::write(&partial, b"first").unwrap();
        assert!(place(&partial, &final_path, &hash(&partial).unwrap()).unwrap());
        assert!(!partial.exists());
        fs::write(&partial, b"second").unwrap();
        assert!(place(&partial, &final_path, &hash(&partial).unwrap()).is_err());
        assert_eq!(fs::read(&final_path).unwrap(), b"first");
        assert!(!partial.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn leaf_is_staged_before_one_rename() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("archive-stage-test-{}", std::process::id()));
        let source = base.join("source");
        let folder = base.join("archive/leaf");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("scene.master.mp4"), b"master").unwrap();
        let manifest = json!({"files": [{"file": "scene.master.mp4", "sha256": hash(&source.join("scene.master.mp4")).unwrap()}]});
        let partial = stage_leaf(&source, &folder, &manifest).unwrap();
        assert!(partial.join("scene.master.mp4").exists());
        assert!(partial.join("manifest.json").exists());
        assert!(!folder.exists());
        assert!(finalize_leaf(&partial, &folder, &manifest).unwrap());
        assert!(!partial.exists());
        assert!(folder.join("manifest.json").exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_still_archives_its_png_deliveries_beside_its_master() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("archive-still-{}", std::process::id()));
        let source = base.join("source");
        let root = base.join("archive");
        let cache = base.join("cache");
        let lock = base.join("runtime/catalog.lock");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&root).unwrap();
        let names = [
            "sample-still-shapes-1920x1080.master.png",
            "sample-still-shapes-1920x1080.png",
            "sample-still-shapes-960x540.png",
            "sample-still-shapes-1920x1080-web-320.png",
            "sample-still-shapes-1920x1080-web-320@2x.png",
            "sample-still-shapes-1920x1080.linear.exr",
        ];
        let mut listed = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let path = source.join(name);
            fs::write(&path, format!("bytes {i}")).unwrap();
            listed.push(json!({"file": name, "sha256": hash(&path).unwrap()}));
        }
        let manifest_path = source.join("sample-still-shapes-1920x1080.json");
        let manifest = json!({
            "product": "neutral", "kind": "still", "shot": "shapes", "version": "v1",
            "made": "2026-10-05T19:00:00+02:00",
            "files": listed,
        });
        write_manifest(&manifest_path, &manifest).unwrap();
        assert_eq!(
            archive_in(&manifest_path, false, &root, &cache, &lock),
            State::Archived
        );
        let leaf = root.join("Masters/pfx/neutral/shapes/2026-10-05_190000_v1_manual");
        for name in &names[..5] {
            assert_eq!(
                hash(&leaf.join(name)).unwrap(),
                hash(&source.join(name)).unwrap(),
                "{name}"
            );
        }
        assert!(!leaf.join(names[5]).exists());
        let catalog: Catalog = serde_json::from_str(
            fs::read_to_string(root.join("Masters/catalog.jsonl"))
                .unwrap()
                .trim(),
        )
        .unwrap();
        let mut caught: Vec<&str> = catalog.files.iter().map(|f| f.name.as_str()).collect();
        caught.sort_unstable();
        let mut wanted = names[..5].to_vec();
        wanted.sort_unstable();
        assert_eq!(caught, wanted);
        let clip = json!({"files": [
            {"file": "c.master.mp4", "sha256": "0"},
            {"file": "c.poster.png", "sha256": "0"},
            {"file": "c-640.png", "sha256": "0"},
            {"file": "c.frames/00000.png", "sha256": "0"}
        ]});
        let kept: Vec<PathBuf> = files(&clip).unwrap().into_iter().map(|(p, _)| p).collect();
        assert_eq!(
            kept,
            [PathBuf::from("c.master.mp4"), PathBuf::from("c.poster.png")]
        );
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn copies_verifies_skips_conflicts_and_retries() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("archive-test-{}", std::process::id()));
        let source = base.join("source");
        let root = base.join("archive");
        let cache = base.join("cache");
        let lock = base.join("runtime/catalog.lock");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&root).unwrap();
        let master = source.join("clip.master.mp4");
        fs::write(&master, b"first master").unwrap();
        let manifest_path = source.join("clip.json");
        let mut manifest = json!({
            "product": "alpha", "kind": "clip", "shot": "checker", "version": "v1",
            "made": "2026-10-02T09:14:00+02:00",
            "files": [{"file": "clip.master.mp4", "sha256": hash(&master).unwrap()}]
        });
        write_manifest(&manifest_path, &manifest).unwrap();
        assert_eq!(
            archive_in(&manifest_path, true, &root, &cache, &lock),
            State::Skipped
        );
        let skipped: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        assert_eq!(skipped["archive"]["state"], "skipped");
        assert!(!root.join("Masters").exists());
        assert_eq!(
            archive_in(&manifest_path, false, &root, &cache, &lock),
            State::Archived
        );
        let target = root.join("Masters/pfx/alpha/checker/2026-10-02_091400_v1_manual");
        assert_eq!(
            hash(&target.join("clip.master.mp4")).unwrap(),
            hash(&master).unwrap()
        );
        assert!(target.join("manifest.json").exists());
        assert!(
            !target
                .with_file_name("2026-10-02_091400_v1_manual.partial")
                .exists()
        );
        let first_catalog = fs::read(root.join("Masters/catalog.jsonl")).unwrap();
        assert_eq!(
            first_catalog.iter().filter(|byte| **byte == b'\n').count(),
            1
        );
        assert_eq!(
            archive_in(&manifest_path, false, &root, &cache, &lock),
            State::Skipped
        );
        assert_eq!(
            fs::read(root.join("Masters/catalog.jsonl")).unwrap(),
            first_catalog
        );
        fs::write(&master, b"different master").unwrap();
        manifest["files"][0]["sha256"] = json!(hash(&master).unwrap());
        manifest["made"] = json!("2026-10-02T09:14:00.500+02:00");
        write_manifest(&manifest_path, &manifest).unwrap();
        assert_eq!(
            archive_in(&manifest_path, false, &root, &cache, &lock),
            State::Archived
        );
        let second = target.with_file_name("2026-10-02_091400_v1_manual-2");
        assert_eq!(
            fs::read(second.join("clip.master.mp4")).unwrap(),
            b"different master"
        );
        assert_eq!(
            fs::read(target.join("clip.master.mp4")).unwrap(),
            b"first master"
        );
        let both = fs::read_to_string(root.join("Masters/catalog.jsonl")).unwrap();
        assert_eq!(both.lines().count(), 2);
        assert!(both.starts_with(&String::from_utf8(first_catalog.clone()).unwrap()));
        let offline = base.join("offline");
        manifest["version"] = json!("v2");
        write_manifest(&manifest_path, &manifest).unwrap();
        assert_eq!(
            archive_in(&manifest_path, false, &offline, &cache, &lock),
            State::Queued
        );
        let queued: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        assert_eq!(queued["archive"]["state"], "queued");
        fs::create_dir_all(&offline).unwrap();
        assert_eq!(retry_in(&cache, &lock).unwrap(), (1, 0));
        let completed: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        assert_eq!(completed["archive"]["state"], "archived");
        assert_eq!(
            fs::read(
                offline
                    .join("Masters/pfx/alpha/checker/2026-10-02_091400_v2_manual/clip.master.mp4")
            )
            .unwrap(),
            b"different master"
        );
        let replay = root.join("Masters/prec/gamma/tour/2026-10-02_1015_v1_gamma");
        fs::create_dir_all(&replay).unwrap();
        let replay_catalog = json!({
            "v": 1, "tool": "prec", "product": "gamma", "shot": "tour",
            "version": "v1", "caller": "gamma", "made": "2026-10-02T10:15:00+02:00",
            "path": "prec/gamma/tour/2026-10-02_1015_v1_gamma",
            "files": [{"name": "tour.master.mp4", "bytes": 3, "sha256": "abc"}]
        });
        write_manifest(
            &replay.join("manifest.json"),
            &json!({"archive": {"catalog": replay_catalog}}),
        )
        .unwrap();
        let replay_line = format!(
            "{}\n",
            serde_json::to_string(&serde_json::from_value::<Catalog>(replay_catalog).unwrap())
                .unwrap()
        );
        fs::write(root.join("Masters/catalog.jsonl"), b"stale\n").unwrap();
        assert_eq!(reindex_in(&root, &lock).unwrap(), 3);
        assert_eq!(
            fs::read_to_string(root.join("Masters/catalog.jsonl")).unwrap(),
            format!("{both}{replay_line}")
        );
        fs::remove_dir_all(base).unwrap();
    }

    struct Fixture {
        base: PathBuf,
        source: PathBuf,
        root: PathBuf,
        cache: PathBuf,
        lock: PathBuf,
        master: PathBuf,
        manifest_path: PathBuf,
        manifest: Value,
    }

    impl Fixture {
        fn new(label: &str) -> Fixture {
            let base = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tmp")
                .join(format!("archive-{label}-{}", std::process::id()));
            let source = base.join("source");
            let root = base.join("archive");
            fs::create_dir_all(&source).unwrap();
            fs::create_dir_all(&root).unwrap();
            let master = source.join("clip.master.mp4");
            fs::write(&master, b"first master").unwrap();
            let manifest_path = source.join("clip.json");
            let manifest = json!({
                "product": "alpha", "kind": "clip", "shot": "checker", "version": "v1",
                "made": "2026-10-02T09:14:00+02:00",
                "files": [{"file": "clip.master.mp4", "sha256": hash(&master).unwrap()}]
            });
            write_manifest(&manifest_path, &manifest).unwrap();
            Fixture {
                cache: base.join("cache"),
                lock: base.join("runtime/catalog.lock"),
                base,
                source,
                root,
                master,
                manifest_path,
                manifest,
            }
        }

        fn leaf(&self) -> PathBuf {
            self.root
                .join("Masters/pfx/alpha/checker/2026-10-02_091400_v1_manual")
        }

        fn entries(&self) -> usize {
            fs::read_dir(self.cache.join("archive-queue"))
                .map(|items| items.count())
                .unwrap_or(0)
        }

        fn manifest_state(&self) -> String {
            let saved: Value =
                serde_json::from_slice(&fs::read(&self.manifest_path).unwrap()).unwrap();
            saved["archive"]["state"].as_str().unwrap().to_owned()
        }

        fn queue_now(&self) -> PathBuf {
            let mut queued = self.manifest.clone();
            let catalog = catalog_for(&queued, &self.root, &self.leaf(), &self.source).unwrap();
            queued["archive"]["catalog"] = serde_json::to_value(&catalog).unwrap();
            set_state(&mut queued, &self.root, &self.leaf(), State::Queued);
            write_manifest(&self.manifest_path, &queued).unwrap();
            queue(
                &self.manifest_path,
                &self.leaf(),
                &self.root,
                &queued,
                &self.cache,
            )
            .unwrap();
            queue_path(&self.manifest_path, &self.leaf(), &self.cache)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.base).ok();
        }
    }

    #[test]
    fn a_successful_archive_leaves_no_queue_entry() {
        let fixture = Fixture::new("queue-clean");
        assert_eq!(
            archive_in(
                &fixture.manifest_path,
                false,
                &fixture.root,
                &fixture.cache,
                &fixture.lock
            ),
            State::Archived
        );
        assert_eq!(fixture.entries(), 0);
        assert_eq!(fixture.manifest_state(), "archived");
        assert_eq!(
            fs::read(fixture.leaf().join("clip.master.mp4")).unwrap(),
            b"first master"
        );
    }

    #[test]
    fn a_conflict_removes_the_queue_entry() {
        let fixture = Fixture::new("queue-conflict");
        let leaf = fixture.leaf();
        fs::create_dir_all(&leaf).unwrap();
        fs::write(leaf.join("clip.master.mp4"), b"another master").unwrap();
        fs::write(leaf.join("manifest.json"), b"{}").unwrap();
        assert_eq!(
            archive_in(
                &fixture.manifest_path,
                false,
                &fixture.root,
                &fixture.cache,
                &fixture.lock
            ),
            State::Conflict
        );
        assert_eq!(fixture.entries(), 0);
        assert_eq!(fixture.manifest_state(), "conflict");
        assert_eq!(
            fs::read(leaf.join("clip.master.mp4")).unwrap(),
            b"another master"
        );
    }

    #[test]
    fn a_copy_interrupted_after_queueing_is_finished_by_retry() {
        let fixture = Fixture::new("queue-killed");
        let entry = fixture.queue_now();
        assert!(entry.join("pending.json").is_file());
        assert!(!fixture.leaf().exists());
        assert_eq!(fixture.manifest_state(), "queued");
        assert_eq!(retry_in(&fixture.cache, &fixture.lock).unwrap(), (1, 0));
        assert_eq!(fixture.entries(), 0);
        assert_eq!(fixture.manifest_state(), "archived");
        assert_eq!(
            fs::read(fixture.leaf().join("clip.master.mp4")).unwrap(),
            b"first master"
        );
        let catalog = fs::read_to_string(fixture.root.join("Masters/catalog.jsonl")).unwrap();
        assert_eq!(catalog.lines().count(), 1);
    }

    #[test]
    fn a_copy_that_fails_stays_queued_until_retry_finishes_it() {
        let fixture = Fixture::new("queue-failed");
        let blocker = partial_folder(&fixture.leaf()).unwrap();
        fs::create_dir_all(blocker.parent().unwrap()).unwrap();
        fs::write(&blocker, b"in the way").unwrap();
        assert_eq!(
            archive_in(
                &fixture.manifest_path,
                false,
                &fixture.root,
                &fixture.cache,
                &fixture.lock
            ),
            State::Queued
        );
        assert_eq!(fixture.entries(), 1);
        assert_eq!(fixture.manifest_state(), "queued");
        fs::remove_file(&blocker).unwrap();
        assert_eq!(retry_in(&fixture.cache, &fixture.lock).unwrap(), (1, 0));
        assert_eq!(fixture.entries(), 0);
        assert!(fixture.leaf().join("manifest.json").is_file());
    }

    #[test]
    fn a_source_rewritten_after_queueing_does_not_change_the_queued_copy() {
        let fixture = Fixture::new("queue-rewrite");
        let entry = fixture.queue_now();
        fs::write(&fixture.master, b"rewritten in place").unwrap();
        assert_eq!(
            fs::read(entry.join("clip.master.mp4")).unwrap(),
            b"first master"
        );
        assert_eq!(retry_in(&fixture.cache, &fixture.lock).unwrap(), (1, 0));
        assert_eq!(
            fs::read(fixture.leaf().join("clip.master.mp4")).unwrap(),
            b"first master"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_queued_copy_is_never_a_hard_link() {
        use std::os::unix::fs::MetadataExt;
        let fixture = Fixture::new("queue-links");
        let entry = fixture.queue_now();
        let queued = fs::metadata(entry.join("clip.master.mp4")).unwrap();
        let original = fs::metadata(&fixture.master).unwrap();
        assert_eq!(queued.nlink(), 1);
        assert_eq!(original.nlink(), 1);
        assert_ne!(queued.ino(), original.ino());
    }

    #[test]
    fn leaf_uses_made_offset_and_sanitized_caller() {
        let root = Path::new("tmp/archive");
        let manifest = json!({"product": "beta", "kind": "still", "shot": "page", "version": "v1.2.3", "made": "2026-10-02T00:30:00+02:00"});
        assert_eq!(
            folder_with_caller(&manifest, root, Some("PITO Invoker!2")).unwrap(),
            root.join("Masters/pfx/beta/page/2026-10-02_003000_v1.2.3_pito-invoker-2")
        );
        assert_eq!(
            folder_with_caller(&manifest, root, None).unwrap(),
            root.join("Masters/pfx/beta/page/2026-10-02_003000_v1.2.3_manual")
        );
    }

    #[test]
    fn concurrent_catalog_appends_are_whole_lines() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("archive-lock-test-{}", std::process::id()));
        fs::create_dir_all(masters(&base)).unwrap();
        let lock = base.join("runtime/catalog.lock");
        let template = Catalog {
            v: 1,
            tool: "pfx".into(),
            product: "alpha".into(),
            shot: "scene".into(),
            version: "v1".into(),
            caller: "manual".into(),
            made: "2026-10-02T09:14:00+02:00".into(),
            path: "a".into(),
            files: vec![],
        };
        std::thread::scope(|scope| {
            for index in 0..2 {
                let mut item = template.clone();
                item.path = format!("leaf-{index}");
                let root = &base;
                let lock = &lock;
                scope.spawn(move || {
                    with_lock(lock, || Ok(append_catalog_unlocked(root, &item)?)).unwrap()
                });
            }
        });
        let lines = fs::read_to_string(base.join("Masters/catalog.jsonl")).unwrap();
        assert_eq!(lines.lines().count(), 2);
        let paths: Vec<String> = lines
            .lines()
            .map(|line| serde_json::from_str::<Catalog>(line).unwrap().path)
            .collect();
        assert!(paths.contains(&"leaf-0".into()));
        assert!(paths.contains(&"leaf-1".into()));
        fs::remove_dir_all(base).unwrap();
    }

    impl Fixture {
        fn archive(&self) -> State {
            archive_in(
                &self.manifest_path,
                false,
                &self.root,
                &self.cache,
                &self.lock,
            )
        }

        fn rerender(&mut self, contents: &[u8], made: &str) {
            fs::write(&self.master, contents).unwrap();
            self.manifest["made"] = json!(made);
            self.manifest["files"][0]["sha256"] = json!(hash(&self.master).unwrap());
            write_manifest(&self.manifest_path, &self.manifest).unwrap();
        }

        fn saved(&self) -> Value {
            serde_json::from_slice(&fs::read(&self.manifest_path).unwrap()).unwrap()
        }

        fn catalog_paths(&self) -> Vec<String> {
            fs::read_to_string(self.root.join("Masters/catalog.jsonl"))
                .unwrap_or_default()
                .lines()
                .map(|line| serde_json::from_str::<Catalog>(line).unwrap().path)
                .collect()
        }
    }

    fn leaf_manifest(catalog: Value, extra: Value) -> Value {
        let mut manifest = json!({"archive": {"catalog": catalog}});
        for (key, value) in extra.as_object().unwrap() {
            manifest["archive"][key] = value.clone();
        }
        manifest
    }

    fn line(tool: &str, made: &str, path: &str) -> Value {
        json!({
            "v": 1, "tool": tool, "product": "beta", "shot": "tour", "version": "v1",
            "caller": "gamma", "made": made, "path": path,
            "files": [{"name": "tour.master.mp4", "bytes": 3, "sha256": "abc"}]
        })
    }

    fn put_leaf(root: &Path, path: &str, manifest: &Value) {
        let leaf = masters(root).join(path);
        fs::create_dir_all(&leaf).unwrap();
        write_manifest(&leaf.join("manifest.json"), manifest).unwrap();
    }

    #[test]
    fn reindex_reads_both_tools_and_both_shapes_sorted_by_instant_then_path() {
        let fixture = Fixture::new("reindex-mix");
        let root = &fixture.root;
        let early = line(
            "prec",
            "2026-10-02T10:00:00+05:00",
            "prec/beta/tour/2026-10-02_1000_v1_gamma",
        );
        let late_a = line(
            "pfx",
            "2026-10-02T09:00:00+00:00",
            "pfx/beta/tour/2026-10-02_090000_v1_gamma",
        );
        let late_b = line(
            "prec",
            "2026-10-02T09:00:00+00:00",
            "prec/beta/tour/2026-10-02_090000_v1_gamma",
        );
        let newest = line(
            "pito-engine",
            "2026-10-03T08:15:00.123456789+02:00",
            "pito-engine/beta/tour/2026-10-03_0815_v1_gamma",
        );
        let string = |value: &Value| json!(serde_json::to_string(value).unwrap());
        put_leaf(
            root,
            "prec/beta/tour/2026-10-02_1000_v1_gamma",
            &leaf_manifest(string(&early), json!({"leaf": "x"})),
        );
        put_leaf(
            root,
            "prec/beta/tour/2026-10-02_090000_v1_gamma",
            &leaf_manifest(string(&late_b), json!({"folder": "x"})),
        );
        put_leaf(
            root,
            "pfx/beta/tour/2026-10-02_090000_v1_gamma",
            &leaf_manifest(late_a.clone(), json!({"leaf": "x"})),
        );
        put_leaf(
            root,
            "pfx/beta/tour/2026-10-03_0815_v1_gamma",
            &leaf_manifest(newest, json!({"folder": "/old/absolute/x"})),
        );
        put_leaf(
            root,
            "pfx/beta/tour/skipped",
            &json!({"archive": {"state": "skipped", "reason": "unpinned"}}),
        );
        fs::create_dir_all(root.join("Masters/pfx/beta/tour/half.partial")).unwrap();
        fs::write(
            root.join("Masters/pfx/beta/tour/half.partial/manifest.json"),
            b"not json",
        )
        .unwrap();
        assert_eq!(reindex_in(root, &fixture.lock).unwrap(), 4);
        let order: Vec<(String, String)> = fs::read_to_string(root.join("Masters/catalog.jsonl"))
            .unwrap()
            .lines()
            .map(|line| {
                let catalog = serde_json::from_str::<Catalog>(line).unwrap();
                (catalog.tool, catalog.path)
            })
            .collect();
        assert_eq!(
            order,
            [
                ("prec", "prec/beta/tour/2026-10-02_1000_v1_gamma"),
                ("pfx", "pfx/beta/tour/2026-10-02_090000_v1_gamma"),
                ("prec", "prec/beta/tour/2026-10-02_090000_v1_gamma"),
                ("pfx", "pfx/beta/tour/2026-10-03_0815_v1_gamma"),
            ]
            .map(|(tool, path)| (tool.to_owned(), path.to_owned()))
        );
        let first = fs::read_to_string(root.join("Masters/catalog.jsonl")).unwrap();
        assert_eq!(reindex_in(root, &fixture.lock).unwrap(), 4);
        assert_eq!(
            fs::read_to_string(root.join("Masters/catalog.jsonl")).unwrap(),
            first
        );
    }

    #[test]
    fn an_unreadable_manifest_fails_the_reindex_by_name() {
        let fixture = Fixture::new("reindex-bad");
        let good = line("pfx", "2026-10-02T09:00:00+00:00", "pfx/beta/tour/ok");
        put_leaf(
            &fixture.root,
            "pfx/beta/tour/ok",
            &leaf_manifest(good, json!({})),
        );
        fs::write(fixture.root.join("Masters/catalog.jsonl"), b"kept\n").unwrap();
        let broken = fixture.root.join("Masters/prec/beta/tour/broken");
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join("manifest.json"), b"{ not json").unwrap();
        let error = reindex_in(&fixture.root, &fixture.lock).unwrap_err();
        assert!(
            error.contains("prec/beta/tour/broken/manifest.json"),
            "{error}"
        );
        assert_eq!(
            fs::read(fixture.root.join("Masters/catalog.jsonl")).unwrap(),
            b"kept\n"
        );
        fs::write(
            broken.join("manifest.json"),
            b"{\"archive\": {\"catalog\": 7}}",
        )
        .unwrap();
        let error = reindex_in(&fixture.root, &fixture.lock).unwrap_err();
        assert!(error.contains("broken/manifest.json"), "{error}");
    }

    #[test]
    fn a_catalog_is_read_as_an_object_or_as_a_string() {
        let entry = line("pfx", "2026-10-02T09:00:00+00:00", "a/b/c/d");
        let object = json!({"archive": {"catalog": entry.clone()}});
        let string = json!({"archive": {"catalog": serde_json::to_string(&entry).unwrap()}});
        assert_eq!(
            catalog_of(&object).unwrap().unwrap().path,
            catalog_of(&string).unwrap().unwrap().path
        );
        assert!(catalog_of(&json!({})).unwrap().is_none());
    }

    #[test]
    fn two_renders_in_the_same_second_get_their_own_leaves() {
        let mut fixture = Fixture::new("same-second");
        fixture.rerender(b"first master", "2026-10-02T09:14:00.100+02:00");
        assert_eq!(fixture.archive(), State::Archived);
        fixture.rerender(b"second master", "2026-10-02T09:14:00.900+02:00");
        assert_eq!(fixture.archive(), State::Archived);
        fixture.rerender(b"third master", "2026-10-02T09:14:00.950+02:00");
        assert_eq!(fixture.archive(), State::Archived);
        let base = "pfx/alpha/checker/2026-10-02_091400_v1_manual";
        assert_eq!(
            fixture.catalog_paths(),
            [base.to_string(), format!("{base}-2"), format!("{base}-3")]
        );
        for (suffix, contents) in [
            ("", &b"first master"[..]),
            ("-2", b"second master"),
            ("-3", b"third master"),
        ] {
            let leaf = masters(&fixture.root).join(format!("{base}{suffix}"));
            assert_eq!(fs::read(leaf.join("clip.master.mp4")).unwrap(), contents);
        }
        let saved = fixture.saved();
        assert_eq!(saved["archive"]["leaf"], format!("{base}-3"));
        assert_eq!(saved["archive"]["catalog"]["path"], format!("{base}-3"));
        assert_eq!(saved["archive"]["catalog"]["caller"], "manual");
        assert_eq!(saved["archive"]["state"], "archived");
        assert!(saved["archive"]["folder"].is_null());
        let leaf_manifest: Value = serde_json::from_slice(
            &fs::read(masters(&fixture.root).join(format!("{base}-3/manifest.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(leaf_manifest, saved);
        assert_eq!(fixture.entries(), 0);
    }

    #[test]
    fn a_free_gap_is_taken_before_a_higher_suffix() {
        let mut fixture = Fixture::new("gap");
        fixture.rerender(b"first master", "2026-10-02T09:14:00.100+02:00");
        assert_eq!(fixture.archive(), State::Archived);
        fixture.rerender(b"second master", "2026-10-02T09:14:00.200+02:00");
        assert_eq!(fixture.archive(), State::Archived);
        fixture.rerender(b"third master", "2026-10-02T09:14:00.300+02:00");
        assert_eq!(fixture.archive(), State::Archived);
        let base = fixture.leaf();
        fs::remove_dir_all(base.with_file_name("2026-10-02_091400_v1_manual-2")).unwrap();
        fixture.rerender(b"fourth master", "2026-10-02T09:14:00.400+02:00");
        assert_eq!(fixture.archive(), State::Archived);
        assert!(
            fixture.saved()["archive"]["leaf"]
                .as_str()
                .unwrap()
                .ends_with("_manual-2")
        );
    }

    #[test]
    fn a_retry_of_an_archived_render_leaves_it_alone() {
        let fixture = Fixture::new("retry-archived");
        assert_eq!(fixture.archive(), State::Archived);
        let catalog = fs::read(fixture.root.join("Masters/catalog.jsonl")).unwrap();
        let leaf_manifest = fs::read(fixture.leaf().join("manifest.json")).unwrap();
        fixture.queue_now();
        assert_eq!(retry_in(&fixture.cache, &fixture.lock).unwrap(), (1, 0));
        assert_eq!(fixture.entries(), 0);
        assert_eq!(
            fs::read(fixture.root.join("Masters/catalog.jsonl")).unwrap(),
            catalog
        );
        assert_eq!(
            fs::read(fixture.leaf().join("manifest.json")).unwrap(),
            leaf_manifest
        );
        assert!(
            !fixture
                .leaf()
                .with_file_name("2026-10-02_091400_v1_manual-2")
                .exists()
        );
        assert_eq!(fixture.manifest_state(), "archived");
    }

    #[test]
    fn a_retry_of_a_render_missing_from_the_catalog_adds_its_line_once() {
        let fixture = Fixture::new("retry-catalog");
        assert_eq!(fixture.archive(), State::Archived);
        fs::remove_file(fixture.root.join("Masters/catalog.jsonl")).unwrap();
        fixture.queue_now();
        assert_eq!(retry_in(&fixture.cache, &fixture.lock).unwrap(), (1, 0));
        assert_eq!(fixture.catalog_paths().len(), 1);
    }

    #[test]
    fn a_retry_never_rewrites_a_newer_local_manifest() {
        let mut fixture = Fixture::new("retry-newer");
        fixture.queue_now();
        fixture.rerender(b"newer master", "2026-10-02T09:20:00+02:00");
        let newer = fs::read(&fixture.manifest_path).unwrap();
        assert_eq!(retry_in(&fixture.cache, &fixture.lock).unwrap(), (1, 0));
        assert_eq!(fs::read(&fixture.manifest_path).unwrap(), newer);
        assert_eq!(fixture.entries(), 0);
        assert_eq!(
            fs::read(fixture.leaf().join("clip.master.mp4")).unwrap(),
            b"first master"
        );
        assert_eq!(fixture.catalog_paths().len(), 1);
    }

    #[test]
    fn a_retry_rewrites_the_local_manifest_of_its_own_render() {
        let fixture = Fixture::new("retry-own");
        fixture.queue_now();
        assert_eq!(fixture.manifest_state(), "queued");
        assert_eq!(retry_in(&fixture.cache, &fixture.lock).unwrap(), (1, 0));
        assert_eq!(fixture.manifest_state(), "archived");
        assert_eq!(
            fixture.saved()["archive"]["leaf"],
            "pfx/alpha/checker/2026-10-02_091400_v1_manual"
        );
    }

    #[test]
    fn a_test_or_ride_run_does_not_retry_the_queue_at_startup() {
        let fixture = Fixture::new("startup");
        fixture.queue_now();
        let run = |pairs: &'static [(&'static str, &'static str)]| {
            startup_in(
                move |key| {
                    pairs
                        .iter()
                        .find(|(name, _)| *name == key)
                        .map(|(_, value)| std::ffi::OsString::from(value))
                },
                || retry_in(&fixture.cache, &fixture.lock),
            )
            .unwrap()
        };
        for pairs in [
            &[("PFX_DIRECT", "1")][..],
            &[("PITO_ARCHIVE", "/work/app/tmp/archive")],
            &[("RIDE_APP", "alpha")],
        ] {
            assert_eq!(run(pairs), (0, 0));
            assert_eq!(fixture.entries(), 1);
            assert!(!fixture.leaf().exists());
        }
        assert_eq!(run(&[("PITO_ARCHIVE", "/srv/archive")]), (1, 0));
        assert_eq!(fixture.entries(), 0);
        assert!(fixture.leaf().join("manifest.json").is_file());
    }

    #[test]
    fn an_older_hhmm_leaf_is_read_and_left_alone() {
        let fixture = Fixture::new("old-leaf");
        let old = fixture
            .root
            .join("Masters/pfx/alpha/checker/2026-10-02_0914_v1_manual");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("clip.master.mp4"), b"first master").unwrap();
        let entry = catalog_for(&fixture.manifest, &fixture.root, &old, &fixture.source).unwrap();
        let mut stored = fixture.manifest.clone();
        stored["archive"] = json!({
            "folder": old,
            "state": "archived",
            "catalog": serde_json::to_string(&entry).unwrap(),
        });
        write_manifest(&old.join("manifest.json"), &stored).unwrap();
        assert_eq!(reindex_in(&fixture.root, &fixture.lock).unwrap(), 1);
        assert_eq!(
            fixture.catalog_paths(),
            ["pfx/alpha/checker/2026-10-02_0914_v1_manual"]
        );
        assert_eq!(fixture.archive(), State::Archived);
        assert_eq!(fixture.catalog_paths().len(), 2);
        assert_eq!(fixture.entries(), 0);
        assert!(old.join("clip.master.mp4").is_file());
    }

    #[test]
    fn a_leaf_that_cannot_be_checked_is_a_conflict() {
        let fixture = Fixture::new("leaf-checks");
        assert_eq!(fixture.archive(), State::Archived);
        fs::write(fixture.leaf().join("clip.master.mp4"), b"damaged").unwrap();
        fixture.queue_now();
        assert_eq!(retry_in(&fixture.cache, &fixture.lock).unwrap(), (1, 0));
        assert_eq!(fixture.manifest_state(), "conflict");
        assert_eq!(fixture.catalog_paths().len(), 1);
    }

    #[test]
    fn an_unsafe_name_is_refused() {
        let root = Path::new("tmp/archive");
        let made = "2026-10-02T09:14:00+02:00";
        for (product, shot, version, caller) in [
            ("alpha", "checker", "release/1.0", "manual"),
            ("..", "checker", "v1", "manual"),
            ("alpha", "", "v1", "manual"),
            ("alpha", "checker", "v1", "a b"),
            ("stu/dio", "checker", "v1", "manual"),
            ("alpha", ".", "v1", "manual"),
        ] {
            assert!(leaf_under(root, product, shot, version, made, caller).is_err());
        }
        for (key, value) in [
            ("product", "../alpha"),
            ("shot", "a b"),
            ("version", "release/1.0"),
        ] {
            let mut fixture = Fixture::new("refused");
            fixture.manifest[key] = json!(value);
            write_manifest(&fixture.manifest_path, &fixture.manifest).unwrap();
            assert_eq!(fixture.archive(), State::Refused);
            let saved = fixture.saved();
            assert_eq!(saved["archive"]["state"], "refused");
            assert!(saved["archive"]["leaf"].is_null());
            assert!(saved["archive"]["error"].as_str().unwrap().contains(value));
            assert!(!fixture.root.join("Masters").exists());
            assert_eq!(fixture.entries(), 0);
        }
    }

    #[test]
    fn a_transient_read_error_leaves_the_render_queued_and_a_retry_archives_it() {
        let fixture = Fixture::new("transient");
        assert_eq!(fixture.archive(), State::Archived);
        let stored = fixture.leaf().join("manifest.json");
        let aside = fixture.base.join("manifest.aside");
        fs::rename(&stored, &aside).unwrap();
        fs::create_dir(&stored).unwrap();
        assert_eq!(fixture.archive(), State::Queued);
        assert_eq!(fixture.manifest_state(), "queued");
        assert_eq!(fixture.entries(), 1);
        fs::remove_dir(&stored).unwrap();
        fs::rename(&aside, &stored).unwrap();
        assert_eq!(retry_in(&fixture.cache, &fixture.lock).unwrap(), (1, 0));
        assert_eq!(fixture.manifest_state(), "archived");
        assert_eq!(fixture.entries(), 0);
        assert_eq!(fixture.catalog_paths().len(), 1);
        let siblings = fixture.leaf().parent().unwrap().to_path_buf();
        assert_eq!(fs::read_dir(siblings).unwrap().count(), 1);
    }

    #[test]
    fn a_read_error_on_a_retry_keeps_the_entry_pending() {
        let fixture = Fixture::new("transient-retry");
        assert_eq!(fixture.archive(), State::Archived);
        let stored = fixture.leaf().join("manifest.json");
        let aside = fixture.base.join("manifest.aside");
        fs::rename(&stored, &aside).unwrap();
        fs::create_dir(&stored).unwrap();
        fixture.queue_now();
        assert_eq!(retry_in(&fixture.cache, &fixture.lock).unwrap(), (0, 1));
        assert_eq!(fixture.entries(), 1);
        assert_eq!(fixture.manifest_state(), "queued");
        fs::remove_dir(&stored).unwrap();
        fs::rename(&aside, &stored).unwrap();
        assert_eq!(retry_in(&fixture.cache, &fixture.lock).unwrap(), (1, 0));
        assert_eq!(fixture.manifest_state(), "archived");
    }

    #[test]
    fn a_real_sha256_mismatch_is_still_a_conflict() {
        let fixture = Fixture::new("mismatch");
        assert_eq!(fixture.archive(), State::Archived);
        fs::write(fixture.leaf().join("clip.master.mp4"), b"damaged").unwrap();
        assert_eq!(fixture.archive(), State::Conflict);
        assert_eq!(fixture.manifest_state(), "conflict");
        assert!(
            fixture.saved()["archive"]["error"]
                .as_str()
                .unwrap()
                .contains("clip.master.mp4")
        );
        assert_eq!(fixture.entries(), 0);
        assert_eq!(
            fs::read(fixture.leaf().join("clip.master.mp4")).unwrap(),
            b"damaged"
        );
    }

    #[test]
    fn a_leaf_manifest_that_cannot_be_this_render_is_a_conflict() {
        let fixture = Fixture::new("foreign-manifest");
        let leaf = fixture.leaf();
        fs::create_dir_all(&leaf).unwrap();
        fs::write(leaf.join("manifest.json"), b"not json").unwrap();
        assert_eq!(fixture.archive(), State::Conflict);
        assert_eq!(fixture.entries(), 0);
        assert_eq!(fixture.catalog_paths().len(), 0);
    }
}
