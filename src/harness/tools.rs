pub use pfx_run::ffmpeg::Pin;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

const CONFIG: &str = pfx_run::RENDER_TOML;
pub const ENGINE_FILE: &str = "render/engine.toml";

#[derive(Deserialize)]
struct Config {
    #[serde(default)]
    moved: Vec<Moved>,
}

#[derive(Deserialize, Clone)]
struct Moved {
    package: String,
    from: String,
    to: String,
}

fn rewrites(lock: &str, moved: &[Moved]) -> Vec<(String, String)> {
    let Ok(lock) = toml::from_str::<toml::Value>(lock) else {
        return Vec::new();
    };
    let pinned = |m: &Moved| {
        lock.get("package")
            .and_then(|p| p.as_array())
            .into_iter()
            .flatten()
            .any(|p| {
                p.get("name").and_then(|n| n.as_str()) == Some(m.package.as_str())
                    && p.get("source")
                        .and_then(|s| s.as_str())
                        .and_then(|s| s.strip_prefix("git+"))
                        .and_then(|s| s.strip_prefix(m.from.as_str()))
                        .is_some_and(|rest| {
                            rest.is_empty() || rest.starts_with('?') || rest.starts_with('#')
                        })
            })
    };
    moved
        .iter()
        .filter(|m| pinned(m))
        .map(|m| (format!("url.{}.insteadOf", m.to), m.from.clone()))
        .collect()
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct App {
    pub repo: String,
    pub package: String,
    pub bin: String,
    #[serde(default)]
    pub was: Vec<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub name: String,
    pub package: String,
    pub bin: String,
    #[serde(default)]
    pub was: Vec<String>,
}

#[derive(Deserialize)]
struct Own {
    app: Row,
}

impl Row {
    fn with_repo(self, repo: &str) -> App {
        App {
            repo: repo.to_string(),
            package: self.package,
            bin: self.bin,
            was: self.was,
        }
    }
}

pub fn own_row(text: &str, place: &str) -> Result<Row, String> {
    toml::from_str::<Own>(text)
        .map(|own| own.app)
        .map_err(|e| format!("{place}: {e}"))
}

fn own_row_in(dir: &Path) -> Result<Option<Row>, String> {
    let path = dir.join(ENGINE_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => own_row(&text, &path.display().to_string()).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

pub fn caller_root() -> Option<PathBuf> {
    git(None, &["rev-parse", "--show-toplevel"])
        .ok()
        .filter(|top| !top.is_empty())
        .map(PathBuf::from)
}

#[derive(Clone, Debug)]
pub struct OwnApp {
    pub dir: PathBuf,
    pub row: Row,
    pub repo: String,
}

pub fn own_app(name: &str, caller: Option<&Path>) -> Result<OwnApp, String> {
    let tool = pfx_run::job::TOOL;
    let dir = caller.ok_or_else(|| {
        format!(
            "{tool} render --app runs inside the app's own git tree: an app renders only itself"
        )
    })?;
    let row = own_row_in(dir)?.ok_or_else(|| {
        format!(
            "{} has no {ENGINE_FILE}: an app renders only itself, from its own tree, whose {ENGINE_FILE} [app] names it",
            dir.display()
        )
    })?;
    if row.name != name {
        return Err(format!(
            "{}'s {ENGINE_FILE} names app '{}', not '{name}': an app renders only itself",
            dir.display(),
            row.name
        ));
    }
    let repo = git(Some(dir), &["remote", "get-url", "origin"])
        .unwrap_or_else(|_| dir.display().to_string());
    Ok(OwnApp {
        dir: dir.to_path_buf(),
        row,
        repo,
    })
}

fn row_or(name: &str, at: Option<Row>, own: &OwnApp, place: &str) -> Result<App, String> {
    let row = at.unwrap_or_else(|| own.row.clone());
    if row.name != name {
        return Err(format!(
            "{place}'s {ENGINE_FILE} names app '{}', not '{name}'",
            row.name
        ));
    }
    Ok(row.with_repo(&own.repo))
}

pub fn pick_bin(app: &App, bins: &[String]) -> Option<String> {
    std::iter::once(&app.bin)
        .chain(&app.was)
        .find(|b| bins.contains(b))
        .cloned()
}

fn adapter_bin(name: &str, app: &App, manifest: &Path) -> Result<String, String> {
    let out = Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()
        .map_err(|e| format!("cargo: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo could not read {}", manifest.display()));
    }
    let meta: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))?;
    let bins: Vec<String> = meta["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["name"] == app.package.as_str())
        .flat_map(|p| p["targets"].as_array().cloned().unwrap_or_default())
        .filter(|t| {
            t["kind"]
                .as_array()
                .is_some_and(|k| k.iter().any(|k| k == "bin"))
        })
        .filter_map(|t| t["name"].as_str().map(String::from))
        .collect();
    pick_bin(app, &bins).ok_or_else(|| {
        let names: Vec<&str> = std::iter::once(app.bin.as_str())
            .chain(app.was.iter().map(String::as_str))
            .collect();
        format!(
            "{name}'s package {} has no adapter named {}",
            app.package,
            names.join(" or ")
        )
    })
}

fn config() -> Result<Config, String> {
    toml::from_str(CONFIG).map_err(|e| format!("render.toml: {e}"))
}

pub fn pin() -> Result<Pin, String> {
    pfx_run::ffmpeg::pin()
}

const STAMP: &str = ".used";
const PRUNE_DAYS: u64 = 30;

pub fn cache() -> Result<PathBuf, String> {
    pfx_run::ffmpeg::cache()
}

pub fn touch(entry: &Path) {
    if entry.is_dir() {
        let _ = std::fs::write(entry.join(STAMP), b"");
    }
}

pub fn size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(t) if t.is_file() => total += entry.metadata().map(|m| m.len()).unwrap_or(0),
                _ => {}
            }
        }
    }
    total
}

pub fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

fn grew(cache: &Path) {
    eprintln!(
        "{}: the cache at {} now holds {} ({} cache --prune drops what went unused for {PRUNE_DAYS} days)",
        pfx_run::job::TOOL,
        cache.display(),
        gigabytes(size(cache)),
        pfx_run::job::TOOL,
    );
}

fn entries(cache: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for group in ["ffmpeg", "adapters", "src"] {
        if let Ok(dir) = std::fs::read_dir(cache.join(group)) {
            out.extend(dir.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
        }
    }
    if let Ok(dir) = std::fs::read_dir(cache) {
        out.extend(dir.flatten().map(|e| e.path()).filter(|p| {
            p.is_dir()
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("target"))
        }));
    }
    out
}

pub fn prune(cache: &Path, now: std::time::SystemTime) -> Result<(usize, u64), String> {
    let limit = std::time::Duration::from_secs(PRUNE_DAYS * 86_400);
    let mut removed = (0, 0);
    for entry in entries(cache) {
        let stamp = entry.join(STAMP);
        let used = std::fs::metadata(&stamp)
            .or_else(|_| std::fs::metadata(&entry))
            .and_then(|m| m.modified())
            .map_err(|e| format!("{}: {e}", entry.display()))?;
        if now.duration_since(used).unwrap_or_default() > limit {
            let bytes = size(&entry);
            std::fs::remove_dir_all(&entry).map_err(|e| format!("{}: {e}", entry.display()))?;
            removed.0 += 1;
            removed.1 += bytes;
        }
    }
    Ok(removed)
}

pub fn caller_tmp() -> Result<PathBuf, String> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{} runs inside a git tree and writes only under that tree's tmp/",
            pfx_run::job::TOOL
        ));
    }
    let root = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let tmp = root.join("tmp");
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    tmp.canonicalize()
        .map_err(|e| format!("{}: {e}", tmp.display()))
}

pub fn inside(tmp: &Path, path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let mut existing = absolute.as_path();
    let mut rest = Vec::new();
    while !existing.exists() {
        rest.push(
            existing
                .file_name()
                .ok_or_else(|| format!("{} names a folder through '..'", path.display()))?,
        );
        existing = existing
            .parent()
            .ok_or_else(|| format!("{} has no existing parent", path.display()))?;
    }
    let mut full = existing
        .canonicalize()
        .map_err(|e| format!("{}: {e}", existing.display()))?;
    for part in rest.iter().rev() {
        full.push(part);
    }
    if !full.starts_with(tmp) || full == tmp {
        return Err(format!(
            "{} is outside the caller's tmp/ ({}); {} writes nowhere else",
            full.display(),
            tmp.display(),
            pfx_run::job::TOOL,
        ));
    }
    std::fs::create_dir_all(&full).map_err(|e| format!("{}: {e}", full.display()))?;
    Ok(full)
}

pub fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn hash_tree(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                let name = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(name, hash_file(&path)?);
            }
        }
    }
    Ok(out)
}

pub fn ffmpeg(tools: &Path) -> Result<(PathBuf, Pin), String> {
    pfx_run::ffmpeg::ffmpeg(tools, pfx_run::job::TOOL)
}

pub struct Built {
    pub path: PathBuf,
    pub source: String,
    pub commit: String,
    pub libs: Option<PathBuf>,
}

fn safe(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

struct Lock(PathBuf);

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn lock(path: PathBuf) -> Result<Lock, String> {
    let started = std::time::Instant::now();
    let limit = std::time::Duration::from_secs(4 * 3600);
    loop {
        if std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .is_ok()
        {
            return Ok(Lock(path));
        }
        let stale = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > limit);
        if stale {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        if started.elapsed() > limit {
            return Err(format!("{} stayed locked", path.display()));
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

fn git(dir: Option<&Path>, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    if let Some(d) = dir {
        cmd.arg("-C").arg(d);
    }
    let out = cmd.args(args).output().map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn shared_library(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "so" | "dylib" | "dll"))
        || p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.contains(".so."))
}

fn target_for_dir(name: &str, dir: &Path, tools: &Path) -> Result<PathBuf, String> {
    let canonical = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let digest = Sha256::digest(canonical.as_os_str().as_encoded_bytes());
    Ok(tools.join(format!("target-{name}-{}", &hex::encode(digest)[..12])))
}

fn store_adapter(tools: &Path, release: &Path, bin: &str) -> Result<(PathBuf, PathBuf), String> {
    let source = release.join(exe(bin));
    let digest = hash_file(&source)?;
    let adapters = tools.join("adapters");
    std::fs::create_dir_all(&adapters).map_err(|e| format!("{}: {e}", adapters.display()))?;
    let _guard = lock(adapters.join(format!(".{digest}.lock")))?;
    let root = adapters.join(&digest);
    let path = root.join(exe(bin));
    if !root.join(".complete").is_file() || !path.is_file() {
        if root.exists() {
            std::fs::remove_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
        }
        let stage = adapters.join(format!(".{digest}.{}.stage", std::process::id()));
        if stage.exists() {
            std::fs::remove_dir_all(&stage).map_err(|e| format!("{}: {e}", stage.display()))?;
        }
        std::fs::create_dir(&stage).map_err(|e| format!("{}: {e}", stage.display()))?;
        std::fs::copy(&source, stage.join(exe(bin)))
            .map_err(|e| format!("{}: {e}", source.display()))?;
        let libs = stage.join("libs");
        for entry in
            std::fs::read_dir(release).map_err(|e| format!("{}: {e}", release.display()))?
        {
            let entry = entry.map_err(|e| e.to_string())?;
            let file = entry.path();
            if file.is_file() && shared_library(&file) {
                std::fs::create_dir_all(&libs).map_err(|e| format!("{}: {e}", libs.display()))?;
                std::fs::copy(&file, libs.join(entry.file_name()))
                    .map_err(|e| format!("{}: {e}", file.display()))?;
            }
        }
        std::fs::write(stage.join(".complete"), b"").map_err(|e| e.to_string())?;
        std::fs::rename(&stage, &root).map_err(|e| format!("{}: {e}", root.display()))?;
    }
    touch(&root);
    Ok((path, root.join("libs")))
}

fn resolve(src: &Path, reference: &str) -> Result<(String, String), String> {
    for (spec, via) in [
        (
            format!("refs/remotes/origin/{reference}^{{commit}}"),
            format!("origin/{reference}"),
        ),
        (
            format!("refs/tags/{reference}^{{commit}}"),
            format!("tag {reference}"),
        ),
        (format!("{reference}^{{commit}}"), "commit".to_string()),
    ] {
        if let Ok(commit) = git(Some(src), &["rev-parse", "--verify", "--quiet", &spec]) {
            return Ok((commit, via));
        }
    }
    Err(format!("no revision '{reference}'"))
}

pub struct Checkout {
    pub src: PathBuf,
    pub repo_key: String,
    pub commit: String,
    pub pinned: bool,
    _guard: Lock,
}

pub fn checkout(name: &str, repo: &str, reference: &str, tools: &Path) -> Result<Checkout, String> {
    let repo_key = safe(
        repo.rsplit('/')
            .next()
            .unwrap_or(name)
            .trim_end_matches(".git"),
    );
    let sources = tools.join("src");
    std::fs::create_dir_all(&sources).map_err(|e| format!("{}: {e}", sources.display()))?;
    let guard = lock(sources.join(format!("{name}-{repo_key}.lock")))?;
    let src = sources.join(format!("{name}-{repo_key}"));
    if !src.join(".git").is_dir() {
        eprintln!("{}: cloning {repo}", pfx_run::job::TOOL);
        git(None, &["clone", "--quiet", repo, &src.to_string_lossy()])?;
    } else {
        git(Some(&src), &["remote", "set-url", "origin", repo])?;
    }
    git(
        Some(&src),
        &["fetch", "--quiet", "--tags", "--force", "origin"],
    )?;
    let _ = git(Some(&src), &["fetch", "--quiet", "origin", reference]);
    let (commit, via) =
        resolve(&src, reference).map_err(|_| format!("{repo} has no revision '{reference}'"))?;
    eprintln!(
        "{}: --rev {reference} resolved to {commit} ({via})",
        pfx_run::job::TOOL
    );
    touch(&src);
    Ok(Checkout {
        src,
        repo_key,
        commit,
        pinned: !via.starts_with("origin/"),
        _guard: guard,
    })
}

fn git_raw(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

fn blob(dir: &Path, spec: &str) -> Result<Vec<u8>, String> {
    git_raw(dir, &["cat-file", "blob", spec])
}

fn row_at(src: &Path, commit: &str) -> Result<Option<Row>, String> {
    match blob(src, &format!("{commit}:{ENGINE_FILE}")) {
        Ok(bytes) => own_row(
            &String::from_utf8_lossy(&bytes),
            &format!("{ENGINE_FILE} at {}", &commit[..12.min(commit.len())]),
        )
        .map(Some),
        Err(_) => Ok(None),
    }
}

pub fn build_from_git(
    name: &str,
    reference: &str,
    tools: &Path,
    own: &OwnApp,
) -> Result<Built, String> {
    let source = own.dir.display().to_string();
    let Checkout {
        src,
        repo_key,
        commit,
        _guard,
        ..
    } = checkout(name, &source, reference, tools)?;
    let app = row_or(
        name,
        row_at(&src, &commit)?,
        own,
        &format!("{} at {reference}", own.repo),
    )?;
    let root = tools
        .join("adapters")
        .join(format!("{name}-{repo_key}-{}-src", &commit[..12]));
    let target = tools.join(format!("target-{name}"));
    std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    let _target_guard = lock(target.join(".lock"))?;
    let index = root.join("sha256");
    let cached = std::fs::read_to_string(&index)
        .ok()
        .map(|s| s.trim().to_string());
    let cached_path = cached
        .as_ref()
        .map(|s| tools.join("adapters").join(s).join(exe(&app.bin)));
    let (path, libs) = if let Some(path) = cached_path.filter(|p| p.is_file()) {
        touch(&root);
        touch(&target);
        let libs = path.parent().unwrap().join("libs");
        (path, libs)
    } else {
        eprintln!(
            "{}: building {name}'s adapter at {reference} ({})",
            pfx_run::job::TOOL,
            &commit[..12]
        );
        git(
            Some(&src),
            &["checkout", "--quiet", "--force", "--detach", &commit],
        )?;
        let built = adapter_bin(name, &app, &src.join("Cargo.toml"))?;
        let mut cargo = Command::new("cargo");
        let lock = std::fs::read_to_string(src.join("Cargo.lock")).unwrap_or_default();
        let moved = rewrites(&lock, &config()?.moved);
        if !moved.is_empty() {
            cargo.env("GIT_CONFIG_COUNT", moved.len().to_string());
            for (i, (key, value)) in moved.iter().enumerate() {
                cargo
                    .env(format!("GIT_CONFIG_KEY_{i}"), key)
                    .env(format!("GIT_CONFIG_VALUE_{i}"), value);
            }
        }
        let status = cargo
            .env("CARGO_NET_GIT_FETCH_WITH_CLI", "true")
            .args(["build", "--locked", "--release", "--manifest-path"])
            .arg(src.join("Cargo.toml"))
            .args(["-p", &app.package, "--bin", &built, "--target-dir"])
            .arg(&target)
            .status()
            .map_err(|e| format!("cargo: {e}"))?;
        if !status.success() {
            return Err(format!(
                "cargo could not build {name}'s adapter at {reference}"
            ));
        }
        let (path, libs) = store_adapter(tools, &target.join("release"), &built)?;
        std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
        std::fs::write(&index, hash_file(&path)?).map_err(|e| e.to_string())?;
        std::fs::write(root.join("commit"), &commit).map_err(|e| e.to_string())?;
        touch(&root);
        touch(&target);
        grew(tools);
        (path, libs)
    };
    Ok(Built {
        path,
        source: format!("{}@{reference}", app.repo),
        commit,
        libs: libs.is_dir().then_some(libs),
    })
}

pub fn build_from_dir(name: &str, dir: &Path, tools: &Path, own: &OwnApp) -> Result<Built, String> {
    let canonical = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let home = own
        .dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", own.dir.display()))?;
    let top = git(Some(&canonical), &["rev-parse", "--show-toplevel"])
        .ok()
        .and_then(|top| PathBuf::from(top).canonicalize().ok());
    if canonical != home && top.as_deref() != Some(home.as_path()) {
        return Err(format!(
            "--app-dir {} is not {name}'s own tree ({}): an app renders only itself",
            canonical.display(),
            home.display()
        ));
    }
    let app = row_or(
        name,
        own_row_in(&canonical)?,
        own,
        &canonical.display().to_string(),
    )?;
    let target = target_for_dir(name, &canonical, tools)?;
    std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    let _guard = lock(target.join(".lock"))?;
    eprintln!(
        "{}: building {name}'s adapter from {}",
        pfx_run::job::TOOL,
        canonical.display()
    );
    let built = adapter_bin(name, &app, &canonical.join("Cargo.toml"))?;
    let status = Command::new("cargo")
        .env("CARGO_NET_GIT_FETCH_WITH_CLI", "true")
        .args(["build", "--locked", "--release", "--manifest-path"])
        .arg(canonical.join("Cargo.toml"))
        .args(["-p", &app.package, "--bin", &built, "--target-dir"])
        .arg(&target)
        .status()
        .map_err(|e| format!("cargo: {e}"))?;
    if !status.success() {
        return Err(format!(
            "cargo could not build {name}'s adapter in {}",
            canonical.display()
        ));
    }
    let (path, libs) = store_adapter(tools, &target.join("release"), &built)?;
    let commit = revision_from_dir(&canonical)?;
    touch(&target);
    Ok(Built {
        path,
        source: canonical.display().to_string(),
        commit,
        libs: libs.is_dir().then_some(libs),
    })
}

pub fn revision_from_dir(dir: &Path) -> Result<String, String> {
    let head = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let commit = String::from_utf8_lossy(&head.stdout).trim().to_string();
    if !head.status.success() || commit.len() < 12 || !commit.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(format!(
            "{} has no Git HEAD for the archive version",
            dir.display()
        ));
    }
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["status", "--porcelain"])
        .output()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    if !status.status.success() {
        return Err(format!("{}: git status failed", dir.display()));
    }
    if status.stdout.is_empty() {
        Ok(commit)
    } else {
        Ok(format!("{commit}+dirty"))
    }
}

#[cfg(test)]
mod tests {
    fn run(dir: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "tag.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    fn a_branch_resolves_to_its_fetched_remote_tip_not_the_clone_s_stale_branch() {
        let base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("resolve-{}", std::process::id()));
        let origin = base.join("origin");
        let clone = base.join("clone");
        std::fs::create_dir_all(&origin).unwrap();
        run(&origin, &["init", "--quiet"]);
        run(
            &origin,
            &["commit", "--quiet", "--allow-empty", "-m", "one"],
        );
        run(&origin, &["tag", "v1.0.0"]);
        run(
            &base,
            &[
                "clone",
                "--quiet",
                origin.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        );
        run(
            &origin,
            &["commit", "--quiet", "--allow-empty", "-m", "two"],
        );
        let tip = run(&origin, &["rev-parse", "HEAD"]);
        run(&clone, &["fetch", "--quiet", "--tags", "--force", "origin"]);
        let (commit, via) = super::resolve(&clone, "main").unwrap();
        assert_eq!(commit, tip);
        assert_eq!(via, "origin/main");
        let (tagged, via) = super::resolve(&clone, "v1.0.0").unwrap();
        assert_eq!(tagged, run(&origin, &["rev-parse", "v1.0.0^{commit}"]));
        assert_eq!(via, "tag v1.0.0");
        let (exact, via) = super::resolve(&clone, &tip[..12]).unwrap();
        assert_eq!(exact, tip);
        assert_eq!(via, "commit");
        assert!(super::resolve(&clone, "nowhere").is_err());
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn a_dependency_that_moved_repo_is_fetched_from_its_new_home_only_where_the_lock_pins_the_old_one()
     {
        let moved = vec![super::Moved {
            package: "engine".into(),
            from: "ssh://git@github.com/gmrdad82/pito-engine.git".into(),
            to: "ssh://git@github.com/gmrdad82/pito-search.git".into(),
        }];
        let old = r#"
[[package]]
name = "engine"
version = "0.1.0"
source = "git+ssh://git@github.com/gmrdad82/pito-engine.git?tag=v0.1.0#3c4bd597aae3c89a20516489985c63737abb5140"
"#;
        assert_eq!(
            super::rewrites(old, &moved),
            vec![(
                "url.ssh://git@github.com/gmrdad82/pito-search.git.insteadOf".to_string(),
                "ssh://git@github.com/gmrdad82/pito-engine.git".to_string()
            )]
        );
        let new = r#"
[[package]]
name = "engine"
version = "0.1.0"
source = "git+ssh://git@github.com/gmrdad82/pito-search.git?tag=v0.1.0#3c4bd597aae3c89a20516489985c63737abb5140"

[[package]]
name = "pito-engine"
version = "0.7.0"
source = "git+ssh://git@github.com/gmrdad82/pito-engine.git?tag=v0.7.0#1111111111111111111111111111111111111111"
"#;
        assert!(super::rewrites(new, &moved).is_empty());
        assert!(super::rewrites("not toml [", &moved).is_empty());
    }

    #[test]
    fn only_the_old_search_engine_moves_and_the_render_engines_old_pins_keep_their_url() {
        let moved = super::config().unwrap().moved;
        let renamed = r#"
[[package]]
name = "pito-engine"
version = "0.20.1"
source = "git+ssh://git@github.com/gmrdad82/pito-engine.git?tag=v0.20.1#1111111111111111111111111111111111111111"
"#;
        assert!(super::rewrites(renamed, &moved).is_empty());
        let meaning = r#"
[[package]]
name = "engine"
version = "0.1.0"
source = "git+ssh://git@github.com/gmrdad82/pito-engine.git?tag=v0.1.0#3c4bd597aae3c89a20516489985c63737abb5140"
"#;
        assert_eq!(
            super::rewrites(meaning, &moved),
            vec![(
                "url.ssh://git@github.com/gmrdad82/pito-search.git.insteadOf".to_string(),
                "ssh://git@github.com/gmrdad82/pito-engine.git".to_string()
            )]
        );
        let current = r#"
[[package]]
name = "pfx"
version = "0.21.0"
source = "git+ssh://git@github.com/gmrdad82/pfx.git?tag=v0.21.0#edd84808a6a906c5cb544aa513235944de9a7703"
"#;
        assert!(super::rewrites(current, &moved).is_empty());
    }

    use super::*;

    #[test]
    fn the_config_pins_ffmpeg_for_linux_and_only_keys_the_mirror_holds() {
        let pins = pfx_run::ffmpeg::pins().unwrap();
        assert!(pins.contains_key("linux-x86_64"));
        for pin in pins.values() {
            assert!(pin.url.starts_with("https://deps.pitomd.com/ffmpeg/"));
            assert_eq!(pin.sha256.len(), 64);
            assert!(pin.size > 0);
        }
        assert!(!CONFIG.contains("[apps."));
    }

    fn sample_app() -> App {
        App {
            repo: "file:///repos/sample.git".into(),
            package: "sample-gui".into(),
            bin: "psample-render".into(),
            was: vec!["sample-render".into()],
        }
    }

    const SAMPLE_ROW: &str = "[app]\nname = \"sample\"\npackage = \"sample-gui\"\nbin = \"psample-render\"\nwas = [\"sample-render\"]\n";

    #[test]
    fn an_adapter_is_found_by_its_new_name_or_the_one_it_had_at_that_rev() {
        let sample = sample_app();
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            pick_bin(&sample, &names(&["sample", "psample-render"])).as_deref(),
            Some("psample-render")
        );
        assert_eq!(
            pick_bin(&sample, &names(&["sample", "sample-render"])).as_deref(),
            Some("sample-render")
        );
        assert_eq!(
            pick_bin(&sample, &names(&["sample-render", "psample-render"])).as_deref(),
            Some("psample-render")
        );
        assert_eq!(pick_bin(&sample, &names(&["sample"])), None);
    }

    #[test]
    fn an_app_renders_only_itself_from_its_own_tree() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("own-{}", std::process::id()));
        let caller = base.join("caller");
        std::fs::create_dir_all(caller.join("render")).unwrap();
        run(&caller, &["init", "--quiet"]);
        let bare = own_app("sample", Some(&caller)).unwrap_err();
        assert!(
            bare.contains(ENGINE_FILE) && bare.contains("only itself"),
            "{bare}"
        );
        std::fs::write(caller.join(ENGINE_FILE), SAMPLE_ROW).unwrap();
        let own = own_app("sample", Some(&caller)).unwrap();
        assert_eq!(own.row.package, "sample-gui");
        assert_eq!(own.repo, caller.display().to_string());
        run(
            &caller,
            &["remote", "add", "origin", "file:///repos/sample.git"],
        );
        assert_eq!(
            own_app("sample", Some(&caller)).unwrap().repo,
            "file:///repos/sample.git"
        );
        let other = own_app("other", Some(&caller)).unwrap_err();
        assert!(
            other.contains("names app 'sample'") && other.contains("only itself"),
            "{other}"
        );
        assert!(own_app("sample", None).unwrap_err().contains("only itself"));
        let elsewhere = base.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let refused = build_from_dir("sample", &elsewhere, &base.join("cache"), &own)
            .err()
            .unwrap();
        assert!(refused.contains("not sample's own tree"), "{refused}");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn a_rev_checks_out_the_apps_own_repository_and_older_revs_use_its_current_row() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("rows-{}", std::process::id()));
        let origin = base.join("sample");
        std::fs::create_dir_all(origin.join("render")).unwrap();
        run(&origin, &["init", "--quiet"]);
        std::fs::write(origin.join("readme.txt"), "before").unwrap();
        run(&origin, &["add", "readme.txt"]);
        run(&origin, &["commit", "--quiet", "-m", "old"]);
        run(&origin, &["tag", "v1.0.0"]);
        std::fs::write(
            origin.join(ENGINE_FILE),
            SAMPLE_ROW.replace("psample-render", "psample-new"),
        )
        .unwrap();
        run(&origin, &["add", ENGINE_FILE]);
        run(&origin, &["commit", "--quiet", "-m", "own row"]);
        run(&origin, &["tag", "v2.0.0"]);
        std::fs::write(origin.join(ENGINE_FILE), SAMPLE_ROW).unwrap();
        let cache = base.join("cache");
        let own = own_app("sample", Some(&origin)).unwrap();
        let source = origin.display().to_string();
        let at = |rev: &str| {
            let out = checkout("sample", &source, rev, &cache).unwrap();
            assert!(out.pinned);
            let row = row_at(&out.src, &out.commit).unwrap();
            row_or("sample", row, &own, rev)
        };
        let old = at("v1.0.0").unwrap();
        assert_eq!(old.bin, "psample-render");
        assert_eq!(old.repo, source);
        let new = at("v2.0.0").unwrap();
        assert_eq!(new.bin, "psample-new");
        assert_eq!(new.was, ["sample-render"]);
        assert!(
            row_or("other", own_row(SAMPLE_ROW, "x").ok(), &own, "v2")
                .unwrap_err()
                .contains("names app 'sample'")
        );
        let tip = checkout("sample", &source, "main", &cache).unwrap();
        assert!(!tip.pinned);
        drop(tip);
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn nothing_is_written_outside_the_callers_tmp() {
        let tmp = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        let tmp = tmp.canonicalize().unwrap();
        assert!(inside(&tmp, &tmp.join("render-test")).is_ok());
        let cache = tmp.join(format!("cache-test-{}", std::process::id()));
        let (old, fresh, target) = (
            cache.join("ffmpeg/aaa"),
            cache.join("adapters/alpha-x"),
            cache.join("target"),
        );
        for d in [&old, &fresh, &target] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join("blob"), vec![0u8; 1000]).unwrap();
            touch(d);
        }
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(31 * 86_400);
        touch(&fresh);
        std::fs::File::options()
            .write(true)
            .open(fresh.join(STAMP))
            .unwrap()
            .set_modified(later)
            .unwrap();
        let (count, bytes) = prune(&cache, later).unwrap();
        assert_eq!(count, 2);
        assert!(bytes >= 2000);
        assert!(fresh.is_dir() && !old.exists() && !target.exists());
        std::fs::remove_dir_all(&cache).ok();
        assert!(inside(&tmp, &tmp).is_err());
        assert!(inside(&tmp, &tmp.join("..").join("src")).is_err());
        let stray = tmp.join("..").join(format!("stray-{}", std::process::id()));
        assert!(inside(&tmp, &stray.join("deep")).is_err());
        assert!(!stray.exists());
    }

    #[test]
    fn concurrent_source_trees_keep_their_own_immutable_adapters() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("adapter-build-{}", std::process::id()));
        let cache = base.join("cache");
        let mut sources = Vec::new();
        for (name, marker) in [("a", "first"), ("b", "second")] {
            let dir = base.join(name);
            std::fs::create_dir_all(dir.join("src")).unwrap();
            std::fs::create_dir_all(dir.join("render")).unwrap();
            std::fs::write(dir.join(ENGINE_FILE), SAMPLE_ROW).unwrap();
            std::fs::write(
                dir.join("Cargo.toml"),
                "[package]\nname = \"sample-gui\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[workspace]\n[[bin]]\nname = \"psample-render\"\npath = \"src/main.rs\"\n",
            )
            .unwrap();
            std::fs::write(
                dir.join("src/main.rs"),
                format!("fn main() {{ print!(\"{marker}\"); }}\n"),
            )
            .unwrap();
            let status = Command::new("cargo")
                .arg("generate-lockfile")
                .current_dir(&dir)
                .status()
                .unwrap();
            assert!(status.success());
            sources.push(dir);
        }
        assert_ne!(
            target_for_dir("sample", &sources[0], &cache).unwrap(),
            target_for_dir("sample", &sources[1], &cache).unwrap()
        );
        let tasks: Vec<_> = sources
            .into_iter()
            .map(|source| {
                let cache = cache.clone();
                std::thread::spawn(move || {
                    let own = OwnApp {
                        dir: source.clone(),
                        row: own_row(SAMPLE_ROW, "test").unwrap(),
                        repo: source.display().to_string(),
                    };
                    build_from_dir("sample", &source, &cache, &own).unwrap()
                })
            })
            .collect();
        let built: Vec<_> = tasks.into_iter().map(|task| task.join().unwrap()).collect();
        assert_ne!(built[0].path, built[1].path);
        for (adapter, marker) in built.iter().zip(["first", "second"]) {
            let digest = hash_file(&adapter.path).unwrap();
            assert_eq!(
                adapter.path.parent().unwrap().file_name().unwrap(),
                digest.as_str()
            );
            let output = Command::new(&adapter.path).output().unwrap();
            assert!(output.status.success());
            assert_eq!(output.stdout, marker.as_bytes());
        }
        std::fs::remove_dir_all(base).unwrap();
    }
}
