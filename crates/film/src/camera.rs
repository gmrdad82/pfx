pub const WIDE: [f32; 2] = [1920.0, 1080.0];
use pfx_live::flat::{Matrix, View};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub focus: [f32; 2],
    pub zoom: f32,
    pub roll: f32,
    pub shake: [f32; 2],
    pub tilt: f32,
    pub safe: [f32; 2],
    pub wide: [f32; 2],
}

impl Default for Camera {
    fn default() -> Camera {
        Camera {
            focus: [WIDE[0] * 0.5, WIDE[1] * 0.5],
            zoom: 1.0,
            roll: 0.0,
            shake: [0.0; 2],
            tilt: 0.0,
            safe: [0.0; 2],
            wide: WIDE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Picture {
    pub cover: [f32; 2],
    pub shown: [f32; 2],
    pub fill: f32,
}

impl Default for Picture {
    fn default() -> Picture {
        Picture {
            cover: WIDE,
            shown: WIDE,
            fill: 1.0,
        }
    }
}

pub fn picture(bars: f32, out: [u32; 2]) -> Picture {
    picture_in(WIDE, bars, out)
}

pub fn picture_in(wide: [f32; 2], bars: f32, out: [u32; 2]) -> Picture {
    let size = [out[0].max(1) as f32, out[1].max(1) as f32];
    let aspect = size[0] / size[1];
    let cover = if aspect >= wide[0] / wide[1] {
        [wide[0], wide[0] / aspect]
    } else {
        [wide[1] * aspect, wide[1]]
    };
    let s = (size[0] / wide[0]).min(size[1] / wide[1]);
    let fill = (size[0] / s / cover[0]).max(1.0);
    let kept = if bars > 0.0 {
        (aspect / bars).clamp(0.0, 1.0)
    } else {
        1.0
    };
    Picture {
        cover,
        shown: [cover[0], cover[1] * kept],
        fill,
    }
}

pub fn layout(out: [u32; 2]) -> [f32; 2] {
    layout_in(WIDE, out)
}

pub fn layout_in(wide: [f32; 2], out: [u32; 2]) -> [f32; 2] {
    if out[1] > out[0] {
        [wide[1], wide[0]]
    } else {
        wide
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    [v[0] / n, v[1] / n, v[2] / n]
}

impl Camera {
    pub fn angle(&self) -> f32 {
        self.roll.to_radians() + self.tilt
    }

    pub fn reach(&self) -> f32 {
        let turn = self.angle().abs().min(0.5);
        let jolt = self.shake[0].abs().max(self.shake[1].abs());
        self.zoom.max(0.05) * (1.0 + 1.6 * turn.sin() + 2.0 * jolt / self.wide[1])
    }

    pub fn centre(&self, picture: Picture) -> [f32; 2] {
        let z = self.reach();
        let half = [picture.shown[0] * 0.5 / z, picture.shown[1] * 0.5 / z];
        let k = picture.cover[0].min(picture.cover[1]) / self.wide[1];
        let hold = |v: f32, h: f32, side: f32, past: [f32; 2]| {
            let (low, high) = (h - past[0], side - h + past[1]);
            if low >= high {
                side * 0.5
            } else {
                v.clamp(low, high)
            }
        };
        let tall = picture.shown[1] / z;
        [
            hold(
                self.focus[0] + self.shake[0] * k,
                half[0],
                self.wide[0],
                [0.0; 2],
            ),
            hold(
                self.focus[1] + self.shake[1] * k,
                half[1],
                self.wide[1],
                self.safe.map(|s| s * tall),
            ),
        ]
    }

    fn still(&self, picture: Picture) -> bool {
        let c = self.centre(picture);
        (self.reach() * picture.fill - 1.0).abs() < 1e-5
            && self.angle().abs() < 1e-6
            && (c[0] - self.wide[0] * 0.5).abs() < 1e-3
            && (c[1] - self.wide[1] * 0.5).abs() < 1e-3
    }

    pub fn view(&self, picture: Picture) -> Option<Matrix> {
        if self.still(picture) {
            return None;
        }
        let c = self.centre(picture);
        let mid = [self.wide[0] * 0.5, self.wide[1] * 0.5];
        let view = View::about(c)
            .zoom(self.reach() * picture.fill)
            .roll(-self.angle())
            .pan([mid[0] - c[0], mid[1] - c[1]]);
        Some(view.matrix())
    }

    pub fn lens(
        &self,
        picture: Picture,
        at: [f32; 3],
        look: [f32; 3],
        fov: f32,
    ) -> ([f32; 3], f32) {
        if self.still(Picture {
            fill: 1.0,
            ..picture
        }) {
            return (look, fov);
        }
        let c = self.centre(picture);
        let forward = unit([look[0] - at[0], look[1] - at[1], look[2] - at[2]]);
        let right = unit(cross(forward, [0.0, 1.0, 0.0]));
        let up = cross(right, forward);
        let half = (fov.to_radians() * 0.5).tan();
        let aspect = self.wide[0] / self.wide[1];
        let x = (c[0] / self.wide[0] * 2.0 - 1.0) * half * aspect;
        let y = (1.0 - c[1] / self.wide[1] * 2.0) * half;
        let aim = std::array::from_fn(|i| at[i] + forward[i] + right[i] * x + up[i] * y);
        let narrow = 2.0 * (half / self.reach()).atan();
        (aim, narrow.to_degrees())
    }
}
