use super::{Draw, IDENTITY, Matrix, Shadow, Shape, Srgba, Stroke, multiply, sdf, translation};
use pfx_core::ease::Ease;
use std::f32::consts::FRAC_PI_2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub size: [f32; 2],
    pub bezel: f32,
    pub radius: f32,
    pub colour: Srgba,
}

impl Frame {
    pub fn outer(&self, orientation: Orientation) -> [f32; 2] {
        let [a, b] = self.size;
        let (short, long) = (a.min(b), a.max(b));
        match orientation {
            Orientation::Portrait => [short, long],
            Orientation::Landscape => [long, short],
        }
    }

    pub fn screen(&self, orientation: Orientation) -> [f32; 2] {
        let [w, h] = self.outer(orientation);
        let bezel = self.bezel.max(0.0);
        [(w - 2.0 * bezel).max(0.0), (h - 2.0 * bezel).max(0.0)]
    }

    pub fn screen_radius(&self) -> f32 {
        (self.radius - self.bezel.max(0.0)).max(0.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

impl Orientation {
    pub fn other(self) -> Self {
        match self {
            Self::Portrait => Self::Landscape,
            Self::Landscape => Self::Portrait,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mirror {
    #[default]
    None,
    Vertical,
    Horizontal,
}

impl Mirror {
    pub fn signs(self) -> [f32; 2] {
        match self {
            Self::None => [1.0, 1.0],
            Self::Vertical => [1.0, -1.0],
            Self::Horizontal => [-1.0, 1.0],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turn {
    pub to: Orientation,
    pub start: f32,
    pub duration: f32,
    pub ease: Ease,
    pub clockwise: bool,
}

impl Turn {
    pub fn progress(&self, now: f32) -> f32 {
        if self.duration.is_nan() || self.duration <= 0.0 || !self.duration.is_finite() {
            return 1.0;
        }
        ((now - self.start) / self.duration).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub centre: [f32; 2],
    pub frame: Frame,
    pub orientation: Orientation,
    pub turn: Option<Turn>,
    pub mirror: Mirror,
}

impl Viewport {
    pub fn new(centre: [f32; 2], frame: Frame) -> Self {
        Self {
            centre,
            frame,
            orientation: Orientation::Portrait,
            turn: None,
            mirror: Mirror::None,
        }
    }

    pub fn orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    pub fn mirror(mut self, mirror: Mirror) -> Self {
        self.mirror = mirror;
        self
    }

    pub fn turn_to(
        &mut self,
        to: Orientation,
        now: f32,
        duration: f32,
        ease: Ease,
        clockwise: bool,
        reduced_motion: bool,
    ) {
        self.orientation = self.at(now).orientation;
        self.turn = (to != self.orientation).then_some(Turn {
            to,
            start: now,
            duration: if reduced_motion { 0.0 } else { duration },
            ease,
            clockwise,
        });
    }

    pub fn settle(&mut self, now: f32) {
        if let Some(turn) = self.turn
            && turn.progress(now) >= 1.0
        {
            self.orientation = turn.to;
            self.turn = None;
        }
    }

    pub fn at(&self, now: f32) -> Placement {
        let (orientation, angle, progress) = match self.turn {
            Some(turn) if turn.progress(now) < 1.0 => {
                let progress = turn.progress(now);
                let sign = if turn.clockwise { 1.0 } else { -1.0 };
                (
                    self.orientation,
                    sign * FRAC_PI_2 * turn.ease.at(progress),
                    progress,
                )
            }
            Some(turn) => (turn.to, 0.0, 1.0),
            None => (self.orientation, 0.0, 1.0),
        };
        Placement {
            layout: self.frame.screen(orientation),
            orientation,
            angle,
            progress,
            centre: self.centre,
            frame: self.frame,
            mirror: self.mirror,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub layout: [f32; 2],
    pub orientation: Orientation,
    pub angle: f32,
    pub progress: f32,
    pub centre: [f32; 2],
    pub frame: Frame,
    pub mirror: Mirror,
}

impl Placement {
    pub fn settled(&self) -> bool {
        self.progress >= 1.0
    }

    fn turned(&self) -> [[f32; 2]; 2] {
        let (s, c) = self.angle.sin_cos();
        [[c, s], [-s, c]]
    }

    fn linear(&self) -> [[f32; 2]; 2] {
        let r = self.turned();
        let [fx, fy] = self.mirror.signs();
        [[r[0][0] * fx, r[0][1] * fx], [r[1][0] * fy, r[1][1] * fy]]
    }

    pub fn to_window(&self, point: [f32; 2]) -> [f32; 2] {
        let m = self.linear();
        let p = [
            point[0] - self.layout[0] * 0.5,
            point[1] - self.layout[1] * 0.5,
        ];
        [
            self.centre[0] + m[0][0] * p[0] + m[1][0] * p[1],
            self.centre[1] + m[0][1] * p[0] + m[1][1] * p[1],
        ]
    }

    pub fn to_layout(&self, window: [f32; 2]) -> [f32; 2] {
        let m = self.linear();
        let d = [window[0] - self.centre[0], window[1] - self.centre[1]];
        [
            self.layout[0] * 0.5 + m[0][0] * d[0] + m[0][1] * d[1],
            self.layout[1] * 0.5 + m[1][0] * d[0] + m[1][1] * d[1],
        ]
    }

    pub fn pointer(&self, window: [f32; 2]) -> Option<[f32; 2]> {
        let point = self.to_layout(window);
        let half = [self.layout[0] * 0.5, self.layout[1] * 0.5];
        let radius = self.frame.screen_radius().min(half[0]).min(half[1]);
        let inside =
            sdf::rect([point[0] - half[0], point[1] - half[1]], half, [radius; 4])[0] <= 0.0;
        inside.then_some(point)
    }

    pub fn gravity(&self) -> [f32; 2] {
        let m = self.linear();
        let g = [m[0][1], m[1][1]];
        let length = (g[0] * g[0] + g[1] * g[1]).sqrt();
        [g[0] / length, g[1] / length]
    }

    fn window_from_layout(&self) -> Matrix {
        let m = self.linear();
        let mut out = IDENTITY;
        out[0][0] = m[0][0];
        out[0][1] = m[0][1];
        out[1][0] = m[1][0];
        out[1][1] = m[1][1];
        multiply(
            translation(self.centre[0], self.centre[1]),
            multiply(
                out,
                translation(-self.layout[0] * 0.5, -self.layout[1] * 0.5),
            ),
        )
    }

    pub fn reflected(&self, transform: Matrix) -> Matrix {
        multiply(self.window_from_layout(), transform)
    }

    pub fn transform(&self, transform: Matrix) -> Matrix {
        let [fx, fy] = self.mirror.signs();
        let mut flip = IDENTITY;
        flip[0][0] = fx;
        flip[1][1] = fy;
        multiply(self.reflected(transform), flip)
    }

    pub fn place(&self, draw: &Draw) -> Draw {
        let mut out = *draw;
        out.transform = self.transform(draw.transform);
        out
    }

    pub fn place_all(&self, draws: &mut [Draw]) {
        for draw in draws {
            draw.transform = self.transform(draw.transform);
        }
    }

    pub fn frame_transform(&self) -> Matrix {
        multiply(
            translation(self.centre[0], self.centre[1]),
            super::rotation(self.angle),
        )
    }

    pub fn screen_bounds(&self) -> [f32; 4] {
        let mut out = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for corner in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]] {
            let p = self.to_window([corner[0] * self.layout[0], corner[1] * self.layout[1]]);
            out = [
                out[0].min(p[0]),
                out[1].min(p[1]),
                out[2].max(p[0]),
                out[3].max(p[1]),
            ];
        }
        out
    }

    pub fn frame_draws(&self, elevation: f32, backdrop: Option<Srgba>, reach: f32) -> Vec<Draw> {
        let screen = [self.layout[0] * 0.5, self.layout[1] * 0.5];
        let bezel = self.frame.bezel.max(0.0);
        let inner = self.frame.screen_radius().min(screen[0]).min(screen[1]);
        let at = self.frame_transform();
        let mut draws = Vec::with_capacity(2);
        if let Some(backdrop) = backdrop
            && reach > 0.0
        {
            let inset = (bezel * 0.5).min(1.0);
            let edge = bezel - inset;
            draws.push(
                Draw::new(Shape::Rect {
                    half: [
                        screen[0] + edge + reach * 0.5,
                        screen[1] + edge + reach * 0.5,
                    ],
                    radii: [inner + edge + reach * 0.5; 4],
                })
                .transform(at)
                .stroke(Stroke::solid(backdrop, reach))
                .elevation(elevation)
                .shadow(Shadow::None),
            );
        }
        if bezel > 0.0 {
            draws.push(
                Draw::new(Shape::Rect {
                    half: [screen[0] + bezel * 0.5, screen[1] + bezel * 0.5],
                    radii: [inner + bezel * 0.5; 4],
                })
                .transform(at)
                .stroke(Stroke::solid(self.frame.colour, bezel))
                .elevation(elevation)
                .shadow(Shadow::None),
            );
        }
        draws
    }
}
