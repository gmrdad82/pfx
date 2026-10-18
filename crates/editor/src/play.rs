use std::path::{Path, PathBuf};

use pfx_game::wasm::ModuleGame;
use pfx_input::InputEvent;
use pfx_live::scene::Staged;
use pfx_load::scene::Scene;
use pfx_play::{Exit, Game, Options, PlaySession, SceneGame, Stopped};

pub const FONT: &[u8] = pfx_theme::MONO_REGULAR;
pub const FONT_FAMILY: &str = "JetBrains Mono";
pub const FOUND_CAP: usize = 32;
pub const FOLDERS_CAP: usize = 2_000;
pub const DEPTH_CAP: usize = 6;
const SKIPPED: &[&str] = &["deps", "build", "incremental", "node_modules"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum From {
    #[default]
    Scene,
    Flag,
    Picked,
}

#[derive(Default)]
pub struct Play {
    module: Option<PathBuf>,
    from: From,
    found: Vec<PathBuf>,
    session: Option<PlaySession>,
    stopped: Option<Stopped>,
}

pub fn modules_in(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![(root.to_path_buf(), 0)];
    let mut seen = 0;
    while let Some((folder, depth)) = folders.pop() {
        seen += 1;
        if seen > FOLDERS_CAP || found.len() >= FOUND_CAP {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.file_name()));
        for entry in entries {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => {
                    if depth < DEPTH_CAP && !SKIPPED.contains(&name.as_ref()) {
                        folders.push((path, depth + 1));
                    }
                }
                Ok(kind) if kind.is_file() && name.ends_with(".wasm") => found.push(path),
                _ => {}
            }
        }
    }
    found.sort();
    found.truncate(FOUND_CAP);
    found
}

impl Play {
    pub fn of(scene: &Scene) -> Play {
        Play {
            module: scene.play.clone(),
            ..Play::default()
        }
    }

    pub fn module(&self) -> Option<&Path> {
        self.module.as_deref()
    }

    pub fn from(&self) -> From {
        self.from
    }

    pub fn set_module(&mut self, module: Option<PathBuf>, from: From) {
        self.module = module;
        self.from = from;
    }

    pub fn follow(&mut self, scene: &Scene) {
        if self.from == From::Scene {
            self.module.clone_from(&scene.play);
        }
    }

    pub fn look(&mut self, root: &Path) {
        self.found = modules_in(root);
    }

    pub fn found(&self) -> &[PathBuf] {
        &self.found
    }

    pub fn offered(&self) -> bool {
        self.module.is_some() || !self.found.is_empty()
    }

    pub fn next(&mut self) {
        let at = self
            .module
            .as_ref()
            .and_then(|module| self.found.iter().position(|found| found == module));
        let module = match at {
            Some(at) => self.found.get(at + 1).cloned(),
            None if self.module.is_some() => None,
            None => self.found.first().cloned(),
        };
        self.set_module(module, From::Picked);
    }

    pub fn name(&self) -> Option<String> {
        self.module.as_deref().map(pfx_game::wasm::module_id)
    }

    pub fn game(&self, family: Option<&str>) -> Result<Option<Box<dyn Game>>, String> {
        let Some(module) = &self.module else {
            return Ok(None);
        };
        let mut game = ModuleGame::open(module)?;
        if let Some(refused) = game.refused() {
            return Err(refused.to_string());
        }
        if let Some(family) = family {
            game = game.font(family);
        }
        Ok(Some(Box::new(game)))
    }

    pub fn start(
        &mut self,
        scene: &Scene,
        staged: Option<&Staged>,
        options: Options,
    ) -> Result<(), String> {
        if self.session.is_some() {
            return Ok(());
        }
        let game = self
            .game(Some(FONT_FAMILY))?
            .unwrap_or_else(|| Box::new(SceneGame));
        let session = PlaySession::play_with(scene, staged, game, options)
            .map_err(|error| error.to_string())?;
        self.stopped = None;
        self.session = Some(session);
        match self.exit() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub fn feed(&mut self, event: InputEvent) {
        if let Some(session) = &mut self.session {
            session.feed(event);
        }
    }

    pub fn frame(&mut self, seconds: f32) -> Result<bool, String> {
        let Some(session) = &mut self.session else {
            return Ok(false);
        };
        session.advance(seconds);
        match self.exit() {
            Some(error) => Err(error),
            None => Ok(self.session.is_some()),
        }
    }

    fn exit(&mut self) -> Option<String> {
        let exit = self.session.as_ref()?.exit()?.clone();
        self.halt();
        match exit {
            Exit::Failed(error) => Some(format!("the game failed: {error}")),
            Exit::Quit(_) => None,
        }
    }

    fn halt(&mut self) {
        if let Some(session) = self.session.take() {
            self.stopped = Some(session.stop());
        }
    }

    pub fn stop(&mut self) -> Option<Stopped> {
        self.halt();
        self.stopped.take()
    }

    pub fn is_running(&self) -> bool {
        self.session.is_some()
    }

    pub fn session(&self) -> Option<&PlaySession> {
        self.session.as_ref()
    }

    pub fn session_mut(&mut self) -> Option<&mut PlaySession> {
        self.session.as_mut()
    }
}
