use std::fs;
use std::path::{Path, PathBuf};

use pfx_editor::script::frames;
use pfx_editor::session::{Session, Setup};
use pfx_editor_shell::{Script, block_on, pixels};
use pfx_game::{Config, Headless};
use pfx_gpu::window::Size;
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;
use pfx_play::SceneGame;

const SCENE: &str = r#"format = 1

[camera]
at = [0.0, 1.0, 4.0]
look_at = [0.0, 1.0, 0.0]
fov = 40.0

[sky]
kind = "room"

[sky.room]
width = 64
floor = [0.3, 0.3, 0.3]
wall = [0.3, 0.3, 0.3]
ceiling = [0.3, 0.3, 0.3]
"#;

const WINDOW: [u32; 2] = [320, 180];
const SHOT: [u32; 2] = [1280, 800];

fn scene_file() -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("colour-path");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("flat.scene.toml");
    fs::write(&path, SCENE).unwrap();
    path
}

fn flat(name: &str, pixels: &[u8], width: u32, area: [u32; 4]) -> [u8; 3] {
    let [x0, y0, x1, y1] = area;
    let at = |x: u32, y: u32| {
        let i = ((y * width + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    };
    let first = at(x0, y0);
    for y in y0..y1 {
        for x in x0..x1 {
            let pixel = at(x, y);
            assert!(
                pixel.iter().zip(first).all(|(a, b)| a.abs_diff(b) <= 1),
                "{name}: the background is not flat at {x} {y}: {pixel:?} against {first:?}"
            );
        }
    }
    first
}

fn inset([width, height]: [u32; 2]) -> [u32; 4] {
    [
        width / 8,
        height / 8,
        width - width / 8,
        height - height / 8,
    ]
}

fn target(gpu: &Gpu, format: wgpu::TextureFormat) -> OffscreenTarget {
    let [width, height] = WINDOW;
    if format != wgpu::TextureFormat::Rgba16Float {
        return gpu.offscreen(width, height, format).unwrap();
    }
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("colour path output"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    OffscreenTarget {
        texture,
        view,
        format,
        width,
        height,
    }
}

fn direct(path: &Path, format: wgpu::TextureFormat) -> Vec<u8> {
    let scene = Scene::open(path).unwrap();
    let gpu = block_on(Gpu::headless()).unwrap();
    let [width, height] = WINDOW;
    let mut renderer = Renderer::new_with_output_format(gpu, width, height, format).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = renderer.stage(&scene).unwrap();
    renderer.wait_sky().unwrap();
    let target = target(renderer.gpu(), format);
    let finish = staged.finish();
    for _ in 0..4 {
        let frame = staged.frame(width as f32 / height as f32, 0.0, 0);
        renderer
            .render(
                &frame.scene,
                &frame.text,
                &frame.effects,
                finish,
                &target.view,
            )
            .unwrap();
    }
    if format == wgpu::TextureFormat::Rgba16Float {
        return renderer
            .gpu()
            .readback_rgba16(&target)
            .unwrap()
            .into_iter()
            .map(|bits| (half::f16::from_bits(bits).to_f32().clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect();
    }
    renderer.gpu().readback_rgba8(&target).unwrap()
}

fn game(path: &Path) -> Vec<u8> {
    let gpu = block_on(Gpu::headless()).unwrap();
    let window = Size {
        width: WINDOW[0],
        height: WINDOW[1],
    };
    let config = Config::new("colour", "0.0.0", path).exposure(pfx_game::Exposure::Fixed(1.0));
    let mut run = Headless::with_gpu(SceneGame, config, gpu, window).unwrap();
    let painter = run.painter_mut().unwrap();
    assert_eq!(
        painter.renderer().output_format(),
        wgpu::TextureFormat::Rgba16Float
    );
    painter.renderer_mut().wait_sky().unwrap();
    run.run_for(0.1).unwrap();
    let drawn = run.drawn.unwrap();
    assert_eq!(drawn.render, WINDOW);
    assert_eq!(drawn.output, WINDOW);
    run.readback().unwrap()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_games_frame_and_the_editors_view_agree_in_colour() {
    let path = scene_file();
    let srgb = direct(&path, wgpu::TextureFormat::Rgba8UnormSrgb);
    let float = direct(&path, wgpu::TextureFormat::Rgba16Float);
    let framed = game(&path);
    let mut session = Session::open(
        &path,
        Setup {
            pgpu: false,
            audio: false,
        },
    )
    .unwrap();
    let shot = pixels(&mut session, SHOT, &Script::parse(&frames(4)).unwrap()).unwrap();
    let live = session.pixels().unwrap();
    let live_size = session.editor.viewport.live().unwrap().size();
    let image = session.editor.viewport.image().unwrap();
    let shown = [
        image.min.x.ceil() as u32,
        image.min.y.ceil() as u32,
        image.max.x.floor() as u32,
        image.max.y.floor() as u32,
    ];
    let shown = [
        shown[0] + (shown[2] - shown[0]) / 8,
        shown[1] + (shown[3] - shown[1]) / 8,
        shown[2] - (shown[2] - shown[0]) / 8,
        shown[1] + (shown[3] - shown[1]) * 3 / 8,
    ];
    let srgb = flat(
        "renderer to Rgba8UnormSrgb",
        &srgb,
        WINDOW[0],
        inset(WINDOW),
    );
    let float = flat("renderer to Rgba16Float", &float, WINDOW[0], inset(WINDOW));
    let framed = flat("pfx-game's painter", &framed, WINDOW[0], inset(WINDOW));
    let live = flat(
        "the viewport's image",
        &live,
        live_size[0],
        inset(live_size),
    );
    let shot = flat("the offscreen shot", &shot, SHOT[0], shown);
    eprintln!(
        "background: renderer sRGB {srgb:?}, renderer Rgba16Float {float:?}, game {framed:?}, viewport image {live:?}, editor shot {shot:?}"
    );
    for (name, got) in [
        ("renderer to Rgba16Float", float),
        ("pfx-game's painter", framed),
        ("the viewport's image", live),
        ("the offscreen shot", shot),
    ] {
        assert!(
            got.iter().zip(srgb).all(|(a, b)| a.abs_diff(b) <= 1),
            "{name}: {got:?} against the renderer's own sRGB output {srgb:?}"
        );
    }
}
