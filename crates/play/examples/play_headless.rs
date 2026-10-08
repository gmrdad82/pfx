use std::fs::File;
use std::path::{Path, PathBuf};

use pfx_gpu::pace::{Pace, WallClock};
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::Staged;
use pfx_load::scene::Scene;
use pfx_play::{PlaySession, SceneGame};
use pfx_sound::Clip;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;
const SEED: u32 = 1;
const FRAME: f32 = 1.0 / 60.0;

const PLAY: &str = r#"format = 1

include = ["room.scene.toml"]

[mesh.ball]
file = "ball.gltf"

[[object]]
name = "ball"
mesh = "ball"
at = [0.3, 1.8, 0.7]
scale = 0.35
material = "blue"

[object.body]
shape = "sphere"
restitution = 0.45

[[object]]
name = "box"
mesh = "block"
at = [-0.2, 2.4, 0.4]
rotate = [20.0, 35.0, 10.0]
scale = 0.3
material = "metal"

[object.body]
friction = 0.6

[[mover]]
name = "spin"
objects = ["right pillar"]
kind = "turn"
pivot = [1.4, 0.0, -0.6]
axis = [0.0, 1.0, 0.0]
travel = [0.0, 90.0]
period = 3.0

[sound.knock]
file = "knock.wav"
play = "hit"
object = "ball"
"#;

fn scene_folder(out: &Path) -> PathBuf {
    let folder = out.join("scene");
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    for entry in std::fs::read_dir(fixtures).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), folder.join(entry.file_name())).unwrap();
    }
    let knock = Clip {
        rate: 48_000,
        channels: 1,
        samples: (0..4_800)
            .map(|i| {
                let t = i as f32 / 48_000.0;
                (t * 220.0 * std::f32::consts::TAU).sin() * (-t * 40.0).exp() * 0.6
            })
            .collect(),
    };
    std::fs::write(folder.join("knock.wav"), knock.to_wav()).unwrap();
    std::fs::write(folder.join("play.scene.toml"), PLAY).unwrap();
    let room = folder.join("room.scene.toml");
    let mut text = std::fs::read_to_string(&room).unwrap();
    for name in ["crate", "floor"] {
        let start = text.find(&format!("name = \"{name}\"\n")).unwrap();
        let end = text[start..]
            .find("\n\n")
            .map_or(text.len(), |at| start + at + 1);
        text.insert_str(end, "\n[object.body]\nkind = \"fixed\"\n");
    }
    std::fs::write(&room, text).unwrap();
    folder
}

fn output(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("play headless output"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    OffscreenTarget {
        texture,
        view,
        format: wgpu::TextureFormat::Rgba16Float,
        width: WIDTH,
        height: HEIGHT,
    }
}

struct Viewer {
    renderer: Renderer,
    output: OffscreenTarget,
    pace: Pace<WallClock>,
    out: PathBuf,
    stills: u32,
}

impl Viewer {
    fn save(&mut self, name: &str) -> Vec<u16> {
        let values = self.renderer.gpu().readback_rgba16(&self.output).unwrap();
        let bytes: Vec<u8> = values
            .iter()
            .map(|&value| {
                (half::f16::from_bits(value).to_f32().clamp(0.0, 1.0) * 255.0).round() as u8
            })
            .collect();
        let path = self.out.join(format!("{:02}-{name}.png", self.stills));
        self.stills += 1;
        let mut encoder = png::Encoder::new(File::create(&path).unwrap(), WIDTH, HEIGHT);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&bytes)
            .unwrap();
        println!("wrote {}", path.display());
        values
    }

    fn edited(&mut self, staged: &mut Staged) {
        let finish = staged.finish();
        let drawn = staged.frame(WIDTH as f32 / HEIGHT as f32, 0.0, SEED);
        let renderer = &mut self.renderer;
        let view = &self.output.view;
        self.pace.frame(|| {
            renderer
                .render(&drawn.scene, &drawn.text, &drawn.effects, finish, view)
                .unwrap()
        });
    }

    fn played(&mut self, session: &mut PlaySession, staged: &Staged) {
        let finish = staged.finish();
        let renderer = &mut self.renderer;
        let drawn = session
            .present(staged, renderer, WIDTH as f32 / HEIGHT as f32, SEED)
            .unwrap();
        let view = &self.output.view;
        self.pace.frame(|| {
            renderer
                .render(&drawn.scene, &drawn.text, &drawn.effects, finish, view)
                .unwrap()
        });
    }

    fn run(&mut self, session: &mut PlaySession, staged: &Staged, frames: u32) {
        for _ in 0..frames {
            session.advance(FRAME);
            self.played(session, staged);
        }
    }
}

fn main() {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/play_headless");
    let folder = scene_folder(&out);
    let scene = Scene::open(folder.join("play.scene.toml")).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = output(&gpu);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = renderer.stage(&scene).unwrap();
    let mut viewer = Viewer {
        renderer,
        output,
        pace: Pace::new(),
        out,
        stills: 0,
    };
    viewer.edited(&mut staged);
    let edited = viewer.save("edited");

    let mut session = PlaySession::play(&scene, &staged, Box::new(SceneGame)).unwrap();
    viewer.played(&mut session, &staged);
    let first = viewer.save("play-first-frame");
    viewer.run(&mut session, &staged, 20);
    viewer.save("play-falling");
    viewer.run(&mut session, &staged, 40);
    viewer.save("play-landed");
    session.pause();
    viewer.run(&mut session, &staged, 10);
    viewer.save("paused");
    for _ in 0..15 {
        session.step();
    }
    viewer.played(&mut session, &staged);
    viewer.save("stepped-15");
    session.resume();
    viewer.run(&mut session, &staged, 60);
    viewer.save("resumed");
    let audio = session.take_audio();
    let rate = session.world().sounds.rate();
    let channels = session.world().sounds.channels();
    let hits = session
        .world()
        .sounds
        .log()
        .iter()
        .filter(|(_, name)| name == "knock")
        .count();
    let ticks = session.world().ticks();
    let stopped = session.stop();
    stopped.restore(&mut staged, &mut viewer.renderer).unwrap();
    viewer.edited(&mut staged);
    let restored = viewer.save("stopped");

    let mut again = PlaySession::play(&stopped.scene, &staged, stopped.game).unwrap();
    viewer.played(&mut again, &staged);
    let replayed = viewer.save("play-again-first-frame");
    let stopped = again.stop();
    stopped.restore(&mut staged, &mut viewer.renderer).unwrap();

    let wav = viewer.out.join("play-audio.wav");
    std::fs::write(
        &wav,
        Clip {
            rate,
            channels,
            samples: audio,
        }
        .to_wav(),
    )
    .unwrap();
    println!("wrote {}", wav.display());
    println!("ticks played: {ticks}, knocks heard: {hits}");
    println!("stop restored the edited pixels: {}", restored == edited);
    println!(
        "play again gave the same first frame: {}",
        replayed == first
    );
    println!(
        "the first play frame is the edited frame: {}",
        first == edited
    );
}
