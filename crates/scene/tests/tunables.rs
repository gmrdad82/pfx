mod common;

use std::path::Path;

use common::{Folder, PROJECT, show};
use pfx_scene::types::{Tunable, TunableKind, TunableValue};
use pfx_scene::{Diagnostic, File, check, code};

const TUNABLES: &str = "\n# feel\n[tunables.jump_height]\ntype = \"float\"\ndefault = 2 # metres\nmin = 0.5\nmax = 6\ngroup = \"Movement\"\n\n[tunables.lives]\ntype = \"int\"\ndefault = 3\nmin = 1\nmax = 9\n\n[tunables.\"god mode\"]\ntype = \"bool\"\ndefault = false\ngroup = \"Debug\"\n\n[tunables.gravity]\ntype = \"vector\"\ndefault = [0.0, -9.81, 0]\nmin = [-1, -30.0, -1]\nmax = [1, 0, 1]\ngroup = \"Movement\"\n\n[tunables.drag]\ntype = \"float\"\ndefault = 0.1\n";

fn tuned(name: &str, tunables: &str) -> Folder {
    let folder = Folder::room(name);
    folder.write("project.toml", &format!("{PROJECT}{tunables}"));
    folder
}

fn found(diagnostics: &[Diagnostic]) -> Vec<(&'static str, &str, u32, &str)> {
    diagnostics
        .iter()
        .map(|d| (d.code, d.key.as_str(), d.line, d.message.as_str()))
        .collect()
}

#[test]
fn tunables_read_as_typed_values_with_whole_numbers_of_a_float_read_as_floats() {
    let folder = tuned("tunables-clean", TUNABLES);
    let diagnostics = folder.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    let Ok(File::Project(project)) = folder.project().read(Path::new("project.toml")) else {
        panic!("the project file reads");
    };
    let tunables: Vec<(&str, &Tunable)> = project
        .tunables
        .iter()
        .map(|(name, tunable)| (name.as_str(), tunable))
        .collect();
    let movement = Some("Movement".to_string());
    assert_eq!(
        tunables,
        [
            (
                "drag",
                &Tunable {
                    kind: TunableKind::Float,
                    default: TunableValue::Float(0.1),
                    min: None,
                    max: None,
                    group: None,
                }
            ),
            (
                "god mode",
                &Tunable {
                    kind: TunableKind::Bool,
                    default: TunableValue::Bool(false),
                    min: None,
                    max: None,
                    group: Some("Debug".to_string()),
                }
            ),
            (
                "gravity",
                &Tunable {
                    kind: TunableKind::Vector,
                    default: TunableValue::Vector([0.0, -9.81, 0.0]),
                    min: Some(TunableValue::Vector([-1.0, -30.0, -1.0])),
                    max: Some(TunableValue::Vector([1.0, 0.0, 1.0])),
                    group: movement.clone(),
                }
            ),
            (
                "jump_height",
                &Tunable {
                    kind: TunableKind::Float,
                    default: TunableValue::Float(2.0),
                    min: Some(TunableValue::Float(0.5)),
                    max: Some(TunableValue::Float(6.0)),
                    group: movement,
                }
            ),
            (
                "lives",
                &Tunable {
                    kind: TunableKind::Int,
                    default: TunableValue::Int(3),
                    min: Some(TunableValue::Int(1)),
                    max: Some(TunableValue::Int(9)),
                    group: None,
                }
            ),
        ]
    );
    assert_eq!(project.project.name, "room");
}

#[test]
fn a_default_may_sit_on_its_bounds_and_a_project_without_tunables_holds_none() {
    let folder = tuned(
        "tunables-edges",
        "\n[tunables.speed]\ntype = \"float\"\ndefault = 1.0\nmin = 1.0\nmax = 1.0\n\n[tunables.at]\ntype = \"vector\"\ndefault = [0, 0, 0]\nmin = [0, -1, -1]\nmax = [1, 0, 1]\n\n[tunables.count]\ntype = \"int\"\ndefault = -2\nmin = -2\n",
    );
    let diagnostics = folder.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    let bare = Folder::room("tunables-none");
    let Ok(File::Project(project)) = bare.project().read(Path::new("project.toml")) else {
        panic!("the project file reads");
    };
    assert!(project.tunables.is_empty());
}

#[test]
fn a_tunable_is_refused_at_the_key_that_breaks_it() {
    let tunables = [
        "[tunables.a]\ntype = \"float\"\ndefault = 1.0\nstep = 0.1\n",
        "[tunables.b]\ntype = \"float\"\nmin = 0.0\n",
        "[tunables.c]\ntype = \"string\"\ndefault = 1.0\n",
        "[tunables.d]\ntype = \"int\"\ndefault = 2.0\n",
        "[tunables.e]\ntype = \"vector\"\ndefault = [1.0, 2.0]\n",
        "[tunables.f]\ntype = \"bool\"\ndefault = true\nmax = 1\n",
        "[tunables.g]\ntype = \"float\"\ndefault = 1.0\nmin = 2.0\nmax = 0.0\n",
        "[tunables.h]\ntype = \"vector\"\ndefault = [0.0, -20.0, 0.0]\nmin = [-1.0, -10.0, -1.0]\n",
        "[tunables.i]\ntype = \"int\"\ndefault = 12\nmax = 10\n",
        "[tunables.j]\ntype = \"float\"\ndefault = nan\n",
        "[tunables.k]\ntype = \"float\"\ndefault = 1.0\ngroup = \" Movement\"\n",
        "[tunables.\" l\"]\ntype = \"bool\"\ndefault = true\n",
        "[tunables.m]\ntype = \"vector\"\ndefault = 1.0\nmin = [0, 0, 0]\n",
        "[tunables.n]\ntype = \"bool\"\ndefault = \"yes\"\n",
    ];
    let folder = tuned("tunables-refused", &format!("\n{}", tunables.join("\n")));
    let diagnostics = folder.check();
    let line = |table: &str, key: &str| -> u32 {
        let text = format!("{PROJECT}\n{}", tunables.join("\n"));
        let start = text
            .lines()
            .position(|line| line == table)
            .unwrap_or_else(|| panic!("{table} is in the text"));
        let offset = text
            .lines()
            .skip(start)
            .position(|line| key.is_empty() || line.starts_with(&format!("{key} =")))
            .unwrap();
        (start + offset + 1) as u32
    };
    assert_eq!(
        found(&diagnostics),
        [
            (
                code::UNKNOWN_KEY,
                "tunables.a.step",
                line("[tunables.a]", "step"),
                "unknown key 'step', expected one of 'type', 'default', 'min', 'max', 'group'",
            ),
            (
                code::MISSING_KEY,
                "tunables.b",
                line("[tunables.b]", ""),
                "missing key 'default'",
            ),
            (
                code::BAD_VALUE,
                "tunables.c.type",
                line("[tunables.c]", "type"),
                "unknown variant 'string', expected one of 'float', 'int', 'bool', 'vector'",
            ),
            (
                code::BAD_TYPE,
                "tunables.d.default",
                line("[tunables.d]", "default"),
                "tunable d is an int, so its default is a whole number, not 2.0",
            ),
            (
                code::BAD_TYPE,
                "tunables.e.default",
                line("[tunables.e]", "default"),
                "invalid length 2, expected an array of length 3",
            ),
            (
                code::BAD_VALUE,
                "tunables.f.max",
                line("[tunables.f]", "max"),
                "tunable f is a bool, which takes no max",
            ),
            (
                code::BAD_VALUE,
                "tunables.g.min",
                line("[tunables.g]", "min"),
                "tunable g: min 2.0 is above max 0.0",
            ),
            (
                code::BAD_VALUE,
                "tunables.h.default",
                line("[tunables.h]", "default"),
                "tunable h: default [0.0, -20.0, 0.0] is below min [-1.0, -10.0, -1.0] in y",
            ),
            (
                code::BAD_VALUE,
                "tunables.i.default",
                line("[tunables.i]", "default"),
                "tunable i: default 12 is above max 10",
            ),
            (
                code::BAD_VALUE,
                "tunables.j.default",
                line("[tunables.j]", "default"),
                "tunable j: default = NaN is not finite",
            ),
            (
                code::BAD_NAME,
                "tunables.k.group",
                line("[tunables.k]", "group"),
                "a tunable group name is empty or starts or ends with a space",
            ),
            (
                code::BAD_NAME,
                "tunables. l",
                line("[tunables.\" l\"]", ""),
                "a tunable name is empty or starts or ends with a space",
            ),
            (
                code::BAD_TYPE,
                "tunables.m.default",
                line("[tunables.m]", "default"),
                "tunable m is a vector, so its default is [x, y, z], not 1.0",
            ),
            (
                code::BAD_TYPE,
                "tunables.n.default",
                line("[tunables.n]", "default"),
                "invalid type: string \"yes\", expected true or false, a number or [x, y, z]",
            ),
        ],
        "{}",
        show(&diagnostics)
    );
}

#[test]
fn tunables_must_be_tables_of_tables_and_belong_to_the_project_file() {
    let folder = Folder::room("tunables-shape");
    folder.write(
        "project.toml",
        &PROJECT.replace("\n\n[project]", "\ntunables = 3\n\n[project]"),
    );
    let diagnostics = folder.check();
    assert_eq!(
        found(&diagnostics),
        [(
            code::BAD_TYPE,
            "tunables",
            2,
            "tunables is a table of [tunables.<name>] tables"
        )],
        "{}",
        show(&diagnostics)
    );
    let scene = Folder::room("tunables-in-a-scene");
    let text = format!(
        "{}\n[tunables.speed]\ntype = \"float\"\ndefault = 1.0\n",
        scene.read("lights.scene.toml")
    );
    let diagnostics = check(&text, Path::new("lights.scene.toml"), &scene.project());
    let codes: Vec<&str> = diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(codes, [code::UNKNOWN_KEY], "{}", show(&diagnostics));
}

#[test]
fn an_unsaved_project_file_is_checked_as_it_is_typed() {
    let folder = tuned("tunables-buffer", TUNABLES);
    let project = folder.project();
    let buffer = format!(
        "{PROJECT}{}",
        TUNABLES.replace("default = 3\n", "default = 30\n")
    );
    let diagnostics = check(&buffer, Path::new("project.toml"), &project);
    let at: Vec<(&str, &str, u32, u32)> = diagnostics
        .iter()
        .map(|d| (d.code, d.key.as_str(), d.line, d.column))
        .collect();
    let line = buffer
        .lines()
        .position(|line| line == "default = 30")
        .unwrap() as u32
        + 1;
    assert_eq!(
        at,
        [(code::BAD_VALUE, "tunables.lives.default", line, 11)],
        "{}",
        show(&diagnostics)
    );
    assert!(project.check().is_empty());
}
