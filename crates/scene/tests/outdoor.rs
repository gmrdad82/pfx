mod common;

use std::path::PathBuf;

use common::{Folder, MATERIALS, PROJECT, codes, show};
use pfx_scene::code;
use pfx_scene::types::{Color, LightKind, LutKey, ParticleBlend, ShutterPosition};

const SCENE: &str = "format = 1\nmaterials = [\"materials.toml\"]\nsound = \"street.sound.toml\"\n\n[mesh.block]\nid = \"me5h00000b\"\nfile = \"block.gltf\"\n\n[[object]]\nid = \"0bj00000c1\"\nname = \"car\"\nmesh = \"block\"\n\n[[light]]\nid = \"11ght0000h\"\nname = \"headlight\"\nkind = \"spot\"\nparent = \"car\"\nat = [0.6, 0.4, 1.9]\ndirection = [0.0, -0.1, 1.0]\ncone = [20.0, 35.0]\nintensity = 40.0\nrange = 30.0\n\n[[particles]]\nid = \"p4rt1c1e01\"\nname = \"spray\"\nparent = \"car\"\nnode = \"wheel_rl\"\nat = [0.0, 0.1, -1.9]\nrate = 200.0\nrate_keys = [[0.0, 0.0], [2.0, 400.0]]\nmax = 2000\nlifetime = [0.4, 0.9]\nspeed = [1.0, 3.0]\ncone = 25.0\ninherit = 0.5\ngravity = [0.0, -9.8, 0.0]\nalpha = [[0.0, 0.0], [0.1, 0.6], [1.0, 0.0]]\ncolour = \"#c8d0d8\"\ncolours = [[0.0, \"#ffffff\"], [1.0, \"#8090a0\"]]\nblend = \"add\"\nlit = true\nseed = 7\n\n[haze]\nheight = 0.0\nfalloff = 0.08\nshafts = 0.6\nsteps = 48\nreach = 400.0\ncolour = \"#ffb070\"\n\n[finish]\nlut = \"grade.cube\"\nlut_amount = 0.8\n\n[flare]\namount = 0.3\nghosts = 4\nghost_spacing = 0.3\nhalo = 0.2\nhalo_radius = 0.45\nstarburst = 0.5\nthreshold = 2.0\n\n[motion_blur]\nshutter = 0.5\nmax_px = 32.0\nsamples = 12\nposition = \"end\"\n\n[camera]\nat = [0.0, 1.4, 6.0]\nlook_at = [0.0, 0.5, 0.0]\nfstop = 2.8\nfocus_keys = [[0.0, 6.0], [3.0, 2.5]]\n";

fn golden(name: &str) -> Folder {
    let folder = Folder::assets(name);
    folder.write("project.toml", PROJECT);
    folder.write("room.scene.toml", SCENE);
    folder.write(
        "materials.toml",
        &format!(
            "{MATERIALS}\n[materials.grey.wet]\namount = 0.7\npuddles = 0.4\npuddle_scale = 3.0\nseed = 11\n"
        ),
    );
    folder.write("street.sound.toml", "");
    folder.write("grade.cube", "");
    folder
}

#[test]
fn the_golden_hour_keys_read_resolve_and_check_clean() {
    let folder = golden("golden");
    let diagnostics = folder.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    let scene = folder
        .project()
        .scene(&folder.path("room.scene.toml"))
        .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)));
    let light = &scene.light("headlight").unwrap().value;
    assert_eq!(light.kind, Some(LightKind::Spot));
    assert_eq!(light.parent.as_deref(), Some("car"));
    assert_eq!(light.cone, Some([20.0, 35.0]));
    let spray = &scene.particles("spray").unwrap().value;
    assert_eq!(spray.parent.as_deref(), Some("car"));
    assert_eq!(spray.node.as_deref(), Some("wheel_rl"));
    assert_eq!(spray.blend, Some(ParticleBlend::Add));
    assert_eq!(spray.color, Some(Color::Hex("#c8d0d8".into())));
    assert_eq!(spray.colors.as_ref().map(Vec::len), Some(2));
    assert_eq!(spray.rate_keys, Some(vec![[0.0, 0.0], [2.0, 400.0]]));
    let haze = scene.haze.as_ref().unwrap();
    assert!(haze.outdoor() && haze.lo.is_none());
    assert_eq!(haze.steps, Some(48));
    let finish = scene.finish.as_ref().unwrap();
    assert_eq!(finish.lut, Some(LutKey::Path("grade.cube".into())));
    assert_eq!(finish.lut_amount, Some(0.8));
    assert_eq!(scene.flare.unwrap().ghosts, Some(4));
    let blur = scene.motion_blur.unwrap();
    assert_eq!(blur.samples, Some(12));
    assert_eq!(blur.position, Some(ShutterPosition::End));
    assert_eq!(
        scene.camera.as_ref().unwrap().focus_keys,
        Some(vec![[0.0, 6.0], [3.0, 2.5]])
    );
    assert_eq!(scene.sound_file, Some(PathBuf::from("street.sound.toml")));
    let grey = &scene.material("grey").unwrap().value;
    assert_eq!(grey.wet.unwrap().puddles, Some(0.4));
}

#[test]
fn out_of_range_and_missing_keys_are_reported_where_they_stand() {
    let folder = golden("golden-bad");
    let bad = SCENE
        .replace("inherit = 0.5", "inherit = 1.5")
        .replace("shutter = 0.5", "shutter = 4.5")
        .replace("lut_amount = 0.8", "lut_amount = -1.0")
        .replace("shafts = 0.6", "shafts = 3.0")
        .replace("[[0.0, 6.0], [3.0, 2.5]]", "[[3.0, 6.0], [1.0, 2.5]]")
        .replace(
            "parent = \"car\"\nat = [0.6",
            "position = [0.0, 1.0, 0.0]\nat = [0.6",
        )
        .replace("grade.cube", "grade.png")
        .replace("street.sound.toml", "missing.sound.toml")
        + "\n[[light]]\nname = \"lamp\"\nnode = \"bulb\"\nintensity = 1.0\n";
    folder.write("room.scene.toml", &bad);
    let diagnostics = folder.check();
    let keys: Vec<&str> = diagnostics.iter().map(|d| d.key.as_str()).collect();
    for key in [
        "particles.p4rt1c1e01.inherit",
        "motion_blur.shutter",
        "finish.lut_amount",
        "haze.shafts",
        "light.11ght0000h.at",
        "finish.lut",
        "sound",
        "camera.focus_keys",
        "light.1.node",
    ] {
        assert!(keys.contains(&key), "{key} not in {}", show(&diagnostics));
    }
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == code::MISSING_KEY && d.message.contains("light lamp")),
        "{}",
        show(&diagnostics)
    );
    assert!(codes(&diagnostics).contains(&code::MISSING_FILE));
}
