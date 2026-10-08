mod common;

use egui::{Pos2, Rect, Vec2};
use glam::DVec3;
use pfx_editor_viewport::pick::{Extents, Picker, Selection};
use pfx_editor_viewport::{Eye, Lens, Ray};
use pfx_load::scene::Scene;

fn opened(name: &str) -> Scene {
    Scene::open(common::fixture(name)).unwrap()
}

fn down(x: f64, z: f64) -> Ray {
    Ray {
        origin: DVec3::new(x, 5.0, z),
        direction: DVec3::new(0.0, -1.0, 0.0),
    }
}

fn name(scene: &Scene, index: Option<usize>) -> Option<&str> {
    index.map(|index| scene.objects[index].name.as_str())
}

#[test]
fn a_ray_picks_the_nearest_object() {
    let scene = opened("pick-ray");
    let picker = Picker::of(&scene, &mut Extents::new());
    assert_eq!(name(&scene, picker.cast(&down(-0.5, 0.0))), Some("ball"));
    assert_eq!(name(&scene, picker.cast(&down(0.7, 0.3))), Some("crate"));
    assert_eq!(name(&scene, picker.cast(&down(1.8, -1.8))), Some("floor"));
    assert_eq!(picker.cast(&down(3.0, 3.0)), None);
    let side = Ray {
        origin: DVec3::new(-5.0, 0.3, 0.0),
        direction: DVec3::X,
    };
    assert_eq!(name(&scene, picker.cast(&side)), Some("plinth"));
}

#[test]
fn the_bounds_hold_every_drawn_part() {
    let scene = opened("pick-bounds");
    let picker = Picker::of(&scene, &mut Extents::new());
    let [low, high] = picker.bounds();
    assert!((low[0] + 2.0).abs() < 1e-6, "{low:?}");
    assert!((low[1] + 0.1).abs() < 1e-6, "{low:?}");
    assert!((high[1] - 0.9).abs() < 1e-3, "{high:?}");
    let crate_index = scene
        .objects
        .iter()
        .position(|o| o.name == "crate")
        .unwrap();
    let [low, high] = picker.bounds_of(crate_index).unwrap();
    assert!(
        low[1] > -1e-6 && (high[1] - 0.4).abs() < 1e-6,
        "{low:?} {high:?}"
    );
    assert_eq!(picker.outline(crate_index).len(), 1);
}

#[test]
fn a_click_through_the_scene_camera_picks_what_it_shows() {
    let scene = opened("pick-camera");
    let picker = Picker::of(&scene, &mut Extents::new());
    let eye = Eye::of(&scene.camera_or_default());
    let image = Rect::from_min_size(Pos2::new(20.0, 10.0), Vec2::new(640.0, 400.0));
    let lens = Lens::new(&eye, image);
    for (object, at) in [("crate", [0.7, 0.2, 0.3]), ("ball", [-0.5, 0.7, 0.0])] {
        let point = lens.project(DVec3::from_array(at)).unwrap();
        let picked = picker.cast(&lens.ray(point));
        assert_eq!(name(&scene, picked), Some(object), "{point:?}");
    }
    let sky = lens.ray(Pos2::new(22.0, 12.0));
    assert_eq!(picker.cast(&sky), None);
}

#[test]
fn a_selection_carries_the_scene_id_and_name() {
    let scene = opened("pick-id");
    let crate_selection = Selection::named(&scene, "crate").unwrap();
    assert_eq!(crate_selection.id, 4);
    assert_eq!(crate_selection.index, 3);
    assert_eq!(Selection::by_id(&scene, 4), Some(crate_selection));
    assert_eq!(Selection::by_id(&scene, 0), None);
    assert_eq!(Selection::by_id(&scene, 99), None);
}

#[test]
fn a_groups_outline_and_bounds_hold_every_descendant() {
    let scene = Scene::open(common::group("pick-group")).unwrap();
    let picker = Picker::of(&scene, &mut Extents::new());
    let index = |name: &str| scene.objects.iter().position(|o| o.name == name).unwrap();
    let pair = index("pair");
    let family: Vec<&str> = picker
        .family(pair)
        .into_iter()
        .map(|at| scene.objects[at].name.as_str())
        .collect();
    assert_eq!(family, vec!["pair", "pair/left", "pair/right"]);
    assert_eq!(picker.outline(pair).len(), 2);
    let mut both = picker.outline(index("pair/left"));
    both.extend(picker.outline(index("pair/right")));
    assert_eq!(picker.outline(pair), both);
    let [low, high] = picker.bounds_of(pair).unwrap();
    assert!(
        low[0] < 1.1 - 0.25 && high[0] > 1.1 + 0.25,
        "{low:?} {high:?}"
    );
    assert_eq!(picker.outline(index("plinth")).len(), 2);
    assert_eq!(picker.outline(index("ball")).len(), 1);
    assert_eq!(picker.outline(index("crate")).len(), 1);
}
