use std::collections::BTreeMap;
use std::path::Path;

use pfx_live::frame::OpaqueFeatures;
use pfx_live::shadow::{Mat4, View};
use pfx_trace::bvh::{Bvh, Ray};

use super::*;

const LIT: u8 = 10;
const DARK: f32 = 0.1;
const BAND: usize = 2;
const MARGIN: usize = 6;
const LIFT: f32 = 2e-4;
const PROBES_MM: [f32; 6] = [0.5, 1.0, 2.0, 4.0, 8.0, 16.0];

pub struct Scene<'a> {
    pub triangles: &'a [Triangle],
    pub owners: &'a [u32],
    pub material_names: &'a [String],
    pub materials: &'a [Material],
    pub radius_deg: f32,
}

#[derive(Clone, Copy, Default)]
struct Hit {
    id: u32,
    material: u32,
    point: [f32; 3],
}

struct Tracer<'a> {
    bvh: Bvh,
    triangles: &'a [Triangle],
    owners: &'a [u32],
    clips: BTreeMap<u32, [[f32; 4]; 2]>,
}

impl Tracer<'_> {
    fn clipped(&self, id: u32, point: [f32; 3]) -> bool {
        self.clips.get(&id).is_some_and(|planes| {
            planes.iter().any(|plane| {
                (plane[0] != 0.0 || plane[1] != 0.0 || plane[2] != 0.0)
                    && plane[0] * point[0] + plane[1] * point[1] + plane[2] * point[2] > plane[3]
            })
        })
    }

    fn first(&self, origin: [f32; 3], direction: [f32; 3]) -> Option<(f32, Hit)> {
        let mut start = origin;
        let mut travelled = 0.0;
        for _ in 0..16 {
            let found = self.bvh.intersect(
                self.triangles,
                Ray {
                    origin: start,
                    direction,
                },
                f32::INFINITY,
            )?;
            let point: [f32; 3] = std::array::from_fn(|k| start[k] + direction[k] * found.distance);
            let id = self.owners[found.index as usize];
            travelled += found.distance;
            if !self.clipped(id, point) {
                return Some((
                    travelled,
                    Hit {
                        id,
                        material: found.material,
                        point,
                    },
                ));
            }
            start = std::array::from_fn(|k| point[k] + direction[k] * 1e-5);
            travelled += 1e-5;
        }
        None
    }

    fn sunlit(&self, toward: [f32; 3], point: [f32; 3]) -> bool {
        let d = unit(toward);
        let o = std::array::from_fn(|k| point[k] + d[k] * LIFT);
        self.first(o, d).is_none()
    }
}

fn parallel<T: Send + Default + Clone>(count: usize, work: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = count.div_ceil(threads);
    let mut out = vec![T::default(); count];
    std::thread::scope(|scope| {
        for (index, slice) in out.chunks_mut(chunk).enumerate() {
            let work = &work;
            scope.spawn(move || {
                for (offset, slot) in slice.iter_mut().enumerate() {
                    *slot = work(index * chunk + offset);
                }
            });
        }
    });
    out
}

fn interior(ids: &[u32], width: usize, height: usize, at: usize, reach: usize) -> bool {
    let (x, y) = (at % width, at / width);
    if x < reach || y < reach || x + reach >= width || y + reach >= height {
        return false;
    }
    let id = ids[at];
    (y - reach..=y + reach).all(|yy| (x - reach..=x + reach).all(|xx| ids[yy * width + xx] == id))
}

fn gobo(sun: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let s = unit(sun);
    let horizontal = [-s[2], 0.0, s[0]];
    let l = (horizontal[0] * horizontal[0] + horizontal[2] * horizontal[2])
        .sqrt()
        .max(1e-6);
    let u = [horizontal[0] / l, 0.0, horizontal[2] / l];
    (u, cross(u, s))
}

struct Reference {
    width: usize,
    height: usize,
    hits: Vec<Hit>,
    classes: Vec<Option<bool>>,
    facing: Vec<f32>,
    skipped: usize,
}

#[derive(Clone, Default)]
struct Wrong {
    facing: [usize; 4],
    near: [usize; 5],
    blockers: BTreeMap<u32, usize>,
    own: [usize; PROBES_MM.len() + 1],
    other: [usize; PROBES_MM.len() + 1],
    outside: [usize; 2],
    reach: f32,
}

fn facing_bucket(facing: f32) -> usize {
    if facing < 0.0 {
        0
    } else if facing < 0.25 {
        1
    } else if facing < 0.4 {
        2
    } else {
        3
    }
}

struct Agreement {
    checked: usize,
    agree: usize,
    extra: usize,
    missing: usize,
    parts: BTreeMap<u32, (usize, usize, usize)>,
    judged: BTreeMap<u32, Vec<(usize, bool, bool)>>,
    painted: Vec<u8>,
}

impl Agreement {
    fn percent(&self) -> f64 {
        self.agree as f64 * 100.0 / self.checked.max(1) as f64
    }
}

fn reference<'a>(bench: &mut Bench, scene: &Scene<'a>, sun: [f32; 3]) -> (Reference, Tracer<'a>) {
    let (width, height) = (PLATE.0 as usize, PLATE.1 as usize);
    let clips = BTreeMap::new();
    let tracer = Tracer {
        bvh: Bvh::build(scene.triangles),
        triangles: scene.triangles,
        owners: scene.owners,
        clips,
    };
    let lens = &bench.lens;
    let forward = unit(sub(lens.target, lens.position));
    let right = unit(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    let tan_y = (lens.fov_y.to_radians() * 0.5).tan();
    let tan_x = tan_y * width as f32 / height as f32;
    let eye = lens.position;
    let shift = lens.shift;
    let hits: Vec<Hit> = parallel(width * height, |i| {
        let nx = ((i % width) as f32 + 0.5) / width as f32 * 2.0 - 1.0;
        let ny = 1.0 - ((i / width) as f32 + 0.5) / height as f32 * 2.0;
        let d = unit(std::array::from_fn(|k| {
            forward[k] + right[k] * (nx + shift[0]) * tan_x + up[k] * (ny + shift[1]) * tan_y
        }));
        tracer.first(eye, d).map(|(_, hit)| hit).unwrap_or_default()
    });
    let traced: Vec<u32> = hits.iter().map(|h| h.id).collect();
    let sun = unit(sun);
    let (u, v) = gobo(sun);
    let spread = scene.radius_deg.to_radians().tan();
    let disc: Vec<[f32; 3]> = [(0.0, 0.0), (1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)]
        .iter()
        .map(|&(a, b)| std::array::from_fn(|k| sun[k] + (u[k] * a + v[k] * b) * spread))
        .collect();
    let refined = std::env::var_os("BENCH_SHADOW_REFINED").is_some();
    let dark: Vec<bool> = scene
        .materials
        .iter()
        .map(|m| {
            0.2126 * m.base[0] + 0.7152 * m.base[1] + 0.0722 * m.base[2] < DARK
                || refined && (m.metalness >= 0.5 || m.emission.iter().any(|e| *e > 0.0))
        })
        .collect();
    let normal = |i: usize| -> Option<[f32; 3]> {
        if i % width + 1 >= width || i + width >= hits.len() {
            return None;
        }
        let (a, b, c) = (hits[i], hits[i + 1], hits[i + width]);
        if a.id != b.id || a.id != c.id {
            return None;
        }
        let n = cross(sub(b.point, a.point), sub(c.point, a.point));
        let l = dot(n, n).sqrt();
        (l > 1e-12).then(|| n.map(|x| -x / l))
    };
    let facing: Vec<f32> = (0..hits.len())
        .map(|i| normal(i).map_or(0.0, |n| dot(n, sun)))
        .collect();
    let classified: Vec<(Option<bool>, bool)> = parallel(hits.len(), |i| {
        let hit = hits[i];
        if hit.id == 0 || !interior(&traced, width, height, i, 2) {
            return (None, false);
        }
        if dark.get(hit.material as usize).copied().unwrap_or(true) {
            return (None, true);
        }
        if normal(i).is_none() {
            return (None, false);
        }
        if facing[i].abs() < 0.15 {
            return (None, false);
        }
        let first = tracer.sunlit(disc[0], hit.point);
        if disc[1..]
            .iter()
            .any(|d| tracer.sunlit(*d, hit.point) != first)
        {
            return (None, false);
        }
        (Some(first && facing[i] > 0.0), false)
    });
    let skipped = classified.iter().filter(|c| c.1).count();
    (
        Reference {
            width,
            height,
            hits,
            classes: classified.into_iter().map(|c| c.0).collect(),
            facing,
            skipped,
        },
        tracer,
    )
}

fn agreement(reference: &Reference, rgb: &[u8], ids: &[u32]) -> Agreement {
    let (width, height) = (reference.width, reference.height);
    let classes = &reference.classes;
    let stable = |i: usize| {
        let (x, y) = (i % width, i / width);
        let class = classes[i];
        class.is_some()
            && interior(ids, width, height, i, 2)
            && ids[i] == reference.hits[i].id
            && (y.saturating_sub(BAND)..(y + BAND + 1).min(height)).all(|yy| {
                (x.saturating_sub(BAND)..(x + BAND + 1).min(width))
                    .all(|xx| classes[yy * width + xx] == class)
            })
    };
    let mut out = Agreement {
        checked: 0,
        agree: 0,
        extra: 0,
        missing: 0,
        parts: BTreeMap::new(),
        judged: BTreeMap::new(),
        painted: vec![0u8; width * height * 4],
    };
    for (i, hit) in reference.hits.iter().enumerate() {
        out.painted[i * 4 + 3] = 255;
        if !stable(i) {
            continue;
        }
        let traced_lit = classes[i] == Some(true);
        let live_lit = rgb[i * 4..i * 4 + 3].iter().any(|&c| c > LIT);
        out.checked += 1;
        out.judged
            .entry(hit.id)
            .or_default()
            .push((i, traced_lit, live_lit));
        let entry = out.parts.entry(hit.id).or_default();
        entry.0 += 1;
        let pixel = match (traced_lit, live_lit) {
            (true, true) => {
                out.agree += 1;
                [200, 200, 200]
            }
            (false, false) => {
                out.agree += 1;
                [90, 90, 90]
            }
            (false, true) => {
                out.extra += 1;
                entry.1 += 1;
                [230, 30, 30]
            }
            (true, false) => {
                out.missing += 1;
                entry.2 += 1;
                [30, 30, 230]
            }
        };
        out.painted[i * 4..i * 4 + 3].copy_from_slice(&pixel);
    }
    out
}

fn masks(reference: &Reference, pixels: &[(usize, bool, bool)], dir: &Path, file: &str) {
    let width = reference.width;
    let height = reference.height;
    let xs = pixels.iter().map(|p| p.0 % width);
    let ys = pixels.iter().map(|p| p.0 / width);
    let x0 = xs.clone().min().unwrap_or(0).saturating_sub(MARGIN);
    let x1 = (xs.max().unwrap_or(0) + MARGIN + 1).min(width);
    let y0 = ys.clone().min().unwrap_or(0).saturating_sub(MARGIN);
    let y1 = (ys.max().unwrap_or(0) + MARGIN + 1).min(height);
    let (w, h) = (x1 - x0, y1 - y0);
    let blank: Vec<u8> = [40u8, 60, 120, 255].repeat(w * h);
    let (mut trace, mut live) = (blank.clone(), blank);
    let shade = |lit: bool| {
        if lit {
            [235u8, 235, 235, 255]
        } else {
            [20, 20, 20, 255]
        }
    };
    for &(i, traced_lit, live_lit) in pixels {
        let at = ((i / width - y0) * w + i % width - x0) * 4;
        trace[at..at + 4].copy_from_slice(&shade(traced_lit));
        live[at..at + 4].copy_from_slice(&shade(live_lit));
    }
    save_png(
        &dir.join(format!("{file}-trace.png")),
        w as u32,
        h as u32,
        &trace,
    );
    save_png(
        &dir.join(format!("{file}-live.png")),
        w as u32,
        h as u32,
        &live,
    );
}

fn diagnose(
    reference: &Reference,
    tracer: &Tracer,
    sun: [f32; 3],
    light: Mat4,
    pixels: &[(usize, bool, bool)],
) -> Wrong {
    let sun = unit(sun);
    let (u, v) = gobo(sun);
    let mut wrong = Wrong::default();
    for &(i, traced_lit, live_lit) in pixels {
        if traced_lit == live_lit {
            continue;
        }
        let hit = reference.hits[i];
        wrong.facing[facing_bucket(reference.facing[i])] += 1;
        let projected = light.transform_point(hit.point);
        if projected[0].abs() > 1.0 || projected[1].abs() > 1.0 {
            wrong.outside[0] += 1;
            wrong.reach = wrong.reach.max(projected[0].abs()).max(projected[1].abs());
        } else if !(0.0..=1.0).contains(&projected[2]) {
            wrong.outside[1] += 1;
        }
        if live_lit {
            let o = std::array::from_fn(|k| hit.point[k] + sun[k] * LIFT);
            match tracer.first(o, sun) {
                Some((distance, blocker)) => {
                    *wrong.blockers.entry(blocker.id).or_default() += 1;
                    let mm = (distance + LIFT) * 1000.0;
                    let bucket = if mm < 2.0 {
                        0
                    } else if mm < 10.0 {
                        1
                    } else if mm < 50.0 {
                        2
                    } else {
                        3
                    };
                    wrong.near[bucket] += 1;
                }
                None => wrong.near[4] += 1,
            }
        } else {
            let mut own = PROBES_MM.len();
            let mut other = PROBES_MM.len();
            for (step, mm) in PROBES_MM.iter().enumerate() {
                for turn in 0..8 {
                    let angle = turn as f32 * std::f32::consts::FRAC_PI_4;
                    let (s, c) = angle.sin_cos();
                    let offset: [f32; 3] =
                        std::array::from_fn(|k| (u[k] * c + v[k] * s) * mm * 1e-3);
                    let o = std::array::from_fn(|k| hit.point[k] + offset[k] + sun[k] * LIFT);
                    if let Some((_, blocker)) = tracer.first(o, sun) {
                        if blocker.id == hit.id {
                            own = own.min(step);
                        } else {
                            other = other.min(step);
                            *wrong.blockers.entry(blocker.id).or_default() += 1;
                        }
                    }
                }
                if other < PROBES_MM.len() {
                    break;
                }
            }
            wrong.own[own] += 1;
            wrong.other[other] += 1;
        }
    }
    wrong
}

fn view(bench: &Bench) -> View {
    let lens = &bench.lens;
    let forward = unit(sub(lens.target, lens.position));
    View {
        eye: lens.position,
        forward,
        up: [0.0, 1.0, 0.0],
        fov_y: lens.fov_y.to_radians(),
        aspect: PLATE.0 as f32 / PLATE.1 as f32,
        near: NEAR,
        far: FAR,
    }
}

fn black(bench: &mut Bench) {
    let sky = Sky {
        width: 1,
        height: 1,
        texels: vec![[0.0, 0.0, 0.0, 1.0]],
    };
    bench.renderer.set_sky(SkySource::Hdr(sky), HOUR).unwrap();
}

fn sun_only(bench: &mut Bench) {
    black(bench);
    let gpu = bench.renderer.gpu();
    let zero = ProbeLighting::fallback(&gpu.device, &gpu.queue, [0.0; 3]);
    bench.renderer.set_probes(zero).unwrap();
    bench
        .renderer
        .frame()
        .set_local_reflections(None, HOUR)
        .unwrap();
    bench.steam = None;
    bench.bird_mesh = None;
    bench.haze = None;
    bench.renderer.set_haze(None);
    bench.motes.clear();
    bench.glass.clear();
    bench.props.clear();
    bench.renderer.set_props(None);
    bench.words.shown = false;
    bench.moving_sun = false;
    bench.hide_unbaked();
    bench.resize(PLATE.0, PLATE.1);
}

fn plate(bench: &mut Bench) -> (Vec<u8>, Vec<u32>) {
    black(bench);
    let mut pace = manners::Pace::new();
    for number in 0..SETTLE_FRAMES {
        pace.frame(|| bench.render(number, true, Shown::Rest));
    }
    (bench.pixels(), bench.ids())
}

fn variant(base: Quality, spec: &str) -> (Quality, OpaqueFeatures) {
    let mut quality = base;
    let mut off = OpaqueFeatures::NONE;
    for pair in spec.split(',').filter(|p| !p.is_empty()) {
        let (key, value) = pair
            .split_once('=')
            .expect("variant settings are key=value");
        let number = || value.parse::<f32>().expect("a number");
        let flag = || value == "1" || value == "true";
        match key {
            "normal_bias" => quality.normal_bias = number(),
            "slope_bias" => quality.slope_bias = number(),
            "depth_bias" => quality.depth_bias = number(),
            "pcf_radius" => quality.pcf_radius = number(),
            "sun_radius_deg" => quality.sun_radius_deg = number(),
            "blocker_taps" => quality.blocker_taps = number() as u32,
            "filter_taps" => quality.filter_taps = number() as u32,
            "nearest_blocker" => quality.nearest_blocker = flag(),
            "umbra_skip" => quality.umbra_skip = flag(),
            "lit_skip" => quality.lit_skip = flag(),
            "layered" => quality.layered = flag(),
            "outer_scale" => quality.outer_scale = number(),
            "near_map" => quality.near_map = flag(),
            "soft_offset" => quality.soft_offset = number(),
            "caster_margin" => quality.caster_margin = number(),
            "resolution" => quality.resolution = number() as u32,
            "off" => {
                for name in value.split('+') {
                    off = off.union(OpaqueFeatures::named(name).expect("an opaque feature"));
                }
            }
            other => panic!("unknown shadow setting {other}"),
        }
    }
    (quality, off)
}

fn file_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn report(
    bench: &Bench,
    light: Mat4,
    scene: &Scene,
    reference: &Reference,
    tracer: &Tracer,
    sun: [f32; 3],
    label: &str,
    result: &Agreement,
    dir: &Path,
) {
    println!(
        "shadows {label}: {} px away from penumbrae, {:.2}% agree; {} lit in live but shadowed in the trace, {} shadowed in live but lit in the trace; {} px of dark materials left out",
        result.checked,
        result.percent(),
        result.extra,
        result.missing,
        reference.skipped
    );
    save_png(
        &dir.join(format!("shadows-{label}.png")),
        PLATE.0,
        PLATE.1,
        &result.painted,
    );
    let parts = dir.join(format!("parts-{label}"));
    std::fs::create_dir_all(&parts).unwrap();
    println!(
        "  part wrong/total extra missing | facing <0 <.25 <.4 more | extra's blocker <2 <10 <50 more mm, none | missing: own surface, other part within 0.5 1 2 4 8 16 mm, none | blockers"
    );
    let named: Vec<String> = std::env::var("BENCH_SHADOW_NAMES")
        .map(|list| list.split(',').map(str::to_string).collect())
        .unwrap_or_default();
    for (&id, &(total, extra, missing)) in &result.parts {
        let wrong = extra + missing;
        let name = &bench.objects[id as usize - 1];
        if wrong * 20 <= total {
            if named.contains(name) {
                println!("  {name} {wrong}/{total} {extra} {missing} (under 5%)");
            }
            continue;
        }
        let material = result.judged[&id]
            .first()
            .map(|&(i, _, _)| reference.hits[i].material as usize)
            .and_then(|m| scene.material_names.get(m))
            .map_or("?", String::as_str);
        let w = diagnose(reference, tracer, sun, light, &result.judged[&id]);
        let mut blockers: Vec<(usize, u32)> = w.blockers.iter().map(|(&id, &n)| (n, id)).collect();
        blockers.sort_by(|a, b| b.cmp(a));
        let blockers: Vec<String> = blockers
            .iter()
            .take(3)
            .map(|&(n, id)| format!("{} {n}", bench.objects[id as usize - 1]))
            .collect();
        println!(
            "  {name} ({material}) {wrong}/{total} {extra} {missing} | {:?} | outside xy, depth {:?} reach {:.2} | {:?} | {:?} {:?} | {}",
            w.facing,
            w.outside,
            w.reach,
            w.near,
            w.own,
            w.other,
            blockers.join(", "),
        );
        masks(reference, &result.judged[&id], &parts, &file_name(name));
        if std::env::var("BENCH_SHADOW_PROBE").is_ok_and(|probe| probe == *name) {
            probe(reference, tracer, sun, light, &result.judged[&id]);
        }
    }
}

fn probe(
    reference: &Reference,
    tracer: &Tracer,
    sun: [f32; 3],
    light: Mat4,
    pixels: &[(usize, bool, bool)],
) {
    let sun = unit(sun);
    let wrong: Vec<usize> = pixels.iter().filter(|p| p.1 != p.2).map(|p| p.0).collect();
    for &i in wrong.iter().step_by((wrong.len() / 8).max(1)).take(8) {
        let hit = reference.hits[i];
        let at = light.transform_point(hit.point);
        let mut chain = Vec::new();
        let mut origin: [f32; 3] = std::array::from_fn(|k| hit.point[k] + sun[k] * LIFT);
        let mut travelled = LIFT;
        for _ in 0..5 {
            let Some((distance, blocker)) = tracer.first(origin, sun) else {
                break;
            };
            travelled += distance;
            let depth = light.transform_point(blocker.point)[2];
            chain.push(format!(
                "{} at {:.2} mm (depth {:.6})",
                blocker.id,
                travelled * 1000.0,
                depth
            ));
            origin = std::array::from_fn(|k| blocker.point[k] + sun[k] * 1e-5);
            travelled += 1e-5;
        }
        println!(
            "    pixel ({}, {}) facing {:.2} light ndc ({:.4}, {:.4}) depth {:.6}: {}",
            i % reference.width,
            i / reference.width,
            reference.facing[i],
            at[0],
            at[1],
            at[2],
            chain.join("; ")
        );
    }
}

const BLOCK: u32 = 24;
const SKIP: u32 = 4;
const BLOCKS: u32 = 30;

fn pass_ms(timings: &[PassTiming], keep: impl Fn(&str) -> bool) -> f64 {
    timings
        .iter()
        .filter(|timing| keep(&timing.label))
        .map(|timing| timing.milliseconds)
        .sum()
}

fn spread(mut values: Vec<f64>) -> String {
    values.sort_by(f64::total_cmp);
    format!(
        "{:.3} [{:.3}–{:.3}]",
        percentile(&values, 0.5),
        percentile(&values, 0.1),
        percentile(&values, 0.9)
    )
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    percentile(&values, 0.5)
}

fn cost(bench: &mut Bench, name: &str, before: Quality, after: Quality) {
    let mut number = 0;
    for (shown, moving) in [
        (Shown::Rest, false),
        (Shown::Rest, true),
        (Shown::Moving, true),
    ] {
        let mut pace = manners::Pace::timed("bench", "shadow cost", u64::from(BLOCKS * 2 * BLOCK));
        let mut opaque = [Vec::new(), Vec::new()];
        let mut cascades = [Vec::new(), Vec::new()];
        let mut saved = [Vec::new(), Vec::new()];
        for block in 0..BLOCKS * 2 {
            let side = (block % 2) as usize;
            bench
                .renderer
                .set_shadow_quality(if side == 0 { before } else { after });
            let mut o = Vec::new();
            let mut c = Vec::new();
            for frame in 0..BLOCK {
                let timings = pace.frame(|| bench.render(number, !moving, shown));
                if number == 0 {
                    let labels: Vec<&str> = timings.iter().map(|t| t.label.as_str()).collect();
                    println!("passes: {labels:?}");
                }
                number += 1;
                if frame >= SKIP {
                    o.push(pass_ms(&timings, |label| label == "opaque PBR"));
                    c.push(pass_ms(&timings, |label| {
                        label.starts_with("shadow cascade")
                    }));
                }
            }
            opaque[side].push(median(o));
            cascades[side].push(median(c));
            if side == 1 {
                let last = opaque[1].len() - 1;
                saved[0].push(opaque[0][last] - opaque[1][last]);
                saved[1].push(cascades[0][last] - cascades[1][last]);
            }
        }
        println!(
            "cost {name}, {}: opaque {} -> {} ms, saved {}; cascades {} -> {} ms, saved {}; {BLOCKS} pairs of {BLOCK}-frame blocks",
            match (shown, moving) {
                (Shown::Moving, _) => "playing",
                (_, true) => "moving",
                _ => "rest",
            },
            spread(opaque[0].clone()),
            spread(opaque[1].clone()),
            spread(saved[0].clone()),
            spread(cascades[0].clone()),
            spread(cascades[1].clone()),
            spread(saved[1].clone()),
        );
    }
    let mut pace = manners::Pace::timed("bench", "shadow cascades", u64::from(BLOCKS * BLOCK));
    let mut cascades = [Vec::new(), Vec::new()];
    let mut saved = Vec::new();
    for pair in 0..BLOCKS * BLOCK / 2 {
        let mut ms = [0.0; 2];
        for (side, quality) in [before, after].into_iter().enumerate() {
            bench.renderer.set_shadow_quality(quality);
            let timings = pace.frame(|| bench.render(number, true, Shown::Rest));
            number += 1;
            ms[side] = pass_ms(&timings, |label| label.starts_with("shadow cascade"));
        }
        if pair >= SKIP {
            cascades[0].push(ms[0]);
            cascades[1].push(ms[1]);
            saved.push(ms[0] - ms[1]);
        }
    }
    println!(
        "cost {name}, every caster redrawn: cascades {} -> {} ms, saved {}",
        spread(cascades[0].clone()),
        spread(cascades[1].clone()),
        spread(saved),
    );
    bench.renderer.set_shadow_quality(after);
}

pub fn run(bench: &mut Bench, scene: &Scene) {
    if let Ok(list) = std::env::var("BENCH_SHADOW_COST") {
        let shipped = bench.renderer.shadows().quality();
        for entry in list.split(';').filter(|e| !e.is_empty()) {
            let (name, spec) = entry.split_once(':').unwrap_or((entry, ""));
            let (before, _) = variant(shipped, spec);
            cost(bench, name, before, shipped);
        }
    }
    let dir = &root().join("shadows");
    std::fs::create_dir_all(dir).unwrap();
    sun_only(bench);
    let base = bench.renderer.shadows().quality();
    let mut variants = vec![("shipped".to_string(), base, OpaqueFeatures::NONE)];
    if let Ok(list) = std::env::var("BENCH_SHADOW_VARIANTS") {
        for entry in list.split(';').filter(|e| !e.is_empty()) {
            let (name, spec) = entry.split_once(':').unwrap_or((entry, ""));
            let (quality, off) = variant(base, spec);
            variants.push((name.to_string(), quality, off));
        }
    }
    let hours: Vec<f32> = std::env::var("BENCH_SHADOW_HOURS")
        .unwrap_or_else(|_| HOUR.to_string())
        .split(',')
        .map(|h| h.trim().parse().expect("an hour"))
        .collect();
    for hour in hours {
        bench.hour = hour;
        let sun = bench.lighting.sun(hour).direction;
        let fit = bench.renderer.shadows().fit(&view(bench), sun);
        println!(
            "{hour}:00, sun toward {sun:?}, radius {}°; cascade texels {:.3} {:.3} {:.3} mm",
            scene.radius_deg,
            fit.cascades[0].texel * 1000.0,
            fit.cascades[1].texel * 1000.0,
            fit.cascades[2].texel * 1000.0
        );
        let started = Instant::now();
        let (reference, tracer) = reference(bench, scene, sun);
        println!(
            "reference: {} px classified by five sun rays in {:.1} s",
            reference.classes.iter().filter(|c| c.is_some()).count(),
            started.elapsed().as_secs_f64()
        );
        for (name, quality, off) in &variants {
            let label = format!("{name}-{hour}");
            bench.renderer.set_shadow_quality(*quality);
            bench.renderer.frame().set_opaque_off(*off);
            let (rgb, ids) = plate(bench);
            save_png(
                &dir.join(format!("sun-only-{label}.png")),
                PLATE.0,
                PLATE.1,
                &rgb,
            );
            let result = agreement(&reference, &rgb, &ids);
            let light = bench.renderer.shadows().fit(&view(bench), sun).cascades[0].view_proj;
            report(
                bench, light, scene, &reference, &tracer, sun, &label, &result, dir,
            );
        }
    }
    bench.hour = HOUR;
    bench.renderer.set_shadow_quality(base);
    bench.renderer.frame().set_opaque_off(OpaqueFeatures::NONE);
}
