use std::path::{Path, PathBuf};

use pfx_post::Pass;

use super::Scene;

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/scene-tests")
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

    fn scene(&self, extra: &str) -> PathBuf {
        let path = self.0.join("room.scene.toml");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(extra);
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn lilac(c: [f32; 3]) -> [f32; 3] {
    [c[0] * 0.9 + 0.06, c[1] * 0.86, c[2] * 0.9 + 0.1]
}

#[test]
fn a_scene_flare_and_cube_lut_reach_the_finish_and_a_bad_cube_is_refused() {
    let folder = Folder::new("flare-lut");
    let cube = pfx_post::cube::format(&pfx_post::cube::generate(17, lilac), "lilac");
    std::fs::write(folder.0.join("lilac.cube"), &cube).unwrap();
    let path = folder.scene(
        "\n[finish]\nlut = \"lilac.cube\"\nlut_amount = 0.6\ntone = \"aces\"\n\n[flare]\namount = 0.3\nghosts = 6\n",
    );
    let scene = Scene::open(&path).unwrap();
    let finish = scene.finish.as_ref().unwrap();
    let names: Vec<&str> = finish.chain.passes.iter().map(Pass::name).collect();
    assert_eq!(names, ["flare", "lut", "tone"]);
    let Pass::Flare(flare) = &finish.chain.passes[0] else {
        unreachable!()
    };
    assert_eq!((flare.amount, flare.ghosts), (0.3, 6));
    let Pass::Lut(lut) = &finish.chain.passes[1] else {
        unreachable!()
    };
    assert_eq!(lut.size, 17);
    assert!((lut.amount - 0.6).abs() < 1e-6);
    assert!(scene.files.keys().any(|file| file.ends_with("lilac.cube")));
    let first = finish.hash;
    std::fs::write(
        folder.0.join("lilac.cube"),
        cube.replacen("0.060000", "0.070000", 1),
    )
    .unwrap();
    let edited = Scene::open(&path).unwrap();
    assert_ne!(edited.finish.unwrap().hash, first);
    std::fs::write(folder.0.join("lilac.cube"), "LUT_3D_SIZE 2\n0 0 0\n").unwrap();
    let refused = Scene::open(&path).expect_err("a short cube is refused");
    assert!(refused.to_string().contains("lilac.cube"), "{refused}");
}

#[test]
fn a_flare_without_a_finish_runs_on_the_standard_finish() {
    let folder = Folder::new("flare-alone");
    let plain = Scene::open(folder.0.join("room.scene.toml")).unwrap();
    assert!(plain.finish.is_none());
    let scene = Scene::open(folder.scene("\n[flare]\n")).unwrap();
    let names: Vec<&str> = scene
        .finish
        .unwrap()
        .chain
        .passes
        .iter()
        .map(Pass::name)
        .collect();
    assert_eq!(names, ["bloom", "flare", "tone"]);
}
