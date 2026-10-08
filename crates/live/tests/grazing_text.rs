use pfx_gpu::pace::{Pacer, Turns};
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::frame::{Matrix, multiply};
use pfx_live::text::{
    COLOR_FORMAT, DEPTH_FORMAT, GpuAtlas, QuadDraw, TextPass, TextSpace, rich_quads,
};
use pfx_load::Sky;
use pfx_text::{Anchor, Face, Representation, RichParagraph, Span, TextEngine};
use pfx_trace::detail::{Lens, Transform};
use pfx_trace::stage::{Placement, Stage};
use pfx_trace::text;
use pfx_trace::{Camera as TraceCamera, Projection, Sun as TraceSun};

const SERIF: &[u8] = pfx_text::fixture::EB_GARAMOND;
const SANS: &[u8] = pfx_text::fixture::DM_SANS;
const UNITS_PER_METRE: f32 = 8192.0;
const OPEN: [f32; 4] = [-1e9, -1e9, 1e9, 1e9];
const SIZE: [u32; 2] = [1024, 256];
const SAMPLE: &str = "Sphinx of black quartz, judge my vow. Pack my box with five dozen jugs.";
const DISTANCE: f32 = 1.0;
const FOV_Y: f32 = 20.0;

fn paragraph(font: &[u8], family: &str, weight: u16, em: f32) -> RichParagraph {
    let mut engine = TextEngine::new(font).unwrap();
    let spans = [Span {
        text: SAMPLE.into(),
        face: Face {
            family: family.into(),
            size: em,
            line: em * 1.25,
            weight,
            italic: false,
            spacing: 0.0,
        },
        color: [1.0; 4],
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

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

fn eye(degrees: f32) -> (Matrix, TraceCamera, Projection) {
    let (s, c) = degrees.to_radians().sin_cos();
    let eye = [0.0, DISTANCE * s, DISTANCE * c];
    let forward = [0.0, -s, -c];
    let right = [1.0, 0.0, 0.0];
    let up = [0.0, c, -s];
    let view: Matrix = [
        [right[0], up[0], -forward[0], 0.0],
        [right[1], up[1], -forward[1], 0.0],
        [right[2], up[2], -forward[2], 0.0],
        [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0],
    ];
    let tan = (FOV_Y.to_radians() * 0.5).tan();
    let aspect = SIZE[0] as f32 / SIZE[1] as f32;
    let (near, far) = (0.05, 30.0);
    let projection: Matrix = [
        [1.0 / (tan * aspect), 0.0, 0.0, 0.0],
        [0.0, 1.0 / tan, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ];
    (
        multiply(projection, view),
        TraceCamera {
            origin: eye,
            forward,
            right: right.map(|v| v * tan * aspect),
            up: up.map(|v| v * tan),
        },
        Projection::Perspective,
    )
}

fn live(gpu: &Gpu, paragraph: &RichParagraph, model_view_projection: Matrix) -> Vec<f32> {
    let target: OffscreenTarget = gpu.offscreen(SIZE[0], SIZE[1], COLOR_FORMAT).unwrap();
    let extent = wgpu::Extent3d {
        width: SIZE[0],
        height: SIZE[1],
        depth_or_array_layers: 1,
    };
    let depth = gpu
        .device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("grazing text depth"),
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
            label: Some("grazing text clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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
    pass.encode_quads(
        &gpu.device,
        &mut encoder,
        QuadDraw {
            target: &target.view,
            depth: Some(&depth),
            atlas: &atlas,
            quads: &rich_quads(paragraph),
            space: TextSpace::Surface {
                model_view_projection,
            },
            deformers: None,
            icons: false,
        },
    )
    .unwrap();
    gpu.queue.submit(Some(encoder.finish()));
    gpu.readback_rgba16(&target)
        .unwrap()
        .chunks_exact(4)
        .map(|p| half::f16::from_bits(p[3]).to_f32())
        .collect()
}

fn traced(gpu: &Gpu, paragraph: &RichParagraph, model: Transform, degrees: f32) -> Vec<f32> {
    let glyphs = text::glyphs(paragraph);
    let lettering = text::lettering(&paragraph.atlas, &glyphs).unwrap();
    let carrier = lettering.carrier();
    let materials = [text::material(false)];
    let placements = [Placement {
        content: Some(lettering.content()),
        ..Placement::new(0, model, 0)
    }];
    let (_, camera, projection) = eye(degrees);
    let staged = Stage {
        meshes: &[carrier.mesh()],
        instances: &placements,
        materials: &materials,
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![[0.0, 0.0, 0.0, 1.0]],
        },
        sun: TraceSun {
            direction: [0.0, 1.0, 0.0],
            color: [1.0; 3],
            intensity: 0.0,
        },
        camera,
        projection,
        lens: Lens::default(),
    }
    .build()
    .unwrap();
    let mut trace = staged.trace(gpu, SIZE[0], SIZE[1]).unwrap();
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    for pass in 0..64 {
        trace
            .sample_paced(gpu, 1, 11 + pass, &mut pacer, |ms| turns.add(ms))
            .unwrap();
    }
    turns.turn();
    trace
        .readback(gpu)
        .unwrap()
        .color
        .chunks_exact(16)
        .map(|p| f32::from_le_bytes(p[0..4].try_into().unwrap()))
        .collect()
}

fn iou(a: &[bool], b: &[bool]) -> f64 {
    let both = a.iter().zip(b).filter(|(a, b)| **a && **b).count();
    let either = a.iter().zip(b).filter(|(a, b)| **a || **b).count();
    both as f64 / either.max(1) as f64
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn bold_and_regular_text_keep_their_weight_at_grazing_views() {
    let model: Transform = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let angles = [90.0f32, 45.0, 25.0, 10.0];
    let mut worst = [f64::INFINITY; 4];
    for em in [0.055f32, 0.0275] {
        for (font, family) in [(SANS, "DM Sans"), (SERIF, "EB Garamond")] {
            for weight in [400u16, 700] {
                let paragraph = paragraph(font, family, weight, em);
                let mut face_on = 0.0;
                for (slot, degrees) in angles.into_iter().enumerate() {
                    let ours = live(&gpu, &paragraph, multiply(eye(degrees).0, model));
                    let mut turns = Turns::default();
                    turns.turn();
                    let theirs = traced(&gpu, &paragraph, model, degrees);
                    let ours_mask: Vec<bool> = ours.iter().map(|&a| a >= 0.5).collect();
                    let theirs_mask: Vec<bool> = theirs.iter().map(|&a| a >= 0.5).collect();
                    let overlap = iou(&ours_mask, &theirs_mask);
                    let ink = |pixels: &[f32]| -> f64 {
                        pixels.iter().map(|&a| f64::from(a.clamp(0.0, 1.0))).sum()
                    };
                    let weight_kept = ink(&ours) / ink(&theirs);
                    let covered = theirs_mask.iter().filter(|&&inside| inside).count();
                    let case = format!("{family} {weight} at {em} m, {degrees} degrees");
                    println!(
                        "{case}: IoU {overlap:.4}, ink live/traced {weight_kept:.3}, {covered} traced pixels"
                    );
                    assert!(covered > 100, "{case}: {covered} pixels");
                    assert!(
                        (0.85..=1.15).contains(&weight_kept),
                        "{case}: ink live/traced {weight_kept:.3}"
                    );
                    if slot == 0 {
                        face_on = overlap;
                    } else if degrees >= 25.0 {
                        assert!(
                            overlap >= face_on - 0.05,
                            "{case}: IoU {overlap:.4} against {face_on:.4} face on"
                        );
                    } else {
                        assert!(overlap >= 0.78, "{case}: IoU {overlap:.4}");
                    }
                    if weight == 700 && degrees >= 25.0 {
                        assert!(overlap >= 0.9, "{case}: IoU {overlap:.4}");
                    }
                    worst[slot] = worst[slot].min(overlap);
                }
            }
        }
    }
    for (degrees, overlap) in angles.iter().zip(worst) {
        println!("worst IoU at {degrees} degrees: {overlap:.4}");
    }
}
