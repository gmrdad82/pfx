use crate::art::{Art, Shape, What};
use crate::camera::Camera;
use crate::colour::{colour, srgba};
use crate::grade::Grade;
use crate::layers::{Style, Text};
use crate::marks::{Badge, LABEL};
use crate::motion::{merged, motion};
use crate::pieces::{Art as Drawn, Piece};
use crate::script::{Kinds, Scene, Script};
use crate::titles::{Align, Words};
use crate::track::{Look, look};
use pfx_core::fx::{Shake, ShakeTuning};
use std::collections::BTreeMap;

const FOLLOW: f32 = 2.5;
const OUTLINE: f32 = 0.045;
const THINNEST: f32 = 2.0;
const MARK: f32 = 48.0;
const BLEED: f32 = 40.0;
const SHAKE: ShakeTuning = ShakeTuning {
    translation: [30.0, 22.0],
    roll: 0.02,
    frequency: 13.0,
    decay: 1.3,
    seed: 0x0066_696c_6d00,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Cut<D> {
    pub grade: Grade,
    pub pieces: Vec<Piece<D>>,
    pub speed: f32,
    pub gain: f32,
}

impl<D> Default for Cut<D> {
    fn default() -> Cut<D> {
        Cut {
            grade: Grade::default(),
            pieces: Vec::new(),
            speed: 0.0,
            gain: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Posed {
    pub centre: [f32; 2],
    pub age: f64,
    pub layout: [f32; 2],
}

pub trait Cast<K: Kinds> {
    fn fill(&self, text: &str) -> String;

    fn place(&self, _art: &K::Art, _layout: [f32; 2]) -> Option<[f32; 2]> {
        None
    }

    fn pose(&self, art: &K::Art, posed: Posed) -> Option<(K::Drawn, f32)>;
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Base {
    bars: f32,
    grain: f32,
    vignette: f32,
    glow: f32,
    safe: [f32; 2],
    wide: [f32; 2],
}

pub struct Director<'a, K: Kinds> {
    cast: &'a dyn Cast<K>,
    film: Base,
    styles: BTreeMap<String, Style>,
    shake: Shake,
    last: f64,
    aim: Option<[f32; 2]>,
    layout: [f32; 2],
}

impl<'a, K: Kinds> Director<'a, K> {
    pub fn new(script: &Script<K>, cast: &'a dyn Cast<K>, layout: [f32; 2]) -> Director<'a, K> {
        let f = &script.film;
        Director {
            layout,
            cast,
            film: Base {
                bars: f.bars,
                grain: f.grain,
                vignette: f.vignette,
                glow: f.glow,
                safe: f.safe,
                wide: f.layout,
            },
            styles: script.style.clone(),
            shake: Shake::new(SHAKE),
            last: 0.0,
            aim: None,
        }
    }

    fn middle(&self) -> [f32; 2] {
        [self.layout[0] * 0.5, self.layout[1] * 0.5]
    }

    pub fn begin(&mut self) {
        self.shake = Shake::new(SHAKE);
        self.last = 0.0;
        self.aim = None;
    }

    fn track(&mut self, target: Option<[f32; 2]>, fps: u32) -> Option<[f32; 2]> {
        let to = target?;
        let k = 1.0 - (-FOLLOW / fps.max(1) as f32).exp();
        let at = self.aim.map_or(to, |a| {
            [a[0] + (to[0] - a[0]) * k, a[1] + (to[1] - a[1]) * k]
        });
        self.aim = Some(at);
        Some(at)
    }

    fn base(&self, scene: &Scene<K>) -> Look {
        Look {
            bars: self.film.bars,
            grain: self.film.grain,
            vignette: self.film.vignette,
            glow: self.film.glow,
            gain: if K::shown(scene) { 1.0 } else { 0.0 },
            focus: [self.film.wide[0] * 0.5, self.film.wide[1] * 0.5],
            ..Look::default()
        }
    }

    fn text(&self, t: &Text, scene: &Scene<K>, s: f64) -> Option<Piece<K::Drawn>> {
        let style = merged(t, &self.styles);
        let end = t.until.unwrap_or(scene.seconds);
        let m = motion(t.at, end, s, &style)?;
        let filled = self.cast.fill(&t.text);
        let text = if style.caps.unwrap_or(false) {
            filled.to_uppercase()
        } else {
            filled
        };
        let place = style.place.unwrap_or(self.middle());
        let size = style.size.unwrap_or(64.0);
        let tracking = style.tracking.unwrap_or(0.0) + style.spread.unwrap_or(0.0) * m.progress;
        let mut w = Words::new(
            text,
            [place[0] + m.offset[0], place[1] + m.offset[1]],
            size,
            srgba(style.colour.as_ref(), 1.0, [1.0; 3]),
        );
        w.align = match style.align.as_deref() {
            Some("start") => Align::Start,
            Some("end") => Align::End,
            _ => Align::Middle,
        };
        w.weight = style.weight.unwrap_or(700);
        w.tracking = tracking;
        w.leading = style.leading.unwrap_or(1.2);
        w.outline = style.outline.unwrap_or((size * OUTLINE).max(THINNEST));
        w.edge = srgba(style.edge.as_ref(), 1.0, [0.0; 3]);
        Some(Piece {
            art: Drawn::Words(w),
            opacity: m.opacity,
            scale: m.scale,
        })
    }

    fn art(&self, a: &Art<K::Art>, scene: &Scene<K>, s: f64) -> Option<Piece<K::Drawn>> {
        let style = Style {
            fade: a.fade,
            out: a.out,
            grow: a.grow,
            smash: a.smash,
            slide: a.slide,
            ..Style::default()
        };
        let end = a.until.unwrap_or(scene.seconds);
        let m = motion(a.at, end, s, &style)?;
        let p = a.place.unwrap_or(match &a.what {
            What::App(app) => self.cast.place(app, self.layout).unwrap_or(self.middle()),
            What::Plain(_) => self.middle(),
        });
        let centre = [p[0] + m.offset[0], p[1] + m.offset[1]];
        let a = match &a.what {
            What::Plain(plain) => plain,
            What::App(app) => {
                let posed = Posed {
                    centre,
                    age: m.age,
                    layout: self.layout,
                };
                let (drawn, grown) = self.cast.pose(app, posed)?;
                return Some(Piece {
                    art: Drawn::App(drawn),
                    opacity: m.opacity,
                    scale: m.scale * grown,
                });
            }
        };
        let alpha = a.alpha.unwrap_or(1.0);
        let art = match a.kind {
            Shape::Rule => Drawn::Rule {
                centre,
                size: [a.width.unwrap_or(200.0), a.height.unwrap_or(3.0)],
                colour: srgba(a.colour.as_ref(), alpha, [1.0; 3]),
            },
            Shape::Band => Drawn::Band {
                centre: [self.layout[0] * 0.5, centre[1]],
                size: [self.layout[0] + 2.0 * BLEED, a.height.unwrap_or(120.0)],
                soft: a.soft.unwrap_or(0.0),
                colour: srgba(a.colour.as_ref(), alpha, [0.0; 3]),
            },
            Shape::Glow => Drawn::Glow {
                centre,
                radius: [
                    a.width.unwrap_or(800.0) * 0.5,
                    a.height.unwrap_or(500.0) * 0.5,
                ],
                colour: srgba(a.colour.as_ref(), alpha, [1.0; 3]),
            },
            Shape::Shade => Drawn::Shade {
                centre,
                radius: [
                    a.width.unwrap_or(1200.0) * 0.5,
                    a.height.unwrap_or(700.0) * 0.5,
                ],
                colour: srgba(a.colour.as_ref(), alpha, [0.0; 3]),
            },
            Shape::Spot => Drawn::Spot {
                centre,
                radius: [
                    a.width.unwrap_or(700.0) * 0.5,
                    a.height.unwrap_or(700.0) * 0.5,
                ],
                colour: srgba(a.colour.as_ref(), alpha, [0.0; 3]),
            },
            Shape::Mark => {
                let size = a.size.unwrap_or(MARK);
                Drawn::Mark(Box::new(Badge {
                    mark: a.file.clone()?,
                    centre,
                    size,
                    tint: a
                        .colour
                        .as_ref()
                        .map(|_| srgba(a.colour.as_ref(), 1.0, [1.0; 3])),
                    label: a.label.as_ref().map(|l| self.cast.fill(l)),
                    colour: srgba(a.colour.as_ref(), 1.0, [1.0; 3]),
                    check: a.check,
                    outline: a.outline.unwrap_or((size * LABEL * OUTLINE).max(THINNEST)),
                    edge: srgba(a.edge.as_ref(), 1.0, [0.0; 3]),
                }))
            }
        };
        Some(Piece {
            art,
            opacity: m.opacity,
            scale: m.scale,
        })
    }

    fn flash(scene: &Scene<K>, s: f64) -> [f32; 4] {
        let mut best = [1.0, 1.0, 1.0, 0.0];
        for f in &scene.flash {
            if s < f.at || s >= f.at + f.seconds {
                continue;
            }
            let k = 1.0 - ((s - f.at) / f.seconds) as f32;
            let a = f.amount * k * k;
            if a > best[3] {
                let c = f
                    .colour
                    .as_deref()
                    .and_then(|t| colour(t).ok())
                    .unwrap_or([1.0; 3]);
                best = [c[0], c[1], c[2], a];
            }
        }
        best
    }

    pub fn speed(&self, scene: &Scene<K>, s: f64) -> f32 {
        look(&scene.key, s, self.base(scene)).speed
    }

    pub fn cut(
        &mut self,
        scene: &Scene<K>,
        s: f64,
        fps: u32,
        target: Option<[f32; 2]>,
    ) -> Cut<K::Drawn> {
        for k in &scene.shake {
            if k.at >= self.last && k.at < s + 1e-9 {
                self.shake.add(k.amount);
            }
        }
        self.last = s + 1e-9;
        self.shake.step(1.0 / fps.max(1) as f32, 1.0);
        let jolt = self.shake.offset(1.0);
        let l = look(&scene.key, s, self.base(scene));
        let aim = self.track(target, fps);
        let focus = match aim {
            Some(a) if l.follow > 0.0 => [
                l.focus[0] + (a[0] - l.focus[0]) * l.follow,
                l.focus[1] + (a[1] - l.focus[1]) * l.follow,
            ],
            _ => l.focus,
        };
        let camera = Camera {
            focus,
            zoom: l.zoom,
            roll: l.roll,
            shake: jolt.translation,
            tilt: jolt.roll,
            safe: self.film.safe,
            wide: self.film.wide,
        };
        let grade = Grade {
            camera,
            game: K::shown(scene),
            bars: l.bars,
            black: l.black,
            flash: Self::flash(scene, s),
            desat: l.desat,
            tint: l.tint,
            contrast: l.contrast,
            dim: l.dim,
            vignette: l.vignette,
            grain: l.grain,
            glow: l.glow,
            split: l.split,
        };
        let mut pieces: Vec<Piece<K::Drawn>> = Vec::new();
        pieces.extend(scene.art.iter().filter_map(|a| self.art(a, scene, s)));
        pieces.extend(scene.text.iter().filter_map(|t| self.text(t, scene, s)));
        Cut {
            grade,
            pieces,
            speed: l.speed,
            gain: l.gain,
        }
    }
}
