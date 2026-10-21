use pfx_build::Registry;
use pfx_build::document::Document;
use pfx_load::scene::Scene;
use std::path::{Path, PathBuf};

const RED: [u8; 4] = [200, 40, 40, 255];
const BLUE: [u8; 4] = [30, 60, 220, 255];
const SCENE: &str = "[output]\nscene = \"street.scene.toml\"\n";

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("card_reference")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn png(rgba: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&rgba.repeat(4)).unwrap();
    }
    out
}

fn project(dir: &Path) {
    std::fs::write(
        dir.join("project.toml"),
        "format = 1\n\n[project]\nname = \"street\"\n",
    )
    .unwrap();
}

fn card(name: &str, image: &str, x: f64) -> String {
    format!(
        "\n[[object]]\nname = \"{name}\"\nkind = \"card\"\nsize = [1.0, 1.0]\nimage = \"{image}\"\nat = [{x:?}, 1.0, 0.0]\n"
    )
}

fn recipe_text(head: &str, cards: &str) -> String {
    format!(
        "recipe = 1\nname = \"street\"\n{head}\n[[object]]\nname = \"ground\"\nkind = \"box\"\nsize = [4.0, 0.1, 4.0]\n{cards}\n[[camera]]\nname = \"main\"\nposition = [0.0, 2.0, 5.0]\ntarget = [0.0, 0.5, 0.0]\nfov_y_deg = 40.0\n"
    )
}

fn street_with(dir: &Path, head: &str, cards: &str) -> PathBuf {
    std::fs::create_dir_all(dir.join("art")).unwrap();
    std::fs::write(dir.join("art/badge.png"), png(RED)).unwrap();
    let folder = dir.join("set/scenes/street");
    std::fs::create_dir_all(&folder).unwrap();
    let recipe = folder.join("street.toml");
    std::fs::write(&recipe, recipe_text(head, cards)).unwrap();
    recipe
}

fn street(dir: &Path, head: &str) -> PathBuf {
    street_with(dir, head, &card("sign", "../../../art/badge.png", 0.0))
}

fn coloured(scene: &Scene, card: &str, rgba: [u8; 4]) {
    let content = scene
        .contents
        .get(card)
        .unwrap_or_else(|| panic!("no content for {card}"));
    assert_eq!((content.width, content.height), (2, 2));
    assert_eq!(*content.rgba, rgba.repeat(4), "{card}");
}

fn red(scene: &Scene) {
    coloured(scene, "sign", RED);
}

fn built(recipe: &Path) -> (pfx_recipe::Recipe, pfx_build::Built) {
    let loaded = pfx_recipe::load(recipe).unwrap();
    let built = pfx_build::build(&loaded.recipe, recipe.parent().unwrap()).unwrap();
    (loaded.recipe, built)
}

#[test]
fn a_card_image_outside_the_recipes_folder_reads_from_its_pack_and_after_a_rebuild() {
    for (name, marked, head) in [
        ("pack", false, SCENE),
        ("pack-marked", true, SCENE),
        ("pack-bare", false, ""),
    ] {
        let dir = scratch(name);
        if marked {
            project(&dir);
        }
        let recipe = street(&dir, head);
        let out = recipe.with_extension("pitofx");
        let doc = Document::pack_for(&recipe, &Registry::new(), &out).unwrap();
        assert!(doc.files.contains_key(Path::new("art/badge.png")), "{name}");
        assert_eq!(doc.recipe_name, "set/scenes/street/street.toml");
        assert_eq!(doc.outside, ["../../../art/badge.png"]);
        doc.write(&out).unwrap();
        red(&Scene::open(&out).unwrap());
        let mut opened = Document::open(&out).unwrap();
        assert!(opened.fresh() && !opened.edited(), "{name}");
        opened.rebuild(&out, &Registry::new()).unwrap();
        opened.write(&out).unwrap();
        red(&Scene::open(&out).unwrap());
        assert_eq!(std::fs::read(dir.join("art/badge.png")).unwrap(), png(RED));
    }
}

#[test]
fn a_pack_shows_the_card_image_it_holds_not_the_file_changed_after() {
    let dir = scratch("held");
    let recipe = street(&dir, SCENE);
    let out = recipe.with_extension("pitofx");
    Document::pack_for(&recipe, &Registry::new(), &out)
        .unwrap()
        .write(&out)
        .unwrap();
    std::fs::write(dir.join("art/badge.png"), png(BLUE)).unwrap();
    red(&Scene::open(&out).unwrap());
    std::fs::remove_file(dir.join("art/badge.png")).unwrap();
    red(&Scene::open(&out).unwrap());
}

#[test]
fn a_scene_written_out_copies_a_card_image_from_outside_into_its_own_folder() {
    let dir = scratch("out");
    let recipe = street(&dir, SCENE);
    let (recipe_read, built) = built(&recipe);
    let out = dir.join("renders/street");
    let written =
        pfx_build::write(&built, &recipe_read, recipe.parent().unwrap(), Some(&out)).unwrap();
    assert!(
        written.iter().all(|path| path.starts_with(&out)),
        "{written:?}"
    );
    let kept = out.join("assets/images/0/badge.png");
    assert!(written.contains(&kept), "{written:?}");
    assert_eq!(std::fs::read(&kept).unwrap(), png(RED));
    assert_eq!(std::fs::read(dir.join("art/badge.png")).unwrap(), png(RED));
    let text = std::fs::read_to_string(out.join("street.scene.toml")).unwrap();
    assert!(
        text.contains("image = \"assets/images/0/badge.png\""),
        "{text}"
    );
    red(&Scene::open(out.join("street.scene.toml")).unwrap());
}

#[test]
fn two_outside_images_of_one_name_keep_two_copies_and_a_shared_one_keeps_one() {
    let dir = scratch("two");
    std::fs::create_dir_all(dir.join("other")).unwrap();
    std::fs::write(dir.join("other/badge.png"), png(BLUE)).unwrap();
    let cards = [
        card("sign", "../../../art/badge.png", 0.0),
        card("plate", "../../../other/badge.png", 1.5),
        card("twin", "../../../art/badge.png", -1.5),
    ]
    .concat();
    let recipe = street_with(&dir, SCENE, &cards);
    let (recipe_read, built) = built(&recipe);
    let out = dir.join("renders/street");
    let written =
        pfx_build::write(&built, &recipe_read, recipe.parent().unwrap(), Some(&out)).unwrap();
    let images: Vec<&PathBuf> = written
        .iter()
        .filter(|path| path.starts_with(out.join("assets/images")))
        .collect();
    assert_eq!(images.len(), 2, "{written:?}");
    let scene = Scene::open(out.join("street.scene.toml")).unwrap();
    coloured(&scene, "sign", RED);
    coloured(&scene, "plate", BLUE);
    coloured(&scene, "twin", RED);
}

#[test]
fn a_missing_card_image_outside_the_recipes_folder_is_refused_naming_the_card() {
    let dir = scratch("missing");
    let recipe = street_with(&dir, SCENE, &card("sign", "../../../art/gone.png", 0.0));
    let (recipe_read, built) = built(&recipe);
    assert!(
        built
            .notes
            .iter()
            .any(|note| note.contains("object[sign].image")),
        "{:?}",
        built.notes
    );
    let out = dir.join("renders/street");
    let error =
        pfx_build::write(&built, &recipe_read, recipe.parent().unwrap(), Some(&out)).unwrap_err();
    assert_eq!(error.path, "object[sign].image");
    assert!(!out.exists());
    let packed = Document::pack_for(&recipe, &Registry::new(), &recipe.with_extension("pitofx"));
    let error = packed.err().expect("the pack is refused");
    assert!(error.contains("object[sign].image"), "{error}");
}

#[test]
fn a_scene_is_never_written_outside_its_declared_project_root() {
    let dir = scratch("refused");
    let recipe = street(&dir, &format!("project_root = \".\"\n\n{SCENE}"));
    let base = recipe.parent().unwrap().to_path_buf();
    project(&base);
    let (recipe_read, built) = built(&recipe);
    let away = dir.join("renders/street");
    let error = pfx_build::write(&built, &recipe_read, &base, Some(&away)).unwrap_err();
    assert_eq!(error.path, "project_root");
    assert!(!away.exists());
    let written = pfx_build::write(&built, &recipe_read, &base, None).unwrap();
    assert!(
        written.iter().all(|path| path.starts_with(&base)),
        "{written:?}"
    );
    assert!(base.join("assets/images/0/badge.png").is_file());
    red(&Scene::open(base.join("street.scene.toml")).unwrap());
}
