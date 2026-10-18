mod common;

use common::{Folder, ROOM, errors, show};
use pfx_scene::types::{Repeat, SpriteClip};
use pfx_scene::{Diagnostic, SceneEdit, Target, code};

const WALKER: &str = "\n[[object]]\nid = \"wa1ker0001\"\nname = \"walker\"\nmesh = \"block\"\n\n[object.animation]\nclip = \"Walk\"\nspeed = 1.5\nloop = \"ping-pong\"\nstart = 0.25\nevents = [{ name = \"step\", time = 0.4 }, { name = \"land\", clip = \"Jump\", time = 0.9 }]\n";

const MIXER: &str = "\n[[object]]\nid = \"m1xer00001\"\nname = \"mixer\"\nmesh = \"block\"\n\n[object.animation]\nblend = [{ clip = \"Walk\", weight = 0.7 }, { clip = \"Run\", weight = 0.3 }]\nevents = [{ name = \"step\", clip = \"Run\", time = 0.2 }]\n";

const FLAME: &str = "\n[[object]]\nid = \"f1ame00001\"\nname = \"flame\"\nmesh = \"panel\"\nface_camera = true\n\n[object.animation]\natlas = \"flame.png\"\ngrid = [4, 2]\nfps = 16.0\nevents = [{ name = \"flare\", frame = 7 }]\n";

const HERO: &str = "\n[[object]]\nid = \"her0000001\"\nname = \"hero\"\nmesh = \"panel\"\n\n[object.animation]\natlas = \"hero.png\"\nframes = [[0, 0, 32, 32], [32, 0, 32, 32], [64, 0, 32, 32], [0, 32, 32, 32], [32, 32, 32, 32]]\nclip = \"idle\"\nevents = [{ name = \"step\", clip = \"run\", frame = 1 }]\n\n[object.animation.clips.idle]\nfrom = 0\nto = 2\n\n[object.animation.clips.run]\nfrom = 3\nto = 4\nfps = 20.0\nloop = \"once\"\n";

fn animated(name: &str, extra: &str) -> Folder {
    let folder = Folder::room(name);
    std::fs::copy(folder.path("screen.png"), folder.path("flame.png")).unwrap();
    std::fs::copy(folder.path("screen.png"), folder.path("hero.png")).unwrap();
    folder.write(
        "room.scene.toml",
        &format!("{ROOM}{WALKER}{MIXER}{FLAME}{HERO}{extra}"),
    );
    folder
}

fn one(diagnostics: &[Diagnostic]) -> &Diagnostic {
    let found = errors(diagnostics);
    assert_eq!(found.len(), 1, "{}", show(diagnostics));
    found[0]
}

#[test]
fn animations_read_resolve_and_check_clean() {
    let folder = animated("animation-clean", "");
    let diagnostics = folder.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    let scene = folder
        .project()
        .scene(&folder.path("room.scene.toml"))
        .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)));

    let walker = &scene.object("walker").unwrap().value;
    let animation = walker.animation.as_ref().unwrap();
    assert_eq!(animation.clip.as_deref(), Some("Walk"));
    assert_eq!(animation.speed, Some(1.5));
    assert_eq!(animation.looping, Some(Repeat::PingPong));
    assert_eq!(animation.start, Some(0.25));
    assert!(!animation.is_sprite());
    let events: Vec<(&str, Option<&str>, Option<f32>)> = animation
        .events
        .iter()
        .map(|event| (event.name.as_str(), event.clip.as_deref(), event.time))
        .collect();
    assert_eq!(
        events,
        [("step", None, Some(0.4)), ("land", Some("Jump"), Some(0.9))]
    );
    assert_eq!(walker.dynamic, Some(true));

    let mixer = scene
        .object("mixer")
        .unwrap()
        .value
        .animation
        .clone()
        .unwrap();
    let blend: Vec<(String, Option<f32>)> = mixer
        .blend
        .unwrap()
        .into_iter()
        .map(|entry| (entry.clip, entry.weight))
        .collect();
    assert_eq!(
        blend,
        [
            ("Walk".to_string(), Some(0.7)),
            ("Run".to_string(), Some(0.3))
        ]
    );

    let flame = scene
        .object("flame")
        .unwrap()
        .value
        .animation
        .clone()
        .unwrap();
    assert!(flame.is_sprite());
    assert_eq!(flame.atlas.as_deref(), Some("flame.png"));
    assert_eq!(flame.frame_count(), Some(8));
    assert_eq!(flame.events[0].frame, Some(7));

    let hero = scene
        .object("hero")
        .unwrap()
        .value
        .animation
        .clone()
        .unwrap();
    assert_eq!(hero.frame_count(), Some(5));
    assert_eq!(
        hero.clips.get("run"),
        Some(&SpriteClip {
            from: 3,
            to: 4,
            fps: Some(20.0),
            looping: Some(Repeat::Once),
        })
    );
    assert_eq!(hero.clips["idle"].count(), 3);
    assert_eq!(scene.object("hero").unwrap().value.dynamic, Some(true));
    assert_eq!(scene.object("crate").unwrap().value.dynamic, Some(true));
    assert_eq!(scene.object("floor").unwrap().value.dynamic, Some(false));
}

#[test]
fn an_animation_takes_the_keys_of_its_kind_with_their_ranges() {
    let glb = |keys: &str| format!("\n[object.animation]\n{keys}");
    let sprite = |keys: &str| format!("\n[object.animation]\natlas = \"flame.png\"\n{keys}");
    let cases = [
        (
            glb("clip = \"Walk\"\nblend = [{ clip = \"Run\" }]\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.blend",
            "clip or blend, not both",
        ),
        (
            glb("speed = -1.0\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.speed",
            "below 0",
        ),
        (
            glb("start = -0.5\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.start",
            "below 0",
        ),
        (
            glb("loop = \"bounce\"\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.loop",
            "unknown variant",
        ),
        (
            glb("blend = []\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.blend",
            "lists no clip",
        ),
        (
            glb("blend = [{ clip = \"Walk\" }, { clip = \"Walk\" }]\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.blend.1.clip",
            "in the blend twice",
        ),
        (
            glb("blend = [{ clip = \"Walk\", weight = -1.0 }, { clip = \"Run\" }]\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.blend.0.weight",
            "below 0",
        ),
        (
            glb("blend = [{ clip = \"Walk\", weight = 0.0 }]\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.blend",
            "add up to 0",
        ),
        (
            glb("clip = \" Walk\"\n"),
            code::BAD_NAME,
            "object.c0be00000x.animation.clip",
            "starts or ends with a space",
        ),
        (
            glb("grid = [2, 2]\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.grid",
            "belongs to a sprite",
        ),
        (
            glb("clip = \"Walk\"\nevents = [{ name = \"step\", frame = 2 }]\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.events.0.frame",
            "a glTF clip has no frames",
        ),
        (
            glb("events = [{ name = \"step\", time = 0.1 }]\n"),
            code::MISSING_KEY,
            "object.c0be00000x.animation.events.0",
            "names its clip",
        ),
        (
            glb("clip = \"Walk\"\nevents = [{ name = \"step\" }]\n"),
            code::MISSING_KEY,
            "object.c0be00000x.animation.events.0",
            "takes a time",
        ),
        (
            glb("clip = \"Walk\"\nevents = [{ name = \"step\", time = -1.0 }]\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.events.0.time",
            "below 0",
        ),
        (
            glb("clip = \"Walk\"\nevents = [{ name = \"\", time = 1.0 }]\n"),
            code::BAD_NAME,
            "object.c0be00000x.animation.events.0.name",
            "event name is empty",
        ),
        (
            sprite(""),
            code::MISSING_KEY,
            "object.c0be00000x.animation",
            "by grid or frames",
        ),
        (
            sprite("grid = [2, 2]\nframes = [[0, 0, 8, 8]]\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.frames",
            "grid or frames, not both",
        ),
        (
            sprite("grid = [0, 2]\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.grid",
            "each at least 1",
        ),
        (
            sprite("frames = [[0, 0, 0, 8]]\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.frames.0",
            "width and height at least 1",
        ),
        (
            sprite("frames = []\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.frames",
            "lists no frame",
        ),
        (
            sprite("grid = [2, 2]\nfps = 0.0\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.fps",
            "not above 0",
        ),
        (
            sprite("grid = [2, 2]\nblend = [{ clip = \"a\" }]\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.blend",
            "belongs to a glTF animation",
        ),
        (
            sprite("grid = [2, 2]\nclips = { a = { from = 2, to = 1 } }\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.clips.a.to",
            "to is at least from",
        ),
        (
            sprite("grid = [2, 2]\nclips = { a = { from = 2, to = 4 } }\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.clips.a.to",
            "past the atlas's last frame 3",
        ),
        (
            sprite("grid = [2, 2]\nclips = { a = { from = 0, to = 1, fps = -2.0 } }\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.clips.a.fps",
            "not above 0",
        ),
        (
            sprite("grid = [2, 2]\nclips = { a = { from = 0, to = 1 } }\nclip = \"b\"\n"),
            code::BAD_REFERENCE,
            "object.c0be00000x.animation.clip",
            "names no clip of clips, which names a",
        ),
        (
            sprite("grid = [2, 2]\nclip = \"a\"\n"),
            code::BAD_REFERENCE,
            "object.c0be00000x.animation.clip",
            "this sprite has no clips",
        ),
        (
            sprite(
                "grid = [2, 2]\nclips = { a = { from = 0, to = 1 } }\nevents = [{ name = \"x\", clip = \"b\", frame = 0 }]\n",
            ),
            code::BAD_REFERENCE,
            "object.c0be00000x.animation.events.0.clip",
            "names no clip of clips",
        ),
        (
            sprite(
                "grid = [2, 2]\nclips = { a = { from = 1, to = 2 } }\nclip = \"a\"\nevents = [{ name = \"x\", frame = 2 }]\n",
            ),
            code::BAD_VALUE,
            "object.c0be00000x.animation.events.0.frame",
            "past its clip's last frame 1",
        ),
        (
            sprite("grid = [2, 2]\nevents = [{ name = \"x\", frame = 4 }]\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.events.0.frame",
            "past its clip's last frame 3",
        ),
        (
            sprite(
                "grid = [2, 2]\nclips = { a = { from = 0, to = 1 } }\nevents = [{ name = \"x\", frame = 0 }]\n",
            ),
            code::MISSING_KEY,
            "object.c0be00000x.animation.events.0",
            "names its clip",
        ),
        (
            sprite("grid = [2, 2]\nevents = [{ name = \"x\", time = 0.1, frame = 1 }]\n"),
            code::BAD_VALUE,
            "object.c0be00000x.animation.events.0.frame",
            "time or frame, not both",
        ),
        (
            glb("clip = \"Walk\"\nweight = 1.0\n"),
            code::UNKNOWN_KEY,
            "object.c0be00000x.animation.weight",
            "unknown key",
        ),
    ];
    for (place, (keys, expected, key, message)) in cases.into_iter().enumerate() {
        let folder = animated(
            &format!("animation-keys-{place}"),
            &format!(
                "\n[[object]]\nid = \"c0be00000x\"\nname = \"cube\"\nmesh = \"block\"\n{keys}"
            ),
        );
        let diagnostics = folder.check();
        let error = one(&diagnostics);
        assert_eq!(error.code, expected, "case {place}: {error}");
        assert_eq!(error.key, key, "case {place}: {error}");
        assert!(error.message.contains(message), "case {place}: {error}");
        assert_eq!(error.file.to_str(), Some("room.scene.toml"));
    }
}

#[test]
fn an_atlas_is_a_path_that_exists_and_a_placement_takes_no_animation() {
    let folder = animated(
        "animation-atlas",
        "\n[[object]]\nid = \"c0be00000x\"\nname = \"cube\"\nmesh = \"block\"\n\n[object.animation]\natlas = \"art/missing.png\"\ngrid = [2, 2]\n",
    );
    let diagnostics = folder.check();
    let error = one(&diagnostics);
    assert_eq!(error.code, code::MISSING_FILE, "{error}");
    assert_eq!(error.key, "object.c0be00000x.animation.atlas");
    let line = folder
        .read("room.scene.toml")
        .lines()
        .position(|line| line.starts_with("atlas = \"art/missing.png\""))
        .unwrap() as u32
        + 1;
    assert_eq!(error.line, line);

    let folder = animated(
        "animation-placement",
        "\n[[object]]\nid = \"1eft1amp00\"\nname = \"lamp\"\nprefab = \"props/lamp.prefab.toml\"\n\n[object.animation]\nclip = \"Glow\"\n",
    );
    folder.write("props/lamp.prefab.toml", common::LAMP);
    let diagnostics = folder.check();
    let error = one(&diagnostics);
    assert_eq!(error.code, code::PLACEMENT_KEY, "{error}");
    assert_eq!(error.key, "object.1eft1amp00.animation");
}

#[test]
fn an_animated_object_is_dynamic_and_refuses_dynamic_false() {
    let folder = animated(
        "animation-static",
        "\n[[object]]\nid = \"c0be00000x\"\nname = \"cube\"\nmesh = \"block\"\ndynamic = false\n\n[object.animation]\nclip = \"Spin\"\n",
    );
    let diagnostics = folder.check();
    let error = one(&diagnostics);
    assert_eq!(error.code, code::BAD_VALUE, "{error}");
    assert_eq!(error.key, "object.c0be00000x.dynamic");
    assert!(error.message.contains("is animated"), "{error}");

    let folder = animated(
        "animation-proxy",
        "\n[plates]\ndir = \"plates\"\nproxies = \"room.proxies.toml\"\n",
    );
    folder.write(
        "room.proxies.toml",
        "format = 1\n\n[[proxy]]\nid = \"pr0xy00001\"\nobject = \"walker\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\n",
    );
    let diagnostics = folder.check();
    let error = one(&diagnostics);
    assert_eq!(error.code, code::BAD_REFERENCE, "{error}");
    assert!(error.message.contains("it is animated"), "{error}");
}

#[test]
fn an_animation_in_a_prefab_is_placed_and_its_overrides_are_checked() {
    let prefab = "format = 1\n\n[mesh.body]\nid = \"m0nstmesh1\"\nfile = \"block.gltf\"\n\n[[object]]\nid = \"m0nster001\"\nname = \"monster\"\nmesh = \"body\"\n\n[object.animation]\nclip = \"Idle\"\n";
    let placed = "\n[[object]]\nid = \"m0nsterp01\"\nname = \"one\"\nprefab = \"props/monster.prefab.toml\"\n\n[object.set]\n\"monster.animation.clip\" = \"Roar\"\n\"monster.animation.speed\" = 2.0\n";
    let folder = animated("animation-prefab", placed);
    folder.write("props/monster.prefab.toml", prefab);
    let diagnostics = folder.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    let scene = folder
        .project()
        .scene(&folder.path("room.scene.toml"))
        .unwrap();
    let monster = &scene.object("one/monster").unwrap().value;
    let animation = monster.animation.as_ref().unwrap();
    assert_eq!(animation.clip.as_deref(), Some("Roar"));
    assert_eq!(animation.speed, Some(2.0));
    assert_eq!(monster.dynamic, Some(true));

    let placed = "\n[[object]]\nid = \"m0nsterp01\"\nname = \"one\"\nprefab = \"props/monster.prefab.toml\"\n\n[object.set]\n\"monster.animation.speed\" = -2.0\n";
    let folder = animated("animation-prefab-bad", placed);
    folder.write("props/monster.prefab.toml", prefab);
    let diagnostics = folder.check();
    let error = one(&diagnostics);
    assert_eq!(error.code, code::BAD_OVERRIDE, "{error}");
    assert_eq!(error.file.to_str(), Some("room.scene.toml"));
    assert!(error.message.contains("speed = -2 is below 0"), "{error}");
}

#[test]
fn the_writer_sets_an_animation_and_refuses_a_bad_one() {
    let folder = animated("animation-edit", "");
    let mut edit = SceneEdit::open(folder.path("room.scene.toml")).unwrap();
    edit.set(
        &Target::Object("walker".into()),
        &["animation", "speed"],
        0.5f32,
    )
    .unwrap();
    edit.set(
        &Target::Object("crate".into()),
        &["animation", "clip"],
        "Shake",
    )
    .unwrap();
    let text = folder.read("room.scene.toml");
    assert!(text.contains("speed = 0.5\n"), "{text}");
    assert_eq!(
        edit.scene()
            .object("crate")
            .unwrap()
            .value
            .animation
            .as_ref()
            .and_then(|animation| animation.clip.as_deref()),
        Some("Shake")
    );
    let error = edit
        .set(
            &Target::Object("hero".into()),
            &["animation", "clip"],
            "fly",
        )
        .unwrap_err();
    assert_eq!(error.code, code::BAD_REFERENCE, "{error}");
    assert_eq!(folder.read("room.scene.toml"), text);
}
