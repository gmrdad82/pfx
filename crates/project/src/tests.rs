use std::path::{Path, PathBuf};

use crate::Project;

fn folder(name: &str) -> PathBuf {
    let root = std::path::absolute(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/project-tests")
            .join(name),
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn touch(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn a_project_reads_its_name_first_scene_and_play_entry() {
    let root = folder("read");
    touch(
        &root,
        "project.toml",
        "format = 1\n\n[project]\nname = \"demo\"\nscene = \"content/a.scene.toml\"\n\n[authoring.pfx]\nplay = \"content/b.scene.toml\"\n",
    );
    let project = Project::open(&root).unwrap();
    assert_eq!(project.name(), "demo");
    assert_eq!(project.scene(), Some(root.join("content/a.scene.toml")));
    assert_eq!(project.first(), Some(root.join("content/a.scene.toml")));
    assert_eq!(project.play(), root.join("content/b.scene.toml"));
    assert_eq!(project.select(), None);
    assert_eq!(project.screen(), None);
    assert!(project.marked());
    assert_eq!(project.version(), None);
    touch(
        &root,
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"2.3.4\"\n",
    );
    assert_eq!(project.version().as_deref(), Some("2.3.4"));
    touch(&root, "Cargo.toml", "[package]\nversion.workspace = true\n");
    assert_eq!(project.version(), None);
}

#[test]
fn play_falls_back_to_the_first_scene_then_to_the_scenes_listed() {
    let root = folder("fallback");
    touch(
        &root,
        "project.toml",
        "format = 1\n[project]\nname = \"demo\"\nscene = \"content/a.scene.toml\"\n",
    );
    assert_eq!(
        Project::open(&root).unwrap().play(),
        root.join("content/a.scene.toml")
    );
    touch(
        &root,
        "project.toml",
        "format = 1\n[project]\nname = \"demo\"\n",
    );
    touch(&root, "content/z.scene.toml", "format = 1\n");
    touch(&root, "content/m.scene.toml", "format = 1\n");
    let project = Project::open(&root).unwrap();
    assert_eq!(project.play(), root.join("content/m.scene.toml"));
    assert_eq!(project.first(), Some(root.join("content/m.scene.toml")));
}

#[test]
fn a_bad_project_file_is_refused_with_its_path() {
    let root = folder("bad");
    touch(&root, "project.toml", "format = 1\n");
    let error = Project::open(&root).unwrap_err();
    assert!(
        error.contains("project.toml") && error.contains("[project]"),
        "{error}"
    );
    touch(&root, "project.toml", "[project]\nname = \"\"\n");
    assert!(Project::open(&root).unwrap_err().contains("no name"));
    touch(
        &root,
        "project.toml",
        "[project]\nname = \"x\"\nscene = 3\n",
    );
    assert!(Project::open(&root).unwrap_err().contains("scene"));
    touch(
        &root,
        "project.toml",
        "[project]\nname = \"x\"\n[authoring.pfx]\nselect = 1\n",
    );
    assert!(Project::open(&root).unwrap_err().contains("select"));
    touch(
        &root,
        "project.toml",
        "[project]\nname = \"x\"\nignore = [3]\n",
    );
    assert!(Project::open(&root).unwrap_err().contains("ignore"));
    touch(
        &root,
        "project.toml",
        "[project]\nname = \"x\"\n[authoring.pfx]\nscreen = \"16:9\"\n",
    );
    assert!(Project::open(&root).unwrap_err().contains("screen"));
    assert!(Project::open(root.join("missing")).is_err());
}

#[test]
fn a_flat_project_says_so_in_its_authoring_table() {
    let root = folder("flat");
    touch(
        &root,
        "project.toml",
        "[project]\nname = \"x\"\n[authoring.pfx]\nflat = true\n",
    );
    let project = Project::open(&root).unwrap();
    assert!(project.flat());
    assert_eq!(project.first(), None);
    touch(&root, "project.toml", "[project]\nname = \"x\"\n");
    assert!(!Project::open(&root).unwrap().flat());
    assert!(!Project::folder(root.join("..")).unwrap().flat());
    touch(
        &root,
        "project.toml",
        "[project]\nname = \"x\"\n[authoring.pfx]\nflat = \"yes\"\n",
    );
    assert!(Project::open(&root).unwrap_err().contains("flat"));
}

#[test]
fn a_project_lists_its_gameplay_modules_by_their_built_files() {
    let root = folder("modules");
    touch(
        &root,
        "project.toml",
        "[project]\nname = \"x\"\n[authoring.pfx]\nmodules = [\"target/wasm32-unknown-unknown/release/brain.wasm\", \"content/legs.wasm\"]\n",
    );
    let modules = Project::open(&root).unwrap().modules();
    assert_eq!(
        modules,
        [
            crate::Module {
                id: "brain".to_string(),
                path: root.join("target/wasm32-unknown-unknown/release/brain.wasm"),
            },
            crate::Module {
                id: "legs".to_string(),
                path: root.join("content/legs.wasm"),
            },
        ]
    );
    for bad in [
        "modules = \"brain.wasm\"",
        "modules = [3]",
        "modules = [\"brain.rs\"]",
    ] {
        touch(
            &root,
            "project.toml",
            &format!("[project]\nname = \"x\"\n[authoring.pfx]\n{bad}\n"),
        );
        let error = Project::open(&root).unwrap_err();
        assert!(error.contains("modules lists the built .wasm"), "{error}");
    }
    touch(&root, "project.toml", "[project]\nname = \"x\"\n");
    assert!(Project::open(&root).unwrap().modules().is_empty());
}

#[test]
fn scenes_and_content_folders_skip_build_output_dot_folders_and_other_projects() {
    let root = folder("walk");
    touch(&root, "project.toml", "[project]\nname = \"demo\"\n");
    touch(&root, "Cargo.toml", "[package]\n");
    touch(&root, "src/lib.rs", "");
    touch(&root, "content/b.scene.toml", "");
    touch(&root, "content/a.scene.toml", "");
    touch(&root, "content/base.materials.toml", "");
    touch(&root, "content/notes.toml", "");
    touch(&root, "content/meshes/card.gltf", "");
    touch(&root, "content/marks/lockup.svg", "");
    touch(&root, "content/marks/lockup.PNG", "");
    touch(&root, "target/debug/x.scene.toml", "");
    touch(&root, "tmp/x.scene.toml", "");
    touch(&root, ".hidden/x.scene.toml", "");
    touch(
        &root,
        "nested/project.toml",
        "[project]\nname = \"other\"\n",
    );
    touch(&root, "nested/x.scene.toml", "");
    touch(&root, "levels/tmp/deep.scene.toml", "");
    touch(&root, "builds/x.scene.toml", "");
    touch(&root, "content/old.scene.toml", "");
    touch(
        &root,
        "project.toml",
        "[project]\nname = \"demo\"\nignore = [\"builds\", \"content/old.scene.toml\"]\n",
    );
    let project = Project::open(&root).unwrap();
    assert_eq!(
        project.scenes(),
        vec![
            root.join("content/a.scene.toml"),
            root.join("content/b.scene.toml"),
            root.join("levels/tmp/deep.scene.toml"),
        ]
    );
    let folders: Vec<(String, usize)> = project
        .folders()
        .iter()
        .map(|folder| (project.relative(&folder.path), folder.files.len()))
        .collect();
    assert_eq!(
        folders,
        vec![
            ("content".to_string(), 3),
            ("content/marks".to_string(), 2),
            ("content/meshes".to_string(), 1),
            ("levels/tmp".to_string(), 1),
        ]
    );
}

#[test]
fn a_folder_without_a_project_file_is_a_bare_project_of_its_scenes() {
    let root = folder("bare");
    touch(&root, "room.scene.toml", "");
    touch(&root, "materials.toml", "");
    touch(&root, "block.gltf", "");
    let project = Project::folder(&root).unwrap();
    assert!(!project.marked());
    assert_eq!(project.name(), "bare");
    assert_eq!(project.scenes(), vec![root.join("room.scene.toml")]);
    assert_eq!(project.first(), Some(root.join("room.scene.toml")));
    assert_eq!(project.folders()[0].files.len(), 3);
    assert!(Project::folder(root.join("missing")).is_err());
}

#[test]
fn find_walks_up_to_the_nearest_project_file() {
    let root = folder("find");
    touch(&root, "project.toml", "[project]\nname = \"demo\"\n");
    touch(&root, "content/deep/a.scene.toml", "");
    assert_eq!(
        Project::find(root.join("content/deep/a.scene.toml")),
        Some(root.clone())
    );
    assert_eq!(Project::find(root.join("content/deep")), Some(root.clone()));
}

#[cfg(feature = "scaffold")]
mod scaffold {
    use super::*;

    fn cyclic(root: &std::path::Path, kind: &str) -> std::path::PathBuf {
        let _ = std::fs::remove_dir_all(root);
        std::fs::create_dir_all(root.join("content")).unwrap();
        let write = |relative: &str, text: &str| std::fs::write(root.join(relative), text).unwrap();
        write(
            "project.toml",
            "format = 1\n\n[project]\nname = \"loop\"\nscene = \"content/main.scene.toml\"\n",
        );
        match kind {
            "prefab" => {
                write(
                    "content/main.scene.toml",
                    "format = 1\n\n[[object]]\nname = \"group\"\nprefab = \"content/loop.prefab.toml\"\n",
                );
                write(
                    "content/loop.prefab.toml",
                    "format = 1\n\n[[object]]\nname = \"again\"\nprefab = \"content/loop.prefab.toml\"\n",
                );
            }
            "include" => {
                write(
                    "content/main.scene.toml",
                    "format = 1\n\ninclude = [\"content/other.scene.toml\"]\n",
                );
                write(
                    "content/other.scene.toml",
                    "format = 1\n\ninclude = [\"content/main.scene.toml\"]\n",
                );
            }
            _ => {
                write(
                    "content/main.scene.toml",
                    "format = 1\n\n[[object]]\nname = \"group\"\nprefab = \"content/p0.prefab.toml\"\n",
                );
                for at in 0..70 {
                    write(
                        &format!("content/p{at}.prefab.toml"),
                        &format!(
                            "format = 1\n\n[[object]]\nname = \"next\"\nprefab = \"content/p{}.prefab.toml\"\n",
                            at + 1
                        ),
                    );
                }
                write("content/p70.prefab.toml", "format = 1\n");
            }
        }
        root.join("content/main.scene.toml")
    }

    const FINDINGS: [(&str, &str); 3] = [
        ("prefab", "which places itself"),
        ("include", "includes itself"),
        ("deep", "nests prefabs more than 64 deep"),
    ];

    #[test]
    fn the_project_check_refuses_a_cyclic_or_too_deep_project_with_its_finding() {
        for (kind, says) in FINDINGS {
            let root = folder(&format!("cyclic-{kind}"));
            cyclic(&root, kind);
            let error = crate::scaffold::fix_ids(&root).unwrap_err();
            assert!(error.contains(says), "{kind}: {error}");
            if kind == "include" {
                for file in [
                    "main.scene.toml:3:12: see here",
                    "other.scene.toml:3:12: see here",
                ] {
                    assert!(error.contains(file), "the chain names {file}: {error}");
                }
            }
        }
    }

    use crate::scaffold::{
        DOCUMENT, MARK, Options, PREFAB, SCENE, check_name, fill, module_path, scaffold,
        scaffold_with,
    };

    fn unbranded(root: &std::path::Path, written: &[std::path::PathBuf]) {
        let marks = [crate::marks::BADGE, crate::marks::WORDMARK];
        let reader = pfx_file::Reader::open(root.join(DOCUMENT)).unwrap();
        let mut texts: Vec<(String, String)> = reader
            .entries()
            .iter()
            .filter(|entry| entry.tag == pfx_file::SCNE)
            .map(|entry| (entry.name.clone(), held(root, &entry.name)))
            .collect();
        for path in written {
            let bytes = std::fs::read(path).unwrap();
            let name = path.display().to_string();
            assert!(
                !marks.contains(&bytes.as_slice()),
                "{name} is pfx's own mark or wordmark"
            );
            if path.starts_with(root.join("content"))
                && let Ok(text) = String::from_utf8(bytes)
            {
                texts.push((name, text));
            }
        }
        for (name, text) in texts {
            for brand in ["badge", "wordmark", "incoming", "pfx-icon", "assets/marks"] {
                assert!(!text.contains(brand), "{name} names pfx's {brand}:\n{text}");
            }
            assert!(
                !text.contains("text = \"v9.8.7\""),
                "{name} writes pfx's version as text"
            );
        }
    }

    fn held(root: &std::path::Path, name: &str) -> String {
        let reader = pfx_file::Reader::open(root.join(DOCUMENT)).unwrap();
        let bytes = reader.named(pfx_file::SCNE, name).unwrap().unwrap();
        String::from_utf8(bytes.into_owned()).unwrap()
    }

    #[test]
    fn a_name_must_be_a_lowercase_slug_that_is_not_taken() {
        for good in ["game", "my-game", "a1", "space-2-go"] {
            assert!(check_name(good).is_ok(), "{good}");
        }
        for bad in [
            "", "Game", "my_game", "my game", "-game", "game-", "my--game", "1game", "pfx",
            "pfx-game", "test", "self", "gämе",
        ] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn templates_fill_the_names_tag_and_source() {
        let text = fill(
            "{{name}} {{crate}} {{type}} {{tag}} {{source}}",
            "space-2-go",
            "1.2.3",
        );
        assert_eq!(
            text,
            "space-2-go space_2_go Space2Go v1.2.3 https://github.com/gmrdad82/pfx.git"
        );
    }

    #[test]
    fn the_scaffold_writes_a_game_into_an_empty_repo_folder_and_its_scene_checks() {
        let root = folder("scaffold");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let written = scaffold("my-game", &root, "9.8.7").unwrap();
        for relative in [
            "Cargo.toml",
            "src/lib.rs",
            "src/main.rs",
            "tests/first.rs",
            "project.toml",
            DOCUMENT,
            "content/meshes/name.glb",
            "content/meshes/mark.glb",
            "content/meshes/grid.glb",
            "content/fonts/IBMPlexMono-Regular.ttf",
            "content/fonts/IBMPlexMono-OFL.txt",
            "AGENTS.md",
            "CLAUDE.md",
            "prompts/RESUME.md",
            ".gitignore",
        ] {
            assert!(written.contains(&root.join(relative)), "{relative}");
        }
        let read = |relative: &str| std::fs::read_to_string(root.join(relative)).unwrap();
        let cargo = read("Cargo.toml");
        assert!(cargo.contains("name = \"my-game\""));
        assert!(cargo.contains("default = [\"tools\"]"));
        assert!(cargo.contains("tools = [\"pfx-game/tools\", \"dep:pfx-workbench\"]"));
        assert!(cargo.contains(
            "pfx-game = { git = \"https://github.com/gmrdad82/pfx.git\", tag = \"v9.8.7\" }"
        ));
        assert!(!cargo.contains("{{"));
        let main = read("src/main.rs");
        assert!(main.starts_with(
            "#![cfg_attr(not(feature = \"tools\"), windows_subsystem = \"windows\")]"
        ));
        assert!(main.contains("pfx_game::launch(my_game::MyGame::new"));
        assert!(main.contains(".usage(\"edit\", pfx_workbench::EDIT_USAGE)"));
        assert!(
            main.contains("pfx_workbench::edit_project(project.root(), factory, config, args)")
        );
        let project = Project::open(&root).unwrap();
        assert_eq!(project.name(), "my-game");
        assert_eq!(project.play(), root.join(DOCUMENT));
        assert_eq!(project.scenes(), vec![root.join(DOCUMENT)]);
        for gone in [
            SCENE,
            "content/materials/base.materials.toml",
            "content/materials/title.materials.toml",
            PREFAB,
        ] {
            assert!(!root.join(gone).exists(), "{gone} is inside the .pitofx");
            assert!(!written.contains(&root.join(gone)), "{gone}");
        }
        assert!(!root.join("content/materials").exists() && !root.join("content/prefabs").exists());
        let scene = held(&root, SCENE);
        assert!(
            scene.contains("\nid = "),
            "the scaffold writes ids:\n{scene}"
        );
        let prefab = held(&root, PREFAB);
        let materials = held(&root, "content/materials/title.materials.toml");
        assert!(materials.contains("format = 1") && !materials.contains("content_layer"));
        assert!(scene.contains("prefab = \"content/prefabs/title.prefab.toml\""));
        assert!(scene.contains("file = \"content/meshes/grid.glb\""));
        assert!(
            prefab.contains("\nid = "),
            "the scaffold writes the prefab's ids:\n{prefab}"
        );
        for mesh in ["name", "mark", "grid"] {
            assert!(
                std::fs::read(root.join(format!("content/meshes/{mesh}.glb")))
                    .unwrap()
                    .starts_with(b"glTF")
            );
        }
        for mesh in ["name", "mark"] {
            assert!(prefab.contains(&format!("file = \"content/meshes/{mesh}.glb\"")));
        }
        assert!(!prefab.contains("content = ") && !prefab.contains("[content."));
        assert!(!read("src/lib.rs").contains(".mark("));
        assert!(prefab.contains("[text.version]\ntext = \"v{version}\""));
        assert!(prefab.contains("font = \"content/fonts/IBMPlexMono-Regular.ttf\""));
        assert!(read("content/fonts/IBMPlexMono-OFL.txt").contains("SIL Open Font License"));
        assert_eq!(project.select(), Some("title"));
        unbranded(&root, &written);
        let screen = project.screen().unwrap();
        assert_eq!(
            screen["desktop"].as_array().unwrap()[0].as_str(),
            Some("16:9")
        );
        assert_eq!(
            screen["deck"].as_array().unwrap()[0].as_str(),
            Some("16:10")
        );
        assert!(written.iter().all(|path| path.starts_with(&root)));
        let again = folder("scaffold-again");
        scaffold("my-game", &again, "9.8.7").unwrap();
        for relative in [DOCUMENT, "Cargo.toml"] {
            assert_eq!(
                std::fs::read(root.join(relative)).unwrap(),
                std::fs::read(again.join(relative)).unwrap(),
                "{relative} is the same bytes every time"
            );
        }
    }

    #[test]
    fn the_scaffold_shows_the_games_name_and_its_own_mark_when_given_one() {
        let art = folder("scaffold-art");
        let picture = art.join("mark.png");
        let file = std::fs::File::create(&picture).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), 4, 2);
        encoder.set_color(png::ColorType::Rgba);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[200; 32]).unwrap();
        writer.finish().unwrap();
        let root = folder("scaffold-marked");
        let options = Options {
            module: false,
            mark: Some(picture.clone()),
        };
        let written = scaffold_with("space-2-go", &root, "9.8.7", &options).unwrap();
        assert_eq!(
            std::fs::read(root.join(MARK)).unwrap(),
            std::fs::read(&picture).unwrap()
        );
        let lib = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();
        assert!(lib.contains(".mark(project.root().join(\"content/marks/mark.png\"))"));
        let prefab = held(&root, PREFAB);
        assert!(prefab.contains("content = \"mark\""));
        assert!(prefab.contains("[content.mark]\nimage = \"content/marks/mark.png\""));
        let materials = held(&root, "content/materials/title.materials.toml");
        assert!(materials.contains("[materials.face.content_layer]\nslot = 0"));
        let plain = folder("scaffold-unmarked");
        scaffold("space-2-go", &plain, "9.8.7").unwrap();
        let mark = std::fs::read(root.join("content/meshes/mark.glb")).unwrap();
        let bare = std::fs::read(plain.join("content/meshes/mark.glb")).unwrap();
        assert!(mark.len() > bare.len(), "the mark's plate carries a face");
        let name = std::fs::read(root.join("content/meshes/name.glb")).unwrap();
        let summary = pfx_glb::summary(&name).unwrap();
        let [low, high] = summary.nodes[0].bounds;
        assert!(
            (high[0] - low[0]) > 1.5 && (high[0] - low[0]) <= 2.41,
            "ten letters fill the title's width: {low:?} {high:?}"
        );
        unbranded(&root, &written);
        let bad = folder("scaffold-bad-mark");
        std::fs::write(art.join("mark.txt"), "not a picture").unwrap();
        let options = Options {
            module: false,
            mark: Some(art.join("mark.txt")),
        };
        let error = scaffold_with("my-game", &bad, "1.0.0", &options).unwrap_err();
        assert!(
            error.contains("mark.txt") && error.contains("PNG"),
            "{error}"
        );
        assert_eq!(std::fs::read_dir(&bad).unwrap().count(), 0);
    }

    #[test]
    fn the_scaffold_refuses_a_folder_that_is_missing_or_not_empty_and_a_bad_name() {
        let root = folder("refuse");
        std::fs::write(root.join("README.md"), "x").unwrap();
        let error = scaffold("my-game", &root, "1.0.0").unwrap_err();
        assert!(
            error.contains("not empty") && error.contains("README.md"),
            "{error}"
        );
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        assert!(
            scaffold("my-game", &root.join("missing"), "1.0.0")
                .unwrap_err()
                .contains("no such folder")
        );
        let clean = folder("refuse-name");
        assert!(scaffold("My Game", &clean, "1.0.0").is_err());
        assert_eq!(std::fs::read_dir(&clean).unwrap().count(), 0);
    }

    #[test]
    fn the_scaffold_with_a_module_adds_a_gameplay_module_its_world_and_its_build() {
        let root = folder("scaffold-module");
        let written = scaffold_with(
            "my-game",
            &root,
            "9.8.7",
            &crate::scaffold::Options {
                module: true,
                mark: None,
            },
        )
        .unwrap();
        for relative in ["module/Cargo.toml", "module/src/lib.rs", "wit/gameplay.wit"] {
            assert!(written.contains(&root.join(relative)), "{relative}");
        }
        let read = |relative: &str| std::fs::read_to_string(root.join(relative)).unwrap();
        let cargo = read("Cargo.toml");
        assert!(cargo.starts_with(
            "[workspace]\nmembers = [\".\", \"module\"]\ndefault-members = [\".\"]\n"
        ));
        assert_eq!(cargo.matches("[workspace]").count(), 1);
        assert!(cargo.contains(
            "pfx-game = { git = \"https://github.com/gmrdad82/pfx.git\", tag = \"v9.8.7\", features = [\"modules\"] }"
        ));
        let module = read("module/Cargo.toml");
        assert!(module.contains("name = \"my-game-module\""));
        assert!(module.contains("crate-type = [\"cdylib\"]"));
        let lib = read("src/lib.rs");
        assert!(lib.contains("pub struct MyGame {"));
        assert!(lib.contains("world: \"gameplay\""));
        assert!(lib.contains("fn modules(&mut self) -> Option<&mut dyn pfx_game::Reload>"));
        let world = read("wit/gameplay.wit");
        for export in ["export tick:", "export save:", "export restore:"] {
            assert!(world.contains(export), "{export}");
        }
        assert_eq!(
            module_path("my-game"),
            "target/wasm32-unknown-unknown/release/my_game_module.wasm"
        );
        let project = Project::open(&root).unwrap();
        assert_eq!(
            project.modules(),
            [crate::Module {
                id: "my_game_module".to_string(),
                path: root.join(module_path("my-game")),
            }]
        );
        let agents = read("AGENTS.md");
        assert!(
            agents.contains(
                "cargo build -p my-game-module --target wasm32-unknown-unknown --release"
            )
        );
        for text in [cargo, module, lib, world, agents, read("project.toml")] {
            assert!(!text.contains("{{"), "{text}");
        }
        let plain = folder("scaffold-plain");
        scaffold("my-game", &plain, "9.8.7").unwrap();
        assert!(!plain.join("module").exists() && !plain.join("wit").exists());
        assert!(Project::open(&plain).unwrap().modules().is_empty());
        let plain = std::fs::read_to_string(plain.join("Cargo.toml")).unwrap();
        assert!(plain.starts_with("[workspace]\n\n[package]\n"), "{plain}");
        assert_eq!(plain.matches("[workspace]").count(), 1);
    }
}
