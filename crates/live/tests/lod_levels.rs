use std::path::Path;
use std::time::Instant;

use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::frame::{Camera, Instance, MeshData, MeshHandle};
use pfx_live::lod::LodSet;
use pfx_live::morph::InstanceWeights;
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::Staged;
use pfx_load::fixture::{
    LOD_RADIUS, LOD_RINGS, LOD_TARGETS, lod_error, lod_glb, lod_morph_glb, lod_sphere, lod_targets,
    plain_glb,
};
use pfx_load::scene::Scene;
use pfx_post::Chain;

const WIDTH: u32 = 192;
const HEIGHT: u32 = 128;
const ASPECT: f32 = WIDTH as f32 / HEIGHT as f32;
const FOV: f32 = 40.0;
const IDENTITY: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn scene(name: &str, glb: &[u8], distance: f32) -> Scene {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("lod-levels")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("ball.glb"), glb).unwrap();
    std::fs::write(
        root.join("ball.materials.toml"),
        "[materials.matte]\nbase = [0.6, 0.5, 0.4]\nroughness = 0.6\nspecular = 0.3\n",
    )
    .unwrap();
    let path = root.join("ball.scene.toml");
    std::fs::write(
        &path,
        format!(
            "materials = [\"ball.materials.toml\"]\nfallback = \"matte\"\n\n[mesh.ball]\nfile = \"ball.glb\"\n\n[[object]]\nname = \"ball\"\nmesh = \"ball\"\n\n[sun]\nmodel = \"authored\"\ntoward = [0.3, 0.6, 0.7]\nirradiance = 4.0\n\n[sky]\nkind = \"room\"\n\n[sky.room]\nwidth = 64\nfloor = [0.05, 0.05, 0.05]\nwall = [0.05, 0.05, 0.05]\nceiling = [0.05, 0.05, 0.05]\n\n[camera]\nat = [0.0, 0.0, {distance:?}]\nlook_at = [0.0, 0.0, 0.0]\nfov = {FOV:?}\nfar = 200.0\n"
        ),
    )
    .unwrap();
    Scene::open(&path).unwrap()
}

fn output(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("lod levels output"),
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

fn render(scene: &Scene) -> (Vec<u16>, Option<usize>) {
    render_with(scene, |_, _| {})
}

fn render_with(
    scene: &Scene,
    prepare: impl FnOnce(&mut Renderer, &Staged),
) -> (Vec<u16>, Option<usize>) {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = output(&gpu);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = renderer.stage(scene).unwrap();
    renderer.set_finish(Chain::new());
    prepare(&mut renderer, &staged);
    for _ in 0..3 {
        let finish = staged.finish();
        let drawn = staged.frame(ASPECT, 0.0, 7);
        renderer
            .render(
                &drawn.scene,
                &drawn.text,
                &drawn.effects,
                finish,
                &output.view,
            )
            .unwrap();
    }
    let level = renderer.lod_mut().swapped_level(0);
    let pixels = renderer.gpu().readback_rgba16(&output).unwrap();
    staged.release(&mut renderer).unwrap();
    assert!(!renderer.lod_mut().has_levels());
    (pixels, level)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn prebuilt_levels_coarsen_as_the_ball_shrinks_and_draw_as_their_own_meshes() {
    let mut levels = Vec::new();
    for (distance, expected) in [(3.0, 0), (20.0, 1), (90.0, 2)] {
        let name = format!("lod-{expected}");
        let (pixels, level) = render(&scene(&name, &lod_glb("ball", None, true), distance));
        assert_eq!(level, Some(expected), "at {distance} m");
        let plain = format!("plain-{expected}");
        let (alone, none) = render(&scene(
            &plain,
            &plain_glb("ball", LOD_RINGS[expected]),
            distance,
        ));
        assert_eq!(none, None);
        assert!(pixels == alone, "level {expected} at {distance} m");
        assert!(pixels.chunks_exact(4).any(|pixel| pixel[0] != pixels[0]));
        levels.push(level);
    }
    assert_eq!(levels, [Some(0), Some(1), Some(2)]);
}

fn channel(bits: u16) -> f32 {
    half::f16::from_bits(bits).to_f32().clamp(0.0, 1.0)
}

fn differing(a: &[u16], b: &[u16]) -> usize {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| (0..3).any(|c| (channel(a[c]) - channel(b[c])).abs() > 1.0 / 255.0))
        .count()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_morphed_level_at_a_weight_draws_as_that_level_blended_on_the_cpu() {
    let distance = 9.0;
    let set = [0.8_f32, 0.6];
    let weighted = |renderer: &mut Renderer, _: &Staged| {
        renderer
            .set_weights(&[InstanceWeights {
                instance: 0,
                current: &set,
                previous: &set,
            }])
            .unwrap();
    };
    let glb = lod_morph_glb("ball", &LOD_TARGETS);
    let (live, level) = render_with(&scene("morphed-lod", &glb, distance), weighted);
    assert_eq!(level, Some(1));
    let (rest, _) = render(&scene("morphed-rest", &glb, distance));
    let rings = LOD_RINGS[1];
    let (positions, tangents, uvs, indices) = lod_sphere(rings);
    let (bulge, bulge_normals, slide) = lod_targets(&positions);
    let blended: Vec<[f32; 3]> = (0..positions.len())
        .map(|i| {
            std::array::from_fn(|c| positions[i][c] + bulge[i][c] * set[0] + slide[i][c] * set[1])
        })
        .collect();
    let normals: Vec<[f32; 3]> = (0..positions.len())
        .map(|i| {
            std::array::from_fn(|c| positions[i][c] / LOD_RADIUS + bulge_normals[i][c] * set[0])
        })
        .collect();
    let (reference, none) = render_with(
        &scene("morphed-cpu", &plain_glb("ball", rings), distance),
        |renderer, staged| {
            renderer
                .replace_mesh(
                    staged.instances()[0].mesh,
                    MeshData {
                        positions: &blended,
                        normals: &normals,
                        tangents: &tangents,
                        uvs: &uvs,
                        uvs1: None,
                        alpha: None,
                        indices: &indices,
                    },
                )
                .unwrap();
        },
    );
    assert_eq!(none, None);
    let moved = differing(&live, &rest);
    let flips = differing(&live, &reference);
    println!("weighted against rest: {moved} pixels; against the CPU blend: {flips}");
    assert!(moved > 20, "the weights move the level ({moved} pixels)");
    assert!(flips <= 4, "{flips} pixels differ from the CPU blend");
}

fn camera(z: f32) -> Camera {
    let mut projection = [[0.0; 4]; 4];
    projection[0][0] = 1.0;
    projection[1][1] = 1.0 / (FOV.to_radians() * 0.5).tan();
    projection[2][3] = -1.0;
    let mut view = IDENTITY;
    view[3][2] = -z;
    Camera {
        view,
        projection,
        previous_view_projection: projection,
        position: [0.0, 0.0, z],
    }
}

#[test]
fn selection_of_a_thousand_leveled_instances_is_measured() {
    let mut set = LodSet::new(2_000_000);
    let mut bounds = pfx_geom::mesh::Bounds::empty();
    bounds.include([-0.5; 3]);
    bounds.include([0.5; 3]);
    let chain = [
        (MeshHandle(0), 0.0),
        (MeshHandle(1), lod_error(LOD_RINGS[1])),
        (MeshHandle(2), lod_error(LOD_RINGS[2])),
    ];
    set.set_levels(bounds, &chain).unwrap();
    let instances: Vec<Instance> = (0..1000)
        .map(|index| {
            let mut model = IDENTITY;
            model[3][2] = -(index as f32);
            Instance::new(MeshHandle(0), model, 0, index + 1)
        })
        .collect();
    let mut coarse = 0;
    let started = Instant::now();
    let rounds = 200;
    for round in 0..rounds {
        let swapped = set
            .swap_levels(&instances, camera(5.0 + round as f32 * 0.01), 3840)
            .unwrap();
        coarse += swapped
            .iter()
            .filter(|instance| instance.mesh != MeshHandle(0))
            .count();
    }
    let each = started.elapsed().as_secs_f64() * 1e6 / rounds as f64;
    println!("LOD level selection: {each:.1} us per frame for 1000 instances");
    let first = set.swapped_level(0).unwrap();
    let last = set.swapped_level(999).unwrap();
    assert!(first < last, "{first} {last}");
    assert!(coarse > 0);
}
