use std::sync::mpsc::{Receiver, channel};

use super::*;
use pfx_editor::editor::{Play as Playing, Request};
use pfx_editor::log::Level;
use pfx_editor::play::{From as Chosen, Play as Module};
use pfx_editor::session::Session;

pub const PLAY_DIR: &str = "tmp/pfx/play";
const PLAY_WAIT: u32 = 3;

pub(super) struct Preparing {
    done: Receiver<Result<PathBuf, String>>,
}

pub fn recipe_scene(recipe: &Path, registry: &Registry, out: &Path) -> Result<PathBuf, String> {
    let file = recipe.display().to_string();
    let loaded = pfx_recipe::load(recipe).map_err(|error| format!("{file}: {error}"))?;
    let mut recipe = loaded.recipe;
    let name = format!("{}.scene.toml", recipe.name);
    recipe.output.scene = Some(name.clone());
    recipe.output.bake = None;
    let base = loaded
        .path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let built = pfx_build::build_with(&recipe, &base, registry)
        .map_err(|error| format!("{file}: {error}"))?;
    let out = out.join(&recipe.name);
    let written = pfx_build::write(&built, &recipe, &base, Some(&out))
        .map_err(|error| format!("{file}: {error}"))?;
    written
        .into_iter()
        .find(|path| path.file_name().is_some_and(|found| found == name.as_str()))
        .ok_or_else(|| format!("{file}: the build wrote no scene"))
}

impl Shell {
    pub fn edited(session: Session) -> Shell {
        let mut shell = Shell::with(Document::empty(""));
        shell.take_session(session);
        shell
    }

    pub(super) fn take_session(&mut self, session: Session) {
        let path = session.editor.path().to_path_buf();
        let document = crate::scenes::document(session.editor.scene(), &path.display().to_string());
        self.scene_revision = session.editor.revision;
        self.edit = Some(Box::new(session));
        self.playing_recipe = false;
        self.preparing_play = None;
        self.editor = None;
        self.scene = true;
        self.adopt(&path, &path, document);
        self.scene = true;
    }

    pub(super) fn drop_session(&mut self) {
        self.edit = None;
        self.playing_recipe = false;
        self.preparing_play = None;
    }

    pub fn session(&self) -> Option<&Session> {
        self.edit.as_deref()
    }

    pub fn session_mut(&mut self) -> Option<&mut Session> {
        self.edit.as_deref_mut()
    }

    pub fn edits_scene(&self) -> bool {
        self.edit.is_some() && !self.playing_recipe
    }

    pub fn plays_recipe(&self) -> bool {
        self.playing_recipe
    }

    pub(super) fn shows_session(&self) -> bool {
        self.edit.is_some()
    }

    pub fn module(&self) -> &Module {
        match self.edit.as_deref() {
            Some(session) if !self.playing_recipe => &session.editor.module,
            _ => &self.module,
        }
    }

    fn module_mut(&mut self) -> &mut Module {
        match self.edit.as_deref_mut() {
            Some(session) if !self.playing_recipe => &mut session.editor.module,
            _ => &mut self.module,
        }
    }

    pub fn choose_module(&mut self, module: PathBuf) {
        let module = std::path::absolute(&module).unwrap_or(module);
        self.module_mut().set_module(Some(module), Chosen::Flag);
    }

    pub(super) fn look_for_modules(&mut self) {
        if self.edit.is_some() {
            return;
        }
        let folder = Path::new(&self.document.path)
            .parent()
            .filter(|folder| !folder.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        self.module.look(&folder);
    }

    pub fn play_state(&self) -> Playing {
        self.edit
            .as_deref()
            .map_or(Playing::Stopped, |session| session.editor.play)
    }

    pub fn can_play(&self) -> bool {
        self.edits_scene() || (self.editor.is_some() && !self.calm)
    }

    pub fn pick_module(&mut self) {
        if self.play_state() != Playing::Stopped || self.preparing_play.is_some() {
            return;
        }
        match self.edit.as_deref_mut() {
            Some(session) => session.editor.pick_module(),
            None => {
                self.module.next();
                self.said_module();
            }
        }
    }

    pub fn use_module(&mut self, module: PathBuf) {
        if self.play_state() != Playing::Stopped || self.preparing_play.is_some() {
            return;
        }
        match self.edit.as_deref_mut() {
            Some(session) => session.editor.use_module(module),
            None => {
                self.module.set_module(Some(module), Chosen::Picked);
                self.said_module();
            }
        }
    }

    fn said_module(&mut self) {
        let line = match self.module.name() {
            Some(name) => format!("F5 plays the recipe with module {name}"),
            None => "F5 plays the recipe with no module".to_string(),
        };
        self.file_note = Some((line, false));
    }

    pub fn play_pressed(&mut self) {
        if let Some(session) = self.edit.as_deref_mut() {
            session.editor.play_pressed();
            return;
        }
        if !self.can_play() || self.preparing_play.is_some() {
            return;
        }
        let recipe = PathBuf::from(&self.document.path);
        let registry = self.registry.clone();
        let (send, done) = channel();
        std::thread::spawn(move || {
            let _ = send.send(recipe_scene(&recipe, &registry, Path::new(PLAY_DIR)));
        });
        self.preparing_play = Some(Preparing { done });
        self.file_note = Some(("play: building the recipe's scene…".to_string(), false));
    }

    pub fn stop_pressed(&mut self) {
        if let Some(session) = self.edit.as_deref_mut()
            && session.editor.play != Playing::Stopped
        {
            session.editor.request(Request::Stop);
        }
    }

    pub(super) fn begin_session(&mut self, ctx: &Context) {
        self.poll_play(ctx);
        let typing = ctx.memory(|memory| memory.focused().is_some());
        let Some(session) = self.edit.as_deref_mut() else {
            let dropped = ctx.input(|input| {
                input
                    .raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path().to_path_buf())
                    .rfind(|path| path.extension().is_some_and(|end| end == "wasm"))
            });
            if let Some(module) = dropped {
                self.use_module(module);
            }
            if !typing && ctx.input(|input| input.modifiers.is_none()) {
                if ctx.input(|input| input.key_pressed(Key::F4)) {
                    self.pick_module();
                }
                if ctx.input(|input| input.key_pressed(Key::F5)) {
                    self.play_pressed();
                }
            }
            return;
        };
        session.begin(ctx);
        session.editor.play_keys(ctx);
        if !self.playing_recipe {
            session.editor.level_tool_keys(ctx);
        }
        if session.editor.play != Playing::Stopped {
            ctx.request_repaint();
        }
        if !self.playing_recipe && session.editor.revision != self.scene_revision {
            self.scene_revision = session.editor.revision;
            let path = session.editor.path().display().to_string();
            let document = crate::scenes::document(session.editor.scene(), &path);
            self.show(document);
        }
    }

    fn poll_play(&mut self, ctx: &Context) {
        if let Some(preparing) = &self.preparing_play {
            match preparing.done.try_recv() {
                Ok(Ok(scene)) => {
                    self.preparing_play = None;
                    self.start_recipe_play(&scene);
                }
                Ok(Err(error)) => {
                    self.preparing_play = None;
                    self.file_note = Some((format!("play: {error}"), true));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.preparing_play = None;
                }
            }
        }
        if !self.playing_recipe {
            return;
        }
        let Some(session) = self.edit.as_deref() else {
            self.playing_recipe = false;
            return;
        };
        if session.editor.play != Playing::Stopped {
            self.play_frames = PLAY_WAIT;
            return;
        }
        if self.play_frames > 0 {
            self.play_frames -= 1;
            return;
        }
        let last = session
            .editor
            .log
            .entries
            .back()
            .map(|entry| (entry.text.clone(), entry.level == Level::Error));
        self.drop_session();
        self.file_note = last;
    }

    fn start_recipe_play(&mut self, scene: &Path) {
        let mut session = match Session::open(scene, pfx_editor::setup()) {
            Ok(session) => session,
            Err(error) => {
                self.file_note = Some((format!("play: {error}"), true));
                return;
            }
        };
        let chosen = self.module.module().map(Path::to_path_buf);
        session.editor.module.set_module(chosen, self.module.from());
        if let Some(wake) = &self.session_wake {
            pfx_editor_shell::App::waker(&mut session, wake.clone());
        }
        session.editor.play_pressed();
        self.edit = Some(Box::new(session));
        self.playing_recipe = true;
        self.play_frames = PLAY_WAIT;
        self.file_note = None;
    }

    pub(crate) fn set_session_wake(&mut self, wake: pfx_editor_shell::worker::Wake) {
        if let Some(session) = self.edit.as_deref_mut() {
            pfx_editor_shell::App::waker(session, wake.clone());
        }
        self.session_wake = Some(wake);
    }

    pub(super) fn session_said(&self) -> Option<(String, Color32)> {
        let entry = self.edit.as_deref()?.editor.log.entries.back()?;
        let colour = match entry.level {
            Level::Error => DANGER,
            Level::Warn => ACCENT,
            Level::Info => TEXT_CODE,
        };
        Some((entry.text.clone(), colour))
    }

    pub(super) fn session_undo(&mut self, command: Command) -> bool {
        let Some(session) = self.edit.as_deref_mut().filter(|_| !self.playing_recipe) else {
            return false;
        };
        match command {
            Command::Undo => session.editor.undo(),
            Command::Redo => session.editor.redo(),
            _ => return false,
        }
        true
    }

    pub(super) fn session_view(&mut self, ui: &mut Ui) {
        if let Some(session) = self.edit.as_deref_mut() {
            session.editor.viewport(ui);
        }
    }

    pub(super) fn session_outliner(&mut self, ui: &mut Ui) {
        if let Some(session) = self.edit.as_deref_mut() {
            session.editor.layout.outline.clear();
            session.editor.layout.buttons.clear();
            session.editor.outline_rows(ui);
        }
    }

    pub(super) fn session_inspector(&mut self, ui: &mut Ui) {
        if let Some(session) = self.edit.as_deref_mut() {
            session.editor.layout.rows.clear();
            session.editor.inspector_rows(ui);
        }
    }

    pub(super) fn session_key(&mut self, key: Key) -> bool {
        let Some(session) = self.edit.as_deref_mut().filter(|_| !self.playing_recipe) else {
            return false;
        };
        let editor = &mut session.editor;
        match key {
            Key::Num0 if editor.editable() => editor.toggle_view(),
            Key::T => editor.trace = !editor.trace,
            Key::F => editor.focus(),
            Key::J => editor.step_selection(1),
            Key::K => editor.step_selection(-1),
            Key::M => editor.muted = !editor.muted,
            Key::Delete | Key::Backspace => {
                editor.delete();
            }
            Key::Escape if editor.viewport.gizmo().dragging().is_none() => editor.select(None),
            _ => return false,
        }
        true
    }

    pub(super) fn session_trace(&mut self) {
        if let Some(session) = self.edit.as_deref_mut() {
            session.editor.trace = !session.editor.trace;
        }
    }

    pub(super) fn session_tracing(&self) -> Option<(bool, u32)> {
        self.edit
            .as_deref()
            .filter(|_| !self.playing_recipe)
            .map(|session| (session.editor.trace, session.editor.trace_samples))
    }
}
