use pfx_gpu::pace::{Pacer, Turns};
use pfx_gpu::{Gpu, OffscreenTarget, PassTiming, wgpu};
use pfx_live::crisp;
use pfx_live::frame::{Camera, Instance, Matrix, MeshData, Scene, SceneWind, Sun, multiply};
use pfx_live::maps::{ContentFormat, TextureFilter};
use pfx_live::renderer::{Effects, Exposure, Finish, Renderer, Text, TextQuadItem};
use pfx_live::text::{GpuAtlas, TextSpace, rich_quads};
use pfx_load::Sky;
use pfx_materials::{Blend, ContentLayer, Material};
use pfx_post::Chain;
use pfx_post::color::srgb_channel;
use pfx_text::{Anchor, Face, Representation, RichParagraph, Span, TextEngine};
use pfx_trace::detail::{Lens, Transform};
use pfx_trace::stage::{Image, Placement, PlacementContent, Stage, Staged};
use pfx_trace::text;
use pfx_trace::{Camera as TraceCamera, Projection, Sun as TraceSun};

const FONT: &[u8] = pfx_text::fixture::EB_GARAMOND;
const UNITS_PER_METRE: f32 = 8192.0;
const OPEN: [f32; 4] = [-1e9, -1e9, 1e9, 1e9];
const WIDTH: u32 = 3840;
const HEIGHT: u32 = 2160;
const PIXELS_PER_METRE: f32 = 6000.0;
const DISTANCE: f32 = 1.0;
const LIFT: f32 = 0.0005;
const LINES: &str = "Sphinx of black quartz, judge my vow.\nThe five boxing wizards jump quickly.\nPack my box with five dozen liquor jugs.\nHow vexingly quick daft zebras jump!";

fn paragraph(em_pixels: f32, color: [f32; 4]) -> RichParagraph {
    let size = em_pixels / PIXELS_PER_METRE;
    let mut engine = TextEngine::new(FONT).unwrap();
    let spans = [Span {
        text: LINES.into(),
        face: Face {
            family: "EB Garamond".into(),
            size,
            line: size * 1.3,
            weight: 500,
            italic: false,
            spacing: 0.0,
        },
        color,
    }];
    let block = engine
        .layout_spans(&spans, None, UNITS_PER_METRE, Anchor::Start)
        .unwrap();
    let origin = [-block.width * 0.5, -block.height * 0.5];
    engine
        .render_spans(
            &spans,
            None,
            UNITS_PER_METRE,
            Anchor::Start,
            Representation::Msdf,
            origin,
            1.0,
            OPEN,
            [0.0; 3],
        )
        .unwrap()
}

fn flip(z: f32) -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, z, 1.0],
    ]
}

fn tan_half_height() -> f32 {
    HEIGHT as f32 / PIXELS_PER_METRE * 0.5 / DISTANCE
}

fn projection() -> Matrix {
    let tan = tan_half_height();
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let (near, far) = (0.05, 30.0);
    [
        [1.0 / (tan * aspect), 0.0, 0.0, 0.0],
        [0.0, 1.0 / tan, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ]
}

fn view_at(eye: [f32; 3]) -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [-eye[0], -eye[1], -eye[2], 1.0],
    ]
}

fn eye_at(frame: u32, drift: [f32; 2]) -> [f32; 3] {
    [
        drift[0] * frame as f32 / PIXELS_PER_METRE,
        drift[1] * frame as f32 / PIXELS_PER_METRE,
        DISTANCE,
    ]
}

fn quad(half: [f32; 2]) -> Flat {
    Flat {
        positions: vec![
            [-half[0], -half[1], 0.0],
            [half[0], -half[1], 0.0],
            [half[0], half[1], 0.0],
            [-half[0], half[1], 0.0],
        ],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        tangents: vec![[1.0, 0.0, 0.0, 1.0]; 4],
        uvs: vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        indices: vec![0, 1, 2, 0, 2, 3],
    }
}

fn output(gpu: &Gpu, width: u32, height: u32) -> OffscreenTarget {
    let format = wgpu::TextureFormat::Rgba16Float;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("crisp output"),
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

fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn decoded(gpu: &Gpu, target: &OffscreenTarget) -> Vec<f32> {
    gpu.readback_rgba16(target)
        .unwrap()
        .chunks_exact(4)
        .map(|p| {
            luminance(std::array::from_fn(|c| {
                srgb_channel(half::f16::from_bits(p[c]).to_f32())
            }))
        })
        .collect()
}

fn gpu_ms(timings: &[PassTiming]) -> f64 {
    timings.iter().map(|t| t.milliseconds).sum()
}

fn percentile(values: &mut [f32], p: f32) -> f32 {
    values.sort_by(f32::total_cmp);
    values[((values.len() - 1) as f32 * p).round() as usize]
}

fn crossing(line: &[f32], from: usize, level: f32, falling: bool) -> f64 {
    for k in from..line.len() - 1 {
        let (a, b) = (line[k], line[k + 1]);
        let passes = if falling {
            a >= level && b < level
        } else {
            a <= level && b > level
        };
        if passes {
            return k as f64 + f64::from((level - a) / (b - a));
        }
    }
    f64::NAN
}

fn line_rises(line: &[f32], out: &mut Vec<f64>) {
    let mut k = 0;
    while k + 1 < line.len() {
        let start = line[k];
        let falling = start >= 0.9;
        let rising = start <= 0.1;
        if !(falling || rising) {
            k += 1;
            continue;
        }
        let mut end = k;
        let mut clean = false;
        while end + 1 < line.len() && end - k < 8 {
            let next = line[end + 1];
            let steady = if falling {
                next <= line[end] + 0.03
            } else {
                next >= line[end] - 0.03
            };
            if !steady {
                break;
            }
            end += 1;
            if (falling && next <= 0.1) || (rising && next >= 0.9) {
                clean = true;
                break;
            }
        }
        if clean {
            let segment = &line[k..=end];
            let (first, last) = if falling { (0.9, 0.1) } else { (0.1, 0.9) };
            let a = crossing(segment, 0, first, falling);
            let b = crossing(segment, 0, last, falling);
            if a.is_finite() && b.is_finite() && b >= a {
                out.push(b - a);
            }
            k = end;
        } else {
            k += 1;
        }
    }
}

struct Rise {
    median: f64,
    p90: f64,
    count: usize,
}

fn rises(lum: &[f32], width: usize, region: [usize; 4]) -> Rise {
    let [x0, y0, x1, y1] = region;
    let mut values: Vec<f32> = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .map(|(x, y)| lum[y * width + x])
        .collect();
    let dark = percentile(&mut values, 0.02);
    let light = percentile(&mut values, 0.98);
    let norm = |v: f32| (v - dark) / (light - dark).max(1e-6);
    let mut widths = Vec::new();
    for y in y0..y1 {
        let row: Vec<f32> = (x0..x1).map(|x| norm(lum[y * width + x])).collect();
        line_rises(&row, &mut widths);
    }
    for x in x0..x1 {
        let column: Vec<f32> = (y0..y1).map(|y| norm(lum[y * width + x])).collect();
        line_rises(&column, &mut widths);
    }
    widths.sort_by(f64::total_cmp);
    let at = |p: f64| widths[((widths.len() - 1) as f64 * p).round() as usize];
    Rise {
        median: at(0.5),
        p90: at(0.9),
        count: widths.len(),
    }
}

fn black() -> Sky {
    Sky {
        width: 1,
        height: 1,
        texels: vec![[0.0, 0.0, 0.0, 1.0]],
    }
}

fn traced_lettering(
    gpu: &Gpu,
    paragraph: &RichParagraph,
    size: [u32; 2],
    samples: u32,
) -> Vec<f32> {
    let glyphs = text::glyphs(paragraph);
    let lettering = text::lettering(&paragraph.atlas, &glyphs).unwrap();
    let carrier = lettering.carrier();
    let materials = [text::material(false)];
    let model: Transform = flip(LIFT);
    let placements = [Placement {
        content: Some(lettering.content()),
        ..Placement::new(0, model, 0)
    }];
    let tan_x = size[0] as f32 / PIXELS_PER_METRE * 0.5 / DISTANCE;
    let tan_y = size[1] as f32 / PIXELS_PER_METRE * 0.5 / DISTANCE;
    let staged: Staged = Stage {
        meshes: &[carrier.mesh()],
        instances: &placements,
        materials: &materials,
        sky: black(),
        sun: TraceSun {
            direction: [0.0, 1.0, 0.0],
            color: [1.0; 3],
            intensity: 0.0,
        },
        camera: TraceCamera {
            origin: [0.0, 0.0, DISTANCE],
            forward: [0.0, 0.0, -1.0],
            right: [tan_x, 0.0, 0.0],
            up: [0.0, tan_y, 0.0],
        },
        projection: Projection::Perspective,
        lens: Lens::default(),
    }
    .build()
    .unwrap();
    let mut trace = staged.trace(gpu, size[0], size[1]).unwrap();
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    for pass in 0..samples {
        trace
            .sample_paced(gpu, 1, 7 + pass, &mut pacer, |ms| turns.add(ms))
            .unwrap();
    }
    turns.turn();
    trace
        .readback(gpu)
        .unwrap()
        .color
        .chunks_exact(16)
        .map(|p| {
            luminance(std::array::from_fn(|c| {
                f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap())
            }))
        })
        .collect()
}

struct Page {
    renderer: Renderer,
    instances: Vec<Instance>,
    materials: Vec<Material>,
    atlas: GpuAtlas,
    quads: Vec<pfx_live::text::RichQuad>,
    target: OffscreenTarget,
    lettered: bool,
}

impl Page {
    fn new(paragraph: &RichParagraph) -> Self {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
        renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
        renderer.set_finish(Chain {
            passes: Vec::new(),
            frame: 0,
            seed: 0,
        });
        let page = quad([0.6, 0.4]);
        let mesh = renderer
            .upload_mesh(MeshData {
                positions: &page.positions,
                normals: &page.normals,
                tangents: &page.tangents,
                uvs: &page.uvs,
                uvs1: None,
                alpha: None,
                indices: &page.indices,
            })
            .unwrap();
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let instances = vec![Instance::new(mesh, identity, 0, 1)];
        let materials = vec![Material {
            base: [0.8, 0.78, 0.74],
            roughness: 0.9,
            specular: 0.0,
            ..Material::default()
        }];
        let atlas = GpuAtlas::new(
            &renderer.gpu().device,
            &renderer.gpu().queue,
            &paragraph.atlas,
        )
        .unwrap();
        let target = output(renderer.gpu(), WIDTH, HEIGHT);
        Self {
            renderer,
            instances,
            materials,
            atlas,
            quads: rich_quads(paragraph),
            target,
            lettered: true,
        }
    }

    fn frame(&mut self, number: u32, drift: [f32; 2]) -> Vec<PassTiming> {
        let projection = projection();
        let view = view_at(eye_at(number, drift));
        let previous = view_at(eye_at(number.saturating_sub(1), drift));
        let view_projection = multiply(projection, view);
        let camera = Camera {
            view,
            projection,
            previous_view_projection: multiply(projection, previous),
            position: eye_at(number, drift),
        };
        let text = Text {
            surface_quads: vec![TextQuadItem {
                atlas: &self.atlas,
                quads: if self.lettered { &self.quads } else { &[] },
                space: TextSpace::Surface {
                    model_view_projection: multiply(view_projection, flip(LIFT)),
                },
                icons: false,
                id: 0,
            }],
            ..Default::default()
        };
        let scene = Scene {
            camera,
            time: number as f32 / 60.0,
            seed: 3,
            sun: Sun {
                direction: [0.25, 0.35, 0.9],
                colour: [1.0; 3],
                intensity: 2.0,
            },
            instances: &self.instances,
            materials: &self.materials,
            deformers: &[],
            wind: SceneWind::default(),
        };
        self.renderer
            .render(
                &scene,
                &text,
                &Effects::default(),
                Finish::Standard,
                &self.target.view,
            )
            .unwrap()
    }

    fn run(&mut self, frames: u32, drift: [f32; 2], turns: &mut Turns) -> Vec<Vec<PassTiming>> {
        (0..frames)
            .map(|number| {
                let timings = self.frame(number, drift);
                turns.add(gpu_ms(&timings));
                timings
            })
            .collect()
    }

    fn luminance(&self) -> Vec<f32> {
        decoded(self.renderer.gpu(), &self.target)
    }
}

fn median_ms(frames: &[Vec<PassTiming>], label: &str) -> f64 {
    let mut values: Vec<f64> = frames
        .iter()
        .map(|timings| {
            timings
                .iter()
                .filter(|t| t.label == label)
                .map(|t| t.milliseconds)
                .sum()
        })
        .collect();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn text_region(paragraph: &RichParagraph) -> [usize; 4] {
    let glyphs = text::glyphs(paragraph);
    let mut lo = [f32::INFINITY; 2];
    let mut hi = [f32::NEG_INFINITY; 2];
    for glyph in &glyphs {
        lo[0] = lo[0].min(glyph.rect[0]);
        lo[1] = lo[1].min(glyph.rect[1]);
        hi[0] = hi[0].max(glyph.rect[0] + glyph.rect[2]);
        hi[1] = hi[1].max(glyph.rect[1] + glyph.rect[3]);
    }
    let to_x = |x: f32| (WIDTH as f32 * 0.5 + x * PIXELS_PER_METRE).round() as usize;
    let to_y = |y: f32| (HEIGHT as f32 * 0.5 + y * PIXELS_PER_METRE).round() as usize;
    [
        to_x(lo[0]) - 8,
        to_y(lo[1]) - 8,
        to_x(hi[0]) + 8,
        to_y(hi[1]) + 8,
    ]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn surface_text_out_of_the_taa_history_keeps_its_edges_at_4k() {
    let em = 40.0;
    let word = paragraph(em, [0.03, 0.03, 0.035, 1.0]);
    let region = text_region(&word);
    println!("text block at {region:?} of {WIDTH}x{HEIGHT}, {em} px em");
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let crop = [
        ((region[2] - region[0]) as u32).min(1600),
        ((region[3] - region[1]) as u32).min(800),
    ];
    let traced = traced_lettering(&gpu, &paragraph(em, [1.0; 4]), crop, 64);
    let reference = rises(
        &traced,
        crop[0] as usize,
        [0, 0, crop[0] as usize, crop[1] as usize],
    );
    println!(
        "traced lettering ({}x{} crop): 10-90% rise median {:.3} px, p90 {:.3}, over {} edges",
        crop[0], crop[1], reference.median, reference.p90, reference.count
    );
    drop(gpu);
    let mut page = Page::new(&word);
    let mut turns = Turns::default();
    let drift = [0.37, 0.23];
    let mut table = Vec::new();
    for (name, after, sharpen, moving) in [
        ("in history, still", false, 0.0, false),
        ("after TAA, still", true, 0.0, false),
        ("in history, drifting", false, 0.0, true),
        ("after TAA, drifting", true, 0.0, true),
        ("in history, drifting, sharpen 0.25", false, 0.25, true),
        ("in history, drifting, sharpen 0.5", false, 0.5, true),
        ("in history, drifting, sharpen 1", false, 1.0, true),
        ("after TAA, drifting, sharpen 0.5", true, 0.5, true),
    ] {
        page.renderer.set_text_after_taa(after);
        page.renderer.set_sharpen(sharpen).unwrap();
        let frames = page.run(48, if moving { drift } else { [0.0; 2] }, &mut turns);
        let lum = page.luminance();
        turns.turn();
        let shift = if moving {
            let eye = eye_at(47, drift);
            [
                (eye[0] * PIXELS_PER_METRE).round() as isize,
                (eye[1] * PIXELS_PER_METRE).round() as isize,
            ]
        } else {
            [0, 0]
        };
        let moved = [
            (region[0] as isize - shift[0]) as usize,
            (region[1] as isize + shift[1]) as usize,
            (region[2] as isize - shift[0]) as usize,
            (region[3] as isize + shift[1]) as usize,
        ];
        let rise = rises(&lum, WIDTH as usize, moved);
        println!(
            "{name}: rise median {:.3} px, p90 {:.3} ({} edges); TAA {:.3} ms, copy {:.3}, sharpen {:.3}, surface text {:.3}",
            rise.median,
            rise.p90,
            rise.count,
            median_ms(&frames, "TAA resolve"),
            median_ms(&frames, "crisp copy"),
            median_ms(&frames, "sharpen"),
            median_ms(&frames, "surface text"),
        );
        table.push((name, rise));
    }
    let find = |name: &str| &table.iter().find(|(n, _)| *n == name).unwrap().1;
    let after_moving = find("after TAA, drifting");
    let before_moving = find("in history, drifting");
    let after_still = find("after TAA, still");
    assert!(
        after_moving.median <= reference.median * 1.2 + 0.05,
        "drifting text after TAA rises over {:.3} px against the trace's {:.3}",
        after_moving.median,
        reference.median
    );
    assert!(
        after_still.median <= reference.median * 1.2 + 0.05,
        "still text after TAA rises over {:.3} px against the trace's {:.3}",
        after_still.median,
        reference.median
    );
    assert!(
        before_moving.median > after_moving.median,
        "text in the history rises over {:.3} px, after TAA {:.3}",
        before_moving.median,
        after_moving.median
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_sharpen_pass_matches_its_cpu_reference() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let (width, height) = (61u32, 37u32);
    let pixels: Vec<[f32; 4]> = (0..width * height)
        .map(|k| {
            let h = k.wrapping_mul(2_654_435_761).rotate_left(7);
            let v = |shift: u32| ((h >> shift) & 255) as f32 / 255.0;
            let x = k % width;
            let step = if x < width / 2 { 0.1 } else { 3.0 };
            [
                step * (0.5 + v(0)),
                step * (0.5 + v(8)),
                step * (0.5 + v(16)),
                0.75,
            ]
        })
        .collect();
    let pixels: Vec<[f32; 4]> = pixels
        .iter()
        .map(|p| p.map(|v| half::f16::from_f32(v).to_f32()))
        .collect();
    let source = gpu
        .offscreen(width, height, wgpu::TextureFormat::Rgba16Float)
        .unwrap();
    let bytes: Vec<u8> = pixels
        .iter()
        .flatten()
        .flat_map(|&v| half::f16::from_f32(v).to_bits().to_le_bytes())
        .collect();
    gpu.queue.write_texture(
        source.texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 8),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    for (strength, exposure) in [(0.0, 1.0), (0.4, 0.8), (1.0, 2.5)] {
        let mut pass = crisp::Pass::new(&gpu.device, width, height);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        pass.encode(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &source.view,
            strength,
            exposure,
            None,
        );
        gpu.queue.submit(Some(encoder.finish()));
        let texture = pass.target().unwrap().clone();
        let target = OffscreenTarget {
            view: texture.create_view(&Default::default()),
            texture,
            format: wgpu::TextureFormat::Rgba16Float,
            width,
            height,
        };
        let got: Vec<f32> = gpu
            .readback_rgba16(&target)
            .unwrap()
            .iter()
            .map(|&bits| half::f16::from_bits(bits).to_f32())
            .collect();
        let expected = crisp::sharpen(&pixels, width as usize, strength, exposure);
        let mut worst = 0.0f32;
        for (k, want) in expected.iter().enumerate() {
            for c in 0..4 {
                let error = (got[k * 4 + c] - want[c]).abs() / want[c].abs().max(0.05);
                worst = worst.max(error);
            }
        }
        println!("strength {strength}, exposure {exposure}: worst relative error {worst:.5}");
        assert!(
            worst < 4e-3,
            "strength {strength}: worst relative error {worst}"
        );
    }
}

const SWATCH_SIZE: [u32; 2] = [640, 360];

const SWATCHES: [(&str, [f32; 3]); 8] = [
    ("red", [0.60, 0.05, 0.04]),
    ("green", [0.08, 0.45, 0.07]),
    ("blue", [0.04, 0.08, 0.55]),
    ("cyan", [0.05, 0.40, 0.50]),
    ("magenta", [0.50, 0.05, 0.40]),
    ("yellow", [0.70, 0.60, 0.05]),
    ("orange", [0.80, 0.30, 0.03]),
    ("grey", [0.40, 0.40, 0.40]),
];

struct Flat {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tangents: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

fn ground(centre: [f32; 2], half: f32, height: f32) -> Flat {
    let [x, z] = centre;
    Flat {
        positions: vec![
            [x - half, height, z - half],
            [x - half, height, z + half],
            [x + half, height, z + half],
            [x + half, height, z - half],
        ],
        normals: vec![[0.0, 1.0, 0.0]; 4],
        tangents: vec![[1.0, 0.0, 0.0, 1.0]; 4],
        uvs: vec![[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
        indices: vec![0, 1, 2, 0, 2, 3],
    }
}

fn swatch_centre(k: usize) -> [f32; 2] {
    [-0.27 + 0.18 * (k % 4) as f32, -0.09 + 0.18 * (k / 4) as f32]
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.map(|x| x / length)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

fn look(
    eye: [f32; 3],
    target: [f32; 3],
    fov_y: f32,
    size: [u32; 2],
) -> (Matrix, Matrix, TraceCamera) {
    let forward = unit(std::array::from_fn(|k| target[k] - eye[k]));
    let right = unit(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    let tan = (fov_y.to_radians() * 0.5).tan();
    let aspect = size[0] as f32 / size[1] as f32;
    let (near, far) = (0.05, 30.0);
    let projection: Matrix = [
        [1.0 / (tan * aspect), 0.0, 0.0, 0.0],
        [0.0, 1.0 / tan, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ];
    let view: Matrix = [
        [right[0], up[0], -forward[0], 0.0],
        [right[1], up[1], -forward[1], 0.0],
        [right[2], up[2], -forward[2], 0.0],
        [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0],
    ];
    (
        view,
        projection,
        TraceCamera {
            origin: eye,
            forward,
            right: right.map(|v| v * tan * aspect),
            up: up.map(|v| v * tan),
        },
    )
}

fn screen(view_projection: Matrix, p: [f32; 3], size: [u32; 2]) -> [usize; 2] {
    let clip: [f32; 4] = std::array::from_fn(|r| {
        (0..3).map(|c| view_projection[c][r] * p[c]).sum::<f32>() + view_projection[3][r]
    });
    [
        ((clip[0] / clip[3] * 0.5 + 0.5) * size[0] as f32) as usize,
        ((0.5 - clip[1] / clip[3] * 0.5) * size[1] as f32) as usize,
    ]
}

fn oklab(c: [f32; 3]) -> [f32; 3] {
    let l = 0.412_221_46 * c[0] + 0.536_332_55 * c[1] + 0.051_445_995 * c[2];
    let m = 0.211_903_5 * c[0] + 0.680_699_5 * c[1] + 0.107_396_96 * c[2];
    let s = 0.088_302_46 * c[0] + 0.281_718_85 * c[1] + 0.629_978_7 * c[2];
    let [l, m, s] = [l, m, s].map(|v| v.max(0.0).cbrt());
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

struct Compared {
    lightness: f32,
    chroma: f32,
    hue: f32,
}

fn compare(ours: [f32; 3], reference: [f32; 3]) -> Compared {
    let a = oklab(ours);
    let b = oklab(reference);
    let chroma = |v: [f32; 3]| v[1].hypot(v[2]);
    let hue = |v: [f32; 3]| v[2].atan2(v[1]).to_degrees();
    let mut turn = hue(a) - hue(b);
    if turn > 180.0 {
        turn -= 360.0;
    }
    if turn < -180.0 {
        turn += 360.0;
    }
    let neutral = chroma(b) < 0.02;
    Compared {
        lightness: a[0] / b[0].max(1e-6),
        chroma: if neutral { 1.0 } else { chroma(a) / chroma(b) },
        hue: if neutral { 0.0 } else { turn },
    }
}

fn means(pixels: &[[f32; 3]], width: usize, centres: &[[usize; 2]]) -> Vec<[f32; 3]> {
    centres
        .iter()
        .map(|&[x, y]| {
            let mut sum = [0.0f64; 3];
            for dy in 0..13 {
                for dx in 0..13 {
                    let p = pixels[(y + dy - 6) * width + x + dx - 6];
                    for c in 0..3 {
                        sum[c] += f64::from(p[c]);
                    }
                }
            }
            sum.map(|v| (v / 169.0) as f32)
        })
        .collect()
}

fn finish(tone: Option<pfx_post::Tone>) -> Chain {
    Chain {
        passes: tone.map(pfx_post::Pass::Tone).into_iter().collect(),
        frame: 0,
        seed: 0,
    }
}

fn halves(gpu: &Gpu, target: &OffscreenTarget) -> Vec<[f32; 3]> {
    gpu.readback_rgba16(target)
        .unwrap()
        .chunks_exact(4)
        .map(|p| std::array::from_fn(|c| half::f16::from_bits(p[c]).to_f32()))
        .collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn live_and_traced_colour_agree_through_the_same_finish() {
    let size = SWATCH_SIZE;
    let sky = Sky {
        width: 1,
        height: 1,
        texels: vec![[0.25, 0.25, 0.25, 1.0]],
    };
    let sun_direction = unit([0.3, 0.8, 0.4]);
    let mut flats = vec![ground([0.0, 0.0], 1.5, 0.0)];
    let mut materials = vec![Material {
        base: [0.18; 3],
        roughness: 1.0,
        specular: 0.0,
        ..Material::default()
    }];
    let mut names: Vec<&str> = Vec::new();
    for (k, (name, base)) in SWATCHES.iter().enumerate() {
        flats.push(ground(swatch_centre(k), 0.06, 0.002));
        names.push(name);
        materials.push(Material {
            base: *base,
            roughness: 1.0,
            specular: 0.0,
            ..Material::default()
        });
    }
    flats.push(ground(swatch_centre(8), 0.06, 0.002));
    names.push("printed");
    materials.push(Material {
        base: [0.9; 3],
        roughness: 1.0,
        specular: 0.0,
        content_layer: ContentLayer {
            slot: 0,
            blend: Blend::Over,
            strength: 1.0,
            ..ContentLayer::default()
        },
        ..Material::default()
    });
    flats.push(ground(swatch_centre(9), 0.06, 0.002));
    names.push("gold");
    materials.push(Material {
        base: [1.0, 0.71, 0.29],
        roughness: 0.35,
        metalness: 1.0,
        ..Material::default()
    });
    let print = vec![[24u8, 142, 168, 255]; 64];
    let print_bytes: Vec<u8> = print.iter().flatten().copied().collect();
    let (view, projection, trace_camera) = look([0.0, 0.85, 0.75], [0.0, 0.0, 0.0], 40.0, size);
    let view_projection = multiply(projection, view);
    let centres: Vec<[usize; 2]> = (0..names.len())
        .map(|k| {
            let [x, z] = swatch_centre(k);
            screen(view_projection, [x, 0.002, z], size)
        })
        .collect();
    let identity: Transform = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let meshes: Vec<pfx_trace::stage::Mesh<'_>> = flats
        .iter()
        .map(|f| pfx_trace::stage::Mesh {
            positions: &f.positions,
            normals: &f.normals,
            tangents: &f.tangents,
            uvs: &f.uvs,
            alpha: None,
            indices: &f.indices,
        })
        .collect();
    let placements: Vec<Placement<'_>> = (0..flats.len())
        .map(|k| Placement {
            content: (k == 9).then(|| {
                PlacementContent::new(Image {
                    width: 8,
                    height: 8,
                    texels: &print,
                    srgb: true,
                })
            }),
            ..Placement::new(k as u32, identity, k as u32)
        })
        .collect();
    let staged = Stage {
        meshes: &meshes,
        instances: &placements,
        materials: &materials,
        sky: sky.clone(),
        sun: TraceSun {
            direction: sun_direction,
            color: [1.0; 3],
            intensity: 3.0,
        },
        camera: trace_camera,
        projection: Projection::Perspective,
        lens: Lens::default(),
    }
    .build()
    .unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut trace = staged.trace(&gpu, size[0], size[1]).unwrap();
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    for pass in 0..256 {
        trace
            .sample_paced(&gpu, 1, 5 + pass, &mut pacer, |ms| turns.add(ms))
            .unwrap();
    }
    turns.turn();
    let traced: Vec<[f32; 4]> = trace
        .readback(&gpu)
        .unwrap()
        .color
        .chunks_exact(16)
        .map(|p| {
            std::array::from_fn(|c| f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()))
        })
        .collect();
    let traced_input = gpu
        .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba16Float)
        .unwrap();
    let bytes: Vec<u8> = traced
        .iter()
        .flatten()
        .flat_map(|&v| half::f16::from_f32(v).to_bits().to_le_bytes())
        .collect();
    gpu.queue.write_texture(
        traced_input.texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size[0] * 8),
            rows_per_image: Some(size[1]),
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
    let traced_linear: Vec<[f32; 3]> = traced.iter().map(|p| [p[0], p[1], p[2]]).collect();
    let traced_means = means(&traced_linear, size[0] as usize, &centres);

    let build = |gpu: Gpu, format: wgpu::TextureFormat| {
        let mut renderer = Renderer::new_with_output_format(gpu, size[0], size[1], format).unwrap();
        renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
        renderer
            .set_sky(pfx_live::sky::SkySource::Hdr(sky.clone()), 12.0)
            .unwrap();
        renderer
            .frame()
            .set_content(0, 8, 8, ContentFormat::Srgb8)
            .unwrap();
        renderer
            .frame()
            .write_content(0, [0, 0, 8, 8], &print_bytes)
            .unwrap();
        let instances: Vec<Instance> = flats
            .iter()
            .enumerate()
            .map(|(k, f)| {
                let mesh = renderer
                    .upload_mesh(MeshData {
                        positions: &f.positions,
                        normals: &f.normals,
                        tangents: &f.tangents,
                        uvs: &f.uvs,
                        uvs1: None,
                        alpha: None,
                        indices: &f.indices,
                    })
                    .unwrap();
                Instance::new(mesh, identity, k as u32, k as u32 + 1)
            })
            .collect();
        (renderer, instances)
    };
    let render = |renderer: &mut Renderer,
                  instances: &[Instance],
                  target: &wgpu::TextureView,
                  turns: &mut Turns| {
        for number in 0..16 {
            let scene = Scene {
                camera: Camera {
                    view,
                    projection,
                    previous_view_projection: view_projection,
                    position: [0.0, 0.85, 0.75],
                },
                time: number as f32 / 60.0,
                seed: 9,
                sun: Sun {
                    direction: sun_direction,
                    colour: [1.0; 3],
                    intensity: 3.0,
                },
                instances,
                materials: &materials,
                deformers: &[],
                wind: SceneWind::default(),
            };
            let timings = renderer
                .render(
                    &scene,
                    &Text::default(),
                    &Effects::default(),
                    Finish::Standard,
                    target,
                )
                .unwrap();
            turns.add(gpu_ms(&timings));
        }
        turns.turn();
    };
    let (mut renderer, instances) = build(gpu, wgpu::TextureFormat::Rgba16Float);
    let target = output(renderer.gpu(), size[0], size[1]);
    let mut worst_pre: f32 = 0.0;
    let mut worst_post: f32 = 0.0;
    let mut worst_hue: f32 = 0.0;
    let mut worst_lightness: f32 = 0.0;
    for (name, tone) in [
        ("linear", None),
        ("aces", Some(pfx_post::Tone::aces())),
        ("agx", Some(pfx_post::Tone::Agx)),
    ] {
        renderer.set_finish(finish(tone));
        render(&mut renderer, &instances, &target.view, &mut turns);
        let live_display: Vec<[f32; 3]> = halves(renderer.gpu(), &target)
            .iter()
            .map(|p| p.map(srgb_channel))
            .collect();
        let mut chain = finish(tone);
        chain.passes.insert(0, pfx_post::Pass::Exposure(1.0));
        chain.passes.push(pfx_post::Pass::Encode);
        let post = pfx_post::gpu::GpuChain::new(chain, &renderer.gpu().device, size[0], size[1]);
        let traced_target = output(renderer.gpu(), size[0], size[1]);
        let mut encoder = renderer
            .gpu()
            .device
            .create_command_encoder(&Default::default());
        post.run(
            &mut encoder,
            &traced_input.view,
            None,
            None,
            &traced_target.view,
            0,
            0,
        );
        renderer.gpu().queue.submit(Some(encoder.finish()));
        let traced_display: Vec<[f32; 3]> = halves(renderer.gpu(), &traced_target)
            .iter()
            .map(|p| p.map(srgb_channel))
            .collect();
        let live_means = means(&live_display, size[0] as usize, &centres);
        let traced_post = means(&traced_display, size[0] as usize, &centres);
        for (k, swatch) in names.iter().enumerate() {
            let pre = compare(
                if tone.is_none() {
                    live_means[k]
                } else {
                    traced_means[k]
                },
                traced_means[k],
            );
            let post = compare(live_means[k], traced_post[k]);
            println!(
                "{name} {swatch}: live {:?} traced {:?}; lightness {:.3}, chroma {:.3}, hue {:+.2} deg",
                live_means[k].map(|v| (v * 1000.0).round() / 1000.0),
                traced_post[k].map(|v| (v * 1000.0).round() / 1000.0),
                post.lightness,
                post.chroma,
                post.hue,
            );
            if tone.is_none() {
                worst_pre = worst_pre.max((pre.chroma - 1.0).abs());
            } else {
                worst_post = worst_post.max((post.chroma - 1.0).abs());
            }
            worst_hue = worst_hue.max(post.hue.abs());
            worst_lightness = worst_lightness.max((post.lightness - 1.0).abs());
        }
    }
    let (mut srgb_renderer, srgb_instances) = build(
        pollster::block_on(Gpu::headless()).unwrap(),
        wgpu::TextureFormat::Rgba8UnormSrgb,
    );
    srgb_renderer.set_finish(finish(Some(pfx_post::Tone::aces())));
    renderer.set_finish(finish(Some(pfx_post::Tone::aces())));
    let srgb = srgb_renderer
        .gpu()
        .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba8UnormSrgb)
        .unwrap();
    render(&mut srgb_renderer, &srgb_instances, &srgb.view, &mut turns);
    render(&mut renderer, &instances, &target.view, &mut turns);
    let encoded: Vec<[f32; 3]> = srgb_renderer
        .gpu()
        .readback_rgba8(&srgb)
        .unwrap()
        .chunks_exact(4)
        .map(|p| std::array::from_fn(|c| f32::from(p[c]) / 255.0))
        .collect();
    let float: Vec<[f32; 3]> = halves(renderer.gpu(), &target);
    let encoded_means = means(&encoded, size[0] as usize, &centres);
    let float_means = means(&float, size[0] as usize, &centres);
    let mut worst_output: f32 = 0.0;
    for k in 0..names.len() {
        for c in 0..3 {
            worst_output = worst_output.max((encoded_means[k][c] - float_means[k][c]).abs());
        }
    }
    println!(
        "an sRGB output and a float output with the encode agree within {:.2}/255",
        worst_output * 255.0
    );
    assert!(
        worst_output * 255.0 < 1.0,
        "the sRGB output is {worst_output} off"
    );
    println!(
        "worst chroma off by {worst_pre:.3} before the finish and {worst_post:.3} after it; worst hue {worst_hue:.2} deg; worst lightness off by {worst_lightness:.3}"
    );
    assert!(worst_hue < 1.0, "a hue turns by {worst_hue:.2} deg");
    assert!(
        worst_pre < 0.02,
        "chroma off by {worst_pre:.3} in linear light"
    );
    assert!(
        worst_post < 0.02,
        "chroma off by {worst_post:.3} after the finish"
    );
    assert!(
        worst_lightness < 0.02,
        "lightness off by {worst_lightness:.3}"
    );
}

const SMALL: [u32; 2] = [960, 540];
const FINE: u32 = 4;

fn page_print() -> (u32, u32, Vec<u8>) {
    let size = 0.012;
    let mut engine = TextEngine::new(FONT).unwrap();
    let text: String = (0..7).map(|_| LINES).collect::<Vec<_>>().join("\n");
    let spans = [Span {
        text,
        face: Face {
            family: "EB Garamond".into(),
            size,
            line: size * 1.25,
            weight: 500,
            italic: false,
            spacing: 0.0,
        },
        color: [1.0; 4],
    }];
    let word = engine
        .render_spans(
            &spans,
            None,
            UNITS_PER_METRE,
            Anchor::Start,
            Representation::Msdf,
            [0.0, 0.0],
            1.0,
            OPEN,
            [0.0; 3],
        )
        .unwrap();
    let glyphs = text::glyphs(&word);
    let print = text::print(&word.atlas, &glyphs, 6000.0).unwrap();
    let bytes = print
        .texels
        .iter()
        .flat_map(|t| {
            let a = f32::from(t[3]) / 255.0;
            let v = (250.0 * (1.0 - a) + 22.0 * a).round() as u8;
            [v, v, v, 255]
        })
        .collect();
    (print.width, print.height, bytes)
}

struct Bench {
    renderer: Renderer,
    instances: Vec<Instance>,
    materials: Vec<Material>,
    target: OffscreenTarget,
    size: [u32; 2],
}

const FLOOR: [f32; 2] = [1.2, 1.6];

fn bench(size: [u32; 2], page: &(u32, u32, Vec<u8>), mapped: bool) -> Bench {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut renderer = Renderer::new(gpu, size[0], size[1]).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    renderer.set_finish(Chain {
        passes: Vec::new(),
        frame: 0,
        seed: 0,
    });
    renderer
        .set_sky(pfx_live::sky::SkySource::Hdr(black()), 12.0)
        .unwrap();
    let (width, height, bytes) = page;
    let mut material = Material {
        base: [1.0; 3],
        roughness: 1.0,
        specular: 0.0,
        ..Material::default()
    };
    if mapped {
        let flat = |v: [u8; 4], space| pfx_load::Image {
            width: 4,
            height: 4,
            space,
            pixels: pfx_load::Pixels::Eight(v.repeat(16)),
        };
        let base = [pfx_load::Image {
            width: *width,
            height: *height,
            space: pfx_load::ColorSpace::Srgb,
            pixels: pfx_load::Pixels::Eight(bytes.clone()),
        }];
        let normal = [flat([128, 128, 255, 255], pfx_load::ColorSpace::Linear)];
        let rough = [flat([255; 4], pfx_load::ColorSpace::Linear)];
        renderer
            .set_maps(&pfx_live::maps::MapImages {
                base: &base,
                normal: &normal,
                roughness: &rough,
                metal: &rough,
            })
            .unwrap();
        material.maps = pfx_materials::Maps {
            layer: 0.0,
            tile: 1.0,
            normal: 0.0,
            albedo: 1.0,
        };
    } else {
        renderer
            .frame()
            .set_content(0, *width, *height, ContentFormat::Srgb8)
            .unwrap();
        renderer
            .frame()
            .write_content(0, [0, 0, *width, *height], bytes)
            .unwrap();
        material.content_layer = ContentLayer {
            slot: 0,
            blend: Blend::Over,
            strength: 1.0,
            ..ContentLayer::default()
        };
    }
    let floor = Flat {
        positions: vec![
            [-FLOOR[0] * 0.5, 0.0, -FLOOR[1] * 0.5],
            [-FLOOR[0] * 0.5, 0.0, FLOOR[1] * 0.5],
            [FLOOR[0] * 0.5, 0.0, FLOOR[1] * 0.5],
            [FLOOR[0] * 0.5, 0.0, -FLOOR[1] * 0.5],
        ],
        normals: vec![[0.0, 1.0, 0.0]; 4],
        tangents: vec![[1.0, 0.0, 0.0, 1.0]; 4],
        uvs: vec![[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
        indices: vec![0, 1, 2, 0, 2, 3],
    };
    let mesh = renderer
        .upload_mesh(MeshData {
            positions: &floor.positions,
            normals: &floor.normals,
            tangents: &floor.tangents,
            uvs: &floor.uvs,
            uvs1: None,
            alpha: None,
            indices: &floor.indices,
        })
        .unwrap();
    let identity = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let target = output(renderer.gpu(), size[0], size[1]);
    Bench {
        renderer,
        instances: vec![Instance::new(mesh, identity, 0, 1)],
        materials: vec![material],
        target,
        size,
    }
}

fn bench_eye(frame: u32, moving: bool) -> [f32; 3] {
    let step = if moving { 0.0006 } else { 0.0 };
    [0.0, 0.16, 0.95 - step * frame as f32]
}

impl Bench {
    fn frame(&mut self, number: u32, moving: bool) -> Vec<PassTiming> {
        let camera_at = |k: u32| look(bench_eye(k, moving), [0.0, 0.0, -0.2], 38.0, self.size);
        let (view, projection, _) = camera_at(number);
        let (previous, _, _) = camera_at(number.saturating_sub(1));
        let scene = Scene {
            camera: Camera {
                view,
                projection,
                previous_view_projection: multiply(projection, previous),
                position: bench_eye(number, moving),
            },
            time: number as f32 / 60.0,
            seed: 4,
            sun: Sun {
                direction: unit([0.2, 1.0, 0.3]),
                colour: [1.0; 3],
                intensity: 3.0,
            },
            instances: &self.instances,
            materials: &self.materials,
            deformers: &[],
            wind: SceneWind::default(),
        };
        self.renderer
            .render(
                &scene,
                &Text::default(),
                &Effects::default(),
                Finish::Standard,
                &self.target.view,
            )
            .unwrap()
    }

    fn luminance(&self) -> Vec<f32> {
        decoded(self.renderer.gpu(), &self.target)
    }

    fn mask(&self) -> Vec<bool> {
        let lum = self.luminance();
        let (w, h) = (self.size[0] as usize, self.size[1] as usize);
        (0..w * h)
            .map(|k| {
                let (x, y) = (k % w, k / w);
                x >= 4
                    && y >= 4
                    && x + 4 < w
                    && y + 4 < h
                    && (0..9).all(|dy| (0..9).all(|dx| lum[(y + dy - 4) * w + x + dx - 4] > 0.0015))
            })
            .collect()
    }
}

fn gradient(lum: &[f32], width: usize, mask: &[bool]) -> f64 {
    let mut sum = 0.0;
    let mut count = 0usize;
    for (k, &inside) in mask.iter().enumerate() {
        if inside {
            let gx = lum[k + 1] - lum[k - 1];
            let gy = lum[k + width] - lum[k - width];
            sum += f64::from(gx.hypot(gy));
            count += 1;
        }
    }
    sum / count.max(1) as f64
}

fn downsample(lum: &[f32], size: [u32; 2], factor: u32) -> Vec<f32> {
    let (w, h) = ((size[0] / factor) as usize, (size[1] / factor) as usize);
    let f = factor as usize;
    (0..w * h)
        .map(|k| {
            let (x, y) = (k % w, k / w);
            let mut sum = 0.0;
            for dy in 0..f {
                for dx in 0..f {
                    sum += lum[(y * f + dy) * size[0] as usize + x * f + dx];
                }
            }
            sum / (f * f) as f32
        })
        .collect()
}

fn masked_rms(a: &[f32], b: &[f32], mask: &[bool]) -> f64 {
    let mut sum = 0.0;
    let mut count = 0usize;
    for k in 0..a.len() {
        if mask[k] {
            sum += f64::from((a[k] - b[k]).powi(2));
            count += 1;
        }
    }
    (sum / count.max(1) as f64).sqrt()
}

fn flicker(frames: &[Vec<f32>], mask: &[bool]) -> f64 {
    let mut sum = 0.0;
    let mut count = 0usize;
    for t in 1..frames.len() - 1 {
        for k in 0..mask.len() {
            if mask[k] {
                sum += f64::from((frames[t + 1][k] - 2.0 * frames[t][k] + frames[t - 1][k]).abs());
                count += 1;
            }
        }
    }
    sum / count.max(1) as f64
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_lod_bias_trades_sharpness_against_shimmer() {
    let page = page_print();
    println!(
        "page {}x{} texels on a {}x{} m floor",
        page.0, page.1, FLOOR[0], FLOOR[1]
    );
    let mut turns = Turns::default();
    let fine_size = [SMALL[0] * FINE, SMALL[1] * FINE];
    let reference = {
        let mut fine = bench(fine_size, &page, false);
        for number in 0..24 {
            let timings = fine.frame(number, false);
            turns.add(gpu_ms(&timings));
        }
        turns.turn();
        let lum = downsample(&fine.luminance(), fine_size, FINE);
        let mut costs = Vec::new();
        for filter in [
            TextureFilter::UNBIASED,
            TextureFilter {
                anisotropy: 16,
                ..TextureFilter::UNBIASED
            },
            TextureFilter {
                content_bias: -0.5,
                detail_bias: -0.5,
                anisotropy: 16,
            },
        ] {
            fine.renderer.set_texture_filter(filter).unwrap();
            let frames: Vec<Vec<PassTiming>> = (0..120)
                .map(|number| {
                    let timings = fine.frame(number, true);
                    turns.add(gpu_ms(&timings));
                    timings
                })
                .collect();
            costs.push((filter, median_ms(&frames, "opaque PBR")));
        }
        turns.turn();
        for (filter, ms) in costs {
            println!(
                "at {}x{}: {filter:?}, opaque PBR {ms:.3} ms",
                fine_size[0], fine_size[1]
            );
        }
        lum
    };
    let width = SMALL[0] as usize;
    for mapped in [false, true] {
        let mut small = bench(SMALL, &page, mapped);
        small.frame(0, false);
        let mask = small.mask();
        let reference_gradient = gradient(&reference, width, &mask);
        let rows = mask
            .iter()
            .enumerate()
            .filter(|(_, inside)| **inside)
            .map(|(k, _)| k / width)
            .collect::<Vec<_>>();
        let horizon = rows.iter().copied().min().unwrap_or(0);
        let far: Vec<bool> = mask
            .iter()
            .enumerate()
            .map(|(k, &inside)| inside && k / width < horizon + 120)
            .collect();
        let reference_far = gradient(&reference, width, &far);
        println!(
            "{}: reference gradient {reference_gradient:.5}, far {reference_far:.5}; {} pixels, {} far",
            if mapped { "base map" } else { "content" },
            mask.iter().filter(|m| **m).count(),
            far.iter().filter(|m| **m).count()
        );
        let mut runs = vec![(TextureFilter::UNBIASED, 0.0)];
        for bias in [0.0, -0.25, -0.5, -0.75, -1.0] {
            runs.push((
                TextureFilter {
                    content_bias: bias,
                    detail_bias: bias,
                    anisotropy: 16,
                },
                0.0,
            ));
        }
        if !mapped {
            for strength in [0.25, 0.5, 1.0] {
                runs.push((TextureFilter::default(), strength));
            }
        }
        for (filter, strength) in runs {
            small.renderer.set_texture_filter(filter).unwrap();
            small.renderer.set_sharpen(strength).unwrap();
            let mut still = Vec::new();
            for number in 0..24 {
                let timings = small.frame(number, false);
                turns.add(gpu_ms(&timings));
                if number >= 16 {
                    still.push(small.luminance());
                }
            }
            let mut moving = Vec::new();
            for number in 0..40 {
                let timings = small.frame(number, true);
                turns.add(gpu_ms(&timings));
                if number >= 16 {
                    moving.push(small.luminance());
                }
            }
            turns.turn();
            let last = still.last().unwrap();
            println!(
                "  anisotropy {:2}, bias {:+.2}, sharpen {strength:.2}: sharpness {:.3} (far {:.3}), rms against 4x {:.4}, flicker still {:.5}, moving {:.5} (far {:.5})",
                filter.anisotropy,
                filter.content_bias,
                gradient(last, width, &mask) / reference_gradient,
                gradient(last, width, &far) / reference_far,
                masked_rms(last, &reference, &mask),
                flicker(&still, &mask),
                flicker(&moving, &mask),
                flicker(&moving, &far),
            );
        }
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_crisp_passes_run_only_when_asked() {
    let word = paragraph(40.0, [0.03, 0.03, 0.035, 1.0]);
    let mut page = Page::new(&word);
    let order = |page: &Page| page.renderer.last_pass_order().to_vec();
    let at = |passes: &[&str], name: &str| passes.iter().position(|p| *p == name);
    page.frame(0, [0.0; 2]);
    let after = order(&page);
    assert!(at(&after, "TAA") < at(&after, "crisp copy"), "{after:?}");
    assert!(
        at(&after, "crisp copy") < at(&after, "surface text"),
        "{after:?}"
    );
    assert!(at(&after, "sharpen").is_none(), "{after:?}");
    page.renderer.set_text_after_taa(false);
    page.frame(1, [0.0; 2]);
    let before = order(&page);
    assert!(
        at(&before, "surface text") < at(&before, "TAA"),
        "{before:?}"
    );
    assert!(
        at(&before, "crisp copy").is_none() && at(&before, "sharpen").is_none(),
        "{before:?}"
    );
    page.renderer.set_sharpen(0.5).unwrap();
    page.frame(2, [0.0; 2]);
    let sharpened = order(&page);
    assert!(
        at(&sharpened, "TAA") < at(&sharpened, "sharpen"),
        "{sharpened:?}"
    );
    assert!(at(&sharpened, "crisp copy").is_none(), "{sharpened:?}");
    assert!(page.renderer.set_sharpen(1.5).is_err());
    page.renderer.set_sharpen(0.0).unwrap();
    page.lettered = false;
    let mut frames = Vec::new();
    for after in [true, false] {
        page.renderer.set_text_after_taa(after);
        for number in 200..204 {
            page.frame(number, [0.0; 2]);
        }
        let passes = order(&page);
        assert!(
            at(&passes, "crisp copy").is_none() && at(&passes, "sharpen").is_none(),
            "{passes:?}"
        );
        frames.push(page.renderer.gpu().readback_rgba16(&page.target).unwrap());
    }
    assert!(
        frames[0] == frames[1],
        "a frame without text changed with the text's place"
    );
}
