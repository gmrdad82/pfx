#[allow(dead_code)]
#[path = "../tests/support/desk.rs"]
mod desk;

use std::time::{Duration, Instant};

use desk::{Desk, Pose, Show};
use pfx_gpu::PassTiming;
use pfx_gpu::pace::{Frames, Pace};
use pfx_live::demand::{Need, WAKE_GROUPS};

fn gpu_ms(timings: &[PassTiming]) -> f64 {
    timings.iter().map(|pass| pass.milliseconds).sum()
}

fn sclk() -> Option<u32> {
    let cards = std::fs::read_dir("/sys/class/drm").ok()?;
    let mut paths: Vec<_> = cards
        .flatten()
        .map(|entry| entry.path().join("device/pp_dpm_sclk"))
        .filter(|path| path.exists())
        .collect();
    paths.sort();
    let text = std::fs::read_to_string(paths.first()?).ok()?;
    text.lines()
        .find(|line| line.trim_end().ends_with('*'))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|clock| {
            clock
                .trim_end_matches("Mhz")
                .trim_end_matches("MHz")
                .parse()
                .ok()
        })
}

struct Tally {
    label: String,
    ticks: usize,
    kinds: [usize; 4],
    per_kind: [f64; 4],
    gpu: Vec<f64>,
    clocks: Vec<u32>,
}

impl Tally {
    fn new(label: String) -> Self {
        Self {
            label,
            ticks: 0,
            kinds: [0; 4],
            per_kind: [0.0; 4],
            gpu: Vec::new(),
            clocks: Vec::new(),
        }
    }

    fn add(&mut self, need: &Need, timings: &[PassTiming]) {
        let kind = match need {
            Need::None => 0,
            Need::Reproject => 1,
            Need::Partial(_) => 2,
            Need::Full => 3,
        };
        let ms = gpu_ms(timings);
        self.ticks += 1;
        self.kinds[kind] += 1;
        self.per_kind[kind] += ms;
        self.gpu.push(ms);
        self.clocks.extend(sclk());
    }

    fn print(&mut self) {
        self.gpu.sort_by(f64::total_cmp);
        self.clocks.sort();
        let clock = self
            .clocks
            .get(self.clocks.len() / 2)
            .map_or("?".to_string(), |mhz| mhz.to_string());
        let frames = Frames::of(&self.gpu);
        let mean = self.gpu.iter().sum::<f64>() / self.ticks.max(1) as f64;
        let each = |kind: usize| {
            if self.kinds[kind] == 0 {
                "-".to_string()
            } else {
                format!(
                    "{} × {:.3}",
                    self.kinds[kind],
                    self.per_kind[kind] / self.kinds[kind] as f64
                )
            }
        };
        println!(
            "{}: GPU mean {mean:.3} ms per tick, p50 {:.3}, p95 {:.3}, sclk p50 {clock} MHz | none {}, reproject {}, partial {}, full {}",
            self.label,
            frames.p50,
            frames.p95,
            each(0),
            each(1),
            each(2),
            each(3)
        );
    }
}

fn run(
    desk: &mut Desk,
    label: String,
    ticks: u32,
    pose: impl Fn(f32) -> Pose,
    show: Show,
    force: Option<Need>,
) {
    let mut pace = Pace::new();
    let mut tally = Tally::new(label);
    let start = 10.0;
    for tick in 0..ticks {
        let time = start + tick as f32 / 60.0;
        let began = Instant::now();
        let (need, timings) = desk.draw(pose(time), time, show, force.clone());
        let ms = began.elapsed().as_secs_f64() * 1000.0;
        let gpu = gpu_ms(&timings);
        pace.after_gpu(ms, (gpu > 0.0).then_some(gpu));
        tally.add(&need, &timings);
    }
    tally.print();
}

fn passes(desk: &mut Desk, pose: Pose, show: Show, need: Need) {
    let (_, timings) = desk.draw(pose, 10.0, show, Some(need.clone()));
    let mut listed: Vec<String> = timings
        .iter()
        .filter(|pass| pass.milliseconds >= 0.005)
        .map(|pass| format!("{} {:.3}", pass.label, pass.milliseconds))
        .collect();
    listed.truncate(12);
    println!("  {need:?} passes: {}", listed.join(", "));
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(f64::NAN)
}

type Trial = (Option<u32>, Option<u32>, f64, f64, f64, f64);

fn wake_study(desk: &mut Desk, size: &str) {
    let show = Show {
        overlay: true,
        ..Show::default()
    };
    let rest = Pose::default();
    let jump = Pose { yaw: 2.0, ..rest };
    let mut wake = Vec::new();
    for _ in 0..10 {
        desk.renderer.wake(WAKE_GROUPS).unwrap();
        let arrived = desk.renderer.wait_frames(0).unwrap();
        wake.extend(arrived.iter().map(|timings| gpu_ms(&timings.passes)));
    }
    println!(
        "{size} first frame after idle (the camera jumps 2°, so a full frame is due); a wake of {WAKE_GROUPS} groups costs {:.3} ms of GPU",
        median(wake)
    );
    println!(
        "  idle s | variant      | sclk MHz before → after | bridge GPU ms | bridge shown ms | full GPU ms | full done ms"
    );
    let mut steady = Vec::new();
    for idle in [0.0, 0.1, 0.3, 1.0, 2.5] {
        for variant in ["plain", "wake", "wake ×8", "bridge", "wake+bridge"] {
            let mut rows = Vec::new();
            for _ in 0..5 {
                let mut pace = Pace::new();
                for tick in 0..40 {
                    let pose = if tick % 2 == 0 { rest } else { jump };
                    let began = Instant::now();
                    let (_, timings) = desk.draw(pose, 10.0, show, Some(Need::Full));
                    let gpu = gpu_ms(&timings);
                    if tick >= 30 {
                        steady.push(gpu);
                    }
                    pace.after_gpu(began.elapsed().as_secs_f64() * 1000.0, Some(gpu));
                }
                desk.draw(rest, 10.0, show, Some(Need::Full));
                std::thread::sleep(Duration::from_secs_f64(idle));
                let before = sclk();
                let input = Instant::now();
                if variant.starts_with("wake") {
                    let groups = if variant == "wake ×8" {
                        WAKE_GROUPS * 8
                    } else {
                        WAKE_GROUPS
                    };
                    desk.renderer.wake(groups).unwrap();
                }
                let (bridge_gpu, bridge_shown) = if variant.ends_with("bridge") {
                    let timings = desk.bridge(jump, 10.0, show).unwrap();
                    (gpu_ms(&timings), input.elapsed().as_secs_f64() * 1000.0)
                } else {
                    std::thread::sleep(Duration::from_micros(300));
                    (f64::NAN, f64::NAN)
                };
                let (_, timings) = desk.draw(jump, 10.0, show, Some(Need::Full));
                let done = input.elapsed().as_secs_f64() * 1000.0;
                let after = sclk();
                rows.push((
                    before,
                    after,
                    bridge_gpu,
                    bridge_shown,
                    gpu_ms(&timings),
                    done,
                ));
            }
            let pick = |f: &dyn Fn(&Trial) -> f64| median(rows.iter().map(f).collect());
            let clock = |f: &dyn Fn(&Trial) -> Option<u32>| {
                let mut values: Vec<u32> = rows.iter().filter_map(f).collect();
                values.sort();
                values
                    .get(values.len() / 2)
                    .map_or("?".to_string(), |mhz| mhz.to_string())
            };
            let shown = |value: f64| {
                if value.is_nan() {
                    "-".to_string()
                } else {
                    format!("{value:.2}")
                }
            };
            println!(
                "  {idle:>6} | {variant:<12} | {:>5} → {:<5}           | {:>13} | {:>15} | {:>11.2} | {:>12.2}",
                clock(&|row| row.0),
                clock(&|row| row.1),
                shown(pick(&|row| row.2)),
                shown(pick(&|row| row.3)),
                pick(&|row| row.4),
                pick(&|row| row.5),
            );
        }
    }
    println!(
        "  steady full frames between trials: GPU p50 {:.2} ms",
        median(steady)
    );
}

fn partial_study(desk: &mut Desk, size: &str) {
    desk.renderer.set_finish(desk::kuwahara_finish());
    let plain = Show {
        overlay: true,
        ..Show::default()
    };
    let steaming = Show {
        steam: true,
        ..plain
    };
    for (width, height) in [(64, 64), (512, 256)] {
        let rect = desk.fit_steam(width, height);
        for tick in 0..30 {
            desk.draw(
                Pose::default(),
                10.0 + tick as f32 / 60.0,
                steaming,
                Some(Need::Full),
            );
        }
        run(
            desk,
            format!(
                "{size} steam in {}×{} under Kuwahara 3, a vignette and ACES",
                rect.width, rect.height
            ),
            300,
            |_| Pose::default(),
            steaming,
            None,
        );
        let need = desk.needed(Pose::default(), 10.0, steaming);
        if let Need::Partial(regions) = &need {
            let rects: Vec<String> = regions
                .iter()
                .map(|r| format!("{}×{} at {},{}", r.width, r.height, r.x, r.y))
                .collect();
            println!("  partial regions: {}", rects.join(", "));
        }
        passes(desk, Pose::default(), steaming, need);
    }
    desk.steam_box = Desk::STEAM_BOX;
    desk.renderer.clear_finish();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let sizes: Vec<(u32, u32)> = if args.iter().any(|arg| arg == "--small") {
        vec![(1280, 800)]
    } else {
        vec![(3840, 2160), (1280, 800)]
    };
    let sizes: Vec<(u32, u32)> = if args.iter().any(|arg| arg == "--large") {
        vec![(3840, 2160)]
    } else {
        sizes
    };
    let study = !args.iter().any(|arg| arg == "--no-wake");
    let only_drift = args.iter().any(|arg| arg == "--drift");
    for (width, height) in sizes {
        let size = format!("{width}x{height}");
        let mut desk = Desk::new(width, height);
        let plain = Show {
            overlay: true,
            ..Show::default()
        };
        let steaming = Show {
            steam: true,
            ..plain
        };
        for tick in 0..30 {
            desk.draw(Pose::default(), tick as f32 / 60.0, plain, Some(Need::Full));
        }
        if args.iter().any(|arg| arg == "--wake") {
            wake_study(&mut desk, &size);
            continue;
        }
        if args.iter().any(|arg| arg == "--partial") {
            partial_study(&mut desk, &size);
            continue;
        }
        if only_drift {
            let bare = Show::default();
            run(
                &mut desk,
                format!("{size} drift, no overlay"),
                300,
                Pose::drift,
                bare,
                None,
            );
            passes(&mut desk, Pose::drift(10.0), bare, Need::Reproject);
            run(
                &mut desk,
                format!("{size} drift"),
                300,
                Pose::drift,
                plain,
                None,
            );
            passes(&mut desk, Pose::default(), plain, Need::Full);
            continue;
        }
        run(
            &mut desk,
            format!("{size} full"),
            300,
            Pose::drift,
            plain,
            Some(Need::Full),
        );
        passes(&mut desk, Pose::default(), plain, Need::Full);
        run(
            &mut desk,
            format!("{size} full with steam"),
            300,
            Pose::drift,
            steaming,
            Some(Need::Full),
        );
        desk.settle(Pose::default(), 10.0, plain);
        run(
            &mut desk,
            format!("{size} idle"),
            300,
            |_| Pose::default(),
            plain,
            None,
        );
        let output = desk.output.view.clone();
        let mut present = Vec::new();
        for _ in 0..60 {
            let submitted = desk.renderer.present_last(&output).unwrap();
            let _ = submitted;
            let arrived = desk.renderer.wait_frames(0).unwrap();
            present.extend(arrived.iter().map(|timings| gpu_ms(&timings.passes)));
        }
        present.sort_by(f64::total_cmp);
        if !present.is_empty() {
            println!(
                "{size} idle with a present each vsync: GPU p50 {:.3} ms",
                present[present.len() / 2]
            );
        }
        run(
            &mut desk,
            format!("{size} drift"),
            600,
            Pose::drift,
            plain,
            None,
        );
        let bare = Show::default();
        run(
            &mut desk,
            format!("{size} drift, no overlay"),
            600,
            Pose::drift,
            bare,
            None,
        );
        passes(&mut desk, Pose::drift(10.0), plain, Need::Reproject);
        passes(&mut desk, Pose::drift(10.0), bare, Need::Reproject);
        desk.settle(Pose::default(), 10.0, plain);
        run(
            &mut desk,
            format!("{size} steam, still camera"),
            300,
            |_| Pose::default(),
            steaming,
            None,
        );
        let need = desk.needed(Pose::default(), 10.0, steaming);
        passes(&mut desk, Pose::default(), steaming, need);
        run(
            &mut desk,
            format!("{size} drift with steam"),
            600,
            Pose::drift,
            steaming,
            None,
        );
        passes(&mut desk, Pose::drift(10.0), steaming, Need::Reproject);
        if study {
            wake_study(&mut desk, &size);
        }
        let shot = std::path::Path::new("tmp/frames");
        std::fs::create_dir_all(shot).unwrap();
        desk.draw(Pose::default(), 10.0, steaming, Some(Need::Full));
        let pixels = desk.pixels();
        desk::save_png(
            &shot.join(format!("desk-{size}.png")),
            width,
            height,
            &pixels,
        );
    }
}
