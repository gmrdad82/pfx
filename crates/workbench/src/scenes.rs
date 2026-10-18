use std::path::Path;

use pfx_load::scene::{Projection, Scene};
use pfx_recipe::Camera as Shot;

use crate::document::{Document, Item, Lamp, tree_of};

pub const CAMERA: &str = "camera";

pub fn is_scene(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name.ends_with(".scene.toml") {
        return true;
    }
    if name.ends_with(".pitofx") {
        return pfx_file::Reader::open(path)
            .is_ok_and(|reader| reader.find(pfx_file::RCPE).is_none());
    }
    name.ends_with(".toml")
        && std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
            .is_some_and(|document| {
                document.get("recipe").is_none_or(|key| !key.is_integer())
                    && document.get("format").is_some()
            })
}

pub fn is_project(path: &Path) -> bool {
    path.join(pfx_project::project::FILE).is_file()
}

fn wide(value: f32) -> f64 {
    value.to_string().parse().unwrap_or(f64::from(value))
}

pub fn shot(scene: &Scene) -> Shot {
    let camera = scene.camera_or_default();
    let (kind, fov, height) = match camera.projection {
        Projection::Perspective { fov } => {
            (pfx_recipe::Projection::Perspective, Some(wide(fov)), None)
        }
        Projection::Orthographic { height } => (
            pfx_recipe::Projection::Orthographic,
            None,
            Some(wide(height)),
        ),
    };
    Shot {
        name: CAMERA.to_string(),
        kind,
        position: camera.at.map(wide),
        target: camera.look_at.map(wide),
        up: camera.up.map(wide),
        fov_y_deg: fov,
        focal_mm: None,
        sensor_mm: None,
        shift: [0.0, 0.0],
        focus_m: None,
        fstop: 0.0,
        ortho_height: height,
        path: None,
        rig: None,
    }
}

fn first_seen(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for name in names {
        if !seen.contains(&name) {
            seen.push(name);
        }
    }
    seen
}

fn lights(scene: &Scene) -> Vec<(Lamp, String, String)> {
    let mut lights = Vec::new();
    if let Some(sun) = &scene.sun {
        let lamp = if sun.daylight.is_some() {
            Lamp::Daylight
        } else {
            Lamp::Sun
        };
        lights.push((
            lamp,
            "sun".to_string(),
            format!("{:.1} W/m²  hour {:.1}", sun.intensity, sun.hour),
        ));
    }
    if let Some(sky) = &scene.sky {
        lights.push((Lamp::Sky, "sky".to_string(), sky.kind.clone()));
    }
    for (name, light) in &scene.lights {
        lights.push((Lamp::Bulb, name.clone(), format!("{:.2}", light.intensity)));
    }
    for (name, emitter) in &scene.emitters {
        lights.push((
            Lamp::Bulb,
            name.clone(),
            format!("{:.2}  r {:.2}", emitter.intensity, emitter.radius),
        ));
    }
    lights
}

pub fn document(scene: &Scene, path: &str) -> Document {
    let objects: Vec<Item> = scene
        .objects
        .iter()
        .map(|object| Item {
            name: object.name.clone(),
            material: match (&object.material, object.materials.is_empty()) {
                (Some(material), _) => material.clone(),
                (None, false) => object
                    .materials
                    .values()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" "),
                (None, true) => String::new(),
            },
            location: object.at,
            rotation: object.rotate,
            scale: object.scale,
        })
        .collect();
    let names: Vec<String> = scene.objects.iter().map(|o| o.name.clone()).collect();
    let parents: Vec<Option<String>> = scene.objects.iter().map(|o| o.parent.clone()).collect();
    let materials = first_seen(
        scene
            .objects
            .iter()
            .flat_map(|object| object.material.iter().chain(object.materials.values()))
            .cloned()
            .chain(scene.library.names().map(str::to_string)),
    );
    let camera = shot(scene);
    let detail = match (camera.fov_y_deg, camera.ortho_height) {
        (_, Some(height)) => format!("ortho  {height:.2} m"),
        (Some(fov), None) => format!("fov {fov:.1}°"),
        (None, None) => String::new(),
    };
    Document {
        name: Path::new(path)
            .file_name()
            .map(|name| {
                name.to_string_lossy()
                    .trim_end_matches(".scene.toml")
                    .to_string()
            })
            .unwrap_or_default(),
        path: path.to_string(),
        objects,
        nodes: tree_of(&names, &parents),
        cameras: vec![(CAMERA.to_string(), detail)],
        views: vec![camera],
        lights: lights(scene),
        materials,
        hour: None,
        aspect: crate::viewport::ASPECT,
        patch: "none".to_string(),
        source: 0,
        timeline: None,
        markers: Vec::new(),
    }
}

#[cfg(test)]
pub fn fixture(name: &str) -> std::path::PathBuf {
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/scene");
    let to = crate::scratch(name).with_file_name("scene");
    std::fs::create_dir_all(&to).unwrap();
    for entry in std::fs::read_dir(&from).unwrap().flatten() {
        std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
    to.join("stand.scene.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened(name: &str) -> (std::path::PathBuf, Scene) {
        let path = fixture(name);
        let scene = Scene::open(&path).unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .starts_with("format = 1\n")
        );
        assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);
        (path, scene)
    }

    #[test]
    fn a_scene_file_is_named_scene_toml() {
        assert!(is_scene(Path::new("rooms/stand.scene.toml")));
        assert!(!is_scene(Path::new("rooms/stand.toml")));
        assert!(!is_scene(Path::new("rooms/scene.toml")));
        let dir = crate::scratch("is-scene");
        let dir = dir.parent().unwrap();
        let level = dir.join("level.toml");
        std::fs::write(&level, "format = 1\n").unwrap();
        assert!(is_scene(&level));
        let recipe = dir.join("recipe.toml");
        std::fs::write(&recipe, "recipe = 1\nformat = 1\n").unwrap();
        assert!(!is_scene(&recipe));
    }

    #[test]
    fn the_document_lists_the_objects_under_their_parents_and_the_scenes_camera() {
        let (path, scene) = opened("scenes-document");
        let document = document(&scene, &path.display().to_string());
        assert_eq!(document.name, "stand");
        let rows: Vec<(usize, &str)> = document
            .nodes
            .iter()
            .map(|node| (node.depth, node.label.as_str()))
            .collect();
        assert_eq!(
            rows,
            [(0, "floor"), (0, "plinth"), (1, "ball"), (0, "crate")]
        );
        assert_eq!(document.objects[2].material, "brass");
        assert_eq!(document.objects[1].location, [-0.5, 0.25, 0.0]);
        assert_eq!(
            document.cameras,
            [("camera".to_string(), "fov 40.0°".to_string())]
        );
        assert_eq!(document.views[0].position, [0.0, 1.2, 3.2]);
        assert_eq!(document.materials, ["felt", "clay", "brass"]);
        let lamps: Vec<(Lamp, &str)> = document
            .lights
            .iter()
            .map(|(lamp, name, _)| (*lamp, name.as_str()))
            .collect();
        assert_eq!(
            lamps,
            [(Lamp::Sun, "sun"), (Lamp::Sky, "sky"), (Lamp::Bulb, "lamp")]
        );
    }
}
