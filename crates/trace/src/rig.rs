use pfx_load::{RoomLamp, RoomSky, Sky};

use crate::lights::LocalLight;
use crate::math::{cross, dot, length, normalize, scale, sub};

pub const ROOM_MEAN: f32 = 0.80;
pub const ROOM_WIDTH: u32 = 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lamp {
    pub az: f32,
    pub el: f32,
    pub width: f32,
    pub height: f32,
    pub power: f32,
    pub colour: [f32; 3],
}

pub const ROOM_LAMPS: [Lamp; 4] = [
    Lamp {
        az: 133.8,
        el: 27.8,
        width: 4.0,
        height: 4.0,
        power: 420.0,
        colour: [0.904, 1.012, 1.163],
    },
    Lamp {
        az: -1.1,
        el: 22.7,
        width: 15.5,
        height: 1.5,
        power: 147.0,
        colour: [0.719, 1.087, 0.97],
    },
    Lamp {
        az: 21.4,
        el: 4.7,
        width: 6.0,
        height: 6.0,
        power: 386.0,
        colour: [0.874, 1.02, 1.173],
    },
    Lamp {
        az: -146.4,
        el: 1.5,
        width: 5.0,
        height: 5.0,
        power: 502.0,
        colour: [0.881, 1.016, 1.193],
    },
];

pub const ROOM_FLOOR: [f32; 3] = [0.103, 0.131, 0.135];
pub const ROOM_WALL: [f32; 3] = [0.020, 0.025, 0.026];
pub const ROOM_CEILING: [f32; 3] = [0.0012, 0.0012, 0.0019];

pub fn softbox_room(turn_deg: f32) -> RoomSky {
    RoomSky {
        width: ROOM_WIDTH,
        floor: ROOM_FLOOR,
        wall: ROOM_WALL,
        ceiling: ROOM_CEILING,
        lights: ROOM_LAMPS
            .iter()
            .map(|lamp| RoomLamp {
                name: String::new(),
                az: lamp.az - turn_deg,
                el: lamp.el,
                power: lamp.power,
                color: lamp.colour,
                width: lamp.width,
                height: lamp.height,
                soft: 0.18,
                slats: 0.0,
                open: 1.0,
            })
            .collect(),
    }
}

pub fn room(turn_deg: f32, strength: f32) -> Sky {
    let sky = Sky::room(&softbox_room(turn_deg));
    let mean = solid_angle_mean(&sky);
    sky.exposed(ROOM_MEAN / mean.max(1e-6) * strength)
}

pub fn solid_angle_mean(sky: &Sky) -> f32 {
    let (mut total, mut weight) = (0.0f64, 0.0f64);
    for (y, row) in sky.texels.chunks(sky.width as usize).enumerate() {
        let theta = (y as f64 + 0.5) / f64::from(sky.height) * std::f64::consts::PI;
        let w = theta.sin();
        for texel in row {
            total += w * f64::from(0.2126 * texel[0] + 0.7152 * texel[1] + 0.0722 * texel[2]);
            weight += w;
        }
    }
    (total / weight.max(1e-12)) as f32
}

pub fn engine_axes(blender: [f32; 3]) -> [f32; 3] {
    [blender[0], blender[2], -blender[1]]
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rig {
    pub key: f32,
    pub fill: f32,
    pub rim: f32,
    pub key_from: [f32; 3],
    pub key_color: Option<[f32; 3]>,
    pub rim_color: Vec<[f32; 3]>,
    pub strength: f32,
    pub turn: f32,
}

impl Default for Rig {
    fn default() -> Self {
        Self {
            key: 1.0,
            fill: 0.35,
            rim: 0.8,
            key_from: [-2.2, -2.6, 3.4],
            key_color: None,
            rim_color: Vec::new(),
            strength: 0.6,
            turn: 25.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Softbox {
    pub at: [f32; 3],
    pub size: f32,
    pub power: f32,
    pub colour: [f32; 3],
}

pub const RIG_TARGET: [f32; 3] = [0.0, 0.0, 0.1];

impl Rig {
    pub fn boxes(&self) -> Vec<Softbox> {
        let white = [1.0; 3];
        let rim = |k: usize| {
            self.rim_color
                .get(k.min(self.rim_color.len().saturating_sub(1)))
                .copied()
                .unwrap_or(white)
        };
        let mut boxes = vec![
            Softbox {
                at: self.key_from,
                size: 3.0,
                power: self.key,
                colour: self.key_color.unwrap_or(white),
            },
            Softbox {
                at: [3.0, -1.2, 2.2],
                size: 2.4,
                power: self.fill,
                colour: white,
            },
            Softbox {
                at: [0.6, 3.4, 2.6],
                size: 2.0,
                power: self.rim,
                colour: rim(0),
            },
        ];
        if self.rim_color.len() > 1 {
            boxes.push(Softbox {
                at: [-0.8, 3.2, 2.4],
                size: 2.0,
                power: self.rim * 0.8,
                colour: rim(1),
            });
        }
        boxes
    }

    pub fn lights(&self) -> Vec<LocalLight> {
        self.boxes().iter().map(|softbox| softbox.light()).collect()
    }

    pub fn sky(&self) -> Sky {
        room(self.turn, self.strength)
    }
}

impl Softbox {
    pub fn light(&self) -> LocalLight {
        let centre = engine_axes(self.at);
        let aim = normalize(sub(engine_axes(RIG_TARGET), centre));
        let back = scale(aim, -1.0);
        let up = [0.0, 1.0, 0.0];
        let along = sub(up, scale(back, dot(up, back)));
        let y = if length(along) > 1e-6 {
            normalize(along)
        } else {
            normalize(cross(back, [1.0, 0.0, 0.0]))
        };
        let x = cross(y, back);
        let distance2 = dot(self.at, self.at);
        LocalLight::rect(
            centre,
            scale(y, self.size * 0.3),
            scale(x, self.size * 0.5),
            self.colour,
            self.power * distance2,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lights::LightShape;

    #[test]
    fn the_room_has_the_mean_it_documents() {
        let sky = room(25.0, 1.0);
        assert_eq!((sky.width, sky.height), (ROOM_WIDTH, ROOM_WIDTH / 2));
        assert!((solid_angle_mean(&sky) - ROOM_MEAN).abs() < 1e-3);
        let dim = room(0.0, 0.5);
        assert!((solid_angle_mean(&dim) - ROOM_MEAN * 0.5).abs() < 1e-3);
        softbox_room(25.0).check().unwrap();
        let w = sky.width as usize;
        let row = |y: usize| {
            sky.texels[y * w..(y + 1) * w]
                .iter()
                .map(|t| t[0])
                .sum::<f32>()
                / w as f32
        };
        let (floor, ceiling) = (row(sky.height as usize - 1), row(0));
        let peak = sky.texels.iter().map(|t| t[0]).fold(0.0, f32::max);
        assert!(
            floor > 10.0 * ceiling && peak > 40.0 * floor,
            "floor {floor} ceiling {ceiling} peak {peak}"
        );
        assert!(floor < 0.5, "floor {floor}");
    }

    #[test]
    fn the_room_turns_its_lamps() {
        let still = room(0.0, 1.0);
        let turned = room(90.0, 1.0);
        let w = still.width as usize;
        let row = (still.height as usize / 2) - 14;
        let brightest = |sky: &Sky| {
            (0..w)
                .max_by(|&a, &b| sky.texels[row * w + a][0].total_cmp(&sky.texels[row * w + b][0]))
                .unwrap()
        };
        let shift = (brightest(&turned) as i64 - brightest(&still) as i64).rem_euclid(w as i64);
        assert!((shift - w as i64 / 4).abs() <= 2, "{shift}");
    }

    #[test]
    fn softboxes_face_the_target_with_blenders_power() {
        let rig = Rig::default();
        let lights = rig.lights();
        assert_eq!(lights.len(), 3);
        let key = lights[0];
        assert!(key.valid());
        assert_eq!(key.position, [-2.2, 3.4, 2.6]);
        let LightShape::Rect {
            half_u,
            half_v,
            two_sided,
        } = key.shape
        else {
            panic!("the key is a rect");
        };
        assert!(!two_sided);
        let facing = normalize(cross(half_u, half_v));
        let aim = normalize(sub(engine_axes(RIG_TARGET), key.position));
        assert!(dot(facing, aim) > 0.9999, "{facing:?} {aim:?}");
        assert!((length(half_u) - 0.9).abs() < 1e-5 && (length(half_v) - 1.5).abs() < 1e-5);
        assert!(half_u[1] > 0.0);
        let distance2 = 2.2f32 * 2.2 + 2.6 * 2.6 + 3.4 * 3.4;
        assert!((key.intensity - distance2).abs() < 1e-4);
        assert!((key.area() - 3.0 * 1.8).abs() < 1e-4);
        let colours = Rig {
            rim_color: vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            ..Rig::default()
        };
        let lights = colours.lights();
        assert_eq!(lights.len(), 4);
        assert_eq!(lights[2].colour, [1.0, 0.0, 0.0]);
        assert_eq!(lights[3].colour, [0.0, 0.0, 1.0]);
        assert!((lights[3].intensity - 0.64 * (0.64 + 3.2 * 3.2 + 2.4 * 2.4)).abs() < 1e-4);
    }
}
