use pfx_live::effects::VolumeCuts;

use super::budget::{Spread, difference};
use super::*;

const PAIRS: u32 = 360;
const SETTLE: u32 = 60;
const CROP: u32 = 480;
const GAIN: u8 = 8;
const VISIBLE: u8 = 8;

struct Pairs {
    first: Spread,
    second: Spread,
    saved: Spread,
    kept: usize,
}

fn volume_ms(timings: &[PassTiming]) -> f64 {
    timings
        .iter()
        .find(|timing| timing.label == "volume")
        .map_or(0.0, |timing| timing.milliseconds)
}

fn paired(
    bench: &mut Bench,
    mode: &str,
    still: bool,
    first: VolumeCuts,
    second: VolumeCuts,
) -> Pairs {
    if std::env::var_os("BENCH_VOLUME_IMAGES_ONLY").is_some() {
        let none = || Spread {
            median: 0.0,
            low: 0.0,
            high: 0.0,
        };
        return Pairs {
            first: none(),
            second: none(),
            saved: none(),
            kept: 0,
        };
    }
    let mut pace = manners::Pace::timed("bench", mode, u64::from(SETTLE) + 2 * u64::from(PAIRS));
    for number in SETTLE_FRAMES..SETTLE_FRAMES + SETTLE {
        bench
            .renderer
            .set_volume_cuts(if number % 2 == 0 { first } else { second });
        pace.frame(|| bench.render(number, still, Shown::Rest));
    }
    let start = SETTLE_FRAMES + SETTLE;
    let mut a = Vec::with_capacity(PAIRS as usize);
    let mut b = Vec::with_capacity(PAIRS as usize);
    for pair in 0..PAIRS {
        let number = start + pair * 2;
        bench.renderer.set_volume_cuts(first);
        let timings = pace.frame(|| bench.render(number, still, Shown::Rest));
        a.push(volume_ms(&timings));
        bench.renderer.set_volume_cuts(second);
        let timings = pace.frame(|| bench.render(number + 1, still, Shown::Rest));
        b.push(volume_ms(&timings));
    }
    let mut sorted = a.clone();
    sorted.sort_by(f64::total_cmp);
    let fast = percentile(&sorted, 0.1) * 1.15;
    let kept: Vec<usize> = (0..a.len()).filter(|&i| a[i] <= fast).collect();
    let pick = |values: &[f64]| kept.iter().map(|&i| values[i]).collect::<Vec<f64>>();
    let saved: Vec<f64> = a.iter().zip(&b).map(|(one, two)| one - two).collect();
    let spread = |values: Vec<f64>| {
        let mut values = values;
        values.sort_by(f64::total_cmp);
        Spread {
            median: percentile(&values, 0.5),
            low: percentile(&values, 0.1),
            high: percentile(&values, 0.9),
        }
    };
    Pairs {
        first: spread(pick(&a)),
        second: spread(pick(&b)),
        saved: spread(pick(&saved)),
        kept: kept.len(),
    }
}

fn still(bench: &mut Bench, cuts: VolumeCuts) -> Vec<u8> {
    bench.renderer.set_volume_cuts(cuts);
    let mut pace = manners::Pace::new();
    for number in 0..SETTLE_FRAMES {
        pace.frame(|| bench.render(number, true, Shown::Rest));
    }
    bench.pixels()
}

struct Region {
    mask: Vec<bool>,
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
    centre: (u32, u32),
}

fn footprint(steamed: &[u8], clear: &[u8], width: u32, height: u32) -> Region {
    let mut mask = vec![false; (width * height) as usize];
    let mut columns = vec![0u32; width as usize];
    let mut rows = vec![0u32; height as usize];
    let (mut sum_x, mut sum_y, mut count) = (0u64, 0u64, 0u64);
    for (index, (a, b)) in steamed
        .chunks_exact(4)
        .zip(clear.chunks_exact(4))
        .enumerate()
    {
        let delta = (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0);
        if delta >= VISIBLE {
            mask[index] = true;
            let x = index as u32 % width;
            let y = index as u32 / width;
            columns[x as usize] += 1;
            rows[y as usize] += 1;
            sum_x += u64::from(x);
            sum_y += u64::from(y);
            count += 1;
        }
    }
    let count = count.max(1);
    let edge = |counts: &[u32], fraction: f64, from_end: bool| -> u32 {
        let limit = (count as f64 * fraction) as u64;
        let mut seen = 0u64;
        let order: Vec<usize> = if from_end {
            (0..counts.len()).rev().collect()
        } else {
            (0..counts.len()).collect()
        };
        for at in order {
            seen += u64::from(counts[at]);
            if seen > limit {
                return at as u32;
            }
        }
        0
    };
    Region {
        mask,
        left: edge(&columns, 0.002, false),
        top: edge(&rows, 0.002, false),
        right: edge(&columns, 0.002, true) + 1,
        bottom: edge(&rows, 0.002, true) + 1,
        centre: ((sum_x / count) as u32, (sum_y / count) as u32),
    }
}

fn region_difference(reference: &[u8], other: &[u8], region: &Region, width: u32) -> (f64, u8) {
    let mut total = 0u64;
    let mut worst = 0u8;
    let mut count = 0u64;
    for y in region.top..region.bottom {
        for x in region.left..region.right {
            let index = (y * width + x) as usize;
            let at = index * 4;
            let mut delta = 0u8;
            for c in 0..3 {
                delta = delta.max(reference[at + c].abs_diff(other[at + c]));
            }
            worst = worst.max(delta);
            if region.mask[index] {
                for c in 0..3 {
                    total += u64::from(reference[at + c].abs_diff(other[at + c]));
                }
                count += 3;
            }
        }
    }
    (total as f64 / count.max(1) as f64, worst)
}

fn overlap(today: &Region, other: &Region) -> (f64, f64) {
    let (mut both, mut either, mut base) = (0u64, 0u64, 0u64);
    for (a, b) in today.mask.iter().zip(&other.mask) {
        both += u64::from(*a && *b);
        either += u64::from(*a || *b);
        base += u64::from(*a);
    }
    (
        both as f64 / base.max(1) as f64,
        both as f64 / either.max(1) as f64,
    )
}

fn crop(pixels: &[u8], width: u32, x0: u32, y0: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((CROP * CROP * 4) as usize);
    for y in y0..y0 + CROP {
        let start = ((y * width + x0) * 4) as usize;
        out.extend_from_slice(&pixels[start..start + (CROP * 4) as usize]);
    }
    out
}

fn triptych(path: &Path, size: (u32, u32), before: &[u8], after: &[u8], centre: (u32, u32)) {
    let (width, height) = size;
    let x0 = centre.0.saturating_sub(CROP / 2).min(width - CROP);
    let y0 = centre.1.saturating_sub(CROP / 2).min(height - CROP);
    let one = crop(before, width, x0, y0);
    let two = crop(after, width, x0, y0);
    let delta: Vec<u8> = one
        .chunks_exact(4)
        .zip(two.chunks_exact(4))
        .flat_map(|(a, b)| {
            let d = (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0);
            let v = d.saturating_mul(GAIN);
            [v, v, v, 255]
        })
        .collect();
    let row = CROP as usize * 4;
    let mut out = Vec::with_capacity((CROP * CROP * 12) as usize);
    for y in 0..CROP as usize {
        for part in [&one, &two, &delta] {
            out.extend_from_slice(&part[y * row..(y + 1) * row]);
        }
    }
    save_png(path, CROP * 3, CROP, &out);
}

fn row(name: &str, rest: &Pairs, moving: &Pairs, mean: f64, worst: u8, covered: f64, iou: f64) {
    println!(
        "| {name} | {} | {} | {} | {} | {} | {} | {} | {} | {mean:.3} | {worst} | {:.1}% | {iou:.3} |",
        rest.first,
        rest.second,
        rest.saved,
        rest.kept,
        moving.first,
        moving.second,
        moving.saved,
        moving.kept,
        covered * 100.0,
    );
}

pub fn cuts(bench: &mut Bench) {
    let (width, height) = bench.renderer.size();
    let today = VolumeCuts::FULL;
    let shipped = VolumeCuts::default();
    let early = VolumeCuts {
        early_out: true,
        ..today
    };
    let mut rows: Vec<(String, VolumeCuts)> = vec![
        ("today".into(), today),
        ("early out before the noise (shipped)".into(), shipped),
        (
            "early out, 1 step (the pass, the upsample and one step)".into(),
            VolumeCuts { steps: 1, ..early },
        ),
    ];
    for steps in [16, 12] {
        rows.push((
            format!("early out, {steps} steps"),
            VolumeCuts { steps, ..early },
        ));
    }
    for warp_octaves in [3, 2] {
        rows.push((
            format!("early out, {warp_octaves} warp octaves"),
            VolumeCuts {
                warp_octaves,
                ..early
            },
        ));
    }
    rows.push((
        "early out, 3 octaves".into(),
        VolumeCuts {
            octaves: 3,
            ..early
        },
    ));
    rows.push((
        "early out, 3 warp octaves, 3 octaves".into(),
        VolumeCuts {
            warp_octaves: 3,
            octaves: 3,
            ..early
        },
    ));
    rows.push((
        "early out, 12 steps, 3 warp octaves".into(),
        VolumeCuts {
            steps: 12,
            warp_octaves: 3,
            ..early
        },
    ));
    if let Ok(only) = std::env::var("BENCH_VOLUME_ROWS") {
        let keep: Vec<usize> = only
            .split(',')
            .filter_map(|part| part.trim().parse().ok())
            .collect();
        let mut index = 0;
        rows.retain(|_| {
            index += 1;
            keep.contains(&(index - 1))
        });
    }
    let steam = bench.steam;
    bench.steam = None;
    let clear = still(bench, today);
    bench.steam = steam;
    still(bench, today);
    let reference = still(bench, today);
    let folder = root();
    save_png(&folder.join("volume-clear.png"), width, height, &clear);
    save_png(&folder.join("volume-today.png"), width, height, &reference);
    let region = footprint(&reference, &clear, width, height);
    println!(
        "volume cuts: {width}x{height}, steam footprint (pixels the steam moves by {VISIBLE}/255 or more over the frame without it; the box trims 0.2% of them at each side) {} px, box {}..{} x {}..{} ({} x {}), centroid {:?}; {PAIRS} frame pairs per row after {SETTLE} frames to settle, each pair one frame as today and one with the row's cuts, pairs kept where today's frame ran at the fast clock; images compared with today's frame at rest: the mean is over the footprint's pixels, the worst over its box; today's cuts are {today:?}",
        region.mask.iter().filter(|m| **m).count(),
        region.left,
        region.right,
        region.top,
        region.bottom,
        region.right - region.left,
        region.bottom - region.top,
        region.centre
    );
    println!(
        "| row | rest ms, today | rest ms, cut | rest saved | pairs | moving ms, today | moving ms, cut | moving saved | pairs | mean /255 | worst /255 | footprint covered | IoU |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for (index, (name, cut)) in rows.into_iter().enumerate() {
        let rest = paired(bench, "volume", true, today, cut);
        let moving = paired(bench, "volume", false, today, cut);
        let reference = still(bench, today);
        let pixels = still(bench, cut);
        let (mean, worst) = region_difference(&reference, &pixels, &region, width);
        let seen = footprint(&pixels, &clear, width, height);
        let (covered, iou) = overlap(&region, &seen);
        let (frame_mean, frame_worst) = difference(&reference, &pixels);
        row(&name, &rest, &moving, mean, worst, covered, iou);
        println!("  whole frame: mean {frame_mean:.5}, worst {frame_worst}");
        triptych(
            &folder.join(format!("volume-cut-{index}.png")),
            (width, height),
            &reference,
            &pixels,
            region.centre,
        );
    }
    bench.renderer.set_volume_cuts(shipped);
}
