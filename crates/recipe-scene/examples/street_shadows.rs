use pfx_gpu::Gpu;
use pfx_recipe_scene::pace::{TurnState, turn, turn_soon};
use pfx_recipe_scene::{Eye, Stage, headless, write_png};
use std::path::{Path, PathBuf};

const RECIPE: &str = r#"
recipe = 1
name = "street-shadows"
seed = 7

[[route]]
name = "main"
points = [[0.0, 0.0, -200.0], [0.0, 0.0, 200.0]]

[[object]]
name = "road"
kind = "generator"
generator = "pfx/road"
params = { route = "main", lanes = 2, lane_width = 3.5, sidewalk = 4.0, seed = 1 }

[[object]]
name = "tower"
kind = "box"
at = [10.0, 15.0, 7.0]
size = [6.0, 30.0, 6.0]
material = "tower"
shadow = "CAST"

[[camera]]
name = "main"
kind = "perspective"
position = [0.0, 1.2, 0.0]
target = [0.0, 1.0, 20.0]
fov_y_deg = 50.0

[sun]
model = "authored"
toward = [0.6, 0.5, -0.4]
radius_deg = 0.27
color = [1.0, 0.95, 0.88]
irradiance = 3.0

[materials.tower]
family = "plain"
base = [0.45, 0.42, 0.4]
roughness = 0.8
"#;

const CORNER: [f64; 2] = [7.0, 4.0];
const ALONG: [f64; 2] = [-0.6, 0.4];
const ROAD_HALF: f64 = 3.0;
const EYE: [f64; 3] = [0.0, 1.2, 0.0];
const LOOK: [f64; 3] = [0.0, 1.0, 20.0];
const FOV_Y: f64 = 50.0;
const FPS: f64 = 60.0;
const SPEED: f64 = 1.0;
const SETTLE: u32 = 48;
const SEARCH: i64 = 28;

struct Args {
    size: [u32; 2],
    frames: u32,
    out: PathBuf,
    bench: bool,
}

fn args() -> Result<Args, String> {
    let mut parsed = Args {
        size: [1920, 1080],
        frames: 120,
        out: PathBuf::from("tmp/street-shadows"),
        bench: false,
    };
    let mut items = std::env::args().skip(1);
    while let Some(item) = items.next() {
        let mut value = || items.next().ok_or(format!("{item} needs a value"));
        match item.as_str() {
            "--size" => {
                let text = value()?;
                let (w, h) = text.split_once('x').ok_or("--size is WxH")?;
                parsed.size = [
                    w.parse().map_err(|_| "--size is WxH")?,
                    h.parse().map_err(|_| "--size is WxH")?,
                ];
            }
            "--frames" => parsed.frames = value()?.parse().map_err(|_| "--frames is a count")?,
            "--out" => parsed.out = PathBuf::from(value()?),
            "--bench" => parsed.bench = true,
            other => return Err(format!("unexpected argument {other}")),
        }
    }
    Ok(parsed)
}

struct Lane {
    stage: Stage,
    target: pfx_gpu::OffscreenTarget,
}

impl Lane {
    fn new(gpu: &Gpu, cast: &str, size: [u32; 2], base: &Path) -> Result<Lane, String> {
        let recipe = pfx_recipe::parse(&RECIPE.replace("CAST", cast)).map_err(|e| e.to_string())?;
        let built = pfx_build::build(&recipe, base).map_err(|e| e.to_string())?;
        let stage = Stage::new(gpu.clone(), &recipe, &built, size).map_err(|e| e.0)?;
        let target = gpu.offscreen(size[0], size[1], stage.output_format())?;
        Ok(Lane { stage, target })
    }

    fn draw(&mut self, eye: &Eye, frame: u32, turns: &mut TurnState) -> Result<f64, String> {
        let ms = self
            .stage
            .render_measured(eye, frame as f32 / FPS as f32, &self.target.view)
            .map_err(|e| e.0)?
            .unwrap_or(16.0);
        turn_soon(turns, ms);
        Ok(ms)
    }

    fn read(&self) -> Result<Vec<u8>, String> {
        self.stage.gpu().readback_rgba8(&self.target)
    }

    fn passes(&self) -> (f64, f64, f64) {
        let Some(timings) = self.stage.timings() else {
            return (0.0, 0.0, 0.0);
        };
        let mut cascades = 0.0;
        let mut opaque = 0.0;
        let mut total = 0.0;
        for pass in &timings.passes {
            total += pass.milliseconds;
            if pass.label.starts_with("shadow cascade") {
                cascades += pass.milliseconds;
            } else if pass.label.starts_with("opaque") {
                opaque += pass.milliseconds;
            }
        }
        (cascades, opaque, total)
    }
}

fn eye_at(z: f64) -> Eye {
    Eye {
        position: [EYE[0], EYE[1], EYE[2] + z],
        target: [LOOK[0], LOOK[1], LOOK[2] + z],
        up: [0.0, 1.0, 0.0],
        fov_y_deg: FOV_Y,
        shift: [0.0, 0.0],
        ortho_height: None,
    }
}

fn project(eye: &Eye, size: [u32; 2], world: [f64; 3]) -> Option<[f64; 2]> {
    let [right, up, forward] = eye.basis();
    let rel = [
        world[0] - eye.position[0],
        world[1] - eye.position[1],
        world[2] - eye.position[2],
    ];
    let dot = |a: [f64; 3]| a[0] * rel[0] + a[1] * rel[1] + a[2] * rel[2];
    let depth = dot(forward);
    if depth <= 1e-3 {
        return None;
    }
    let tan_y = (eye.fov_y_deg.to_radians() * 0.5).tan();
    let tan_x = tan_y * f64::from(size[0]) / f64::from(size[1]);
    let x = dot(right) / (depth * tan_x);
    let y = dot(up) / (depth * tan_y);
    Some([
        (x * 0.5 + 0.5) * f64::from(size[0]),
        (0.5 - y * 0.5) * f64::from(size[1]),
    ])
}

fn linear(value: u8) -> f64 {
    let c = f64::from(value) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(rgba: &[u8]) -> Vec<f64> {
    rgba.chunks_exact(4)
        .map(|p| 0.2126 * linear(p[0]) + 0.7152 * linear(p[1]) + 0.0722 * linear(p[2]))
        .collect()
}

fn ratio(shadowed: &[u8], open: &[u8]) -> Vec<f64> {
    luminance(shadowed)
        .into_iter()
        .zip(luminance(open))
        .map(|(a, b)| if b > 1e-5 { (a / b).min(1.5) } else { 1.0 })
        .collect()
}

fn sample(image: &[f64], size: [u32; 2], at: [f64; 2]) -> Option<f64> {
    let (w, h) = (f64::from(size[0]), f64::from(size[1]));
    let (x, y) = (at[0] - 0.5, at[1] - 0.5);
    if x < 0.0 || y < 0.0 || x >= w - 1.0 || y >= h - 1.0 {
        return None;
    }
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let index = |cx: f64, cy: f64| image[cy as usize * size[0] as usize + cx as usize];
    let top = index(x0, y0) * (1.0 - fx) + index(x0 + 1.0, y0) * fx;
    let bottom = index(x0, y0 + 1.0) * (1.0 - fx) + index(x0 + 1.0, y0 + 1.0) * fx;
    Some(top * (1.0 - fy) + bottom * fy)
}

fn boundary(x: f64) -> [f64; 3] {
    let s = (x - CORNER[0]) / ALONG[0];
    [x, 0.0, CORNER[1] + ALONG[1] * s]
}

struct Edge {
    columns: usize,
    rms: f64,
    worst: f64,
    width: f64,
}

fn edge(image: &[f64], size: [u32; 2], eye: &Eye) -> Edge {
    let ends = [
        project(eye, size, boundary(ROAD_HALF)),
        project(eye, size, boundary(-ROAD_HALF)),
    ];
    let [Some(a), Some(b)] = ends else {
        return Edge {
            columns: 0,
            rms: f64::NAN,
            worst: f64::NAN,
            width: f64::NAN,
        };
    };
    let (left, right) = if a[0] < b[0] { (a, b) } else { (b, a) };
    let mut points = Vec::new();
    let mut widths = Vec::new();
    let (w, h) = (size[0] as i64, size[1] as i64);
    let first = left[0].ceil() as i64 + 4;
    let last = right[0].floor() as i64 - 4;
    for column in first.max(0)..=last.min(w - 1) {
        let t = (column as f64 - left[0]) / (right[0] - left[0]);
        let expected = (left[1] + (right[1] - left[1]) * t).round() as i64;
        let rows: Vec<i64> = (expected - SEARCH..=expected + SEARCH)
            .filter(|row| (0..h).contains(row))
            .collect();
        if rows.len() < (2 * SEARCH) as usize {
            continue;
        }
        let profile: Vec<f64> = rows
            .iter()
            .map(|&row| image[(row * w + column) as usize])
            .collect();
        let tail = 6;
        let top = profile[..tail].iter().sum::<f64>() / tail as f64;
        let bottom = profile[profile.len() - tail..].iter().sum::<f64>() / tail as f64;
        if (bottom - top).abs() < 0.2 {
            continue;
        }
        let crossing = |level: f64| {
            let target = top + (bottom - top) * level;
            profile.windows(2).enumerate().find_map(|(i, pair)| {
                let (p, q) = (pair[0] - target, pair[1] - target);
                (p * q <= 0.0 && p != q).then(|| rows[i] as f64 + p / (p - q))
            })
        };
        let (Some(mid), Some(low), Some(high)) = (crossing(0.5), crossing(0.1), crossing(0.9))
        else {
            continue;
        };
        points.push((column as f64, mid));
        widths.push((high - low).abs());
    }
    let n = points.len() as f64;
    if points.len() < 8 {
        return Edge {
            columns: points.len(),
            rms: f64::NAN,
            worst: f64::NAN,
            width: f64::NAN,
        };
    }
    let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let sxy = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum::<f64>();
    let sxx = points.iter().map(|p| (p.0 - mx).powi(2)).sum::<f64>();
    let slope = sxy / sxx.max(1e-9);
    let residuals: Vec<f64> = points
        .iter()
        .map(|p| p.1 - (my + slope * (p.0 - mx)))
        .collect();
    let rms = (residuals.iter().map(|r| r * r).sum::<f64>() / n).sqrt();
    let worst = residuals.iter().fold(0.0f64, |m, r| m.max(r.abs()));
    widths.sort_by(f64::total_cmp);
    Edge {
        columns: points.len(),
        rms,
        worst,
        width: widths[widths.len() / 2],
    }
}

fn band() -> Vec<[f64; 3]> {
    let mut points = Vec::new();
    let mut x = -ROAD_HALF;
    while x <= ROAD_HALF {
        let centre = boundary(x);
        let mut dz = -0.6;
        while dz <= 0.6 {
            points.push([x, 0.0, centre[2] + dz]);
            dz += 0.01;
        }
        x += 0.02;
    }
    points
}

fn crawl(previous: &[Option<f64>], current: &[Option<f64>]) -> f64 {
    let mut sum = 0.0;
    let mut count = 0usize;
    for (a, b) in previous.iter().zip(current) {
        if let (Some(a), Some(b)) = (a, b) {
            sum += (a - b).abs();
            count += 1;
        }
    }
    if count == 0 {
        f64::NAN
    } else {
        sum / count as f64
    }
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn grey(image: &[f64]) -> Vec<u8> {
    image
        .iter()
        .flat_map(|&v| {
            let g = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            [g, g, g, 255]
        })
        .collect()
}

fn run() -> Result<(), String> {
    let args = args()?;
    std::fs::create_dir_all(&args.out).map_err(|e| e.to_string())?;
    let gpu = headless().map_err(|e| e.0)?;
    let size = args.size;
    let mut turns = TurnState::default();
    let mut cast = Lane::new(&gpu, "cast", size, &args.out)?;
    if args.bench {
        for frame in 0..SETTLE {
            cast.draw(&eye_at(0.0), frame, &mut turns)?;
        }
        let (mut cascades, mut opaque, mut total) = (Vec::new(), Vec::new(), Vec::new());
        let (mut still_cascades, mut still_opaque, mut still_total) =
            (Vec::new(), Vec::new(), Vec::new());
        for frame in 0..args.frames {
            cast.draw(&eye_at(0.0), SETTLE + frame, &mut turns)?;
            let (c, o, t) = cast.passes();
            still_cascades.push(c);
            still_opaque.push(o);
            still_total.push(t);
        }
        for frame in 0..args.frames {
            let z = SPEED * f64::from(frame) / FPS;
            cast.draw(&eye_at(z), SETTLE + args.frames + frame, &mut turns)?;
            let (c, o, t) = cast.passes();
            cascades.push(c);
            opaque.push(o);
            total.push(t);
        }
        turn();
        println!(
            "{{\"size\":[{},{}],\"frames\":{},\"still\":{{\"cascades_ms\":{:.4},\"opaque_ms\":{:.4},\"frame_ms\":{:.4}}},\"dolly\":{{\"cascades_ms\":{:.4},\"opaque_ms\":{:.4},\"frame_ms\":{:.4}}}}}",
            size[0],
            size[1],
            args.frames,
            median(&mut still_cascades),
            median(&mut still_opaque),
            median(&mut still_total),
            median(&mut cascades),
            median(&mut opaque),
            median(&mut total)
        );
        return Ok(());
    }
    let mut open = Lane::new(&gpu, "none", size, &args.out)?;
    for frame in 0..SETTLE {
        cast.draw(&eye_at(0.0), frame, &mut turns)?;
        open.draw(&eye_at(0.0), frame, &mut turns)?;
    }
    let shot = cast.read()?;
    let still = ratio(&shot, &open.read()?);
    let eye = eye_at(0.0);
    let measured = edge(&still, size, &eye);
    write_png(&args.out.join("cast.png"), size, &shot).map_err(|e| e.0)?;
    write_png(&args.out.join("ratio.png"), size, &grey(&still)).map_err(|e| e.0)?;
    let points = band();
    let at = |image: &[f64], eye: &Eye| -> Vec<Option<f64>> {
        points
            .iter()
            .map(|&p| project(eye, size, p).and_then(|px| sample(image, size, px)))
            .collect()
    };
    let mut floor = Vec::new();
    let mut previous = at(&still, &eye);
    for frame in 0..24 {
        cast.draw(&eye, SETTLE + frame, &mut turns)?;
        open.draw(&eye, SETTLE + frame, &mut turns)?;
        let image = ratio(&cast.read()?, &open.read()?);
        let current = at(&image, &eye);
        floor.push(crawl(&previous, &current));
        previous = current;
    }
    let start = SETTLE + 24;
    let mut moving = Vec::new();
    let mut edges = Vec::new();
    for frame in 0..args.frames {
        let eye = eye_at(SPEED * f64::from(frame) / FPS);
        cast.draw(&eye, start + frame, &mut turns)?;
        open.draw(&eye, start + frame, &mut turns)?;
        let image = ratio(&cast.read()?, &open.read()?);
        let current = at(&image, &eye);
        if frame > 0 {
            moving.push(crawl(&previous, &current));
        }
        if frame + 1 == args.frames {
            write_png(&args.out.join("ratio-moved.png"), size, &grey(&image)).map_err(|e| e.0)?;
        }
        edges.push(edge(&image, size, &eye).rms);
        previous = current;
    }
    turn();
    let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len().max(1) as f64;
    let worst = moving.iter().fold(0.0f64, |m, v| m.max(*v));
    println!(
        "{{\"size\":[{},{}],\"edge\":{{\"columns\":{},\"rms_px\":{:.4},\"worst_px\":{:.4},\"width_px\":{:.3}}},\"edge_dolly_rms_px\":{:.4},\"crawl\":{{\"still\":{:.5},\"dolly\":{:.5},\"dolly_worst\":{:.5}}}}}",
        size[0],
        size[1],
        measured.columns,
        measured.rms,
        measured.worst,
        measured.width,
        mean(&edges),
        mean(&floor),
        mean(&moving),
        worst
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("street_shadows: {error}");
        std::process::exit(1);
    }
}
