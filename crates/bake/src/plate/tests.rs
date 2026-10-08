use super::*;

const RECIPE: &str = "scene = \"room.glb\"
anchors = [8.0, 12.0]
samples = 16

[plates]
scene = \"desk.scene.toml\"
size = [640, 400]
overscan = 0.1
anchors = [12.0, 16.0, 19.0]
layers = [\"depth\", \"color\", \"id\"]
key = \"manf:abc\"

[[plates.view]]
name = \"desk\"
at = [0.0, 1.0, 3.0]
fov = 30.0
";

fn camera() -> SceneCamera {
    SceneCamera {
        at: [0.3, 1.2, 3.0],
        look_at: [0.0, 0.6, 0.0],
        shift: [0.1, -0.2],
        projection: SceneProjection::Perspective { fov: 35.0 },
        ..SceneCamera::default()
    }
}

#[test]
fn a_recipe_reads_its_plates_with_defaults_and_views() {
    assert_eq!(parse_plates(b"scene = \"a.glb\"\n").unwrap(), None);
    let recipe = parse_plates(RECIPE.as_bytes()).unwrap().unwrap();
    assert_eq!(recipe.scene, "desk.scene.toml");
    assert_eq!(recipe.size, [640, 400]);
    assert_eq!(recipe.scale, 1.0);
    assert_eq!(recipe.anchors.as_deref(), Some(&[12.0, 16.0, 19.0][..]));
    assert_eq!(
        recipe.layers,
        Layers {
            normal: false,
            id: true,
            sun: false,
            transfer: false
        }
    );
    assert_eq!(recipe.layers.names(), ["color", "depth", "id"]);
    assert_eq!(recipe.adaptive.threshold, 0.01);
    assert_eq!(recipe.adaptive.max_samples, 1024);
    assert_eq!(recipe.key.as_deref(), Some("manf:abc"));
    assert_eq!(recipe.views.len(), 1);
    assert_eq!(recipe.views[0].name, "desk");
    assert_eq!(recipe.views[0].keys.fov, Some(30.0));
    let plain = parse_plates(b"[plates]\nscene = \"s.scene.toml\"\nsize = [64, 64]\n")
        .unwrap()
        .unwrap();
    assert_eq!(plain.layers, Layers::ALL);
    assert_eq!(plain.views[0].name, DEFAULT_VIEW);
    assert_eq!(plain.overscan, 0.1);
    assert_eq!((plain.clamp_indirect, plain.filter_glossy), (None, None));
}

#[test]
fn the_plates_firefly_keys_override_the_scenes_trace_settings_one_by_one() {
    let base = "[plates]\nscene = \"s.scene.toml\"\nsize = [64, 64]\n";
    let scene = TraceSettings {
        transmissive_shadows: true,
        clamp_indirect: 4.0,
        filter_glossy: 0.1,
    };
    let plain = parse_plates(base.as_bytes()).unwrap().unwrap();
    assert_eq!(plain.trace(scene), scene);
    let both =
        parse_plates(format!("{base}clamp_indirect = 20\nfilter_glossy = 0.25\n").as_bytes())
            .unwrap()
            .unwrap();
    assert_eq!(
        both.trace(scene),
        TraceSettings {
            transmissive_shadows: true,
            clamp_indirect: 20.0,
            filter_glossy: 0.25,
        }
    );
    let off = parse_plates(format!("{base}filter_glossy = 0.0\n").as_bytes())
        .unwrap()
        .unwrap();
    assert_eq!(
        off.trace(scene),
        TraceSettings {
            filter_glossy: 0.0,
            ..scene
        }
    );
}

#[test]
fn a_recipe_refuses_bad_plates_by_name() {
    let base = "[plates]\nscene = \"s.scene.toml\"\nsize = [64, 64]\n";
    let cases = [
        ("size = [8, 64]", "16 to 16384"),
        ("overscan = 1.5", "overscan must be 0 to 1"),
        ("scale = 9.0", "scale must be 0.25 to 4"),
        ("anchors = [12.0, 9.0]", "strictly increasing"),
        ("layers = [\"color\"]", "need color and depth"),
        (
            "layers = [\"color\", \"depth\", \"albedo\"]",
            "albedo is unknown",
        ),
        ("layers = [\"color\", \"depth\", \"depth\"]", "named twice"),
        (
            "layers = [\"color\", \"depth\", \"sun\", \"transfer\"]",
            "needs sun and normal",
        ),
        ("max_samples = 1", "2 to 65536 samples"),
        (
            "clamp_indirect = -1.0",
            "plates clamp_indirect = -1 is outside 0 to 10000",
        ),
        ("clamp_indirect = 1e9", "clamp_indirect"),
        (
            "filter_glossy = 2.0",
            "plates filter_glossy = 2 is outside 0 to 1",
        ),
        ("filter_glossy = nan", "filter_glossy"),
        ("colour = true", "unknown field"),
        ("[[plates.view]]\nname = \"a b\"", "ASCII label"),
        (
            "[[plates.view]]\nname = \"a\"\n[[plates.view]]\nname = \"a\"",
            "named twice",
        ),
        (
            "[[plates.view]]\nname = \"a\"\nfov = 30.0\nfocal = 50.0",
            "not both",
        ),
    ];
    for (extra, message) in cases {
        let text = if extra.starts_with("size") {
            format!("[plates]\nscene = \"s.scene.toml\"\n{extra}\n")
        } else {
            format!("{base}{extra}\n")
        };
        let error = parse_plates(text.as_bytes()).unwrap_err();
        assert!(error.contains(message), "{extra}: {error}");
    }
}

#[test]
fn a_view_takes_the_scene_camera_and_overrides_its_keys() {
    let base = camera();
    let same = ViewKeys::default().camera(&base).unwrap();
    assert_eq!(same, base);
    let focal = ViewKeys {
        focal: Some(50.0),
        at: Some([0.0, 2.0, 4.0]),
        ..ViewKeys::default()
    }
    .camera(&base)
    .unwrap();
    assert_eq!(focal.at, [0.0, 2.0, 4.0]);
    let SceneProjection::Perspective { fov } = focal.projection else {
        panic!("perspective");
    };
    assert!((fov - 26.991).abs() < 0.01, "{fov}");
    let ortho = SceneCamera {
        projection: SceneProjection::Orthographic { height: 2.0 },
        ..base
    };
    assert!(
        ViewKeys::default()
            .camera(&ortho)
            .unwrap_err()
            .contains("orthographic")
    );
}

#[test]
fn the_plate_camera_widens_the_view_by_its_overscan_and_keeps_its_centre() {
    let scene = camera();
    let size = [640, 400];
    assert_eq!(plate_size(size, 0.1, 1.0), [704, 440]);
    assert_eq!(plate_size(size, 0.0, 1.25), [800, 500]);
    let plate = PlateCamera::new(&scene, size, 0.1, 1.0);
    let wide = plate_size(size, 0.1, 1.0);
    let projection = scene.projection(size[0] as f32 / size[1] as f32);
    let view = scene.view();
    for world in [[0.0, 0.6, 0.0], [0.4, 0.2, -0.5], [-0.7, 1.1, 0.4]] {
        let (texel, depth) = plate.project(world, wide).unwrap();
        let eye: [f32; 4] = std::array::from_fn(|r| {
            (0..3).map(|c| view[c][r] * world[c]).sum::<f32>() + view[3][r]
        });
        assert!((depth + eye[2]).abs() < 1e-4, "{depth} {eye:?}");
        let clip: [f32; 4] =
            std::array::from_fn(|r| (0..4).map(|c| projection[c][r] * eye[c]).sum::<f32>());
        let ndc = [clip[0] / clip[3], clip[1] / clip[3]];
        let live = [
            (ndc[0] + 1.0) * 0.5 * size[0] as f32,
            (1.0 - ndc[1]) * 0.5 * size[1] as f32,
        ];
        let margin = [
            (wide[0] - size[0]) as f32 * 0.5,
            (wide[1] - size[1]) as f32 * 0.5,
        ];
        assert!(
            (texel[0] - margin[0] - live[0]).abs() < 2e-3,
            "{texel:?} {live:?}"
        );
        assert!(
            (texel[1] - margin[1] - live[1]).abs() < 2e-3,
            "{texel:?} {live:?}"
        );
        let ray = plate.ray(texel, wide);
        let to: [f32; 3] = std::array::from_fn(|k| world[k] - plate.origin[k]);
        let cross = [
            ray[1] * to[2] - ray[2] * to[1],
            ray[2] * to[0] - ray[0] * to[2],
            ray[0] * to[1] - ray[1] * to[0],
        ];
        let sin = (dot(cross, cross) / (dot(ray, ray) * dot(to, to))).sqrt();
        assert!(sin < 1e-5 && dot(ray, to) > 0.0, "{sin}");
    }
}

#[test]
fn a_strip_camera_traces_the_rows_of_the_whole_plate() {
    let plate = PlateCamera::new(&camera(), [640, 400], 0.1, 1.0);
    let size = plate_size([640, 400], 0.1, 1.0);
    let (strip, shift) = plate.strip(size, 96, 64);
    for (x, y) in [(0.5f32, 0.5f32), (300.5, 31.5), (703.5, 63.5)] {
        let ndc = [x / size[0] as f32 * 2.0 - 1.0, y / 64.0 * 2.0 - 1.0];
        let ray: [f32; 3] = std::array::from_fn(|k| {
            strip.forward[k]
                + strip.right[k] * (ndc[0] + shift[0])
                + strip.up[k] * (-ndc[1] + shift[1])
        });
        let whole = plate.ray([x, y + 96.0], size);
        for k in 0..3 {
            assert!((ray[k] - whole[k]).abs() < 1e-5, "{ray:?} {whole:?}");
        }
    }
}

#[test]
fn anchors_blend_between_the_two_nearest_and_hold_past_the_ends() {
    let anchors = [12.0, 16.0, 19.0];
    assert_eq!(anchor_blend(&anchors, 10.0), (0, 0, 0.0));
    assert_eq!(anchor_blend(&anchors, 12.0), (0, 0, 0.0));
    assert_eq!(anchor_blend(&anchors, 14.0), (0, 1, 0.5));
    assert_eq!(anchor_blend(&anchors, 16.0), (1, 2, 0.0));
    assert_eq!(anchor_blend(&anchors, 18.25), (1, 2, 0.75));
    assert_eq!(anchor_blend(&anchors, 21.0), (2, 2, 0.0));
    assert_eq!(anchor_blend(&anchors, f32::NAN), (0, 0, 0.0));
    assert_eq!(anchor_blend(&[], 3.0), (0, 0, 0.0));
}

fn sample_plate() -> Plate {
    let texels = 6;
    Plate {
        manifest: PlateManifest {
            schema_version: SCHEMA_VERSION,
            format: FORMAT.into(),
            engine: "test".into(),
            key: "k".into(),
            caller_key: None,
            view: "main".into(),
            size: [2, 2],
            overscan: 0.5,
            scale: 1.0,
            width: 3,
            height: 2,
            camera: PlateCamera::new(&camera(), [2, 2], 0.5, 1.0),
            depth_near: 0.5,
            bounds: Bounds {
                min: [0.0; 3],
                max: [1.0; 3],
            },
            anchors: vec![12.0, 16.0],
            suns: vec![
                SunRecord {
                    direction: [0.0, 1.0, 0.0],
                    color: [1.0; 3],
                    intensity: 2.0,
                };
                2
            ],
            layers: BTreeMap::new(),
            sampling: Sampling {
                threshold: 0.01,
                min_samples: 16,
                max_samples: 64,
                growth: 2.0,
                seed: 7,
            },
            samples: Vec::new(),
            ids: vec![IdEntry {
                id: 3,
                object: "desk".into(),
            }],
            files: Vec::new(),
        },
        depth: Depth::Inverse(vec![0, 1, 2, 3, 65535, 9]),
        normal: Some(vec![1; texels]),
        id: Some(vec![0, 3, 3, 3, 0, 3]),
        color: vec![
            vec![codec::rgb9e5([1.0, 0.5, 0.25]); texels],
            vec![7; texels],
        ],
        sun: Some(vec![vec![5; texels], vec![6; texels]]),
        transfer: Some(vec![vec![8; texels], vec![9; texels]]),
    }
}

fn folder(name: &str) -> std::path::PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/plate-tests")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn a_plate_writes_and_reads_back_from_a_folder_and_from_bytes() {
    let plate = sample_plate();
    let dir = folder("round-trip");
    let manifest = write_plate(&dir, &plate).unwrap();
    assert_eq!(
        manifest
            .files
            .iter()
            .map(|file| file.name.as_str())
            .collect::<Vec<_>>(),
        [
            "depth.bin",
            "normal.bin",
            "id.bin",
            "color-000.bin",
            "color-001.bin",
            "sun-000.bin",
            "sun-001.bin",
            "transfer-000.bin",
            "transfer-001.bin"
        ]
    );
    assert_eq!(manifest.layers["depth"], "r16-inverse");
    assert_eq!(manifest.layers["normal"], "oct16");
    let back = read_plate(&dir).unwrap();
    assert_eq!(back.depth, plate.depth);
    assert_eq!(back.color, plate.color);
    assert_eq!(back.sun, plate.sun);
    assert_eq!(back.id, plate.id);
    assert_eq!(back.manifest.files, manifest.files);
    assert_eq!(back.transfer, plate.transfer);
    assert_eq!(back.bytes_per_anchor(), 72);
    assert_eq!(back.shared_bytes(), 48);
    assert_eq!(back.depth_at(0, 0), None);
    assert_eq!(back.depth_at(1, 1), Some(0.5));
    let from_bytes = read_plate_from(&|name| std::fs::read(dir.join(name)).ok()).unwrap();
    assert_eq!(from_bytes, back);
    let missing = read_plate_from(&|name| {
        (name != "sun-001.bin")
            .then(|| std::fs::read(dir.join(name)).ok())
            .flatten()
    })
    .unwrap_err();
    assert_eq!(missing, "missing plate file sun-001.bin");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_changed_byte_or_manifest_is_refused() {
    let plate = sample_plate();
    let dir = folder("tamper");
    write_plate(&dir, &plate).unwrap();
    let mut bytes = std::fs::read(dir.join("color-001.bin")).unwrap();
    bytes[0] ^= 1;
    std::fs::write(dir.join("color-001.bin"), bytes).unwrap();
    assert!(
        read_plate(&dir)
            .unwrap_err()
            .contains("color-001.bin does not match")
    );
    write_plate(&dir, &plate).unwrap();
    let json = std::fs::read_to_string(dir.join("manifest.json")).unwrap();
    std::fs::write(dir.join("manifest.json"), json.replace("\"k\"", "\"j\"")).unwrap();
    assert!(read_plate(&dir).unwrap_err().contains("manifest.sha256"));
    let mut wrong = plate.clone();
    wrong.color.pop();
    assert!(
        write_plate(&dir, &wrong)
            .unwrap_err()
            .contains("do not match")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_float_depth_plate_reads_its_depth_as_written() {
    let mut plate = sample_plate();
    plate.depth = Depth::Float(vec![0.0, 1.0, 2.0, 3.0, 400.0, 5.0]);
    let dir = folder("float");
    let manifest = write_plate(&dir, &plate).unwrap();
    assert_eq!(manifest.layers["depth"], "r32f");
    let back = read_plate(&dir).unwrap();
    assert_eq!(back.depth_at(1, 1), Some(400.0));
    assert_eq!(back.depth_at(0, 0), None);
    assert_eq!(back.shared_bytes(), 60);
    let _ = std::fs::remove_dir_all(&dir);
}
