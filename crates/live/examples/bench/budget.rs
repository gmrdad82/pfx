use pfx_live::frame::OpaqueFeatures;
use pfx_live::probes::PROBE_CORNER_FLOOR;
use pfx_live::shadow::{BLOCKER_TAPS, FILTER_TAPS, SOFT_TAPS};
use pfx_materials::NoiseKind;
use pfx_trace::detail::{Detail, Lens as TraceLens};
use pfx_trace::{Camera as TraceCamera, Scene as TraceScene, Trace};

use super::*;

const PAIRS: u32 = 360;
const SETTLE: u32 = 60;

pub struct Spread {
    pub median: f64,
    pub low: f64,
    pub high: f64,
}

impl Spread {
    fn of(mut values: Vec<f64>) -> Self {
        values.sort_by(f64::total_cmp);
        Self {
            median: percentile(&values, 0.5),
            low: percentile(&values, 0.1),
            high: percentile(&values, 0.9),
        }
    }
}

impl std::fmt::Display for Spread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.3} [{:.3}–{:.3}]", self.median, self.low, self.high)
    }
}

#[derive(Clone, Copy)]
pub struct Setting {
    pub on: OpaqueFeatures,
    pub off: OpaqueFeatures,
    pub quality: Quality,
    pub probe_floor: f32,
}

impl Setting {
    fn apply(&self, bench: &mut Bench) {
        let frame = bench.renderer.frame();
        frame.set_opaque_on(self.on);
        frame.set_opaque_off(self.off);
        if frame.probe_corner_floor() != self.probe_floor {
            frame.set_probe_corner_floor(self.probe_floor).unwrap();
        }
        if bench.renderer.shadows().quality() != self.quality {
            bench.renderer.set_shadow_quality(self.quality);
        }
    }
}

pub struct Paired {
    pub first: Spread,
    pub second: Spread,
    pub saved: Spread,
    pub kept: usize,
    pub tops: [f64; 2],
}

fn fine_noise() -> OpaqueFeatures {
    OpaqueFeatures::named("fine_noise").expect("fine_noise is an opaque feature")
}

fn opaque_ms(timings: &[PassTiming]) -> Option<f64> {
    timings
        .iter()
        .find(|timing| timing.label == "opaque PBR")
        .map(|timing| timing.milliseconds)
}

pub fn paired(
    bench: &mut Bench,
    mode: &str,
    still: bool,
    first: Setting,
    second: Setting,
) -> Paired {
    let mut pace = manners::Pace::timed("bench", mode, u64::from(SETTLE) + 2 * u64::from(PAIRS));
    for number in SETTLE_FRAMES..SETTLE_FRAMES + SETTLE {
        let setting = if number % 2 == 0 { first } else { second };
        setting.apply(bench);
        pace.frame(|| bench.render(number, still, Shown::Rest));
    }
    let start = SETTLE_FRAMES + SETTLE;
    let mut a = Vec::with_capacity(PAIRS as usize);
    let mut b = Vec::with_capacity(PAIRS as usize);
    let mut saved = Vec::with_capacity(PAIRS as usize);
    for pair in 0..PAIRS {
        let number = start + pair * 2;
        first.apply(bench);
        let timings = pace.frame(|| bench.render(number, still, Shown::Rest));
        let one = opaque_ms(&timings).expect("the opaque pass reports a timing");
        second.apply(bench);
        let timings = pace.frame(|| bench.render(number + 1, still, Shown::Rest));
        let two = opaque_ms(&timings).expect("the opaque pass reports a timing");
        a.push(one);
        b.push(two);
        saved.push(one - two);
    }
    let mut sorted = a.clone();
    sorted.sort_by(f64::total_cmp);
    let fast = percentile(&sorted, 0.1) * 1.15;
    let kept: Vec<usize> = (0..a.len()).filter(|&i| a[i] <= fast).collect();
    let pick = |values: &[f64]| kept.iter().map(|&i| values[i]).collect::<Vec<f64>>();
    Paired {
        first: Spread::of(pick(&a)),
        second: Spread::of(pick(&b)),
        saved: Spread::of(pick(&saved)),
        kept: kept.len(),
        tops: [&a, &b].map(|values| {
            let mut sorted = pick(values);
            sorted.sort_by(f64::total_cmp);
            percentile(&sorted, 0.99)
        }),
    }
}

pub fn still_pixels(bench: &mut Bench, setting: Setting) -> Vec<u8> {
    setting.apply(bench);
    let mut pace = manners::Pace::new();
    for number in 0..SETTLE_FRAMES {
        pace.frame(|| bench.render(number, true, Shown::Rest));
    }
    bench.pixels()
}

pub fn difference(reference: &[u8], other: &[u8]) -> (f64, u8) {
    let mut total = 0u64;
    let mut worst = 0u8;
    let mut count = 0u64;
    for (a, b) in reference.chunks_exact(4).zip(other.chunks_exact(4)) {
        for channel in 0..3 {
            let delta = a[channel].abs_diff(b[channel]);
            total += u64::from(delta);
            worst = worst.max(delta);
            count += 1;
        }
    }
    (total as f64 / count as f64, worst)
}

fn header(first: &str) {
    println!(
        "| {first} | rest ms, on | rest ms, off | rest saved | pairs | moving ms, on | moving ms, off | moving saved | pairs |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|");
}

fn row(name: &str, rest: &Paired, moving: &Paired) {
    println!(
        "| {name} | {} | {} | {} | {} | {} | {} | {} | {} |",
        rest.first,
        rest.second,
        rest.saved,
        rest.kept,
        moving.first,
        moving.second,
        moving.saved,
        moving.kept
    );
}

fn feature_names() -> Vec<&'static str> {
    match std::env::var("BENCH_BREAKDOWN_ONLY") {
        Ok(list) => list
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(|name| {
                OpaqueFeatures::NAMES
                    .into_iter()
                    .find(|known| *known == name)
                    .unwrap_or_else(|| panic!("unknown opaque feature {name}"))
            })
            .collect(),
        Err(_) => OpaqueFeatures::NAMES.to_vec(),
    }
}

pub fn today(quality: Quality) -> Quality {
    Quality {
        blocker_taps: SOFT_TAPS,
        filter_taps: SOFT_TAPS,
        blocker_gathers: 0,
        nearest_blocker: false,
        umbra_skip: false,
        lit_skip: true,
        blend_dither: false,
        ..quality
    }
}

pub fn breakdown(bench: &mut Bench) {
    let (width, height) = bench.renderer.size();
    let shipped = bench.renderer.shadows().quality();
    let quality = if std::env::var_os("BENCH_BREAKDOWN_CUT").is_some() {
        shipped
    } else {
        today(shipped)
    };
    let general = std::env::var_os("BENCH_BREAKDOWN_CUT").is_none();
    let base = Setting {
        on: if general {
            OpaqueFeatures::ALL
        } else {
            OpaqueFeatures::NONE
        },
        off: OpaqueFeatures::NONE,
        quality,
        probe_floor: if general { 0.0 } else { PROBE_CORNER_FLOOR },
    };
    println!(
        "opaque breakdown: {width}x{height}, {PAIRS} frame pairs per row after {SETTLE} frames to settle, each pair one frame with every feature and one with the row's feature switched off; {}; the room's scene-wide features: {:?}",
        if general {
            "today's shader and soft sun"
        } else {
            "the shipped cuts"
        },
        bench.renderer.frame().scene_features().names()
    );
    header("switched off");
    let mut everything = OpaqueFeatures::NONE;
    for name in feature_names() {
        let off = OpaqueFeatures::named(name).unwrap();
        everything = everything.union(off);
        let variant = Setting { off, ..base };
        let rest = paired(bench, "breakdown", true, base, variant);
        let moving = paired(bench, "breakdown", false, base, variant);
        row(name, &rest, &moving);
    }
    let variant = Setting {
        off: everything,
        ..base
    };
    let rest = paired(bench, "breakdown", true, base, variant);
    let moving = paired(bench, "breakdown", false, base, variant);
    row("all of the above", &rest, &moving);
    let pixels = still_pixels(bench, base);
    save_png(&root().join("opaque-breakdown.png"), width, height, &pixels);
}

fn number(name: &str, fallback: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(fallback)
}

fn cut_row(name: &str, rest: &Paired, moving: &Paired, mean: f64, worst: u8) {
    println!(
        "| {name} | {} | {} | {} | {} | {} | {} | {} | {} | {mean:.3} | {worst} |",
        rest.first,
        rest.second,
        rest.saved,
        rest.kept,
        moving.first,
        moving.second,
        moving.saved,
        moving.kept
    );
}

pub fn cuts(bench: &mut Bench) {
    let (width, height) = bench.renderer.size();
    let shipped = bench.renderer.shadows().quality();
    let old = today(shipped);
    let reference = Setting {
        on: OpaqueFeatures::ALL,
        off: OpaqueFeatures::NONE,
        quality: old,
        probe_floor: 0.0,
    };
    let mut setting = reference;
    let mut steps: Vec<(String, Setting)> = Vec::new();
    setting.on = fine_noise();
    steps.push((
        "pipelines specialized to the features each part uses".into(),
        setting,
    ));
    setting.quality.blend_dither = true;
    steps.push(("one dithered cascade in the blend band".into(), setting));
    setting.quality.nearest_blocker = true;
    steps.push((
        "nearest blocker sets the penumbra (request 8)".into(),
        setting,
    ));
    setting.quality.umbra_skip = true;
    steps.push(("umbra skips the filter".into(), setting));
    setting.quality.filter_taps = number("BENCH_CUT_FILTER_TAPS", FILTER_TAPS);
    steps.push((
        format!("{} filter taps", setting.quality.filter_taps),
        setting,
    ));
    setting.quality.blocker_taps = number("BENCH_CUT_BLOCKER_TAPS", BLOCKER_TAPS);
    steps.push((
        format!("{} blocker taps", setting.quality.blocker_taps),
        setting,
    ));
    setting.probe_floor = PROBE_CORNER_FLOOR;
    steps.push((
        format!("probe corners under {PROBE_CORNER_FLOOR} fade out"),
        setting,
    ));
    setting.on = OpaqueFeatures::NONE;
    steps.push(("octaves under the pixel footprint fade".into(), setting));
    steps.push((
        "shipped defaults".into(),
        Setting {
            on: OpaqueFeatures::NONE,
            off: OpaqueFeatures::NONE,
            quality: shipped,
            probe_floor: PROBE_CORNER_FLOOR,
        },
    ));
    steps.push((
        "shipped, with 4 gathers for the blocker search".into(),
        Setting {
            on: OpaqueFeatures::NONE,
            off: OpaqueFeatures::NONE,
            quality: Quality {
                blocker_gathers: 4,
                ..shipped
            },
            probe_floor: PROBE_CORNER_FLOOR,
        },
    ));
    println!(
        "opaque cuts: {width}x{height}, {PAIRS} frame pairs per row after {SETTLE} frames to settle, each pair one frame as today and one with the cuts up to the row, pairs kept where today's frame ran at the fast clock; images compared with today's frame at rest"
    );
    println!(
        "| cut (cumulative) | rest ms, today | rest ms, cut | rest saved | pairs | moving ms, today | moving ms, cut | moving saved | pairs | mean /255 | worst /255 |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    let today_pixels = still_pixels(bench, reference);
    save_png(
        &root().join("opaque-today.png"),
        width,
        height,
        &today_pixels,
    );
    for (index, (name, cut)) in steps.into_iter().enumerate() {
        let rest = paired(bench, "cuts", true, reference, cut);
        let moving = paired(bench, "cuts", false, reference, cut);
        let pixels = still_pixels(bench, cut);
        let (mean, worst) = difference(&today_pixels, &pixels);
        cut_row(&name, &rest, &moving, mean, worst);
        let mut sets: Vec<Vec<&str>> = bench
            .renderer
            .frame()
            .opaque_drawn()
            .iter()
            .map(|used| used.names())
            .collect();
        sets.sort();
        sets.dedup();
        println!("  drawn with {} feature sets: {sets:?}", sets.len());
        save_png(
            &root().join(format!("opaque-cut-{index}.png")),
            width,
            height,
            &pixels,
        );
    }
}

const CROP: u32 = 480;
const TRACE_SAMPLES: u32 = 256;

fn crop(pixels: &[u8], width: u32, x0: u32, y0: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((CROP * CROP * 4) as usize);
    for y in y0..y0 + CROP {
        let start = ((y * width + x0) * 4) as usize;
        out.extend_from_slice(&pixels[start..start + (CROP * 4) as usize]);
    }
    out
}

fn amplified(before: &[u8], after: &[u8], gain: u8) -> Vec<u8> {
    before
        .chunks_exact(4)
        .zip(after.chunks_exact(4))
        .flat_map(|(a, b)| {
            let delta = (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0);
            let v = delta.saturating_mul(gain);
            [v, v, v, 255]
        })
        .collect()
}

fn triptych(path: &Path, width: u32, before: &[u8], after: &[u8], x0: u32, y0: u32, gain: u8) {
    let parts = [
        crop(before, width, x0, y0),
        crop(after, width, x0, y0),
        amplified(
            &crop(before, width, x0, y0),
            &crop(after, width, x0, y0),
            gain,
        ),
    ];
    let mut out = Vec::with_capacity((CROP * CROP * 12) as usize);
    for y in 0..CROP as usize {
        for part in &parts {
            let row = CROP as usize * 4;
            out.extend_from_slice(&part[y * row..(y + 1) * row]);
        }
    }
    save_png(path, CROP * 3, CROP, &out);
}

fn object_name(bench: &Bench, id: u32) -> &str {
    id.checked_sub(1)
        .and_then(|i| bench.objects.get(i as usize))
        .map_or("(nothing)", String::as_str)
}

fn procedural(bench: &Bench, id: u32) -> Option<Vec<String>> {
    let instance = bench.instances.iter().find(|instance| instance.id == id)?;
    let material = bench.bare.get(instance.material as usize)?;
    let kinds: Vec<String> = material
        .layers
        .iter()
        .filter(|layer| layer.kind != NoiseKind::None && layer.amplitude != 0.0)
        .map(|layer| format!("{:?}", layer.kind))
        .collect();
    (!kinds.is_empty()).then_some(kinds)
}

fn shipped(bench: &Bench) -> Setting {
    Setting {
        on: OpaqueFeatures::NONE,
        off: OpaqueFeatures::NONE,
        quality: bench.renderer.shadows().quality(),
        probe_floor: PROBE_CORNER_FLOOR,
    }
}

struct Comparison<'a> {
    prefix: &'a str,
    row: &'a str,
    gain: u8,
    parts: &'a [&'a str],
    by_kind: bool,
}

pub fn footprint(bench: &mut Bench, triangles: &[Triangle], traced: bool) {
    let (width, height) = bench.renderer.size();
    let after = shipped(bench);
    let before = Setting {
        on: fine_noise(),
        ..after
    };
    println!(
        "noise footprint: {width}x{height}, {PAIRS} frame pairs after {SETTLE} frames to settle, each pair one frame with every noise octave evaluated (before) and one with octaves under the pixel footprint faded (after), the shipped cuts on both; images compared at rest"
    );
    compare(
        bench,
        triangles,
        traced,
        before,
        after,
        Comparison {
            prefix: "footprint",
            row: "octaves under the pixel footprint fade",
            gain: 16,
            parts: &[],
            by_kind: false,
        },
    );
}

fn named(name: &str) -> OpaqueFeatures {
    OpaqueFeatures::named(name).unwrap_or_else(|| panic!("{name} is an opaque feature"))
}

pub fn lacquer(bench: &mut Bench, triangles: &[Triangle], traced: bool) {
    let (width, height) = bench.renderer.size();
    let after = shipped(bench);
    let before = Setting {
        on: named("procedural_lacquer"),
        ..after
    };
    let drawn = |bench: &Bench| {
        bench
            .renderer
            .frame()
            .opaque_drawn()
            .iter()
            .filter(|used| used.lacquer)
            .count()
    };
    still_pixels(bench, after);
    println!(
        "lacquer tile: {width}x{height}, {PAIRS} frame pairs after {SETTLE} frames to settle, each pair one frame with the lacquer bump's hash noise (before) and one sampling the tiling noise texture (after), the shipped cuts on both; {} opaque batches carry lacquer",
        drawn(bench)
    );
    println!("the lacquer's share: each pair one frame with the bump and one with it compiled out");
    println!(
        "| bump | rest ms, with | rest ms, without | rest share | pairs | moving ms, with | moving ms, without | moving share | pairs |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|");
    let without = |setting: Setting| Setting {
        off: named("lacquer"),
        ..setting
    };
    for (name, setting) in [("hash noise (before)", before), ("tile (after)", after)] {
        let rest = paired(bench, "lacquer", true, setting, without(setting));
        let moving = paired(bench, "lacquer", false, setting, without(setting));
        row(name, &rest, &moving);
    }
    compare(
        bench,
        triangles,
        traced,
        before,
        after,
        Comparison {
            prefix: "lacquer",
            row: "the lacquer bump samples a tiling noise texture",
            gain: 8,
            parts: &["lacquer sphere", "shelf 1", "cavity top"],
            by_kind: false,
        },
    );
}

const KINDS: [(&str, &str); 6] = [
    ("scratch", "Scratch"),
    ("grime", "Grime"),
    ("wood", "PlankWood"),
    ("wall", "WallMottle"),
    ("fibre", "Fibre"),
    ("crinkle", "Crinkle"),
];

fn timing_row(name: &str, rest: &Paired, moving: &Paired) {
    println!(
        "| {name} | {:.3} | {:.3} | {:.3} | {:.3} | {} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {} | {} |",
        rest.first.median,
        rest.tops[0],
        rest.second.median,
        rest.tops[1],
        rest.saved,
        rest.kept,
        moving.first.median,
        moving.tops[0],
        moving.second.median,
        moving.tops[1],
        moving.saved,
        moving.kept
    );
}

pub fn tiles(bench: &mut Bench, triangles: &[Triangle], traced: bool) {
    let (width, height) = bench.renderer.size();
    let after = shipped(bench);
    let before = Setting {
        on: named("procedural_detail"),
        ..after
    };
    still_pixels(bench, after);
    let drawn: Vec<String> = KINDS
        .iter()
        .map(|(name, _)| {
            let count = bench
                .renderer
                .frame()
                .opaque_drawn()
                .iter()
                .filter(|used| used.names().contains(name))
                .count();
            format!("{name} {count}")
        })
        .collect();
    println!(
        "detail tiles: {width}x{height}, {PAIRS} frame pairs after {SETTLE} frames to settle, the shipped cuts and the lacquer tile on both sides; opaque batches carrying each kind: {}",
        drawn.join(", ")
    );
    let every = KINDS.iter().fold(OpaqueFeatures::NONE, |all, (name, _)| {
        all.union(named(name))
    });
    let mode = std::env::var("BENCH_TILES").unwrap_or_default();
    let settings: Vec<(&str, Setting)> = match mode.as_str() {
        "share" => vec![("hash noise", before)],
        _ => vec![("hash noise", before), ("tiles", after)],
    };
    println!(
        "each kind's share: each pair one frame with every kind and one with the row's kind compiled out"
    );
    header("compiled out");
    for (label, setting) in &settings {
        for (name, _) in KINDS {
            let without = Setting {
                off: named(name),
                ..*setting
            };
            let rest = paired(bench, "tiles", true, *setting, without);
            let moving = paired(bench, "tiles", false, *setting, without);
            row(&format!("{name}, {label}"), &rest, &moving);
        }
        let without = Setting {
            off: every,
            ..*setting
        };
        let rest = paired(bench, "tiles", true, *setting, without);
        let moving = paired(bench, "tiles", false, *setting, without);
        row(&format!("all six, {label}"), &rest, &moving);
    }
    if mode == "share" || mode == "shares" {
        return;
    }
    let general = Setting {
        on: before.on.union(every),
        ..before
    };
    println!(
        "specialization, hash noise on both sides: each pair one frame where every detail part compiles all six kinds and one where each compiles only its own"
    );
    header("");
    let rest = paired(bench, "tiles", true, general, before);
    let moving = paired(bench, "tiles", false, general, before);
    row("each kind only where used", &rest, &moving);
    println!(
        "the opaque pass, hash noise (before) against tiles (after), median and 99th percentile"
    );
    println!(
        "| | rest p50, before | rest p99, before | rest p50, after | rest p99, after | rest saved | pairs | moving p50, before | moving p99, before | moving p50, after | moving p99, after | moving saved | pairs |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    let rest = paired(bench, "tiles", true, before, after);
    let moving = paired(bench, "tiles", false, before, after);
    timing_row("opaque ms", &rest, &moving);
    compare(
        bench,
        triangles,
        traced,
        before,
        after,
        Comparison {
            prefix: "tiles",
            row: "scratch, the wall's fine octaves, fibre and crinkle sample tiling noise tiles",
            gain: 8,
            parts: &[],
            by_kind: true,
        },
    );
}

fn fullest_box(ids: &[u32], id: u32, width: u32, height: u32) -> Option<(u32, u32)> {
    let cell = 40;
    let (columns, rows) = (width / cell, height / cell);
    let mut counts = vec![0u32; (columns * rows) as usize];
    for (index, &at) in ids.iter().enumerate() {
        if at == id {
            let (x, y) = (index as u32 % width / cell, index as u32 / width / cell);
            counts[(y.min(rows - 1) * columns + x.min(columns - 1)) as usize] += 1;
        }
    }
    let span = CROP / cell;
    let mut best = (0u32, 0u32, 0u32);
    for y in 0..=rows - span {
        for x in 0..=columns - span {
            let total: u32 = (y..y + span)
                .flat_map(|row| (x..x + span).map(move |column| (row, column)))
                .map(|(row, column)| counts[(row * columns + column) as usize])
                .sum();
            if total > best.0 {
                best = (total, x * cell, y * cell);
            }
        }
    }
    (best.0 > 0).then_some((best.1, best.2))
}

fn part_box(bench: &Bench, ids: &[u32], name: &str) -> Option<(u32, u32)> {
    let (width, height) = bench.renderer.size();
    let id = bench.objects.iter().position(|object| object == name)? as u32 + 1;
    let mut sum = (0u64, 0u64, 0u64);
    for (index, &at) in ids.iter().enumerate() {
        if at == id {
            sum.0 += (index as u32 % width) as u64;
            sum.1 += (index as u32 / width) as u64;
            sum.2 += 1;
        }
    }
    if sum.2 == 0 {
        return None;
    }
    let (x, y) = ((sum.0 / sum.2) as u32, (sum.1 / sum.2) as u32);
    Some((
        x.saturating_sub(CROP / 2).min(width - CROP),
        y.saturating_sub(CROP / 2).min(height - CROP),
    ))
}

fn compare(
    bench: &mut Bench,
    triangles: &[Triangle],
    traced: bool,
    before: Setting,
    after: Setting,
    comparison: Comparison<'_>,
) {
    let (width, height) = bench.renderer.size();
    let Comparison {
        prefix,
        row,
        gain,
        parts: named_parts,
        by_kind,
    } = comparison;
    println!(
        "| | rest ms, before | rest ms, after | rest saved | pairs | moving ms, before | moving ms, after | moving saved | pairs | mean /255 | worst /255 |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    let rest = paired(bench, prefix, true, before, after);
    let moving = paired(bench, prefix, false, before, after);
    let old = still_pixels(bench, before);
    let new = still_pixels(bench, after);
    let (mean, worst) = difference(&old, &new);
    cut_row(row, &rest, &moving, mean, worst);
    let root = root();
    save_png(
        &root.join(format!("{prefix}-before.png")),
        width,
        height,
        &old,
    );
    save_png(
        &root.join(format!("{prefix}-after.png")),
        width,
        height,
        &new,
    );
    save_png(
        &root.join(format!("{prefix}-diff-x{gain}.png")),
        width,
        height,
        &amplified(&old, &new, gain),
    );
    let ids = bench.ids();
    let mut parts: BTreeMap<u32, (u64, u64, u8, i64)> = BTreeMap::new();
    for (index, (a, b)) in old.chunks_exact(4).zip(new.chunks_exact(4)).enumerate() {
        let entry = parts.entry(ids[index]).or_default();
        entry.0 += 1;
        for channel in 0..3 {
            let delta = a[channel].abs_diff(b[channel]);
            entry.1 += u64::from(delta);
            entry.2 = entry.2.max(delta);
            entry.3 += i64::from(b[channel]) - i64::from(a[channel]);
        }
    }
    let mut changed: Vec<(u32, u64, f64, u8, f64)> = parts
        .into_iter()
        .filter(|(_, (count, _, _, _))| *count > 0)
        .map(|(id, (count, total, worst, signed))| {
            let samples = (count * 3) as f64;
            (
                id,
                count,
                total as f64 / samples,
                worst,
                signed as f64 / samples,
            )
        })
        .collect();
    changed.sort_by(|a, b| b.2.total_cmp(&a.2));
    println!("parts changed most (mean, signed mean and worst /255 over the part's pixels):");
    for (id, count, mean, worst, signed) in changed.iter().take(10) {
        println!(
            "  {:28} {count:9} px  mean {mean:.3}  signed {signed:+.3}  worst {worst}  layers {:?}",
            object_name(bench, *id),
            procedural(bench, *id).unwrap_or_default()
        );
    }
    let unlacquered: Vec<&(u32, u64, f64, u8, f64)> = changed
        .iter()
        .filter(|(id, ..)| {
            !procedural(bench, *id)
                .unwrap_or_default()
                .iter()
                .any(|kind| kind == "CoatWobble")
        })
        .collect();
    let moved: Vec<&&(u32, u64, f64, u8, f64)> = unlacquered
        .iter()
        .filter(|(_, _, mean, ..)| *mean > 0.0)
        .collect();
    println!(
        "parts without a lacquer layer: {} on screen, {} with any changed pixel, the largest mean {:.3}/255 ({})",
        unlacquered.len(),
        moved.len(),
        moved.first().map_or(0.0, |part| part.2),
        moved
            .first()
            .map_or("none", |part| object_name(bench, part.0))
    );
    let block = 240;
    let mut best = (0.0f64, 0, 0);
    for by in 0..(height - CROP) / block + 1 {
        for bx in 0..(width - CROP) / block + 1 {
            let (x0, y0) = (bx * block, by * block);
            let mut total = 0u64;
            for y in (y0..y0 + CROP).step_by(2) {
                for x in (x0..x0 + CROP).step_by(2) {
                    let i = ((y * width + x) * 4) as usize;
                    total += (0..3)
                        .map(|c| u64::from(old[i + c].abs_diff(new[i + c])))
                        .sum::<u64>();
                }
            }
            if total as f64 > best.0 {
                best = (total as f64, x0, y0);
            }
        }
    }
    let mut crops: Vec<(String, u32, u32)> = named_parts
        .iter()
        .filter_map(|name| part_box(bench, &ids, name).map(|(x, y)| (name.to_string(), x, y)))
        .collect();
    if by_kind {
        println!(
            "parts by kind (pixel-weighted mean, signed mean and worst /255 over every part carrying the kind, and its largest part):"
        );
        for (_, kind) in KINDS {
            let carriers: Vec<&(u32, u64, f64, u8, f64)> = changed
                .iter()
                .filter(|(id, ..)| {
                    procedural(bench, *id)
                        .unwrap_or_default()
                        .iter()
                        .any(|name| name == kind)
                })
                .collect();
            let pixels: u64 = carriers.iter().map(|part| part.1).sum();
            if pixels == 0 {
                println!("  {kind:14} not on screen");
                continue;
            }
            let weighted = |pick: fn(&(u32, u64, f64, u8, f64)) -> f64| {
                carriers
                    .iter()
                    .map(|part| pick(part) * part.1 as f64)
                    .sum::<f64>()
                    / pixels as f64
            };
            let largest = carriers
                .iter()
                .max_by_key(|part| part.1)
                .expect("a kind on screen has a part");
            let worst = carriers.iter().map(|part| part.3).max().unwrap_or(0);
            println!(
                "  {kind:14} {:3} parts {pixels:9} px  mean {:.3}  signed {:+.3}  worst {worst}  largest {} ({} px, mean {:.3}, signed {:+.3})",
                carriers.len(),
                weighted(|part| part.2),
                weighted(|part| part.4),
                object_name(bench, largest.0),
                largest.1,
                largest.2,
                largest.4
            );
            let name = object_name(bench, largest.0).to_string();
            if let Some((x, y)) = fullest_box(&ids, largest.0, width, height) {
                crops.push((format!("{kind}-{name}"), x, y));
            }
        }
    }
    let (worst_at, _) = old
        .chunks_exact(4)
        .zip(new.chunks_exact(4))
        .map(|(a, b)| (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0))
        .enumerate()
        .fold(
            (0, 0),
            |best, (index, delta)| {
                if delta > best.1 { (index, delta) } else { best }
            },
        );
    let worst_x = (worst_at as u32 % width)
        .saturating_sub(CROP / 2)
        .min(width - CROP);
    let worst_y = (worst_at as u32 / width)
        .saturating_sub(CROP / 2)
        .min(height - CROP);
    crops.extend(
        [
            ("worst", worst_x, worst_y),
            ("most-changed", best.1, best.2),
            ("centre", (width - CROP) / 2, (height - CROP) / 2),
            ("near", (width - CROP) / 2, height - CROP),
            ("top-left", 0, 0),
        ]
        .map(|(name, x, y)| (name.to_string(), x, y)),
    );
    for (name, x0, y0) in crops {
        let path = root.join(format!("{prefix}-crop-{name}.png"));
        triptych(&path, width, &old, &new, x0, y0, gain);
        println!(
            "crop {name} at ({x0}, {y0}), before | after | difference x{gain}: {}",
            path.display()
        );
    }
    if traced {
        trace_check(bench, triangles, before, after);
    }
    after.apply(bench);
}

fn plate_luma(c: [f32; 4]) -> f64 {
    0.2126 * f64::from(c[0]) + 0.7152 * f64::from(c[1]) + 0.0722 * f64::from(c[2])
}

fn trace_check(bench: &mut Bench, triangles: &[Triangle], before: Setting, after: Setting) {
    let size = bench.renderer.size();
    let (width, height) = PLATE;
    bench.resize(width, height);
    let live = |bench: &mut Bench, setting: Setting| -> Vec<[f32; 4]> {
        setting.apply(bench);
        let mut pace = manners::Pace::new();
        for number in 0..SETTLE_FRAMES {
            pace.frame(|| bench.render(number, true, Shown::Bare));
        }
        bench.renderer.readback_geometry().unwrap().colour
    };
    let old = live(bench, before);
    let new = live(bench, after);
    let ids = bench.ids();
    let lens = &bench.lens;
    let forward = unit(sub(lens.target, lens.position));
    let right = unit(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    let tan_y = (lens.fov_y.to_radians() * 0.5).tan();
    let tan_x = tan_y * width as f32 / height as f32;
    let anchor = bench.lighting.anchor(HOUR);
    let scene = TraceScene {
        triangles: triangles.to_vec(),
        shapes: Vec::new(),
        materials: bench.bare.clone(),
        sky: anchor.sky.clone(),
        camera: TraceCamera {
            origin: lens.position,
            forward,
            right: right.map(|v| v * tan_x),
            up: up.map(|v| v * tan_y),
        },
        sun: anchor.sun,
    };
    let detail = Detail {
        lens: TraceLens {
            shift: lens.shift,
            ..TraceLens::default()
        },
        ..Detail::default()
    };
    let gpu = bench.renderer.gpu();
    let mut trace = Trace::new_detailed(gpu, &scene, &detail, width, height).unwrap();
    for _ in 0..TRACE_SAMPLES / 4 {
        trace.sample(gpu, 4, SEED).unwrap();
    }
    let output = trace.readback(gpu).unwrap();
    let floats = |bytes: &[u8]| -> Vec<[f32; 4]> {
        bytes
            .chunks_exact(16)
            .map(|p| {
                std::array::from_fn(|i| f32::from_le_bytes(p[i * 4..i * 4 + 4].try_into().unwrap()))
            })
            .collect()
    };
    let traced = floats(&output.color);
    let covered: Vec<bool> = floats(&output.normal)
        .into_iter()
        .map(|n| n[3] > 0.99)
        .collect();
    let mut parts: BTreeMap<u32, [f64; 4]> = BTreeMap::new();
    for (index, colour) in traced.iter().enumerate() {
        if !covered[index] {
            continue;
        }
        let entry = parts.entry(ids[index]).or_default();
        entry[0] += 1.0;
        entry[1] += plate_luma(*colour);
        entry[2] += plate_luma(old[index]);
        entry[3] += plate_luma(new[index]);
    }
    println!(
        "traced against live at the bench's camera, {width}x{height}, {TRACE_SAMPLES} samples, bare materials, sun and sky at {HOUR}:00: mean linear luma per procedural part"
    );
    println!(
        "| part | layers | px | traced | live before | live after | before − traced | after − traced |"
    );
    println!("|---|---|---:|---:|---:|---:|---:|---:|");
    let mut all = [0.0f64; 4];
    let mut by_kind: BTreeMap<String, [f64; 5]> = BTreeMap::new();
    for (id, sums) in &parts {
        let Some(kinds) = procedural(bench, *id) else {
            continue;
        };
        if sums[0] < 500.0 {
            continue;
        }
        for (total, value) in all.iter_mut().zip(sums) {
            *total += value;
        }
        let m = [sums[1], sums[2], sums[3]].map(|v| v / sums[0]);
        for kind in &kinds {
            let entry = by_kind.entry(kind.clone()).or_default();
            entry[0] += sums[0];
            entry[1] += (m[1] - m[0]) * sums[0];
            entry[2] += (m[2] - m[0]) * sums[0];
            entry[3] += (m[1] - m[0]).abs() * sums[0];
            entry[4] += (m[2] - m[0]).abs() * sums[0];
        }
        println!(
            "| {} | {} | {} | {:.4} | {:.4} | {:.4} | {:+.4} | {:+.4} |",
            object_name(bench, *id),
            kinds.join(", "),
            sums[0],
            m[0],
            m[1],
            m[2],
            m[1] - m[0],
            m[2] - m[0]
        );
    }
    let m = [all[1], all[2], all[3]].map(|v| v / all[0].max(1.0));
    println!(
        "| every procedural part | | {} | {:.4} | {:.4} | {:.4} | {:+.4} | {:+.4} |",
        all[0],
        m[0],
        m[1],
        m[2],
        m[1] - m[0],
        m[2] - m[0]
    );
    println!(
        "the gap by kind, pixel-weighted over the parts carrying it: the signed gap and the mean of each part's absolute gap"
    );
    println!("| kind | px | before − traced | after − traced | abs, before | abs, after |");
    println!("|---|---:|---:|---:|---:|---:|");
    for (kind, sums) in &by_kind {
        let m = [sums[1], sums[2], sums[3], sums[4]].map(|v| v / sums[0]);
        println!(
            "| {kind} | {} | {:+.4} | {:+.4} | {:.4} | {:.4} |",
            sums[0], m[0], m[1], m[2], m[3]
        );
    }
    bench.resize(size.0, size.1);
}
