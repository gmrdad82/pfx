mod common;

use glam::DMat4;
use pfx_build::library;
use pfx_build::{Built, build};
use pfx_glb::{Node, Order};
use pfx_mesh::{Mesh, bounds, rounded_box};
use pfx_recipe::parse;
use std::path::Path;

fn lump(dir: &Path) {
    let mut whole = rounded_box([2.0, 1.2, 4.0], 0.15, 4);
    whole.set_material("clay");
    let mut knob = rounded_box([0.4, 0.4, 0.4], 0.0, 1);
    pfx_mesh::transform(&mut knob, DMat4::from_translation([0.0, 1.0, 0.0].into()));
    knob.set_material("clay");
    whole.append(&knob);
    let node = Node {
        name: "lump".into(),
        mesh: whole,
        transform: DMat4::IDENTITY,
        keep_transform: false,
        targets: None,
    };
    std::fs::write(
        dir.join("lump.glb"),
        pfx_glb::write(&[node], Order::File, "t"),
    )
    .unwrap();
}

fn built(dir: &Path, object: &str) -> Built {
    let text = format!("recipe = 1\nname = \"t\"\n\n[[object]]\nname = \"prop\"\n{object}");
    build(&parse(&text).unwrap(), dir).unwrap()
}

fn failure(dir: &Path, object: &str) -> String {
    let text = format!("recipe = 1\nname = \"t\"\n\n[[object]]\nname = \"prop\"\n{object}");
    build(&parse(&text).unwrap(), dir).unwrap_err().to_string()
}

fn refused(object: &str) -> String {
    let text = format!("recipe = 1\nname = \"t\"\n\n[[object]]\nname = \"prop\"\n{object}");
    parse(&text).unwrap_err().to_string()
}

fn names(b: &Built) -> Vec<&str> {
    b.parts.iter().map(|p| p.node.name.as_str()).collect()
}

fn mesh<'a>(b: &'a Built, name: &str) -> &'a Mesh {
    &b.parts
        .iter()
        .find(|p| p.node.name == name)
        .unwrap()
        .node
        .mesh
}

#[test]
fn split_turns_a_baked_part_into_its_own_node_to_drop_or_repaint() {
    let dir = common::scratch("edit-split");
    lump(&dir);
    let whole = built(&dir, "kind = \"mesh\"\npath = \"lump.glb\"\n");
    assert_eq!(names(&whole), ["prop"]);
    let split = built(&dir, "kind = \"mesh\"\npath = \"lump.glb\"\nsplit = true\n");
    assert_eq!(names(&split), ["prop.1", "prop.2"]);
    let triangles = |b: &Built| {
        b.parts
            .iter()
            .map(|p| p.node.mesh.triangle_count())
            .sum::<usize>()
    };
    assert_eq!(triangles(&split), triangles(&whole));
    assert_eq!(mesh(&split, "prop.2").triangle_count(), 12);
    assert!((bounds(mesh(&split, "prop.2")).unwrap()[1][1] - 1.2).abs() < 1e-6);
    let dropped = built(
        &dir,
        "kind = \"mesh\"\npath = \"lump.glb\"\nsplit = true\npieces = { \"/#2\" = { hidden = true } }\n",
    );
    assert_eq!(names(&dropped), ["prop.1"]);
    let painted = built(
        &dir,
        "kind = \"mesh\"\npath = \"lump.glb\"\nmaterial = \"stone\"\nsplit = true\npieces = { \"/#2\" = { material = \"brass\" } }\n",
    );
    assert_eq!(mesh(&painted, "prop.1").slots[0].material, "stone");
    assert_eq!(mesh(&painted, "prop.2").slots[0].material, "brass");
    assert_eq!(
        failure(
            &dir,
            "kind = \"mesh\"\npath = \"lump.glb\"\nsplit = true\npieces = { \"/#3\" = { hidden = true } }\n"
        ),
        "object[prop].pieces./#3: the object builds no piece `/#3`, expected one of: /#1, /#2"
    );
}

#[test]
fn a_region_on_an_imported_mesh_hides_or_recolours_the_faces_inside_its_curve() {
    let dir = common::scratch("edit-regions");
    lump(&dir);
    let region = "kind = \"mesh\"\npath = \"lump.glb\"\nsplit = true\n\n[[object.regions]]\nname = \"label\"\nview = \"side\"\nmaterial = \"decal\"\ncurve = [[-1.0, -0.3], [1.0, -0.3], [1.0, 0.3], [-1.0, 0.3]]\n";
    let b = built(&dir, region);
    assert_eq!(names(&b), ["prop.1", "prop.1.label", "prop.2"]);
    let label = mesh(&b, "prop.1.label");
    assert!(label.slots.iter().all(|s| s.material == "decal"));
    assert!(
        label
            .positions
            .iter()
            .all(|p| (p[0].abs() - 1.0).abs() < 1e-6)
    );
    let hidden = built(
        &dir,
        &format!("{region}\n[object.pieces]\n\"/#1/label\" = {{ hidden = true }}\n"),
    );
    assert_eq!(names(&hidden), ["prop.1", "prop.2"]);
    assert_eq!(mesh(&hidden, "prop.1"), mesh(&b, "prop.1"));
    assert_eq!(
        failure(
            &dir,
            "kind = \"mesh\"\npath = \"lump.glb\"\n\n[[object.regions]]\nname = \"label\"\nview = \"aside\"\ncurve = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]\n"
        ),
        "object[prop].regions.0.view: expected one of: side, top, front, rear"
    );
}

#[test]
fn a_library_object_splits_and_carves_its_nodes_too() {
    let dir = common::scratch("edit-library");
    lump(&dir);
    let text = "recipe = 1\nname = \"t\"\n\n[[object]]\nname = \"prop\"\nkind = \"mesh\"\npath = \"lump.glb\"\n";
    let r = parse(text).unwrap();
    let b = build(&r, &dir).unwrap();
    let exported = library::export(&r, &b, "prop", "prop/lump").unwrap();
    let root = dir.join("library");
    let folder = root.join("prop/lump");
    std::fs::create_dir_all(&folder).unwrap();
    library::Stored {
        model: exported.glb.clone(),
        sidecar: exported.sidecar.text(),
        thumb: None,
    }
    .write(&folder)
    .unwrap();
    library::write_index(&root).unwrap();
    let b = built(
        &dir,
        "kind = \"library\"\nobject = \"prop/lump\"\nsplit = true\npieces = { \"body/#2\" = { material = \"brass\" } }\n",
    );
    assert_eq!(names(&b), ["prop.body.1", "prop.body.2"]);
    assert_eq!(mesh(&b, "prop.body.2").slots[0].material, "brass");
}

#[test]
fn a_skinned_mesh_keeps_its_skin_through_piece_edits_and_refuses_a_carve() {
    let dir = common::scratch("edit-skinned");
    std::fs::write(dir.join("column.glb"), pfx_load::fixture::skinned_column(3)).unwrap();
    let head = "recipe = 1\nname = \"t\"\n\n[timeline]\nfps = 10\nframes = 10\n\n[[object]]\nname = \"col\"\nkind = \"mesh\"\npath = \"column.glb\"\n";
    let painted = build(
        &parse(&format!(
            "{head}split = true\npieces = {{ column = {{ material = \"brass\" }} }}\n"
        ))
        .unwrap(),
        &dir,
    )
    .unwrap();
    assert!(painted.skinned());
    assert_eq!(mesh(&painted, "col").slots[0].material, "brass");
    let carved = build(
        &parse(&format!(
            "{head}\n[[object.regions]]\nname = \"band\"\nview = \"front\"\ncurve = [[-1.0, 0.2], [1.0, 0.2], [1.0, 0.4], [-1.0, 0.4]]\n"
        ))
        .unwrap(),
        &dir,
    )
    .unwrap_err();
    assert_eq!(
        carved.to_string(),
        "object[col].regions: the mesh is skinned, and carving would leave its skin behind; `pieces` works on it"
    );
}

#[test]
fn one_pieces_key_reaches_file_nodes_and_the_pieces_cut_from_them() {
    let dir = common::scratch("edit-unified");
    lump(&dir);
    let painted = built(
        &dir,
        "kind = \"mesh\"\npath = \"lump.glb\"\nsplit = true\npieces = { lump = { material = \"stone\" }, \"/#2\" = { material = \"brass\" } }\n",
    );
    assert_eq!(mesh(&painted, "prop.1").slots[0].material, "stone");
    assert_eq!(mesh(&painted, "prop.2").slots[0].material, "brass");
    assert_eq!(
        refused(
            "kind = \"mesh\"\npath = \"lump.glb\"\nsplit = true\npieces = { \"lump/#2\" = { hidden = true } }\n"
        ),
        "line 9: object[prop].pieces.lump/#2: a mesh object merges the file's nodes into one piece, so a piece cut from it is `/#<k>` or `/<region>`, without a node"
    );
    let text = "recipe = 1\nname = \"t\"\n\n[[object]]\nname = \"prop\"\nkind = \"mesh\"\npath = \"lump.glb\"\n";
    let r = parse(text).unwrap();
    let b = build(&r, &dir).unwrap();
    let exported = library::export(&r, &b, "prop", "prop/lump").unwrap();
    let root = dir.join("library");
    let folder = root.join("prop/lump");
    std::fs::create_dir_all(&folder).unwrap();
    library::Stored {
        model: exported.glb.clone(),
        sidecar: exported.sidecar.text(),
        thumb: None,
    }
    .write(&folder)
    .unwrap();
    library::write_index(&root).unwrap();
    let b = built(
        &dir,
        "kind = \"library\"\nobject = \"prop/lump\"\nsplit = true\npieces = { body = { hidden = true }, \"body/#2\" = { hidden = false } }\n",
    );
    let shown: Vec<(&str, bool)> = b
        .parts
        .iter()
        .map(|p| (p.node.name.as_str(), p.hidden))
        .collect();
    assert_eq!(shown, [("prop.body.1", true), ("prop.body.2", false)]);
    assert!(
        refused(
            "kind = \"library\"\nobject = \"prop/lump\"\nsplit = true\npieces = { \"/#2\" = { hidden = true } }\n"
        )
        .ends_with("a library object's pieces are cut from its nodes, so the key starts with the node, as in `<node>/#<k>` or `<node>/<region>`")
    );
}
