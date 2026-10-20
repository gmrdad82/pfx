use pfx_build::{Built, build};
use pfx_recipe::parse;
use std::path::Path;

const HEAD: &str = r#"
points = [
  [-0.62, -0.85, -0.7], [0.0, -0.85, -0.7], [0.62, -0.85, -0.7], [-0.75, 0.0, -0.7],
  [0.0, 0.0, -0.7], [0.75, 0.0, -0.7], [-0.75, 0.95, -0.7], [0.0, 0.95, -0.7],
  [0.75, 0.95, -0.7], [-0.62, -0.85, 0.0], [0.0, -0.85, 0.0], [0.62, -0.85, 0.0],
  [-0.75, 0.0, 0.0], [0.75, 0.0, 0.0], [-0.75, 0.95, 0.0], [0.0, 0.95, 0.0],
  [0.75, 0.95, 0.0], [-0.62, -0.85, 0.82], [0.0, -0.85, 0.82], [0.62, -0.85, 0.82],
  [-0.75, 0.0, 0.7], [0.0, 0.0, 0.88], [0.75, 0.0, 0.7], [-0.75, 0.95, 0.7],
  [0.0, 0.95, 0.7], [0.75, 0.95, 0.7],
]
faces = [
  [9, 12, 3, 0], [17, 20, 12, 9], [12, 14, 6, 3], [20, 23, 14, 12], [2, 5, 13, 11], [11, 13, 22, 19],
  [5, 8, 16, 13], [13, 16, 25, 22], [1, 10, 9, 0], [2, 11, 10, 1], [10, 18, 17, 9], [11, 19, 18, 10],
  [6, 14, 15, 7], [7, 15, 16, 8], [14, 23, 24, 15], [15, 24, 25, 16], [3, 4, 1, 0], [6, 7, 4, 3],
  [4, 5, 2, 1], [7, 8, 5, 4], [17, 18, 21, 20], [20, 21, 24, 23], [18, 19, 22, 21], [21, 22, 25, 24],
]
creases = [{ edge = [23, 24], sharpness = 2 }, { edge = [24, 25], sharpness = 2 }]
subdivide = 3
"#;

fn recipe(params: &str) -> String {
    format!(
        "recipe = 1\nname = \"t\"\n\n[[object]]\nname = \"body\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\n\n[[object]]\nname = \"head\"\nkind = \"generator\"\ngenerator = \"pfx/cage\"\nparent = \"body\"\nat = [0.0, 1.2, 0.0]\nmaterial = \"skin\"\n\n[object.params]\n{params}\n\n[materials.skin]\nbase = [0.8, 0.6, 0.5]\n"
    )
}

fn built(params: &str) -> Result<Built, String> {
    build(
        &parse(&recipe(params)).map_err(|e| e.to_string())?,
        Path::new("."),
    )
    .map_err(|e| e.to_string())
}

#[test]
fn a_head_cage_builds_a_closed_smooth_part_in_its_own_material() {
    let b = built(HEAD).unwrap();
    let part = b.parts.iter().find(|p| p.node.name == "head").unwrap();
    let m = &part.node.mesh;
    assert_eq!(m.triangle_count(), 24 * 64 * 2);
    assert!(m.slots.iter().all(|s| s.material == "skin"));
    let volume: f64 = m
        .indices
        .chunks_exact(3)
        .map(|t| {
            let p = [0, 1, 2].map(|k| m.point(t[k]));
            p[0].dot(p[1].cross(p[2]))
        })
        .sum::<f64>()
        / 6.0;
    assert!(volume > 2.0 && volume < 3.5, "{volume}");
    assert_eq!(built(HEAD).unwrap().parts.len(), b.parts.len());
}

#[test]
fn a_broken_cage_is_refused_naming_the_object_and_key() {
    let e = built(&HEAD.replace("[9, 12, 3, 0]", "[9, 12, 3, 99]")).unwrap_err();
    assert!(
        e.contains("params.faces[0]: point 99 is out of range; the cage has 26 points"),
        "{e}"
    );
    let e = built(&HEAD.replace("subdivide = 3", "subdivide = 5")).unwrap_err();
    assert!(
        e.contains("params.subdivide: expected a whole number from 0 to 4"),
        "{e}"
    );
    let e = built(&format!("{HEAD}\nsmooth_by = 1")).unwrap_err();
    assert!(e.contains("params.smooth_by: unknown param"), "{e}");
    assert!(!e.contains('\n'));
}
