use crate::camera::layout_in;
use crate::cut::{Cast, Director};
use crate::input;
use crate::mix::Mixdown;
use crate::score::{CHANNELS, Sounds};
use crate::script::Script;
use crate::show::{FilmFrame, Show, plan};
use crate::write::Encoder;

pub const RATE: u32 = 48_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rig {
    pub size: [u32; 2],
    pub fps: u32,
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

pub fn samples(frame: u64, fps: u32) -> u64 {
    frame * u64::from(RATE) / u64::from(fps.max(1))
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
    for (scene, take) in script.scenes.iter().zip(&takes) {
        show.seek(scene, take)?;
        director.begin();
        mix.cut();
        for i in 0..take.frames {
            let s = i as f64 / f64::from(fps.max(1));
            let next = (i + 1) as f64 / f64::from(fps.max(1));
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
            let rgba = show.readback()?;
            if let Some(name) = still {
                out.still(name, rgba.clone(), rig.size)?;
                count.stills += 1;
            }
            if script.film.frames {
                out.frame(at, rgba, rig.size)?;
            }
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
