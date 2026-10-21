use std::path::{Path, PathBuf};
use std::sync::Arc;

use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::frame::batches;
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::fixture::{KitMaps, kit_quad};
use pfx_load::scene::Scene;
use pfx_post::Chain;

const WIDTH: u32 = 384;
const HEIGHT: u32 = 256;
const SAMPLES: u32 = 48;
const TINT: [f32; 3] = [1.0, 0.5, 0.25];
const RECOLOR: [f32; 3] = [0.2, 0.6, 0.2];
const MATTE: f32 = 0.8;
const CENTRES: [u32; 3] = [87, 192, 297];
const LIVE_BOUND: f32 = 0.08;
const TRACED_BOUND: f32 = 0.12;

fn folder(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("tint")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("panel.glb"),
        kit_quad("panel", [0.5, 0.5], [1.0; 4], KitMaps::NONE),
    )
    .unwrap();
    std::fs::write(
        root.join("tint.materials.toml"),
        format!("format = 1\n\n[materials.matte]\nbase = [{MATTE:?}, {MATTE:?}, {MATTE:?}]\nroughness = 1.0\nspecular = 0.0\n"),
    )
    .unwrap();
    let [r, g, b] = TINT;
    let [rr, rg, rb] = RECOLOR;
    let path = root.join("tint.scene.toml");
    std::fs::write(
        &path,
        format!(
            "format = 1\nmaterials = [\"tint.materials.toml\"]\n\n[mesh.panel]\nfile = \"panel.glb\"\n\n\
             [[object]]\nname = \"plain\"\nmesh = \"panel\"\nmaterial = \"matte\"\nat = [-1.2, 0.0, 0.0]\n\n\
             [[object]]\nname = \"tinted\"\nmesh = \"panel\"\nmaterial = \"matte\"\ntint = [{r:?}, {g:?}, {b:?}]\n\n\
             [[object]]\nname = \"recolored\"\nmesh = \"panel\"\nmaterial = \"matte\"\nat = [1.2, 0.0, 0.0]\n\n\
             [object.recolor.matte]\nbase = [{rr:?}, {rg:?}, {rb:?}]\n\n\
             [sun]\nmodel = \"authored\"\ntoward = [0.2, 0.4, 1.0]\ncolor = [1.0, 1.0, 1.0]\nirradiance = 3.0\n\n\
             [sky]\nkind = \"analytic\"\n\n[camera]\nat = [0.0, 0.0, 4.0]\nlook_at = [0.0, 0.0, 0.0]\nfov = 40.0\n"
        ),
    )
    .unwrap();
    path
}

fn output(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tint output"),
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

fn region(pixels: &[[f32; 3]], centre: u32) -> [f32; 3] {
    let mut sum = [0.0f32; 3];
    let mut count = 0.0;
    for row in HEIGHT / 2 - 16..HEIGHT / 2 + 16 {
        for column in centre - 16..centre + 16 {
            let pixel = pixels[(row * WIDTH + column) as usize];
            for c in 0..3 {
                sum[c] += pixel[c];
            }
            count += 1.0;
        }
    }
    sum.map(|value| value / count)
}

fn check(pixels: &[[f32; 3]], bound: f32, what: &str) {
    let [plain, tinted, recolored] = CENTRES.map(|centre| region(pixels, centre));
    assert!(
        plain.iter().all(|&value| value > 0.02),
        "{what}: plain panel is dark: {plain:?}"
    );
    for c in 0..3 {
        let wanted = [TINT[c], RECOLOR[c] / MATTE];
        let seen = [tinted[c] / plain[c], recolored[c] / plain[c]];
        for (wanted, seen) in wanted.into_iter().zip(seen) {
            assert!(
                (seen - wanted).abs() <= bound * wanted.max(0.25),
                "{what}: channel {c} draws {seen} of the plain panel where {wanted} is wanted ({plain:?} {tinted:?} {recolored:?})"
            );
        }
    }
}

fn live(scene: &Scene) -> Vec<[f32; 3]> {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let target = output(&gpu);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = renderer.stage(scene).unwrap();
    renderer.set_finish(Chain::new());
    for _ in 0..8 {
        let finish = staged.finish();
        let drawn = staged.frame(WIDTH as f32 / HEIGHT as f32, 0.0, 7);
        assert_eq!(drawn.scene.instances.len(), 3);
        assert_eq!(
            batches(drawn.scene.instances, |_| false, |_| false).len(),
            1
        );
        renderer
            .render(
                &drawn.scene,
                &drawn.text,
                &drawn.effects,
                finish,
                &target.view,
            )
            .unwrap();
    }
    let pixels = renderer
        .gpu()
        .readback_rgba16(&target)
        .unwrap()
        .chunks_exact(4)
        .map(|p| {
            std::array::from_fn(|c| {
                pfx_materials::linear_channel(half::f16::from_bits(p[c]).to_f32())
            })
        })
        .collect();
    staged.release(&mut renderer).unwrap();
    pixels
}

fn traced(scene: &Scene) -> Vec<[f32; 3]> {
    let staged = pfx_trace::stage::scene_at(scene, WIDTH as f32 / HEIGHT as f32, 0.0).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut trace = staged.trace(&gpu, WIDTH, HEIGHT).unwrap();
    for pass in 0..SAMPLES {
        trace.sample(&gpu, 1, 11 + pass).unwrap();
    }
    trace
        .readback(&gpu)
        .unwrap()
        .color
        .chunks_exact(16)
        .map(|p| {
            std::array::from_fn(|c| f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()))
        })
        .collect()
}

#[test]
fn tinted_copies_stage_as_one_shared_traced_mesh_and_material() {
    let scene = Scene::open(folder("staged")).unwrap();
    let draws = scene.draws();
    assert_eq!(draws.materials.len(), 1);
    let staged = pfx_trace::stage::scene_at(&scene, WIDTH as f32 / HEIGHT as f32, 0.0).unwrap();
    let instances = &staged.detail.instances;
    assert_eq!(instances.len(), 3);
    assert!(
        instances
            .iter()
            .all(|instance| Arc::ptr_eq(&instance.triangles, &instances[0].triangles))
    );
    assert_eq!(instances[1].surface.tint.base, TINT);
    assert!(instances[2].surface.tint.replace);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_tinted_instance_draws_its_multiplied_base_live() {
    let scene = Scene::open(folder("live")).unwrap();
    check(&live(&scene), LIVE_BOUND, "live");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_tinted_instance_draws_its_multiplied_base_traced() {
    let scene = Scene::open(folder("traced")).unwrap();
    check(&traced(&scene), TRACED_BOUND, "traced");
}
