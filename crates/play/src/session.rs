use std::collections::BTreeMap;
use std::sync::Arc;

use pfx_core::clock::Tick;
use pfx_gpu::window::Viewport;
use pfx_input::{Actions, BindingMap, Input, InputEvent, MotorCommand, Recording};
use pfx_live::renderer::Renderer;
use pfx_live::scene::{Applied, Override, Staged};
use pfx_live::stats::FrameStats;
use pfx_load::scene::{Reload, Scene, SceneDiff, SceneError};

use crate::PlayError;
use crate::game::{FrameTime, Game};
use crate::hooks::{NoHooks, PlayHooks};
use crate::live::{Edit, Pending, Stamped};
use crate::present::{self, Presenter, Touched};
use crate::rigs::Rigs;
use crate::tunables::Tunables;
use crate::ui::{Ui, UiFrame};
use crate::world::{Exit, RumbleRequest, World};

pub const MOTORS_KEPT: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub seed: u64,
    pub audio_rate: u32,
    pub audio_channels: u16,
    pub max_ticks: u32,
    pub record: bool,
    pub replay: Option<Recording>,
    pub edits: Vec<Stamped>,
    pub tunables: Option<Tunables>,
    pub rigs: Option<Rigs>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            seed: 0,
            audio_rate: 48_000,
            audio_channels: 2,
            max_ticks: 8,
            record: false,
            replay: None,
            edits: Vec::new(),
            tunables: None,
            rigs: None,
        }
    }
}

pub struct PlaySession {
    snapshot: Arc<Scene>,
    overrides: BTreeMap<String, Override>,
    game: Box<dyn Game>,
    hooks: Box<dyn PlayHooks>,
    world: World,
    input: Input,
    pending: Vec<InputEvent>,
    replay: Option<(Recording, usize)>,
    edits: Vec<Stamped>,
    replay_edits: Option<(Vec<Stamped>, usize)>,
    held: Pending,
    latest: Option<Scene>,
    errors: Vec<SceneError>,
    max_ticks: u32,
    presenter: Presenter,
    motors: Vec<MotorCommand>,
    stats: FrameStats,
    #[cfg(feature = "dev-hud")]
    hud: pfx_live::hud::Hud,
}

pub struct Stopped {
    pub game: Box<dyn Game>,
    pub hooks: Box<dyn PlayHooks>,
    pub snapshot: Arc<Scene>,
    pub scene: Scene,
    pub diff: SceneDiff,
    pub errors: Vec<SceneError>,
    pub ticks: u64,
    pub pending: Pending,
    pub edits: Vec<Stamped>,
    overrides: BTreeMap<String, Override>,
    touched: Touched,
}

impl PlaySession {
    pub fn play(scene: &Scene, staged: &Staged, game: Box<dyn Game>) -> Result<Self, PlayError> {
        Self::play_with(scene, Some(staged), game, Options::default())
    }

    pub fn play_with(
        scene: &Scene,
        staged: Option<&Staged>,
        game: Box<dyn Game>,
        options: Options,
    ) -> Result<Self, PlayError> {
        Self::play_hooked(scene, staged, game, options, Box::new(NoHooks))
    }

    pub fn play_hooked(
        scene: &Scene,
        staged: Option<&Staged>,
        mut game: Box<dyn Game>,
        options: Options,
        mut hooks: Box<dyn PlayHooks>,
    ) -> Result<Self, PlayError> {
        let snapshot = Arc::new(scene.clone());
        let overrides = staged
            .map(|staged| staged.overrides().clone())
            .unwrap_or_default();
        let tunables = match options.tunables {
            Some(tunables) => tunables,
            None => Tunables::of_scene(&snapshot.path).map_err(PlayError::Tunables)?,
        };
        let rigs = match options.rigs {
            Some(rigs) => rigs,
            None => Rigs::of_scene(&snapshot.path).map_err(PlayError::Rigs)?,
        };
        let mut world = World::new(
            snapshot.clone(),
            options.seed,
            (options.audio_rate, options.audio_channels),
            tunables,
            rigs,
        )?;
        let actions = Actions::new(game.actions()).map_err(PlayError::Actions)?;
        let step_us = (1_000_000.0 / world.clock().tick_rate()).round() as u32;
        let mut input = Input::new(actions, step_us);
        let replay = options.replay.map(|recording| {
            input.begin_replay(&recording);
            (recording, 0)
        });
        if options.record && replay.is_none() {
            input.start_recording();
        }
        hooks.on_play(&snapshot);
        game.start(&mut world);
        world.render_audio();
        Ok(Self {
            snapshot,
            overrides,
            game,
            hooks,
            world,
            input,
            pending: Vec::new(),
            replay,
            edits: Vec::new(),
            replay_edits: (!options.edits.is_empty()).then_some((options.edits, 0)),
            held: Pending::default(),
            latest: None,
            errors: Vec::new(),
            max_ticks: options.max_ticks.max(1),
            stats: FrameStats::off(),
            presenter: Presenter::default(),
            motors: Vec::new(),
            #[cfg(feature = "dev-hud")]
            hud: pfx_live::hud::Hud::new(),
        })
    }

    pub fn set_frame_stats(&mut self, stats: FrameStats) {
        self.stats = stats;
    }

    pub fn advance(&mut self, real_seconds: f32) -> Tick {
        self.stats.sim_begin();
        let tick = self.advance_ticks(real_seconds);
        self.stats.sim_end();
        tick
    }

    fn advance_ticks(&mut self, real_seconds: f32) -> Tick {
        if self.world.exit().is_some() {
            return Tick::default();
        }
        let mut tick = self.world.clock_mut().advance(real_seconds);
        tick.ticks = tick.ticks.min(self.max_ticks);
        for ran in 0..tick.ticks {
            if self.world.exit().is_some() {
                tick.ticks = ran;
                return tick;
            }
            self.run(&tick);
        }
        if self.world.exit().is_some() {
            return tick;
        }
        self.world.fx.shake.step(tick.paced, 1.0);
        self.world.apply_rig(tick.alpha);
        self.frame(FrameTime {
            alpha: tick.alpha,
            seconds: tick.real,
        });
        tick
    }

    fn frame(&mut self, time: FrameTime) {
        if let Err(error) = self.game.try_frame(&self.world, time) {
            self.world.fail(error);
        }
    }

    pub fn step(&mut self) -> bool {
        if !self.paused() || self.world.exit().is_some() {
            return false;
        }
        let dt = self.world.clock().tick_dt() as f32;
        let tick = Tick {
            ticks: 1,
            dt,
            paced: dt,
            real: 0.0,
            scale: 0.0,
            alpha: 0.0,
        };
        self.stats.sim_begin();
        self.run(&tick);
        self.stats.sim_end();
        if self.world.exit().is_none() {
            self.world.apply_rig(1.0);
            self.frame(FrameTime::default());
        }
        true
    }

    fn run(&mut self, tick: &Tick) {
        self.replay_due();
        self.world.begin_tick();
        match &mut self.replay {
            Some((recording, at)) => {
                if let Some(events) = recording.steps.get(*at) {
                    for event in events {
                        self.input.feed(event.clone());
                    }
                }
                *at += 1;
            }
            None => {
                for event in self.pending.drain(..) {
                    self.input.feed(event);
                }
            }
        }
        let step = self.input.step();
        if self.replay.is_none() {
            self.motors.extend(step.motors);
            let extra = self.motors.len().saturating_sub(MOTORS_KEPT);
            self.motors.drain(..extra);
        }
        if let Err(error) = self.game.try_tick(&mut self.world, tick, &self.input) {
            self.world.fail(error);
        }
        if self.world.exit().is_some() {
            return;
        }
        for request in self.world.take_rumble() {
            match request {
                RumbleRequest::Active(rumble) => self.input.rumble(rumble),
                RumbleRequest::Pad(pad, rumble) => self.input.rumble_pad(pad, rumble),
            };
        }
        self.world.step_physics();
        if !self.world.events().is_empty() {
            let events = self.world.events().to_vec();
            self.game.events(&mut self.world, &events);
        }
        self.world.step_animations();
        if !self.world.animation_events().is_empty() {
            let events = self.world.animation_events().to_vec();
            self.game.animated(&mut self.world, &events);
        }
        self.world.step_rigs(&self.input);
        self.world.render_audio();
    }

    fn replay_due(&mut self) {
        let Some((edits, at)) = &mut self.replay_edits else {
            return;
        };
        let now = self.world.ticks();
        let mut due = Vec::new();
        while let Some(stamped) = edits.get(*at) {
            if stamped.tick > now {
                break;
            }
            if stamped.tick == now {
                due.push(stamped.edit.clone());
            }
            *at += 1;
        }
        for edit in due {
            let _ = self.apply(edit);
        }
    }

    fn apply(&mut self, edit: Edit) -> Result<(), String> {
        let kept = self.world.apply(&edit)?;
        self.edits.push(Stamped {
            tick: self.world.ticks(),
            edit,
        });
        self.held.hold(kept);
        Ok(())
    }

    pub fn edit(&mut self, edit: Edit) -> Result<(), String> {
        if self.replay_edits.is_some() {
            return Err(format!(
                "{}: this session replays recorded edits",
                edit.key()
            ));
        }
        self.apply(edit)
    }

    pub fn edits(&self) -> &[Stamped] {
        &self.edits
    }

    pub fn pending(&self) -> &Pending {
        &self.held
    }

    pub fn pause(&mut self) {
        if !self.paused() {
            self.world.clock_mut().pause();
            self.hooks.on_pause();
        }
    }

    pub fn resume(&mut self) {
        if self.paused() {
            self.world.clock_mut().resume();
            self.hooks.on_resume();
        }
    }

    pub fn paused(&self) -> bool {
        self.world.clock().paused()
    }

    pub fn feed(&mut self, event: InputEvent) {
        #[cfg(feature = "dev-hud")]
        if self.hud.input(&event) {
            return;
        }
        if self.replay.is_none() {
            self.pending.push(event);
        }
    }

    pub fn input(&self) -> &Input {
        &self.input
    }

    pub fn recording(&self) -> Option<&Recording> {
        self.input.recording()
    }

    pub fn take_motors(&mut self) -> Vec<MotorCommand> {
        std::mem::take(&mut self.motors)
    }

    pub fn set_rumble_intensity(&mut self, percent: u8) {
        self.input.set_rumble_intensity(percent);
    }

    pub fn set_bindings(&mut self, map: BindingMap) {
        self.input.set_bindings(map);
    }

    pub fn set_viewport(&mut self, viewport: Option<Viewport>) {
        self.input.set_viewport(viewport);
    }

    pub fn game(&self) -> &dyn Game {
        self.game.as_ref()
    }

    pub fn game_mut(&mut self) -> &mut dyn Game {
        self.game.as_mut()
    }

    pub fn ui(&mut self, frame: &UiFrame) -> Ui {
        if self.world.exit().is_some() {
            return Ui::default();
        }
        match self.game.try_ui(&self.world, frame) {
            Ok(ui) => ui,
            Err(error) => {
                self.world.fail(error);
                Ui::default()
            }
        }
    }

    pub fn exit(&self) -> Option<&Exit> {
        self.world.exit()
    }

    pub fn set_display(&mut self, display: &pfx_gpu::screens::DisplaySettings) {
        self.world.set_display(display);
    }

    pub fn take_recording(&mut self) -> Option<Recording> {
        self.input.stop_recording()
    }

    pub fn queue(&mut self, reload: Result<Reload, SceneError>) {
        match reload {
            Ok(reload) => {
                self.latest = Some(reload.scene);
                self.errors.clear();
            }
            Err(error) => self.errors.push(error),
        }
    }

    pub fn queued(&self) -> Option<&Scene> {
        self.latest.as_ref()
    }

    pub fn snapshot(&self) -> &Scene {
        &self.snapshot
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    pub fn take_audio(&mut self) -> Vec<f32> {
        self.world.sounds.take_audio()
    }

    pub fn time(&self) -> f32 {
        let rate = self.world.clock().tick_rate();
        let alpha = if self.paused() {
            0.0
        } else {
            f64::from(self.world.clock().alpha())
        };
        ((self.world.ticks() as f64 + alpha) / rate) as f32
    }

    pub fn present<'a>(
        &'a mut self,
        staged: &'a Staged,
        renderer: &mut Renderer,
        aspect: f32,
        seed: u32,
    ) -> Result<present::PlayFrame<'a>, PlayError> {
        let time = self.time();
        self.world.set_view_aspect(aspect);
        self.presenter
            .present(&self.world, staged, renderer, aspect, time, seed)
    }

    #[cfg(feature = "dev-hud")]
    pub fn hud(&self) -> &pfx_live::hud::Hud {
        &self.hud
    }

    #[cfg(feature = "dev-hud")]
    pub fn hud_mut(&mut self) -> &mut pfx_live::hud::Hud {
        &mut self.hud
    }

    #[cfg(feature = "dev-hud")]
    pub fn draw_hud_on(
        &mut self,
        renderer: &mut Renderer,
        output: &pfx_gpu::wgpu::TextureView,
        format: pfx_gpu::wgpu::TextureFormat,
        size: [u32; 2],
    ) {
        let stats = pfx_live::hud::Hud::attach(renderer);
        if !self.stats.enabled() {
            self.stats = stats;
        }
        self.hud.draw_on(renderer, output, format, size);
    }

    #[cfg(not(feature = "dev-hud"))]
    pub fn draw_hud_on(
        &mut self,
        _renderer: &mut Renderer,
        _output: &pfx_gpu::wgpu::TextureView,
        _format: pfx_gpu::wgpu::TextureFormat,
        _size: [u32; 2],
    ) {
    }

    pub fn stop(mut self) -> Stopped {
        self.game.stop();
        self.hooks.on_stop();
        let ticks = self.world.ticks();
        drop(self.world);
        let scene = self
            .latest
            .take()
            .unwrap_or_else(|| (*self.snapshot).clone());
        Stopped {
            diff: self.snapshot.diff(&scene),
            game: self.game,
            hooks: self.hooks,
            snapshot: self.snapshot,
            scene,
            errors: self.errors,
            ticks,
            pending: self.held,
            edits: self.edits,
            overrides: self.overrides,
            touched: self.presenter.touched(),
        }
    }
}

impl Stopped {
    pub fn restore(
        &self,
        staged: &mut Staged,
        renderer: &mut Renderer,
    ) -> Result<Applied, PlayError> {
        staged.clear_overrides();
        for (object, change) in &self.overrides {
            staged.set_override(object, *change);
        }
        let (width, height) = renderer.size();
        renderer.set_lens(staged.lens(width as f32 / height.max(1) as f32));
        present::untouch(&self.snapshot, self.touched, renderer)?;
        if self.touched.posed {
            renderer.set_poses(&[]).map_err(PlayError::Render)?;
        }
        if self.touched.sprites {
            renderer
                .set_surfaces(staged.surfaces())
                .map_err(PlayError::Render)?;
        }
        present::cut(renderer)?;
        staged
            .apply(renderer, &self.scene, &self.diff)
            .map_err(PlayError::Render)
    }
}
