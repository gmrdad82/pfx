use serde::{Deserialize, Serialize};

use crate::Sky;
use crate::vec3::{cross, dot};

const MAX_WIDTH: u32 = 8192;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoomSky {
    #[serde(default = "room_width")]
    pub width: u32,
    #[serde(default = "room_floor")]
    pub floor: [f32; 3],
    #[serde(default = "room_wall")]
    pub wall: [f32; 3],
    #[serde(default = "room_ceiling")]
    pub ceiling: [f32; 3],
    #[serde(default)]
    pub lights: Vec<RoomLamp>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoomLamp {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default)]
    pub az: f32,
    #[serde(default = "lamp_el")]
    pub el: f32,
    #[serde(default = "lamp_power")]
    pub power: f32,
    #[serde(default = "lamp_color")]
    pub color: [f32; 3],
    #[serde(default = "lamp_width")]
    pub width: f32,
    #[serde(default = "lamp_height")]
    pub height: f32,
    #[serde(default = "lamp_soft")]
    pub soft: f32,
    #[serde(default)]
    pub slats: f32,
    #[serde(default = "lamp_open")]
    pub open: f32,
}

impl Default for RoomSky {
    fn default() -> Self {
        Self {
            width: room_width(),
            floor: room_floor(),
            wall: room_wall(),
            ceiling: room_ceiling(),
            lights: Vec::new(),
        }
    }
}

impl Default for RoomLamp {
    fn default() -> Self {
        Self {
            name: String::new(),
            az: 0.0,
            el: lamp_el(),
            power: lamp_power(),
            color: lamp_color(),
            width: lamp_width(),
            height: lamp_height(),
            soft: lamp_soft(),
            slats: 0.0,
            open: lamp_open(),
        }
    }
}

fn room_width() -> u32 {
    64
}
fn room_floor() -> [f32; 3] {
    [0.5, 0.5, 0.5]
}
fn room_wall() -> [f32; 3] {
    [0.5, 0.5, 0.5]
}
fn room_ceiling() -> [f32; 3] {
    [0.5, 0.5, 0.5]
}
fn lamp_el() -> f32 {
    0.0
}
fn lamp_power() -> f32 {
    1.0
}
fn lamp_color() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}
fn lamp_width() -> f32 {
    45.0
}
fn lamp_height() -> f32 {
    45.0
}
fn lamp_soft() -> f32 {
    0.1
}
fn lamp_open() -> f32 {
    1.0
}

impl RoomSky {
    pub fn check(&self) -> Result<(), String> {
        if !(2..=MAX_WIDTH).contains(&self.width) {
            return Err(format!(
                "room width is {}, it must be 2 to {MAX_WIDTH}",
                self.width
            ));
        }
        let colour = |c: &[f32; 3]| c.iter().all(|v| v.is_finite() && *v >= 0.0);
        if !colour(&self.floor) || !colour(&self.wall) || !colour(&self.ceiling) {
            return Err("room floor, wall and ceiling must be finite and nonnegative".into());
        }
        for lamp in &self.lights {
            if !colour(&lamp.color)
                || [lamp.az, lamp.el, lamp.open].iter().any(|v| !v.is_finite())
                || [lamp.power, lamp.soft, lamp.slats]
                    .iter()
                    .any(|v| !v.is_finite() || *v < 0.0)
                || [lamp.width, lamp.height]
                    .iter()
                    .any(|v| !v.is_finite() || *v <= 0.0 || *v >= 180.0)
            {
                return Err(format!(
                    "room lamp {:?} needs finite values, nonnegative power, colour, soft and slats, and a width and height between 0 and 180 degrees",
                    lamp.name
                ));
            }
        }
        Ok(())
    }
}

struct Lamp {
    center: [f32; 3],
    t1: [f32; 3],
    t2: [f32; 3],
    half: [f32; 2],
    soft: f32,
    color: [f32; 3],
    slats: f32,
    open: f32,
}

impl Lamp {
    fn new(lamp: &RoomLamp) -> Self {
        let az = lamp.az.to_radians();
        let el = lamp.el.to_radians();
        let center = [az.sin() * el.cos(), el.sin(), az.cos() * el.cos()];
        let t1 = norm(cross([0.0, 1.0, 0.0], center));
        let t2 = cross(center, t1);
        Self {
            center,
            t1,
            t2,
            half: [
                (lamp.width.to_radians() * 0.5).tan(),
                (lamp.height.to_radians() * 0.5).tan(),
            ],
            soft: lamp.soft,
            color: lamp.color.map(|x| x * lamp.power),
            slats: lamp.slats,
            open: lamp.open,
        }
    }

    fn weight(&self, d: [f32; 3]) -> f32 {
        let along = dot(d, self.center);
        if along <= 0.05 {
            return 0.0;
        }
        let u = dot(d, self.t1) / along / self.half[0];
        let v = dot(d, self.t2) / along / self.half[1];
        let edge = |q: f32| ((1.0 - q.abs()) / self.soft.max(1e-3)).clamp(0.0, 1.0);
        let mut w = edge(u) * edge(v);
        if self.slats > 0.0 && w > 0.0 {
            let s = ((v * 0.5 + 0.5) * self.slats).fract();
            w *= ((self.open - s) / 0.04).clamp(0.0, 1.0) * (s / 0.04).clamp(0.0, 1.0);
        }
        w
    }
}

impl Sky {
    pub fn room(room: &RoomSky) -> Sky {
        let width = room.width;
        let height = width / 2;
        let lamps: Vec<Lamp> = room.lights.iter().map(Lamp::new).collect();
        let mut texels = Vec::with_capacity(width as usize * height as usize);
        for y in 0..height {
            for x in 0..width {
                let theta = (y as f32 + 0.5) / height as f32 * std::f32::consts::PI;
                let phi = ((x as f32 + 0.5) / width as f32 - 0.5) * std::f32::consts::TAU;
                let d = [
                    theta.sin() * phi.sin(),
                    theta.cos(),
                    -theta.sin() * phi.cos(),
                ];
                let mut c = if d[1] < 0.0 {
                    mix(room.wall, room.floor, (-d[1]).powf(0.5))
                } else {
                    mix(room.wall, room.ceiling, d[1].powf(0.6))
                };
                for lamp in &lamps {
                    let w = lamp.weight(d);
                    for (ck, lk) in c.iter_mut().zip(lamp.color) {
                        *ck += lk * w;
                    }
                }
                texels.push([c[0], c[1], c[2], 1.0]);
            }
        }
        Sky {
            width,
            height,
            texels,
        }
    }
}

fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-9);
    [a[0] / l, a[1] / l, a[2] / l]
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn reference_room(spec: &Value) -> (u32, u32, Vec<[f32; 4]>) {
        let num = |v: &Value, d: f32| v.as_f64().map(|x| x as f32).unwrap_or(d);
        let rgb = |v: &Value, d: [f32; 3]| match v.as_array() {
            Some(a) if a.len() == 3 => [num(&a[0], d[0]), num(&a[1], d[1]), num(&a[2], d[2])],
            _ => d,
        };
        let width = num(&spec["width"], 64.0) as u32;
        let height = width / 2;
        let floor = rgb(&spec["floor"], [0.5, 0.5, 0.5]);
        let wall = rgb(&spec["wall"], [0.5, 0.5, 0.5]);
        let ceiling = rgb(&spec["ceiling"], [0.5, 0.5, 0.5]);
        struct Lamp {
            center: [f32; 3],
            t1: [f32; 3],
            t2: [f32; 3],
            half: [f32; 2],
            soft: f32,
            color: [f32; 3],
            slats: f32,
            open: f32,
        }
        let lamps: Vec<Lamp> = spec["lights"]
            .as_array()
            .map(|list| {
                list.iter()
                    .map(|l| {
                        let az = num(&l["az"], 0.0).to_radians();
                        let el = num(&l["el"], 0.0).to_radians();
                        let c = [az.sin() * el.cos(), el.sin(), az.cos() * el.cos()];
                        let up = [0.0, 1.0, 0.0];
                        let t1 = norm(cross(up, c));
                        let t2 = cross(c, t1);
                        let power = num(&l["power"], 1.0);
                        let tint = rgb(&l["color"], [1.0, 1.0, 1.0]);
                        Lamp {
                            center: c,
                            t1,
                            t2,
                            half: [
                                (num(&l["width"], 45.0).to_radians() * 0.5).tan(),
                                (num(&l["height"], 45.0).to_radians() * 0.5).tan(),
                            ],
                            soft: num(&l["soft"], 0.1),
                            color: tint.map(|x| x * power),
                            slats: num(&l["slats"], 0.0),
                            open: num(&l["open"], 1.0),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut texels = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                let theta = (y as f32 + 0.5) / height as f32 * std::f32::consts::PI;
                let phi = ((x as f32 + 0.5) / width as f32 - 0.5) * std::f32::consts::TAU;
                let d = [
                    theta.sin() * phi.sin(),
                    theta.cos(),
                    -theta.sin() * phi.cos(),
                ];
                let mut c = if d[1] < 0.0 {
                    let t = (-d[1]).powf(0.5);
                    mix(wall, floor, t)
                } else {
                    let t = d[1].powf(0.6);
                    mix(wall, ceiling, t)
                };
                for lamp in &lamps {
                    let along = dot(d, lamp.center);
                    if along <= 0.05 {
                        continue;
                    }
                    let u = dot(d, lamp.t1) / along / lamp.half[0];
                    let v = dot(d, lamp.t2) / along / lamp.half[1];
                    let edge = |q: f32| ((1.0 - q.abs()) / lamp.soft.max(1e-3)).clamp(0.0, 1.0);
                    let mut w = edge(u) * edge(v);
                    if lamp.slats > 0.0 && w > 0.0 {
                        let s = ((v * 0.5 + 0.5) * lamp.slats).fract();
                        w *= ((lamp.open - s) / 0.04).clamp(0.0, 1.0)
                            * ((s - 0.0) / 0.04).clamp(0.0, 1.0).max(0.0);
                    }
                    for (ck, lk) in c.iter_mut().zip(lamp.color) {
                        *ck += lk * w;
                    }
                }
                texels.push([c[0], c[1], c[2], 1.0]);
            }
        }
        (width, height, texels)
    }

    fn made_up() -> Value {
        json!({
            "width": 32,
            "floor": [0.2, 0.3, 0.4],
            "wall": [0.45, 0.5, 0.55],
            "ceiling": [0.7, 0.75, 0.8],
            "lights": [
                {"name": "north", "az": 15.0, "el": 25.0, "power": 2.0, "color": [0.9, 0.8, 0.7],
                 "width": 20.0, "height": 12.0, "soft": 0.2, "slats": 5.0, "open": 0.6}
            ]
        })
    }

    fn every_field() -> Value {
        json!({
            "width": 512,
            "floor": [0.2, 0.12, 0.05],
            "wall": [0.25, 0.2, 0.15],
            "ceiling": [0.6, 0.5, 0.4],
            "lights": [
                {"name": "blinds", "az": -70.0, "el": 18.0, "power": 9.0, "color": [1.0, 0.88, 0.7],
                 "width": 40.0, "height": 34.0, "soft": 0.05, "slats": 9.0, "open": 0.62},
                {"az": 120.0, "el": 55.0, "power": 2.5, "color": [0.8, 0.9, 1.0],
                 "width": 25.0, "height": 60.0, "soft": 0.2, "slats": 3.5, "open": 0.4},
                {"az": 200.0, "el": -10.0, "power": 0.7, "color": [1.0, 1.0, 1.0],
                 "width": 120.0, "height": 15.0, "soft": 0.0, "slats": 0.0, "open": 1.0}
            ]
        })
    }

    fn matches(spec: Value) {
        let (width, height, want) = reference_room(&spec);
        let room: RoomSky = serde_json::from_value(spec).unwrap();
        room.check().unwrap();
        let sky = Sky::room(&room);
        assert_eq!((sky.width, sky.height), (width, height));
        assert_eq!(sky.texels.len(), want.len());
        for (got, want) in sky.texels.iter().zip(&want) {
            for k in 0..4 {
                assert!((got[k] - want[k]).abs() <= 1e-6, "{got:?} against {want:?}");
            }
        }
    }

    #[test]
    fn the_room_matches_a_reference_texel_for_texel() {
        matches(made_up());
        matches(json!({}));
        matches(every_field());
    }

    #[test]
    fn a_spec_deserializes_with_its_names_and_neutral_defaults() {
        let room: RoomSky = serde_json::from_value(made_up()).unwrap();
        assert_eq!(room.width, 32);
        assert_eq!(room.lights.len(), 1);
        assert_eq!(room.lights[0].name, "north");
        assert_eq!(room.lights[0].slats, 5.0);
        assert_eq!(room.lights[0].open, 0.6);
        let empty: RoomSky = serde_json::from_value(json!({})).unwrap();
        assert_eq!(empty, RoomSky::default());
        let lamp: RoomLamp = serde_json::from_value(json!({})).unwrap();
        assert_eq!(lamp, RoomLamp::default());
        assert!(serde_json::from_value::<RoomSky>(json!({"lamps": []})).is_err());
    }

    #[test]
    fn the_slats_cut_dark_bands_through_a_lamp() {
        let lamp = RoomLamp {
            az: 0.0,
            el: 0.0,
            width: 40.0,
            height: 40.0,
            soft: 0.01,
            slats: 4.0,
            open: 0.5,
            ..RoomLamp::default()
        };
        let lit = Lamp::new(&RoomLamp {
            slats: 0.0,
            ..lamp.clone()
        });
        let slatted = Lamp::new(&lamp);
        let mut dark = 0;
        let mut bright = 0;
        for step in 0..64 {
            let v = (step as f32 + 0.5) / 64.0 * 1.6 - 0.8;
            let d = norm([0.0, v * slatted.half[1], 1.0]);
            assert!((lit.weight(d) - 1.0).abs() < 1e-6);
            let w = slatted.weight(d);
            if w < 1e-6 {
                dark += 1;
            } else if w > 0.999 {
                bright += 1;
            }
        }
        assert!(dark > 16 && bright > 16, "{dark} dark and {bright} bright");
    }

    #[test]
    fn checking_refuses_a_broken_room() {
        assert!(RoomSky::default().check().is_ok());
        let narrow = RoomSky {
            width: 1,
            ..RoomSky::default()
        };
        assert!(narrow.check().is_err());
        let dark = RoomSky {
            floor: [-0.1, 0.0, 0.0],
            ..RoomSky::default()
        };
        assert!(dark.check().is_err());
        let wide = RoomSky {
            lights: vec![RoomLamp {
                width: 180.0,
                ..RoomLamp::default()
            }],
            ..RoomSky::default()
        };
        assert!(wide.check().is_err());
        let nan = RoomSky {
            lights: vec![RoomLamp {
                power: f32::NAN,
                ..RoomLamp::default()
            }],
            ..RoomSky::default()
        };
        assert!(nan.check().is_err());
    }
}
