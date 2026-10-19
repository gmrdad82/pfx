use crate::camera::layout_in;
use crate::cut::{Cast, Director};
use crate::input;
use crate::mix::Mixdown;
use crate::score::{CHANNELS, Sounds};
use crate::script::Script;
use crate::show::{FilmFrame, Show, plan};
use crate::write::Encoder;
use pfx_core::rate::Rate;

pub const RATE: u32 = 48_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rig {
    pub size: [u32; 2],
    pub fps: Rate,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reeled {
    pub frames: u64,
    pub ticks: u64,
    pub stills: u64,
    pub samples: u64,
}

pub struct Crew<'a, S: Show> {
    pub show: &'a mut S,
    pub cast: &'a dyn Cast<S::Kinds>,
    pub score: &'a mut dyn Sounds,
    pub encoder: Option<&'a mut dyn Encoder>,
}

pub fn samples(frame: u64, fps: Rate) -> u64 {
    fps.samples(frame, RATE)
}

pub fn clock(frame: u64, start: f64, fps: Rate) -> f64 {
    (fps.time(frame) - start).max(0.0)
}

pub fn shoot<S: Show>(
    script: &Script<S::Kinds>,
    rig: Rig,
    crew: Crew<'_, S>,
) -> Result<Reeled, String> {
    let Crew {
        show,
        cast,
        score,
        mut encoder,
    } = crew;
    let fps = rig.fps;
    let takes = plan(show, script, fps)?;
    show.set_theme(script.film.theme);
    let mut director = Director::new(script, cast, layout_in(script.film.layout, rig.size));
    let mut mix = Mixdown::new(script, RATE, score);
    mix.route()?;
    let mut count = Reeled::default();
    let mut fresh = Vec::new();
    let starts = script.starts();
    let cuts = script.cuts(fps);
    let pictures = encoder.is_some() && script.film.frames;
    let mut tail: Vec<Vec<u8>> = Vec::new();
    for (k, ((scene, take), start)) in script.scenes.iter().zip(&takes).zip(&starts).enumerate() {
        show.seek(scene, take)?;
        director.begin();
        mix.cut();
        let first = count.frames;
        let last = take.frames - 1;
        for i in 0..take.frames {
            let s = clock(first + i, *start, fps);
            let next = if i == last {
                scene.seconds
            } else {
                clock(first + i + 1, *start, fps)
            };
            let aim = show.aim(scene);
            show.tick(count.frames, &input::events(&scene.input, (s, next)));
            let cut = director.cut(scene, s, fps, aim);
            let (speed, gain) = (cut.speed, if cut.grade.game { cut.gain } else { 0.0 });
            let frame = FilmFrame {
                index: count.frames,
                fps,
                speed,
                cut: &cut,
            };
            show.draw(&frame)
                .map_err(|e| format!("scene '{}': {e} at frame {}", scene.name, count.frames))?;
            let at = count.frames;
            count.frames += 1;
            fresh.clear();
            show.audio(&mut fresh);
            mix.feed(&fresh);
            let due = samples(count.frames, fps) - samples(at, fps);
            mix.frame(due as usize, speed, gain)?;
            let still = scene.still.as_ref().filter(|_| i + 1 == take.frames);
            let Some(out) = encoder.as_deref_mut() else {
                continue;
            };
            if !script.film.frames && still.is_none() {
                continue;
            }
            if still.is_none() && out.kept(at) {
                continue;
            }
            let mut rgba = show.readback()?;
            if let Some(t) = &scene.transition
                && let Some(out) = tail.get(i as usize)
            {
                let at_time = |f: u64| ((fps.time(f) - start) / t.seconds) as f32;
                rgba = t
                    .mix(
                        out,
                        &rgba,
                        rig.size,
                        at_time(at),
                        at_time(at.saturating_sub(1)),
                    )
                    .map_err(|e| format!("scene '{}': {e}", scene.name))?;
            }
            if let Some(name) = still {
                out.still(name, rgba.clone(), rig.size)?;
                count.stills += 1;
            }
            if script.film.frames {
                out.frame(at, rgba, rig.size)?;
            }
        }
        tail.clear();
        let Some(t) = script.scenes.get(k + 1).and_then(|n| n.transition.as_ref()) else {
            continue;
        };
        if !pictures {
            continue;
        }
        let end = fps.frames(starts[k + 1] + t.seconds).round() as u64;
        let overlap = end
            .saturating_sub(cuts[k + 1])
            .clamp(1, takes[k + 1].frames);
        let held = clock(first + last, *start, fps);
        for j in 0..overlap {
            let s = clock(first + take.frames + j, *start, fps);
            let aim = show.aim(scene);
            show.tick(count.frames + j, &[]);
            let cut = director.cut(scene, s.min(held), fps, aim);
            let frame = FilmFrame {
                index: count.frames + j,
                fps,
                speed: cut.speed,
                cut: &cut,
            };
            show.draw(&frame).map_err(|e| {
                format!("scene '{}': {e} in the {} out of it", scene.name, t.name())
            })?;
            fresh.clear();
            show.audio(&mut fresh);
            tail.push(show.readback()?);
        }
    }
    count.ticks = show.ticks();
    let audio = mix.master();
    count.samples = audio.len() as u64 / u64::from(CHANNELS);
    show.finish()?;
    if let Some(out) = encoder {
        out.finish(script.film.audio.then_some(&audio[..]), RATE, CHANNELS)?;
    }
    Ok(count)
}
