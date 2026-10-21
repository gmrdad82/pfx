use pfx_build::{Rate, Registry, build_with};
use pfx_recipe_scene::pace::{TurnState, turn, turn_soon};
use pfx_recipe_scene::{Eye, Keyed, Stage, camera, headless};
use std::path::Path;
use std::time::Instant;

const GROUND: f64 = 40.0;
const FPS: u32 = 24;

struct Args {
    size: [u32; 2],
    copies: Vec<usize>,
    keyed: Vec<usize>,
    frames: u32,
    check: bool,
}

fn args() -> Result<Args, String> {
    let mut parsed = Args {
        size: [540, 960],
        copies: vec![0, 2_000, 8_000, 32_000],
        keyed: vec![0, 128],
        frames: 24,
        check: false,
    };
    let list = |text: String| -> Result<Vec<usize>, String> {
        text.split(',')
            .map(|n| n.parse().map_err(|e| format!("{n}: {e}")))
            .collect()
    };
    let mut items = std::env::args().skip(1);
    while let Some(item) = items.next() {
        let mut value = || items.next().ok_or(format!("{item} needs a value"));
        match item.as_str() {
            "--size" => {
                let text = value()?;
                let (w, h) = text.split_once('x').ok_or("--size is WxH")?;
                parsed.size = [
                    w.parse().map_err(|e| format!("--size: {e}"))?,
                    h.parse().map_err(|e| format!("--size: {e}"))?,
                ];
            }
            "--copies" => parsed.copies = list(value()?)?,
            "--keyed" => parsed.keyed = list(value()?)?,
            "--frames" => {
                parsed.frames = value()?.parse().map_err(|e| format!("--frames: {e}"))?;
            }
            "--check" => parsed.check = true,
            other => return Err(format!("unexpected argument {other}")),
        }
    }
    Ok(parsed)
}

fn recipe(copies: usize, keyed: usize) -> String {
    let mut text = format!(
        r#"recipe = 1
name = "crowd-cost"
seed = 5

[timeline]
fps = {FPS}
frames = 240

[[object]]
name = "ground"
kind = "plane"
size = [{GROUND}, {GROUND}]
material = "ground"

[[object]]
name = "leaf"
kind = "box"
size = [0.12, 0.01, 0.08]
material = "leaf"
hidden = true

[[camera]]
name = "main"
position = [0.0, 6.0, -14.0]
target = [0.0, 0.0, 4.0]
fov_y_deg = 50.0

[sun]
model = "authored"
toward = [0.5, 0.7, -0.3]
radius_deg = 0.27
color = [1.0, 0.96, 0.9]
irradiance = 3.0

[materials.ground]
family = "plain"
base = [0.35, 0.33, 0.3]
roughness = 0.9

[materials.leaf]
family = "plain"
base = [0.6, 0.35, 0.12]
roughness = 0.7
"#
    );
    if copies > 0 {
        let density = copies as f64 / (GROUND * GROUND);
        text.push_str(&format!(
            r#"
[[scatter]]
name = "leaves"
object = "leaf"
on = "ground"
seed = 9
density = {density}
rotate_max = [0.0, 360.0, 0.0]
scale_min = 0.6
scale_max = 1.4
"#
        ));
    }
    for k in 0..keyed {
        let x = (k % 16) as f64 - 7.5;
        let z = (k / 16) as f64 - 4.0;
        text.push_str(&format!(
            r#"
[[object]]
name = "mover{k}"
kind = "box"
size = [0.3, 0.3, 0.3]
material = "leaf"
at = [{x}, 0.15, {z}]
keys = {{ at = [[0.0, [{x}, 0.15, {z}]], [10.0, [{x}, 2.15, {z}]]], rotate = [[0.0, [0.0, 0.0, 0.0]], [10.0, [0.0, 720.0, 0.0]]] }}
"#
        ));
    }
    text
}

struct Cost {
    copies: usize,
    keyed: usize,
    instances: usize,
    packed: usize,
    draws: u32,
    pose_ms: f64,
    wall_ms: f64,
    gpu_ms: f64,
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(0.0)
}

fn measure(
    gpu: &pfx_gpu::Gpu,
    args: &Args,
    copies: usize,
    keyed: usize,
    turns: &mut TurnState,
) -> Result<Cost, String> {
    let text = recipe(copies, keyed);
    let parsed = pfx_recipe::parse(&text).map_err(|e| e.to_string())?;
    let built = build_with(&parsed, Path::new("."), &Registry::new()).map_err(|e| e.to_string())?;
    let recipe = built.recipe(&parsed);
    let mut stage =
        Stage::with_parts(gpu.clone(), &recipe, &built.parts, args.size).map_err(|e| e.0)?;
    stage.blur_motion(true);
    let target = gpu.offscreen(args.size[0], args.size[1], stage.output_format())?;
    let shot = camera(&recipe, "main").map_err(|e| e.0)?;
    stage.look_through(Some(shot));
    let eye = Eye::from_camera(shot, f64::from(args.size[0]) / f64::from(args.size[1]));
    let mut keys = Keyed::of(&built, Rate::whole(FPS));
    stage.bind_keys(keys.names());
    let (mut pose_ms, mut wall_ms, mut gpu_ms) = (Vec::new(), Vec::new(), Vec::new());
    for frame in 0..args.frames + 4 {
        let time = f64::from(frame) / f64::from(FPS);
        let started = Instant::now();
        stage.set_keyed(keys.sample(&built, time));
        let posed = started.elapsed().as_secs_f64() * 1e3;
        let measured = stage
            .render_measured(&eye, time as f32, &target.view)
            .map_err(|e| e.0)?;
        let wall = started.elapsed().as_secs_f64() * 1e3;
        let ms = measured.unwrap_or(0.0);
        turn_soon(turns, ms.max(4.0));
        if frame >= 4 {
            pose_ms.push(posed);
            wall_ms.push(wall);
            gpu_ms.push(ms);
        }
    }
    Ok(Cost {
        copies: built
            .parts
            .iter()
            .filter(|part| part.scattered.is_some())
            .count(),
        keyed,
        instances: stage.instance_count(),
        packed: stage.packed_records(),
        draws: stage.draw_count(),
        pose_ms: median(pose_ms),
        wall_ms: median(wall_ms),
        gpu_ms: median(gpu_ms),
    })
}

fn main() -> Result<(), String> {
    let args = args()?;
    let gpu = headless().map_err(|e| e.0)?;
    let mut turns = TurnState::default();
    let mut costs = Vec::new();
    println!("copies\tkeyed\tinstances\tpacked\tdraws\tpose_ms\twall_ms\tgpu_ms");
    for &keyed in &args.keyed {
        for &copies in &args.copies {
            let cost = measure(&gpu, &args, copies, keyed, &mut turns)?;
            println!(
                "{}\t{}\t{}\t{}\t{}\t{:.3}\t{:.2}\t{:.2}",
                cost.copies,
                cost.keyed,
                cost.instances,
                cost.packed,
                cost.draws,
                cost.pose_ms,
                cost.wall_ms,
                cost.gpu_ms
            );
            costs.push(cost);
            turn();
        }
    }
    if args.check {
        for keyed in &args.keyed {
            let row: Vec<&Cost> = costs.iter().filter(|c| c.keyed == *keyed).collect();
            let Some(first) = row.first() else { continue };
            for cost in &row {
                let chunks = cost.copies.div_ceil(pfx_live::crowd::CHUNK) as u32;
                if cost.instances != first.instances
                    || cost.packed != first.packed
                    || cost.draws > first.draws + chunks
                {
                    return Err(format!(
                        "{} copies drew {} instances, packed {} records in {} draws, {} copies {}, {} in {}: the frame's work grew with the copies",
                        cost.copies,
                        cost.instances,
                        cost.packed,
                        cost.draws,
                        first.copies,
                        first.instances,
                        first.packed,
                        first.draws
                    ));
                }
            }
        }
        println!(
            "flat: instances and packed records do not grow with the copies and draws only by chunks"
        );
    }
    Ok(())
}
