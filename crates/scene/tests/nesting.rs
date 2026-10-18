mod common;

use common::{Folder, LAMP, ROOM, codes, errors, show};
use pfx_scene::{Diagnostic, EditError, MAX_DEPTH, SceneEdit, Target, Value, check, code};

const HEAD: &str = "format = 1\n\n[project]\nname = \"nesting\"\n";

const STACK: usize = 2 * 1024 * 1024;

const PLACED: &str = "\n[[object]]\nid = \"1eft1amp00\"\nname = \"left lamp\"\nprefab = \"props/lamp.prefab.toml\"\nat = [-1.2, 0.0, 0.4]\n";

fn on_small_stack<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(work)
        .unwrap()
        .join()
        .unwrap()
}

fn includes(folder: &Folder, count: usize) {
    for at in 0..=count {
        let include = if at < count {
            format!("include = [\"s{}.scene.toml\"]\n", at + 1)
        } else {
            String::new()
        };
        folder.write(
            &format!("s{at}.scene.toml"),
            &format!(
                "format = 1\n{include}\n[[light]]\nid = \"s{at:09}\"\nname = \"light {at}\"\nposition = [0.0, 1.0, 0.0]\nintensity = 1.0\n"
            ),
        );
    }
}

fn prefabs(folder: &Folder, count: usize) {
    folder.write(
        "room.scene.toml",
        "format = 1\n\n[[object]]\nid = \"r000000000\"\nname = \"next\"\nprefab = \"p1.prefab.toml\"\n",
    );
    for at in 1..count {
        folder.write(
            &format!("p{at}.prefab.toml"),
            &format!(
                "format = 1\n\n[[object]]\nid = \"p{at:09}\"\nname = \"next\"\nprefab = \"p{}.prefab.toml\"\n",
                at + 1
            ),
        );
    }
    folder.write(
        &format!("p{count}.prefab.toml"),
        "format = 1\n\n[mesh.b]\nid = \"e000000001\"\nfile = \"block.gltf\"\n\n[[object]]\nid = \"e000000002\"\nname = \"end\"\nmesh = \"b\"\n",
    );
}

fn one<'a>(diagnostics: &'a [Diagnostic], code: &str) -> &'a Diagnostic {
    let found: Vec<&Diagnostic> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == code)
        .collect();
    assert_eq!(found.len(), 1, "one {code} in:\n{}", show(diagnostics));
    found[0]
}

fn place(diagnostic: &Diagnostic) -> (String, u32, u32) {
    (
        diagnostic.file.to_str().unwrap().to_string(),
        diagnostic.line,
        diagnostic.column,
    )
}

fn related(diagnostic: &Diagnostic) -> Vec<(String, u32, u32)> {
    diagnostic
        .related
        .iter()
        .map(|location| {
            (
                location.file.to_str().unwrap().to_string(),
                location.line,
                location.column,
            )
        })
        .collect()
}

fn at(file: &str, line: u32, column: u32) -> (String, u32, u32) {
    (file.to_string(), line, column)
}

#[test]
fn an_include_cycle_names_every_file_of_it_in_related() {
    let folder = Folder::assets("nesting-include-cycle");
    folder.write("project.toml", HEAD);
    folder.write("a.scene.toml", "format = 1\ninclude = [\"b.scene.toml\"]\n");
    folder.write("b.scene.toml", "format = 1\ninclude = [\"c.scene.toml\"]\n");
    folder.write("c.scene.toml", "format = 1\ninclude = [\"a.scene.toml\"]\n");
    let project = folder.project();
    let found = project.scene(&folder.path("a.scene.toml")).unwrap_err();
    let cycle = one(&found, code::INCLUDE_CYCLE);
    assert_eq!(place(cycle), at("c.scene.toml", 2, 12));
    assert!(
        cycle.message.ends_with(
            "the scene includes itself through a.scene.toml: a.scene.toml -> b.scene.toml -> c.scene.toml -> a.scene.toml"
        ),
        "{cycle}"
    );
    assert_eq!(
        related(cycle),
        [
            at("a.scene.toml", 2, 12),
            at("b.scene.toml", 2, 12),
            at("c.scene.toml", 2, 12)
        ]
    );
    let text = folder.read("c.scene.toml");
    let checked = check(&text, &folder.path("c.scene.toml"), &project);
    assert_eq!(codes(&checked), [code::INCLUDE_CYCLE], "{}", show(&checked));
    assert_eq!(checked[0], *cycle);
    assert_eq!(*one(&project.check(), code::INCLUDE_CYCLE), *cycle);
}

#[test]
fn a_prefab_cycle_names_every_file_of_it_in_related() {
    let folder = Folder::assets("nesting-prefab-cycle");
    folder.write("project.toml", HEAD);
    folder.write(
        "room.scene.toml",
        "format = 1\n\n[[object]]\nid = \"r000000001\"\nname = \"a\"\nprefab = \"a.prefab.toml\"\n",
    );
    folder.write(
        "a.prefab.toml",
        "format = 1\n\n[[object]]\nid = \"a000000001\"\nname = \"b\"\nprefab = \"b.prefab.toml\"\n",
    );
    folder.write(
        "b.prefab.toml",
        "format = 1\n\n[[object]]\nid = \"b000000001\"\nname = \"a\"\nprefab = \"a.prefab.toml\"\n",
    );
    let project = folder.project();
    let found = project.scene(&folder.path("room.scene.toml")).unwrap_err();
    let cycle = one(&found, code::PREFAB_CYCLE);
    assert_eq!(place(cycle), at("b.prefab.toml", 6, 10));
    assert_eq!(
        cycle.message,
        "object a places a.prefab.toml, which places itself: a.prefab.toml -> b.prefab.toml -> a.prefab.toml"
    );
    assert_eq!(
        related(cycle),
        [at("a.prefab.toml", 6, 10), at("b.prefab.toml", 6, 10)]
    );
    let text = folder.read("b.prefab.toml");
    let checked = check(&text, &folder.path("b.prefab.toml"), &project);
    assert_eq!(codes(&checked), [code::PREFAB_CYCLE], "{}", show(&checked));
    assert_eq!(*one(&project.check(), code::PREFAB_CYCLE), *cycle);
}

#[test]
fn a_prefab_placement_resolves_on_a_two_megabyte_stack() {
    on_small_stack(|| {
        let folder = Folder::room("nesting-placement");
        folder.write("props/lamp.prefab.toml", LAMP);
        folder.write("room.scene.toml", &format!("{ROOM}{PLACED}"));
        let project = folder.project();
        let scene = project
            .scene(&folder.path("room.scene.toml"))
            .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)));
        let shade = scene.object("left lamp/shade").unwrap();
        assert_eq!(shade.id.as_deref(), Some("1eft1amp00/1amp000001"));
        assert!(scene.light("left lamp/bulb").is_some());
        let diagnostics = project.check();
        assert!(errors(&diagnostics).is_empty(), "{}", show(&diagnostics));
    });
}

#[test]
fn includes_and_prefabs_nest_to_the_limit_on_a_two_megabyte_stack() {
    on_small_stack(|| {
        let folder = Folder::assets("nesting-limit");
        folder.write("project.toml", HEAD);
        includes(&folder, MAX_DEPTH);
        prefabs(&folder, MAX_DEPTH);
        let project = folder.project();
        let scene = project
            .scene(&folder.path("s0.scene.toml"))
            .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)));
        assert_eq!(scene.lights.len(), MAX_DEPTH + 1);
        assert_eq!(scene.lights[0].key, format!("light {MAX_DEPTH}"));
        let scene = project
            .scene(&folder.path("room.scene.toml"))
            .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)));
        assert_eq!(scene.objects.len(), MAX_DEPTH + 1);
        let end = format!("{}end", "next/".repeat(MAX_DEPTH));
        assert_eq!(scene.objects[MAX_DEPTH].key, end);
        let diagnostics = project.check();
        assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    });
}

#[test]
fn chains_a_thousand_deep_are_refused_with_their_chain() {
    on_small_stack(|| {
        let folder = Folder::assets("nesting-thousand");
        folder.write("project.toml", HEAD);
        includes(&folder, 1000);
        prefabs(&folder, 1000);
        let project = folder.project();

        let found = project.scene(&folder.path("s0.scene.toml")).unwrap_err();
        let deep = one(&found, code::INCLUDE_DEPTH);
        assert_eq!(place(deep), at(&format!("s{MAX_DEPTH}.scene.toml"), 2, 12));
        assert_eq!(
            deep.message,
            format!(
                "including s65.scene.toml nests the scene's includes more than {MAX_DEPTH} deep: s0.scene.toml -> s1.scene.toml -> s2.scene.toml -> ... -> s63.scene.toml -> s64.scene.toml -> s65.scene.toml"
            )
        );
        let chain: Vec<(String, u32, u32)> = (0..=MAX_DEPTH)
            .map(|step| at(&format!("s{step}.scene.toml"), 2, 12))
            .collect();
        assert_eq!(related(deep), chain);

        let found = project.scene(&folder.path("room.scene.toml")).unwrap_err();
        let deep = one(&found, code::PREFAB_DEPTH);
        assert_eq!(place(deep), at(&format!("p{MAX_DEPTH}.prefab.toml"), 6, 10));
        assert_eq!(
            deep.message,
            format!(
                "object next places p65.prefab.toml, which nests prefabs more than {MAX_DEPTH} deep: room.scene.toml -> p1.prefab.toml -> p2.prefab.toml -> ... -> p63.prefab.toml -> p64.prefab.toml -> p65.prefab.toml"
            )
        );
        let chain: Vec<(String, u32, u32)> = std::iter::once(at("room.scene.toml", 6, 10))
            .chain((1..=MAX_DEPTH).map(|step| at(&format!("p{step}.prefab.toml"), 6, 10)))
            .collect();
        assert_eq!(related(deep), chain);

        let diagnostics = project.check();
        let other: Vec<&Diagnostic> = diagnostics
            .iter()
            .filter(|d| d.code != code::INCLUDE_DEPTH && d.code != code::PREFAB_DEPTH)
            .collect();
        assert!(other.is_empty(), "{}", show(&diagnostics));
        assert!(diagnostics.contains(deep));
    });
}

fn cycle(folder: &Folder) {
    folder.write("project.toml", HEAD);
    folder.write("a.scene.toml", "format = 1\ninclude = [\"b.scene.toml\"]\n");
    folder.write("b.scene.toml", "format = 1\ninclude = [\"c.scene.toml\"]\n");
    folder.write("c.scene.toml", "format = 1\ninclude = [\"a.scene.toml\"]\n");
}

fn refused(opened: Result<SceneEdit, EditError>) -> EditError {
    match opened {
        Ok(_) => panic!("the scene opened"),
        Err(error) => error,
    }
}

#[test]
fn a_scene_edit_refuses_an_include_cycle_with_its_chain_on_a_two_megabyte_stack() {
    on_small_stack(|| {
        let folder = Folder::assets("nesting-edit-cycle");
        cycle(&folder);
        folder.write(
            "self.scene.toml",
            "format = 1\ninclude = [\"self.scene.toml\"]\n",
        );
        let project = folder.project();

        let found = project.scene(&folder.path("a.scene.toml")).unwrap_err();
        let error = refused(SceneEdit::open(folder.path("a.scene.toml")));
        assert_eq!(error.code, code::INCLUDE_CYCLE, "{error}");
        assert_eq!(error.file, folder.path("c.scene.toml"));
        assert_eq!(error.line, Some(2));
        assert_eq!(error.message, one(&found, code::INCLUDE_CYCLE).message);
        assert_eq!(error.diagnostics, found);
        assert_eq!(
            related(one(&error.diagnostics, code::INCLUDE_CYCLE)),
            [
                at("a.scene.toml", 2, 12),
                at("b.scene.toml", 2, 12),
                at("c.scene.toml", 2, 12)
            ]
        );

        let found = project.scene(&folder.path("self.scene.toml")).unwrap_err();
        let error = refused(SceneEdit::open(folder.path("self.scene.toml")));
        assert_eq!(error.code, code::INCLUDE_CYCLE, "{error}");
        assert_eq!(error.diagnostics, found);
        assert!(
            error
                .message
                .ends_with("self.scene.toml -> self.scene.toml"),
            "{error}"
        );
    });
}

#[test]
fn a_scene_edit_refuses_includes_too_deep_with_their_chain_on_a_two_megabyte_stack() {
    on_small_stack(|| {
        let folder = Folder::assets("nesting-edit-depth");
        folder.write("project.toml", HEAD);
        includes(&folder, MAX_DEPTH + 1);
        let project = folder.project();
        let found = project.scene(&folder.path("s0.scene.toml")).unwrap_err();
        let error = refused(SceneEdit::open(folder.path("s0.scene.toml")));
        assert_eq!(error.code, code::INCLUDE_DEPTH, "{error}");
        assert_eq!(error.file, folder.path(&format!("s{MAX_DEPTH}.scene.toml")));
        assert_eq!(error.diagnostics, found);
        assert_eq!(
            related(one(&error.diagnostics, code::INCLUDE_DEPTH)).len(),
            MAX_DEPTH + 1
        );
        assert!(SceneEdit::open(folder.path("s1.scene.toml")).is_ok());
    });
}

#[test]
fn an_edit_that_makes_an_include_cycle_is_refused_with_its_chain_in_a_dry_run_too() {
    on_small_stack(|| {
        let folder = Folder::assets("nesting-edit-makes-cycle");
        folder.write("project.toml", HEAD);
        includes(&folder, 1);
        let before = folder.read("s0.scene.toml");
        let mut edit = SceneEdit::open(folder.path("s0.scene.toml")).unwrap();
        let looped = || {
            Value::Array(vec![
                Value::Text("s1.scene.toml".into()),
                Value::Text("s0.scene.toml".into()),
            ])
        };
        let chain = [at("s0.scene.toml", 2, 29)];

        let error = edit
            .set(&Target::Scene, &["include"], looped())
            .unwrap_err();
        assert_eq!(error.code, code::INCLUDE_CYCLE, "{error}");
        assert_eq!(related(one(&error.diagnostics, code::INCLUDE_CYCLE)), chain);
        assert_eq!(folder.read("s0.scene.toml"), before);

        let error = edit
            .dry_run(|edit| edit.set(&Target::Scene, &["include"], looped()))
            .unwrap_err();
        assert_eq!(error.code, code::INCLUDE_CYCLE, "{error}");
        assert_eq!(related(one(&error.diagnostics, code::INCLUDE_CYCLE)), chain);
        assert_eq!(folder.read("s0.scene.toml"), before);

        assert_eq!(edit.scene().lights.len(), 2);
        edit.set(&Target::Light("light 0".into()), &["intensity"], 2.0f32)
            .unwrap();
    });
}

#[test]
fn a_fix_refuses_a_cycle_or_a_chain_too_deep_with_the_findings_check_gives() {
    on_small_stack(|| {
        let folder = Folder::assets("nesting-fix");
        cycle(&folder);
        prefabs(&folder, MAX_DEPTH + 1);
        let project = folder.project();

        let found = project.scene(&folder.path("a.scene.toml")).unwrap_err();
        let error = project.fix(&folder.path("a.scene.toml")).unwrap_err();
        assert_eq!(error.code, code::INCLUDE_CYCLE, "{error}");
        assert_eq!(error.file, folder.path("c.scene.toml"));
        assert_eq!(error.line, Some(2));
        assert_eq!(error.diagnostics, found);
        assert_eq!(
            related(one(&error.diagnostics, code::INCLUDE_CYCLE)).len(),
            3
        );

        let found = project.scene(&folder.path("room.scene.toml")).unwrap_err();
        let error = project.fix(&folder.path("room.scene.toml")).unwrap_err();
        assert_eq!(error.code, code::PREFAB_DEPTH, "{error}");
        assert_eq!(
            error.file,
            folder.path(&format!("p{MAX_DEPTH}.prefab.toml"))
        );
        assert_eq!(error.diagnostics, found);
        assert_eq!(
            related(one(&error.diagnostics, code::PREFAB_DEPTH)).len(),
            MAX_DEPTH + 1
        );
    });
}
