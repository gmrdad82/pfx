#![allow(dead_code)]

use std::path::{Path, PathBuf};

use pfx_scene::{Diagnostic, Project};

pub const PROJECT: &str = "format = 1\n\n[project]\nname = \"room\"\nscene = \"room.scene.toml\"\n";

pub const ROOM: &str = "format = 1\n\n# the room\ninclude = [\"lights.scene.toml\"]\nmaterials = [\"materials.toml\"]\nfallback = \"grey\"\n\n[mesh.block]\nid = \"me5h00000b\"\nfile = \"block.gltf\"\n\n[mesh.pillar]\nid = \"me5h00000p\"\nfile = \"pillar.gltf\"\nnode = \"Pillar\"\n\n[mesh.pillar.nodes.Cap]\nmaterial = \"metal\" # a bright cap\n\n[mesh.panel]\nid = \"me5h0000p2\"\nfile = \"panel.gltf\"\n\n# the floor\n[[object]]\nid = \"0bj000000f\"\nname = \"floor\"\nmesh = \"block\"\nat = [0.0, -0.05, 0.0]\nscale = [6.0, 0.1, 6.0]\nmaterial = \"grey\"\n\n[[object]]\nid = \"0bj000000c\"\nname = \"crate\"\nmesh = \"block\"\nat = [ -0.9,   0.350, 0.0 ]  # by the wall\nrotate = [0.0, 25.0, 0.0]\nscale = 0.7 # small\npick = 7\n\n# the pillars\n[[object]]\nid = \"0bj00000p1\"\nname = \"left pillar\"\nmesh = \"pillar\"\nat = [0.6, 0.0, -0.6]\n\n[[object]]\nid = \"0bj00000p2\"\nname = \"right pillar\"\nmesh = \"pillar\"\nparent = \"left pillar\"\nat = [1.4, 0.0, -0.6]\nmaterials = { Pillar = \"blue\" }\n\n[[object]]\nid = \"0bj00000s1\"\nname = \"screen\"\nmesh = \"panel\"\nat = [0.0, 1.4, -1.94]\nscale = [1.6, 0.8, 1.0]\ncontent = \"screen\"\n\n[content.screen]\nid = \"c0ntent001\"\nimage = \"screen.png\"\n\n[[mover]]\nid = \"m0ver00001\"\nname = \"spin\"\nobjects = [\"left pillar\", \"crate\"]\nkind = \"turn\"\naxis = [0.0, 1.0, 0.0]\ntravel = [0.0, 90.0]\n\n[sky]\nkind = \"room\"\n\n[sky.room]\nwidth = 64\nfloor = [0.18, 0.16, 0.14]\n\n[[sky.room.lights]]\nname = \"window\"\naz = -40.0\nel = 25.0\nwidth = 40.0\nheight = 30.0\npower = 6.0\n\n[sun]\nmodel = \"authored\"\ntoward = [-0.4, 0.8, 0.45]\ncolor = [1.0, 0.92, 0.8]\nirradiance = 2.0\n\n[camera]\nat = [0.0, 1.4, 4.2]\nlook_at = [0.0, 0.7, 0.0]\nfov = 40.0\n";

pub const LIGHTS: &str = "format = 1\n\n# lights\n[[light]]\nid = \"11ght00001\"\nname = \"warm\"\nposition = [-1.6, 2.2, 1.4]\ncolor = [1.0, 0.82, 0.62]\nintensity = 6.0 # strong\nradius = 0.05\nrange = 8.0\n\n[[light]]\nid = \"11ght00002\"\nname = \"cool\"\nposition = [1.8, 1.6, 1.2]\nintensity = 3.0\nshadow = false\n";

pub const MATERIALS: &str = "format = 1\n\n# materials\n[materials.grey]\nid = \"mat0000001\"\nbase = [0.5, 0.5, 0.5]\nroughness = 0.8\n\n[materials.clay]\nid = \"mat0000002\"\nbase = [0.7, 0.32, 0.22]\nroughness = 0.90 # dry\nspecular = 0.02\n\n[materials.stone]\nid = \"mat0000003\"\nbase = [0.42, 0.44, 0.46]\nroughness = 0.7\n\n[materials.metal]\nid = \"mat0000004\"\nbase = [0.9, 0.88, 0.82]\nroughness = 0.3\nmetalness = 1.0\n\n[materials.blue]\nid = \"mat0000005\"\nbase = [0.15, 0.25, 0.7]\nroughness = 0.5\n\n[materials.screen]\nid = \"mat0000006\"\nbase = [0.02, 0.02, 0.02]\nroughness = 0.4\n\n[materials.screen.content_layer]\nslot = 0\n";

pub const LAMP: &str = "format = 1\n\n# a desk lamp\nmaterials = [\"materials.toml\"]\n\n[mesh.cone]\nid = \"1amp0mesh1\"\nfile = \"ball.gltf\"\n\n[[object]]\nid = \"1amp000001\"\nname = \"shade\"\nmesh = \"cone\"\nmaterial = \"clay\"\nat = [0.0, 0.4, 0.0]\n\n[[object]]\nid = \"1amp000002\"\nname = \"stem\"\nmesh = \"cone\"\nparent = \"shade\"\nscale = 0.2\n\n[[light]]\nid = \"1amp0b01b0\"\nname = \"bulb\"\nposition = [0.0, 0.5, 0.0]\nintensity = 2.0\n";

pub const ASSETS: &[&str] = &[
    "ball.gltf",
    "block.gltf",
    "panel.gltf",
    "pillar.gltf",
    "screen.png",
];

pub struct Folder {
    pub root: PathBuf,
}

impl Folder {
    pub fn empty(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("scenes")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    pub fn assets(name: &str) -> Self {
        let folder = Self::empty(name);
        folder.copy_assets("");
        folder
    }

    pub fn copy_assets(&self, under: &str) {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/assets");
        std::fs::create_dir_all(self.root.join(under)).unwrap();
        for asset in ASSETS {
            std::fs::copy(fixtures.join(asset), self.root.join(under).join(asset)).unwrap();
        }
    }

    pub fn room(name: &str) -> Self {
        let folder = Self::assets(name);
        folder.write("project.toml", PROJECT);
        folder.write("room.scene.toml", ROOM);
        folder.write("lights.scene.toml", LIGHTS);
        folder.write("materials.toml", MATERIALS);
        folder
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    pub fn write(&self, name: &str, text: &str) {
        let path = self.path(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }

    pub fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.path(name)).unwrap()
    }

    pub fn project(&self) -> Project {
        Project::open(&self.root).unwrap()
    }

    pub fn check(&self) -> Vec<Diagnostic> {
        self.project().check()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub fn show(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn codes(diagnostics: &[Diagnostic]) -> Vec<&'static str> {
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

pub fn errors(diagnostics: &[Diagnostic]) -> Vec<&Diagnostic> {
    diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .collect()
}
