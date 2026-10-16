use crate::camera::{Picture, picture_in};
use crate::cut::Cut;
use crate::script::{Kinds, Scene, Scheme, Script, frames};
use pfx_input::InputEvent;
use pfx_live::flat::Matrix;
use pfx_live::flat::look::Look;
use std::fmt::Debug;

pub trait Cue: Clone + Debug {
    fn at(&self) -> u64;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Take<C> {
    pub frames: u64,
    pub cue: C,
}

pub struct FilmFrame<'a, D> {
    pub index: u64,
    pub fps: u32,
    pub speed: f32,
    pub cut: &'a Cut<D>,
}

impl<D> FilmFrame<'_, D> {
    pub fn picture(&self, out: [u32; 2]) -> Picture {
        picture_in(self.cut.grade.camera.wide, self.cut.grade.bars, out)
    }

    pub fn view(&self, out: [u32; 2]) -> Option<Matrix> {
        self.cut.grade.camera.view(self.picture(out))
    }

    pub fn game_look(&self, out: [u32; 2]) -> Look {
        self.cut.grade.game_look(out)
    }

    pub fn over_look(&self, out: [u32; 2]) -> Look {
        self.cut.grade.over_look(out)
    }

    pub fn seconds(&self) -> f64 {
        f64::from(self.speed.max(0.0)) / f64::from(self.fps.max(1))
    }
}

pub type Drawing<S> = <<S as Show>::Kinds as Kinds>::Drawn;

pub trait Show {
    type Kinds: Kinds;
    type Cue: Cue;

    fn plan(&mut self, script: &Script<Self::Kinds>) -> Result<Vec<Self::Cue>, String>;
    fn set_theme(&mut self, scheme: Option<Scheme>);
    fn seek(&mut self, scene: &Scene<Self::Kinds>, take: &Take<Self::Cue>) -> Result<(), String>;
    fn aim(&self, scene: &Scene<Self::Kinds>) -> Option<[f32; 2]>;
    fn tick(&mut self, frame: u64, input: &[InputEvent]);
    fn draw(&mut self, frame: &FilmFrame<'_, Drawing<Self>>) -> Result<(), String>;
    fn audio(&mut self, out: &mut Vec<f32>);
    fn readback(&mut self) -> Result<Vec<u8>, String>;

    fn ticks(&self) -> u64 {
        0
    }

    fn finish(&mut self) -> Result<(), String> {
        Ok(())
    }
}

pub fn plan<S: Show>(
    show: &mut S,
    script: &Script<S::Kinds>,
    fps: u32,
) -> Result<Vec<Take<S::Cue>>, String> {
    script.off_grid(fps)?;
    let cues = show.plan(script)?;
    if cues.len() != script.scenes.len() {
        return Err(format!(
            "the show planned {} of {} scenes",
            cues.len(),
            script.scenes.len()
        ));
    }
    Ok(script
        .scenes
        .iter()
        .zip(cues)
        .map(|(scene, cue)| Take {
            frames: frames(scene.seconds, fps),
            cue,
        })
        .collect())
}
