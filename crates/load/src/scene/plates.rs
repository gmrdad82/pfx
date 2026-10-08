use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use pfx_scene::Entry;
use pfx_scene::types as file;

use super::build::{Places, Reader, Step};
use super::{Matrix, Mover, Object, Scene, SceneError, model};
use crate::vec3::{cross, dot, sub};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlateCasters {
    Meshes,
    Proxies,
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProxyShape {
    Box {
        at: [f32; 3],
        rotate: [f32; 3],
        size: [f32; 3],
        model: Matrix,
    },
    Hull {
        points: Vec<[f32; 3]>,
        triangles: Vec<[u32; 3]>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Proxy {
    pub object: String,
    pub id: u32,
    pub shape: ProxyShape,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Proxies {
    pub file: PathBuf,
    pub items: Vec<Proxy>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plates {
    pub dir: PathBuf,
    pub manifest: Option<[u8; 32]>,
    pub proxies: Option<Proxies>,
    pub casters: PlateCasters,
}

pub(super) fn build(
    places: &Places,
    holder: &Path,
    plates: &file::Plates,
    proxies: &[Entry<file::Proxy>],
    objects: &[Object],
    reader: &mut Reader,
) -> Result<Plates, SceneError> {
    let steps = [Step::Key("plates")];
    let dir = places.root.join(&plates.dir);
    let manifest_path = dir.join("manifest.json");
    let manifest = if manifest_path.is_file() {
        let bytes = reader
            .read(&manifest_path)
            .map_err(|message| places.refusal(holder, &steps, message))?;
        Some(crate::sha256(&bytes))
    } else {
        None
    };
    let casters = match plates.casters.unwrap_or_default() {
        file::Casters::Meshes => PlateCasters::Meshes,
        file::Casters::Proxies => PlateCasters::Proxies,
        file::Casters::None => PlateCasters::None,
    };
    let proxies = match &plates.proxies {
        Some(relative) => {
            let path = places.root.join(relative);
            let mut items = Vec::new();
            for entry in proxies {
                items.push(proxy(places, relative, entry, objects)?);
            }
            Some(Proxies { file: path, items })
        }
        None => None,
    };
    Ok(Plates {
        dir,
        manifest,
        proxies,
        casters,
    })
}

fn proxy(
    places: &Places,
    relative: &str,
    entry: &Entry<file::Proxy>,
    objects: &[Object],
) -> Result<Proxy, SceneError> {
    let place: usize = entry.key.parse().unwrap_or(0);
    let value = &entry.value;
    let Some(object) = objects.iter().find(|object| object.name == value.object) else {
        return places.error(
            Path::new(relative),
            &[Step::Index("proxy", place)],
            format!(
                "proxy names object {}, which the scene has not",
                value.object
            ),
        );
    };
    let shape = match value.kind {
        file::ProxyKind::Box => {
            let at = value.at.unwrap_or([0.0; 3]);
            let rotate = value.rotate.unwrap_or([0.0; 3]);
            let size = value.size.unwrap_or([1.0; 3]);
            ProxyShape::Box {
                at,
                rotate,
                size,
                model: model(at, rotate, size),
            }
        }
        file::ProxyKind::Hull => ProxyShape::Hull {
            points: value.points.clone().unwrap_or_default(),
            triangles: value.triangles.clone().unwrap_or_default(),
        },
    };
    Ok(Proxy {
        object: value.object.clone(),
        id: object.id,
        shape,
    })
}

fn moved(movers: &BTreeMap<String, Mover>, name: &str) -> bool {
    movers
        .values()
        .any(|mover| mover.objects.iter().any(|object| object == name))
}

fn dynamic_at(
    objects: &[Object],
    movers: &BTreeMap<String, Mover>,
    animated: &BTreeMap<String, super::Animation>,
    index: usize,
) -> bool {
    let mut at = index;
    for _ in 0..=objects.len() {
        let object = &objects[at];
        if let Some(dynamic) = object.dynamic {
            return dynamic;
        }
        if object.face_camera || moved(movers, &object.name) || animated.contains_key(&object.name)
        {
            return true;
        }
        match object
            .parent
            .as_ref()
            .and_then(|parent| objects.iter().position(|other| &other.name == parent))
        {
            Some(parent) => at = parent,
            None => return false,
        }
    }
    false
}

impl Scene {
    pub fn dynamic(&self, index: usize) -> bool {
        index < self.objects.len()
            && dynamic_at(&self.objects, &self.movers, &self.animations, index)
    }

    pub fn dynamic_names(&self) -> BTreeSet<String> {
        (0..self.objects.len())
            .filter(|&index| self.dynamic(index))
            .map(|index| self.objects[index].name.clone())
            .collect()
    }

    pub fn static_subset(&self) -> Scene {
        let mut subset = self.clone();
        for index in 0..self.objects.len() {
            if self.dynamic(index) {
                subset.objects[index].hidden = true;
            }
        }
        subset.texts.retain(|_, text| !text.dynamic);
        subset
    }
}

impl Proxies {
    pub fn pick(&self, origin: [f32; 3], direction: [f32; 3]) -> Option<(u32, f32)> {
        let mut best: Option<(u32, f32)> = None;
        for proxy in &self.items {
            let hit = match &proxy.shape {
                ProxyShape::Box {
                    at, rotate, size, ..
                } => box_hit(*at, *rotate, *size, origin, direction),
                ProxyShape::Hull { points, triangles } => triangles
                    .iter()
                    .filter_map(|triangle| {
                        let [a, b, c] = triangle.map(|corner| points[corner as usize]);
                        triangle_hit(a, b, c, origin, direction)
                    })
                    .reduce(f32::min),
            };
            if let Some(t) = hit
                && best.is_none_or(|(_, nearest)| t < nearest)
            {
                best = Some((proxy.id, t));
            }
        }
        best
    }
}

fn box_hit(
    at: [f32; 3],
    rotate: [f32; 3],
    size: [f32; 3],
    origin: [f32; 3],
    direction: [f32; 3],
) -> Option<f32> {
    let turn = model([0.0; 3], rotate, [1.0; 3]);
    let axes: [[f32; 3]; 3] = std::array::from_fn(|k| [turn[k][0], turn[k][1], turn[k][2]]);
    let offset = sub(origin, at);
    let mut near = f32::NEG_INFINITY;
    let mut far = f32::INFINITY;
    for k in 0..3 {
        let half = size[k] * 0.5;
        let o = dot(offset, axes[k]);
        let d = dot(direction, axes[k]);
        if d.abs() < 1e-12 {
            if o.abs() > half {
                return None;
            }
            continue;
        }
        let a = (-half - o) / d;
        let b = (half - o) / d;
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    if near > far || far < 0.0 {
        return None;
    }
    Some(if near >= 0.0 { near } else { far })
}

fn triangle_hit(
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    origin: [f32; 3],
    direction: [f32; 3],
) -> Option<f32> {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let p = cross(direction, ac);
    let det = dot(ab, p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inverse = 1.0 / det;
    let s = sub(origin, a);
    let u = dot(s, p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, ab);
    let v = dot(direction, q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot(ac, q) * inverse;
    (t >= 0.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::super::Scene;
    use super::*;

    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tmp/plate-scene-tests")
                .join(name);
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scenes");
            for entry in std::fs::read_dir(fixtures).unwrap() {
                let entry = entry.unwrap();
                std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
            }
            Self(root)
        }

        fn write(&self, name: &str, text: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, text).unwrap();
            path
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const BASE: &str = "materials = [\"materials.toml\"]
fallback = \"grey\"

[mesh.block]
file = \"block.gltf\"

[[object]]
name = \"desk\"
mesh = \"block\"
scale = [2.0, 0.1, 1.0]

[[object]]
name = \"lamp\"
mesh = \"block\"
parent = \"desk\"
at = [0.0, 3.0, 0.0]

[[object]]
name = \"lid\"
mesh = \"block\"
at = [1.0, 0.5, 0.0]

[[object]]
name = \"knob\"
mesh = \"block\"
parent = \"lid\"
at = [0.0, 1.0, 0.0]

[[object]]
name = \"card\"
mesh = \"block\"
face_camera = true

[[object]]
name = \"slot\"
mesh = \"block\"
dynamic = true

[[object]]
name = \"pinned\"
mesh = \"block\"
parent = \"lid\"
dynamic = false

[[mover]]
name = \"hinge\"
objects = [\"lid\"]
kind = \"turn\"
axis = [0.0, 0.0, 1.0]
travel = [0.0, 30.0]
";

    #[test]
    fn dynamic_is_derived_from_movers_cards_and_parents_and_overridden_by_the_key() {
        let folder = Folder::new("derived");
        let path = folder.write("desk.scene.toml", BASE);
        let scene = Scene::open(&path).unwrap();
        let names: Vec<String> = scene.dynamic_names().into_iter().collect();
        assert_eq!(names, ["card", "knob", "lid", "slot"]);
        assert!(!scene.dynamic(0));
        assert!(!scene.dynamic(99));
        let subset = scene.static_subset();
        let hidden: Vec<&str> = subset
            .objects
            .iter()
            .filter(|object| object.hidden)
            .map(|object| object.name.as_str())
            .collect();
        assert_eq!(hidden, ["lid", "knob", "card", "slot"]);
        let draws = subset.draws();
        let shown: Vec<&str> = draws
            .items
            .iter()
            .map(|draw| draw.object.as_str())
            .collect();
        assert_eq!(shown, ["desk", "lamp", "pinned"]);
    }

    #[test]
    fn dynamic_text_leaves_the_static_subset_and_static_text_stays() {
        let folder = Folder::new("text");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/family.ttf"),
            folder.0.join("family.ttf"),
        )
        .unwrap();
        let path = folder.write(
            "desk.scene.toml",
            &format!(
                "{BASE}\n[text.sign]\ntext = \"A\"\nfont = \"family.ttf\"\nsize = 0.1\n\n[text.ink]\ntext = \"B\"\nfont = \"family.ttf\"\nsize = 0.1\ndynamic = true\n"
            ),
        );
        let scene = Scene::open(&path).unwrap();
        assert!(!scene.texts["sign"].dynamic);
        assert!(scene.texts["ink"].dynamic);
        let subset = scene.static_subset();
        assert_eq!(subset.texts.keys().collect::<Vec<_>>(), ["sign"]);
    }

    #[test]
    fn plates_read_their_manifest_and_proxies_and_watch_both() {
        let folder = Folder::new("plates");
        std::fs::create_dir_all(folder.0.join("plates/main")).unwrap();
        folder.write("plates/main/manifest.json", "{}");
        folder.write(
            "desk.proxies.toml",
            "[[proxy]]\nobject = \"desk\"\nkind = \"box\"\nsize = [2.0, 0.1, 1.0]\n\n[[proxy]]\nobject = \"lamp\"\nkind = \"hull\"\npoints = [[0.0, 3.0, 0.0], [1.0, 3.0, 0.0], [0.0, 4.0, 0.0], [0.0, 3.0, 1.0]]\ntriangles = [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]]\n",
        );
        let path = folder.write(
            "desk.scene.toml",
            &format!(
                "{BASE}\n[plates]\ndir = \"plates/main\"\nproxies = \"desk.proxies.toml\"\ncasters = \"proxies\"\n"
            ),
        );
        let scene = Scene::open(&path).unwrap();
        let plates = scene.plates.as_ref().unwrap();
        assert!(plates.manifest.is_some());
        assert_eq!(plates.casters, PlateCasters::Proxies);
        let proxies = plates.proxies.as_ref().unwrap();
        assert_eq!(proxies.items.len(), 2);
        assert_eq!(proxies.items[0].id, 1);
        assert_eq!(proxies.items[1].id, 2);
        let watched: Vec<String> = scene
            .files
            .keys()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(watched.contains(&"manifest.json".to_string()));
        assert!(watched.contains(&"desk.proxies.toml".to_string()));
        let (id, t) = proxies.pick([0.0, 5.0, 0.0], [0.0, -1.0, 0.0]).unwrap();
        assert_eq!(id, 2);
        assert!((t - 1.0).abs() < 1e-5);
        let (id, t) = proxies.pick([0.9, 5.0, 0.4], [0.0, -1.0, 0.0]).unwrap();
        assert_eq!(id, 1);
        assert!((t - 4.95).abs() < 1e-5);
        assert!(proxies.pick([5.0, 5.0, 0.0], [0.0, -1.0, 0.0]).is_none());
        let unbaked = folder.write(
            "unbaked.scene.toml",
            &format!("{BASE}\n[plates]\ndir = \"plates/none\"\n"),
        );
        let scene = Scene::open(&unbaked).unwrap();
        assert_eq!(scene.plates.unwrap().manifest, None);
    }

    #[test]
    fn plates_refuse_a_dynamic_proxy_an_unknown_kind_and_casters_without_proxies() {
        let folder = Folder::new("refused");
        let cases = [
            (
                "[[proxy]]\nobject = \"lid\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\n",
                "proxies = \"p.proxies.toml\"",
                "which is dynamic",
            ),
            (
                "[[proxy]]\nobject = \"desk\"\nkind = \"ball\"\n",
                "proxies = \"p.proxies.toml\"",
                "unknown variant 'ball'",
            ),
            (
                "[[proxy]]\nobject = \"nobody\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\n",
                "proxies = \"p.proxies.toml\"",
                "names no object",
            ),
            (
                "[[proxy]]\nobject = \"desk\"\nkind = \"box\"\nsize = [1.0, 0.0, 1.0]\n",
                "proxies = \"p.proxies.toml\"",
                "is above 0",
            ),
            ("", "casters = \"proxies\"", "needs a proxies file"),
            ("", "casters = \"all\"", "'meshes', 'proxies', 'none'"),
        ];
        for (proxies, key, message) in cases {
            folder.write("p.proxies.toml", proxies);
            let path = folder.write(
                "desk.scene.toml",
                &format!("{BASE}\n[plates]\ndir = \"plates\"\n{key}\n"),
            );
            let error = Scene::open(&path).unwrap_err();
            assert!(error.message.contains(message), "{error}");
        }
        let path = folder.write(
            "desk.scene.toml",
            &format!("{BASE}\n[plates]\ndir = \"plates\"\nshadows = true\n"),
        );
        let error = Scene::open(&path).unwrap_err();
        assert!(error.message.contains("unknown key"), "{error}");
    }

    #[test]
    fn open_at_sets_the_sun_hour_and_the_sky_follows() {
        let folder = Folder::new("hour");
        let path = folder.write(
            "desk.scene.toml",
            &format!(
                "{BASE}\n[sun]\nmodel = \"daylight\"\nhour = 12.0\n\n[sky]\nkind = \"analytic\"\n"
            ),
        );
        let noon = Scene::open(&path).unwrap();
        let same = Scene::open_at(&path, 12.0).unwrap();
        let evening = Scene::open_at(&path, 18.0).unwrap();
        assert_eq!(noon.sun, same.sun);
        assert_eq!(evening.sun.unwrap().hour, 18.0);
        assert!(evening.sun.unwrap().direction[1] < noon.sun.unwrap().direction[1]);
        assert_ne!(noon.sky, evening.sky);
        let authored = folder.write(
            "authored.scene.toml",
            &format!(
                "{BASE}\n[sun]\nmodel = \"authored\"\ntoward = [0.0, 1.0, 0.2]\nirradiance = 2.0\n"
            ),
        );
        let rest = Scene::open(&authored).unwrap().sun.unwrap();
        let later = Scene::open_at(&authored, 17.0).unwrap().sun.unwrap();
        assert_eq!(rest.hour, 12.0);
        assert_eq!(later.hour, 17.0);
        assert_ne!(rest.direction, later.direction);
    }
}
