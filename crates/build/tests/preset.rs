use glam::DVec3;
use pfx_build::{Built, build};
use pfx_recipe::{Recipe, parse};
use std::path::Path;

const HEAD: &str = r#"
recipe = 1
name = "presets"
seed = 4

[timeline]
fps = 30
frames = 60

[[route]]
name = "main"
points = [[0.0, 0.0, 0.0], [0.0, 0.0, 40.0], [-10.0, 0.0, 90.0]]

[[object]]
name = "actor"
kind = "box"
size = [1.8, 1.2, 4.0]

[[follow]]
object = "actor"
route = "main"
speed = [[0.0, 20.0]]

"#;

fn recipe(tail: &str) -> Recipe {
    let text = format!("{HEAD}{tail}");
    parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

fn built(tail: &str) -> Built {
    build(&recipe(tail), Path::new(".")).unwrap_or_else(|e| panic!("{e}"))
}

fn eye(built: &Built, n: usize) -> DVec3 {
    let track = built
        .tracks
        .iter()
        .find(|t| t.node == "camera.c")
        .expect("camera track");
    DVec3::from(track.keys[n].translation)
}

#[test]
fn a_drive_by_builds_the_bytes_of_its_expanded_rig_and_the_camera_stands_still() {
    let preset = built(
        "[[camera]]\nname = \"c\"\nfov_y_deg = 40.0\nfstop = 4.0\n\n[camera.preset]\nkind = \"drive-by\"\nfollow = \"actor\"\nside = 5.0\nahead = 30.0\n",
    );
    let expanded = built(
        "[[camera]]\nname = \"c\"\nfov_y_deg = 40.0\nfstop = 4.0\n\n[camera.rig]\nfollow = \"actor\"\noffset = [-5.0, 1.4, 30.0]\nlook = [0.0, 0.5, 0.0]\nrelease = 0.0\nrelease_ease = 0.001\nfocus = \"follow\"\n",
    );
    assert_eq!(preset.glb(), expanded.glb());
    let late = (eye(&preset, 59) - eye(&preset, 10)).length();
    assert!(late < 1e-9, "{late}");
    assert!(eye(&preset, 1).distance(eye(&preset, 0)) < 1.0);
}

#[test]
fn a_crane_rises_while_the_actor_drives() {
    let crane = built(
        "[[camera]]\nname = \"c\"\nfov_y_deg = 40.0\n\n[camera.preset]\nkind = \"crane\"\nfollow = \"actor\"\nfrom_pitch = 5.0\nto_pitch = 60.0\n",
    );
    let actor = crane
        .tracks
        .iter()
        .find(|t| t.node == "actor")
        .expect("actor track");
    let height = |n: usize| eye(&crane, n).y - actor.keys[n].translation[1];
    assert!(height(59) > height(0) + 3.0, "{} {}", height(0), height(59));
}
