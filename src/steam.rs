use std::collections::BTreeSet;
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use pfx::harness::tools;
use serde::Serialize;

const PLATFORMS: &[&str] = &["windows", "linux", "macos"];

pub const TOOLS_MARKER: &str =
    "pfx-game tools build: the launcher's commands and pfx's editor are linked in";

const EXECUTABLE_MAGIC: &[&[u8]] = &[
    b"\x7fELF",
    b"MZ",
    b"\xfe\xed\xfa\xce",
    b"\xfe\xed\xfa\xcf",
    b"\xce\xfa\xed\xfe",
    b"\xcf\xfa\xed\xfe",
    b"\xca\xfe\xba\xbe",
];

struct Spec {
    app_id: u32,
    description: String,
    setlive: String,
    preview: bool,
    depots: Vec<Depot>,
}

struct Depot {
    platform: String,
    id: u32,
    mappings: Vec<Mapping>,
    files: Vec<Staged>,
}

struct Mapping {
    local: String,
    depot_path: String,
    recursive: bool,
}

struct Staged {
    src: PathBuf,
    rel: String,
}

enum Dest {
    Root,
    File(String),
    Inside(String),
}

pub fn run(args: &[String]) -> Result<(), String> {
    let mut manifest = None;
    let mut out = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if !matches!(flag, "--manifest" | "--out") {
            return Err(format!("unknown steam flag {flag}"));
        }
        let Some(value) = args.get(index + 1) else {
            return Err(format!("{flag} needs a value"));
        };
        let slot = if flag == "--manifest" {
            &mut manifest
        } else {
            &mut out
        };
        if slot.is_some() {
            return Err(format!("{flag} was given twice"));
        }
        *slot = Some(PathBuf::from(value));
        index += 2;
    }
    let manifest = manifest.ok_or("--manifest is required")?;
    let out = out.ok_or("--out is required")?;
    let line = stage(&manifest, &out)?;
    println!("{line}");
    Ok(())
}

fn stage(manifest: &Path, out: &Path) -> Result<String, String> {
    let root = caller_root()?;
    let manifest = locate_manifest(&root, manifest)?;
    let text = fs::read_to_string(&manifest).map_err(|error| io(&manifest, error))?;
    let base = manifest
        .parent()
        .ok_or_else(|| format!("{} has no directory", manifest.display()))?
        .to_path_buf();
    let spec = parse(&text, &root, &base)?;
    refuse_tools(&spec)?;
    let out = empty_out(out)?;
    if let Err(error) = write_stage(&out, &spec) {
        clear_out(&out);
        return Err(error);
    }
    Ok(steamcmd_line(
        &out.join(format!("app_build_{}.vdf", spec.app_id)),
    ))
}

fn is_executable(head: &[u8]) -> bool {
    EXECUTABLE_MAGIC.iter().any(|magic| head.starts_with(magic))
}

fn carries_tools(bytes: &[u8]) -> bool {
    let needle = TOOLS_MARKER.as_bytes();
    is_executable(bytes) && bytes.windows(needle.len()).any(|window| window == needle)
}

fn refuse_tools(spec: &Spec) -> Result<(), String> {
    for depot in &spec.depots {
        for file in &depot.files {
            let mut head = Vec::with_capacity(4);
            fs::File::open(&file.src)
                .and_then(|source| source.take(4).read_to_end(&mut head))
                .map_err(|error| io(&file.src, error))?;
            if !is_executable(&head) {
                continue;
            }
            let bytes = fs::read(&file.src).map_err(|error| io(&file.src, error))?;
            if carries_tools(&bytes) {
                return Err(format!(
                    "{}: built with pfx-game's tools feature, so its commands and pfx's editor would ship to Steam; build the Steam binary with `cargo build --release --no-default-features`",
                    file.src.display()
                ));
            }
        }
    }
    Ok(())
}

fn caller_root() -> Result<PathBuf, String> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| format!("git: {error}"))?;
    if !output.status.success() {
        return Err("pfx steam stage runs inside a git tree".into());
    }
    let text = String::from_utf8(output.stdout).map_err(|error| format!("git: {error}"))?;
    let root = PathBuf::from(text.trim());
    root.canonicalize().map_err(|error| io(&root, error))
}

fn cwd() -> Result<PathBuf, String> {
    std::env::current_dir().map_err(|error| format!("cwd: {error}"))
}

fn locate_manifest(root: &Path, raw: &Path) -> Result<PathBuf, String> {
    let path = resolve(root, &cwd()?, raw)?;
    if !path.is_file() {
        return Err(format!("{} is not a file", raw.display()));
    }
    Ok(path)
}

fn resolve(root: &Path, base: &Path, raw: &Path) -> Result<PathBuf, String> {
    let built = normalize(base, raw);
    if let Some(link) = first_symlink(&built)? {
        let canon = fs::canonicalize(&link).map_err(|error| io(&link, error))?;
        if !canon.starts_with(root) {
            return Err(format!("{} escapes the caller's folder", raw.display()));
        }
        return Err(format!("{} is a symlink", raw.display()));
    }
    let mut existing = built.as_path();
    let mut rest = Vec::new();
    while !existing.exists() {
        let Some(parent) = existing.parent() else {
            return Err(format!("{} escapes the caller's folder", raw.display()));
        };
        if parent == existing {
            return Err(format!("{} escapes the caller's folder", raw.display()));
        }
        let Some(name) = existing.file_name() else {
            return Err(format!("{} escapes the caller's folder", raw.display()));
        };
        rest.push(name.to_os_string());
        existing = parent;
    }
    let mut full = existing
        .canonicalize()
        .map_err(|error| io(existing, error))?;
    for name in rest.iter().rev() {
        full.push(name);
    }
    if !full.starts_with(root) {
        return Err(format!("{} escapes the caller's folder", raw.display()));
    }
    if !full.exists() {
        return Err(format!("missing file {}", raw.display()));
    }
    Ok(full)
}

fn normalize(base: &Path, raw: &Path) -> PathBuf {
    let absolute = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        base.join(raw)
    };
    let mut built = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                built.pop();
            }
            other => built.push(other),
        }
    }
    built
}

fn first_symlink(path: &Path) -> Result<Option<PathBuf>, String> {
    let mut check = PathBuf::new();
    for part in path.components() {
        check.push(part);
        if check == Path::new("/") {
            continue;
        }
        match fs::symlink_metadata(&check) {
            Ok(meta) if meta.file_type().is_symlink() => return Ok(Some(check)),
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io(&check, error)),
        }
    }
    Ok(None)
}

fn empty_out(raw: &Path) -> Result<PathBuf, String> {
    let absolute = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        cwd()?.join(raw)
    };
    if let Some(link) = first_symlink(&absolute)? {
        let canon = fs::canonicalize(&link).map_err(|error| io(&link, error))?;
        let tmp = tools::caller_tmp()?;
        if !canon.starts_with(&tmp) {
            return Err(format!("{} escapes the caller's folder", raw.display()));
        }
        return Err(format!("{} is a symlink", raw.display()));
    }
    let out = tools::inside(&tools::caller_tmp()?, raw)?;
    if fs::read_dir(&out)
        .map_err(|error| io(&out, error))?
        .next()
        .is_some()
    {
        return Err("--out is not empty".into());
    }
    out.canonicalize().map_err(|error| io(&out, error))
}

fn parse(text: &str, root: &Path, base: &Path) -> Result<Spec, String> {
    let value: toml::Value =
        toml::from_str(text).map_err(|error| format!("steam.toml: {error}"))?;
    let table = value.as_table().ok_or("steam.toml must be a table")?;
    unknown(
        table,
        "",
        &[
            "app_id",
            "description",
            "setlive",
            "preview",
            "setlive_default",
            "depots",
        ],
    )?;
    let app_id = id_of(required(table, "app_id")?, "app_id")?;
    let description = text_of(required(table, "description")?, "description")?;
    if description.trim().is_empty() {
        return Err("description is required".into());
    }
    let setlive = match table.get("setlive") {
        None => String::new(),
        Some(value) => text_of(value, "setlive")?.trim().to_string(),
    };
    let setlive_default = match table.get("setlive_default") {
        None => false,
        Some(value) => flag(value, "setlive_default")?,
    };
    if setlive == "default" && !setlive_default {
        return Err("setlive default needs setlive_default = true".into());
    }
    let preview = match table.get("preview") {
        None => false,
        Some(value) => flag(value, "preview")?,
    };
    let depots = match table.get("depots") {
        None => return Err("steam.toml names no depot".into()),
        Some(value) => value.as_table().ok_or("depots must be a table")?,
    };
    if depots.is_empty() {
        return Err("steam.toml names no depot".into());
    }
    for key in depots.keys() {
        if !PLATFORMS.contains(&key.as_str()) {
            return Err(format!("unknown platform {key}"));
        }
    }
    let mut built = Vec::new();
    let mut ids = BTreeSet::new();
    for platform in PLATFORMS {
        let Some(value) = depots.get(*platform) else {
            continue;
        };
        let depot = parse_depot(platform, value, root, base)?;
        if !ids.insert(depot.id) {
            return Err(format!("depot id {} is duplicated", depot.id));
        }
        built.push(depot);
    }
    Ok(Spec {
        app_id,
        description,
        setlive,
        preview,
        depots: built,
    })
}

fn parse_depot(
    platform: &str,
    value: &toml::Value,
    root: &Path,
    base: &Path,
) -> Result<Depot, String> {
    let table = value
        .as_table()
        .ok_or_else(|| format!("{platform} must be a table"))?;
    unknown(table, platform, &["id", "files"])?;
    let id = id_of(required(table, "id")?, &format!("{platform} id"))?;
    let files = match table.get("files") {
        None => return Err(format!("depot {id} has no files")),
        Some(value) => value
            .as_array()
            .ok_or_else(|| format!("{platform} files must be a list"))?,
    };
    if files.is_empty() {
        return Err(format!("depot {id} has no files"));
    }
    let mut mappings = Vec::new();
    let mut staged = Vec::new();
    let mut seen = BTreeSet::new();
    for item in files {
        let file = item
            .as_table()
            .ok_or_else(|| format!("{platform} file must be a table"))?;
        unknown(file, &format!("{platform}.files"), &["path", "depot"])?;
        let path = text_of(required(file, "path")?, "path")?;
        let depot = text_of(required(file, "depot")?, "depot")?;
        let src = resolve(root, base, Path::new(&path))?;
        add_mapping(
            id,
            &path,
            &depot,
            &src,
            &mut mappings,
            &mut staged,
            &mut seen,
        )?;
    }
    Ok(Depot {
        platform: platform.to_string(),
        id,
        mappings,
        files: staged,
    })
}

fn add_mapping(
    id: u32,
    source: &str,
    depot: &str,
    src: &Path,
    mappings: &mut Vec<Mapping>,
    staged: &mut Vec<Staged>,
    seen: &mut BTreeSet<String>,
) -> Result<(), String> {
    let dest = depot_dest(depot)?;
    let meta = fs::metadata(src).map_err(|error| io(src, error))?;
    if meta.is_file() {
        let rel = file_rel(&dest, src)?;
        remember(id, &rel, seen)?;
        let (local, depot_path, recursive) = file_mapping(&rel);
        mappings.push(Mapping {
            local,
            depot_path,
            recursive,
        });
        staged.push(Staged {
            src: src.to_path_buf(),
            rel,
        });
        return Ok(());
    }
    if !meta.is_dir() {
        return Err(format!("{source} is not a file"));
    }
    let prefix = dir_prefix(&dest);
    let children = walk_files(src)?;
    if children.is_empty() {
        return Err(format!("{source} maps no files"));
    }
    let (local, depot_path, recursive) = dir_mapping(&prefix);
    mappings.push(Mapping {
        local,
        depot_path,
        recursive,
    });
    for (child, rel) in children {
        let full = join_rel(&prefix, &rel);
        remember(id, &full, seen)?;
        staged.push(Staged {
            src: child,
            rel: full,
        });
    }
    Ok(())
}

fn file_rel(dest: &Dest, src: &Path) -> Result<String, String> {
    let name = src
        .file_name()
        .and_then(|text| text.to_str())
        .ok_or_else(|| format!("{} is not utf-8", src.display()))?;
    if name.is_empty() || name == "." || name == ".." {
        return Err(format!("depot path {name} escapes the depot"));
    }
    Ok(match dest {
        Dest::Root => name.to_string(),
        Dest::Inside(dir) => format!("{dir}/{name}"),
        Dest::File(path) => path.clone(),
    })
}

fn dir_prefix(dest: &Dest) -> String {
    match dest {
        Dest::Root => String::new(),
        Dest::Inside(dir) | Dest::File(dir) => dir.clone(),
    }
}

fn join_rel(prefix: &str, rel: &str) -> String {
    if prefix.is_empty() {
        rel.to_string()
    } else {
        format!("{prefix}/{rel}")
    }
}

fn file_mapping(rel: &str) -> (String, String, bool) {
    match rel.rsplit_once('/') {
        Some((dir, _)) => (rel.to_string(), dir.to_string(), false),
        None => (rel.to_string(), ".".to_string(), false),
    }
}

fn dir_mapping(prefix: &str) -> (String, String, bool) {
    if prefix.is_empty() {
        ("*".to_string(), ".".to_string(), true)
    } else {
        (format!("{prefix}/*"), prefix.to_string(), true)
    }
}

fn remember(id: u32, rel: &str, seen: &mut BTreeSet<String>) -> Result<(), String> {
    if !seen.insert(rel.to_string()) {
        return Err(format!("depot {id} maps {rel} twice"));
    }
    Ok(())
}

fn depot_dest(raw: &str) -> Result<Dest, String> {
    if raw.is_empty() || raw == "." {
        return Ok(Dest::Root);
    }
    if raw.starts_with('/') || raw.contains('\\') {
        return Err(format!("depot path {raw} escapes the depot"));
    }
    let inside = raw.ends_with('/');
    let trimmed = raw.trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok(Dest::Root);
    }
    let mut parts = Vec::new();
    for part in trimmed.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.contains('*')
            || part.contains('?')
            || part.chars().any(char::is_control)
        {
            return Err(format!("depot path {raw} escapes the depot"));
        }
        parts.push(part);
    }
    let joined = parts.join("/");
    if inside {
        Ok(Dest::Inside(joined))
    } else {
        Ok(Dest::File(joined))
    }
}

fn walk_files(dir: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((current, prefix)) = stack.pop() {
        let mut paths = Vec::new();
        for entry in fs::read_dir(&current).map_err(|error| io(&current, error))? {
            let entry = entry.map_err(|error| io(&current, error))?;
            paths.push(entry.path());
        }
        paths.sort();
        for path in paths {
            let name = path
                .file_name()
                .and_then(|text| text.to_str())
                .ok_or_else(|| format!("{} is not utf-8", path.display()))?;
            let rel = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            };
            let meta = fs::symlink_metadata(&path).map_err(|error| io(&path, error))?;
            if meta.file_type().is_symlink() {
                return Err(format!("{} is a symlink", path.display()));
            }
            if meta.is_dir() {
                stack.push((path, rel));
            } else if meta.is_file() {
                out.push((path, rel));
            } else {
                return Err(format!("{} is not a file", path.display()));
            }
        }
    }
    out.sort_by(|left, right| left.1.cmp(&right.1));
    Ok(out)
}

fn write_stage(out: &Path, spec: &Spec) -> Result<(), String> {
    let output = out.join("output");
    fs::create_dir_all(&output).map_err(|error| io(&output, error))?;
    for depot in &spec.depots {
        let folder = out.join("content").join(depot.id.to_string());
        fs::create_dir_all(&folder).map_err(|error| io(&folder, error))?;
        for file in &depot.files {
            let dest = join_slash(&folder, &file.rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|error| io(parent, error))?;
            }
            fs::copy(&file.src, &dest).map_err(|error| io(&dest, error))?;
        }
        let path = out.join(format!("depot_build_{}.vdf", depot.id));
        fs::write(&path, depot_script(depot)?).map_err(|error| io(&path, error))?;
    }
    let app_path = out.join(format!("app_build_{}.vdf", spec.app_id));
    fs::write(&app_path, app_script(spec)?).map_err(|error| io(&app_path, error))?;
    let manifest_path = out.join("steam-manifest.json");
    fs::write(&manifest_path, manifest_json(spec, out)?)
        .map_err(|error| io(&manifest_path, error))?;
    Ok(())
}

fn app_script(spec: &Spec) -> Result<String, String> {
    let mut vdf = Vdf::new("appbuild");
    vdf.key(1, "appid", &spec.app_id.to_string())?;
    vdf.key(1, "desc", &spec.description)?;
    vdf.key(1, "buildoutput", "output")?;
    vdf.key(1, "contentroot", "content")?;
    vdf.key(1, "setlive", &spec.setlive)?;
    vdf.key(1, "preview", if spec.preview { "1" } else { "0" })?;
    vdf.key(1, "local", "")?;
    vdf.raw("\t\"depots\"\n\t{\n");
    for depot in &spec.depots {
        vdf.key(
            2,
            &depot.id.to_string(),
            &format!("depot_build_{}.vdf", depot.id),
        )?;
    }
    vdf.raw("\t}\n");
    Ok(vdf.finish())
}

fn depot_script(depot: &Depot) -> Result<String, String> {
    let mut vdf = Vdf::new("DepotBuildConfig");
    vdf.key(1, "DepotID", &depot.id.to_string())?;
    vdf.key(1, "ContentRoot", &format!("content/{}", depot.id))?;
    for mapping in &depot.mappings {
        vdf.raw("\t\"FileMapping\"\n\t{\n");
        vdf.key(2, "LocalPath", &mapping.local)?;
        vdf.key(2, "DepotPath", &mapping.depot_path)?;
        vdf.key(2, "recursive", if mapping.recursive { "1" } else { "0" })?;
        vdf.raw("\t}\n");
    }
    Ok(vdf.finish())
}

struct Vdf {
    text: String,
}

impl Vdf {
    fn new(root: &str) -> Self {
        Self {
            text: format!("\"{root}\"\n{{\n"),
        }
    }

    fn key(&mut self, indent: usize, key: &str, value: &str) -> Result<(), String> {
        self.text.push_str(&"\t".repeat(indent));
        self.text.push_str(&quote(key)?);
        self.text.push('\t');
        self.text.push_str(&quote(value)?);
        self.text.push('\n');
        Ok(())
    }

    fn raw(&mut self, text: &str) {
        self.text.push_str(text);
    }

    fn finish(mut self) -> String {
        self.text.push_str("}\n");
        self.text
    }
}

fn quote(value: &str) -> Result<String, String> {
    if value.chars().any(char::is_control) {
        return Err(format!("value {value:?} must be one line"));
    }
    let mut out = String::from("\"");
    for c in value.chars() {
        if matches!(c, '\\' | '"') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    Ok(out)
}

fn manifest_json(spec: &Spec, out: &Path) -> Result<String, String> {
    let steamcmd = steamcmd_line(&out.join(format!("app_build_{}.vdf", spec.app_id)));
    let mut depots = Vec::new();
    for depot in &spec.depots {
        let folder = out.join("content").join(depot.id.to_string());
        let mut files = Vec::new();
        for file in &depot.files {
            let staged = join_slash(&folder, &file.rel);
            let bytes = fs::metadata(&staged)
                .map_err(|error| io(&staged, error))?
                .len();
            files.push(FileRecord {
                path: file.rel.clone(),
                bytes,
                sha256: tools::hash_file(&staged)?,
            });
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        depots.push(DepotRecord {
            platform: depot.platform.clone(),
            id: depot.id,
            script: format!("depot_build_{}.vdf", depot.id),
            files,
        });
    }
    let body = SteamManifest {
        v: 1,
        app_id: spec.app_id,
        description: spec.description.clone(),
        setlive: spec.setlive.clone(),
        preview: spec.preview,
        app_build: format!("app_build_{}.vdf", spec.app_id),
        steamcmd,
        depots,
    };
    let mut text = serde_json::to_string_pretty(&body).map_err(|error| error.to_string())?;
    text.push('\n');
    Ok(text)
}

fn steamcmd_line(app_vdf: &Path) -> String {
    format!(
        "steamcmd +login \"$STEAM_BUILDER\" +run_app_build {} +quit",
        shell_quote(&app_vdf.display().to_string())
    )
}

fn shell_quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        if matches!(c, '\\' | '"' | '$' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn join_slash(root: &Path, rel: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for part in rel.split('/') {
        if !part.is_empty() {
            path.push(part);
        }
    }
    path
}

fn clear_out(out: &Path) {
    let Ok(entries) = fs::read_dir(out) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            let _ = fs::remove_dir_all(&path);
        } else {
            let _ = fs::remove_file(&path);
        }
    }
}

fn unknown(table: &toml::Table, at: &str, allowed: &[&str]) -> Result<(), String> {
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            let name = if at.is_empty() {
                key.clone()
            } else {
                format!("{at}.{key}")
            };
            return Err(format!("unknown steam.toml key {name}"));
        }
    }
    Ok(())
}

fn required<'a>(table: &'a toml::Table, key: &str) -> Result<&'a toml::Value, String> {
    table.get(key).ok_or_else(|| format!("{key} is required"))
}

fn id_of(value: &toml::Value, key: &str) -> Result<u32, String> {
    let Some(number) = value.as_integer() else {
        return Err(format!("{key} must be a positive integer"));
    };
    if number <= 0 || number > u32::MAX as i64 {
        return Err(format!("{key} must be a positive integer"));
    }
    Ok(number as u32)
}

fn flag(value: &toml::Value, key: &str) -> Result<bool, String> {
    value
        .as_bool()
        .ok_or_else(|| format!("{key} must be true or false"))
}

fn text_of(value: &toml::Value, key: &str) -> Result<String, String> {
    let Some(text) = value.as_str() else {
        return Err(format!("{key} must be a string"));
    };
    if text.chars().any(char::is_control) {
        return Err(format!("{key} must be one line"));
    }
    Ok(text.to_string())
}

fn io(path: &Path, error: std::io::Error) -> String {
    format!("{}: {error}", path.display())
}

#[derive(Serialize)]
struct SteamManifest {
    v: u32,
    app_id: u32,
    description: String,
    setlive: String,
    preview: bool,
    app_build: String,
    steamcmd: String,
    depots: Vec<DepotRecord>,
}

#[derive(Serialize)]
struct DepotRecord {
    platform: String,
    id: u32,
    script: String,
    files: Vec<FileRecord>,
}

#[derive(Serialize)]
struct FileRecord {
    path: String,
    bytes: u64,
    sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join("steam-stage")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn toml_basic(value: &str) -> String {
        let mut out = String::from("\"");
        for c in value.chars() {
            if matches!(c, '\\' | '"') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out
    }

    fn sha(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    fn write_manifest(game: &Path, body: &str) -> PathBuf {
        let path = game.join("steam.toml");
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn two_depots_are_golden_and_stage_exactly_the_mapped_files() {
        let root = scratch("golden");
        let game = root.join("game");
        fs::create_dir_all(game.join("windows/assets/nested")).unwrap();
        fs::create_dir_all(game.join("linux")).unwrap();
        fs::write(game.join("windows/example-game.exe"), b"WIN\n").unwrap();
        fs::write(game.join("windows/assets/readme.txt"), b"readme\n").unwrap();
        fs::write(game.join("windows/assets/nested/a.txt"), b"a\n").unwrap();
        fs::write(game.join("windows/stray.bin"), b"no\n").unwrap();
        let linux = game.join("linux/example-game");
        fs::write(&linux, b"LIN\n").unwrap();
        let body = format!(
            "app_id = 424242\ndescription = \"Example Game 1.0.0\"\n\n[depots.windows]\nid = 424243\n\n[[depots.windows.files]]\npath = \"windows/example-game.exe\"\ndepot = \"example-game.exe\"\n\n[[depots.windows.files]]\npath = \"windows/assets\"\ndepot = \"assets\"\n\n[depots.linux]\nid = 424244\n\n[[depots.linux.files]]\npath = {}\ndepot = \"example-game\"\n",
            toml_basic(&linux.display().to_string())
        );
        let manifest = write_manifest(&game, &body);
        let out = root.join("out");
        let line = stage(&manifest, &out).unwrap();
        let out = out.canonicalize().unwrap();

        let app = fs::read_to_string(out.join("app_build_424242.vdf")).unwrap();
        assert_eq!(
            app,
            "\
\"appbuild\"
{
\t\"appid\"\t\"424242\"
\t\"desc\"\t\"Example Game 1.0.0\"
\t\"buildoutput\"\t\"output\"
\t\"contentroot\"\t\"content\"
\t\"setlive\"\t\"\"
\t\"preview\"\t\"0\"
\t\"local\"\t\"\"
\t\"depots\"
\t{
\t\t\"424243\"\t\"depot_build_424243.vdf\"
\t\t\"424244\"\t\"depot_build_424244.vdf\"
\t}
}
"
        );
        let windows = fs::read_to_string(out.join("depot_build_424243.vdf")).unwrap();
        assert_eq!(
            windows,
            "\
\"DepotBuildConfig\"
{
\t\"DepotID\"\t\"424243\"
\t\"ContentRoot\"\t\"content/424243\"
\t\"FileMapping\"
\t{
\t\t\"LocalPath\"\t\"example-game.exe\"
\t\t\"DepotPath\"\t\".\"
\t\t\"recursive\"\t\"0\"
\t}
\t\"FileMapping\"
\t{
\t\t\"LocalPath\"\t\"assets/*\"
\t\t\"DepotPath\"\t\"assets\"
\t\t\"recursive\"\t\"1\"
\t}
}
"
        );
        let linux_vdf = fs::read_to_string(out.join("depot_build_424244.vdf")).unwrap();
        assert_eq!(
            linux_vdf,
            "\
\"DepotBuildConfig\"
{
\t\"DepotID\"\t\"424244\"
\t\"ContentRoot\"\t\"content/424244\"
\t\"FileMapping\"
\t{
\t\t\"LocalPath\"\t\"example-game\"
\t\t\"DepotPath\"\t\".\"
\t\t\"recursive\"\t\"0\"
\t}
}
"
        );

        let expected = format!(
            "steamcmd +login \"$STEAM_BUILDER\" +run_app_build \"{}\" +quit",
            out.join("app_build_424242.vdf").display()
        );
        assert_eq!(line, expected);
        assert!(!line.contains("password"));
        assert!(line.contains("+login \"$STEAM_BUILDER\" +run_app_build "));
        assert!(out.join("output").is_dir());

        let content = out.join("content");
        let mut staged = Vec::new();
        for depot in ["424243", "424244"] {
            let mut stack = vec![content.join(depot)];
            while let Some(dir) = stack.pop() {
                let mut entries: Vec<_> = fs::read_dir(&dir)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect();
                entries.sort();
                for path in entries {
                    if path.is_dir() {
                        stack.push(path);
                    } else {
                        staged.push(
                            path.strip_prefix(&content)
                                .unwrap()
                                .to_string_lossy()
                                .replace('\\', "/"),
                        );
                    }
                }
            }
        }
        staged.sort();
        assert_eq!(
            staged,
            [
                "424243/assets/nested/a.txt",
                "424243/assets/readme.txt",
                "424243/example-game.exe",
                "424244/example-game",
            ]
        );
        assert_eq!(
            fs::read(content.join("424243/example-game.exe")).unwrap(),
            b"WIN\n"
        );
        assert_eq!(
            fs::read(content.join("424243/assets/readme.txt")).unwrap(),
            b"readme\n"
        );
        assert_eq!(
            fs::read(content.join("424243/assets/nested/a.txt")).unwrap(),
            b"a\n"
        );
        assert_eq!(
            fs::read(content.join("424244/example-game")).unwrap(),
            b"LIN\n"
        );

        let record: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(out.join("steam-manifest.json")).unwrap())
                .unwrap();
        assert_eq!(record["v"], 1);
        assert_eq!(record["app_id"], 424242);
        assert_eq!(record["description"], "Example Game 1.0.0");
        assert_eq!(record["setlive"], "");
        assert_eq!(record["preview"].as_bool(), Some(false));
        assert_eq!(record["steamcmd"], line);
        assert_eq!(record["depots"][0]["platform"], "windows");
        assert_eq!(record["depots"][0]["id"], 424243);
        assert_eq!(record["depots"][1]["platform"], "linux");
        assert_eq!(record["depots"][1]["id"], 424244);
        let files = record["depots"][0]["files"].as_array().unwrap();
        assert_eq!(files[0]["path"], "assets/nested/a.txt");
        assert_eq!(files[0]["bytes"], 2);
        assert_eq!(files[0]["sha256"], sha(b"a\n"));
        assert_eq!(files[1]["path"], "assets/readme.txt");
        assert_eq!(files[1]["sha256"], sha(b"readme\n"));
        assert_eq!(files[2]["path"], "example-game.exe");
        assert_eq!(files[2]["bytes"], 4);
        assert_eq!(files[2]["sha256"], sha(b"WIN\n"));
        let linux_files = record["depots"][1]["files"].as_array().unwrap();
        assert_eq!(linux_files[0]["path"], "example-game");
        assert_eq!(linux_files[0]["sha256"], sha(b"LIN\n"));
        let _ = fs::remove_dir_all(&root);
    }

    fn game_with(name: &str, extra: &str, files: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = scratch(name);
        let game = root.join("game");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("game.bin"), b"game").unwrap();
        let body = format!("app_id = 10\ndescription = \"build\"\n{extra}\n{files}\n");
        let manifest = write_manifest(&game, &body);
        let out = root.join("out");
        (root, manifest, out)
    }

    #[test]
    fn refusals() {
        let (root, manifest, out) = game_with(
            "missing",
            "",
            "[depots.windows]\nid = 11\n\n[[depots.windows.files]]\npath = \"missing.bin\"\ndepot = \"game.bin\"\n",
        );
        let error = stage(&manifest, &out).unwrap_err();
        assert!(error.contains("missing file missing.bin"), "{error}");
        assert!(!out.join("content").exists());
        let _ = fs::remove_dir_all(&root);

        let (root, manifest, out) = game_with(
            "absolute",
            "",
            "[depots.windows]\nid = 11\n\n[[depots.windows.files]]\npath = \"/etc/passwd\"\ndepot = \"passwd\"\n",
        );
        let error = stage(&manifest, &out).unwrap_err();
        assert!(
            error.contains("/etc/passwd") && error.contains("escapes the caller's folder"),
            "{error}"
        );
        assert!(!out.exists());
        let _ = fs::remove_dir_all(&root);

        let (root, manifest, out) = game_with("relative", "", "");
        let game = manifest.parent().unwrap();
        let mut climb = String::new();
        let mut dir = game.to_path_buf();
        while dir.pop() {
            climb.push_str("../");
        }
        climb.push_str("etc/passwd");
        let body = format!(
            "app_id = 10\ndescription = \"build\"\n\n[depots.linux]\nid = 12\n\n[[depots.linux.files]]\npath = \"{climb}\"\ndepot = \"passwd\"\n"
        );
        fs::write(&manifest, body).unwrap();
        let error = stage(&manifest, &out).unwrap_err();
        assert!(error.contains("escapes the caller's folder"), "{error}");
        assert!(!out.exists());
        let _ = fs::remove_dir_all(&root);

        let (root, manifest, out) =
            game_with("empty", "", "[depots.windows]\nid = 11\nfiles = []\n");
        let error = stage(&manifest, &out).unwrap_err();
        assert_eq!(error, "depot 11 has no files");
        let _ = fs::remove_dir_all(&root);

        let (root, manifest, out) = game_with(
            "duplicate",
            "",
            "[depots.windows]\nid = 11\n\n[[depots.windows.files]]\npath = \"game.bin\"\ndepot = \"game.bin\"\n\n[depots.linux]\nid = 11\n\n[[depots.linux.files]]\npath = \"game.bin\"\ndepot = \"game.bin\"\n",
        );
        let error = stage(&manifest, &out).unwrap_err();
        assert_eq!(error, "depot id 11 is duplicated");
        let _ = fs::remove_dir_all(&root);

        let (root, manifest, out) = game_with(
            "setlive",
            "setlive = \"default\"",
            "[depots.windows]\nid = 11\n\n[[depots.windows.files]]\npath = \"game.bin\"\ndepot = \"game.bin\"\n",
        );
        let error = stage(&manifest, &out).unwrap_err();
        assert_eq!(error, "setlive default needs setlive_default = true");
        assert!(!out.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_executable_built_with_pfx_games_tools_is_refused() {
        let (root, manifest, out) = game_with(
            "tools",
            "",
            "[depots.linux]\nid = 11\n\n[[depots.linux.files]]\npath = \"game.bin\"\ndepot = \"game\"\n",
        );
        let game = manifest.parent().unwrap().join("game.bin");
        let mut tools = b"\x7fELF\x02\x01\x01".to_vec();
        tools.extend(vec![0; 4096]);
        tools.extend(TOOLS_MARKER.as_bytes());
        tools.extend(vec![0; 64]);
        fs::write(&game, &tools).unwrap();
        let error = stage(&manifest, &out).unwrap_err();
        assert!(
            error.contains("game.bin")
                && error.contains("pfx-game's tools feature")
                && error.contains("--no-default-features"),
            "{error}"
        );
        assert!(!out.exists(), "nothing is staged");
        let mut windows = b"MZ".to_vec();
        windows.extend(TOOLS_MARKER.as_bytes());
        assert!(carries_tools(&windows));
        fs::write(&game, &tools[..4096]).unwrap();
        stage(&manifest, &out).unwrap();
        let mut asset = b"notes: ".to_vec();
        asset.extend(TOOLS_MARKER.as_bytes());
        assert!(!carries_tools(&asset), "only executables are scanned");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_marker_is_pfx_games_own() {
        let launcher = fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("crates/game/src/launcher.rs"),
        )
        .unwrap();
        assert!(
            launcher.contains(&format!(
                "pub const TOOLS_MARKER: &str =\n    {TOOLS_MARKER:?};"
            )),
            "src/steam.rs and crates/game/src/launcher.rs carry the same marker"
        );
    }

    #[test]
    fn setlive_default_is_written_when_the_manifest_says_so() {
        let (root, manifest, out) = game_with(
            "live",
            "setlive = \"default\"\nsetlive_default = true\npreview = true",
            "[depots.macos]\nid = 13\n\n[[depots.macos.files]]\npath = \"game.bin\"\ndepot = \"bin/\"\n",
        );
        let line = stage(&manifest, &out).unwrap();
        let app = fs::read_to_string(out.join("app_build_10.vdf")).unwrap();
        assert!(app.contains("\t\"setlive\"\t\"default\"\n"), "{app}");
        assert!(app.contains("\t\"preview\"\t\"1\"\n"), "{app}");
        assert!(
            app.contains("\t\t\"13\"\t\"depot_build_13.vdf\"\n"),
            "{app}"
        );
        let depot = fs::read_to_string(out.join("depot_build_13.vdf")).unwrap();
        assert!(
            depot.contains("\t\t\"LocalPath\"\t\"bin/game.bin\"\n"),
            "{depot}"
        );
        assert!(depot.contains("\t\t\"DepotPath\"\t\"bin\"\n"), "{depot}");
        assert_eq!(
            fs::read(out.join("content/13/bin/game.bin")).unwrap(),
            b"game"
        );
        assert!(line.contains("app_build_10.vdf"));
        assert!(line.starts_with("steamcmd +login \"$STEAM_BUILDER\" +run_app_build "));
        assert!(line.ends_with(" +quit"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_is_refused() {
        let (root, manifest, out) = game_with(
            "link",
            "",
            "[depots.windows]\nid = 11\n\n[[depots.windows.files]]\npath = \"link.bin\"\ndepot = \"game.bin\"\n",
        );
        let game = manifest.parent().unwrap();
        std::os::unix::fs::symlink(game.join("game.bin"), game.join("link.bin")).unwrap();
        let error = stage(&manifest, &out).unwrap_err();
        assert!(error.contains("is a symlink"), "{error}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_used_out_directory_is_refused() {
        let (root, manifest, out) = game_with(
            "used",
            "",
            "[depots.windows]\nid = 11\n\n[[depots.windows.files]]\npath = \"game.bin\"\ndepot = \"game.bin\"\n",
        );
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("keep"), b"keep").unwrap();
        let error = stage(&manifest, &out).unwrap_err();
        assert_eq!(error, "--out is not empty");
        assert_eq!(fs::read(out.join("keep")).unwrap(), b"keep");
        let _ = fs::remove_dir_all(&root);
    }
}
