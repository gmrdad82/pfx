mod common;

use common::model::{Model, TILTED, TURN};
use pfx_build::{Built, build};
use pfx_recipe::{parse, validate};
use std::path::{Path, PathBuf};

fn folder(name: &str, files: &[(&str, Model)]) -> PathBuf {
    let dir = common::scratch(name);
    for (file, model) in files {
        std::fs::write(dir.join(file), model.glb()).unwrap();
    }
    dir
}

fn built(dir: &Path, objects: &str) -> Built {
    let r = parse(&format!("recipe = 1\nname = \"t\"\n\n{objects}")).unwrap();
    assert_eq!(validate(&r), []);
    build(&r, dir).unwrap()
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
}

const PROP: &str = "[[object]]\nname = \"prop\"\nkind = \"mesh\"\npath = \"model.glb\"\n";

#[test]
fn a_mesh_import_keeps_the_file_s_normals_and_morph_normal_deltas_at_any_node_scale() {
    for scale in [1.0, 1e-4] {
        let dir = folder(
            "import-normals",
            &[(
                "model.glb",
                Model {
                    scale,
                    ..Model::default()
                },
            )],
        );
        let b = built(&dir, PROP);
        let node = &b.parts[0].node;
        let mesh = &node.mesh;
        let morph = &node.targets.as_ref().unwrap().morph;
        let tilted: Vec<usize> = (0..mesh.normals.len())
            .filter(|&i| close(mesh.normals[i], TILTED))
            .collect();
        assert_eq!(tilted.len(), 6, "{scale}: {:?}", mesh.normals);
        for (i, n) in mesh.normals.iter().enumerate() {
            let turn = morph.normals[0][i];
            if tilted.contains(&i) {
                assert!(close(turn, TURN), "{scale}: {turn:?}");
            } else {
                assert!(close(*n, [0.0, 0.0, 1.0]), "{scale}: {n:?}");
                assert!(close(turn, [0.0; 3]), "{scale}: {turn:?}");
            }
        }
    }
}

#[test]
fn keep_normals_false_or_smooth_makes_new_normals_and_a_file_without_normals_is_flat() {
    let dir = folder(
        "import-new-normals",
        &[
            ("model.glb", Model::default()),
            (
                "bare.glb",
                Model {
                    normals: false,
                    ..Model::default()
                },
            ),
        ],
    );
    let b = built(
        &dir,
        &format!(
            "{PROP}keep_normals = false\n\n[[object]]\nname = \"soft\"\nkind = \"mesh\"\npath = \"model.glb\"\nsmooth = true\n\n[[object]]\nname = \"bare\"\nkind = \"mesh\"\npath = \"bare.glb\"\n"
        ),
    );
    for part in &b.parts {
        for n in &part.node.mesh.normals {
            assert!(close(*n, [0.0, 0.0, 1.0]), "{}: {n:?}", part.node.name);
        }
    }
    let r = parse(&format!(
        "recipe = 1\nname = \"t\"\n\n{PROP}keep_normals = true\nsmooth = true\n"
    ))
    .unwrap();
    let problems: Vec<String> = validate(&r).iter().map(ToString::to_string).collect();
    assert_eq!(
        problems,
        [
            "line 8: object[prop].keep_normals: `keep_normals` keeps the file's normals and `smooth` or `smooth_angle` makes new ones, expected only one"
        ]
    );
    let e = parse("recipe = 1\nname = \"t\"\n\n[[object]]\nname = \"b\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\nkeep_normals = true\n")
        .unwrap_err()
        .to_string();
    assert!(e.contains("unknown key `keep_normals`"), "{e}");
}

#[test]
fn a_mesh_import_draws_the_file_s_own_materials_and_the_recipe_overrides_only_the_ones_it_names() {
    let dir = folder("import-materials-own", &[("model.glb", Model::default())]);
    let b = built(
        &dir,
        &format!(
            "[output]\nscene = \"t.scene.toml\"\n\n{PROP}\n[materials.Steel]\nbase = [0.1, 0.2, 0.3]\nroughness = 0.6\n"
        ),
    );
    let r = parse(&format!(
        "recipe = 1\nname = \"t\"\n\n[output]\nscene = \"t.scene.toml\"\n\n{PROP}\n[materials.Steel]\nbase = [0.1, 0.2, 0.3]\nroughness = 0.6\n"
    ))
    .unwrap();
    let merged = b.recipe(&r).materials;
    assert_eq!(
        merged["Paint"]["base"],
        serde_json::json!([0.800000011920929, 0.10000000149011612, 0.10000000149011612])
    );
    assert_eq!(
        merged["Paint"]["roughness"]
            .as_f64()
            .map(|v| (v * 100.0).round()),
        Some(30.0)
    );
    assert_eq!(merged["Steel"]["base"], serde_json::json!([0.1, 0.2, 0.3]));
    assert_eq!(b.notes, Vec::<String>::new());
    let out = common::scratch("import-materials-own-out");
    pfx_build::write(&b, &r, &dir, Some(&out)).unwrap();
    let library = std::fs::read_to_string(out.join("t.materials.toml")).unwrap();
    assert!(library.contains("[materials.Paint]"), "{library}");
    assert!(library.contains("[materials.Steel]"), "{library}");
}

#[test]
fn two_imports_that_reuse_a_material_name_differently_share_a_scene_under_their_own_names() {
    let dir = folder(
        "import-materials-shared",
        &[
            ("model.glb", Model::default()),
            (
                "blue.glb",
                Model {
                    paint: [0.1, 0.2, 0.9],
                    ..Model::default()
                },
            ),
        ],
    );
    let two =
        "[[object]]\nname = \"two\"\nkind = \"mesh\"\npath = \"blue.glb\"\nat = [3.0, 0.0, 0.0]\n";
    let slots = |b: &Built, at: usize| -> Vec<String> {
        b.parts[at]
            .node
            .mesh
            .slots
            .iter()
            .map(|slot| slot.material.clone())
            .collect()
    };
    let b = built(&dir, &format!("{PROP}\n{two}"));
    assert_eq!(slots(&b, 0), ["Paint", "Steel"]);
    assert_eq!(slots(&b, 1), ["two.Paint", "Steel"]);
    let base = |name: &str| b.materials[name]["base"][2].as_f64().unwrap();
    assert!(base("Paint") < 0.2 && base("two.Paint") > 0.8);
    assert_eq!(
        b.notes,
        [
            "materials.Paint: the object `two` has its own `Paint`, unlike the object `prop`'s, so it is named `two.Paint`"
        ]
    );
    let text =
        format!("{PROP}\n{two}\n[materials.Paint]\nbase = [0.5, 0.5, 0.5]\nroughness = 0.5\n");
    let b = built(&dir, &text);
    let r = parse(&format!("recipe = 1\nname = \"t\"\n\n{text}")).unwrap();
    let merged = b.recipe(&r).materials;
    assert_eq!(merged["Paint"]["base"], serde_json::json!([0.5, 0.5, 0.5]));
    assert_eq!(
        merged["two.Paint"]["base"],
        serde_json::json!([0.5, 0.5, 0.5])
    );
    let text =
        format!("{text}\n[materials.\"two.Paint\"]\nbase = [0.0, 0.0, 1.0]\nroughness = 0.5\n");
    let b = built(&dir, &text);
    let r = parse(&format!("recipe = 1\nname = \"t\"\n\n{text}")).unwrap();
    let merged = b.recipe(&r).materials;
    assert_eq!(merged["Paint"]["base"], serde_json::json!([0.5, 0.5, 0.5]));
    assert_eq!(
        merged["two.Paint"]["base"],
        serde_json::json!([0.0, 0.0, 1.0])
    );
}

fn slots(b: &Built, at: usize) -> Vec<String> {
    b.parts[at]
        .node
        .mesh
        .slots
        .iter()
        .map(|slot| slot.material.clone())
        .collect()
}

const RUBBER: &str = "\n[materials.Rubber]\nbase = [0.05, 0.05, 0.05]\nroughness = 0.9\n";

#[test]
fn pieces_of_a_mesh_import_take_their_own_material_or_hide_by_name_path_or_index() {
    let dir = folder("import-pieces", &[("model.glb", Model::default())]);
    let with = |pieces: &str, extra: &str| {
        built(
            &dir,
            &format!("{PROP}{extra}pieces = {{ {pieces} }}\n{RUBBER}"),
        )
    };
    let b = with(
        "\"Arm/Lid\" = { material = \"Rubber\" }, \"#1\" = { hidden = true }",
        "",
    );
    assert_eq!(slots(&b, 0), ["Steel", "Rubber"]);
    assert_eq!(b.parts[0].node.mesh.triangle_count(), 4);
    let b = with(
        "Lid = { material = \"Rubber\" }, \"#3\" = { material = \"Paint\" }",
        "",
    );
    assert_eq!(slots(&b, 0), ["Rubber", "Steel", "Paint"]);
    let b = with("Arm = { hidden = true }", "");
    assert_eq!(slots(&b, 0), ["Paint"]);
    let b = with("Arm = { material = \"Steel\" }", "material = \"Rubber\"\n");
    assert_eq!(slots(&b, 0), ["Rubber", "Steel"]);
    let r = parse(&format!(
        "recipe = 1\nname = \"t\"\n\n{PROP}pieces = {{ Wheel = {{ hidden = true }} }}\n"
    ))
    .unwrap();
    assert_eq!(
        build(&r, &dir).unwrap_err().to_string(),
        "object[prop].pieces.Wheel: model.glb has no node `Wheel`, expected a node name, a parent/child path or #<index> of: Root, Lid, Arm, Lid.3"
    );
    let r = parse(&format!(
        "recipe = 1\nname = \"t\"\n\n{PROP}pieces = {{ Arm = {{ material = \"Rubbr\" }} }}\n{RUBBER}"
    ))
    .unwrap();
    let problems = pfx_build::missing_materials(&r, &dir, &pfx_build::Registry::new());
    assert_eq!(problems.len(), 1);
    assert!(
        problems[0]
            .to_string()
            .contains("object[prop].pieces.Arm.material"),
        "{}",
        problems[0]
    );
    for wrong in ["\"#x\"", "\"a//b\"", "Arm"] {
        let value = if wrong == "Arm" {
            "{}"
        } else {
            "{ hidden = true }"
        };
        let text = format!("recipe = 1\nname = \"t\"\n\n{PROP}pieces = {{ {wrong} = {value} }}\n");
        assert!(parse(&text).is_err(), "{wrong}");
    }
}

#[test]
fn pieces_of_a_library_object_take_their_own_material_or_hide() {
    let dir = common::scratch("import-library-pieces");
    let root = dir.join("library");
    let folder = root.join("prop/box");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join(pfx_build::library::MODEL),
        Model::default().glb(),
    )
    .unwrap();
    let sidecar = pfx_build::library::Sidecar {
        kind: "prop".into(),
        name: "box".into(),
        tags: Vec::new(),
        licence: pfx_build::library::LICENCE.into(),
        credit: None,
        author: None,
        source: None,
        generator: None,
        params: Default::default(),
        pfx: "1.3.0".into(),
        facts: Default::default(),
    };
    std::fs::write(folder.join(pfx_build::library::SIDECAR), sidecar.text()).unwrap();
    pfx_build::library::write_index(&root).unwrap();
    let b = built(
        &dir,
        "[[object]]\nname = \"crate\"\nkind = \"library\"\nobject = \"prop/box\"\npieces = { \"Arm/Lid\" = { hidden = true }, Lid = { material = \"Steel\" } }\n",
    );
    let names: Vec<(&str, bool)> = b
        .parts
        .iter()
        .map(|p| (p.node.name.as_str(), p.hidden))
        .collect();
    assert_eq!(
        names,
        [
            ("crate.Lid", false),
            ("crate.Arm", false),
            ("crate.Lid.3", true)
        ]
    );
    assert_eq!(slots(&b, 0), ["Steel"]);
    assert_eq!(b.nodes().len(), 2);
}
