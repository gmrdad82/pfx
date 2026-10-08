use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use pfx_editor_doc::Stamp;
use pfx_editor_shell::Wake;
use pfx_load::scene::{Reload, Scene, SceneError, SceneWatch};

pub type Waker = Arc<Mutex<Option<Wake>>>;

pub fn worded(error: &SceneError, scene: &Path) -> String {
    let folder = scene.parent().unwrap_or(Path::new(""));
    let file = error.file.strip_prefix(folder).unwrap_or(&error.file);
    match error.line {
        Some(line) => format!("{}:{line}: {}", file.display(), error.message),
        None => format!("{}: {}", file.display(), error.message),
    }
}

pub fn is_scene(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().ends_with(".scene.toml"))
        || path.file_name().is_some_and(|name| name == "scene.toml")
}

#[derive(Debug)]
pub enum Event {
    Reloaded(Box<Reload>),
    Broken(String),
}

pub struct Watched {
    path: PathBuf,
    watch: Option<SceneWatch>,
    waker: Waker,
    error: Option<String>,
    tried: Option<Option<Stamp>>,
    reloads: u64,
}

fn ring(waker: &Waker) {
    let wake = waker.lock().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(wake) = wake {
        wake();
    }
}

impl Watched {
    pub fn open(path: impl Into<PathBuf>) -> Watched {
        let mut watched = Watched {
            path: path.into(),
            watch: None,
            waker: Arc::new(Mutex::new(None)),
            error: None,
            tried: None,
            reloads: 0,
        };
        watched.reopen();
        watched
    }

    fn stamp(&self) -> Option<Stamp> {
        std::fs::read_to_string(&self.path)
            .ok()
            .map(|text| Stamp::of(&text))
    }

    fn reopen(&mut self) -> bool {
        self.tried = Some(self.stamp());
        let waker = self.waker.clone();
        match SceneWatch::open_with(&self.path, move || ring(&waker)) {
            Ok(watch) => {
                self.watch = Some(watch);
                self.error = None;
                true
            }
            Err(error) => {
                self.error = Some(worded(&error, &self.path));
                false
            }
        }
    }

    pub fn set_wake(&mut self, wake: Wake) {
        *self.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(wake);
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn scene(&self) -> Option<&Scene> {
        self.watch.as_ref().map(SceneWatch::scene)
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn reloads(&self) -> u64 {
        self.reloads
    }

    pub fn files(&self) -> Vec<PathBuf> {
        match &self.watch {
            Some(watch) => watch.files().map(Path::to_path_buf).collect(),
            None => vec![self.path.clone()],
        }
    }

    pub fn check(&mut self) -> Option<Event> {
        let result = match self.watch.as_mut() {
            Some(watch) => watch.check(),
            None => return self.opened(),
        };
        self.take(result)
    }

    pub fn poll(&mut self) -> Option<Event> {
        if self.watch.is_none() && self.tried == Some(self.stamp()) {
            return None;
        }
        let result = match self.watch.as_mut() {
            Some(watch) => watch.poll(),
            None => return self.opened(),
        };
        self.take(result)
    }

    fn opened(&mut self) -> Option<Event> {
        if !self.reopen() {
            return None;
        }
        let scene = self.scene()?.clone();
        self.reloads += 1;
        Some(Event::Reloaded(Box::new(Reload {
            diff: Scene::diff(&scene, &scene),
            scene,
            recovered: true,
        })))
    }

    fn take(&mut self, result: Option<Result<Reload, SceneError>>) -> Option<Event> {
        match result? {
            Ok(reload) => {
                self.error = None;
                self.reloads += 1;
                Some(Event::Reloaded(Box::new(reload)))
            }
            Err(error) => {
                let worded = worded(&error, &self.path);
                self.error = Some(worded.clone());
                Some(Event::Broken(worded))
            }
        }
    }
}
