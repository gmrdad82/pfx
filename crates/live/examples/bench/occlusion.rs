use std::path::Path;

use pfx_bake::Artifact;
use pfx_bake::reflection::read_reflection_artifact;
use pfx_gpu::PassTiming;
use pfx_live::probes::ProbeLighting;

use super::reference::{self, Region};
use super::{
    Bench, HOUR, PLATE, SETTLE_FRAMES, Shown, manners, percentile, probe_artifacts, reflections,
    root, save_png,
};

const HOURS: [f32; 3] = [12.0, 16.0, 19.0];
const THRESHOLD: f32 = 0.5;
const PAIRS: u32 = 300;
const SETTLE: u32 = 60;
const KEEP: &str = "anodised=0,chrome=0";
const NAMED: [&str; 8] = [
    "part table top",
    "part board",
    "part cavity ball",
    "part cavity back",
    "part shelf 1",
    "part back wall",
    "part chrome sphere",
    "part lacquer sphere",
];

pub struct Light {
    main: Artifact,
    locals: Vec<Artifact>,
}

impl Light {
    pub fn load() -> Light {
        let (main, locals) = probe_artifacts();
        Light { main, locals }
    }

    fn probes(&self, bench: &Bench, locals: bool, threshold: f32) -> ProbeLighting {
        let gpu = bench.renderer.gpu();
        let locals = if locals { self.locals.as_slice() } else { &[] };
        let mut probes = ProbeLighting::from_artifacts(
            &gpu.device,
            &gpu.queue,
            &self.main,
            locals,
            bench.hour,
            [-1.0; 3],
        )
        .unwrap();
        probes
            .set_reflection_occlusion(&gpu.queue, threshold)
            .unwrap();
        probes
    }

    pub fn install(&self, bench: &mut Bench, hour: f32, locals: bool, threshold: f32) {
        bench.set_hour(hour);
        let probes = self.probes(bench, locals, threshold);
        bench.renderer.set_probes(probes).unwrap();
        let cubes = reflections()
            .into_iter()
            .map(|spec| {
                let name = spec.name.unwrap_or_default();
                read_reflection_artifact(&root().join(format!("reflection-{name}"))).unwrap()
            })
            .collect();
        bench
            .renderer
            .frame()
            .set_local_reflections(Some(cubes), hour)
            .unwrap();
    }
}

pub struct Variant {
    pub name: String,
    pub locals: bool,
    pub threshold: f32,
    pub scales: Vec<(String, f32)>,
}

pub fn scales(text: &str) -> Vec<(String, f32)> {
    text.split(',')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, scale)| {
            (
                name.to_string(),
                scale.parse().expect("a scale is a number"),
            )
        })
        .collect()
}

fn threshold() -> f32 {
    std::env::var("BENCH_THRESHOLD")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(THRESHOLD)
}

fn variants() -> Vec<Variant> {
    let mut list = vec![
        Variant {
            name: "off".into(),
            locals: true,
            threshold: 0.0,
            scales: Vec::new(),
        },
        Variant {
            name: "on".into(),
            locals: true,
            threshold: threshold(),
            scales: Vec::new(),
        },
        Variant {
            name: "on, no local volumes".into(),
            locals: false,
            threshold: threshold(),
            scales: Vec::new(),
        },
    ];
    let extra = std::env::var("BENCH_SCALES").unwrap_or_else(|_| KEEP.into());
    for set in extra.split(';').filter(|set| !set.is_empty()) {
        list.push(Variant {
            name: format!("on, {set}"),
            locals: true,
            threshold: threshold(),
            scales: scales(set),
        });
    }
    list
}

fn luma(rgb: [f64; 3]) -> f64 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

fn ratios(live: &[Region], trace: &[Region]) -> Vec<String> {
    NAMED
        .iter()
        .map(|name| {
            let pick = |list: &[Region]| list.iter().find(|r| r.name == *name).map(|r| luma(r.rgb));
            match (pick(live), pick(trace)) {
                (Some(a), Some(b)) => format!("{:.3}", a / b.max(1e-9)),
                _ => "-".into(),
            }
        })
        .collect()
}

pub fn plates(bench: &mut Bench, light: &Light, out: &Path) {
    let (width, height) = PLATE;
    bench.resize(width, height);
    reference::stage_live(bench);
    std::fs::create_dir_all(out).unwrap();
    let staging = reference::staging();
    let ids = reference::ids(&staging, width, height);
    let names = reference::names(&staging);
    println!(
        "occlusion plates: {width}x{height}, the bench's own bakes, the room at rest, bare materials; live over the engine's trace at {} spp, luma per part",
        reference::SPP
    );
    println!(
        "| hour | variant | {} |",
        NAMED
            .map(|name| name.trim_start_matches("part "))
            .join(" | ")
    );
    println!("|---|---|{}", "---|".repeat(NAMED.len()));
    for hour in HOURS {
        let traced = reference::trace_rgb(&staging, hour);
        save_png(
            &out.join(format!("trace-{hour}.png")),
            width,
            height,
            &reference::tone(&traced),
        );
        let trace = reference::regions(&traced, &ids, &names, width, height);
        for (index, variant) in variants().iter().enumerate() {
            light.install(bench, hour, variant.locals, variant.threshold);
            bench.occlusion_scales = variant.scales.clone();
            let rgb = reference::live_rgb(bench, SETTLE_FRAMES);
            save_png(
                &out.join(format!("plate-{hour}-v{index}.png")),
                width,
                height,
                &reference::tone(&rgb),
            );
            let live = reference::regions(&rgb, &ids, &names, width, height);
            println!(
                "| {hour} | {} | {} |",
                variant.name,
                ratios(&live, &trace).join(" | ")
            );
            reference::report(&live, &trace, &format!("{hour} {}: ", variant.name));
        }
    }
    bench.occlusion_scales = Vec::new();
}

fn opaque(timings: &[PassTiming]) -> f64 {
    timings
        .iter()
        .find(|timing| timing.label == "opaque PBR")
        .map(|timing| timing.milliseconds)
        .expect("the opaque pass reports a timing")
}

fn spread(mut values: Vec<f64>) -> String {
    values.sort_by(f64::total_cmp);
    format!(
        "{:.3} / {:.3}",
        percentile(&values, 0.5),
        percentile(&values, 0.99)
    )
}

fn paired(
    bench: &mut Bench,
    label: &str,
    still: bool,
    first: &mut dyn FnMut(&mut Bench),
    second: &mut dyn FnMut(&mut Bench),
) {
    let mut pace = manners::Pace::timed(
        "bench",
        "occlusion",
        u64::from(SETTLE) + 2 * u64::from(PAIRS),
    );
    for number in SETTLE_FRAMES..SETTLE_FRAMES + SETTLE {
        if number % 2 == 0 {
            first(bench);
        } else {
            second(bench);
        }
        pace.frame(|| bench.render(number, still, Shown::Rest));
    }
    let start = SETTLE_FRAMES + SETTLE;
    let mut a = Vec::with_capacity(PAIRS as usize);
    let mut b = Vec::with_capacity(PAIRS as usize);
    for pair in 0..PAIRS {
        let number = start + pair * 2;
        first(bench);
        a.push(opaque(
            &pace.frame(|| bench.render(number, still, Shown::Rest)),
        ));
        second(bench);
        b.push(opaque(
            &pace.frame(|| bench.render(number + 1, still, Shown::Rest)),
        ));
    }
    let saved: Vec<f64> = a.iter().zip(&b).map(|(x, y)| x - y).collect();
    println!(
        "| {label} | {} | {} | {} | {} | {PAIRS} |",
        if still { "rest" } else { "moving" },
        spread(a),
        spread(b),
        spread(saved)
    );
}

pub fn timing(bench: &mut Bench, light: &Light) {
    let (width, height) = bench.renderer.size();
    light.install(bench, HOUR, true, THRESHOLD);
    println!(
        "occlusion timing: {width}x{height}, {HOUR}:00, {PAIRS} frame pairs after {SETTLE} to settle, each pair one frame with the row's term on and one with it off; opaque PBR ms, p50 / p99"
    );
    println!("| term | camera | on | off | on − off | pairs |");
    println!("|---|---|---:|---:|---:|---:|");
    for still in [true, false] {
        paired(
            bench,
            "reflection occlusion 0.5",
            still,
            &mut |bench| bench.renderer.set_reflection_occlusion(THRESHOLD).unwrap(),
            &mut |bench| bench.renderer.set_reflection_occlusion(0.0).unwrap(),
        );
    }
    for still in [true, false] {
        let mut on = |bench: &mut Bench| {
            let probes = light.probes(bench, true, THRESHOLD);
            bench.renderer.set_probes(probes).unwrap();
        };
        let mut off = |bench: &mut Bench| {
            let probes = light.probes(bench, false, THRESHOLD);
            bench.renderer.set_probes(probes).unwrap();
        };
        paired(
            bench,
            "the two local probe volumes",
            still,
            &mut on,
            &mut off,
        );
    }
    light.install(bench, HOUR, true, THRESHOLD);
    for still in [true, false] {
        paired(
            bench,
            "chrome and anodised keep their reflection",
            still,
            &mut |bench| bench.occlusion_scales = scales(KEEP),
            &mut |bench| bench.occlusion_scales = Vec::new(),
        );
    }
    bench.occlusion_scales = Vec::new();
}
