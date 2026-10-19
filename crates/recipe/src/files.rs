use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::ThreadId;

type Shared = Arc<Mutex<Layer>>;

pub type Load = Arc<dyn Fn() -> io::Result<Vec<u8>> + Send + Sync>;

pub enum Content {
    Bytes(Vec<u8>),
    Lazy(Load),
}

enum Slot {
    Ready(Arc<[u8]>),
    Lazy(Load),
}

#[derive(Default)]
struct Layer {
    files: BTreeMap<PathBuf, Slot>,
    changed: BTreeSet<PathBuf>,
    names: BTreeMap<PathBuf, String>,
}

fn lock(layer: &Shared) -> MutexGuard<'_, Layer> {
    layer.lock().unwrap_or_else(|e| e.into_inner())
}

fn fetch(layer: &Shared, key: &Path) -> Option<io::Result<Arc<[u8]>>> {
    let load = match lock(layer).files.get(key)? {
        Slot::Ready(bytes) => return Some(Ok(bytes.clone())),
        Slot::Lazy(load) => load.clone(),
    };
    let bytes: Arc<[u8]> = match load() {
        Ok(bytes) => Arc::from(bytes),
        Err(error) => return Some(Err(error)),
    };
    let mut layer = lock(layer);
    match layer.files.get_mut(key) {
        Some(Slot::Ready(now)) => Some(Ok(now.clone())),
        Some(slot) => {
            *slot = Slot::Ready(bytes.clone());
            Some(Ok(bytes))
        }
        None => Some(Ok(bytes)),
    }
}

struct Mounted {
    id: u64,
    owner: ThreadId,
    layer: Shared,
}

#[derive(Default)]
struct Table {
    mounts: Vec<Mounted>,
    next: u64,
}

impl Table {
    fn find<T>(&self, mut pick: impl FnMut(&Layer) -> Option<T>) -> Option<(T, Shared)> {
        let here = std::thread::current().id();
        let mut take = |m: &Mounted| pick(&lock(&m.layer)).map(|found| (found, m.layer.clone()));
        self.mounts
            .iter()
            .rev()
            .filter(|m| m.owner == here)
            .find_map(&mut take)
            .or_else(|| self.mounts.iter().rev().find_map(&mut take))
    }
}

static TEMPS: AtomicU64 = AtomicU64::new(0);

static TABLE: Mutex<Table> = Mutex::new(Table {
    mounts: Vec::new(),
    next: 0,
});

fn table() -> MutexGuard<'static, Table> {
    TABLE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn clean(path: &Path) -> PathBuf {
    let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for component in full.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

pub struct Mount {
    id: u64,
    layer: Shared,
}

impl Mount {
    pub fn changed(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        let layer = lock(&self.layer);
        layer
            .changed
            .iter()
            .filter_map(|path| match layer.files.get(path)? {
                Slot::Ready(bytes) => Some((path.clone(), bytes.to_vec())),
                Slot::Lazy(_) => None,
            })
            .collect()
    }

    pub fn get(&self, path: &Path) -> Option<Vec<u8>> {
        fetch(&self.layer, &clean(path))?
            .ok()
            .map(|bytes| bytes.to_vec())
    }

    pub fn settle(&self) {
        lock(&self.layer).changed.clear();
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        table().mounts.retain(|m| m.id != self.id);
    }
}

pub fn mount(files: BTreeMap<PathBuf, Vec<u8>>) -> Mount {
    mount_named(
        files
            .into_iter()
            .map(|(path, bytes)| (path, Content::Bytes(bytes)))
            .collect(),
        BTreeMap::new(),
    )
}

pub fn mount_named(files: BTreeMap<PathBuf, Content>, names: BTreeMap<PathBuf, String>) -> Mount {
    let layer = Layer {
        files: files
            .into_iter()
            .map(|(path, content)| {
                let slot = match content {
                    Content::Bytes(bytes) => Slot::Ready(Arc::from(bytes)),
                    Content::Lazy(load) => Slot::Lazy(load),
                };
                (clean(&path), slot)
            })
            .collect(),
        changed: BTreeSet::new(),
        names: names
            .into_iter()
            .map(|(path, name)| (clean(&path), name))
            .collect(),
    };
    let layer = Arc::new(Mutex::new(layer));
    let mut table = table();
    table.next += 1;
    let id = table.next;
    table.mounts.push(Mounted {
        id,
        owner: std::thread::current().id(),
        layer: layer.clone(),
    });
    Mount { id, layer }
}

pub struct Recorder(());

impl Recorder {
    pub fn seen(&self) -> BTreeSet<PathBuf> {
        RECORDING.with(|r| r.borrow().clone().unwrap_or_default())
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        RECORDING.with(|r| *r.borrow_mut() = None);
    }
}

thread_local! {
    static RECORDING: RefCell<Option<BTreeSet<PathBuf>>> = const { RefCell::new(None) };
}

pub fn record() -> Recorder {
    RECORDING.with(|r| *r.borrow_mut() = Some(BTreeSet::new()));
    Recorder(())
}

fn layer_of(path: &Path) -> Option<(Shared, PathBuf)> {
    let table = table();
    if table.mounts.is_empty() {
        return None;
    }
    let key = clean(path);
    table
        .find(|layer| layer.files.contains_key(&key).then_some(()))
        .map(|((), layer)| (layer, key))
}

pub fn shown(path: &str) -> String {
    let table = table();
    if table.mounts.is_empty() {
        return path.to_string();
    }
    let key = clean(Path::new(path));
    table
        .find(|layer| layer.names.get(&key).cloned())
        .map_or_else(|| path.to_string(), |(name, _)| name)
}

fn noted(path: &Path) {
    RECORDING.with(|r| {
        if let Some(seen) = r.borrow_mut().as_mut() {
            seen.insert(clean(path));
        }
    });
}

pub fn mounted(path: &Path) -> bool {
    layer_of(path).is_some()
}

pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    if let Some((layer, key)) = layer_of(path)
        && let Some(bytes) = fetch(&layer, &key)
    {
        return bytes.map(|bytes| bytes.to_vec());
    }
    let bytes = std::fs::read(path)?;
    noted(path);
    Ok(bytes)
}

pub fn read_to_string(path: &Path) -> io::Result<String> {
    String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn is_file(path: &Path) -> bool {
    mounted(path) || path.is_file()
}

pub fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some((layer, key)) = layer_of(path) {
        let now = fetch(&layer, &key).and_then(Result::ok);
        if now.as_deref() != Some(bytes) {
            let mut layer = lock(&layer);
            layer
                .files
                .insert(key.clone(), Slot::Ready(Arc::from(bytes)));
            layer.changed.insert(key);
        }
        return Ok(());
    }
    let name = path
        .file_name()
        .map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned());
    let temp = path.with_file_name(format!(
        ".{name}.{}.{}.pitofx-tmp",
        std::process::id(),
        TEMPS.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn a_thread_reads_and_writes_its_own_mount_while_a_newer_one_holds_the_same_path() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp/no-disk/shared/rig.toml");
        let mine = mount(BTreeMap::from([(path.clone(), b"mine".to_vec())]));
        let (mounted, ready) = channel();
        let (release, done) = channel::<()>();
        let theirs = path.clone();
        let holder = std::thread::spawn(move || {
            let held = mount(BTreeMap::from([(theirs.clone(), b"theirs".to_vec())]));
            mounted.send(()).unwrap();
            let _ = done.recv();
            let seen = read(&theirs).unwrap();
            (held.changed(), seen)
        });
        ready.recv().unwrap();
        let read_here = read(&path).unwrap();
        replace(&path, b"folded").unwrap();
        release.send(()).unwrap();
        let (their_changes, their_read) = holder.join().unwrap();
        assert_eq!(read_here, b"mine");
        assert_eq!(
            mine.changed(),
            BTreeMap::from([(clean(&path), b"folded".to_vec())])
        );
        assert!(their_changes.is_empty());
        assert_eq!(their_read, b"theirs");
        assert!(!path.exists());
    }
}
