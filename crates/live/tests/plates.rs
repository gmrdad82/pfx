#[path = "support/plates_desk.rs"]
mod plates_desk;

use pfx_bake::plate::codec::from_rgb9e5;
use pfx_live::demand::{Limits, Need, Next, SETTLE};
use pfx_live::plates::PlateSettings;
use pfx_live::scene::Override;
use pfx_load::scene::Scene;

use plates_desk::{Desk, Viewer, close, luminance, off};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 200;

const BEHIND: &str = "[[object]]
name = \"behind\"
mesh = \"block\"
at = [0.0, 1.2, -1.7]
scale = 0.4
material = \"red\"
dynamic = true
";

fn hide_dynamic(viewer: &mut Viewer, scene: &Scene) {
    for name in scene.dynamic_names() {
        assert!(viewer.staged.set_override(
            &name,
            Override {
                hidden: true,
                ..Override::default()
            }
        ));
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_plated_frame_is_the_plate_where_nothing_moves_and_draws_the_moving_parts_over_it() {
    let desk = Desk::new("composite", BEHIND, [WIDTH, HEIGHT]);
    desk.bake_with(&[12.0, 17.0], 0.005, 512);
    let scene = desk.scene();
    let plate = desk.plate();
    let mut viewer = Viewer::new(&scene, WIDTH, HEIGHT);
    assert!(viewer.staged.plated());
    assert_eq!(viewer.renderer.plates().unwrap().info().anchors, 2);
    let instances = viewer.staged.instances().len();
    assert_eq!(instances, 4, "only the dynamic parts are drawn live");
    hide_dynamic(&mut viewer, &scene);
    let empty = viewer.frames(0.0, 12);
    assert!(
        viewer
            .renderer
            .last_pass_order()
            .contains(&"plate composite")
    );
    let margin = [
        (plate.manifest.width - WIDTH) / 2,
        (plate.manifest.height - HEIGHT) / 2,
    ];
    let texel = |x: u32, y: u32| ((y + margin[1]) * plate.manifest.width + x + margin[0]) as usize;
    let mut checked = 0;
    let mut worst = 0.0f32;
    let corners = [
        viewer.pixel(&scene, [-0.32, 0.785, 0.2]),
        viewer.pixel(&scene, [-0.08, 0.785, 0.3]),
    ];
    let text = |x: u32, y: u32| {
        x + 3 >= corners[0][0].min(corners[1][0])
            && x <= corners[0][0].max(corners[1][0]) + 3
            && y + 3 >= corners[0][1].min(corners[1][1])
            && y <= corners[0][1].max(corners[1][1]) + 3
    };
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            if plate.depth_at(x + margin[0], y + margin[1]).is_none() || text(x, y) {
                continue;
            }
            let expected = from_rgb9e5(plate.color[0][texel(x, y)]);
            let got = empty[(y * WIDTH + x) as usize];
            worst = worst.max(off(got, expected));
            checked += 1;
        }
    }
    let mut plate_crop = Vec::new();
    let mut bad = Vec::new();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let expected = from_rgb9e5(plate.color[0][texel(x, y)]);
            plate_crop.push(expected);
            let error = off(empty[(y * WIDTH + x) as usize], expected);
            if error > 0.02 && !text(x, y) {
                bad.push((x, y, error));
            }
        }
    }
    let heat: Vec<[f32; 3]> = (0..(WIDTH * HEIGHT) as usize)
        .map(|i| {
            let e = off(empty[i], plate_crop[i]);
            [(e * 10.0).min(1.0), (e * 50.0).min(1.0) * 0.3, 0.0]
        })
        .collect();
    plates_desk::save_png(
        &plates_desk::debug_dir().join("static-error.png"),
        WIDTH,
        HEIGHT,
        &heat,
    );
    plates_desk::save_png(
        &plates_desk::debug_dir().join("static-live.png"),
        WIDTH,
        HEIGHT,
        &empty,
    );
    plates_desk::save_png(
        &plates_desk::debug_dir().join("static-plate.png"),
        WIDTH,
        HEIGHT,
        &plate_crop,
    );
    let hdr = viewer.renderer.readback_geometry().unwrap().colour;
    let mut raw = 0.0f32;
    let mut raw_mean = 0.0f32;
    for i in 0..(WIDTH * HEIGHT) as usize {
        let e = off([hdr[i][0], hdr[i][1], hdr[i][2]], plate_crop[i]);
        raw = raw.max(e);
        raw_mean += e;
    }
    println!(
        "hdr target against the plate: worst {raw}, mean {}",
        raw_mean / (WIDTH * HEIGHT) as f32
    );
    let mut errors: Vec<f32> = bad.iter().map(|b| b.2).collect();
    errors.sort_by(f32::total_cmp);
    println!(
        "{} pixels off by more than 2%: {:?}; largest {:?}",
        bad.len(),
        &bad[..bad.len().min(20)],
        &errors[errors.len().saturating_sub(10)..]
    );
    assert!(checked > (WIDTH * HEIGHT * 9 / 10) as usize, "{checked}");
    assert!(worst < 0.01, "the static frame is the plate: worst {worst}");

    viewer.renderer.set_plate_hour(14.5);
    let plain = PlateSettings {
        relight: false,
        ..PlateSettings::default()
    };
    viewer.renderer.set_plate_settings(plain);
    let between = viewer.frames(0.0, 12);
    viewer.renderer.set_plate_settings(PlateSettings::default());
    let mut worst = 0.0f32;
    for (x, y) in [(40, 150), (160, 140), (280, 60), (100, 40)] {
        let a = from_rgb9e5(plate.color[0][texel(x, y)]);
        let b = from_rgb9e5(plate.color[1][texel(x, y)]);
        let sun = plate.sun.as_ref().unwrap();
        let sa = from_rgb9e5(sun[0][texel(x, y)]);
        let sb = from_rgb9e5(sun[1][texel(x, y)]);
        let la = plate.manifest.suns[0].light();
        let lb = plate.manifest.suns[1].light();
        let now = scene.sun.unwrap();
        let got = between[(y * WIDTH + x) as usize];
        let expected = std::array::from_fn(|k| {
            let unit = |share: f32, light: f32| if light > 0.0 { share / light } else { 0.0 };
            let direct =
                (unit(sa[k], la[k]) + unit(sb[k], lb[k])) * 0.5 * now.color[k] * now.intensity;
            ((a[k] - sa[k]) + (b[k] - sb[k])) * 0.5 + direct
        });
        worst = worst.max(off(got, expected));
    }
    assert!(
        worst < 0.02,
        "between anchors the plate blends: worst {worst}"
    );
    viewer.renderer.set_plate_hour(12.0);

    viewer.staged.clear_overrides();
    let full = viewer.frames(0.0, 16);
    plates_desk::save_png(
        &plates_desk::debug_dir().join("full-live.png"),
        WIDTH,
        HEIGHT,
        &full,
    );
    let lid = viewer.pixel(&scene, [0.5, 0.85, 0.12]);
    let lid_colour = full[(lid[1] * WIDTH + lid[0]) as usize];
    assert!(
        lid_colour[0] > lid_colour[1] * 1.5,
        "the lid draws: {lid_colour:?}"
    );
    let behind = viewer.pixel(&scene, [0.0, 1.2, -1.7]);
    let wall = from_rgb9e5(plate.color[0][texel(behind[0], behind[1])]);
    let seen = full[(behind[1] * WIDTH + behind[0]) as usize];
    assert!(
        close(seen, wall, 0.02),
        "the wall hides what is behind it: {seen:?} {wall:?}"
    );
    let desk_id = scene.object("desk").unwrap().id;
    let wall_id = scene.object("wall").unwrap().id;
    let card_id = scene.object("screen card").unwrap().id;
    let lid_id = scene.object("lid").unwrap().id;
    let open_desk = viewer.pixel(&scene, [0.55, 0.78, 0.3]);
    assert_eq!(viewer.pick(open_desk), desk_id);
    assert_eq!(viewer.pick(behind), wall_id);
    assert_eq!(viewer.pick(lid), lid_id);
    let card = viewer.pixel(&scene, [0.22, 0.95, 0.05]);
    assert_eq!(viewer.pick(card), card_id);

    assert!(viewer.staged.set_override(
        "lid",
        Override {
            hidden: true,
            ..Override::default()
        }
    ));
    let without = viewer.frames(0.0, 16);
    let mut shaded = 0;
    let mut brighter = 0;
    for index in 0..(WIDTH * HEIGHT) as usize {
        let (x, y) = (index as u32 % WIDTH, index as u32 / WIDTH);
        let id = plate.id.as_ref().unwrap()[texel(x, y)];
        if u32::from(id) != desk_id {
            continue;
        }
        let a = luminance(full[index]);
        let b = luminance(without[index]);
        if a < b * 0.85 {
            let hue = |c: [f32; 3]| c[0] / c[1].max(1e-4);
            if (hue(full[index]) / hue(without[index]) - 1.0).abs() < 0.08 {
                shaded += 1;
            }
        }
        if a > b * 1.05 && full[index][0] < full[index][1] * 1.5 {
            brighter += 1;
        }
    }
    assert!(shaded > 30, "the lid shades the plate: {shaded}");
    assert!(brighter < 10, "{brighter}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_small_drift_reprojects_the_plate_by_its_depth() {
    let desk = Desk::new("drift", "", [WIDTH, HEIGHT]);
    desk.bake_with(&[12.0], 0.005, 512);
    let scene = desk.scene();
    let plate = desk.plate();
    let mut viewer = Viewer::new(&scene, WIDTH, HEIGHT);
    hide_dynamic(&mut viewer, &scene);
    let mut camera = scene.camera_or_default();
    camera.at = [
        camera.at[0] + 0.04,
        camera.at[1] - 0.025,
        camera.at[2] + 0.06,
    ];
    camera.look_at = [
        camera.look_at[0] + 0.02,
        camera.look_at[1],
        camera.look_at[2],
    ];
    let drifted = viewer.frames_from(&camera, 0.0, 12);
    let size = [plate.manifest.width, plate.manifest.height];
    let mut worst = 0.0f32;
    for (point, plane) in [
        ([0.45, 0.78, 0.28], [0.0, 1.0, 0.0, 0.78]),
        ([-0.5, 0.78, 0.25], [0.0, 1.0, 0.0, 0.78]),
        ([0.62, 0.78, -0.05], [0.0, 1.0, 0.0, 0.78]),
        ([-0.9, 1.6, -1.2], [0.0, 0.0, 1.0, -1.2]),
        ([0.9, 0.0, 0.9], [0.0, 1.0, 0.0, 0.0]),
    ] {
        let pixel = viewer.pixel_from(&camera, point);
        let got = drifted[(pixel[1] * WIDTH + pixel[0]) as usize];
        let seen = viewer.through(&camera, pixel, plane);
        let (at, _) = plate.manifest.camera.project(seen, size).unwrap();
        let expected = plates_desk::bilinear(&plate, 0, at);
        println!("{point:?} at {pixel:?}: {got:?} against {expected:?}");
        worst = worst.max(off(got, expected));
    }
    assert!(
        worst < 0.03,
        "the drifted frame samples the plate where its depth says: {worst}"
    );
}

struct OnDemand {
    renderer: pfx_live::renderer::Renderer,
    staged: pfx_live::scene::Staged,
    output: pfx_gpu::OffscreenTarget,
    previous: Option<pfx_live::frame::Matrix>,
    turns: pfx_gpu::pace::Turns,
}

impl OnDemand {
    fn new(scene: &Scene) -> Self {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let format = pfx_gpu::wgpu::TextureFormat::Rgba8UnormSrgb;
        let output = gpu.offscreen(WIDTH, HEIGHT, format).unwrap();
        let mut renderer =
            pfx_live::renderer::Renderer::new_with_output_format(gpu, WIDTH, HEIGHT, format)
                .unwrap();
        renderer
            .set_exposure(pfx_live::renderer::Exposure::Fixed(1.0))
            .unwrap();
        let staged = renderer.stage(scene).unwrap();
        renderer
            .set_frames_on_demand(Some(Limits::default()))
            .unwrap();
        Self {
            renderer,
            staged,
            output,
            previous: None,
            turns: pfx_gpu::pace::Turns::default(),
        }
    }

    fn tick(&mut self, camera: &pfx_load::scene::Camera, time: f32) -> Need {
        let aspect = WIDTH as f32 / HEIGHT as f32;
        let finish = self.staged.finish();
        let drawn = self.staged.frame(aspect, time, 7);
        let mut scene = drawn.scene;
        let mut view = pfx_live::scene::camera(camera, aspect);
        let now = pfx_live::frame::multiply(view.projection, view.view);
        view.previous_view_projection = self.previous.unwrap_or(now);
        self.previous = Some(now);
        scene.camera = view;
        let next = Next::new(&scene, &drawn.text, &drawn.effects, finish);
        let need = self.renderer.frame_needed(&next);
        let renderer = &mut self.renderer;
        let output = &self.output;
        self.turns
            .time(|| renderer.render_needed(&next, &need, &output.view).unwrap());
        need
    }

    fn settle(&mut self, camera: &pfx_load::scene::Camera, time: f32) -> u32 {
        let mut drawn = 0;
        while self.tick(camera, time) != Need::None {
            drawn += 1;
            assert!(drawn < 64, "the plated desk never settles");
        }
        drawn
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_plated_scene_on_demand_idles_still_reprojects_a_drift_and_redraws_moving_parts() {
    let desk = Desk::new("demand", "", [WIDTH, HEIGHT]);
    let path = desk.path();
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        text.replace("face_camera = true", "face_camera = false")
            .replace("lit = true\ndynamic = true", "lit = true"),
    )
    .unwrap();
    desk.bake(&[12.0, 17.0], 64);
    let scene = desk.scene();
    let mut view = OnDemand::new(&scene);
    assert!(view.staged.plated());
    let rest = scene.camera_or_default();
    let drawn = view.settle(&rest, 0.0);
    assert!(drawn <= 1 + SETTLE + 1, "{drawn}");
    assert!(view.renderer.last_pass_order().contains(&"plate composite"));
    let shown = view.renderer.gpu().readback_rgba8(&view.output).unwrap();
    let submitted = view.renderer.frames_submitted();
    for _ in 0..30 {
        assert_eq!(
            view.tick(&rest, 0.0),
            Need::None,
            "a still plated desk is idle"
        );
    }
    assert_eq!(view.renderer.frames_submitted(), submitted);
    let output = view.output.view.clone();
    view.renderer.present_last(&output).unwrap();
    assert_eq!(
        view.renderer.gpu().readback_rgba8(&view.output).unwrap(),
        shown
    );

    view.renderer.set_plate_hour(14.5);
    assert_eq!(
        view.tick(&rest, 0.0),
        Need::Full,
        "a new hour redraws the plate"
    );
    view.settle(&rest, 0.0);
    view.renderer.set_plate_hour(14.5);
    assert_eq!(
        view.tick(&rest, 0.0),
        Need::None,
        "the same hour changes nothing"
    );
    view.renderer.set_plate_settings(PlateSettings::default());
    assert_eq!(
        view.tick(&rest, 0.0),
        Need::None,
        "the same settings change nothing"
    );

    let mut drift = rest;
    drift.at = [rest.at[0] + 0.004, rest.at[1], rest.at[2]];
    assert_eq!(
        view.tick(&drift, 0.0),
        Need::Reproject,
        "a drift reprojects"
    );
    let order = view.renderer.last_pass_order();
    assert!(
        order.contains(&"reproject") && !order.contains(&"plate composite"),
        "{order:?}"
    );
    view.settle(&drift, 0.0);

    let before = view.renderer.gpu().readback_rgba8(&view.output).unwrap();
    let mut full = 0;
    for frame in 1..=6 {
        if view.tick(&drift, frame as f32 * 0.25) == Need::Full {
            full += 1;
        }
        assert!(view.renderer.last_pass_order().contains(&"plate composite"));
    }
    assert_eq!(full, 6, "moving parts redraw over the composite every tick");
    let after = view.renderer.gpu().readback_rgba8(&view.output).unwrap();
    let changed = before
        .chunks_exact(4)
        .zip(after.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(changed > 200, "the moving parts moved: {changed}");
}
