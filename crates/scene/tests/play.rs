mod common;

use std::path::Path;

use common::{Folder, PROJECT, ROOM, errors, show};
use pfx_scene::types::{BodyShape, Bounds, CharacterKind, Plane, RigKind};
use pfx_scene::{Diagnostic, File, Id, Kind, SceneEdit, Target, check, code};

const LAYERS: &str = "\n[layers]\nworld = [\"world\", \"player\", \"enemy\"]\nplayer = [\"world\", \"enemy\", \"pickup\"]\nenemy = [\"world\", \"player\"]\npickup = [\"player\"]\n";

const HERO: &str = "\n[[object]]\nid = \"her0000001\"\nname = \"hero\"\nmesh = \"block\"\nat = [0.0, 0.9, 0.0]\n\n[object.character]\nkind = \"2d\"\nradius = 0.3\nheight = 1.8\nmax_climb = 50.0\ncoyote = 8\nlayer = \"player\"\nplane = \"xy\"\nvariable_jump = 0.5\nwall_slide = 2.0\nwall_jump = [4.0, 6.0]\n";

const ZONE: &str = "\n[[object]]\nid = \"z0ne000001\"\nname = \"coin zone\"\nmesh = \"block\"\nhidden = true\nat = [2.0, 0.5, 0.0]\n\n[object.trigger]\nshape = \"sphere\"\nradius = 0.5\nlayer = \"pickup\"\nmask = [\"player\"]\n";

const BOX: &str = "\n[[object]]\nid = \"b0x0000001\"\nname = \"box\"\nmesh = \"block\"\nat = [-2.0, 0.5, 0.0]\n\n[object.body]\nlayer = \"world\"\n";

const RIGS: &str = "\n[[rig]]\nid = \"r1g0000001\"\nname = \"side\"\nkind = \"follow\"\ntarget = \"hero\"\noffset = [0.0, 1.0, 10.0]\ndead_zone = [0.5, 0.25]\nlook_ahead = 1.5\ndamping = [0.2, 0.3, 0.0]\nbounds = { min = [-20.0, 0.0, 10.0], max = [20.0, 8.0, 10.0] }\northographic = 9.0\nsnap = 16.0\n\n[[rig]]\nid = \"eyes000001\"\nname = \"eyes\"\nkind = \"first-person\"\ntarget = \"her0000001\"\noffset = [0.0, 0.7, 0.0]\nsensitivity = 0.8\ninvert = true\nsmoothing = 0.05\npitch = [-80.0, 80.0]\nhead_bob = 0.04\n\n[[rig]]\nid = \"0rb1t00001\"\nname = \"orbit\"\nkind = \"third-person\"\ntarget = \"hero\"\noffset = [0.4, 0.6, 0.0]\ndistance = 3.5\ncollide = true\npitch = [-30.0, 60.0]\n";

fn played(name: &str, layers: &str, extra: &str) -> Folder {
    let folder = Folder::room(name);
    folder.write("project.toml", &format!("{PROJECT}{layers}"));
    folder.write(
        "room.scene.toml",
        &format!("{ROOM}{HERO}{ZONE}{BOX}{RIGS}{extra}"),
    );
    folder
}

fn found(diagnostics: &[Diagnostic]) -> Vec<(&'static str, &str, &str)> {
    diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d| (d.code, d.file.to_str().unwrap(), d.key.as_str()))
        .collect()
}

fn line_of(folder: &Folder, file: &str, needle: &str) -> u32 {
    let text = folder.read(file);
    let at = text.find(needle).unwrap();
    text[..at].matches('\n').count() as u32 + 1
}

fn one(diagnostics: &[Diagnostic]) -> &Diagnostic {
    let found = errors(diagnostics);
    assert_eq!(found.len(), 1, "{}", show(diagnostics));
    found[0]
}

#[test]
fn characters_triggers_rigs_and_layers_read_resolve_and_check_clean() {
    let folder = played("play-clean", LAYERS, "");
    let diagnostics = folder.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    let project = folder.project();
    let scene = project
        .scene(&folder.path("room.scene.toml"))
        .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)));

    let hero = &scene.object("hero").unwrap().value;
    let character = hero.character.as_ref().unwrap();
    assert_eq!(character.kind, Some(CharacterKind::TwoD));
    assert_eq!(character.plane, Some(Plane::Xy));
    assert_eq!(character.wall_jump, Some([4.0, 6.0]));
    assert_eq!(character.coyote, Some(8));
    assert_eq!(character.jump_buffer, None);
    assert_eq!(character.layer.as_deref(), Some("player"));
    assert_eq!(hero.dynamic, Some(true));

    let zone = &scene.object("coin zone").unwrap().value;
    let trigger = zone.trigger.as_ref().unwrap();
    assert_eq!(trigger.shape, Some(BodyShape::Sphere));
    assert_eq!(trigger.mask, Some(vec!["player".to_string()]));
    assert_eq!(zone.dynamic, Some(false));
    assert_eq!(
        scene
            .object("box")
            .unwrap()
            .value
            .body
            .as_ref()
            .unwrap()
            .layer,
        Some("world".to_string())
    );

    let rigs: Vec<(&str, Option<&str>, RigKind, Option<&str>)> = scene
        .rigs
        .iter()
        .map(|entry| {
            (
                entry.key.as_str(),
                entry.id.as_deref(),
                entry.value.kind,
                entry.value.target.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        rigs,
        [
            ("side", Some("r1g0000001"), RigKind::Follow, Some("hero")),
            (
                "eyes",
                Some("eyes000001"),
                RigKind::FirstPerson,
                Some("hero")
            ),
            (
                "orbit",
                Some("0rb1t00001"),
                RigKind::ThirdPerson,
                Some("hero")
            ),
        ]
    );
    let side = &scene.rig("side").unwrap().value;
    assert_eq!(
        side.bounds,
        Some(Bounds {
            min: [-20.0, 0.0, 10.0],
            max: [20.0, 8.0, 10.0],
        })
    );
    assert_eq!(side.snap, Some(16.0));
    assert_eq!(scene.rig("eyes").unwrap().value.invert, Some(true));

    let layers: Vec<(&str, usize)> = scene
        .layers
        .iter()
        .map(|(name, row)| (name.as_str(), row.len()))
        .collect();
    assert_eq!(
        layers,
        [("enemy", 2), ("pickup", 1), ("player", 3), ("world", 3)]
    );

    let Ok(File::Project(file)) = project.read(Path::new("project.toml")) else {
        panic!("the project file reads");
    };
    assert_eq!(file.layers, scene.layers);
    let Ok(File::Scene(file)) = project.read(&folder.path("room.scene.toml")) else {
        panic!("the room reads");
    };
    assert_eq!(file.rig[1].target.as_deref(), Some("her0000001"));
    assert_eq!(file.object[5].dynamic, None);
    let text = toml::to_string(&file).unwrap();
    assert_eq!(toml::from_str::<pfx_scene::SceneFile>(&text).unwrap(), file);
}

#[test]
fn a_layer_name_is_refused_where_it_is_written_unless_the_project_names_it() {
    let folder = played(
        "play-unknown-layer",
        LAYERS,
        "\n[[object]]\nid = \"crab000001\"\nname = \"crab\"\nmesh = \"block\"\n\n[object.trigger]\nlayer = \"enemy\"\nmask = [\"player\", \"ghost\"]\n\n[object.character]\nlayer = \"enemies\"\n",
    );
    let diagnostics = folder.check();
    assert_eq!(
        found(&diagnostics),
        [
            (
                code::BAD_REFERENCE,
                "room.scene.toml",
                "object.crab000001.trigger.mask.1"
            ),
            (
                code::BAD_REFERENCE,
                "room.scene.toml",
                "object.crab000001.character.layer"
            ),
        ],
        "{}",
        show(&diagnostics)
    );
    let error = errors(&diagnostics)[1];
    assert_eq!(
        error.line,
        line_of(&folder, "room.scene.toml", "layer = \"enemies\"")
    );
    assert_eq!(
        error.message,
        "enemies names no layer of the project; [layers] in project.toml names enemy, pickup, player, world"
    );

    let bare = played("play-no-layers", "", "");
    let diagnostics = bare.check();
    assert_eq!(
        found(&diagnostics),
        [
            (
                code::BAD_REFERENCE,
                "room.scene.toml",
                "object.her0000001.character.layer"
            ),
            (
                code::BAD_REFERENCE,
                "room.scene.toml",
                "object.z0ne000001.trigger.layer"
            ),
            (
                code::BAD_REFERENCE,
                "room.scene.toml",
                "object.z0ne000001.trigger.mask.0"
            ),
            (
                code::BAD_REFERENCE,
                "room.scene.toml",
                "object.b0x0000001.body.layer"
            ),
        ],
        "{}",
        show(&diagnostics)
    );
    assert!(
        errors(&diagnostics)[3]
            .message
            .contains("the project names no layers"),
        "{}",
        show(&diagnostics)
    );
    assert!(bare.project().scene(&bare.path("room.scene.toml")).is_err());

    let folder = played("play-unsaved-layer", LAYERS, "");
    let text = folder
        .read("room.scene.toml")
        .replace("layer = \"world\"", "layer = \"wrold\"");
    let diagnostics = check(&text, Path::new("room.scene.toml"), &folder.project());
    let error = one(&diagnostics);
    assert_eq!(error.key, "object.b0x0000001.body.layer");
    let text = folder
        .read("project.toml")
        .replace("\n[layers]", "\n[layers]\nghost = []");
    let mut overlay = std::collections::BTreeMap::new();
    overlay.insert(folder.path("project.toml"), text);
    overlay.insert(
        folder.path("room.scene.toml"),
        folder
            .read("room.scene.toml")
            .replace("layer = \"world\"", "layer = \"ghost\""),
    );
    let diagnostics = folder.project().check_with(overlay);
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
}

#[test]
fn a_broken_layers_table_is_reported_once_in_the_project_file() {
    let folder = played("play-broken-layers", "\n[layers]\nworld = \"player\"\n", "");
    let diagnostics = folder.check();
    let error = one(&diagnostics);
    assert_eq!(
        (error.code, error.file.to_str().unwrap()),
        (code::BAD_TYPE, "project.toml")
    );
    assert!(
        folder
            .project()
            .scene(&folder.path("room.scene.toml"))
            .is_ok()
    );
}

#[test]
fn the_layers_table_is_symmetric_named_and_holds_at_most_32_layers() {
    let cases = [
        (
            "[layers]\na = [\"b\"]\nb = []\n",
            code::BAD_VALUE,
            "layers.a.0",
            "layer a meets b, and b does not list a; each pair is listed on both sides",
        ),
        (
            "[layers]\na = [\"a\", \"c\"]\n",
            code::BAD_REFERENCE,
            "layers.a.1",
            "c names no layer of [layers]",
        ),
        (
            "[layers]\na = [\"a\", \"a\"]\n",
            code::BAD_VALUE,
            "layers.a.1",
            "layer a lists a twice",
        ),
        (
            "[layers]\n\" a\" = []\n",
            code::BAD_NAME,
            "layers. a",
            "a layer name is empty or starts or ends with a space",
        ),
    ];
    for (place, (layers, code, key, message)) in cases.into_iter().enumerate() {
        let folder = Folder::room(&format!("play-layers-{place}"));
        folder.write("project.toml", &format!("{PROJECT}\n{layers}"));
        let diagnostics = folder.check();
        let error = one(&diagnostics);
        assert_eq!(
            (error.code, error.key.as_str(), error.message.as_str()),
            (code, key, message),
            "{error}"
        );
    }

    let many: String = (0..33).map(|n| format!("l{n:02} = []\n")).collect();
    let folder = Folder::room("play-layers-many");
    folder.write("project.toml", &format!("{PROJECT}\n[layers]\n{many}"));
    let diagnostics = folder.check();
    let error = one(&diagnostics);
    assert_eq!(error.key, "layers.l32");
    assert_eq!(error.message, "the project names 33 layers; at most 32");
    assert_eq!(error.line, line_of(&folder, "project.toml", "l32 = []"));
}

#[test]
fn a_character_takes_the_keys_of_its_kind_with_their_ranges() {
    let cases = [
        (
            "plane = \"xy\"\n",
            "character.plane",
            "plane belongs to a 2d character",
        ),
        (
            "radius = 0.5\nheight = 0.8\n",
            "character.height",
            "less than twice its radius",
        ),
        (
            "max_climb = 90.0\n",
            "character.max_climb",
            "is outside 0 to 90",
        ),
        (
            "max_step = 2.0\nheight = 1.8\n",
            "character.max_step",
            "is not below its height",
        ),
        ("snap = -0.1\n", "character.snap", "is below 0"),
        (
            "coyote = 2000\n",
            "character.coyote",
            "outside 0 to 1000 ticks",
        ),
        (
            "kind = \"2d\"\nvariable_jump = 1.5\n",
            "character.variable_jump",
            "outside 0 to 1",
        ),
        (
            "kind = \"2d\"\nwall_jump = [-1.0, 2.0]\n",
            "character.wall_jump",
            "two speeds",
        ),
        ("radius = 0.0\n", "character.radius", "is not above 0"),
    ];
    for (place, (keys, key, message)) in cases.into_iter().enumerate() {
        let folder = played(
            &format!("play-character-{place}"),
            LAYERS,
            &format!(
                "\n[[object]]\nid = \"crab000001\"\nname = \"crab\"\nmesh = \"block\"\n\n[object.character]\n{keys}"
            ),
        );
        let diagnostics = folder.check();
        let error = one(&diagnostics);
        assert_eq!(
            (error.code, error.key.as_str()),
            (code::BAD_VALUE, format!("object.crab000001.{key}").as_str()),
            "{error}"
        );
        assert!(error.message.contains(message), "{error}");
    }

    let refused = [
        (
            "[object.character]\nkind = \"4d\"\n",
            code::BAD_VALUE,
            "object.crab000001.character.kind",
        ),
        (
            "[object.character]\nspeed = 3.0\n",
            code::UNKNOWN_KEY,
            "object.crab000001.character.speed",
        ),
        (
            "[object.character]\n\n[object.body]\n",
            code::BAD_VALUE,
            "object.crab000001.character",
        ),
        (
            "[object.character]\nface_camera = true\n",
            code::UNKNOWN_KEY,
            "object.crab000001.character.face_camera",
        ),
    ];
    for (place, (tables, code, key)) in refused.into_iter().enumerate() {
        let folder = played(
            &format!("play-character-refused-{place}"),
            LAYERS,
            &format!(
                "\n[[object]]\nid = \"crab000001\"\nname = \"crab\"\nmesh = \"block\"\n\n{tables}"
            ),
        );
        let diagnostics = folder.check();
        let error = one(&diagnostics);
        assert_eq!((error.code, error.key.as_str()), (code, key), "{error}");
    }

    let folder = played(
        "play-character-static",
        LAYERS,
        "\n[[object]]\nid = \"crab000001\"\nname = \"crab\"\nmesh = \"block\"\ndynamic = false\nface_camera = true\n\n[object.character]\n",
    );
    let diagnostics = folder.check();
    let keys: Vec<&str> = errors(&diagnostics)
        .iter()
        .map(|d| d.key.as_str())
        .collect();
    assert_eq!(
        keys,
        ["object.crab000001.dynamic", "object.crab000001.character"],
        "{}",
        show(&diagnostics)
    );
}

#[test]
fn a_character_sits_on_a_root_object_or_under_placements_and_no_mover_moves_it() {
    let folder = played(
        "play-character-parent",
        LAYERS,
        "\n[[object]]\nid = \"crab000001\"\nname = \"crab\"\nmesh = \"block\"\nparent = \"floor\"\n\n[object.character]\n",
    );
    let diagnostics = folder.check();
    let error = one(&diagnostics);
    assert_eq!(error.key, "object.crab000001.character");
    assert!(
        error.message.contains(
            "crab has a character under object floor, which places no prefab; a character sits on a root object or under placements"
        ),
        "{error}"
    );

    let folder = played(
        "play-character-moved",
        LAYERS,
        "\n[[mover]]\nid = \"m0ver00009\"\nname = \"lift\"\nobjects = [\"hero\"]\nkind = \"slide\"\naxis = [0.0, 1.0, 0.0]\ntravel = [0.0, 1.0]\n",
    );
    let diagnostics = folder.check();
    let error = one(&diagnostics);
    assert_eq!(error.key, "object.her0000001.character");
    assert!(
        error
            .message
            .contains("hero is moved by mover lift; a character or a mover, not both"),
        "{error}"
    );
}

#[test]
fn a_trigger_takes_a_shape_with_its_own_sizes() {
    let cases = [
        (
            "radius = 0.5\n",
            code::BAD_VALUE,
            "object.crab000001.trigger.shape",
        ),
        (
            "shape = \"sphere\"\nradius = -1.0\n",
            code::BAD_VALUE,
            "object.crab000001.trigger",
        ),
        (
            "shape = \"capsule\"\nradius = 0.2\nhalf_height = 0.5\noffset = [0.0, 0.5, 0.0]\n",
            "",
            "",
        ),
        (
            "shape = \"cone\"\n",
            code::BAD_VALUE,
            "object.crab000001.trigger.shape",
        ),
        (
            "mask = \"player\"\n",
            code::BAD_TYPE,
            "object.crab000001.trigger.mask",
        ),
    ];
    for (place, (keys, code, key)) in cases.into_iter().enumerate() {
        let folder = played(
            &format!("play-trigger-{place}"),
            LAYERS,
            &format!(
                "\n[[object]]\nid = \"crab000001\"\nname = \"crab\"\nmesh = \"block\"\n\n[object.trigger]\n{keys}"
            ),
        );
        let diagnostics = folder.check();
        if code.is_empty() {
            assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
            continue;
        }
        let error = one(&diagnostics);
        assert_eq!((error.code, error.key.as_str()), (code, key), "{error}");
    }

    let folder = played(
        "play-trigger-placement",
        LAYERS,
        "\n[[object]]\nid = \"1amp000009\"\nname = \"lamp\"\nprefab = \"props/lamp.prefab.toml\"\n\n[object.trigger]\nshape = \"sphere\"\n\n[object.character]\n",
    );
    folder.write("props/lamp.prefab.toml", common::LAMP);
    let diagnostics = folder.check();
    let found: Vec<(&str, &str)> = errors(&diagnostics)
        .iter()
        .map(|d| (d.code, d.key.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            (code::PLACEMENT_KEY, "object.1amp000009.trigger"),
            (code::PLACEMENT_KEY, "object.1amp000009.character"),
        ],
        "{}",
        show(&diagnostics)
    );
}

#[test]
fn a_rig_takes_the_keys_of_its_kind_and_names_an_object() {
    let cases = [
        (
            "kind = \"follow\"\nsensitivity = 1.0\n",
            code::BAD_VALUE,
            "sensitivity",
            "a follow rig takes target, offset, dead_zone",
        ),
        (
            "kind = \"first-person\"\ndistance = 3.0\n",
            code::BAD_VALUE,
            "distance",
            "a first-person rig takes",
        ),
        (
            "kind = \"third-person\"\nhead_bob = 0.1\n",
            code::BAD_VALUE,
            "head_bob",
            "a third-person rig takes",
        ),
        (
            "kind = \"first-person\"\npitch = [-100.0, 10.0]\n",
            code::BAD_VALUE,
            "pitch",
            "each from -90 to 90",
        ),
        (
            "kind = \"first-person\"\npitch = [10.0, -10.0]\n",
            code::BAD_VALUE,
            "pitch",
            "min at most max",
        ),
        (
            "kind = \"follow\"\nsnap = 16.0\n",
            code::BAD_VALUE,
            "snap",
            "needs orthographic",
        ),
        (
            "kind = \"follow\"\nbounds = { min = [0.0, 5.0, 0.0], max = [1.0, 1.0, 1.0] }\n",
            code::BAD_VALUE,
            "bounds",
            "min is above max in y",
        ),
        (
            "kind = \"follow\"\ndamping = [0.1, -0.1, 0.0]\n",
            code::BAD_VALUE,
            "damping",
            "at least 0",
        ),
        (
            "kind = \"third-person\"\ndistance = 0.0\n",
            code::BAD_VALUE,
            "distance",
            "is not above 0",
        ),
        (
            "kind = \"follow\"\ntarget = \"nobody\"\n",
            code::BAD_REFERENCE,
            "target",
            "nobody names no object of this scene",
        ),
        (
            "kind = \"orbit\"\n",
            code::BAD_VALUE,
            "kind",
            "unknown variant",
        ),
        (
            "target = \"hero\"\n",
            code::MISSING_KEY,
            "",
            "missing key 'kind'",
        ),
    ];
    for (place, (keys, code, key, message)) in cases.into_iter().enumerate() {
        let folder = played(
            &format!("play-rig-{place}"),
            LAYERS,
            &format!("\n[[rig]]\nid = \"cam0000001\"\nname = \"cam\"\n{keys}"),
        );
        let diagnostics = folder.check();
        let error = one(&diagnostics);
        let wanted = if key.is_empty() {
            "rig.cam0000001".to_string()
        } else {
            format!("rig.cam0000001.{key}")
        };
        assert_eq!(
            (error.code, error.key.as_str()),
            (code, wanted.as_str()),
            "{error}"
        );
        assert!(error.message.contains(message), "{error}");
    }

    let folder = played(
        "play-rig-twice",
        LAYERS,
        "\n[[rig]]\nname = \"side\"\nkind = \"follow\"\n",
    );
    let diagnostics = folder.check();
    let error = one(&diagnostics);
    assert_eq!(
        (error.code, error.key.as_str()),
        (code::DUPLICATE_NAME, "rig.3.name")
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == code::MISSING_ID && d.key == "rig.3"),
        "{}",
        show(&diagnostics)
    );
}

const PLAYER: &str = "format = 1\n\n# a player and its camera\n[mesh.figure]\nid = \"p1ayermesh\"\nfile = \"ball.gltf\"\n\n[[object]]\nid = \"p1ayer0001\"\nname = \"body\"\nmesh = \"figure\"\n\n[object.character]\nradius = 0.3\nheight = 1.8\nlayer = \"player\"\n\n[[rig]]\nid = \"p1ayercam1\"\nname = \"cam\"\nkind = \"third-person\"\ntarget = \"body\"\ndistance = 3.0\n";

#[test]
fn a_rig_in_a_prefab_is_placed_with_its_target_and_overrides_are_checked() {
    let placed = "\n[[object]]\nid = \"p1ayer000a\"\nname = \"one\"\nprefab = \"props/player.prefab.toml\"\n\n[[object]]\nid = \"p1ayer000b\"\nname = \"two\"\nprefab = \"props/player.prefab.toml\"\nat = [3.0, 0.0, 0.0]\n\n[object.set]\n\"cam.distance\" = 5.0\n\"body.character.layer\" = \"enemy\"\n";
    let folder = played("play-rig-prefab", LAYERS, placed);
    folder.write("props/player.prefab.toml", PLAYER);
    let diagnostics = folder.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    let scene = folder
        .project()
        .scene(&folder.path("room.scene.toml"))
        .unwrap();
    let rigs: Vec<String> = scene
        .rigs
        .iter()
        .map(|entry| {
            format!(
                "{} {} {} {}",
                entry.key,
                entry.id.as_deref().unwrap_or("-"),
                entry.value.target.as_deref().unwrap_or("-"),
                entry.value.distance.unwrap_or(0.0)
            )
        })
        .collect();
    assert_eq!(
        &rigs[3..],
        [
            "one/cam p1ayer000a/p1ayercam1 one/body 3",
            "two/cam p1ayer000b/p1ayercam1 two/body 5",
        ]
    );
    let layer = |key: &str| {
        scene
            .object(key)
            .unwrap()
            .value
            .character
            .as_ref()
            .unwrap()
            .layer
            .clone()
    };
    assert_eq!(layer("one/body").as_deref(), Some("player"));
    assert_eq!(layer("two/body").as_deref(), Some("enemy"));

    let cases = [
        (
            "\"cam.dead_zone\" = [1.0, 1.0]\n",
            "a third-person rig takes",
        ),
        (
            "\"body.character.layer\" = \"ghost\"\n",
            "ghost names no layer of the project",
        ),
    ];
    for (place, (set, message)) in cases.into_iter().enumerate() {
        let placed = format!(
            "\n[[object]]\nid = \"p1ayer000a\"\nname = \"one\"\nprefab = \"props/player.prefab.toml\"\n\n[object.set]\n{set}"
        );
        let folder = played(&format!("play-rig-prefab-{place}"), LAYERS, &placed);
        folder.write("props/player.prefab.toml", PLAYER);
        let diagnostics = folder.check();
        let error = one(&diagnostics);
        assert_eq!(error.code, code::BAD_OVERRIDE, "{error}");
        assert_eq!(error.file.to_str(), Some("room.scene.toml"));
        assert!(error.message.contains(message), "{error}");
    }
}

#[test]
fn the_writer_adds_and_sets_a_rig_and_a_rename_follows_its_target() {
    let folder = played("play-edit", LAYERS, "");
    let mut edit = SceneEdit::open(folder.path("room.scene.toml")).unwrap();
    edit.add(
        Kind::Rig,
        Id::from_bits(0x5eed),
        "top",
        &[
            ("kind", "follow".into()),
            ("target", "hero".into()),
            ("orthographic", 12.0f32.into()),
        ],
        None,
    )
    .unwrap();
    edit.set(&Target::Rig("top".into()), &["look_ahead"], 2.0f32)
        .unwrap();
    edit.set(
        &Target::Object("hero".into()),
        &["character", "jump_speed"],
        7.5f32,
    )
    .unwrap();
    edit.rename_object("hero", "player").unwrap();
    let text = folder.read("room.scene.toml");
    assert!(
        text.ends_with(&format!(
            "[[rig]]\nid = \"{}\"\nname = \"top\"\nkind = \"follow\"\ntarget = \"player\"\northographic = 12.0\nlook_ahead = 2.0\n",
            Id::from_bits(0x5eed)
        )),
        "{text}"
    );
    assert!(
        text.contains("wall_jump = [4.0, 6.0]\njump_speed = 7.5\n"),
        "{text}"
    );
    let scene = edit.scene();
    assert_eq!(
        scene.rig("top").unwrap().value.target.as_deref(),
        Some("player")
    );
    assert_eq!(
        scene.rig("side").unwrap().value.target.as_deref(),
        Some("player")
    );

    let error = edit
        .set(
            &Target::Rig("top".into()),
            &["pitch"],
            vec![0.0f32.into(), 10.0f32.into()],
        )
        .unwrap_err();
    assert_eq!(error.code, code::BAD_VALUE);
    assert!(error.message.contains("a follow rig takes"), "{error}");
    let error = edit
        .set(
            &Target::Object("player".into()),
            &["character", "layer"],
            "ghost",
        )
        .unwrap_err();
    assert_eq!(error.code, code::BAD_REFERENCE);
    assert_eq!(folder.read("room.scene.toml"), text);
}
