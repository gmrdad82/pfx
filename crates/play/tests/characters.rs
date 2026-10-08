use std::path::Path;

use pfx_core::clock::Tick;
use pfx_gpu::Gpu;
use pfx_gpu::pace::Pace;
use pfx_input::{ActionSpec, Binding, Input, InputEvent, Key, Recording};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;
use pfx_physics::rigid::{CharacterDesc, Pose};
use pfx_play::{Game, Options, PlaySession, World};

mod common;

const WIDTH: u32 = 160;
const HEIGHT: u32 = 96;
const SEED: u32 = 3;
const TICKS: u32 = 240;
const FRAME: f32 = 1.0 / 60.0;

struct Runner;

impl Game for Runner {
    fn actions(&self) -> Vec<ActionSpec> {
        vec![
            ActionSpec::digital("left").key(Binding::key(Key::A)),
            ActionSpec::digital("right").key(Binding::key(Key::D)),
            ActionSpec::digital("jump").key(Binding::key(Key::Space)),
        ]
    }

    fn start(&mut self, world: &mut World) {
        let player = world.object("player").unwrap();
        let desc = CharacterDesc {
            speed: 6.0,
            ..*world.character(player).unwrap().desc()
        };
        world.set_character_desc(player, desc).unwrap();
    }

    fn tick(&mut self, world: &mut World, _tick: &Tick, input: &Input) {
        let held = |name: &str| f32::from(u8::from(input.action(name).held));
        let lift = world.body(world.object("lift").unwrap()).unwrap();
        let phase = (world.ticks().saturating_sub(60) % 240) as f32 / 120.0;
        let swing = if phase < 1.0 { phase } else { 2.0 - phase };
        world
            .physics
            .move_to(lift, Pose::at([-3.0, -0.2 + 2.0 * swing, 0.0]));
        let player = world.object("player").unwrap();
        world.character(player).unwrap().drive(
            [held("right") - held("left"), 0.0, 0.0],
            input.action("jump").held,
        );
    }
}

fn scene() -> Scene {
    Scene::open(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/characters/platformer/platformer.scene.toml"),
    )
    .unwrap()
}

fn feet(session: &PlaySession) -> [u32; 3] {
    let world = session.world();
    world
        .character_of(world.object("player").unwrap())
        .unwrap()
        .feet()
        .map(f32::to_bits)
}

fn recorded() -> (Recording, [u32; 3]) {
    let options = Options {
        record: true,
        ..Options::default()
    };
    let mut session = PlaySession::play_with(&scene(), None, Box::new(Runner), options).unwrap();
    session.feed(InputEvent::Key {
        key: Key::D,
        pressed: true,
    });
    for tick in 0..TICKS {
        if tick % 50 == 30 || tick % 50 == 45 {
            session.feed(InputEvent::Key {
                key: Key::Space,
                pressed: tick % 50 == 30,
            });
        }
        session.advance(FRAME);
    }
    let end = feet(&session);
    (session.take_recording().unwrap(), end)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_platformer_plays_its_recorded_input_to_the_same_positions_while_drawn() {
    let (recording, headless) = recorded();
    let scene = scene();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = common::output(&gpu, "characters output", [WIDTH, HEIGHT]);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let staged = renderer.stage(&scene).unwrap();
    let options = Options {
        replay: Some(recording),
        ..Options::default()
    };
    let mut session =
        PlaySession::play_with(&scene, Some(&staged), Box::new(Runner), options).unwrap();
    let mut pace = Pace::new();
    let mut draw = |session: &mut PlaySession, renderer: &mut Renderer| {
        let finish = staged.finish();
        let drawn = session
            .present(&staged, renderer, WIDTH as f32 / HEIGHT as f32, SEED)
            .unwrap();
        pace.frame(|| {
            renderer
                .render(
                    &drawn.scene,
                    &drawn.text,
                    &drawn.effects,
                    finish,
                    &output.view,
                )
                .unwrap()
        });
        renderer.gpu().readback_rgba16(&output).unwrap()
    };
    let first = draw(&mut session, &mut renderer);
    let mut last = first.clone();
    while session.world().ticks() < u64::from(TICKS) {
        session.advance(FRAME);
        last = draw(&mut session, &mut renderer);
    }
    assert_eq!(feet(&session), headless, "drawing changes nothing it plays");
    let [x, y, z] = feet(&session).map(f32::from_bits);
    assert_eq!(z, 0.0);
    assert!(
        x > 14.5 && x < 17.75,
        "past the ledge, short of the wall: {x}"
    );
    assert!(y > 1.5, "up by the wall: {y}");
    assert_ne!(first, last, "the player moved on screen");
}
