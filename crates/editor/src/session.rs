use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, SystemTime};

use egui_wgpu::Renderer;
use pfx_editor_shell::{App, Pgpu, Theme, Timing, Wake, Watch};
use pfx_game::Config;
use pfx_gpu::screens::ScreenPolicy;
use pfx_gpu::{Gpu, OffscreenTarget};
use pfx_input::{InputEvent, KeyLabels};
use pfx_play::{Level, Reload};
use pfx_project::Project;

use crate::audio::Audio;
use crate::editor::{Editor, Play, Request};
use crate::input::Layout;
use crate::view::{Stage, View};

pub const STATUS_EVERY: Duration = Duration::from_secs(3);
pub const MAX_SECONDS: f32 = 0.25;

pub type Factory = pfx_game::Factory;

pub const FLAT_SCENE: &str = "tmp/pfx/flat.scene.toml";

pub fn free_policy() -> ScreenPolicy {
    ScreenPolicy {
        deck: Vec::new(),
        desktop: Vec::new(),
        ..ScreenPolicy::default()
    }
}

pub fn policy_of(project: Option<&Project>) -> ScreenPolicy {
    project
        .and_then(Project::screen)
        .and_then(|table| ScreenPolicy::from_table(table).ok())
        .unwrap_or_else(free_policy)
}

pub fn empty_world(project: &Project) -> Result<PathBuf, String> {
    let path = project.root().join(FLAT_SCENE);
    if !path.is_file() {
        let folder = path.parent().unwrap_or(project.root());
        std::fs::create_dir_all(folder)
            .map_err(|error| format!("{}: {error}", folder.display()))?;
        std::fs::write(&path, "format = 1\n")
            .map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(path)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
}

fn stamp(file: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(file).ok()?;
    Some(Stamp {
        modified: meta.modified().ok(),
        len: meta.len(),
    })
}

pub struct Session {
    pub editor: Editor,
    pub view: Option<Box<View>>,
    setup: Setup,
    factory: Option<Factory>,
    config: Option<Config>,
    wake: Option<Wake>,
    status: Option<Receiver<String>>,
    seconds: f32,
    watched: Vec<PathBuf>,
    modules: Vec<PathBuf>,
    stamps: Vec<Option<Stamp>>,
    held: bool,
    drawn: bool,
    modifiers: egui::Modifiers,
    layout: Option<Layout>,
    labels: KeyLabels,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setup {
    pub pgpu: bool,
    pub audio: bool,
}

pub fn status_watch(wake: impl Fn() + Send + 'static) -> Receiver<String> {
    let (send, receive) = channel();
    std::thread::spawn(move || {
        loop {
            let line = match crate::pgpu::status() {
                Ok(status) => status.line(),
                Err(error) => error,
            };
            if send.send(line).is_err() {
                return;
            }
            wake();
            std::thread::sleep(STATUS_EVERY);
        }
    });
    receive
}

impl Session {
    pub fn open(path: &Path, setup: Setup) -> Result<Session, String> {
        let editor = Editor::open(path)?;
        let watched = editor.files();
        Ok(Session {
            editor,
            view: None,
            setup,
            factory: None,
            config: None,
            wake: None,
            status: None,
            seconds: 0.0,
            watched,
            modules: Vec::new(),
            stamps: Vec::new(),
            held: false,
            drawn: false,
            modifiers: egui::Modifiers::NONE,
            layout: crate::input::layout(None),
            labels: KeyLabels::new(),
        })
    }

    pub fn open_project(
        folder: &Path,
        setup: Setup,
        factory: Option<Factory>,
    ) -> Result<Session, String> {
        Session::open_with(folder, setup, factory, None)
    }

    pub fn open_game(
        folder: &Path,
        setup: Setup,
        factory: Factory,
        config: Config,
    ) -> Result<Session, String> {
        Session::open_with(folder, setup, Some(factory), Some(config))
    }

    fn open_with(
        folder: &Path,
        setup: Setup,
        factory: Option<Factory>,
        mut config: Option<Config>,
    ) -> Result<Session, String> {
        let project = Project::folder(folder)?;
        match &config {
            Some(config) => config.bind_text(),
            None => {
                pfx_load::scene::bind("name", project.name());
                if let Some(version) = project.version() {
                    pfx_load::scene::bind("version", &version);
                }
            }
        }
        let flat = project.flat() || config.as_ref().is_some_and(|config| config.scene.is_none());
        let (first, empty) = match project.first() {
            Some(first) => (first, false),
            None if flat => (empty_world(&project)?, true),
            None => {
                return Err(format!(
                    "{}: no *.scene.toml in this project to open",
                    project.root().display()
                ));
            }
        };
        let select = project.select().map(str::to_string);
        let name = project.name().to_string();
        let mut session = Session::open(&first, setup)?;
        session.editor.set_project(project);
        if empty {
            session.editor.log.info(format!(
                "project {name} is flat and has no scene: the editor shows an empty world, and F5 plays the game on it"
            ));
        }
        if let Some(name) = select
            && session.editor.scene().object(&name).is_some()
        {
            session
                .editor
                .select(Some(crate::outline::Item::Object(name)));
        }
        if let Some(config) = &config {
            session.editor.app = crate::editor::version_line(&config.name, &config.version);
        }
        session.factory = factory;
        session.layout =
            crate::input::layout(config.as_mut().and_then(|config| config.layout.take()));
        session.config = config;
        Ok(session)
    }

    pub fn flat(&self) -> bool {
        self.project().is_some_and(Project::flat)
            || self
                .config
                .as_ref()
                .is_some_and(|config| config.scene.is_none())
    }

    fn scene_config(&self) -> Config {
        let name = match self.project() {
            Some(project) => project.name().to_string(),
            None => self.editor.name(),
        };
        Config::flat(name, env!("CARGO_PKG_VERSION")).screen(policy_of(self.project()))
    }

    pub fn project(&self) -> Option<&Project> {
        self.editor.project.as_ref()
    }

    pub fn plays_a_game(&self) -> bool {
        self.factory.is_some()
    }

    pub fn switch(&mut self, scene: &Path) -> Result<(), String> {
        if let Some(view) = &mut self.view {
            view.stop_play(&mut self.editor, false);
        }
        self.editor.end_group();
        let mut editor = Editor::open(scene)?;
        editor.muted = self.editor.muted;
        editor.app = std::mem::take(&mut self.editor.app);
        editor.pgpu = self.editor.pgpu.take();
        editor.sound = self.editor.sound.take();
        editor.sound_available = self.editor.sound_available;
        editor.screen = self.editor.screen;
        if let Some(wake) = &self.wake {
            editor.viewport.set_wake(wake.clone());
        }
        if self.held {
            editor.viewport.set_pgpu(Pgpu::Play);
        }
        let mut log = std::mem::take(&mut self.editor.log);
        for entry in std::mem::take(&mut editor.log.entries) {
            log.push(entry);
        }
        editor.log = log;
        if let Some(project) = self.editor.project.take() {
            editor.project = Some(project);
            editor.frame_project();
            editor.refresh();
        }
        self.editor = editor;
        if let Some(view) = &mut self.view {
            view.forget_trace();
        }
        self.drawn = false;
        Ok(())
    }

    pub fn held(&self) -> bool {
        self.held
    }

    pub fn pixels(&self) -> Result<Vec<u8>, String> {
        let view = self.view.as_ref().ok_or("no frame has run yet")?;
        if view.playing() || (self.editor.trace && view.trace_samples() > 0) {
            return view.pixels();
        }
        let live = self
            .editor
            .viewport
            .live()
            .ok_or("the viewport has not drawn")?;
        let texture = live.image().clone();
        let [width, height] = live.size();
        let target = OffscreenTarget {
            view: texture.create_view(&Default::default()),
            texture,
            format: pfx_editor_viewport::live::FORMAT,
            width,
            height,
        };
        view.gpu.readback_bytes(&target)
    }

    fn playing(&self) -> bool {
        self.view.as_deref().is_some_and(View::playing)
    }

    fn game_modules(&mut self) -> Option<&mut dyn Reload> {
        self.view.as_mut()?.session()?.game_mut().modules()
    }

    pub fn poll_modules(&mut self) {
        let files = self
            .game_modules()
            .map(|modules| modules.files())
            .unwrap_or_default();
        if files != self.modules {
            if !files.is_empty() {
                self.editor.log.info(format!(
                    "modules: watching {} built module{} while the game plays",
                    files.len(),
                    if files.len() == 1 { "" } else { "s" }
                ));
            }
            self.stamps = files.iter().map(|file| stamp(file)).collect();
            self.modules = files;
            return;
        }
        let mut rebuilt = Vec::new();
        for (file, last) in self.modules.iter().zip(&mut self.stamps) {
            let now = stamp(file);
            if now != *last {
                *last = now;
                if now.is_some() {
                    rebuilt.push(file.clone());
                }
            }
        }
        let Some(modules) = self.game_modules() else {
            return;
        };
        for file in &rebuilt {
            modules.changed(file);
        }
        for file in rebuilt {
            let name = match &self.editor.project {
                Some(project) => project.relative(&file),
                None => file.display().to_string(),
            };
            self.editor.log.info(format!(
                "modules: {name} was rebuilt; it swaps in at the next tick"
            ));
        }
    }

    fn module_notices(&mut self) {
        let Some(driver) = self.view.as_mut().and_then(|view| view.driver()) else {
            return;
        };
        let notices = driver.take_notices();
        for notice in notices {
            let text = format!("modules: {}", notice.text);
            match notice.level {
                Level::Info => self.editor.log.info(text),
                Level::Warn => self.editor.log.warn(text),
                Level::Error => self.editor.log.error(text),
            }
        }
    }

    fn size(&self, pixels_per_point: f32) -> [u32; 2] {
        let ppp = pixels_per_point.max(0.25);
        let screen = self.editor.screen;
        [
            (screen.width() * ppp).round().max(1.0) as u32,
            (screen.height() * ppp).round().max(1.0) as u32,
        ]
    }

    fn view(&mut self, gpu: &Gpu, pixels_per_point: f32) -> &mut View {
        let size = self.size(pixels_per_point);
        let (pgpu, audio) = (self.setup.pgpu, self.setup.audio);
        self.view.get_or_insert_with(|| {
            let audio = if audio { Audio::device() } else { Audio::off() };
            Box::new(View::open(gpu.clone(), size, pgpu, audio))
        })
    }

    fn requests(&mut self) {
        let mut requests = self.editor.take_requests();
        let mut opens = Vec::new();
        requests.retain(|request| match request {
            Request::Open(scene) => {
                opens.push(scene.clone());
                false
            }
            _ => true,
        });
        if let Some(scene) = opens.pop() {
            if let Err(error) = self.switch(&scene) {
                self.editor.log.error(error);
            }
            return;
        }
        let flat = self.flat();
        let fallback = (self.config.is_none() && requests.contains(&Request::Play))
            .then(|| self.scene_config());
        let Some(view) = &mut self.view else {
            for request in requests {
                if let Request::World(change) = request {
                    self.editor
                        .log
                        .warn(format!("no viewport for {}", change.describe()));
                }
            }
            return;
        };
        for request in requests {
            match request {
                Request::Play => {
                    let Some(config) = self.config.as_ref().or(fallback.as_ref()) else {
                        continue;
                    };
                    let game = self.factory.as_ref().map(|factory| factory());
                    let stage = Stage {
                        config,
                        flat,
                        layout: self.layout.clone(),
                        labels: self.labels.clone(),
                    };
                    if let Err(error) = view.start_play(&mut self.editor, game, stage) {
                        self.editor.log.error(error);
                    }
                }
                Request::Pause => view.pause(&mut self.editor),
                Request::Resume => view.resume(&mut self.editor),
                Request::Step => view.step(&mut self.editor),
                Request::Stop => view.stop_play(&mut self.editor, false),
                Request::Keep => view.stop_play(&mut self.editor, true),
                Request::Record => view.save_recording(&mut self.editor),
                Request::World(change) => view.world_edit(&mut self.editor, &change),
                Request::Open(_) => {}
            }
        }
    }

    pub fn update(
        &mut self,
        gpu: &Gpu,
        painter: &mut Renderer,
        pixels_per_point: f32,
    ) -> Result<bool, String> {
        if let Some(status) = &self.status
            && let Some(line) = status.try_iter().last()
        {
            self.editor.pgpu = Some(line);
        }
        let size = self.size(pixels_per_point);
        self.view(gpu, pixels_per_point);
        self.requests();
        self.poll_modules();
        let seconds = self.seconds;
        let mut more = false;
        if let Some(view) = &mut self.view {
            let resized = view.resize(painter, size)?;
            view.sync(&mut self.editor);
            if view.playing() {
                if self.held {
                    self.held = false;
                    self.editor.viewport.set_pgpu(Pgpu::Free);
                }
                let drew = view.play(&mut self.editor, seconds);
                if view.playing() {
                    self.editor.texture = view.shown().then(|| view.texture(painter));
                    self.drawn |= drew;
                    let more = drew || resized || view.running();
                    self.module_notices();
                    return Ok(more);
                }
                self.editor.texture = None;
                more = true;
            }
            if self.editor.trace && !self.held {
                self.editor.texture = Some(view.texture(painter));
                let camera = self.editor.viewport.camera();
                let scene = self.editor.viewport.shown();
                if let (Some(camera), Some(scene)) = (camera, scene) {
                    let drew = view.trace_frame(scene, (camera, self.editor.revision))?;
                    self.editor.trace_samples = view.trace_samples();
                    more |= drew || resized;
                }
            } else {
                self.editor.texture = None;
            }
        }
        let drew = self.editor.viewport.frame(gpu, painter, pixels_per_point);
        self.drawn |= drew;
        Ok(more || drew)
    }

    fn feed(&mut self, ctx: &egui::Context) {
        let was = self.modifiers;
        self.modifiers = ctx.input(|input| input.modifiers);
        if self.editor.play != Play::Playing || ctx.wants_keyboard_input() {
            return;
        }
        let Some(view) = self.view.as_deref_mut() else {
            return;
        };
        let (events, ppp) = ctx.input(|input| (input.events.clone(), input.pixels_per_point));
        if let Some(driver) = view.driver()
            && crate::input::learn(driver.labels_mut(), &events)
        {
            self.labels = driver.labels().clone();
        }
        let Some(play) = view.session() else {
            return;
        };
        let origin = self.editor.screen.min;
        for event in crate::input::events(&events, ppp, was, self.modifiers) {
            play.feed(match event {
                InputEvent::PointerMoved { window: [x, y] } => InputEvent::PointerMoved {
                    window: [x - origin.x * ppp, y - origin.y * ppp],
                },
                event => event,
            });
        }
    }
}

impl App for Session {
    fn ui(&mut self, ctx: &egui::Context) {
        self.seconds = ctx.input(|input| input.unstable_dt).clamp(0.0, MAX_SECONDS);
        self.feed(ctx);
        self.editor.ui(ctx);
    }

    fn viewport(&self) -> bool {
        true
    }

    fn theme(&self) -> Theme {
        crate::theme::pfx()
    }

    fn frame(&mut self, gpu: &Gpu, renderer: &mut Renderer, pixels_per_point: f32) -> bool {
        match self.update(gpu, renderer, pixels_per_point) {
            Ok(more) => more,
            Err(error) => {
                self.editor.log.error(error);
                false
            }
        }
    }

    fn forget(&mut self) {
        if let Some(view) = &mut self.view {
            view.stop_play(&mut self.editor, false);
        }
        self.view = None;
        self.editor.texture = None;
        self.editor.viewport.forget();
        self.drawn = false;
    }

    fn name(&self) -> &str {
        "pfx edit"
    }

    fn label(&self) -> Option<String> {
        Some(match &self.editor.project {
            Some(project) => format!("{} · {}", project.name(), self.editor.name()),
            None => self.editor.name(),
        })
    }

    fn watched(&self) -> Watch {
        Watch {
            files: self.watched.clone(),
            ..Watch::default()
        }
    }

    fn changed(&mut self, files: &[PathBuf]) {
        if files.iter().any(|file| !self.modules.contains(file)) {
            self.editor.outside = true;
        }
        if files.iter().any(|file| self.modules.contains(file)) {
            self.poll_modules();
        }
    }

    fn waker(&mut self, wake: Wake) {
        self.wake = Some(wake.clone());
        self.editor.viewport.set_wake(wake.clone());
        if self.setup.pgpu && self.status.is_none() {
            self.status = Some(status_watch(move || wake()));
        }
    }

    fn opened(&mut self) -> bool {
        let mut files = self.editor.files();
        files.extend(self.modules.iter().cloned());
        if files == self.watched {
            return false;
        }
        self.watched = files;
        true
    }

    fn pgpu(&mut self, state: Pgpu) {
        if !self.setup.pgpu {
            return;
        }
        let held = state.paused() && !self.playing();
        if held == self.held {
            return;
        }
        self.held = held;
        self.editor
            .viewport
            .set_pgpu(if held { state } else { Pgpu::Free });
        if held {
            self.editor
                .log
                .info("pgpu: a game is playing; the viewport waits");
        } else {
            self.editor.log.info("pgpu: the viewport draws again");
        }
    }

    fn timing(&self) -> Timing {
        match self.view.as_deref().and_then(View::renderer) {
            Some(renderer) => {
                let drops = renderer.timing_drops();
                Timing {
                    gpu_ms: renderer.timings().and_then(|timings| timings.gpu_ms()),
                    drops: drops.ring + drops.late,
                    drawn: self.drawn,
                }
            }
            None => self.editor.viewport.timing(),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(view) = &mut self.view {
            view.stop_play(&mut self.editor, false);
        }
        self.editor.end_group();
    }
}
