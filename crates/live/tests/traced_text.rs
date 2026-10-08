use pfx_gpu::pace::{Pacer, Turns};
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::frame::{Matrix, multiply};
use pfx_live::text::{
    COLOR_FORMAT, DEPTH_FORMAT, GpuAtlas, QuadDraw, RichQuad, TextPass, TextSpace, rich_quads,
};
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_post::color::encode_channel;
use pfx_text::{Anchor, Atlas, Face, MipLevel, Representation, RichParagraph, Span, TextEngine};
use pfx_trace::detail::{Lens, Transform};
use pfx_trace::stage::{Image, Mesh, Placement, PlacementContent, Stage, Staged};
use pfx_trace::text::{self, Carrier, Glyph};
use pfx_trace::{Camera as TraceCamera, Projection, Sun as TraceSun};
use sha2::{Digest, Sha256};

const FONT: &[u8] = pfx_text::fixture::EB_GARAMOND;
const UNITS_PER_METRE: f32 = 8192.0;
const OPEN: [f32; 4] = [-1e9, -1e9, 1e9, 1e9];
const LIFT: f32 = 0.0005;
const RAY_START: f32 = 0.0005;

const IDENTITY: Transform = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

struct Arrays {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tangents: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Arrays {
    fn mesh(&self) -> Mesh<'_> {
        Mesh {
            positions: &self.positions,
            normals: &self.normals,
            tangents: &self.tangents,
            uvs: &self.uvs,
            alpha: None,
            indices: &self.indices,
        }
    }

    fn floor(half: f32) -> Self {
        Self {
            positions: vec![
                [-half, 0.0, -half],
                [half, 0.0, -half],
                [half, 0.0, half],
                [-half, 0.0, half],
            ],
            normals: vec![[0.0, 1.0, 0.0]; 4],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 2, 1, 0, 3, 2],
        }
    }

    fn cube(centre: [f32; 3], half: [f32; 3]) -> Self {
        let mut arrays = Self {
            positions: Vec::new(),
            normals: Vec::new(),
            tangents: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
        };
        for axis in 0..3 {
            for sign in [-1.0_f32, 1.0] {
                let mut normal = [0.0; 3];
                normal[axis] = sign;
                let u_axis = (axis + 1) % 3;
                let v_axis = (axis + 2) % 3;
                let start = arrays.positions.len() as u32;
                for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    let mut p = normal;
                    p[u_axis] = a;
                    p[v_axis] = b;
                    arrays
                        .positions
                        .push(std::array::from_fn(|k| centre[k] + p[k] * half[k]));
                    arrays.normals.push(normal);
                    arrays.tangents.push([1.0, 0.0, 0.0, 1.0]);
                    arrays.uvs.push([(a + 1.0) * 0.5, (b + 1.0) * 0.5]);
                }
                let quad = if sign > 0.0 {
                    [0, 1, 2, 0, 2, 3]
                } else {
                    [0, 2, 1, 0, 3, 2]
                };
                arrays.indices.extend(quad.map(|k| start + k));
            }
        }
        arrays
    }
}

fn lambert(base: [f32; 3]) -> Material {
    Material {
        base,
        roughness: 1.0,
        specular: 0.0,
        ..Material::default()
    }
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.map(|x| x / length)
}

fn black() -> Sky {
    Sky {
        width: 1,
        height: 1,
        texels: vec![[0.0, 0.0, 0.0, 1.0]],
    }
}

fn top_down(width: f32, height: f32, centre: [f32; 2]) -> (TraceCamera, Projection) {
    (
        TraceCamera {
            origin: [centre[0], 3.0, centre[1]],
            forward: [0.0, -1.0, 0.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 0.0, -1.0],
        },
        Projection::Orthographic { width, height },
    )
}

fn traced(gpu: &Gpu, staged: &Staged, size: [u32; 2], samples: u32) -> Vec<[f32; 4]> {
    timed(gpu, staged, size, samples).0
}

fn timed(gpu: &Gpu, staged: &Staged, size: [u32; 2], samples: u32) -> (Vec<[f32; 4]>, f64) {
    let mut trace = staged.trace(gpu, size[0], size[1]).unwrap();
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    let mut milliseconds = 0.0;
    for pass in 0..samples {
        let stats = trace
            .sample_paced(gpu, 1, 7 + pass, &mut pacer, |ms| turns.add(ms))
            .unwrap();
        milliseconds += stats.milliseconds.iter().sum::<f64>();
    }
    turns.turn();
    let pixels = trace
        .readback(gpu)
        .unwrap()
        .color
        .chunks_exact(16)
        .map(|p| {
            std::array::from_fn(|c| f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()))
        })
        .collect();
    (pixels, milliseconds / f64::from(samples))
}

fn digest(pixels: &[[f32; 4]]) -> String {
    let mut hasher = Sha256::new();
    for pixel in pixels {
        for value in pixel {
            hasher.update(value.to_le_bytes());
        }
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn paragraph(
    text: &str,
    size: f32,
    scale: f32,
    at: Option<[f32; 2]>,
    color: [f32; 4],
) -> RichParagraph {
    let representation = if scale == 1.0 {
        Representation::Coverage
    } else {
        Representation::Msdf
    };
    let mut engine = TextEngine::new(FONT).unwrap();
    let spans = [Span {
        text: text.into(),
        face: Face {
            family: "EB Garamond".into(),
            size,
            line: size * 1.25,
            weight: 700,
            italic: false,
            spacing: 0.0,
        },
        color,
    }];
    let origin = at.unwrap_or_else(|| {
        let block = engine
            .layout_spans(&spans, None, scale, Anchor::Start)
            .unwrap();
        [-block.width * 0.5, -block.height * 0.5]
    });
    engine
        .render_spans(
            &spans,
            None,
            scale,
            Anchor::Start,
            representation,
            origin,
            1.0,
            OPEN,
            [0.0; 3],
        )
        .unwrap()
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn view(eye: [f32; 3], forward: [f32; 3], right: [f32; 3], up: [f32; 3]) -> Matrix {
    [
        [right[0], up[0], -forward[0], 0.0],
        [right[1], up[1], -forward[1], 0.0],
        [right[2], up[2], -forward[2], 0.0],
        [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0],
    ]
}

fn perspective(
    eye: [f32; 3],
    target: [f32; 3],
    fov_y: f32,
    size: [u32; 2],
) -> (Matrix, TraceCamera, Projection) {
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
    (
        multiply(projection, view(eye, forward, right, up)),
        TraceCamera {
            origin: eye,
            forward,
            right: right.map(|v| v * tan * aspect),
            up: up.map(|v| v * tan),
        },
        Projection::Perspective,
    )
}

fn front_on(width: f32, height: f32) -> (Matrix, TraceCamera, Projection) {
    let eye = [0.0, 0.0, 1.0];
    let (forward, right, up) = ([0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    let (near, far) = (0.01, 10.0);
    let projection: Matrix = [
        [2.0 / width, 0.0, 0.0, 0.0],
        [0.0, 2.0 / height, 0.0, 0.0],
        [0.0, 0.0, -1.0 / (far - near), 0.0],
        [0.0, 0.0, -near / (far - near), 1.0],
    ];
    (
        multiply(projection, view(eye, forward, right, up)),
        TraceCamera {
            origin: eye,
            forward,
            right,
            up,
        },
        Projection::Orthographic { width, height },
    )
}

fn dark() -> TraceSun {
    TraceSun {
        direction: [0.0, 1.0, 0.0],
        color: [1.0; 3],
        intensity: 0.0,
    }
}

fn stage(
    meshes: &[Mesh<'_>],
    placements: &[Placement<'_>],
    materials: &[Material],
    eye: (TraceCamera, Projection),
    sun: TraceSun,
) -> Staged {
    Stage {
        meshes,
        instances: placements,
        materials,
        sky: black(),
        sun,
        camera: eye.0,
        projection: eye.1,
        lens: Lens::default(),
    }
    .build()
    .unwrap()
}

fn halves(pixels: &[[f32; 4]]) -> Vec<u8> {
    pixels
        .iter()
        .flatten()
        .flat_map(|&value| half::f16::from_f32(value).to_bits().to_le_bytes())
        .collect()
}

fn drawn(
    gpu: &Gpu,
    size: [u32; 2],
    background: Option<&[[f32; 4]]>,
    paragraph: &RichParagraph,
    quads: &[RichQuad],
    space: TextSpace,
) -> Vec<[f32; 4]> {
    let target: OffscreenTarget = gpu.offscreen(size[0], size[1], COLOR_FORMAT).unwrap();
    let extent = wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
    };
    if let Some(background) = background {
        gpu.queue.write_texture(
            target.texture.as_image_copy(),
            &halves(background),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 8),
                rows_per_image: Some(size[1]),
            },
            extent,
        );
    }
    let depth = gpu
        .device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("traced text depth"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default());
    let atlas = GpuAtlas::new(&gpu.device, &gpu.queue, &paragraph.atlas).unwrap();
    let pass = TextPass::new(&gpu.device);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("traced text clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if background.is_some() {
                        wgpu::LoadOp::Load
                    } else {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
    let overlay = matches!(space, TextSpace::Overlay { .. });
    pass.encode_quads(
        &gpu.device,
        &mut encoder,
        QuadDraw {
            target: &target.view,
            depth: (!overlay).then_some(&depth),
            atlas: &atlas,
            quads,
            space,
            deformers: None,
            icons: false,
        },
    )
    .unwrap();
    gpu.queue.submit(Some(encoder.finish()));
    gpu.readback_rgba16(&target)
        .unwrap()
        .chunks_exact(4)
        .map(|p| std::array::from_fn(|c| half::f16::from_bits(p[c]).to_f32()))
        .collect()
}

fn iou(a: &[bool], b: &[bool]) -> f64 {
    let both = a.iter().zip(b).filter(|(a, b)| **a && **b).count();
    let either = a.iter().zip(b).filter(|(a, b)| **a || **b).count();
    both as f64 / either.max(1) as f64
}

fn centroid(weights: impl Iterator<Item = f32>, width: u32) -> [f64; 2] {
    let mut sum = [0.0f64; 3];
    for (index, weight) in weights.enumerate() {
        let weight = f64::from(weight.max(0.0));
        sum[0] += weight * ((index as u32 % width) as f64 + 0.5);
        sum[1] += weight * ((index as u32 / width) as f64 + 0.5);
        sum[2] += weight;
    }
    assert!(sum[2] > 0.0, "nothing to weigh");
    [sum[0] / sum[2], sum[1] / sum[2]]
}

fn content_alpha(print: &text::Print, layout: [f32; 2]) -> f32 {
    let [x, y, w, h] = print.rect;
    let uv = [(layout[0] - x) / w, (layout[1] - y) / h];
    if !(0.0..=1.0).contains(&uv[0]) || !(0.0..=1.0).contains(&uv[1]) {
        return 0.0;
    }
    let size = [print.width as f32, print.height as f32];
    let p = [0, 1].map(|k| (uv[k] * size[k] - 0.5).clamp(0.0, size[k] - 1.0));
    let lo = p.map(|v| v.floor() as u32);
    let hi = [
        (lo[0] + 1).min(print.width - 1),
        (lo[1] + 1).min(print.height - 1),
    ];
    let f = [p[0].fract(), p[1].fract()];
    let at = |column: u32, row: u32| {
        f32::from(print.texels[(row * print.width + column) as usize][3]) / 255.0
    };
    let top = at(lo[0], lo[1]) * (1.0 - f[0]) + at(hi[0], lo[1]) * f[0];
    let bottom = at(lo[0], hi[1]) * (1.0 - f[0]) + at(hi[0], hi[1]) * f[0];
    top * (1.0 - f[1]) + bottom * f[1]
}

fn manf_box(half: [f32; 3]) -> Arrays {
    let mut arrays = Arrays::cube([0.0; 3], half);
    for (uv, (p, n)) in arrays
        .uvs
        .iter_mut()
        .zip(arrays.positions.iter().zip(&arrays.normals))
    {
        let a = n.map(f32::abs);
        let (u, v) = if a[0] >= a[1] && a[0] >= a[2] {
            (2, 1)
        } else if a[1] >= a[2] {
            (0, 2)
        } else {
            (0, 1)
        };
        let q: [f32; 3] = std::array::from_fn(|k| (p[k] + half[k]) / (2.0 * half[k]));
        *uv = [q[u], 1.0 - q[v]];
    }
    arrays
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_traced_free_standing_word_matches_live_surface_text() {
    let size = [512, 256];
    let word = paragraph("Pito", 0.2, UNITS_PER_METRE, None, [1.0; 4]);
    let (s, c) = 0.35_f32.sin_cos();
    let model: Transform = [
        [c, 0.0, -s, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [s, 0.0, c, 0.0],
        [0.0, 0.05, 0.0, 1.0],
    ];
    let (view_projection, camera, projection) =
        perspective([0.08, 0.1, 0.55], [0.0, 0.05, 0.0], 36.0, size);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let live = drawn(
        &gpu,
        size,
        None,
        &word,
        &rich_quads(&word),
        TextSpace::Surface {
            model_view_projection: multiply(view_projection, model),
        },
    );
    let mut turns = Turns::default();
    turns.turn();

    let glyphs = text::glyphs(&word);
    let lettering = text::lettering(&word.atlas, &glyphs).unwrap();
    let print = text::print(&word.atlas, &glyphs, 4000.0).unwrap();
    let live_mask: Vec<bool> = live.iter().map(|p| p[3] >= 0.5).collect();
    let covered = live_mask.iter().filter(|&&inside| inside).count();
    assert!(covered > 4000, "{covered} pixels of text");
    for (name, carrier, content, least) in [
        ("lettering", lettering.carrier(), lettering.content(), 0.99),
        ("print", print.carrier(), print.content(), 0.95),
    ] {
        let materials = [text::material(false)];
        let placements = [Placement {
            content: Some(content),
            ..Placement::new(0, model, 0)
        }];
        let staged = stage(
            &[carrier.mesh()],
            &placements,
            &materials,
            (camera, projection),
            dark(),
        );
        let traced = traced(&gpu, &staged, size, 64);
        let traced_mask: Vec<bool> = traced.iter().map(|p| p[0] >= 0.5).collect();
        let overlap = iou(&live_mask, &traced_mask);
        println!(
            "free-standing word, {name}: {covered} live pixels, {} traced, IoU {overlap:.4}",
            traced_mask.iter().filter(|&&inside| inside).count()
        );
        assert!(overlap >= least, "{name}: IoU {overlap:.4}");
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_traced_word_casts_the_shadow_of_its_glyphs() {
    let size = [400, 200];
    let height = 0.5;
    let word = paragraph("Pito", 0.25, UNITS_PER_METRE, None, [0.0, 0.0, 0.0, 1.0]);
    let glyphs = text::glyphs(&word);
    let lettering = text::lettering(&word.atlas, &glyphs).unwrap();
    let print = text::print(&word.atlas, &glyphs, 2000.0).unwrap();
    let lettered = lettering.sampler();
    let floor = Arrays::floor(3.0);
    let model: Transform = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, height, 0.0, 1.0],
    ];
    let materials = [lambert([0.6; 3]), text::material(true)];
    let sun = unit([2.0, 1.0, 0.0]);
    let shift = (height - RAY_START) * sun[0] / sun[1];
    let centre = [-shift, 0.0];
    let (view_width, view_height) = (1.0, 0.5);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let lettered = |at: [f32; 2]| lettered(at)[3];
    let printed = |at: [f32; 2]| content_alpha(&print, at);
    let alphas: [&dyn Fn([f32; 2]) -> f32; 2] = [&lettered, &printed];
    let cases = [
        ("lettering", lettering.carrier(), lettering.content()),
        ("print", print.carrier(), print.content()),
    ];
    for ((name, carrier, content), alpha) in cases.into_iter().zip(alphas) {
        let placements = [
            Placement::new(0, IDENTITY, 0),
            Placement {
                content: Some(content),
                ..Placement::new(1, model, 1)
            },
        ];
        let staged = stage(
            &[floor.mesh(), carrier.mesh()],
            &placements,
            &materials,
            top_down(view_width, view_height, centre),
            TraceSun {
                direction: sun,
                color: [1.0; 3],
                intensity: 3.0,
            },
        );
        let traced = traced(&gpu, &staged, size, 32);
        let mut lit: Vec<f32> = traced.iter().map(|p| p[0]).collect();
        lit.sort_by(f32::total_cmp);
        let lit = lit[lit.len() * 9 / 10];
        assert!(lit > 0.1, "the floor is lit at {lit}");
        let shadowed: Vec<bool> = traced.iter().map(|p| p[0] < 0.5 * lit).collect();
        let expected: Vec<bool> = (0..size[0] * size[1])
            .map(|index| {
                let inside = (0..16)
                    .filter(|k| {
                        let column = (index % size[0]) as f32 + (k % 4) as f32 / 4.0 + 0.125;
                        let row = (index / size[0]) as f32 + (k / 4) as f32 / 4.0 + 0.125;
                        let x = centre[0] + (column / size[0] as f32 - 0.5) * view_width;
                        let z = centre[1] + (row / size[1] as f32 - 0.5) * view_height;
                        alpha([x + shift, z]) >= 0.5
                    })
                    .count();
                inside >= 8
            })
            .collect();
        let dark = expected.iter().filter(|&&inside| inside).count();
        let overlap = iou(&shadowed, &expected);
        println!(
            "shadow, {name}: {dark} expected pixels, {} traced, IoU {overlap:.4}",
            shadowed.iter().filter(|&&inside| inside).count()
        );
        assert!(dark > 3000, "{dark} pixels of shadow");
        assert!(overlap >= 0.95, "{name}: IoU {overlap:.4}");
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn text_on_a_part_lands_where_live_puts_it() {
    let density = 2000.0;
    let half = [0.3, 0.15, 0.05];
    let (view_width, view_height) = (2.0 * half[0], 2.0 * half[1]);
    let size = [
        (view_width * density) as u32,
        (view_height * density) as u32,
    ];
    let part = manf_box(half);
    let word = paragraph(
        "Bench",
        0.06,
        UNITS_PER_METRE,
        Some([-0.22, -0.11]),
        [1.0; 4],
    );
    let model: Transform = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, half[2] + LIFT, 1.0],
    ];
    let fit = text::fit(&part.mesh(), IDENTITY, model).unwrap();
    assert_eq!(fit.face.len(), 2);
    let glyphs = text::glyphs(&word);
    let lettering = text::lettering(&word.atlas, &glyphs).unwrap();
    let print = text::print(&word.atlas, &glyphs, density).unwrap();
    let shared = fit.elsewhere(&part.mesh(), &fit.content(&print));
    println!(
        "the box's {} other triangles share the face's UVs",
        shared.len()
    );
    assert!(!shared.is_empty());

    let (view_projection, camera, projection) = front_on(view_width, view_height);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let live = drawn(
        &gpu,
        size,
        None,
        &word,
        &rich_quads(&word),
        TextSpace::Surface {
            model_view_projection: multiply(view_projection, model),
        },
    );
    let mut turns = Turns::default();
    turns.turn();
    let ours = centroid(live.iter().map(|p| p[3]), size[0]);
    let texels_per_pixel = f64::from(density) * f64::from(view_width) / f64::from(size[0]);
    let materials = [text::material(false)];
    for (name, content) in [
        ("lettering", fit.content(&lettering)),
        ("print", fit.content(&print)),
    ] {
        let placements = [Placement {
            content: Some(content),
            ..Placement::new(0, IDENTITY, 0)
        }];
        let staged = stage(
            &[part.mesh()],
            &placements,
            &materials,
            (camera, projection),
            dark(),
        );
        let traced = traced(&gpu, &staged, size, 16);
        let theirs = centroid(traced.iter().map(|p| p[0]), size[0]);
        let apart = [0, 1].map(|k| (ours[k] - theirs[k]).abs() * texels_per_pixel);
        println!(
            "text on a part, {name}: live {ours:?}, traced {theirs:?}, {apart:?} texels apart"
        );
        let ink = traced.iter().filter(|p| p[0] >= 0.5).count();
        assert!(ink > 2000, "{name}: {ink} traced pixels of text");
        assert!(
            apart.iter().all(|&d| d <= 1.0),
            "{name}: {apart:?} texels apart"
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_traced_overlay_matches_live_within_one_level() {
    let size = [320, 160];
    let title = paragraph(
        "Overlay",
        40.0,
        1.0,
        Some([12.0, 30.0]),
        [1.0, 0.75, 0.3, 0.85],
    );
    let quads = rich_quads(&title);
    let background: Vec<[f32; 4]> = (0..size[0] * size[1])
        .map(|index| {
            let x = (index % size[0]) as f32 / size[0] as f32;
            let y = (index / size[0]) as f32 / size[1] as f32;
            [x, y, 0.5 * (1.0 - x), 1.0].map(|v| half::f16::from_f32(v).to_f32())
        })
        .collect();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let live = drawn(
        &gpu,
        size,
        Some(&background),
        &title,
        &quads,
        TextSpace::Overlay {
            width: size[0],
            height: size[1],
        },
    );
    let mut turns = Turns::default();
    turns.turn();
    let glyphs: Vec<Glyph> = text::glyphs(&title);
    let mut traced = background.clone();
    text::overlay(&mut traced, size[0], size[1], &title.atlas, &glyphs).unwrap();
    let byte = |value: f32| (encode_channel(value.clamp(0.0, 1.0)) * 255.0).round() as i32;
    let mut worst = 0;
    let mut touched = 0;
    for ((ours, theirs), under) in live.iter().zip(&traced).zip(&background) {
        if (0..3).any(|k| byte(theirs[k]) != byte(under[k])) {
            touched += 1;
        }
        for k in 0..3 {
            worst = worst.max((byte(ours[k]) - byte(theirs[k])).abs());
        }
    }
    println!("overlay: {touched} pixels carry text, worst difference {worst}/255");
    assert!(touched > 500, "{touched} pixels of overlay");
    assert!(worst <= 1, "{worst}/255 apart");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_stage_without_text_traces_the_same_bytes() {
    let floor = Arrays::floor(2.0);
    let block = Arrays::cube([0.2, 0.3, -0.1], [0.3, 0.3, 0.2]);
    let meshes = [floor.mesh(), block.mesh()];
    let placements = [
        Placement::new(0, IDENTITY, 0),
        Placement::new(1, IDENTITY, 1),
    ];
    let materials = [lambert([0.6; 3]), lambert([0.7, 0.3, 0.2])];
    let (camera, projection) = top_down(2.0, 1.25, [0.0, 0.0]);
    let staged = Stage {
        meshes: &meshes,
        instances: &placements,
        materials: &materials,
        sky: black(),
        sun: TraceSun {
            direction: unit([0.6, 1.0, 0.3]),
            color: [1.0; 3],
            intensity: 3.0,
        },
        camera,
        projection,
        lens: Lens::default(),
    }
    .build()
    .unwrap();
    assert!(staged.detail.content_slots.is_empty());
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let first = traced(&gpu, &staged, [96, 60], 8);
    let second = traced(&gpu, &staged, [96, 60], 8);
    assert_eq!(digest(&first), digest(&second));
    println!("text-free stage digest {}", digest(&first));
}

fn front_stage<'a>(
    carrier: &'a Carrier,
    content: PlacementContent<'a>,
    centre: [f32; 2],
    view: [f32; 2],
) -> Staged {
    let flip: Transform = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let materials = [text::material(false)];
    let placements = [Placement {
        content: Some(content),
        ..Placement::new(0, flip, 0)
    }];
    let (_, mut camera, projection) = front_on(view[0], view[1]);
    camera.origin = [centre[0], -centre[1], 1.0];
    stage(
        &[carrier.mesh()],
        &placements,
        &materials,
        (camera, projection),
        dark(),
    )
}

fn edge_width(pixels: &[[f32; 4]], width: usize) -> (f64, usize) {
    let value: Vec<f32> = pixels.iter().map(|p| p[0]).collect();
    let partial = value.iter().filter(|&&v| v > 0.01 && v < 0.99).count();
    let inside: Vec<bool> = value.iter().map(|&v| v >= 0.5).collect();
    let mut edges = 0;
    for (index, &here) in inside.iter().enumerate() {
        if index % width + 1 < width && inside[index + 1] != here {
            edges += 1;
        }
        if index + width < inside.len() && inside[index + width] != here {
            edges += 1;
        }
    }
    (partial as f64 / edges.max(1) as f64, edges)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn lettering_edges_stay_sharp_at_eight_times_zoom() {
    let size = [512u32, 256];
    let word = paragraph("Pito", 0.2, UNITS_PER_METRE, None, [1.0; 4]);
    let glyphs = text::glyphs(&word);
    let lettering = text::lettering(&word.atlas, &glyphs).unwrap();
    let wide = [0.5f32, 0.25];
    let density = size[0] as f32 / wide[0];
    let print = text::print(&word.atlas, &glyphs, density).unwrap();
    let close = wide.map(|v| v / 8.0);
    let sample = lettering.sampler();
    let ink = |centre: [f32; 2]| {
        let mut inside = 0;
        for row in 0..32 {
            for column in 0..64 {
                let at = [
                    centre[0] + close[0] * ((column as f32 + 0.5) / 64.0 - 0.5),
                    centre[1] + close[1] * ((row as f32 + 0.5) / 32.0 - 0.5),
                ];
                inside += usize::from(sample(at)[3] >= 0.5);
            }
        }
        inside as f32 / 2048.0
    };
    let centre = glyphs
        .iter()
        .map(|glyph| {
            [
                glyph.rect[0] + glyph.rect[2] * 0.5,
                glyph.rect[1] + glyph.rect[3] * 0.5,
            ]
        })
        .min_by(|a, b| (ink(*a) - 0.5).abs().total_cmp(&(ink(*b) - 0.5).abs()))
        .unwrap();
    let middle = [
        lettering.rect[0] + lettering.rect[2] * 0.5,
        lettering.rect[1] + lettering.rect[3] * 0.5,
    ];
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut widths = Vec::new();
    for (name, carrier, content) in [
        ("lettering", lettering.carrier(), lettering.content()),
        ("print", print.carrier(), print.content()),
    ] {
        let mut pair = [0.0; 2];
        for (k, (at, view)) in [(middle, wide), (centre, close)].into_iter().enumerate() {
            let staged = front_stage(&carrier, content, at, view);
            let pixels = traced(&gpu, &staged, size, 128);
            let (width, edges) = edge_width(&pixels, size[0] as usize);
            assert!(edges > 300, "{name}: {edges} edge pixels");
            println!(
                "{name} at {}x: {width:.3} pixels of edge per edge pixel over {edges}",
                [1, 8][k]
            );
            pair[k] = width;
        }
        widths.push(pair);
    }
    let lettered = widths[0][1] / widths[0][0];
    let printed = widths[1][1] / widths[1][0];
    println!("8x zoom: the lettering's edge grows {lettered:.3}x, the print's {printed:.3}x");
    assert!(
        (lettered - 1.0).abs() <= 0.1,
        "the lettering's edge grows {lettered:.3}x"
    );
    assert!(printed >= 3.0, "the print's edge grows only {printed:.3}x");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn lettering_over_a_picture_lands_where_the_composed_print_does() {
    let half = [0.3, 0.15, 0.05];
    let (view_width, view_height) = (2.0 * half[0], 2.0 * half[1]);
    let size = [600u32, 300];
    let part = manf_box(half);
    let word = paragraph(
        "Bench",
        0.08,
        UNITS_PER_METRE,
        Some([-0.22, -0.11]),
        [1.0; 4],
    );
    let model: Transform = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, half[2] + LIFT, 1.0],
    ];
    let fit = text::fit(&part.mesh(), IDENTITY, model).unwrap();
    let glyphs = text::glyphs(&word);
    let lettering = text::lettering(&word.atlas, &glyphs).unwrap();
    let side = 512u32;
    let grey: Vec<[u8; 4]> = (0..side * side)
        .map(|index| {
            let shade = 60 + (index % side * 40 / side) as u8;
            [shade, shade, shade, 255]
        })
        .collect();
    let picture = PlacementContent::new(Image {
        width: side,
        height: side,
        texels: &grey,
        srgb: true,
    });
    let over = fit.over(&picture, &lettering).unwrap();
    let composed = fit.onto(&picture, &word.atlas, &glyphs).unwrap();
    let printed = PlacementContent::new(Image {
        width: side,
        height: side,
        texels: &composed,
        srgb: true,
    });
    let (_, camera, projection) = front_on(view_width, view_height);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let materials = [text::material(false)];
    let mut centres = Vec::new();
    for (name, content) in [("lettering", over), ("composed print", printed)] {
        let placements = [Placement {
            content: Some(content),
            ..Placement::new(0, IDENTITY, 0)
        }];
        let staged = stage(
            &[part.mesh()],
            &placements,
            &materials,
            (camera, projection),
            dark(),
        );
        let traced = traced(&gpu, &staged, size, 16);
        let ink = traced.iter().filter(|p| p[0] >= 0.6).count();
        let bare = traced
            .iter()
            .filter(|p| (0.03..0.12).contains(&p[0]))
            .count();
        assert!(ink > 2000, "{name}: {ink} pixels of ink");
        assert!(bare > 50_000, "{name}: {bare} pixels of picture");
        let at = centroid(
            traced.iter().map(|p| if p[0] >= 0.6 { 1.0 } else { 0.0 }),
            size[0],
        );
        println!("picture and text, {name}: {ink} pixels of ink centred at {at:?}");
        centres.push(at);
    }
    let texels_per_pixel = f64::from(side) / f64::from(size[0]);
    let apart = [0, 1].map(|k| (centres[0][k] - centres[1][k]).abs() * texels_per_pixel);
    println!("picture and text: {apart:?} picture texels apart");
    assert!(apart.iter().all(|&d| d <= 1.0), "{apart:?} texels apart");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn lettering_cost_per_sample_against_the_print() {
    let size = [1024u32, 512];
    let lines = [
        "The tracer draws text from the MSDF at every hit, never baked,",
        "so a shot can zoom past any density and the edges stay crisp.",
        "Each sample finds the glyphs under its hit through a small grid,",
        "reads the atlas and takes the median against one half, and the",
        "jitter of the samples does the anti-aliasing at any distance.",
    ]
    .join("\n");
    let word = paragraph(&lines, 0.03, UNITS_PER_METRE, None, [1.0; 4]);
    let glyphs = text::glyphs(&word);
    let lettering = text::lettering(&word.atlas, &glyphs).unwrap();
    let rect = lettering.rect;
    let view = [rect[2] * 1.05, rect[2] * 1.05 * 0.5];
    let density = 2.0 * size[0] as f32 / view[0];
    let print = text::print(&word.atlas, &glyphs, density).unwrap();
    let middle = [rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5];
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let samples = 16;
    let mut costs = Vec::new();
    for (name, carrier, content) in [
        ("lettering", lettering.carrier(), lettering.content()),
        ("print", print.carrier(), print.content()),
    ] {
        let staged = front_stage(&carrier, content, middle, view);
        let (pixels, ms) = timed(&gpu, &staged, size, samples);
        let ink = pixels.iter().filter(|p| p[0] >= 0.5).count();
        assert!(ink > 10_000, "{name}: {ink} pixels of ink");
        println!(
            "{name}: {} glyphs at {}x{}, {ms:.3} ms per sample",
            glyphs.len(),
            size[0],
            size[1]
        );
        costs.push(ms);
    }
    println!(
        "lettering costs {:.2}x the print per sample ({} print texels)",
        costs[0] / costs[1],
        print.width * print.height
    );
    assert!(glyphs.len() >= 250, "{} glyphs", glyphs.len());
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn lettering_cut_at_a_placement_cutoff_moves_its_edge_as_an_image_does() {
    let side = 64u32;
    let ramp: Vec<u8> = (0..side * side)
        .map(|index| {
            let x = ((index % side) as f32 + 0.5) / side as f32 - 0.5;
            let y = ((index / side) as f32 + 0.5) / side as f32 - 0.5;
            let coverage = ((0.45 - (x * x + y * y).sqrt()) / 0.25).clamp(0.0, 1.0);
            (coverage * 255.0).round() as u8
        })
        .collect();
    let atlas = Atlas {
        channels: 1,
        levels: vec![MipLevel {
            width: side,
            height: side,
            bytes: ramp.clone(),
        }],
    };
    let disc = Glyph {
        rect: [-0.5, -0.5, 1.0, 1.0],
        uv: [0.0, 0.0, 1.0, 1.0],
        color: [0.0, 0.0, 0.0, 1.0],
        clip: OPEN,
        turn: [0.0; 3],
    };
    let lettering = text::lettering(&atlas, &[disc]).unwrap();
    assert_eq!(lettering.rect, disc.rect);
    let texels: Vec<[u8; 4]> = ramp.iter().map(|&alpha| [0, 0, 0, alpha]).collect();
    let image = Image {
        width: side,
        height: side,
        texels: &texels,
        srgb: false,
    };
    let carrier = lettering.carrier();
    let backdrop = Arrays {
        positions: vec![
            [-1.0, -1.0, -0.3],
            [1.0, -1.0, -0.3],
            [1.0, 1.0, -0.3],
            [-1.0, 1.0, -0.3],
        ],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        tangents: vec![[1.0, 0.0, 0.0, 1.0]; 4],
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        indices: vec![0, 1, 2, 0, 2, 3],
    };
    let glow = Material {
        base: [0.0; 3],
        emission: [1.0; 3],
        ..lambert([0.0; 3])
    };
    let materials = [text::material(false), glow];
    let size = [256u32, 256];
    let view = 1.2f32;
    let (_, camera, projection) = front_on(view, view);
    let pixels_per_unit = size[0] as f32 / view;
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut radii = Vec::new();
    for cutoff in [0.25f32, 0.9] {
        let expected = (0.45 - 0.25 * cutoff) * pixels_per_unit;
        let mut pair = Vec::new();
        for (name, kind) in [
            ("lettering", lettering.content()),
            ("image", {
                PlacementContent {
                    cutout: true,
                    ..PlacementContent::new(image)
                }
            }),
        ] {
            let placements = [
                Placement {
                    content: Some(kind),
                    alpha_cutoff: cutoff,
                    ..Placement::new(0, IDENTITY, 0)
                },
                Placement::new(1, IDENTITY, 1),
            ];
            let staged = stage(
                &[carrier.mesh(), backdrop.mesh()],
                &placements,
                &materials,
                (camera, projection),
                dark(),
            );
            let traced = traced(&gpu, &staged, size, 32);
            let kept: f64 = traced
                .iter()
                .map(|p| f64::from(1.0 - p[0].clamp(0.0, 1.0)))
                .sum();
            let radius = (kept / std::f64::consts::PI).sqrt();
            println!(
                "cut at {cutoff}, {name}: kept radius {radius:.3} px, the ramp puts it at {expected:.3}"
            );
            assert!(
                (radius - f64::from(expected)).abs() <= 1.5,
                "{name} at {cutoff}: {radius:.3} px against {expected:.3}"
            );
            pair.push(radius);
        }
        assert!(
            (pair[0] - pair[1]).abs() <= 0.25,
            "at {cutoff} the lettering keeps {:.3} px and the image {:.3}",
            pair[0],
            pair[1]
        );
        radii.push(pair);
    }
    for (wide, narrow) in radii[0].iter().zip(&radii[1]) {
        assert!(
            wide - narrow > 30.0,
            "the edge moves {:.3} px",
            wide - narrow
        );
    }
}
