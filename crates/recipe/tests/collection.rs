use pfx_recipe::{Pick, parse, validate};

const YARD: &str = r#"recipe = 1
name = "yard"

[[object]]
name = "ground"
kind = "plane"
size = [4.0, 4.0]

[[object]]
name = "shed"
kind = "box"
size = [1.0, 1.0, 1.0]

[[object]]
name = "post_a"
kind = "box"
size = [0.1, 1.0, 0.1]

[[object]]
name = "post_b"
kind = "box"
size = [0.1, 1.0, 0.1]

[[object]]
name = "pebble"
kind = "sphere"
radius = 0.05
hidden = true

[[scatter]]
name = "pebbles"
object = "pebble"
on = "ground"
spacing = 0.5

[[collection]]
name = "fence"
objects = ["post_*"]

[[collection]]
name = "loose"
objects = ["pebbles"]

[[camera]]
name = "main"
position = [0.0, 3.0, 3.0]
target = [0.0, 0.0, 0.0]
fov_y_deg = 40.0
"#;

fn hidden(recipe: &pfx_recipe::Recipe) -> Vec<&str> {
    recipe
        .objects
        .iter()
        .filter(|o| o.hidden)
        .map(|o| o.name.as_str())
        .collect()
}

#[test]
fn a_collection_hides_its_objects_or_renders_alone() {
    let recipe = parse(YARD).unwrap();
    assert!(validate(&recipe).is_empty(), "{:?}", validate(&recipe));
    assert_eq!(recipe.collected("post_a"), ["fence"]);
    assert_eq!(hidden(&recipe), ["pebble"]);
    let off = parse(&YARD.replace(
        "objects = [\"post_*\"]",
        "objects = [\"post_*\"]\nhidden = true",
    ))
    .unwrap();
    assert_eq!(hidden(&off), ["post_a", "post_b", "pebble"]);
    let solo = parse(&YARD.replace(
        "objects = [\"post_*\"]",
        "objects = [\"post_*\"]\nsolo = true",
    ))
    .unwrap();
    assert_eq!(hidden(&solo), ["ground", "shed", "pebble"]);
    assert!(solo.collection_hides("pebbles"));
}

#[test]
fn only_takes_a_collection_or_a_scatter_and_drops_what_they_need_not() {
    let recipe = parse(YARD).unwrap();
    let pick = |globs: &[&str]| {
        Pick {
            stage: None,
            only: globs.iter().map(|g| g.to_string()).collect(),
        }
        .apply(&recipe)
    };
    let names = |r: &pfx_recipe::Recipe| -> Vec<String> {
        r.objects.iter().map(|o| o.name.clone()).collect()
    };
    let fence = pick(&["fence"]).unwrap();
    assert_eq!(names(&fence), ["post_a", "post_b"]);
    assert!(fence.scatters.is_empty());
    assert_eq!(fence.collections.len(), 1);
    assert!(validate(&fence).is_empty(), "{:?}", validate(&fence));
    let loose = pick(&["loose"]).unwrap();
    assert_eq!(names(&loose), ["ground", "pebble"]);
    assert_eq!(loose.scatters.len(), 1);
    let shed = pick(&["shed"]).unwrap();
    assert!(shed.scatters.is_empty() && shed.collections.is_empty());
    assert!(validate(&shed).is_empty(), "{:?}", validate(&shed));
    let error = pick(&["nothing"]).unwrap_err();
    assert!(
        error.text.contains("a collection or a scatter"),
        "{}",
        error.text
    );
}

#[test]
fn a_scatter_names_two_objects_and_a_spacing_or_a_density() {
    let bad = YARD
        .replace("on = \"ground\"", "on = \"lawn\"")
        .replace("spacing = 0.5", "scale_min = 2.0\nscale_max = 1.0");
    let texts: Vec<String> = validate(&parse(&bad).unwrap())
        .into_iter()
        .map(|p| format!("{}: {}", p.path, p.text))
        .collect();
    for wanted in [
        "scatter[pebbles].on: no object named `lawn`",
        "scatter[pebbles].spacing: expected `spacing`",
        "scatter[pebbles].scale_max: expected a scale at or above `scale_min`",
    ] {
        assert!(
            texts.iter().any(|t| t.starts_with(wanted)),
            "{wanted} in {texts:?}"
        );
    }
    let typo = YARD.replace("[\"post_*\"]", "[\"pots_*\"]");
    let texts: Vec<String> = validate(&parse(&typo).unwrap())
        .into_iter()
        .map(|p| p.text)
        .collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("`pots_*` matches no object or scatter")),
        "{texts:?}"
    );
}
