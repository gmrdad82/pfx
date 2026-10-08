use pfx_core::clock::Tick;
use pfx_input::{ActionSpec, Binding, Input, Key};
use pfx_load::scene::{Camera, Matrix};
use pfx_play::{Game, SceneGame, World};

pub const MOVE_SPEED: f32 = 2.0;
pub const TURN_SPEED: f32 = 60.0;

pub fn actions() -> Vec<ActionSpec> {
    vec![
        ActionSpec::axis("walk").key(Binding::keys(Key::S, Key::W)),
        ActionSpec::axis("strafe").key(Binding::keys(Key::A, Key::D)),
        ActionSpec::axis("lift").key(Binding::keys(Key::Q, Key::E)),
        ActionSpec::axis("turn").key(Binding::keys(Key::Left, Key::Right)),
        ActionSpec::axis("tilt").key(Binding::keys(Key::Down, Key::Up)),
    ]
}

fn add(a: [f32; 3], b: [f32; 3], scale: f32) -> [f32; 3] {
    [
        a[0] + b[0] * scale,
        a[1] + b[1] * scale,
        a[2] + b[2] * scale,
    ]
}

fn forward(yaw: f32, pitch: f32) -> [f32; 3] {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    [cp * sy, sp, -cp * cy]
}

pub struct BenchGame {
    scene: SceneGame,
    camera: Option<String>,
    yaw: f32,
    pitch: f32,
}

impl BenchGame {
    pub fn new(camera: Option<String>) -> Self {
        Self {
            scene: SceneGame,
            camera,
            yaw: 0.0,
            pitch: 0.0,
        }
    }

    fn aim(&mut self, world: &World) {
        let camera = world.camera;
        let (direction, _, _) = camera.axes();
        self.yaw = direction[0].atan2(-direction[2]);
        self.pitch = direction[1].clamp(-1.0, 1.0).asin();
    }

    fn look(&self, world: &mut World) {
        let at = world.camera.at;
        world.camera.look_at = add(at, forward(self.yaw, self.pitch), 1.0);
    }
}

fn placed(model: Matrix, base: Camera) -> Camera {
    let at = [model[3][0], model[3][1], model[3][2]];
    let ahead = [-model[2][0], -model[2][1], -model[2][2]];
    let up = [model[1][0], model[1][1], model[1][2]];
    Camera {
        at,
        look_at: add(at, ahead, 1.0),
        up,
        ..base
    }
}

impl Game for BenchGame {
    fn actions(&self) -> Vec<ActionSpec> {
        actions()
    }

    fn start(&mut self, world: &mut World) {
        self.scene.start(world);
        if let Some(id) = self.camera.as_deref().and_then(|name| world.object(name)) {
            world.camera = placed(world.model(id), world.camera);
        }
        self.aim(world);
    }

    fn tick(&mut self, world: &mut World, tick: &Tick, input: &Input) {
        self.scene.tick(world, tick, input);
        let dt = tick.dt;
        let walk = input.action("walk").value;
        let strafe = input.action("strafe").value;
        let lift = input.action("lift").value;
        let turn = input.action("turn").value;
        let tilt = input.action("tilt").value;
        if walk == 0.0 && strafe == 0.0 && lift == 0.0 && turn == 0.0 && tilt == 0.0 {
            return;
        }
        self.yaw += (turn * TURN_SPEED * dt).to_radians();
        self.pitch = (self.pitch + (tilt * TURN_SPEED * dt).to_radians()).clamp(-1.5, 1.5);
        let (ahead, right, _) = {
            let mut aimed = world.camera;
            aimed.look_at = add(aimed.at, forward(self.yaw, self.pitch), 1.0);
            aimed.axes()
        };
        let mut at = world.camera.at;
        at = add(at, ahead, walk * MOVE_SPEED * dt);
        at = add(at, right, strafe * MOVE_SPEED * dt);
        at = add(at, world.camera.up, lift * MOVE_SPEED * dt);
        world.camera.at = at;
        self.look(world);
    }

    fn stop(&mut self) {
        self.scene.stop();
    }
}
