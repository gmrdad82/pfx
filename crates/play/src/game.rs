use pfx_core::clock::Tick;
use pfx_core::modules::Reload;
use pfx_gpu::wgpu;
use pfx_input::{ActionSpec, Input};
use pfx_live::renderer::Renderer;
use pfx_load::scene::Cue;

use crate::events::WorldEvent;
use crate::paint::{PaintFrame, Warm, Warming};
use crate::settings::Settings;
use crate::ui::{Ui, UiFrame};
use crate::world::{AnimationEvent, World};

pub type GameError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameTime {
    pub alpha: f32,
    pub seconds: f32,
}

pub trait Game {
    fn actions(&self) -> Vec<ActionSpec> {
        Vec::new()
    }

    fn start(&mut self, world: &mut World);

    fn tick(&mut self, _world: &mut World, _tick: &Tick, _input: &Input) {}

    fn try_tick(&mut self, world: &mut World, tick: &Tick, input: &Input) -> Result<(), GameError> {
        self.tick(world, tick, input);
        Ok(())
    }

    fn events(&mut self, _world: &mut World, _events: &[WorldEvent]) {}

    fn animated(&mut self, _world: &mut World, _events: &[AnimationEvent]) {}

    fn frame(&mut self, _world: &World, _alpha: f32) {}

    fn try_frame(&mut self, world: &World, time: FrameTime) -> Result<(), GameError> {
        self.frame(world, time.alpha);
        Ok(())
    }

    fn ui(&mut self, _world: &World, _frame: &UiFrame) -> Ui {
        Ui::default()
    }

    fn warm(&mut self, _warm: &mut Warm<'_>) -> Result<Warming, GameError> {
        Ok(Warming::Done)
    }

    fn paint(
        &mut self,
        _renderer: &mut Renderer,
        _target: &wgpu::TextureView,
        _frame: &PaintFrame,
    ) -> Result<(), GameError> {
        Ok(())
    }

    fn try_ui(&mut self, world: &World, frame: &UiFrame) -> Result<Ui, GameError> {
        Ok(self.ui(world, frame))
    }

    fn settings(&self) -> Option<&Settings> {
        None
    }

    fn stop(&mut self) {}

    fn modules(&mut self) -> Option<&mut dyn Reload> {
        None
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SceneGame;

impl Game for SceneGame {
    fn start(&mut self, world: &mut World) {
        world.pose_movers();
        world.play_cued(Cue::Start);
    }

    fn tick(&mut self, world: &mut World, _tick: &Tick, _input: &Input) {
        world.play_hits();
        world.pose_movers();
    }
}
