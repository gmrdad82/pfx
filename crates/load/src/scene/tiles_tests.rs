use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::super::{Scene, SceneEdit};
use super::*;

struct Level {
    root: PathBuf,
}

impl Level {
    fn new(name: &str) -> Level {
        let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/level");
        let root = std::path::absolute(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tmp/tiles-tests")
                .join(name),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        copy(&from, &root);
        Level { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.path(name)).unwrap()
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.path(name), text).unwrap();
    }

    fn open(&self) -> Result<Scene, SceneError> {
        Scene::open(self.path("level.scene.toml"))
    }
}

impl Drop for Level {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn strip() -> Tiles {
    toml::from_str(
        "name = \"ground\"\norigin = [-3.5, 0.0, -2.0]\npalette = { \"#\" = \"b.prefab.toml\", \"p\" = \"p.prefab.toml\" }\nrows = [\"....p...\", \"##..####\"]\n",
    )
    .unwrap()
}

#[test]
fn a_cell_is_found_from_a_point_and_drawn_around_its_position() {
    let layer = strip();
    assert_eq!(layer.position([4, 1]), [0.5, 1.0, -2.0]);
    assert_eq!(cell_at(&layer, [0.4, 1.3, -2.0]), [4, 1]);
    assert_eq!(cell_at(&layer, [-4.1, -0.2, 5.0]), [-1, 0]);
    assert_eq!(
        corners(&layer, [0, 0]),
        [
            [-4.0, -0.5, -2.0],
            [-3.0, -0.5, -2.0],
            [-3.0, 0.5, -2.0],
            [-4.0, 0.5, -2.0]
        ]
    );
    assert_eq!(get(&layer, [4, 1]).as_deref(), Some("p"));
    assert_eq!(get(&layer, [2, 0]), None);
    assert_eq!(key(&layer, "p.prefab.toml").as_deref(), Some("p"));
    assert_eq!(free_key(&layer).as_deref(), Some("@"));
    let floor = Tiles {
        plane: Some(TilePlane::Xz),
        ..layer.clone()
    };
    assert_eq!(floor.position([0, 1]), [-3.5, 0.0, -3.0]);
    assert_eq!(cell_at(&floor, [-3.5, 4.0, -3.0]), [0, 1]);
}

#[test]
fn painting_rows_grows_up_right_and_left_down_by_moving_the_origin() {
    let layer = strip();
    let painted_rows = painted(
        &layer,
        &[([2, 0], Some("#")), ([4, 1], None), ([9, 9], None)],
    )
    .unwrap();
    assert_eq!(
        painted_rows,
        Painted::Rows {
            rows: vec!["........".into(), "###.####".into()],
            origin: [-3.5, 0.0, -2.0]
        }
    );
    let grown = painted(&layer, &[([-2, -1], Some("#")), ([5, 3], Some("p"))]).unwrap();
    let Painted::Rows { rows, origin } = grown else {
        panic!()
    };
    assert_eq!(origin, [-5.5, -1.0, -2.0]);
    let moved = Tiles {
        rows: Some(rows),
        origin,
        ..layer.clone()
    };
    assert_eq!(moved.cells().len(), 9);
    assert_eq!(moved.position([0, 0]), [-5.5, -1.0, -2.0]);
    assert_eq!(get(&moved, [2, 1]).as_deref(), Some("#"));
    assert_eq!(get(&moved, [7, 4]).as_deref(), Some("p"));
    assert_eq!(get(&moved, [0, 0]).as_deref(), Some("#"));
    assert!(painted(&layer, &[([0, 0], Some("##"))]).is_err());
    let sparse = Tiles {
        rows: None,
        cells: Some(vec![TileCell {
            at: [-3, 2],
            tile: "wall".into(),
        }]),
        ..layer
    };
    assert_eq!(
        painted(&sparse, &[([0, 0], Some("wall")), ([-3, 2], None)]).unwrap(),
        Painted::Cells(vec![TileCell {
            at: [0, 0],
            tile: "wall".into()
        }])
    );
}

#[test]
fn lines_and_rectangles_cover_their_cells() {
    assert_eq!(line([0, 0], [3, 0]), [[0, 0], [1, 0], [2, 0], [3, 0]]);
    assert_eq!(line([0, 0], [2, 2]), [[0, 0], [1, 1], [2, 2]]);
    assert_eq!(line([1, 1], [1, 1]), [[1, 1]]);
    assert_eq!(line([3, 1], [0, 0]).len(), 4);
    assert_eq!(rectangle([1, 1], [0, 0]), [[0, 0], [1, 0], [0, 1], [1, 1]]);
    assert_eq!(cell_key("ground", [-3, 4]), "ground[-3,4]");
    assert_eq!(
        cell_of("ground[-3,4]/block"),
        Some(("ground".to_string(), [-3, 4]))
    );
    assert_eq!(cell_of("crate"), None);
}

#[test]
fn a_tile_layer_loads_as_the_prefabs_objects_sharing_their_meshes() {
    let level = Level::new("load");
    let scene = level.open().unwrap();
    let names: Vec<&str> = scene.objects.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names[..2], ["floor", "ledge"]);
    assert!(names.contains(&"ground[0,0]/block"), "{names:?}");
    assert!(names.contains(&"ground[4,1]/foot"), "{names:?}");
    assert_eq!(names.len(), 2 + 6 + 2);
    let block = scene.object("ground[0,0]/block").unwrap();
    assert_eq!(block.mesh, "prefabs/block.prefab.toml:block");
    assert_eq!(block.at, [-3.5, 0.5, -2.0]);
    assert_eq!(block.model[3], [-3.5, 0.5, -2.0, 1.0]);
    assert_eq!(block.parent, None);
    assert_eq!(block.material.as_deref(), Some("clay"));
    let shaft = scene.object("ground[4,1]/shaft").unwrap();
    assert_eq!(shaft.parent.as_deref(), Some("ground[4,1]/foot"));
    assert_eq!(shaft.at, [0.0, 2.5, 0.0]);
    assert!((shaft.model[3][1] - (1.0 + 0.1 + 2.5 * 0.2)).abs() < 1e-5);
    let ids: BTreeSet<u32> = scene.objects.iter().map(|o| o.id).collect();
    assert_eq!(ids.len(), scene.objects.len());
    assert_eq!(scene.objects[2].id, 3);
    let tile_meshes = scene
        .meshes
        .keys()
        .filter(|key| key.contains(".prefab.toml:"))
        .count();
    assert_eq!(tile_meshes, 2);
    assert!(
        scene
            .files
            .keys()
            .any(|file| file.ends_with("prefabs/post.prefab.toml"))
    );
    let draws = scene.draws();
    assert_eq!(draws.items.len(), 2 + 6 + 2);
    let geometry: BTreeSet<[u8; 32]> = draws.items.iter().map(|draw| draw.geometry.hash).collect();
    assert_eq!(geometry.len(), 1);
}

#[test]
fn a_tile_prefab_that_does_not_fit_is_refused_at_its_palette_key() {
    let level = Level::new("refused");
    let post = level.read("prefabs/post.prefab.toml");
    let lit = post.clone()
        + "\n[[light]]\nname = \"lamp\"\nposition = [0.0, 1.0, 0.0]\nintensity = 1.0\nid = \"7m0p3x8z4q\"\n";
    level.write("prefabs/post.prefab.toml", &lit);
    let error = level.open().unwrap_err();
    assert!(error.message.contains("it has lights"), "{error}");
    assert!(error.file.ends_with("level.scene.toml"), "{error}");
    assert!(error.line.is_some(), "{error}");
}

#[test]
fn ten_thousand_cells_load_as_ten_thousand_objects_on_one_mesh() {
    let level = Level::new("many");
    let row = "#".repeat(100);
    let rows: Vec<String> = (0..100).map(|_| format!("  \"{row}\",")).collect();
    let scene = level.read("level.scene.toml");
    let start = scene.find("# a side-view").unwrap();
    level.write(
        "level.scene.toml",
        &format!(
            "{}[[tiles]]\nname = \"floor\"\nplane = \"xz\"\ncell = 0.5\norigin = [-25.0, 0.0, 25.0]\npalette = {{ \"#\" = \"prefabs/block.prefab.toml\" }}\nrows = [\n{}\n]\n",
            &scene[..start],
            rows.join("\n")
        ),
    );
    let started = std::time::Instant::now();
    let scene = level.open().unwrap();
    eprintln!(
        "10,000 cells load in {:.1} ms",
        started.elapsed().as_secs_f64() * 1e3
    );
    assert_eq!(scene.objects.len(), 2 + 10_000);
    let draws = scene.draws();
    let geometry: BTreeSet<[u8; 32]> = draws.items.iter().map(|draw| draw.geometry.hash).collect();
    assert_eq!(geometry.len(), 1);
    assert_eq!(
        scene.object("floor[99,99]/block").unwrap().at,
        [24.5, 0.5, -24.5]
    );
}

#[test]
fn an_edit_keeps_the_tile_layer_and_a_tiles_edit_reloads_it() {
    let level = Level::new("edit");
    let mut edit = SceneEdit::open(level.path("level.scene.toml")).unwrap();
    assert_eq!(edit.scene().objects.len(), 10);
    edit.set_at("ledge", [2.0, 0.5, -1.5]).unwrap();
    assert_eq!(edit.scene().objects.len(), 10);
    let target = super::super::Target::Tiles("ground".into());
    edit.set(&target, &["rows", "1"], "########").unwrap();
    assert_eq!(edit.scene().objects.len(), 12);
    assert!(level.read("level.scene.toml").contains("  \"########\",\n"));
}

#[test]
fn an_asset_has_an_extent() {
    let level = Level::new("extent");
    let block = extent(&level.root, "prefabs/block.prefab.toml").unwrap();
    assert_eq!(block, [[-0.5, 0.0, -0.5], [0.5, 1.0, 0.5]]);
    let mesh = extent(&level.root, "meshes/block.gltf").unwrap();
    assert_eq!(mesh, [[-0.5; 3], [0.5; 3]]);
    assert!(extent(&level.root, "prefabs/gone.prefab.toml").is_err());
}

#[test]
fn a_tile_prefabs_body_and_character_are_placed_per_cell() {
    let level = Level::new("bodies");
    let block = level.read("prefabs/block.prefab.toml");
    level.write(
        "prefabs/block.prefab.toml",
        &(block + "\n[object.body]\nfriction = 0.5\n"),
    );
    let post = level.read("prefabs/post.prefab.toml");
    let foot = "id = \"sh40v4bdjz\"\n";
    assert!(post.contains(foot));
    level.write(
        "prefabs/post.prefab.toml",
        &post.replacen(
            foot,
            &format!("{foot}\n[object.character]\nradius = 0.05\nheight = 0.2\n"),
            1,
        ),
    );
    let scene = level.open().unwrap();
    let bodies: Vec<&String> = scene.bodies.keys().collect();
    assert_eq!(bodies.len(), 6, "{bodies:?}");
    assert!(scene.bodies.contains_key("ground[0,0]/block"));
    assert_eq!(scene.bodies["ground[0,0]/block"].friction, 0.5);
    assert!(
        scene.characters.contains_key("ground[4,1]/foot"),
        "{:?}",
        scene.characters.keys().collect::<Vec<_>>()
    );
}

#[test]
fn a_tile_prefab_with_a_sprite_is_refused_at_its_palette_key() {
    let level = Level::new("sprite");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scenes/screen.png"),
        level.path("atlas.png"),
    )
    .unwrap();
    let block = level.read("prefabs/block.prefab.toml");
    level.write(
        "prefabs/block.prefab.toml",
        &(block + "\n[object.animation]\natlas = \"atlas.png\"\ngrid = [2, 1]\n"),
    );
    let error = level.open().unwrap_err();
    assert!(error.message.contains("is a sprite"), "{error}");
    assert!(error.file.ends_with("level.scene.toml"), "{error}");
}
