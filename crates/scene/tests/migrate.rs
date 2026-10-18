mod common;

use std::path::Path;

use common::{Folder, errors, show};
use pfx_scene::{FileKind, Id, code, migrate};

const LEGACY: &[&str] = &["room.scene.toml", "lights.scene.toml", "materials.toml"];

fn legacy(name: &str) -> Folder {
    let folder = Folder::empty(name);
    folder.copy_assets("scenes");
    folder.write(
        "project.toml",
        "format = 1\n\n[project]\nname = \"legacy\"\nscene = \"scenes/room.scene.toml\"\n",
    );
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy");
    for file in LEGACY {
        let text = std::fs::read_to_string(fixtures.join(file)).unwrap();
        folder.write(&format!("scenes/{file}"), &text);
    }
    folder
}

fn derived(file: &str, kind: &str, name: &str) -> String {
    Id::derive(format!("{file}\n{kind}\n{name}").as_bytes()).to_string()
}

fn scene(text: &str) -> String {
    migrate(
        text,
        Path::new("a.scene.toml"),
        Path::new("/project"),
        FileKind::Scene,
    )
    .unwrap()
}

#[test]
fn format_1_goes_on_the_first_line() {
    let light = "[[light]]\nname = \"l\"\nposition = [0.0, 0.0, 0.0]\nintensity = 1.0\n";
    let id = derived("a.scene.toml", "light", "l");
    assert_eq!(
        scene(light),
        format!("format = 1\n\n{light}id = \"{id}\"\n")
    );
    assert_eq!(
        scene(&format!("# a lamp\n{light}")),
        format!("format = 1\n\n# a lamp\n{light}id = \"{id}\"\n")
    );
    assert_eq!(
        scene("\n[sky]\nkind = \"analytic\"\n"),
        "format = 1\n\n[sky]\nkind = \"analytic\"\n"
    );
    assert_eq!(
        scene("format = 0\nfallback = \"grey\"\n"),
        "format = 1\n\nfallback = \"grey\"\n"
    );
    assert_eq!(scene(""), "format = 1\n");
}

#[test]
fn paths_become_relative_to_the_project_root() {
    let text = "include = [\"more.scene.toml\"]\nmaterials = [\"../lib/m.toml\"]\n\n[mesh.b]\nfile = \"./meshes/b.gltf\" # the block\nid = \"me5h00000b\"\n\n[content.c]\nimage = \"c.png\"\nid = \"c0ntent001\"\n\n[text.t]\ntext = \"hi\"\nfont = \"../fonts/f.ttf\"\nsize = 1.0\nid = \"text000001\"\n\n[sound.s]\nfile = \"s.wav\"\nid = \"s0nd000001\"\n\n[sky]\nkind = \"mix\"\n\n[[sky.layer]]\nkind = \"hdr\"\npath = 'sky.hdr'\n\n[[sky.layer]]\nkind = \"analytic\"\n\n[finish]\nfile = \"look.toml\"\n";
    let migrated = migrate(
        text,
        Path::new("scenes/a.scene.toml"),
        Path::new("/project"),
        FileKind::Scene,
    )
    .unwrap();
    assert_eq!(
        migrated,
        "format = 1\n\ninclude = [\"scenes/more.scene.toml\"]\nmaterials = [\"lib/m.toml\"]\n\n[mesh.b]\nfile = \"scenes/meshes/b.gltf\" # the block\nid = \"me5h00000b\"\n\n[content.c]\nimage = \"scenes/c.png\"\nid = \"c0ntent001\"\n\n[text.t]\ntext = \"hi\"\nfont = \"fonts/f.ttf\"\nsize = 1.0\nid = \"text000001\"\n\n[sound.s]\nfile = \"scenes/s.wav\"\nid = \"s0nd000001\"\n\n[sky]\nkind = \"mix\"\n\n[[sky.layer]]\nkind = \"hdr\"\npath = \"scenes/sky.hdr\"\n\n[[sky.layer]]\nkind = \"analytic\"\n\n[finish]\nfile = \"scenes/look.toml\"\n"
    );
}

#[test]
fn an_objects_id_becomes_its_pick_in_place() {
    let text = "[[object]]\nname = \"x\"\nmesh = \"b\"\nid = 3 # the pick\nat = [0.0, 0.0, 0.0]\n";
    let id = derived("a.scene.toml", "object", "x");
    assert_eq!(
        scene(text),
        format!(
            "format = 1\n\n[[object]]\nname = \"x\"\nmesh = \"b\"\npick = 3 # the pick\nat = [0.0, 0.0, 0.0]\nid = \"{id}\"\n"
        )
    );
}

#[test]
fn a_body_moves_into_its_object() {
    let text = "[[object]]\nname = \"crate\"\nmesh = \"b\"\n\n[[object]]\nname = \"ball\"\nmesh = \"b\"\n\n# the crate's body\n[body.crate]\nkind = \"fixed\"\nfriction = 0.2\n\n[sky]\nkind = \"analytic\"\n";
    let crate_id = derived("a.scene.toml", "object", "crate");
    let ball_id = derived("a.scene.toml", "object", "ball");
    assert_eq!(
        scene(text),
        format!(
            "format = 1\n\n[[object]]\nname = \"crate\"\nmesh = \"b\"\nid = \"{crate_id}\"\n\n# the crate's body\n[object.body]\nkind = \"fixed\"\nfriction = 0.2\n\n[[object]]\nname = \"ball\"\nmesh = \"b\"\nid = \"{ball_id}\"\n\n[sky]\nkind = \"analytic\"\n"
        )
    );
}

#[test]
fn ids_are_written_into_every_entry_that_can_be_referred_to() {
    let text = "[mesh.b]\nfile = \"b.gltf\"\n\n[[object]]\nname = \"o\"\nmesh = \"b\"\n\n[[light]]\nname = \"l\"\nposition = [0.0, 0.0, 0.0]\nintensity = 1.0\n\n[[emitter]]\nname = \"e\"\nposition = [0.0, 0.0, 0.0]\nradius = 1.0\nintensity = 1.0\n\n[[mover]]\nname = \"m\"\nobjects = [\"o\"]\nkind = \"slide\"\naxis = [1.0, 0.0, 0.0]\ntravel = [0.0, 1.0]\n\n[content.c]\nimage = \"c.png\"\n\n[text.t]\ntext = \"hi\"\nfont = \"f.ttf\"\nsize = 1.0\n\n[sound.s]\nfile = \"s.wav\"\n";
    let migrated = scene(text);
    for (kind, name) in [
        ("mesh", "b"),
        ("object", "o"),
        ("light", "l"),
        ("emitter", "e"),
        ("mover", "m"),
        ("content", "c"),
        ("text", "t"),
        ("sound", "s"),
    ] {
        let id = derived("a.scene.toml", kind, name);
        assert!(
            migrated.contains(&format!("id = \"{id}\"\n")),
            "{kind} {name}:\n{migrated}"
        );
    }
    assert_eq!(migrated.matches("id = ").count(), 8);
    let library = migrate(
        "[materials.a]\nbase = [1.0, 0.0, 0.0]\n\n[materials.b]\nid = \"mat0000002\"\n",
        Path::new("lib/m.toml"),
        Path::new("/project"),
        FileKind::Materials,
    )
    .unwrap();
    assert_eq!(
        library,
        format!(
            "format = 1\n\n[materials.a]\nbase = [1.0, 0.0, 0.0]\nid = \"{}\"\n\n[materials.b]\nid = \"mat0000002\"\n",
            derived("lib/m.toml", "materials", "a")
        )
    );
}

#[test]
fn a_taken_derived_id_is_derived_again() {
    let taken = derived("a.scene.toml", "light", "l");
    let text = format!(
        "[[light]]\nname = \"l\"\nposition = [0.0, 0.0, 0.0]\nintensity = 1.0\n\n[[emitter]]\nid = \"{taken}\"\nname = \"e\"\nposition = [0.0, 0.0, 0.0]\nradius = 1.0\nintensity = 1.0\n"
    );
    let again = Id::derive(b"a.scene.toml\nlight\nl\n1").to_string();
    assert!(scene(&text).contains(&format!("intensity = 1.0\nid = \"{again}\"\n")));
}

#[test]
fn a_format_1_file_is_left_as_it_is() {
    let text = "format = 1\n\n# kept\n[sky]\nkind = \"analytic\"\n";
    assert_eq!(scene(text), text);
}

#[test]
fn a_lone_scene_reaching_past_its_folder_is_named_with_a_suggested_project_file() {
    let folder = Folder::empty("lone");
    folder.copy_assets("shared");
    folder.write(
        "scenes/a.scene.toml",
        "[mesh.b]\nfile = \"../shared/block.gltf\"\n",
    );
    let root = folder.path("scenes");
    let project = pfx_scene::Project::open(&root).unwrap();
    let errors = project.migrate(&root.join("a.scene.toml")).unwrap_err();
    assert_eq!(errors.len(), 1, "{}", show(&errors));
    assert_eq!(errors[0].code, code::OUTSIDE_ROOT);
    assert_eq!((errors[0].line, errors[0].column), (2, 8));
    assert!(
        errors[0].message.contains("../shared/block.gltf"),
        "{}",
        errors[0]
    );
    assert!(
        errors[0].message.contains("a project.toml in .."),
        "{}",
        errors[0]
    );
    assert_eq!(
        std::fs::read_to_string(root.join("a.scene.toml")).unwrap(),
        "[mesh.b]\nfile = \"../shared/block.gltf\"\n"
    );
    let checked = project.check();
    let outside: Vec<_> = checked
        .iter()
        .filter(|d| d.code == code::OUTSIDE_ROOT)
        .collect();
    assert_eq!(outside.len(), 1, "{}", show(&checked));
    folder.write("project.toml", "format = 1\n\n[project]\nname = \"lone\"\n");
    let project = pfx_scene::Project::open(&folder.root).unwrap();
    let migrated = project.migrate(&root.join("a.scene.toml")).unwrap();
    assert!(
        migrated.contains("file = \"shared/block.gltf\""),
        "{migrated}"
    );
}

#[test]
fn a_body_whose_object_is_elsewhere_stops_the_migration() {
    let errors = migrate(
        "[body.ghost]\nkind = \"fixed\"\n",
        Path::new("a.scene.toml"),
        Path::new("/project"),
        FileKind::Scene,
    )
    .unwrap_err();
    assert_eq!(errors[0].code, code::MIGRATION);
    assert_eq!((errors[0].line, errors[0].column), (1, 1));
    assert!(errors[0].message.contains("[body.ghost]"));
}

#[test]
fn the_legacy_room_reads_in_memory_as_its_migration_does_on_disk() {
    let folder = legacy("legacy-room");
    let project = folder.project();
    let diagnostics = project.check();
    let codes: Vec<&str> = diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(
        codes,
        ["old-format", "old-format", "old-format"],
        "{}",
        show(&diagnostics)
    );
    let before = project
        .scene(&folder.path("scenes/room.scene.toml"))
        .unwrap();
    for file in LEGACY {
        let path = folder.path(&format!("scenes/{file}"));
        let migrated = project.migrate(&path).unwrap();
        std::fs::write(&path, &migrated).unwrap();
        assert_eq!(project.migrate(&path).unwrap(), migrated);
    }
    let project = folder.project();
    let diagnostics = project.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    let after = project
        .scene(&folder.path("scenes/room.scene.toml"))
        .unwrap();
    assert_eq!(after.objects, before.objects);
    assert_eq!(after.meshes, before.meshes);
    assert_eq!(after.lights, before.lights);
    assert_eq!(after.materials, before.materials);
    assert_eq!(after.contents, before.contents);
    assert_eq!(after.sky, before.sky);
    assert_eq!(after.camera, before.camera);
    assert_eq!(after.mesh("block").unwrap().value.file, "scenes/block.gltf");
    assert_eq!(
        after.object("crate").unwrap().id,
        Some(derived("scenes/room.scene.toml", "object", "crate"))
    );
    let room = folder.read("scenes/room.scene.toml");
    let original = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/room.scene.toml"),
    )
    .unwrap();
    let kept: Vec<&str> = room
        .lines()
        .skip(2)
        .filter(|line| !line.starts_with("id = "))
        .collect();
    let expected: Vec<String> = original
        .lines()
        .map(|line| {
            line.replace("\"lights.scene.toml\"", "\"scenes/lights.scene.toml\"")
                .replace("\"materials.toml\"", "\"scenes/materials.toml\"")
                .replace("file = \"", "file = \"scenes/")
                .replace("image = \"", "image = \"scenes/")
        })
        .collect();
    assert_eq!(kept, expected);
}

#[test]
fn an_old_files_diagnostics_point_into_the_file_as_written() {
    let folder = Folder::assets("legacy-positions");
    folder.write("project.toml", "format = 1\n\n[project]\nname = \"p\"\n");
    folder.write(
        "a.scene.toml",
        "# old\n[mesh.b]\nfile = \"block.gltf\"\n\n[[object]]\nname = \"ball\"\nmesh = \"b\"\nid = \"seven\"\n\n[[object]]\nname = \"box\"\nmesh = \"b\"\ncolour = 1.0\n\n[[object]]\nname = \"crate\"\nmesh = \"b\"\n\n[body.crate]\ndensity = -1.0\n",
    );
    let diagnostics = folder.check();
    let errors = errors(&diagnostics);
    let find = |code: &str| {
        errors
            .iter()
            .find(|d| d.code == code)
            .unwrap_or_else(|| panic!("no {code} in {}", show(&diagnostics)))
    };
    let pick = find(code::BAD_TYPE);
    assert_eq!((pick.line, pick.column), (8, 6), "{pick}");
    let colour = find(code::UNKNOWN_KEY);
    assert_eq!((colour.line, colour.column), (13, 1), "{colour}");
    let density = find(code::BAD_VALUE);
    assert_eq!(density.line, 19, "{density}");
    let old = diagnostics
        .iter()
        .find(|d| d.code == code::OLD_FORMAT)
        .unwrap();
    assert_eq!((old.line, old.column), (1, 1));
}
