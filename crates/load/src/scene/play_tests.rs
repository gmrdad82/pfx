use std::path::{Path, PathBuf};

use super::*;

struct Folder {
    root: PathBuf,
}

impl Folder {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/scene-play-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scenes");
        for entry in std::fs::read_dir(fixtures).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
        }
        std::fs::write(root.join("tone.wav"), wav()).unwrap();
        std::fs::write(root.join("noise.bin"), b"not a wave file").unwrap();
        Self { root }
    }

    fn open(&self, extra: &str) -> Result<Scene, SceneError> {
        let path = self.root.join("room.scene.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, with_bodies(&text, extra)).unwrap();
        Scene::open(&path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn with_bodies(text: &str, extra: &str) -> String {
    let mut text = text.to_string();
    let mut bodies = Vec::new();
    for section in extra.split("\n[").filter(|section| !section.is_empty()) {
        let section = format!("[{section}");
        let found = ["body", "trigger"].into_iter().find_map(|table| {
            section
                .strip_prefix(&format!("[{table}."))
                .map(|header| (table, header.to_string()))
        });
        match found {
            Some((table, header)) => {
                let (name, keys) = header
                    .split_once("]\n")
                    .unwrap_or((header.trim_end_matches(']'), ""));
                bodies.push((table, name.trim_matches('"').to_string(), keys.to_string()));
            }
            None => {
                text.push('\n');
                text.push_str(&section);
            }
        }
    }
    for (table, name, keys) in bodies {
        let start = text
            .find(&format!("name = \"{name}\"\n"))
            .unwrap_or_else(|| panic!("no object {name}"));
        let end = text[start..]
            .find("\n\n")
            .map_or(text.len(), |at| start + at + 1);
        let keys = if keys.ends_with('\n') || keys.is_empty() {
            keys
        } else {
            format!("{keys}\n")
        };
        let lead = if text[..end].ends_with('\n') {
            ""
        } else {
            "\n"
        };
        text.insert_str(end, &format!("{lead}\n[object.{table}]\n{keys}"));
    }
    text
}

fn wav() -> Vec<u8> {
    let samples: Vec<i16> = (0..64).map(|i| (i * 300) as i16).collect();
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&8000u32.to_le_bytes());
    out.extend_from_slice(&16000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

fn refused(name: &str, extra: &str) -> String {
    Folder::new(name)
        .open(extra)
        .expect_err("the scene is refused")
        .to_string()
}

#[test]
fn a_scene_without_play_keys_has_no_bodies_sounds_or_physics() {
    let folder = Folder::new("none");
    let scene = folder.open("").unwrap();
    assert!(scene.bodies.is_empty());
    assert!(scene.sounds.is_empty());
    assert_eq!(scene.physics, None);
    assert_eq!(scene.physics_or_default(), Physics::default());
}

#[test]
fn a_body_takes_its_box_from_the_mesh_bounds_and_the_object_scale() {
    let folder = Folder::new("bounds");
    let scene = folder
        .open("\n[body.crate]\n\n[body.floor]\nkind = \"fixed\"\n\n[body.\"left pillar\"]\nshape = \"capsule\"\n")
        .unwrap();
    let body = scene.bodies["crate"];
    assert_eq!(body.kind, BodyKind::Dynamic);
    let BodyShape::Box { half } = body.shape else {
        panic!("a box");
    };
    for value in half {
        assert!((value - 0.35).abs() < 1e-5, "{half:?}");
    }
    assert!(body.offset.iter().all(|value| value.abs() < 1e-5));
    assert_eq!(body.friction, 0.5);
    assert_eq!(body.density, 1.0);
    assert_eq!(scene.bodies["floor"].kind, BodyKind::Fixed);
    let BodyShape::Box { half } = scene.bodies["floor"].shape else {
        panic!("a box");
    };
    assert!((half[0] - 3.0).abs() < 1e-5 && (half[1] - 0.05).abs() < 1e-5);
    let pillar = scene.bodies["left pillar"];
    let BodyShape::Capsule {
        half_height,
        radius,
    } = pillar.shape
    else {
        panic!("a capsule");
    };
    assert!(half_height > 0.0 && radius > 0.0);
    assert!(pillar.offset[1] > 0.0, "{:?}", pillar.offset);
}

#[test]
fn physics_and_sounds_load_with_their_defaults() {
    let folder = Folder::new("sounds");
    let scene = folder
        .open(concat!(
            "\n[physics]\ngravity = [0.0, -4.0, 0.0]\nrate = 120.0\n",
            "\n[body.crate]\nrestitution = 0.4\nvelocity = [0.0, 2.0, 0.0]\n",
            "\n[sound.hum]\nfile = \"tone.wav\"\nloop = true\nvolume = 0.5\n",
            "\n[sound.knock]\nfile = \"tone.wav\"\nplay = \"hit\"\nobject = \"crate\"\n",
            "\n[sound.chime]\nfile = \"tone.wav\"\nplay = \"game\"\npan = -0.5\n",
        ))
        .unwrap();
    let physics = scene.physics.unwrap();
    assert_eq!(physics.gravity, [0.0, -4.0, 0.0]);
    assert_eq!(physics.rate, 120.0);
    assert_eq!(physics.substeps, 1);
    assert_eq!(scene.bodies["crate"].restitution, 0.4);
    let hum = &scene.sounds["hum"];
    assert!(hum.looping);
    assert_eq!(hum.cue, Cue::Start);
    assert_eq!(hum.volume, 0.5);
    assert_eq!(scene.sounds["knock"].cue, Cue::Hit);
    assert_eq!(scene.sounds["knock"].object.as_deref(), Some("crate"));
    assert_eq!(scene.sounds["chime"].cue, Cue::Game);
    assert_eq!(*scene.sounds["chime"].bytes, wav());
    assert!(scene.files.keys().any(|path| path.ends_with("tone.wav")));
}

#[test]
fn an_ogg_vorbis_sound_loads_and_a_damaged_one_is_refused() {
    let folder = Folder::new("ogg");
    let ogg = std::fs::read(folder.root.join("tone.ogg")).unwrap();
    let scene = folder
        .open("\n[sound.hum]\nfile = \"tone.ogg\"\nloop = true\n")
        .unwrap();
    assert_eq!(*scene.sounds["hum"].bytes, ogg);
    assert!(scene.files.keys().any(|path| path.ends_with("tone.ogg")));
    let mut no_stream = ogg.clone();
    no_stream[28 + 1..28 + 7].copy_from_slice(b"theora");
    let mut no_duration = ogg.clone();
    let mut at = 0;
    while at < no_duration.len() {
        no_duration[at + 6..at + 14].copy_from_slice(&0i64.to_le_bytes());
        let segments = usize::from(no_duration[at + 26]);
        let body: usize = no_duration[at + 27..at + 27 + segments]
            .iter()
            .map(|len| usize::from(*len))
            .sum();
        at += 27 + segments + body;
    }
    let cases = [
        (
            "truncated",
            ogg[..ogg.len() - 10].to_vec(),
            "is a truncated Ogg file",
        ),
        ("no vorbis", no_stream, "no Vorbis stream"),
        ("no duration", no_duration, "no duration"),
    ];
    for (name, bytes, expected) in cases {
        let folder = Folder::new(&format!("ogg {name}"));
        std::fs::write(folder.root.join("bad.ogg"), bytes).unwrap();
        let error = folder
            .open("\n[sound.hum]\nfile = \"bad.ogg\"\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{name}: {error}");
        assert!(error.contains("room.scene.toml:"), "{name}: {error}");
    }
}

#[test]
fn unknown_play_keys_are_refused_by_name() {
    let body = refused("body key", "\n[body.crate]\nbounce = 1.0\n");
    assert!(body.contains("unknown key 'bounce'"), "{body}");
    assert!(body.contains("'restitution'"), "{body}");
    let sound = refused(
        "sound key",
        "\n[sound.hum]\nfile = \"tone.wav\"\nrepeat = true\n",
    );
    assert!(sound.contains("unknown key 'repeat'"), "{sound}");
    assert!(sound.contains("'loop'"), "{sound}");
    let physics = refused("physics key", "\n[physics]\nspeed = 2.0\n");
    assert!(physics.contains("unknown key 'speed'"), "{physics}");
}

#[test]
fn bad_bodies_and_sounds_are_refused_with_their_line() {
    let cases = [
        (
            "mover",
            "\n[[mover]]\nname = \"spin\"\nobjects = [\"crate\"]\nkind = \"slide\"\naxis = [1.0, 0.0, 0.0]\ntravel = [0.0, 1.0]\n\n[body.crate]\n",
            "a body or a mover, not both",
        ),
        (
            "shape",
            "\n[body.crate]\nshape = \"cone\"\n",
            "unknown variant 'cone'",
        ),
        (
            "foreign size",
            "\n[body.crate]\nshape = \"sphere\"\nhalf = [1.0, 1.0, 1.0]\n",
            "a sphere body takes",
        ),
        (
            "kind",
            "\n[body.crate]\nkind = \"floating\"\n",
            "unknown variant 'floating'",
        ),
        (
            "density",
            "\n[body.crate]\ndensity = 0.0\n",
            "a positive density",
        ),
        (
            "not wav",
            "\n[sound.hum]\nfile = \"noise.bin\"\n",
            "is not a WAV or an Ogg Vorbis file",
        ),
        (
            "hit without body",
            "\n[sound.knock]\nfile = \"tone.wav\"\nplay = \"hit\"\nobject = \"wall\"\n",
            "which has no body",
        ),
        (
            "hit without object",
            "\n[sound.knock]\nfile = \"tone.wav\"\nplay = \"hit\"\n",
            "names no object",
        ),
        (
            "cue",
            "\n[sound.knock]\nfile = \"tone.wav\"\nplay = \"later\"\n",
            "unknown variant 'later'",
        ),
        (
            "two loops",
            "\n[sound.a]\nfile = \"tone.wav\"\nloop = true\n\n[sound.b]\nfile = \"tone.wav\"\nloop = true\n",
            "one looping sound at most",
        ),
        ("rate", "\n[physics]\nrate = 0.0\n", "a rate of 1 to 1000"),
    ];
    for (name, extra, expected) in cases {
        let error = refused(name, extra);
        assert!(error.contains(expected), "{name}: {error}");
        assert!(error.contains("room.scene.toml:"), "{name}: {error}");
    }
}

#[test]
fn a_second_physics_table_is_refused_naming_the_first_file() {
    let folder = Folder::new("two physics");
    std::fs::write(
        folder.root.join("lights.scene.toml"),
        std::fs::read_to_string(folder.root.join("lights.scene.toml")).unwrap()
            + "\n[physics]\nrate = 30.0\n",
    )
    .unwrap();
    let error = folder
        .open("\n[physics]\nrate = 60.0\n")
        .unwrap_err()
        .to_string();
    assert!(error.contains("[physics] is set again"), "{error}");
    assert!(error.contains("lights.scene.toml"), "{error}");
}

#[test]
fn the_diff_names_changed_bodies_sounds_and_physics() {
    let folder = Folder::new("diff");
    let older = folder
        .open("\n[body.crate]\n\n[sound.hum]\nfile = \"tone.wav\"\n")
        .unwrap();
    let mut newer = older.clone();
    newer.bodies.get_mut("crate").unwrap().friction = 0.9;
    newer.sounds.get_mut("hum").unwrap().volume = 0.2;
    newer.physics = Some(Physics {
        rate: 30.0,
        ..Physics::default()
    });
    let diff = older.diff(&newer);
    assert_eq!(diff.bodies.changed, ["crate"]);
    assert_eq!(diff.sounds.changed, ["hum"]);
    assert!(diff.physics);
    assert!(!diff.draws());
    assert!(!diff.is_empty());
    assert!(older.diff(&older.clone()).is_empty());
}

#[test]
fn triggers_body_layers_and_project_layers_come_through_resolved() {
    let folder = Folder::new("layers");
    std::fs::write(
        folder.root.join("project.toml"),
        "[project]\n\n[layers]\nworld = [\"world\", \"player\"]\nplayer = [\"world\"]\nvolume = []\n",
    )
    .unwrap();
    let scene = folder
        .open("\n[body.crate]\nlayer = \"player\"\n\n[body.floor]\nkind = \"fixed\"\nlayer = \"world\"\n\n[trigger.\"left pillar\"]\nshape = \"sphere\"\nlayer = \"volume\"\nmask = [\"player\"]\n\n[trigger.\"right pillar\"]\nshape = \"capsule\"\n")
        .unwrap();
    assert_eq!(scene.body_layers["crate"], "player");
    assert_eq!(scene.body_layers["floor"], "world");
    assert_eq!(scene.body_layers.len(), 2);
    assert_eq!(scene.layers["world"], vec!["world", "player"]);
    assert_eq!(scene.layers.len(), 3);
    let left = &scene.triggers["left pillar"];
    assert_eq!(left.layer.as_deref(), Some("volume"));
    assert_eq!(left.mask, Some(vec!["player".to_string()]));
    let BodyShape::Sphere { radius } = left.shape else {
        panic!("{:?}", left.shape);
    };
    assert!(radius > 0.0);
    let right = &scene.triggers["right pillar"];
    assert_eq!(right.layer, None);
    assert_eq!(right.mask, None);
    assert!(matches!(right.shape, BodyShape::Capsule { .. }));
}

#[test]
fn a_trigger_with_a_layer_the_project_does_not_name_is_refused() {
    let message = refused(
        "bad trigger layer",
        "\n[trigger.crate]\nlayer = \"nowhere\"\n",
    );
    assert!(message.contains("nowhere"), "{message}");
}
