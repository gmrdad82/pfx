use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use super::{Scene, SceneDiff, SceneError};
use crate::sha256;

pub const DEBOUNCE: Duration = Duration::from_millis(100);

#[derive(Clone, Debug)]
pub struct Reload {
    pub scene: Scene,
    pub diff: SceneDiff,
    pub recovered: bool,
}

pub struct SceneWatch {
    path: PathBuf,
    scene: Scene,
    watcher: RecommendedWatcher,
    events: Receiver<Instant>,
    folders: BTreeSet<PathBuf>,
    hashes: BTreeMap<PathBuf, Option<[u8; 32]>>,
    pending: Option<Instant>,
    debounce: Duration,
    shared_debounce: Arc<AtomicU64>,
    broken: Option<SceneError>,
}

fn counts(event: &notify::Result<notify::Event>) -> bool {
    matches!(event, Ok(event) if !matches!(event.kind, EventKind::Access(_)))
}

fn nanos(debounce: Duration) -> u64 {
    u64::try_from(debounce.as_nanos()).unwrap_or(u64::MAX)
}

fn waker(
    signals: Receiver<()>,
    debounce: Arc<AtomicU64>,
    wake: Box<dyn Fn() + Send>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("scene watch waker".into())
        .spawn(move || {
            while signals.recv().is_ok() {
                loop {
                    let quiet = Duration::from_nanos(debounce.load(Ordering::Relaxed));
                    match signals.recv_timeout(quiet) {
                        Ok(()) => {}
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                wake();
            }
        })
        .map(|_| ())
}

fn folder(file: &Path) -> PathBuf {
    match file.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn hashes(files: impl IntoIterator<Item = PathBuf>) -> BTreeMap<PathBuf, Option<[u8; 32]>> {
    files
        .into_iter()
        .map(|file| {
            let hash = std::fs::read(&file).ok().map(|bytes| sha256(&bytes));
            (file, hash)
        })
        .collect()
}

impl SceneWatch {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SceneError> {
        Self::start(path.as_ref(), None)
    }

    pub fn open_with(
        path: impl AsRef<Path>,
        wake: impl Fn() + Send + 'static,
    ) -> Result<Self, SceneError> {
        Self::start(path.as_ref(), Some(Box::new(wake)))
    }

    fn start(path: &Path, wake: Option<Box<dyn Fn() + Send>>) -> Result<Self, SceneError> {
        let path = path.to_path_buf();
        let scene = Scene::open(&path)?;
        let (sender, events) = channel();
        let shared_debounce = Arc::new(AtomicU64::new(nanos(DEBOUNCE)));
        let signals: Option<Sender<()>> = match wake {
            Some(wake) => {
                let (signal, signals) = channel();
                waker(signals, shared_debounce.clone(), wake).map_err(|error| {
                    SceneError::new(&path, None, format!("cannot start the waker: {error}"))
                })?;
                Some(signal)
            }
            None => None,
        };
        let watcher = notify::recommended_watcher(move |event| {
            if counts(&event) {
                let _ = sender.send(Instant::now());
                if let Some(signal) = &signals {
                    let _ = signal.send(());
                }
            }
        })
        .map_err(|error| SceneError::new(&path, None, format!("cannot watch: {error}")))?;
        let mut watch = Self {
            hashes: scene
                .files
                .iter()
                .map(|(file, hash)| (file.clone(), Some(*hash)))
                .collect(),
            path,
            scene,
            watcher,
            events,
            folders: BTreeSet::new(),
            pending: None,
            debounce: DEBOUNCE,
            shared_debounce,
            broken: None,
        };
        watch.follow()?;
        Ok(watch)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    pub fn error(&self) -> Option<&SceneError> {
        self.broken.as_ref()
    }

    pub fn set_debounce(&mut self, debounce: Duration) {
        self.debounce = debounce;
        self.shared_debounce
            .store(nanos(debounce), Ordering::Relaxed);
    }

    fn drain(&mut self) {
        while let Ok(at) = self.events.try_recv() {
            self.pending = Some(self.pending.map_or(at, |seen| seen.max(at)));
        }
    }

    pub fn pending(&mut self) -> Option<Instant> {
        self.drain();
        self.pending.map(|seen| seen + self.debounce)
    }

    pub fn files(&self) -> impl Iterator<Item = &Path> {
        self.hashes.keys().map(PathBuf::as_path)
    }

    fn follow(&mut self) -> Result<(), SceneError> {
        let wanted: BTreeSet<PathBuf> = self.hashes.keys().map(|file| folder(file)).collect();
        for gone in self.folders.difference(&wanted) {
            let _ = self.watcher.unwatch(gone);
        }
        for new in wanted.difference(&self.folders) {
            self.watcher
                .watch(new, RecursiveMode::NonRecursive)
                .map_err(|error| {
                    SceneError::new(
                        &self.path,
                        None,
                        format!("cannot watch {}: {error}", new.display()),
                    )
                })?;
        }
        self.folders = wanted;
        Ok(())
    }

    pub fn poll(&mut self) -> Option<Result<Reload, SceneError>> {
        self.drain();
        let since = self.pending?;
        if since.elapsed() < self.debounce {
            return None;
        }
        self.pending = None;
        self.check()
    }

    pub fn check(&mut self) -> Option<Result<Reload, SceneError>> {
        let now = hashes(self.hashes.keys().cloned());
        if now == self.hashes {
            return None;
        }
        match Scene::open(&self.path) {
            Ok(scene) => {
                let diff = self.scene.diff(&scene);
                let recovered = self.broken.take().is_some();
                self.hashes = scene
                    .files
                    .iter()
                    .map(|(file, hash)| (file.clone(), Some(*hash)))
                    .collect();
                self.scene = scene.clone();
                if let Err(error) = self.follow() {
                    return Some(Err(error));
                }
                if diff.is_empty() && !recovered {
                    return None;
                }
                Some(Ok(Reload {
                    scene,
                    diff,
                    recovered,
                }))
            }
            Err(error) => {
                self.hashes = now;
                self.broken = Some(error.clone());
                Some(Err(error))
            }
        }
    }
}
