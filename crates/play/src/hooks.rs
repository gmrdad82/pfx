use pfx_load::scene::Scene;

pub trait PlayHooks {
    fn on_play(&mut self, _scene: &Scene) {}

    fn on_pause(&mut self) {}

    fn on_resume(&mut self) {}

    fn on_stop(&mut self) {}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoHooks;

impl PlayHooks for NoHooks {}
