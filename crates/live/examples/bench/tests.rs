use super::*;
use pfx_materials::{Family, NoiseKind};

#[test]
fn the_room_carries_every_family_and_detail_kind() {
    let room = room::room();
    let used: Vec<&Material> = room
        .parts
        .iter()
        .map(|part| &room.materials[part.material].1)
        .collect();
    let glass = room::glass_library();
    for family in [
        Family::Plain,
        Family::Plaster,
        Family::Stone,
        Family::Cloth,
        Family::Metal,
        Family::Occluder,
        Family::Petal,
        Family::Leaf,
        Family::Bark,
        Family::Cord,
        Family::Wood,
        Family::Lacquer,
        Family::Emissive,
        Family::Chrome,
    ] {
        assert!(
            used.iter().any(|m| m.family == family),
            "no part is {family:?}"
        );
    }
    for family in [Family::Glass, Family::Liquid] {
        assert!(glass.iter().any(|(_, m)| m.family == family));
    }
    for kind in [
        NoiseKind::PlankWood,
        NoiseKind::Grime,
        NoiseKind::Scratch,
        NoiseKind::Fibre,
        NoiseKind::Crinkle,
        NoiseKind::CoatWobble,
        NoiseKind::WallMottle,
        NoiseKind::Fbm,
        NoiseKind::Value,
        NoiseKind::Leaf,
        NoiseKind::Bark,
    ] {
        assert!(
            used.iter()
                .any(|m| m.layers.iter().any(|layer| layer.kind == kind)),
            "no part carries {kind:?}"
        );
    }
    assert!(
        used.iter()
            .any(|m| m.normal.source == pfx_materials::NormalSource::Bump)
    );
}

#[test]
fn the_room_has_enough_instanced_static_parts() {
    let room = room::room();
    let instanced = room
        .parts
        .iter()
        .filter(|part| part.mesh <= room::CAN)
        .count();
    assert!(instanced >= 48, "{instanced} parts on the shared meshes");
    assert!(room.parts.iter().any(|part| part.cast == Cast::Only));
    assert!(room.parts.iter().any(|part| part.cast == Cast::Never));
    assert!(room.parts.iter().any(|part| part.cutout));
    assert!(room.parts.iter().any(|part| part.age > 0.0));
    assert!(room.parts.iter().any(|part| part.age == 0.0));
    assert!(room.meshes.iter().any(|(_, mesh)| mesh.uvs1.is_some()));
    let names: std::collections::BTreeSet<&str> =
        room.parts.iter().map(|part| part.name.as_str()).collect();
    assert_eq!(names.len(), room.parts.len(), "part names are unique");
}

#[test]
fn the_room_is_the_same_on_every_build() {
    let a = room::room();
    let b = room::room();
    assert_eq!(a.parts.len(), b.parts.len());
    for (x, y) in a.parts.iter().zip(&b.parts) {
        assert_eq!(x.model, y.model);
        assert_eq!(x.material, y.material);
    }
    let (triangles, owners) = triangles(&a);
    assert_eq!(triangles.len(), owners.len());
    assert!(triangles.len() > 10_000);
}

#[test]
fn the_slats_let_the_afternoon_sun_onto_the_table() {
    let sun = Lighting::load().sun(HOUR).direction;
    assert!(sun[0] < -0.5 && sun[1] > 0.4, "{sun:?}");
    let down = sun.map(|v| -v);
    let top = room::WINDOW_Y[1];
    let along = (top - room::TABLE_TOP) / -down[1];
    let x = room::LEFT + down[0] * along;
    let [cx, _] = room::TABLE_CENTRE;
    let [hx, _] = room::TABLE_HALF;
    assert!(x > cx - hx && x < cx + hx, "the beam lands at x = {x}");
    assert!(room::LENS[0] < x, "the lens sits in the beam");
}

#[test]
fn the_caustic_focuses_light_inside_the_lens_shadow() {
    let (gain, rect) = paint::caustic(Lighting::load().sun(HOUR).direction);
    let side = paint::CAUSTIC as usize;
    assert_eq!(gain.len(), side * side);
    assert!(rect[2] > 0.0 && rect[3] > 0.0);
    let peak = gain.iter().copied().fold(f32::MIN, f32::max);
    let low = gain.iter().copied().fold(f32::MAX, f32::min);
    assert!(peak > 2.0, "peak gain {peak}");
    assert!(low < -0.5, "shadow gain {low}");
    let mean = gain.iter().sum::<f32>() / gain.len() as f32;
    assert!(mean.abs() < 0.05, "energy is kept, mean {mean}");
}

#[test]
fn every_finish_has_a_name() {
    assert_eq!(finish_named("standard"), Some(Finish::Standard));
    for style in Style::ALL {
        let name = format!("{style:?}").to_lowercase();
        assert_eq!(finish_named(&name), Some(Finish::Style(style)));
    }
    assert_eq!(finish_named("nothing"), None);
}

#[test]
fn the_director_moves_the_camera_a_little() {
    let lens = Lens::default();
    let moving = lens.camera(300, false, 1280, 800);
    let still = lens.camera(300, true, 1280, 800);
    assert_ne!(moving.view, still.view);
    let drift = sub(moving.position, still.position);
    assert!(dot(drift, drift).sqrt() < 0.2);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn three_frames_render_non_black_with_finite_timings() {
    let mut bench = build(&Options::small(640, 360));
    for frame in 0..3 {
        let times = bench.render(frame, false, Shown::Moving);
        assert!(!times.is_empty());
        assert!(times.iter().all(|time| time.milliseconds.is_finite()));
    }
    let order = bench.renderer.last_pass_order();
    println!("pass order: {}", order.join(" -> "));
    for pass in [
        "shadows",
        "prepass",
        "opaque",
        "sky",
        "glass",
        "volume march",
        "particles",
        "creatures",
        "surface text",
        "TAA",
        "post",
        "overlay text",
    ] {
        assert!(order.contains(&pass), "{pass} is missing from {order:?}");
    }
    assert!(bench.renderer.history_valid());
    let pixels = bench.renderer.gpu().readback_rgba16(&bench.output).unwrap();
    assert!(pixels.chunks_exact(4).any(|pixel| {
        pixel[..3]
            .iter()
            .any(|channel| half::f16::from_bits(*channel).to_f32() > 0.02)
    }));
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn history_resets_on_a_cut_and_a_daylight_jump() {
    let mut bench = build(&Options::small(320, 180));
    bench.render(0, false, Shown::Rest);
    assert!(!bench.renderer.history_valid());
    bench.render(1, false, Shown::Rest);
    assert!(bench.renderer.history_valid());
    bench.render(40, false, Shown::Rest);
    assert!(!bench.renderer.history_valid());
    bench.render(41, false, Shown::Rest);
    assert!(bench.renderer.history_valid());
    bench.set_hour(19.0);
    bench.render(42, false, Shown::Rest);
    assert!(!bench.renderer.history_valid());
    bench.render(43, false, Shown::Rest);
    assert!(bench.renderer.history_valid());
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn resize_recreates_targets_and_history() {
    let mut bench = build(&Options::small(320, 180));
    bench.render(0, false, Shown::Rest);
    bench.render(1, false, Shown::Rest);
    bench.resize(640, 360);
    assert_eq!(bench.renderer.size(), (640, 360));
    assert!(!bench.render(2, false, Shown::Rest).is_empty());
    assert!(!bench.renderer.history_valid());
    assert_eq!(
        bench
            .renderer
            .gpu()
            .readback_rgba16(&bench.output)
            .unwrap()
            .len(),
        640 * 360 * 4
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn srgb_surface_output() {
    let mut options = Options::small(640, 360);
    options.format = wgpu::TextureFormat::Rgba8UnormSrgb;
    options.effects = false;
    options.content = false;
    let mut bench = build(&options);
    bench.words.shown = false;
    let times = bench.render(0, false, Shown::Rest);
    assert!(times.iter().any(|time| time.label == "output blit"));
    assert_eq!(
        bench.renderer.last_pass_order().last(),
        Some(&"output blit")
    );
    let bytes = bench.renderer.gpu().readback_rgba8(&bench.output).unwrap();
    assert!(
        bytes
            .chunks_exact(4)
            .any(|pixel| pixel[..3].iter().any(|channel| *channel > 5))
    );
}

fn files(folder: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let name = path.strip_prefix(folder).unwrap().display().to_string();
                out.insert(name, std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_room_bakes_byte_identical_twice() {
    let scene = bake_scene(&Lighting::load());
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let (_, spec) = volumes()
        .into_iter()
        .find(|(name, _)| *name == "cavity")
        .unwrap();
    let reflection = reflections().remove(0);
    let hash = scene_hash(&scene, spec, SAMPLES, SEED).unwrap();
    let tmp = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp");
    std::fs::create_dir_all(tmp.join("bench")).unwrap();
    let tmp = tmp.canonicalize().unwrap();
    let mut runs = Vec::new();
    for run in 0..2 {
        let relative = PathBuf::from(format!("bench/determinism-{run}"));
        let folder = tmp.join(&relative);
        if folder.exists() {
            std::fs::remove_dir_all(&folder).unwrap();
        }
        let grids = bake(&gpu, &scene, spec, SAMPLES, SEED).unwrap();
        write_artifact(
            &tmp,
            &relative.join("probes"),
            &scene,
            &grids,
            SAMPLES,
            SEED,
        )
        .unwrap();
        let cube = bake_reflection(&gpu, &scene, &reflection, SAMPLES, SEED, 1).unwrap();
        write_reflection_artifact(ReflectionWrite {
            tmp_root: &tmp,
            relative: &relative.join("reflection"),
            scene_hash: hash.clone(),
            spec: &reflection,
            anchors: &scene.anchors[1..2],
            cubes: &[cube],
            samples: SAMPLES,
            seed: SEED,
        })
        .unwrap();
        runs.push(files(&folder));
    }
    assert!(
        runs[0].len() >= 4,
        "{:?}",
        runs[0].keys().collect::<Vec<_>>()
    );
    assert_eq!(
        runs[0].keys().collect::<Vec<_>>(),
        runs[1].keys().collect::<Vec<_>>()
    );
    for (name, bytes) in &runs[0] {
        assert!(bytes == &runs[1][name], "{name} differs between two bakes");
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_tank_and_the_rope_run_twice_alike() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let first = physics::run(&gpu, 120);
    let second = physics::run(&gpu, 120);
    assert!(first.state.live > 0);
    assert_eq!(first.state, second.state);
    assert!(first.height == second.height);
    assert_eq!(first.wires, second.wires);
}
