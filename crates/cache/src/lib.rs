use pfx_core::atomic;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

pub const VARIABLE: &str = "PFX_CACHE_DIR";
pub const CAP_VARIABLE: &str = "PFX_CACHE_CAP";
pub const DEFAULT_CAP: u64 = 4_000_000_000;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MAGIC: [u8; 8] = *b"PFXCACH1";
pub const KINDS: &[&str] = &["pipelines", "bakes"];
pub const MARKER: &str = "CACHEDIR.TAG";

const FOLDER: &str = "pfx";
pub const OFF: &str = "off";
const DIR_MAGIC: [u8; 8] = *b"PFXDIR01";
const HEADER: usize = MAGIC.len() + 8 + 32;
const ABANDONED: Duration = Duration::from_secs(3600);
const TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n# This file is a cache directory tag created by pfx.\n";
const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
const ATTEMPTS: usize = 16;

static NAMES: AtomicU64 = AtomicU64::new(0);
static SHARED: OnceLock<Store> = OnceLock::new();

pub fn share(store: Store) -> bool {
    SHARED.set(store).is_ok()
}

pub fn shared() -> Option<&'static Store> {
    SHARED.get()
}

fn unique() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        NAMES.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key([u8; 32]);

impl Key {
    pub fn hex(&self) -> String {
        hex::encode(self.0)
    }
}

pub struct Keyer(Sha256);

impl Keyer {
    pub fn new(kind: &str, format: u32) -> Self {
        Keyer(Sha256::new())
            .part(MAGIC)
            .part(kind)
            .part(format.to_le_bytes())
            .part(VERSION)
    }

    pub fn part(mut self, bytes: impl AsRef<[u8]>) -> Self {
        let bytes = bytes.as_ref();
        self.0.update((bytes.len() as u64).to_le_bytes());
        self.0.update(bytes);
        self
    }

    pub fn finish(self) -> Key {
        Key(self.0.finalize().into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Hit,
    Made { stored: Result<(), String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KindInfo {
    pub kind: String,
    pub entries: usize,
    pub bytes: u64,
    pub oldest: Option<SystemTime>,
    pub newest: Option<SystemTime>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    pub root: PathBuf,
    pub cap: u64,
    pub entries: usize,
    pub bytes: u64,
    pub kinds: Vec<KindInfo>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cleaned {
    pub entries: usize,
    pub bytes: u64,
}

struct Entry {
    path: PathBuf,
    bytes: u64,
    used: SystemTime,
}

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
    cap: u64,
}

pub fn root() -> Result<PathBuf, String> {
    root_from(|name| std::env::var_os(name))
}

pub fn root_from(lookup: impl Fn(&str) -> Option<std::ffi::OsString>) -> Result<PathBuf, String> {
    let set = |name: &str| lookup(name).filter(|value| !value.is_empty());
    let absolute = |name: &str| set(name).map(PathBuf::from).filter(|dir| dir.is_absolute());
    if set(VARIABLE).is_some_and(|dir| dir == OFF) {
        return Err(format!("the machine cache is off ({VARIABLE}={OFF})"));
    }
    if let Some(dir) = set(VARIABLE) {
        let dir = PathBuf::from(dir);
        return if dir.is_absolute() {
            Ok(dir)
        } else {
            Err(format!(
                "{VARIABLE} must be an absolute path, not {}",
                dir.display()
            ))
        };
    }
    absolute("XDG_CACHE_HOME")
        .map(|dir| dir.join(FOLDER))
        .or_else(|| {
            cfg!(windows)
                .then(|| absolute("LOCALAPPDATA"))
                .flatten()
                .map(|dir| dir.join(FOLDER))
        })
        .or_else(|| absolute("HOME").map(|home| home.join(".cache").join(FOLDER)))
        .or_else(|| absolute("LOCALAPPDATA").map(|dir| dir.join(FOLDER)))
        .ok_or_else(|| format!("no absolute cache folder: set {VARIABLE}"))
}

pub fn set_by() -> Option<&'static str> {
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    set(VARIABLE).then_some(VARIABLE)
}

fn private(root: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if let Ok(meta) = fs::metadata(root) {
            if meta.uid() != unsafe { libc::geteuid() } {
                return Err(format!(
                    "{}: another user owns it, and the machine cache must be yours alone",
                    root.display()
                ));
            }
            if meta.permissions().mode() & 0o022 != 0 {
                return Err(format!(
                    "{}: other users can write in it, and the machine cache must be private to you (chmod 700)",
                    root.display()
                ));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = root;
    Ok(())
}

fn make_dirs(dir: &Path) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir).map_err(|e| shown(dir, e))
}

pub fn cap() -> Result<u64, String> {
    match std::env::var(CAP_VARIABLE) {
        Ok(text) if !text.trim().is_empty() => {
            parse_size(&text).map_err(|reason| format!("{CAP_VARIABLE}={text}: {reason}"))
        }
        _ => Ok(DEFAULT_CAP),
    }
}

pub fn parse_size(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let number: f64 = number
        .parse()
        .map_err(|_| "expected a size such as 4GB, 500MB or 2000000000".to_string())?;
    let scale = match unit.trim().to_ascii_uppercase().as_str() {
        "" | "B" => 1.0,
        "K" | "KB" => 1e3,
        "M" | "MB" => 1e6,
        "G" | "GB" => 1e9,
        "T" | "TB" => 1e12,
        other => return Err(format!("unknown unit '{other}': use B, KB, MB, GB or TB")),
    };
    Ok((number * scale).round() as u64)
}

pub fn parse_age(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    let split = text
        .find(|c: char| !c.is_ascii_digit())
        .ok_or("expected a number and a unit such as 30d, 12h, 90m or 45s")?;
    let (number, unit) = text.split_at(split);
    let number: u64 = number
        .parse()
        .map_err(|_| "expected a number and a unit such as 30d, 12h, 90m or 45s".to_string())?;
    let seconds = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        "w" => 7 * 86_400,
        other => return Err(format!("unknown unit '{other}': use s, m, h, d or w")),
    };
    Ok(Duration::from_secs(number.saturating_mul(seconds)))
}

fn shown(path: &Path, error: io::Error) -> String {
    format!("{}: {error}", path.display())
}

pub fn valid_kind(kind: &str) -> bool {
    !kind.is_empty()
        && kind
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn checked(kind: &str) -> Result<(), String> {
    if valid_kind(kind) {
        Ok(())
    } else {
        Err(format!(
            "cache kind '{kind}' is not lowercase letters, digits and dashes"
        ))
    }
}

fn lower_hex(name: &str, len: usize) -> bool {
    name.len() == len
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_entry_name(shard: &str, name: &str) -> bool {
    lower_hex(name, 64) && name.starts_with(shard)
}

fn header(payload: &[u8]) -> [u8; HEADER] {
    let mut out = [0u8; HEADER];
    out[..8].copy_from_slice(&MAGIC);
    out[8..16].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    out[16..].copy_from_slice(&Sha256::digest(payload));
    out
}

fn sized(head: &[u8], total: u64) -> Result<u64, String> {
    if head.len() < HEADER || total < HEADER as u64 {
        return Err("truncated".into());
    }
    if head[..8] != MAGIC {
        return Err("not a pfx cache entry".into());
    }
    let size = u64::from_le_bytes(head[8..16].try_into().expect("eight bytes"));
    let payload = total - HEADER as u64;
    if payload != size {
        return Err(format!("payload is {payload} bytes, header says {size}"));
    }
    Ok(size)
}

fn verified(head: &[u8], payload: &[u8]) -> Result<(), String> {
    if Sha256::digest(payload).as_slice() != &head[16..HEADER] {
        return Err("payload checksum does not match".into());
    }
    Ok(())
}

enum Unread {
    Damaged,
    Kept,
}

fn read_entry(file: &mut fs::File, cap: u64) -> Result<(Vec<u8>, [u8; 32]), Unread> {
    let total = file.metadata().map_err(|_| Unread::Kept)?.len();
    let mut head = [0u8; HEADER];
    file.read_exact(&mut head).map_err(|_| Unread::Damaged)?;
    let size = sized(&head, total).map_err(|_| Unread::Damaged)?;
    if size > cap {
        return Err(Unread::Kept);
    }
    let size = usize::try_from(size).map_err(|_| Unread::Kept)?;
    let mut payload = Vec::new();
    payload.try_reserve_exact(size).map_err(|_| Unread::Kept)?;
    file.take(size as u64)
        .read_to_end(&mut payload)
        .map_err(|_| Unread::Kept)?;
    if payload.len() != size {
        return Err(Unread::Damaged);
    }
    verified(&head, &payload).map_err(|_| Unread::Damaged)?;
    let stamp = head[16..].try_into().expect("thirty-two bytes");
    Ok((payload, stamp))
}

fn open_entry(path: &Path) -> Option<fs::File> {
    fs::symlink_metadata(path)
        .ok()
        .filter(|meta| meta.file_type().is_file())?;
    let mut options = fs::File::options();
    options.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(
        &mut options,
        libc::O_NONBLOCK | libc::O_NOFOLLOW,
    );
    let file = options.open(path).ok()?;
    file.metadata().ok().filter(|meta| meta.is_file())?;
    Some(file)
}

fn temp(dir: &Path, name: &str) -> Result<(PathBuf, fs::File), String> {
    for _ in 0..ATTEMPTS {
        let temp = dir.join(format!(".{name}.{}.partial", unique()));
        match fs::File::options().write(true).create_new(true).open(&temp) {
            Ok(file) => return Ok((temp, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(shown(&temp, e)),
        }
    }
    Err(format!(
        "{}: no free name for a partial entry",
        dir.display()
    ))
}

fn written(temp: &Path, mut file: fs::File, parts: &[&[u8]]) -> Result<(), String> {
    let done = parts.iter().try_for_each(|part| file.write_all(part));
    drop(file);
    done.map_err(|error| {
        let _ = fs::remove_file(temp);
        shown(temp, error)
    })
}

fn children(dir: &Path) -> Vec<fs::DirEntry> {
    let mut out: Vec<fs::DirEntry> = fs::read_dir(dir)
        .map(|entries| entries.flatten().collect())
        .unwrap_or_default();
    out.sort_by_key(|entry| entry.file_name());
    out
}

fn touch(path: &Path, at: SystemTime) {
    if let Ok(file) = fs::File::options().write(true).open(path) {
        let _ = file.set_modified(at);
    }
}

impl Store {
    pub fn open() -> Result<Self, String> {
        let root = root()?;
        private(&root)?;
        Ok(Self::at(root).with_cap(cap()?))
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            cap: DEFAULT_CAP,
        }
    }

    pub fn with_cap(mut self, cap: u64) -> Self {
        self.cap = cap;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn cap(&self) -> u64 {
        self.cap
    }

    pub fn path(&self, kind: &str, key: &Key) -> PathBuf {
        let hex = key.hex();
        self.root.join(kind).join(&hex[..2]).join(hex)
    }

    pub fn get(&self, kind: &str, key: &Key) -> Option<Vec<u8>> {
        self.get_stamped(kind, key).map(|(payload, _)| payload)
    }

    pub fn get_stamped(&self, kind: &str, key: &Key) -> Option<(Vec<u8>, [u8; 32])> {
        if !valid_kind(kind) {
            return None;
        }
        let path = self.path(kind, key);
        let mut file = open_entry(&path)?;
        let read = read_entry(&mut file, self.cap);
        drop(file);
        match read {
            Ok(found) => {
                touch(&path, SystemTime::now());
                Some(found)
            }
            Err(Unread::Damaged) => {
                let _ = fs::remove_file(&path);
                None
            }
            Err(Unread::Kept) => None,
        }
    }

    pub fn stamp(&self, kind: &str, key: &Key) -> Option<[u8; 32]> {
        if !valid_kind(kind) {
            return None;
        }
        let mut file = open_entry(&self.path(kind, key))?;
        let total = file.metadata().ok()?.len();
        let mut head = [0u8; HEADER];
        file.read_exact(&mut head).ok()?;
        sized(&head, total).ok()?;
        head[16..].try_into().ok()
    }

    pub fn put(&self, kind: &str, key: &Key, payload: &[u8]) -> Result<(), String> {
        checked(kind)?;
        let size = HEADER as u64 + payload.len() as u64;
        if size > self.cap {
            return Err(format!(
                "a {kind} entry of {size} bytes is over the cache's cap of {} ({CAP_VARIABLE})",
                self.cap
            ));
        }
        make_dirs(&self.root)?;
        private(&self.root)?;
        self.mark()?;
        let path = self.path(kind, key);
        let dir = path.parent().expect("an entry sits in a folder");
        make_dirs(dir)?;
        let (temp, file) = temp(dir, &key.hex())?;
        written(&temp, file, &[&header(payload), payload])?;
        atomic::commit(&temp, &path)?;
        self.evict_sparing(Some(&path)).map(|_| ())
    }

    fn marked(&self) -> bool {
        self.root.join(MARKER).is_file()
    }

    fn mark(&self) -> Result<(), String> {
        if self.marked() {
            return Ok(());
        }
        make_dirs(&self.root)?;
        let (temp, file) = temp(&self.root, MARKER)?;
        written(&temp, file, &[TAG.as_bytes()])?;
        atomic::commit(&temp, &self.root.join(MARKER))
    }

    pub fn get_or_make<E: From<String>>(
        &self,
        kind: &str,
        key: &Key,
        make: impl FnOnce() -> Result<Vec<u8>, E>,
    ) -> Result<(Vec<u8>, Source), E> {
        checked(kind)?;
        if let Some(bytes) = self.get(kind, key) {
            return Ok((bytes, Source::Hit));
        }
        let bytes = make()?;
        let stored = self.put(kind, key, &bytes);
        Ok((bytes, Source::Made { stored }))
    }

    fn kinds(&self) -> Vec<String> {
        if !self.marked() {
            return Vec::new();
        }
        children(&self.root)
            .into_iter()
            .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| valid_kind(name))
            .collect()
    }

    fn entries(&self, kind: &str, now: SystemTime, partials: &mut Vec<PathBuf>) -> Vec<Entry> {
        let mut out = Vec::new();
        for shard in children(&self.root.join(kind)) {
            let prefix = shard.file_name().to_string_lossy().into_owned();
            if !lower_hex(&prefix, 2) || !shard.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            for file in children(&shard.path()) {
                let name = file.file_name().to_string_lossy().into_owned();
                let Ok(meta) = file.metadata() else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                let used = meta.modified().unwrap_or(now);
                if is_entry_name(&prefix, &name) {
                    out.push(Entry {
                        path: file.path(),
                        bytes: meta.len(),
                        used,
                    });
                } else if atomic::is_partial(&name)
                    && now.duration_since(used).unwrap_or_default() > ABANDONED
                {
                    partials.push(file.path());
                }
            }
        }
        out
    }

    fn scan(&self, kind: Option<&str>, now: SystemTime) -> (Vec<Entry>, Vec<PathBuf>) {
        let mut partials = Vec::new();
        let kinds = match kind {
            Some(_) if !self.marked() => Vec::new(),
            Some(kind) => vec![kind.to_string()],
            None => self.kinds(),
        };
        let entries = kinds
            .iter()
            .flat_map(|kind| self.entries(kind, now, &mut partials))
            .collect();
        (entries, partials)
    }

    pub fn info(&self) -> Info {
        let now = SystemTime::now();
        let mut kinds: Vec<KindInfo> = KINDS
            .iter()
            .map(|kind| kind.to_string())
            .chain(self.kinds())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|kind| {
                let (entries, _) = self.scan(Some(&kind), now);
                KindInfo {
                    entries: entries.len(),
                    bytes: entries.iter().map(|e| e.bytes).sum(),
                    oldest: entries.iter().map(|e| e.used).min(),
                    newest: entries.iter().map(|e| e.used).max(),
                    kind,
                }
            })
            .collect();
        kinds.sort_by(|a, b| a.kind.cmp(&b.kind));
        Info {
            root: self.root.clone(),
            cap: self.cap,
            entries: kinds.iter().map(|k| k.entries).sum(),
            bytes: kinds.iter().map(|k| k.bytes).sum(),
            kinds,
        }
    }

    fn remove(entries: impl IntoIterator<Item = Entry>, partials: Vec<PathBuf>) -> Cleaned {
        let mut done = Cleaned::default();
        for entry in entries {
            if fs::remove_file(&entry.path).is_ok() {
                done.entries += 1;
                done.bytes += entry.bytes;
            }
        }
        for partial in partials {
            let _ = fs::remove_file(partial);
        }
        done
    }

    pub fn clean(
        &self,
        kind: Option<&str>,
        older_than: Option<Duration>,
        now: SystemTime,
    ) -> Result<Cleaned, String> {
        if let Some(kind) = kind {
            checked(kind)?;
        }
        let (entries, partials) = self.scan(kind, now);
        let stale = entries.into_iter().filter(|entry| {
            older_than.is_none_or(|age| now.duration_since(entry.used).unwrap_or_default() > age)
        });
        Ok(Self::remove(stale, partials))
    }

    pub fn evict(&self) -> Result<Cleaned, String> {
        self.evict_sparing(None)
    }

    fn evict_sparing(&self, spared: Option<&Path>) -> Result<Cleaned, String> {
        let (mut entries, partials) = self.scan(None, SystemTime::now());
        let total: u64 = entries.iter().map(|e| e.bytes).sum();
        if total <= self.cap {
            return Ok(Self::remove([], partials));
        }
        entries.sort_by(|a, b| a.used.cmp(&b.used).then_with(|| a.path.cmp(&b.path)));
        let mut over = total - self.cap;
        let doomed: Vec<Entry> = entries
            .into_iter()
            .filter(|entry| Some(entry.path.as_path()) != spared)
            .take_while(|entry| {
                let take = over > 0;
                over = over.saturating_sub(entry.bytes);
                take
            })
            .collect();
        Ok(Self::remove(doomed, partials))
    }
}

#[cfg(test)]
mod tests;

fn files_under(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
    for entry in children(dir) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let kind = entry.file_type().map_err(|e| shown(&entry.path(), e))?;
        if kind.is_dir() {
            files_under(&entry.path(), &relative, out)?;
        } else if kind.is_file() {
            out.push((relative, entry.path()));
        } else {
            return Err(format!(
                "{}: only files and folders can be cached",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

pub fn pack_dir(dir: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let mut files = Vec::new();
    files_under(dir, "", &mut files)?;
    let mut sizes = Vec::with_capacity(files.len());
    let mut total = (DIR_MAGIC.len() + 8) as u64;
    for (name, path) in &files {
        let size = fs::metadata(path).map_err(|e| shown(path, e))?.len();
        total = total.saturating_add(16 + name.len() as u64 + size);
        sizes.push(size);
    }
    if total.saturating_add(HEADER as u64) > limit {
        return Err(format!(
            "{}: {total} bytes packed is over the cache's cap of {limit} ({CAP_VARIABLE})",
            dir.display()
        ));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(usize::try_from(total).map_err(|e| e.to_string())?)
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    out.extend_from_slice(&DIR_MAGIC);
    out.extend_from_slice(&(files.len() as u64).to_le_bytes());
    for ((name, path), size) in files.into_iter().zip(sizes) {
        out.extend_from_slice(&(name.len() as u64).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        let at = out.len();
        out.extend_from_slice(&0u64.to_le_bytes());
        let read = fs::File::open(&path)
            .and_then(|file| file.take(size).read_to_end(&mut out))
            .map_err(|e| shown(&path, e))?;
        out[at..at + 8].copy_from_slice(&(read as u64).to_le_bytes());
    }
    Ok(out)
}

fn take<'a>(bytes: &mut &'a [u8], count: u64) -> Result<&'a [u8], String> {
    let count = usize::try_from(count).map_err(|_| "a packed folder is damaged".to_string())?;
    if bytes.len() < count {
        return Err("a packed folder is truncated".into());
    }
    let (head, rest) = bytes.split_at(count);
    *bytes = rest;
    Ok(head)
}

fn number(bytes: &mut &[u8]) -> Result<u64, String> {
    Ok(u64::from_le_bytes(
        take(bytes, 8)?.try_into().expect("eight bytes"),
    ))
}

fn device_name(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).to_ascii_uppercase();
    let numbered = stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.as_bytes()[3].is_ascii_digit();
    numbered || RESERVED.contains(&stem.as_str())
}

fn safe_part(part: &str) -> bool {
    !part.is_empty()
        && !part.ends_with('.')
        && !part.ends_with(' ')
        && !part
            .chars()
            .any(|c| c.is_control() || matches!(c, ':' | '<' | '>' | '"' | '|' | '?' | '*'))
        && !device_name(part)
}

fn safe_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('\\') && name.split('/').all(safe_part)
}

pub fn unpack_dir(mut bytes: &[u8], into: &Path) -> Result<(), String> {
    if take(&mut bytes, 8)? != DIR_MAGIC {
        return Err("not a packed folder".into());
    }
    let count = number(&mut bytes)?;
    fs::create_dir_all(into).map_err(|e| shown(into, e))?;
    let mut seen = BTreeSet::new();
    for _ in 0..count {
        let length = number(&mut bytes)?;
        let name = std::str::from_utf8(take(&mut bytes, length)?)
            .map_err(|_| "a packed file name is not UTF-8".to_string())?;
        if !safe_name(name) {
            return Err(format!("a packed folder names an unsafe path '{name}'"));
        }
        if !seen.insert(name.to_lowercase()) {
            return Err(format!("a packed folder names '{name}' twice"));
        }
        let length = number(&mut bytes)?;
        let data = take(&mut bytes, length)?;
        let path = into.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| shown(parent, e))?;
        }
        fs::write(&path, data).map_err(|e| shown(&path, e))?;
    }
    if !bytes.is_empty() {
        return Err("a packed folder has bytes past its last file".into());
    }
    Ok(())
}

pub fn restore_dir(bytes: &[u8], target: &Path) -> Result<(), String> {
    let name = target
        .file_name()
        .ok_or_else(|| format!("{}: not a folder name", target.display()))?
        .to_string_lossy()
        .into_owned();
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    sweep_beside(parent, &name, SystemTime::now());
    let tag = unique();
    let next = target.with_file_name(format!(".{name}.{tag}.cache-next"));
    let old = target.with_file_name(format!(".{name}.{tag}.cache-old"));
    if let Err(error) = unpack_dir(bytes, &next) {
        let _ = fs::remove_dir_all(&next);
        return Err(error);
    }
    if target.exists() {
        if let Err(error) = fs::rename(target, &old) {
            let _ = fs::remove_dir_all(&next);
            return Err(shown(target, error));
        }
        touch_dir(&old);
    }
    if let Err(error) = fs::rename(&next, target) {
        if old.exists() && fs::rename(&old, target).is_err() {
            let _ = fs::remove_dir_all(&old);
        }
        let _ = fs::remove_dir_all(&next);
        return Err(shown(target, error));
    }
    let _ = fs::remove_dir_all(&old);
    atomic::sync_dir(parent)
}

fn touch_dir(dir: &Path) {
    if let Ok(file) = fs::File::open(dir) {
        let _ = file.set_modified(SystemTime::now());
    }
}

fn swept_tag(file: &str, name: &str) -> bool {
    let Some(rest) = file
        .strip_prefix('.')
        .and_then(|rest| rest.strip_prefix(name))
        .and_then(|rest| rest.strip_prefix('.'))
    else {
        return false;
    };
    let Some(tag) = rest
        .strip_suffix(".cache-next")
        .or_else(|| rest.strip_suffix(".cache-old"))
    else {
        return false;
    };
    tag.split_once('-').is_some_and(|(pid, n)| {
        [pid, n]
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
    })
}

fn sweep_beside(parent: &Path, name: &str, now: SystemTime) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        if !swept_tag(&entry.file_name().to_string_lossy(), name) {
            continue;
        }
        let abandoned = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .is_ok_and(|at| now.duration_since(at).unwrap_or_default() > ABANDONED);
        if abandoned && entry.file_type().is_ok_and(|t| t.is_dir()) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}
