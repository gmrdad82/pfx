mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::{Folder, ROOM, errors, show};
use pfx_scene::types::{CellSize, TileCell, TilePlane, Tiles};
use pfx_scene::{Diagnostic, Id, Kind, SceneEdit, Target, check, code};

const BRICK: &str = "format = 1\n\n[mesh.brick]\nid = \"br1ckmesh1\"\nfile = \"block.gltf\"\n\n[[object]]\nid = \"br1ck00001\"\nname = \"brick\"\nmesh = \"brick\"\n\n[object.body]\nkind = \"fixed\"\n";

const SPIKE: &str = "format = 1\n\n[mesh.spike]\nid = \"sp1kemesh1\"\nfile = \"pillar.gltf\"\n\n[[object]]\nid = \"sp1ke00001\"\nname = \"spike\"\nmesh = \"spike\"\n";

const LAYERS: &str = "\n[[object]]\nid = \"sp1kep1ace\"\nname = \"spare spike\"\nprefab = \"tiles/spike.prefab.toml\"\nhidden = true\n\n[[tiles]]\nid = \"t11e000001\"\nname = \"ground\"\ncell = 0.5\norigin = [-4.0, 0.0, 0.0]\npalette = { \"#\" = \"tiles/brick.prefab.toml\", \"^\" = \"sp1kep1ace\" }\nrows = [\n  \"....^...\",\n  \"########\",\n]\n\n[[tiles]]\nid = \"t11e000002\"\nname = \"floor plan\"\ncell = [2.0, 3.0]\nplane = \"xz\"\npalette = { wall = \"tiles/brick.prefab.toml\" }\ncells = [\n  { at = [0, 0], tile = \"wall\" },\n  { at = [-1, 2], tile = \"wall\" },\n]\n";

fn tiled(name: &str, layers: &str) -> Folder {
    let folder = Folder::room(name);
    folder.write("tiles/brick.prefab.toml", BRICK);
    folder.write("tiles/spike.prefab.toml", SPIKE);
    folder.write("room.scene.toml", &format!("{ROOM}{layers}"));
    folder
}

fn one(diagnostics: &[Diagnostic]) -> &Diagnostic {
    let found = errors(diagnostics);
    assert_eq!(found.len(), 1, "{}", show(diagnostics));
    found[0]
}

fn place_of(folder: &Folder, file: &str, needle: &str) -> (u32, u32) {
    let text = folder.read(file);
    let at = text.find(needle).unwrap();
    let line = text[..at].matches('\n').count() as u32 + 1;
    let start = text[..at].rfind('\n').map_or(0, |end| end + 1);
    (line, text[start..at].chars().count() as u32 + 1)
}

#[test]
fn tile_layers_read_resolve_and_check_clean() {
    let folder = tiled("tiles-clean", LAYERS);
    let diagnostics = folder.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    let scene = folder
        .project()
        .scene(&folder.path("room.scene.toml"))
        .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)));

    let keys: Vec<(&str, Option<&str>)> = scene
        .tiles
        .iter()
        .map(|entry| (entry.key.as_str(), entry.id.as_deref()))
        .collect();
    assert_eq!(
        keys,
        [
            ("ground", Some("t11e000001")),
            ("floor plan", Some("t11e000002"))
        ]
    );

    let ground = &scene.tile_layer("ground").unwrap().value;
    let palette: Vec<(&str, &str)> = ground
        .palette
        .iter()
        .map(|(tile, prefab)| (tile.as_str(), prefab.as_str()))
        .collect();
    assert_eq!(
        palette,
        [
            ("#", "tiles/brick.prefab.toml"),
            ("^", "tiles/spike.prefab.toml")
        ]
    );
    let cells = ground.cells();
    assert_eq!(cells.len(), 9);
    assert_eq!(cells[0], ([4, 1], "^"));
    assert_eq!(cells[1], ([0, 0], "#"));
    assert_eq!(cells[8], ([7, 0], "#"));
    assert_eq!(ground.cell_sides(), [0.5, 0.5]);
    assert_eq!(ground.position([4, 1]), [-2.0, 0.5, 0.0]);

    let plan = &scene.tile_layer("floor plan").unwrap().value;
    assert_eq!(plan.plane, Some(TilePlane::Xz));
    assert_eq!(plan.cell, Some(CellSize::Sides([2.0, 3.0])));
    assert_eq!(plan.cells(), [([0, 0], "wall"), ([-1, 2], "wall")]);
    assert_eq!(plan.position([-1, 2]), [-2.0, 0.0, -6.0]);

    for prefab in ["tiles/brick.prefab.toml", "tiles/spike.prefab.toml"] {
        assert!(
            scene.files.iter().any(|file| file == Path::new(prefab)),
            "{:?}",
            scene.files
        );
    }
    let brick = folder
        .project()
        .scene(Path::new("tiles/brick.prefab.toml"))
        .unwrap();
    assert!(brick.object("brick").unwrap().value.body.is_some());
    assert!(scene.object("spare spike/spike").unwrap().value.hidden);
}

#[test]
fn a_plane_names_the_axes_cells_run_along() {
    let mut tiles = Tiles {
        id: None,
        name: "side".to_string(),
        cell: Some(CellSize::Square(2.0)),
        origin: [1.0, 0.0, 0.0],
        plane: Some(TilePlane::Yz),
        palette: BTreeMap::from([("#".to_string(), "a.prefab.toml".to_string())]),
        rows: Some(vec!["#.".to_string(), ".#".to_string()]),
        cells: None,
        authoring: None,
    };
    assert_eq!(tiles.cells(), [([0, 1], "#"), ([1, 0], "#")]);
    assert_eq!(tiles.position([1, 1]), [1.0, 2.0, -2.0]);
    tiles.plane = None;
    assert_eq!(tiles.position([1, 1]), [3.0, 2.0, 0.0]);
    tiles.plane = Some(TilePlane::Xz);
    assert_eq!(tiles.position([1, 1]), [3.0, 0.0, -2.0]);
    tiles.rows = None;
    tiles.cells = Some(vec![TileCell {
        at: [-3, 4],
        tile: "#".to_string(),
    }]);
    assert_eq!(tiles.cells(), [([-3, 4], "#")]);
}

#[test]
fn a_tile_no_palette_names_is_refused_at_its_character() {
    let layer = "\n[[tiles]]\nid = \"t11e000001\"\nname = \"ground\"\npalette = { \"#\" = \"tiles/brick.prefab.toml\" }\nrows = [\n  \"..x..x\",\n  \"#####é\",\n]\n";
    let folder = tiled("tiles-row", layer);
    let diagnostics = folder.check();
    let found = errors(&diagnostics);
    assert_eq!(found.len(), 2, "{}", show(&diagnostics));
    assert!(found.iter().all(|d| d.code == code::BAD_REFERENCE));
    let (line, column) = place_of(&folder, "room.scene.toml", "x..x");
    assert_eq!((found[0].line, found[0].column), (line, column));
    assert_eq!(found[0].end_column, column + 1);
    assert_eq!(found[0].key, "tiles.t11e000001.rows.0");
    assert!(
        found[0]
            .message
            .contains("tile x names no tile of its palette, which names #"),
        "{}",
        found[0]
    );
    let (line, column) = place_of(&folder, "room.scene.toml", "é");
    assert_eq!((found[1].line, found[1].column), (line, column));
    assert_eq!(found[1].key, "tiles.t11e000001.rows.1");

    let project = folder.project();
    let text = folder.read("room.scene.toml");
    let buffer = check(&text, &folder.path("room.scene.toml"), &project);
    assert_eq!(errors(&buffer).len(), 2, "{}", show(&buffer));

    let escaped = "\n[[tiles]]\nid = \"t11e000001\"\nname = \"ground\"\npalette = { \"#\" = \"tiles/brick.prefab.toml\" }\nrows = [\"\\u0023x\"]\n";
    let folder = tiled("tiles-row-escaped", escaped);
    let diagnostics = folder.check();
    let error = one(&diagnostics);
    assert_eq!(error.code, code::BAD_REFERENCE, "{error}");
    let (line, column) = place_of(&folder, "room.scene.toml", "\"\\u0023x\"");
    assert_eq!((error.line, error.column), (line, column));
}

#[test]
fn a_palette_names_prefabs_that_exist() {
    let cases = [
        (
            "palette = { \"#\" = \"tiles/missing.prefab.toml\" }\nrows = [\"#\"]\n",
            code::MISSING_FILE,
            "tiles.t11e000001.palette.#",
            "tiles/missing.prefab.toml\"",
            "does not exist",
        ),
        (
            "palette = { \"#\" = \"block.gltf\" }\nrows = [\"#\"]\n",
            code::BAD_PATH,
            "tiles.t11e000001.palette.#",
            "\"block.gltf\" }",
            "is not a .prefab.toml file",
        ),
        (
            "palette = { \"#\" = \"0bj000000f\" }\nrows = [\"#\"]\n",
            code::BAD_REFERENCE,
            "tiles.t11e000001.palette.#",
            "\"0bj000000f\" }",
            "which is no placement of this scene",
        ),
        (
            "palette = { \"#\" = \"zzzzzzzzzz\" }\nrows = [\"#\"]\n",
            code::BAD_REFERENCE,
            "tiles.t11e000001.palette.#",
            "\"zzzzzzzzzz\"",
            "which is no placement of this scene",
        ),
        (
            "palette = { wall = \"tiles/brick.prefab.toml\" }\ncells = [{ at = [0, 0], tile = \"door\" }]\n",
            code::BAD_REFERENCE,
            "tiles.t11e000001.cells.0.tile",
            "\"door\"",
            "tile door names no tile of its palette, which names wall",
        ),
    ];
    for (place, (keys, expected, key, needle, message)) in cases.into_iter().enumerate() {
        let layer = format!("\n[[tiles]]\nid = \"t11e000001\"\nname = \"ground\"\n{keys}");
        let folder = tiled(&format!("tiles-palette-{place}"), &layer);
        let diagnostics = folder.check();
        let error = one(&diagnostics);
        assert_eq!(error.code, expected, "case {place}: {error}");
        assert_eq!(error.key, key, "case {place}: {error}");
        assert!(error.message.contains(message), "case {place}: {error}");
        let (line, column) = place_of(&folder, "room.scene.toml", needle);
        assert_eq!(error.line, line, "case {place}: {error}");
        if needle.starts_with('"') {
            assert_eq!(error.column, column, "case {place}: {error}");
        }
        let project = folder.project();
        let text = folder.read("room.scene.toml");
        let buffer = check(&text, &folder.path("room.scene.toml"), &project);
        assert_eq!(errors(&buffer).len(), 1, "case {place}: {}", show(&buffer));
    }
}

#[test]
fn an_error_in_a_tile_prefab_is_reported_in_the_prefab() {
    let folder = tiled("tiles-prefab-error", LAYERS);
    folder.write(
        "tiles/brick.prefab.toml",
        &BRICK.replace("kind = \"fixed\"", "kind = \"stone\""),
    );
    let project = folder.project();
    let found = project.scene(&folder.path("room.scene.toml")).unwrap_err();
    let error = one(&found);
    assert_eq!(error.code, code::BAD_VALUE, "{error}");
    assert_eq!(error.file, Path::new("tiles/brick.prefab.toml"));
    let diagnostics = folder.check();
    assert_eq!(one(&diagnostics), error);
}

#[test]
fn a_tile_layer_takes_its_keys_with_their_ranges() {
    let cases = [
        (
            "cell = 0.0\n",
            code::BAD_VALUE,
            "tiles.t11e000001.cell",
            "sides are above 0",
        ),
        (
            "cell = [1.0, -1.0]\n",
            code::BAD_VALUE,
            "tiles.t11e000001.cell",
            "sides are above 0",
        ),
        (
            "cell = [1.0, 2.0, 3.0]\n",
            code::BAD_TYPE,
            "tiles.t11e000001.cell",
            "invalid length",
        ),
        (
            "plane = \"xw\"\n",
            code::BAD_VALUE,
            "tiles.t11e000001.plane",
            "unknown variant",
        ),
        (
            "palette = { \"#\" = \"tiles/brick.prefab.toml\" }\nrows = [\"#\"]\ncells = [{ at = [1, 0], tile = \"#\" }]\n",
            code::BAD_VALUE,
            "tiles.t11e000001.cells",
            "rows or a list of cells, not both",
        ),
        (
            "palette = { \"#\" = \"tiles/brick.prefab.toml\" }\ncells = [{ at = [1, 0], tile = \"#\" }, { at = [1, 0], tile = \"#\" }]\n",
            code::BAD_VALUE,
            "tiles.t11e000001.cells.1.at",
            "cell [1, 0] is set twice; cell 0 sets it already",
        ),
        (
            "palette = { \".\" = \"tiles/brick.prefab.toml\" }\n",
            code::BAD_NAME,
            "tiles.t11e000001.palette..",
            "marks an empty cell",
        ),
        (
            "palette = { \"a b\" = \"tiles/brick.prefab.toml\" }\n",
            code::BAD_NAME,
            "tiles.t11e000001.palette.a b",
            "holds a space",
        ),
        (
            "palette = { wall = \"tiles/brick.prefab.toml\" }\nrows = [\"..\"]\n",
            code::BAD_VALUE,
            "tiles.t11e000001.palette.wall",
            "longer than one character",
        ),
        (
            "cells = [{ at = [0.5, 0], tile = \"#\" }]\n",
            code::BAD_TYPE,
            "tiles.t11e000001.cells.0.at.0",
            "invalid type",
        ),
        (
            "rows = [\"#\"]\n",
            code::BAD_REFERENCE,
            "tiles.t11e000001.rows.0",
            "the layer's palette is empty",
        ),
        (
            "layer = 2\n",
            code::UNKNOWN_KEY,
            "tiles.t11e000001.layer",
            "unknown key",
        ),
    ];
    for (place, (keys, expected, key, message)) in cases.into_iter().enumerate() {
        let layer = format!("\n[[tiles]]\nid = \"t11e000001\"\nname = \"ground\"\n{keys}");
        let folder = tiled(&format!("tiles-keys-{place}"), &layer);
        let diagnostics = folder.check();
        assert_eq!(
            errors(&diagnostics).len(),
            1,
            "case {place}: {}",
            show(&diagnostics)
        );
        let error = one(&diagnostics);
        assert_eq!(error.code, expected, "case {place}: {error}");
        assert_eq!(error.key, key, "case {place}: {error}");
        assert!(error.message.contains(message), "case {place}: {error}");
    }

    let twice = "\n[[tiles]]\nid = \"t11e000001\"\nname = \"ground\"\n\n[[tiles]]\nid = \"t11e000001\"\nname = \"ground\"\n";
    let folder = tiled("tiles-twice", twice);
    let diagnostics = folder.check();
    let mut found: Vec<&str> = errors(&diagnostics).iter().map(|d| d.code).collect();
    found.sort();
    assert_eq!(
        found,
        [code::DUPLICATE_ID, code::DUPLICATE_NAME],
        "{}",
        show(&diagnostics)
    );
}

#[test]
fn a_tile_layer_belongs_to_a_scene_and_takes_an_id() {
    let folder = tiled("tiles-in-prefab", "");
    folder.write(
        "tiles/brick.prefab.toml",
        &format!("{BRICK}\n[[tiles]]\nname = \"inner\"\n"),
    );
    folder.write(
        "room.scene.toml",
        &format!("{ROOM}\n[[object]]\nid = \"br1ckp1ace\"\nname = \"brick\"\nprefab = \"tiles/brick.prefab.toml\"\n"),
    );
    let diagnostics = folder.check();
    let error = one(&diagnostics);
    assert_eq!(error.code, code::UNKNOWN_KEY, "{error}");
    assert_eq!(error.file, Path::new("tiles/brick.prefab.toml"));

    let folder = tiled(
        "tiles-no-id",
        "\n[[tiles]]\nname = \"ground\"\npalette = { \"#\" = \"tiles/brick.prefab.toml\" }\nrows = [\"##\"]\n",
    );
    let diagnostics = folder.check();
    assert_eq!(diagnostics.len(), 1, "{}", show(&diagnostics));
    assert_eq!(diagnostics[0].code, code::MISSING_ID);
    assert_eq!(diagnostics[0].key, "tiles.0");
    let project = folder.project();
    let group = project.fix(&folder.path("room.scene.toml")).unwrap();
    group.write().unwrap();
    let text = folder.read("room.scene.toml");
    assert!(text.contains("rows = [\"##\"]\nid = \""), "{text}");
    assert!(folder.check().is_empty());
}

#[test]
fn the_writer_adds_a_tile_layer_and_paints_its_rows() {
    let folder = tiled("tiles-edit", "");
    let mut edit = SceneEdit::open(folder.path("room.scene.toml")).unwrap();
    let id = Id::from_bits(0x711e);
    edit.add(
        Kind::Tiles,
        id,
        "ground",
        &[
            ("cell", 0.5f32.into()),
            ("plane", "xy".into()),
            ("rows", vec!["##".into()].into()),
        ],
        None,
    )
    .unwrap_err();
    edit.add(Kind::Tiles, id, "ground", &[("cell", 0.5f32.into())], None)
        .unwrap();
    edit.set(
        &Target::Tiles("ground".into()),
        &["palette", "#"],
        "tiles/brick.prefab.toml",
    )
    .unwrap();
    edit.set(
        &Target::Tiles("ground".into()),
        &["rows"],
        vec!["..".into(), "##".into()],
    )
    .unwrap();
    let text = folder.read("room.scene.toml");
    assert!(
        text.ends_with(&format!(
            "[[tiles]]\nid = \"{id}\"\nname = \"ground\"\ncell = 0.5\nrows = [\"..\", \"##\"]\n\n[tiles.palette]\n\"#\" = \"tiles/brick.prefab.toml\"\n"
        )),
        "{text}"
    );
    let scene = edit.scene();
    assert_eq!(
        scene.tile_layer("ground").unwrap().value.cells(),
        [([0, 0], "#"), ([1, 0], "#")]
    );
    let error = edit
        .set(&Target::Tiles("ground".into()), &["rows", "0"], "x.")
        .unwrap_err();
    assert_eq!(error.code, code::BAD_REFERENCE, "{error}");
    assert_eq!(folder.read("room.scene.toml"), text);
}
