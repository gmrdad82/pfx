use std::path::{Path, PathBuf};

use pfx_gpu::pace::{Pacer, Turns};
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::{Override, Staged};
use pfx_load::scene::Scene;
use pfx_post::Chain;
use pfx_post::color::srgb_channel;

const WIDTH: u32 = 192;
const HEIGHT: u32 = 128;
const ASPECT: f32 = WIDTH as f32 / HEIGHT as f32;

const MATERIALS: &str = "[materials.glow]
base = [0.0, 0.0, 0.0]
roughness = 1.0
specular = 0.0
emission = [1.0, 1.0, 1.0]

[materials.dim]
base = [0.0, 0.0, 0.0]
roughness = 1.0
specular = 0.0
emission = [0.2, 0.2, 0.2]

[materials.matte]
base = [0.7, 0.7, 0.7]
roughness = 1.0
specular = 0.0

[materials.glass]
family = \"glass\"
base = [1.0, 1.0, 1.0]
roughness = 0.02
transmission = 1.0
ior = 1.5
thickness = 0.01
";

fn folder(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("scene-keys")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    for entry in std::fs::read_dir(fixtures).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
    }
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../text/fonts/EBGaramond[wght].ttf"),
        root.join("serif.ttf"),
    )
    .unwrap();
    std::fs::write(root.join("keys.materials.toml"), MATERIALS).unwrap();
    root
}

fn scene(name: &str, body: &str) -> Scene {
    let root = folder(name);
    let path = root.join("keys.scene.toml");
    std::fs::write(
        &path,
        format!("materials = [\"keys.materials.toml\"]\nfallback = \"matte\"\n\n[mesh.block]\nfile = \"block.gltf\"\n\n[mesh.panel]\nfile = \"panel.gltf\"\n\n{body}"),
    )
    .unwrap();
    Scene::open(&path).unwrap()
}

fn output(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene keys output"),
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
    staged: Staged,
    output: OffscreenTarget,
    turns: Turns,
}

impl Viewer {
    fn new(scene: &Scene) -> Self {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let output = output(&gpu);
        let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
        renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
        let staged = renderer.stage(scene).unwrap();
        renderer.set_finish(Chain::new());
        Self {
            renderer,
            staged,
            output,
            turns: Turns::default(),
        }
    }

    fn frames(&mut self, time: f32, count: u32) -> Vec<[f32; 3]> {
        for _ in 0..count {
            let finish = self.staged.finish();
            let drawn = self.staged.frame(ASPECT, time, 7);
            let renderer = &mut self.renderer;
            let output = &self.output;
            self.turns.poll();
            let timings = renderer
                .render(
                    &drawn.scene,
                    &drawn.text,
                    &drawn.effects,
                    finish,
                    &output.view,
                )
                .unwrap();
            self.turns
                .add(pfx_gpu::pace::gpu_ms(&timings).unwrap_or(f64::NAN));
        }
        self.turns.turn();
        self.renderer
            .gpu()
            .readback_rgba16(&self.output)
            .unwrap()
            .chunks_exact(4)
            .map(|p| std::array::from_fn(|c| srgb_channel(half::f16::from_bits(p[c]).to_f32())))
            .collect()
    }
}

fn traced(scene: &Scene, time: f32, samples: u32) -> Vec<[f32; 3]> {
    let staged = pfx_trace::stage::scene_at(scene, ASPECT, time).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut trace = staged.trace(&gpu, WIDTH, HEIGHT).unwrap();
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    for pass in 0..samples {
        trace
            .sample_paced(&gpu, 1, 11 + pass, &mut pacer, |ms| turns.add(ms))
            .unwrap();
    }
    turns.turn();
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

fn luminance(pixel: &[f32; 3]) -> f32 {
    0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2]
}

fn mask(pixels: &[[f32; 3]], threshold: f32) -> Vec<bool> {
    pixels
        .iter()
        .map(|pixel| luminance(pixel) >= threshold)
        .collect()
}

fn iou(a: &[bool], b: &[bool]) -> f64 {
    let both = a.iter().zip(b).filter(|(a, b)| **a && **b).count();
    let either = a.iter().zip(b).filter(|(a, b)| **a || **b).count();
    both as f64 / either.max(1) as f64
}

fn count(mask: &[bool]) -> usize {
    mask.iter().filter(|&&inside| inside).count()
}

fn mean(pixels: &[[f32; 3]]) -> [f64; 3] {
    let mut sum = [0.0f64; 3];
    for pixel in pixels {
        for k in 0..3 {
            sum[k] += f64::from(pixel[k]);
        }
    }
    sum.map(|total| total / pixels.len() as f64)
}

fn centroid(mask: &[bool]) -> [f64; 2] {
    let mut sum = [0.0f64; 3];
    for (index, inside) in mask.iter().enumerate() {
        if *inside {
            sum[0] += (index % WIDTH as usize) as f64 + 0.5;
            sum[1] += (index / WIDTH as usize) as f64 + 0.5;
            sum[2] += 1.0;
        }
    }
    assert!(sum[2] > 0.0, "the mask is empty");
    [sum[0] / sum[2], sum[1] / sum[2]]
}

const POSED: &str = "[[object]]
name = \"card\"
mesh = \"panel\"
at = [-1.1, 0.3, 0.0]
rotate = [0.0, 75.0, 0.0]
scale = [0.9, 0.9, 1.0]
material = \"glow\"
face_camera = true
alpha_cutoff = 0.25

[[object]]
name = \"slider\"
mesh = \"block\"
at = [0.4, -0.4, 0.0]
scale = 0.6
material = \"glow\"

[[object]]
name = \"cap\"
mesh = \"block\"
parent = \"slider\"
at = [0.0, 0.9, 0.0]
scale = 0.5
material = \"glow\"

[[mover]]
name = \"track\"
objects = [\"slider\"]
kind = \"slide\"
axis = [1.0, 0.0, 0.0]
travel = [0.0, 0.9]
period = 2.0

[[mover]]
name = \"tilt\"
objects = [\"cap\"]
kind = \"turn\"
pivot = [0.4, 0.2, 0.0]
axis = [0.0, 0.0, 1.0]
travel = [0.0, 40.0]
period = 2.0
";

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn cards_movers_and_lens_shift_draw_live_where_the_tracer_puts_them() {
    let scene = scene(
        "posed",
        &format!(
            "{POSED}\n[camera]\nat = [1.5, 0.6, 5.0]\nlook_at = [0.0, 0.0, 0.0]\nfov = 40.0\nshift = [0.15, -0.1]\n"
        ),
    );
    let mut viewer = Viewer::new(&scene);
    let card = scene
        .draws()
        .items
        .iter()
        .position(|draw| draw.object == "card")
        .unwrap();
    assert_eq!(viewer.staged.instances()[card].alpha_cutoff, 0.25);
    let rest = mask(&viewer.frames(0.0, 6), 0.5);
    let moved = mask(&viewer.frames(1.0, 6), 0.5);
    let traced_rest = mask(&traced(&scene, 0.0, 16), 0.5);
    let traced_moved = mask(&traced(&scene, 1.0, 16), 0.5);
    let staged = pfx_trace::stage::scene_at(&scene, ASPECT, 1.0).unwrap();
    assert_eq!(staged.detail.instances[card].surface.alpha_cutoff, 0.25);
    let at_rest = iou(&rest, &traced_rest);
    let at_one = iou(&moved, &traced_moved);
    let motion = iou(&rest, &moved);
    println!(
        "posed: live against traced IoU {at_rest:.4} at rest and {at_one:.4} moved; rest against moved {motion:.4}; {} and {} live pixels",
        count(&rest),
        count(&moved)
    );
    assert!(count(&rest) > 1500, "{} pixels", count(&rest));
    assert!(at_rest >= 0.9, "rest IoU {at_rest:.4}");
    assert!(at_one >= 0.9, "moved IoU {at_one:.4}");
    assert!(motion < 0.85, "the movers move: {motion:.4}");

    let centred = self::scene(
        "posed-centred",
        &format!(
            "{POSED}\n[camera]\nat = [1.5, 0.6, 5.0]\nlook_at = [0.0, 0.0, 0.0]\nfov = 40.0\n"
        ),
    );
    let live_centred = mask(&Viewer::new(&centred).frames(0.0, 6), 0.5);
    let traced_centred = mask(&traced(&centred, 0.0, 16), 0.5);
    let live_shift = centroid(&rest)[0] - centroid(&live_centred)[0];
    let traced_shift = centroid(&traced_rest)[0] - centroid(&traced_centred)[0];
    println!("shift moves the picture {live_shift:.2} px live and {traced_shift:.2} px traced");
    assert!(live_shift < -8.0, "{live_shift}");
    assert!((live_shift - traced_shift).abs() < 1.5);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn scene_text_draws_live_where_the_tracer_letters_it() {
    let scene = scene(
        "text",
        "[text.sign]\ntext = \"Hello\"\nfont = \"serif.ttf\"\nsize = 0.8\nat = [0.0, 0.2, 0.0]\nrotate = [0.0, -20.0, 0.0]\n\n[camera]\nat = [0.0, 0.0, 4.0]\nlook_at = [0.0, 0.0, 0.0]\nfov = 40.0\n",
    );
    let mut viewer = Viewer::new(&scene);
    assert_eq!(viewer.staged.texts(), 1);
    let live = mask(&viewer.frames(0.0, 6), 0.5);
    let traced = mask(&traced(&scene, 0.0, 32), 0.5);
    let overlap = iou(&live, &traced);
    println!(
        "text: {} live pixels, {} traced, IoU {overlap:.4}",
        count(&live),
        count(&traced)
    );
    assert!(count(&live) > 300, "{} pixels", count(&live));
    assert!(overlap >= 0.85, "IoU {overlap:.4}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn depth_of_field_softens_live_and_traced_alike() {
    let body = |lens: &str| {
        format!(
            "[[object]]\nname = \"a\"\nmesh = \"block\"\nat = [-0.7, 0.0, 0.0]\nrotate = [20.0, 30.0, 0.0]\nmaterial = \"glow\"\n\n[[object]]\nname = \"b\"\nmesh = \"panel\"\nat = [0.8, 0.1, 0.0]\nscale = 0.8\nmaterial = \"glow\"\n\n[[object]]\nname = \"wall\"\nmesh = \"panel\"\nat = [0.0, 0.0, -3.0]\nscale = 12.0\nmaterial = \"dim\"\n\n[camera]\nat = [0.0, 0.0, 5.0]\nlook_at = [0.0, 0.0, 0.0]\nfov = 40.0\n{lens}"
        )
    };
    let edges = |pixels: &[[f32; 3]]| {
        pixels
            .iter()
            .filter(|pixel| (0.3..0.9).contains(&luminance(pixel)))
            .count()
    };
    let sharp = scene("dof-sharp", &body(""));
    let soft = scene("dof-soft", &body("fstop = 0.5\nfocus = 1.0\n"));
    let live_sharp = edges(&Viewer::new(&sharp).frames(0.0, 8));
    let live_soft = edges(&Viewer::new(&soft).frames(0.0, 8));
    let traced_sharp = edges(&traced(&sharp, 0.0, 64));
    let traced_soft = edges(&traced(&soft, 0.0, 64));
    println!(
        "depth of field edge pixels: live {live_sharp} sharp, {live_soft} soft; traced {traced_sharp} sharp, {traced_soft} soft"
    );
    assert!(
        live_soft > live_sharp * 2 + 50,
        "{live_sharp} -> {live_soft}"
    );
    assert!(
        traced_soft > traced_sharp * 2 + 50,
        "{traced_sharp} -> {traced_soft}"
    );
    let ratio = live_soft as f64 / traced_soft as f64;
    assert!(
        (0.25..4.0).contains(&ratio),
        "live {live_soft} against traced {traced_soft}"
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_soft_sun_widens_the_penumbra_live_and_traced() {
    let body = |radius: f32| {
        format!(
            "[[object]]\nname = \"floor\"\nmesh = \"block\"\nat = [0.0, -0.55, 0.0]\nscale = [8.0, 0.1, 8.0]\n\n[[object]]\nname = \"post\"\nmesh = \"block\"\nat = [0.0, 0.5, 0.0]\nscale = [0.3, 2.0, 0.3]\nshadow = \"only\"\n\n[sun]\nmodel = \"authored\"\ntoward = [0.6, 0.55, 0.3]\nirradiance = 3.0\nradius = {radius}\n\n[camera]\nat = [0.0, 4.5, 3.5]\nlook_at = [-0.8, -0.5, -0.4]\nfov = 45.0\n"
        )
    };
    let penumbra = |pixels: &[[f32; 3]]| {
        let mut lit: Vec<f32> = pixels.iter().map(luminance).collect();
        lit.sort_by(f32::total_cmp);
        let top = lit[lit.len() * 95 / 100].max(1e-6);
        pixels
            .iter()
            .filter(|pixel| {
                let value = luminance(pixel) / top;
                (0.15..0.85).contains(&value)
            })
            .count()
    };
    let hard = scene("sun-hard", &body(0.0));
    let soft = scene("sun-soft", &body(4.0));
    let mut live_soft_viewer = Viewer::new(&soft);
    assert_eq!(
        live_soft_viewer.renderer.shadows().quality().sun_radius_deg,
        4.0
    );
    let live_hard = penumbra(&Viewer::new(&hard).frames(0.0, 8));
    let live_soft = penumbra(&live_soft_viewer.frames(0.0, 8));
    let traced_hard = penumbra(&traced(&hard, 0.0, 64));
    let traced_soft = penumbra(&traced(&soft, 0.0, 64));
    println!(
        "penumbra pixels: live {live_hard} hard, {live_soft} soft; traced {traced_hard} hard, {traced_soft} soft"
    );
    assert!(
        live_soft > live_hard * 3 / 2 + 20,
        "{live_hard} -> {live_soft}"
    );
    assert!(
        traced_soft > traced_hard * 3 / 2 + 20,
        "{traced_hard} -> {traced_soft}"
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_mix_sky_brightens_live_and_traced_by_the_same_share() {
    let room = "width = 64\nfloor = [0.1, 0.1, 0.1]\nwall = [0.2, 0.25, 0.3]\nceiling = [0.4, 0.4, 0.45]\n";
    let camera = "[[object]]\nname = \"behind\"\nmesh = \"block\"\nat = [0.0, 0.0, 6.0]\nscale = 0.1\n\n[camera]\nat = [0.0, 0.0, 0.0]\nlook_at = [0.3, 0.4, -1.0]\nfov = 60.0\n";
    let sun = "[sun]\nmodel = \"authored\"\ntoward = [0.2, 0.7, -0.6]\nirradiance = 4.0\n";
    let alone = scene(
        "mix-alone",
        &format!("{sun}\n[sky]\nkind = \"room\"\n\n[sky.room]\n{room}\n{camera}"),
    );
    let mixed = scene(
        "mix",
        &format!(
            "{sun}\n[sky]\nkind = \"mix\"\n\n[[sky.layer]]\nkind = \"room\"\nweight = 1.0\n\n[sky.layer.room]\n{room}\n[[sky.layer]]\nkind = \"analytic\"\nweight = 2.0\n\n{camera}"
        ),
    );
    let live_alone = mean(&Viewer::new(&alone).frames(0.0, 4));
    let live_mixed = mean(&Viewer::new(&mixed).frames(0.0, 4));
    let traced_alone = mean(&traced(&alone, 0.0, 8));
    let traced_mixed = mean(&traced(&mixed, 0.0, 8));
    let mut shown = 0.0f64;
    for k in 0..3 {
        let live = live_mixed[k] / live_alone[k];
        let traced = traced_mixed[k] / traced_alone[k];
        println!("mix sky channel {k}: live {live:.4}, traced {traced:.4}");
        assert!(
            (live / traced - 1.0).abs() < 0.1,
            "channel {k}: {live} against {traced}"
        );
        shown = shown.max((live - 1.0).abs());
    }
    assert!(shown > 0.1, "the mix shows: {shown}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn glass_draws_through_the_effects() {
    let scene = scene(
        "glass",
        "[[object]]\nname = \"back\"\nmesh = \"panel\"\nat = [0.0, 0.0, -1.0]\nscale = 3.0\nmaterial = \"glow\"\n\n[[object]]\nname = \"pane\"\nmesh = \"block\"\nscale = [1.2, 1.2, 0.2]\nmaterial = \"glass\"\n\n[camera]\nat = [0.0, 0.0, 4.0]\nlook_at = [0.0, 0.0, 0.0]\nfov = 40.0\n",
    );
    let mut viewer = Viewer::new(&scene);
    assert_eq!(viewer.staged.glass().len(), 1);
    assert_eq!(viewer.staged.instances().len(), 1);
    assert_eq!(viewer.staged.effects().glass.len(), 1);
    let live = viewer.frames(0.0, 6);
    let centre = (HEIGHT / 2 * WIDTH + WIDTH / 2) as usize;
    let traced = traced(&scene, 0.0, 32);
    println!(
        "through the glass: live {:?}, traced {:?}",
        live[centre], traced[centre]
    );
    assert!(live.iter().flatten().all(|value| value.is_finite()));
    assert!(luminance(&live[centre]) > 0.3, "{:?}", live[centre]);
    assert!(luminance(&traced[centre]) > 0.3, "{:?}", traced[centre]);
    assert!(viewer.staged.set_override(
        "pane",
        Override {
            hidden: true,
            ..Override::default()
        }
    ));
    assert!(viewer.staged.glass().is_empty());
    viewer.staged.clear_overrides();
    assert_eq!(viewer.staged.glass().len(), 1);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn overrides_hide_recolour_and_highlight_one_object_without_touching_the_scene() {
    let scene = scene(
        "overrides",
        "[[object]]\nname = \"lamp\"\nmesh = \"block\"\nat = [-1.0, 0.0, 0.0]\nmaterial = \"glow\"\n\n[[object]]\nname = \"crate\"\nmesh = \"block\"\nat = [1.0, 0.0, 0.0]\n\n[sun]\nmodel = \"authored\"\ntoward = [0.3, 0.6, 0.8]\nirradiance = 3.0\n\n[camera]\nat = [0.0, 0.0, 5.0]\nlook_at = [0.0, 0.0, 0.0]\nfov = 40.0\n",
    );
    let mut viewer = Viewer::new(&scene);
    let materials = viewer.staged.materials().to_vec();
    let base: Vec<u32> = viewer
        .staged
        .instances()
        .iter()
        .map(|i| i.material)
        .collect();
    let plain = viewer.frames(0.0, 6);
    let half = WIDTH as usize / 2;
    let side = |pixels: &[[f32; 3]], right: bool| -> Vec<[f32; 3]> {
        pixels
            .iter()
            .enumerate()
            .filter(|(index, _)| (index % WIDTH as usize >= half) == right)
            .map(|(_, pixel)| *pixel)
            .collect()
    };
    let lamp_lit = count(&mask(&side(&plain, false), 0.5));
    assert!(lamp_lit > 500, "{lamp_lit}");

    assert!(!viewer.staged.set_override(
        "nothing",
        Override {
            hidden: true,
            ..Override::default()
        }
    ));
    assert!(viewer.staged.set_override(
        "lamp",
        Override {
            hidden: true,
            ..Override::default()
        }
    ));
    let hidden = viewer.frames(0.0, 6);
    let lamp_hidden = count(&mask(&side(&hidden, false), 0.5));
    assert!(lamp_hidden < lamp_lit / 20, "{lamp_lit} -> {lamp_hidden}");

    viewer.staged.clear_override("lamp");
    assert!(viewer.staged.set_override(
        "crate",
        Override {
            color: Some([0.05, 0.8, 0.05]),
            ..Override::default()
        }
    ));
    let green = mean(&side(&viewer.frames(0.0, 6), true));
    let grey = mean(&side(&plain, true));
    assert!(
        green[1] / green[0] > 2.0 * grey[1] / grey[0],
        "{grey:?} -> {green:?}"
    );

    assert!(viewer.staged.set_override(
        "crate",
        Override {
            highlight: Some([2.0, 0.0, 0.0]),
            ..Override::default()
        }
    ));
    let red = mean(&side(&viewer.frames(0.0, 6), true));
    assert!(red[0] > grey[0] + 0.05, "{grey:?} -> {red:?}");
    assert_eq!(viewer.staged.materials().len(), materials.len() + 1);

    viewer.staged.clear_overrides();
    assert_eq!(viewer.staged.materials(), &materials[..]);
    let after: Vec<u32> = viewer
        .staged
        .instances()
        .iter()
        .map(|i| i.material)
        .collect();
    assert_eq!(after, base);
    assert!(
        viewer
            .staged
            .instances()
            .iter()
            .all(|i| !i.shadow_only && i.casts_shadow)
    );
    assert!(viewer.staged.overrides().is_empty());
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn haze_from_the_scene_marches_live_and_goes_with_an_edit() {
    let body = |haze: &str| {
        format!(
            "[[object]]\nname = \"floor\"\nmesh = \"block\"\nat = [0.0, -0.55, 0.0]\nscale = [8.0, 0.1, 8.0]\n\n[sun]\nmodel = \"authored\"\ntoward = [0.4, 0.5, 0.3]\nirradiance = 4.0\n\n[sky]\nkind = \"room\"\n\n[sky.room]\nwidth = 64\nfloor = [0.1, 0.1, 0.1]\nwall = [0.3, 0.3, 0.3]\nceiling = [0.5, 0.5, 0.5]\n\n[camera]\nat = [0.0, 1.0, 4.0]\nlook_at = [0.0, 0.3, 0.0]\nfov = 50.0\n{haze}"
        )
    };
    let clear = scene("haze-clear", &body(""));
    let hazy = scene(
        "haze",
        &body("\n[haze]\nlo = [-4.0, -0.5, -4.0]\nhi = [4.0, 3.0, 4.0]\namount = 0.6\n"),
    );
    let mut viewer = Viewer::new(&hazy);
    let with = mean(&viewer.frames(0.0, 8));
    let without = mean(&Viewer::new(&clear).frames(0.0, 8));
    println!("haze: {with:?} against {without:?}");
    assert!((0..3).any(|k| (with[k] - without[k]).abs() > 0.01));
    let diff = hazy.diff(&clear);
    assert!(diff.haze && !diff.draws());
    let applied = viewer
        .staged
        .apply(&mut viewer.renderer, &clear, &diff)
        .unwrap();
    assert!(applied.haze);
    let back = mean(&viewer.frames(1.0, 8));
    let gone = (0..3)
        .map(|k| (back[k] - without[k]).abs())
        .fold(0.0, f64::max);
    let kept = (0..3)
        .map(|k| (with[k] - without[k]).abs())
        .fold(0.0, f64::max);
    assert!(gone < kept * 0.25, "{gone} against {kept}");
}
