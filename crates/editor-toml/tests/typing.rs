use std::time::Instant;

use egui::{Event, Pos2, RawInput, Rect, Vec2};
use pfx_editor_doc::{Files, History};
use pfx_editor_shell::context;
use pfx_editor_style::fixture;
use pfx_editor_toml::{NoCheck, Source};

const LINES: usize = 10_000;
const FRAMES: usize = 120;

fn scene() -> String {
    let mut text = String::new();
    for index in 0..LINES / 5 {
        text.push_str(&format!(
            "[object.n{index}]\nname = \"part {index}\" # a part\nat = [{index}.0, 1.5, -2.0]\nlit = true\n\n"
        ));
    }
    text
}

fn input(time: f64, events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
        time: Some(time),
        predicted_dt: 1.0 / 60.0,
        focused: true,
        events,
        ..RawInput::default()
    }
}

fn median(mut times: Vec<f64>) -> (f64, f64) {
    times.sort_by(f64::total_cmp);
    (times[times.len() / 2], times[times.len() - 1])
}

#[test]
#[ignore = "bench: cargo test --release -p pfx-editor-toml --test typing -- --ignored --nocapture"]
fn typing_into_ten_thousand_lines_times_each_frame() {
    let text = scene();
    let ctx = context(&fixture::theme());
    let mut files = Files::new();
    files.open("scene.toml", text.clone());
    let mut history = History::default();
    let mut source = Source::new("bench", "scene.toml");
    source.sync(&files, &NoCheck);
    source.jump(text.len() / 2);
    let mut time = 0.0;
    let mut frame = |source: &mut Source, events: Vec<Event>| {
        time += 1.0 / 60.0;
        let start = Instant::now();
        let output = ctx.run(input(time, events), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                source.show(ui, &mut files, &mut history, &NoCheck, &mut |_, _| {});
            });
        });
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        std::hint::black_box(primitives);
        start.elapsed().as_secs_f64() * 1000.0
    };
    for _ in 0..3 {
        frame(&mut source, Vec::new());
    }
    let misses = source.cache().misses();
    let idle: Vec<f64> = (0..FRAMES)
        .map(|_| frame(&mut source, Vec::new()))
        .collect();
    assert_eq!(source.cache().misses(), misses);
    let typed: Vec<f64> = (0..FRAMES)
        .map(|_| frame(&mut source, vec![Event::Text("x".into())]))
        .collect();
    assert_eq!(source.text().len(), text.len() + FRAMES);
    assert!(
        source.cache().relexed() <= 2,
        "{}",
        source.cache().relexed()
    );
    let (idle_median, idle_worst) = median(idle);
    let (typed_median, typed_worst) = median(typed);
    println!(
        "{LINES} lines, {} bytes: idle frame median {idle_median:.2} ms, worst {idle_worst:.2} ms; typing frame median {typed_median:.2} ms, worst {typed_worst:.2} ms",
        text.len()
    );
}
