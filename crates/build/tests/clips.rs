mod common;

use glam::{DMat4, DVec3};
use pfx_build::library::{self, Sidecar};
use pfx_build::{Built, build, matrix};
use pfx_load::fixture::{TIMELINE_CLIP, timeline_glb, timeline_x};
use pfx_recipe::parse;
use std::path::{Path, PathBuf};

const HEAD: &str = r#"recipe = 1
name = "clips"

[timeline]
fps = 8
frames = 8

[[route]]
name = "main"
points = [[0.0, 0.0, 0.0], [0.0, 0.0, 100.0]]

[[object]]
name = "box"
kind = "library"
object = "prop/box"
at = [0.0, 0.0, 2.0]
"#;

const FOLLOW: &str = r#"
[[follow]]
object = "box"
route = "main"
speed = [[0.0, 4.0]]
"#;

fn library(name: &str, model: &[u8]) -> PathBuf {
    let dir = common::scratch(name);
    let root = dir.join("library");
    let folder = root.join("prop/box");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(library::MODEL), model).unwrap();
    let sidecar = Sidecar {
        kind: "prop".into(),
        name: "box".into(),
        tags: vec!["test".into()],
        licence: library::LICENCE.into(),
        credit: None,
        author: None,
        source: None,
        generator: None,
        params: Default::default(),
        pfx: "1.3.0".into(),
        facts: Default::default(),
    };
    std::fs::write(folder.join(library::SIDECAR), sidecar.text()).unwrap();
    library::write_index(&root).unwrap();
    dir
}

fn built(dir: &Path, text: &str) -> Built {
    let r = parse(text).unwrap_or_else(|e| panic!("{e}"));
    build(&r, dir).unwrap_or_else(|e| panic!("{e}"))
}

fn keys(built: &Built) -> Vec<DMat4> {
    let part = built
        .parts
        .iter()
        .find(|p| p.node.name == "box.body")
        .expect("the box is built");
    let [lo, hi] = pfx_mesh::bounds(&part.node.mesh).expect("the box has vertices");
    let centre = DMat4::from_translation((DVec3::from(lo) + DVec3::from(hi)) * 0.5);
    built
        .tracks
        .iter()
        .find(|t| t.node == "box.body")
        .expect("the box has a track")
        .keys
        .iter()
        .map(|k| matrix(k) * centre)
        .collect()
}

fn at(m: DMat4) -> DVec3 {
    m.transform_point3(DVec3::ZERO)
}

fn near(a: DVec3, b: DVec3) -> bool {
    (a - b).abs().max_element() < 1e-5
}

fn x(time: f64) -> f64 {
    f64::from(timeline_x(time as f32))
}

fn looped(time: f64) -> f64 {
    x(time.rem_euclid(1.0))
}

#[test]
fn a_library_clip_plays_on_the_timeline() {
    let dir = library("clips-plain", &timeline_glb());
    let keys = keys(&built(&dir, HEAD));
    assert_eq!(keys.len(), 9);
    for (n, key) in keys.iter().enumerate() {
        let t = n as f64 / 8.0;
        assert!(
            near(at(*key), DVec3::new(looped(t), 0.0, 2.0)),
            "{n}: {}",
            at(*key)
        );
    }
}

#[test]
fn a_clip_starts_late_and_runs_at_its_speed() {
    let dir = library("clips-timing", &timeline_glb());
    let text = format!(
        "{HEAD}clip = \"{TIMELINE_CLIP}\"\nloop = false\nclip_start = 0.25\nclip_speed = 2.0\n"
    );
    let keys = keys(&built(&dir, &text));
    let t = |n: usize| ((n as f64 / 8.0 - 0.25).max(0.0) * 2.0).min(1.0);
    for (n, key) in keys.iter().enumerate() {
        assert!(
            near(at(*key), DVec3::new(x(t(n)), 0.0, 2.0)),
            "{n}: {}",
            at(*key)
        );
    }
    assert!(near(at(keys[0]), at(keys[2])));
    assert!(near(at(keys[6]), at(keys[8])));
}

#[test]
fn a_next_clip_blends_in_over_its_seconds() {
    let dir = library("clips-blend", &timeline_glb());
    let text = format!(
        "{HEAD}loop = false\nnext_clip = \"{TIMELINE_CLIP}\"\nnext_at = 0.5\nblend = 0.5\n"
    );
    let keys = keys(&built(&dir, &text));
    for (n, key) in keys.iter().enumerate() {
        let t = n as f64 / 8.0;
        let w = ((t - 0.5) / 0.5).clamp(0.0, 1.0);
        let want = x(t) + (x((t - 0.5).max(0.0)) - x(t)) * w;
        assert!(
            near(at(*key), DVec3::new(want, 0.0, 2.0)),
            "{n}: {}",
            at(*key)
        );
    }
}

#[test]
fn an_unknown_next_clip_is_refused_with_the_names() {
    let dir = library("clips-unknown", &timeline_glb());
    let r = parse(&format!("{HEAD}next_clip = \"run\"\n")).unwrap();
    let error = build(&r, &dir).unwrap_err().to_string();
    assert!(error.contains("object[box].next_clip"), "{error}");
    assert!(error.contains(TIMELINE_CLIP), "{error}");
}

#[test]
fn a_followed_object_keeps_its_clip_and_ignores_its_root_motion() {
    let dir = library("clips-follow", &timeline_glb());
    let plain = keys(&built(&dir, HEAD));
    let followed = keys(&built(&dir, &format!("{HEAD}{FOLLOW}")));
    let applied = keys(&built(
        &dir,
        &format!("{HEAD}{FOLLOW}root_motion = \"apply\"\n"),
    ));
    for n in 0..9 {
        let t = n as f64 / 8.0;
        let travelled = 4.0 * t;
        assert!(
            near(at(followed[n]), DVec3::new(0.0, 0.0, travelled)),
            "{n}: {}",
            at(followed[n])
        );
        assert!(
            near(at(applied[n]), DVec3::new(looped(t), 0.0, travelled)),
            "{n}: {}",
            at(applied[n])
        );
        let turn = |m: DMat4| m.transform_vector3(DVec3::X);
        assert!(near(turn(followed[n]), turn(plain[n])), "{n}");
    }
    let turn = |m: DMat4| m.transform_vector3(DVec3::X);
    assert!(!near(turn(followed[0]), turn(followed[4])));
}

#[test]
fn root_motion_takes_ignore_or_apply() {
    let r = parse(&format!("{HEAD}{FOLLOW}root_motion = \"drift\"\n"));
    let error = r.unwrap_err().to_string();
    assert!(error.contains("root_motion"), "{error}");
    assert!(error.contains("\"ignore\" or \"apply\""), "{error}");
}

const BONES: usize = 3;

fn column_text(head: &str, kind: &str) -> String {
    let body = match kind {
        "mesh" => "kind = \"mesh\"\npath = \"column.glb\"\n",
        _ => "kind = \"library\"\nobject = \"prop/box\"\n",
    };
    format!(
        r#"recipe = 1
name = "skin"
{head}
[[route]]
name = "main"
points = [[0.0, 0.0, 0.0], [0.0, 0.0, 100.0]]

[[object]]
name = "col"
{body}at = [1.0, 0.0, -2.0]
"#
    )
}

const TIMELINE: &str = "[timeline]\nfps = 10\nframes = 10\n";

fn slide() -> Vec<pfx_core::anim::Clip> {
    use pfx_core::anim::{Channel, Clip, Interpolation, Property};
    let mut clips = pfx_core::anim::fixture::clips(BONES);
    clips.push(
        Clip::new(
            "slide",
            vec![Channel {
                joint: 0,
                property: Property::Translation,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 1.0],
                values: vec![0.0, 0.0, 0.0, 2.0, 0.25, 0.0],
            }],
        )
        .unwrap(),
    );
    clips
}

fn column_dir(name: &str) -> PathBuf {
    let model = pfx_load::fixture::skinned_column_with(BONES, slide());
    let dir = library(name, &model);
    std::fs::write(dir.join("column.glb"), &model).unwrap();
    dir
}

fn reference(clip: &str, local: f32, grounded: bool) -> Vec<DVec3> {
    use pfx_core::anim::{Pose, Rig, fixture};
    let rig = Rig::new(fixture::skeleton(BONES), slide()).unwrap();
    let column = fixture::column(BONES);
    let mut pose = Pose::rest(rig.skeleton());
    let found = rig.clips().iter().find(|c| c.name() == clip).unwrap();
    found.sample(local, &mut pose);
    if grounded {
        let rest = rig.skeleton().joints()[0].rest.translation;
        let t = &mut pose.locals_mut()[0].translation;
        t[0] = rest[0];
        t[2] = rest[2];
    }
    let palette = pose.palette(rig.skeleton());
    let skinned = pfx_core::anim::skin(
        &column.positions,
        &column.normals,
        &[],
        &column.joints,
        &column.weights,
        &palette,
    )
    .unwrap();
    skinned
        .positions
        .iter()
        .map(|p| DVec3::from(Vec3Like(*p)))
        .collect()
}

struct Vec3Like([f32; 3]);

impl From<Vec3Like> for DVec3 {
    fn from(v: Vec3Like) -> DVec3 {
        DVec3::new(f64::from(v.0[0]), f64::from(v.0[1]), f64::from(v.0[2]))
    }
}

fn posed(dir: &Path, text: &str, time: f64) -> Vec<DVec3> {
    let r = parse(text).unwrap_or_else(|e| panic!("{e}"));
    let b = build(&r, dir).unwrap_or_else(|e| panic!("{e}"));
    let (_, b) = pfx_build::pose(&r, &b, time).unwrap();
    let part = b
        .parts
        .iter()
        .find(|p| p.node.name.starts_with("col"))
        .expect("the column is built");
    let place = if part.node.keep_transform {
        part.node.transform
    } else {
        DMat4::IDENTITY
    };
    part.node
        .mesh
        .positions
        .iter()
        .map(|p| place.transform_point3(DVec3::from(Vec3Like(*p))))
        .collect()
}

fn same_shape(made: &[DVec3], want: &[DVec3], offset: DMat4) {
    let want: Vec<DVec3> = want.iter().map(|p| offset.transform_point3(*p)).collect();
    for p in made {
        let nearest = want
            .iter()
            .map(|w| (*w - *p).length())
            .fold(f64::INFINITY, f64::min);
        assert!(nearest < 1e-4, "{p} is {nearest} from the reference");
    }
    for w in &want {
        let nearest = made
            .iter()
            .map(|p| (*w - *p).length())
            .fold(f64::INFINITY, f64::min);
        assert!(
            nearest < 1e-4,
            "the reference {w} is {nearest} from the build"
        );
    }
}

#[test]
fn a_skinned_clip_bends_the_mesh_as_the_reference_skinning_does() {
    let dir = column_dir("skin-library");
    for kind in ["library", "mesh"] {
        let text = format!(
            "{}clip = \"bend\"\nloop = false\n",
            column_text(TIMELINE, kind)
        );
        let placed = DMat4::from_translation(DVec3::new(1.0, 0.0, -2.0));
        let early = posed(&dir, &text, 0.0);
        let late = posed(&dir, &text, 0.5);
        same_shape(&early, &reference("bend", 0.0, false), placed);
        same_shape(&late, &reference("bend", 0.5, false), placed);
        let moved = early
            .iter()
            .zip(&late)
            .map(|(a, b)| (*a - *b).length())
            .fold(0.0, f64::max);
        assert!(moved > 0.5, "{kind}: {moved}");
    }
}

#[test]
fn a_skinned_object_without_a_timeline_builds_at_rest() {
    let dir = column_dir("skin-rest");
    let r = parse(&column_text("", "library")).unwrap();
    let b = build(&r, &dir).unwrap();
    assert!(!b.skinned());
    let part = b
        .parts
        .iter()
        .find(|p| p.node.name == "col.column")
        .unwrap();
    let points: Vec<DVec3> = part
        .node
        .mesh
        .positions
        .iter()
        .map(|p| DVec3::from(Vec3Like(*p)))
        .collect();
    let rest = pfx_core::anim::fixture::column(BONES)
        .positions
        .iter()
        .map(|p| DVec3::from(Vec3Like(*p)))
        .collect::<Vec<_>>();
    same_shape(
        &points,
        &rest,
        DMat4::from_translation(DVec3::new(1.0, 0.0, -2.0)),
    );
}

#[test]
fn a_followed_skinned_object_walks_its_clip_with_root_motion_ignored_or_applied() {
    let dir = column_dir("skin-follow");
    let follow = "\n[[follow]]\nobject = \"col\"\nroute = \"main\"\nspeed = [[0.0, 4.0]]\n";
    let text = format!(
        "{}clip = \"slide\"\nloop = false\n{follow}",
        column_text(TIMELINE, "library")
    );
    let at = |t: f64| DMat4::from_translation(DVec3::new(0.0, 0.0, 4.0 * t));
    for t in [0.0, 0.5] {
        same_shape(
            &posed(&dir, &text, t),
            &reference("slide", t as f32, true),
            at(t),
        );
    }
    let applied = format!("{text}root_motion = \"apply\"\n");
    for t in [0.0, 0.5] {
        same_shape(
            &posed(&dir, &applied, t),
            &reference("slide", t as f32, false),
            at(t),
        );
    }
    let ignored = posed(&dir, &text, 0.5);
    let kept = posed(&dir, &applied, 0.5);
    let mean = |points: &[DVec3]| points.iter().copied().sum::<DVec3>() / points.len() as f64;
    let moved = mean(&kept) - mean(&ignored);
    assert!((moved.x - 1.0).abs() < 1e-4, "{moved}");
    assert!(moved.y.abs() < 1e-4, "{moved}");
}

#[test]
fn a_skinned_clip_blends_into_the_next() {
    let dir = column_dir("skin-blend");
    let text = format!(
        "{}clip = \"bend\"\nloop = false\nnext_clip = \"slide\"\nnext_at = 0.2\nblend = 0.4\n",
        column_text(TIMELINE, "library")
    );
    let made = posed(&dir, &text, 0.4);
    let r = pfx_core::anim::Rig::new(pfx_core::anim::fixture::skeleton(BONES), slide()).unwrap();
    let mut pose = pfx_core::anim::Pose::rest(r.skeleton());
    r.clips()[0].sample(0.4, &mut pose);
    let mut other = pfx_core::anim::Pose::rest(r.skeleton());
    r.clips()
        .iter()
        .find(|c| c.name() == "slide")
        .unwrap()
        .sample(0.2, &mut other);
    pose.mix(&other, 0.5);
    let column = pfx_core::anim::fixture::column(BONES);
    let want: Vec<DVec3> = pfx_core::anim::skin(
        &column.positions,
        &column.normals,
        &[],
        &column.joints,
        &column.weights,
        &pose.palette(r.skeleton()),
    )
    .unwrap()
    .positions
    .iter()
    .map(|p| DVec3::from(Vec3Like(*p)))
    .collect();
    same_shape(
        &made,
        &want,
        DMat4::from_translation(DVec3::new(1.0, 0.0, -2.0)),
    );
}
