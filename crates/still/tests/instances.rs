use pfx_build::Registry;
use pfx_still::{Film, Noise, Reel, Seed, Take, TracedPiece};
use std::collections::BTreeMap;
use std::path::PathBuf;

const FPS: f64 = 24.0;
const SIZE: [u32; 2] = [108, 192];
const CARDS: usize = 57;
const CARD: [f64; 2] = [1.6, 5.0];
const POLE: [f32; 3] = [0.05, 4.0, 0.05];
const POLE_AT: [f32; 3] = [9.0, 2.0, 2.2];
const SLANT: f64 = 37.0;

#[derive(Clone, Copy, PartialEq)]
enum Face {
    Front,
    Back,
    Slanted,
}

fn face(index: usize) -> Face {
    match index % 3 {
        0 => Face::Back,
        1 => Face::Front,
        _ => Face::Slanted,
    }
}

fn card_at(index: usize) -> [f64; 3] {
    let side = if index.is_multiple_of(2) { 4.0 } else { -4.0 };
    [10.0 + 3.0 * index as f64, CARD[1] * 0.5, side]
}

fn card_turn(index: usize) -> [f64; 3] {
    let near = index.is_multiple_of(2);
    let stand = if near { -90.0 } else { 90.0 };
    let turn = match face(index) {
        Face::Front => 0.0,
        Face::Back => 180.0,
        Face::Slanted if near => SLANT,
        Face::Slanted => -SLANT,
    };
    [stand, turn, 0.0]
}

fn turned(degrees: [f64; 3], p: [f64; 3]) -> [f64; 3] {
    let [x, y, z] = degrees.map(f64::to_radians);
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let p = [p[0], cx * p[1] - sx * p[2], sx * p[1] + cx * p[2]];
    let p = [cy * p[0] + sy * p[2], p[1], -sy * p[0] + cy * p[2]];
    let (sz, cz) = z.sin_cos();
    [cz * p[0] - sz * p[1], sz * p[0] + cz * p[1], p[2]]
}

fn recipe() -> PathBuf {
    let mut text = String::from(
        r#"recipe = 1
name = "street"

[timeline]
fps = 24
frames = 9

[materials.asphalt]
base = [0.3, 0.3, 0.3]
roughness = 0.9

[materials.leaf]
base = [0.1, 0.6, 0.1]
roughness = 0.8

[materials.back]
base = [0.6, 0.05, 0.6]
roughness = 0.8

[materials.slant]
base = [0.6, 0.5, 0.05]
roughness = 0.8

[materials.paint]
base = [0.1, 0.2, 0.7]
roughness = 0.4

[materials.metal]
base = [0.8, 0.1, 0.1]
roughness = 0.5

[sun]
model = "authored"
toward = [0.2, 0.7, 0.6]
radius_deg = 0.5
color = [1.0, 1.0, 1.0]
irradiance = 3.0

[sky]
kind = "analytic"

[motion_blur]
shutter = 0.5

[[camera]]
name = "chase"
position = [0.0, 1.6, 0.0]
target = [10.0, 1.8, 0.0]
fov_y_deg = 70.0
keys = { position = [[0.0, [0.0, 1.6, 0.0]], [0.4, [3.0, 1.6, 0.2]]], target = [[0.0, [10.0, 1.8, 0.0]], [0.4, [13.0, 1.8, 0.2]]] }

[[object]]
name = "road"
kind = "box"
size = [400.0, 0.1, 30.0]
material = "asphalt"
at = [150.0, -0.05, 0.0]

[[object]]
name = "car"
kind = "box"
size = [4.0, 1.2, 1.8]
material = "paint"
at = [6.0, 0.6, 0.0]
keys = { at = [[0.0, [6.0, 0.6, 0.0]], [0.4, [9.5, 0.6, 0.3]]] }

[[particles]]
name = "spray"
parent = "car"
at = [-2.1, -0.4, 0.0]
rate = 300.0
max = 120
prewarm = 0.3
lifetime = [0.1, 0.3]
speed = [0.5, 1.5]
direction = [-1.0, 0.2, 0.0]
cone = 20.0
drag = 3.0
size = [0.02, 0.06]
alpha = [[0.0, 0.0], [0.1, 0.5], [1.0, 0.0]]
color = [0.8, 0.82, 0.85]
lit = true
seed = 7
"#,
    );
    text.push_str(&format!(
        "\n[[object]]\nname = \"pole\"\nkind = \"box\"\nsize = {POLE:?}\nmaterial = \"metal\"\nat = {POLE_AT:?}\n"
    ));
    for index in 0..CARDS {
        let material = match face(index) {
            Face::Back => "back",
            Face::Front => "leaf",
            Face::Slanted => "slant",
        };
        text.push_str(&format!(
            "\n[[object]]\nname = \"card_{index:02}\"\nkind = \"plane\"\nsize = {CARD:?}\nmaterial = \"{material}\"\ntwo_sided = true\nat = {:?}\nrotate = {:?}\n",
            card_at(index),
            card_turn(index)
        ));
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = root.join("tmp/still-tests/instances");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("street.toml");
    std::fs::write(&path, text).unwrap();
    path
}

fn take() -> Take {
    Take {
        size: SIZE,
        samples: 16,
        noise: Some(
            Noise {
                threshold: 0.01,
                min_samples: 8,
            }
            .floored(),
        ),
        seed: Seed::Locked,
    }
}

fn expected(object: &str) -> Option<([f32; 3], [f32; 3])> {
    if object == "pole" {
        return Some((
            std::array::from_fn(|axis| POLE_AT[axis] - POLE[axis] * 0.5),
            std::array::from_fn(|axis| POLE_AT[axis] + POLE[axis] * 0.5),
        ));
    }
    let index: usize = object.strip_prefix("card_")?.parse().ok()?;
    let at = card_at(index);
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for (u, v) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let corner = turned(
            card_turn(index),
            [u * CARD[0] * 0.5, 0.0, v * CARD[1] * 0.5],
        );
        for axis in 0..3 {
            lo[axis] = lo[axis].min(at[axis] + corner[axis]);
            hi[axis] = hi[axis].max(at[axis] + corner[axis]);
        }
    }
    Some((lo.map(|v| v as f32), hi.map(|v| v as f32)))
}

fn faults(
    frame: u32,
    listed: &[TracedPiece],
    first: &mut BTreeMap<String, [f32; 12]>,
) -> Vec<String> {
    let mut found = Vec::new();
    let mut fault = |text: String| found.push(format!("frame {frame}: {text}"));
    let pieces: Vec<&TracedPiece> = listed
        .iter()
        .filter(|piece| piece.object.is_some())
        .collect();
    for name in ["road", "car", "pole"]
        .into_iter()
        .map(String::from)
        .chain((0..CARDS).map(|index| format!("card_{index:02}")))
    {
        let mine: Vec<&&TracedPiece> = pieces
            .iter()
            .filter(|piece| piece.object.as_deref() == Some(name.as_str()))
            .collect();
        if mine.len() != 1 {
            fault(format!(
                "{name} has {} traced instances, expected 1",
                mine.len()
            ));
            continue;
        }
        let traced = &mine[0].traced;
        if traced.mesh.is_none() || traced.transform.is_none() {
            fault(format!("{name} is staged but not in the trace"));
            continue;
        }
        if traced.squash().is_none_or(|squash| squash < 0.999) {
            fault(format!("{name} is placed by a squashed transform"));
        }
        let card = name.starts_with("card_");
        let determinant = traced.determinant().unwrap_or(0.0);
        if determinant <= 0.0 || (card && (determinant - 1.0).abs() > 1e-3) {
            fault(format!(
                "{name} is placed by a transform of determinant {determinant}, expected a turn"
            ));
        }
        if card && traced.one_sided {
            fault(format!("{name} is two-sided and traced one-sided"));
        }
        if traced.shadow_only || !traced.casts_shadow || traced.clipped {
            fault(format!(
                "{name} is traced shadow only {}, casting a shadow {}, clipped {}",
                traced.shadow_only, traced.casts_shadow, traced.clipped
            ));
        }
        let moves = name == "car";
        if traced.moving != moves {
            fault(format!(
                "{name} moving is {}, expected {moves}",
                traced.moving
            ));
        }
        if let Some(listed) = traced.hardware {
            let mask = if moves { 0 } else { 0xff };
            if listed.mask != Some(mask) || listed.mover != moves {
                fault(format!(
                    "{name} sits in the TLAS with mask {:?} and mover {}, expected mask {mask} and mover {moves}",
                    listed.mask, listed.mover
                ));
            }
            if listed.rows.is_none() || listed.rows != traced.rows {
                fault(format!(
                    "{name}'s TLAS slot holds {:?}, and the trace places it by {:?}",
                    listed.rows, traced.rows
                ));
            }
            if listed.custom != traced.record {
                fault(format!(
                    "{name}'s TLAS slot points at record {:?}, expected {:?}",
                    listed.custom, traced.record
                ));
            }
            if !moves && let Some(rows) = listed.rows {
                let was = *first.entry(name.clone()).or_insert(rows);
                if was != rows {
                    fault(format!(
                        "{name} stands still and its TLAS slot moved from {was:?} to {rows:?}"
                    ));
                }
            }
        }
        if let (Some((lo, hi)), Some(bounds)) = (expected(&name), traced.bounds) {
            let off = (0..3)
                .map(|axis| {
                    (bounds.lo[axis] - lo[axis])
                        .abs()
                        .max((bounds.hi[axis] - hi[axis]).abs())
                })
                .fold(0.0, f32::max);
            if off > 1e-3 {
                fault(format!(
                    "{name} is traced within {:?}..{:?}, expected {lo:?}..{hi:?}",
                    bounds.lo, bounds.hi
                ));
            }
        }
    }
    let cards: Vec<&&TracedPiece> = pieces
        .iter()
        .filter(|piece| {
            piece
                .object
                .as_deref()
                .is_some_and(|name| name.starts_with("card_"))
        })
        .collect();
    let meshes: std::collections::BTreeSet<Option<usize>> =
        cards.iter().map(|piece| piece.traced.mesh).collect();
    if meshes.len() != 1 || cards.iter().any(|piece| piece.traced.instanced < CARDS) {
        fault(format!(
            "the {CARDS} cards trace on {} meshes, expected one mesh they all share",
            meshes.len()
        ));
    }
    let sprites: Vec<&TracedPiece> = listed
        .iter()
        .filter(|piece| piece.object.is_none() && piece.traced.sprite)
        .collect();
    if frame > 0 && sprites.is_empty() {
        fault("the spray places no sprites, so the frame never places them in the TLAS".into());
    }
    for sprite in sprites {
        let Some(listed) = sprite.traced.hardware else {
            continue;
        };
        if listed.rows != sprite.traced.rows || listed.custom != sprite.traced.record {
            fault(format!(
                "{}'s TLAS slot holds {:?} for record {:?}, and the trace places it by {:?} as record {:?}",
                sprite.name, listed.rows, listed.custom, sprite.traced.rows, sprite.traced.record
            ));
        }
        let mask = if sprite.traced.moving {
            0
        } else {
            pfx_trace::rt::SPRITE_MASK
        };
        if listed.mask != Some(mask) {
            fault(format!(
                "{} sits in the TLAS with mask {:?}, expected the sprite mask {mask}",
                sprite.name, listed.mask
            ));
        }
    }
    found
}

#[derive(Clone, Copy, Debug)]
struct Seen {
    front: usize,
    back: usize,
    slanted: usize,
    pole: usize,
}

fn seen(film: &Film) -> Seen {
    let count = |test: &dyn Fn(f32, f32, f32) -> bool| {
        film.color
            .iter()
            .filter(|[r, g, b, _]| test(*r, *g, *b))
            .count()
    };
    Seen {
        front: count(&|r, g, b| g > 0.02 && g > 2.0 * r && g > 2.0 * b),
        back: count(&|r, g, b| r.min(b) > 0.02 && r > 2.5 * g && b > 2.5 * g && b > 0.4 * r),
        slanted: count(&|r, g, b| r.min(g) > 0.02 && r > 2.5 * b && g > 2.5 * b && g > 0.5 * r),
        pole: count(&|r, g, b| r > 0.02 && r > 3.0 * g && r > 3.0 * b),
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_traced_clip_holds_every_copy_of_a_shared_two_sided_card_from_either_side_and_a_thin_pole_among_sprites_frame_after_frame()
 {
    let recipe = recipe();
    let mut reel = Reel::open(&recipe, None, &[], None, &Registry::new()).unwrap();
    let mut found = Vec::new();
    let mut first = BTreeMap::new();
    let mut all = Vec::new();
    let mut hardware = false;
    for frame in 0..4 {
        let camera = reel.pose_moving(f64::from(frame) / FPS, "chase").unwrap();
        let film = reel.frame(&camera, take(), &mut |_| {}).unwrap().film;
        let listed = reel
            .traced_instances()
            .expect("a traced frame lists its instances");
        hardware |= listed.iter().any(|piece| piece.traced.hardware.is_some());
        found.extend(faults(frame, &listed, &mut first));
        all.push(seen(&film));
    }
    if !hardware {
        eprintln!("the trace ran on the software BVH, so no TLAS slot was checked");
    }
    eprintln!("pixels drawn by frame: {all:?}");
    assert!(found.is_empty(), "{}", found.join("\n"));
    for (frame, seen) in all.iter().enumerate() {
        assert!(
            seen.front > 150 && seen.slanted > 400,
            "frame {frame}: only {} pixels show the cards facing the road and {} the cards slanted toward it",
            seen.front,
            seen.slanted
        );
        assert!(
            seen.back >= 50,
            "frame {frame}: only {} pixels show the cards turned away, so a two-sided copy lost its back face",
            seen.back
        );
        assert!(
            seen.pole >= 10,
            "frame {frame}: only {} pixels show the pole",
            seen.pole
        );
    }
    let start = all[0];
    for (frame, seen) in all.iter().enumerate().skip(1) {
        assert!(
            seen.front * 10 > start.front * 8
                && seen.slanted * 10 > start.slanted * 8
                && seen.pole * 10 > start.pole * 6,
            "frame {frame} lost what frame 0 drew: {seen:?} against {start:?}"
        );
    }
}
