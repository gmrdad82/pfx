use std::path::PathBuf;

use pfx_core::daylight::{Daylight, REFERENCE_HOUR};
use pfx_geom::shapes::Shape;
use pfx_gpu::{Gpu, OffscreenTarget, PassTiming, wgpu};
use pfx_live::flat::{
    CurvePoint, Draw, Fill, FlatScene, FlatSprites, Light, Shadow, ShadowCurve, Shape as FlatShape,
    SpriteFill, SpriteFilter, Srgba,
};
use pfx_live::frame::{Camera, Instance, Matrix, MeshData, MeshHandle, Scene, SceneWind, Sun};
use pfx_live::renderer::{Effects, Exposure, Finish, Renderer, Text};
use pfx_live::shadow::{Quality, ReceiverBox};
use pfx_live::sky::{AnalyticSky, SkySource};
use pfx_materials::Material;
use pfx_post::{Chain, Pass as PostPass, Tone};

const ATLAS: [u32; 2] = [1280, 640];
const SHEET: [u32; 2] = [1200, 600];
const SCREEN: [u32; 2] = [3840, 2160];
const METRES_PER_PIXEL: f32 = 0.002;
const WARM: u32 = 60;
const FRAMES: u32 = 300;

struct Cell {
    at: [f32; 2],
    size: f32,
}

fn cells() -> Vec<Cell> {
    let mut cells = vec![
        Cell {
            at: [0.0, 0.0],
            size: 600.0,
        },
        Cell {
            at: [600.0, 0.0],
            size: 300.0,
        },
        Cell {
            at: [900.0, 0.0],
            size: 300.0,
        },
    ];
    cells.extend((0..8).map(|k| Cell {
        at: [
            600.0 + 120.0 * (k % 5) as f32,
            300.0 + 120.0 * (k / 5) as f32,
        ],
        size: 120.0,
    }));
    cells
}

fn frame_of(cell: &Cell) -> [f32; 4] {
    [
        cell.at[0] / ATLAS[0] as f32,
        cell.at[1] / ATLAS[1] as f32,
        (cell.at[0] + cell.size) / ATLAS[0] as f32,
        (cell.at[1] + cell.size) / ATLAS[1] as f32,
    ]
}

fn multiply(a: Matrix, b: Matrix) -> Matrix {
    pfx_live::frame::multiply(a, b)
}

fn translate(at: [f32; 3]) -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [at[0], at[1], at[2], 1.0],
    ]
}

fn scale(by: [f32; 3]) -> Matrix {
    [
        [by[0], 0.0, 0.0, 0.0],
        [0.0, by[1], 0.0, 0.0],
        [0.0, 0.0, by[2], 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn turn_y(angle: f32) -> Matrix {
    let (s, c) = angle.sin_cos();
    [
        [c, 0.0, -s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [s, 0.0, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn turn_z(angle: f32) -> Matrix {
    let (s, c) = angle.sin_cos();
    [
        [c, s, 0.0, 0.0],
        [-s, c, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn orthographic(half: [f32; 2], near: f32, far: f32) -> Matrix {
    [
        [1.0 / half[0], 0.0, 0.0, 0.0],
        [0.0, 1.0 / half[1], 0.0, 0.0],
        [0.0, 0.0, -1.0 / (far - near), 0.0],
        [0.0, 0.0, -near / (far - near), 1.0],
    ]
}

fn upload(renderer: &mut Renderer, shape: Shape) -> MeshHandle {
    let mesh = shape.mesh(0.004);
    renderer
        .upload_mesh(MeshData {
            positions: &mesh.positions,
            normals: &mesh.normals,
            tangents: &mesh.tangents,
            uvs: &mesh.uvs,
            uvs1: None,
            alpha: None,
            indices: &mesh.indices,
        })
        .unwrap()
}

struct Meshes {
    head: MeshHandle,
    hat: MeshHandle,
    eye: MeshHandle,
}

fn heads(meshes: &Meshes, cells: &[Cell], time: f32) -> Vec<Instance> {
    let mut instances = Vec::new();
    for (index, cell) in cells.iter().enumerate() {
        let centre = [cell.at[0] + cell.size * 0.5, cell.at[1] + cell.size * 0.57];
        let world = [
            (centre[0] - SHEET[0] as f32 * 0.5) * METRES_PER_PIXEL,
            (SHEET[1] as f32 * 0.5 - centre[1]) * METRES_PER_PIXEL,
            0.0,
        ];
        let radius = cell.size * 0.3 * METRES_PER_PIXEL;
        let yaw = 0.35 * (time * 1.3 + index as f32 * 0.9).sin();
        let head = multiply(
            translate(world),
            multiply(turn_y(yaw), scale([radius, radius * 1.1, radius])),
        );
        let id = instances.len() as u32 + 1;
        instances.push(Instance::new(meshes.head, head, index as u32 % 3, id));
        let hat = multiply(head, multiply(translate([0.0, 0.7, 0.0]), turn_z(0.18)));
        let id = instances.len() as u32 + 1;
        instances.push(Instance::new(meshes.hat, hat, 3 + index as u32 % 2, id));
        for side in [-1.0f32, 1.0] {
            let eye = multiply(head, translate([0.34 * side, 0.12, 0.86]));
            let id = instances.len() as u32 + 1;
            instances.push(Instance::new(meshes.eye, eye, 5, id));
        }
    }
    instances
}

fn materials() -> Vec<Material> {
    let plain = |base: [f32; 3], roughness: f32| Material {
        base,
        roughness,
        ..Material::default()
    };
    vec![
        plain([0.78, 0.56, 0.42], 0.6),
        plain([0.55, 0.38, 0.26], 0.6),
        plain([0.86, 0.7, 0.58], 0.55),
        plain([0.16, 0.24, 0.42], 0.5),
        plain([0.62, 0.14, 0.12], 0.45),
        plain([0.03, 0.03, 0.035], 0.25),
    ]
}

fn milliseconds(timings: &[PassTiming]) -> f64 {
    timings.iter().map(|timing| timing.milliseconds).sum()
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn write_png(path: &PathBuf, size: [u32; 2], rgba: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size[0], size[1]);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(rgba)
        .unwrap();
}

struct Board {
    draws: Vec<Draw>,
}

fn image(draw: Draw, cell: &Cell) -> Draw {
    Draw {
        fill: Fill::Image(SpriteFill::new(frame_of(cell))),
        shadow: Shadow::None,
        ..draw
    }
}

fn board(cells: &[Cell]) -> Board {
    let card = Srgba::hex(0xe9e4da);
    let ink = Srgba::hex(0x2b3442);
    let accent = Srgba::hex(0xf0b429);
    let mut draws = vec![
        Draw::rect([1920.0, 1080.0], [3840.0, 2160.0], 0.0)
            .fill(Srgba::hex(0x1b2230))
            .shadow(Shadow::None),
        Draw::rect([900.0, 1080.0], [760.0, 940.0], 40.0)
            .fill(card)
            .elevation(1.0),
        Draw::rect([2350.0, 700.0], [1240.0, 560.0], 40.0)
            .fill(card)
            .elevation(1.0),
        Draw::rect([2350.0, 1520.0], [1240.0, 520.0], 40.0)
            .fill(card)
            .elevation(1.0),
        Draw::circle([900.0, 1000.0], 316.0)
            .fill(Srgba::hex(0xc9d6df))
            .elevation(1.5),
    ];
    draws.push(image(Draw::circle([900.0, 1000.0], 300.0), &cells[0]).elevation(2.0));
    draws.push(
        Draw::rect([900.0, 1430.0], [520.0, 96.0], 48.0)
            .fill(ink)
            .elevation(3.0),
    );
    for (k, x) in [2050.0f32, 2650.0].into_iter().enumerate() {
        draws.push(
            Draw::rect([x, 700.0], [340.0, 340.0], 56.0)
                .fill(Srgba::hex(0xd8cfc0))
                .elevation(1.5),
        );
        draws.push(
            image(Draw::rect([x, 700.0], [300.0, 300.0], 48.0), &cells[1 + k]).elevation(2.0),
        );
        draws.push(
            Draw::circle([x + 140.0, 560.0], 30.0)
                .fill(accent)
                .elevation(3.0),
        );
    }
    for k in 0..8 {
        let centre = [
            1900.0 + 300.0 * (k % 4) as f32,
            1420.0 + 200.0 * (k / 4) as f32,
        ];
        draws.push(image(Draw::circle(centre, 60.0), &cells[3 + k]).elevation(2.0));
        draws.push(
            Draw::new(FlatShape::Ring {
                radius: 63.0,
                width: 6.0,
            })
            .at(centre)
            .fill(if k % 3 == 0 { accent } else { ink })
            .elevation(3.0),
        );
    }
    Board { draws }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().map(PathBuf::from);
    let atlas_out = args.next().map(PathBuf::from);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let daylight = Daylight {
        hour: 16.0,
        day: Daylight::DAY,
        latitude: Daylight::LATITUDE,
        heading: Daylight::HEADING,
    };
    let mut live = Renderer::new_with_output_format(
        gpu.clone(),
        ATLAS[0],
        ATLAS[1],
        wgpu::TextureFormat::Rgba8Unorm,
    )
    .unwrap();
    live.set_exposure(Exposure::Fixed(1.0)).unwrap();
    live.set_sky(
        SkySource::Analytic(AnalyticSky::new(
            daylight,
            REFERENCE_HOUR,
            2.6,
            [0.32, 0.3, 0.27],
        )),
        16.0,
    )
    .unwrap();
    live.wait_sky().unwrap();
    let half = [
        SHEET[0] as f32 * 0.5 * METRES_PER_PIXEL,
        SHEET[1] as f32 * 0.5 * METRES_PER_PIXEL,
    ];
    live.set_shadow_quality(Quality {
        resolution: 1024,
        receiver: Some(ReceiverBox {
            min: [-half[0], -half[1], -0.5],
            max: [half[0], half[1], 0.5],
        }),
        caster_margin: 0.5,
        ..Quality::default()
    });
    live.set_finish(Chain {
        passes: vec![PostPass::Tone(Tone::aces())],
        ..Chain::new()
    });
    live.set_sharpen(0.0).unwrap();
    live.set_lens(None);
    live.set_haze(None);
    live.set_sky_visibility(false);
    live.set_alpha_from_coverage(true).unwrap();
    live.set_viewport(SHEET[0], SHEET[1]).unwrap();
    let meshes = Meshes {
        head: upload(
            &mut live,
            Shape::Ellipsoid {
                radius: 1.0,
                squash: 1.0,
            },
        ),
        hat: upload(
            &mut live,
            Shape::RoundCone {
                a: [0.0, 0.0, 0.0],
                b: [0.0, 0.55, 0.0],
                r1: 0.62,
                r2: 0.12,
            },
        ),
        eye: upload(
            &mut live,
            Shape::Ellipsoid {
                radius: 0.12,
                squash: 0.6,
            },
        ),
    };
    let materials = materials();
    let atlas = FlatSprites::live(&gpu.device, ATLAS, SpriteFilter::Linear);
    let mut flat = Renderer::new_with_output_format(
        gpu.clone(),
        SCREEN[0],
        SCREEN[1],
        wgpu::TextureFormat::Rgba8UnormSrgb,
    )
    .unwrap();
    let target = gpu
        .offscreen(SCREEN[0], SCREEN[1], wgpu::TextureFormat::Rgba8UnormSrgb)
        .unwrap();
    let cells = cells();
    let board = board(&cells);
    let curve = ShadowCurve::new(
        Srgba::hex(0x0d1018),
        vec![CurvePoint {
            elevation: 1.0,
            offset: 6.0,
            sigma: 10.0,
            alpha: 0.28,
            contact: None,
        }],
    )
    .unwrap();
    let eye = [0.0, 0.0, 10.0];
    let view = translate([-eye[0], -eye[1], -eye[2]]);
    let projection = orthographic(half, 8.0, 12.0);
    let view_projection = multiply(projection, view);
    let mut sun = Sun::from_daylight(daylight, REFERENCE_HOUR);
    let toward: [f32; 3] = [-0.45, 0.6, 0.66];
    let length = toward.iter().map(|v| v * v).sum::<f32>().sqrt();
    sun.direction = toward.map(|v| v / length);
    let mut previous: Option<Vec<Instance>> = None;
    let mut live_ms = Vec::new();
    let mut flat_ms = Vec::new();
    for frame in 0..WARM + FRAMES {
        let time = frame as f32 / 60.0;
        let mut instances = heads(&meshes, &cells, time);
        if let Some(previous) = &previous {
            for (instance, before) in instances.iter_mut().zip(previous) {
                instance.previous_model = before.model;
            }
        }
        let scene = Scene {
            camera: Camera {
                view,
                projection,
                previous_view_projection: view_projection,
                position: eye,
            },
            time,
            seed: 5,
            sun,
            instances: &instances,
            materials: &materials,
            deformers: &[],
            wind: SceneWind::default(),
        };
        let timings = live
            .render(
                &scene,
                &Text::default(),
                &Effects::default(),
                Finish::Standard,
                atlas.view(),
            )
            .unwrap();
        let flat_scene = FlatScene {
            layout: [SCREEN[0] as f32, SCREEN[1] as f32],
            clear: Some(Srgba::hex(0x1b2230)),
            curve: &curve,
            light: Light::default(),
            draws: &board.draws,
            groups: &[],
            text: &[],
            icons: None,
            sprites: Some(&atlas),
            environment: None,
            post: false,
            frame,
            seed: 0,
        };
        let flat_timings = flat.render_flat(&flat_scene, &target.view).unwrap();
        if frame >= WARM {
            live_ms.push(milliseconds(&timings));
            flat_ms.push(milliseconds(&flat_timings));
        }
        previous = Some(instances);
    }
    live_ms.sort_by(f64::total_cmp);
    flat_ms.sort_by(f64::total_cmp);
    println!(
        "{} heads ({} instances) in a {}x{} sheet of a {}x{} atlas, flat frame {}x{}, {FRAMES} frames after {WARM}",
        cells.len(),
        cells.len() * 4,
        SHEET[0],
        SHEET[1],
        ATLAS[0],
        ATLAS[1],
        SCREEN[0],
        SCREEN[1]
    );
    println!(
        "live GPU p50 {:.3} ms, p99 {:.3} ms; flat GPU p50 {:.3} ms, p99 {:.3} ms",
        percentile(&live_ms, 0.5),
        percentile(&live_ms, 0.99),
        percentile(&flat_ms, 0.5),
        percentile(&flat_ms, 0.99)
    );
    if let Some(out) = out {
        write_png(&out, SCREEN, &gpu.readback_rgba8(&target).unwrap());
        println!("wrote {}", out.display());
    }
    if let Some(atlas_out) = atlas_out {
        let sheet = OffscreenTarget {
            texture: atlas.texture().clone(),
            view: atlas.view().clone(),
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: ATLAS[0],
            height: ATLAS[1],
        };
        write_png(&atlas_out, ATLAS, &gpu.readback_bytes(&sheet).unwrap());
        println!("wrote {}", atlas_out.display());
    }
}
