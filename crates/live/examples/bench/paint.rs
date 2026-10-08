use pfx_live::content_atlas::{AtlasImage, ContentAtlas};
use pfx_live::frame::InstanceSurface;
use pfx_live::maps::ContentFormat;
use pfx_live::renderer::Renderer;

use super::room::{self, Room};
use super::{HOUR, Lighting, dot, unit};

pub const ATLAS_SLOT: usize = 0;
pub const PHOTO_SLOT: usize = 1;
pub const VIDEO_SLOT: usize = 2;
pub const CAUSTIC_SLOT: usize = 3;
pub const ATLAS: u32 = 2048;
pub const CARD: (u32, u32) = (96, 72);
pub const CARDS: u32 = 216;
pub const PHOTO: (u32, u32) = (1024, 768);
pub const VIDEO: (u32, u32) = (960, 540);
pub const VIDEO_FRAMES: usize = 90;
pub const CAUSTIC: u32 = 256;
pub const CAUSTIC_SIDE: f32 = 0.5;
const INK: (u32, u32) = (512, 368);
const DECAL: u32 = 256;
const RAYS: u32 = 1024;

pub struct Painted {
    pub atlas: ContentAtlas,
    pub cards: Vec<usize>,
    pub ink: usize,
    pub decal: usize,
    pub photo: Vec<u8>,
    pub video: Vec<Vec<u8>>,
    pub caustic: Vec<f32>,
    pub caustic_rect: [f32; 4],
}

fn srgb(value: f32) -> u8 {
    let v = value.clamp(0.0, 1.0);
    let encoded = pfx_materials::encode_channel(v);
    (encoded * 255.0).round() as u8
}

fn pixel(rgb: [f32; 3], alpha: f32) -> [u8; 4] {
    [
        srgb(rgb[0]),
        srgb(rgb[1]),
        srgb(rgb[2]),
        (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

fn smooth(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

pub fn card(index: u32) -> Vec<u8> {
    let (width, height) = CARD;
    let hue = room::hash(index * 7 + 1);
    let ground = [
        0.25 + 0.6 * hue,
        0.3 + 0.5 * room::hash(index * 7 + 2),
        0.25 + 0.6 * (1.0 - hue),
    ];
    let bars = 2 + index % 4;
    let centre = [
        0.3 + 0.4 * room::hash(index * 7 + 3),
        0.35 + 0.3 * room::hash(index * 7 + 4),
    ];
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let u = (x as f32 + 0.5) / width as f32;
            let v = (y as f32 + 0.5) / height as f32;
            let mut rgb = mix(ground, [0.95, 0.94, 0.9], 0.6 * v);
            let d = ((u - centre[0]).powi(2) + (v - centre[1]).powi(2)).sqrt();
            if d < 0.18 {
                rgb = mix(rgb, [0.08, 0.07, 0.1], smooth(0.18, 0.16, d));
            }
            let band = (v * bars as f32 * 3.0).fract();
            if v > 0.72 && band < 0.4 && u > 0.08 && u < 0.08 + 0.8 * room::hash(index + y) {
                rgb = [0.1, 0.1, 0.12];
            }
            out.extend(pixel(rgb, 1.0));
        }
    }
    out
}

fn ink() -> Vec<u8> {
    let (width, height) = INK;
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let u = x as f32 / width as f32;
            let v = y as f32 / height as f32;
            let line = (v * 12.0).floor();
            let within = (v * 12.0).fract();
            let length = 0.55 + 0.35 * room::hash(line as u32 + 9);
            let stroke = 0.5 + 0.18 * (u * 140.0 + line * 1.7).sin() * (u * 23.0 + line).cos();
            let distance = (within - stroke).abs();
            let word = ((u * 9.0 + room::hash(line as u32) * 3.0).fract() < 0.82) as u8 as f32;
            let coverage = if u > 0.06 && u < length && line > 0.0 && line < 11.0 {
                smooth(0.07, 0.03, distance) * word
            } else {
                0.0
            };
            out.extend(pixel([0.03, 0.03, 0.06], coverage));
        }
    }
    out
}

fn decal() -> Vec<u8> {
    let mut out = Vec::with_capacity((DECAL * DECAL * 4) as usize);
    for y in 0..DECAL {
        for x in 0..DECAL {
            let u = (x as f32 + 0.5) / DECAL as f32 - 0.5;
            let v = (y as f32 + 0.5) / DECAL as f32 - 0.5;
            let r = (u * u + v * v).sqrt();
            let angle = v.atan2(u);
            let star = 0.22 + 0.1 * (angle * 5.0).cos();
            let ring = smooth(0.012, 0.004, (r - 0.42).abs());
            let fill = smooth(star + 0.005, star - 0.005, r);
            let alpha = ring.max(fill);
            let rgb = if fill > ring {
                [0.75, 0.18, 0.05]
            } else {
                [0.05, 0.08, 0.2]
            };
            out.extend(pixel(rgb, alpha));
        }
    }
    out
}

pub fn photo() -> Vec<u8> {
    let (width, height) = PHOTO;
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let u = (x as f32 + 0.5) / width as f32;
            let v = (y as f32 + 0.5) / height as f32;
            let rgb = if v < 0.55 {
                let sun = ((u - 0.7).powi(2) + (v - 0.25).powi(2)).sqrt();
                let sky = mix([0.25, 0.45, 0.85], [0.9, 0.8, 0.65], v / 0.55);
                mix(sky, [1.0, 0.95, 0.8], smooth(0.08, 0.05, sun))
            } else {
                let depth = (v - 0.55) / 0.45;
                let z = 1.0 / depth.max(0.02);
                let gx = ((u - 0.5) * z * 4.0).floor() as i32;
                let gz = (z * 2.0).floor() as i32;
                let check = (gx + gz).rem_euclid(2) as f32;
                let fade = smooth(0.0, 0.3, depth);
                mix(
                    [0.55, 0.55, 0.5],
                    mix([0.85, 0.82, 0.75], [0.2, 0.25, 0.18], check),
                    fade,
                )
            };
            out.extend(pixel(rgb, 1.0));
        }
    }
    out
}

pub fn video_frame(index: usize) -> Vec<u8> {
    let (width, height) = VIDEO;
    let t = index as f32 / VIDEO_FRAMES as f32;
    let colours = [
        [0.75, 0.75, 0.75],
        [0.75, 0.75, 0.0],
        [0.0, 0.75, 0.75],
        [0.0, 0.75, 0.0],
        [0.75, 0.0, 0.75],
        [0.75, 0.0, 0.0],
        [0.0, 0.0, 0.75],
    ];
    let ball = [
        0.5 + 0.35 * (t * std::f32::consts::TAU).cos(),
        0.55 + 0.25 * (t * std::f32::consts::TAU * 2.0).sin(),
    ];
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let u = (x as f32 + 0.5) / width as f32;
            let v = (y as f32 + 0.5) / height as f32;
            let shifted = (u + t).fract();
            let mut rgb = if v < 0.3 {
                colours[((shifted * 7.0) as usize).min(6)]
            } else {
                mix([0.05, 0.05, 0.08], [0.15, 0.2, 0.3], v)
            };
            let d = (((u - ball[0]) * 16.0 / 9.0).powi(2) + (v - ball[1]).powi(2)).sqrt();
            if d < 0.1 {
                rgb = mix(rgb, [1.0, 0.8, 0.3], smooth(0.1, 0.09, d));
            }
            let block = (u * 30.0) as usize;
            if v > 0.9 && block < 30 && (index >> (block % 7)) & 1 == 1 {
                rgb = [0.9, 0.9, 0.9];
            }
            out.extend(pixel(rgb, 1.0));
        }
    }
    out
}

fn sphere_hit(
    origin: [f32; 3],
    direction: [f32; 3],
    centre: [f32; 3],
    radius: f32,
    far: bool,
) -> Option<f32> {
    let o = std::array::from_fn(|k| origin[k] - centre[k]);
    let b = dot(o, direction);
    let c = dot(o, o) - radius * radius;
    let h = b * b - c;
    if h < 0.0 {
        return None;
    }
    let t = if far { -b + h.sqrt() } else { -b - h.sqrt() };
    (t > 1e-6).then_some(t)
}

fn refract(direction: [f32; 3], normal: [f32; 3], eta: f32) -> Option<[f32; 3]> {
    let cos_i = -dot(direction, normal);
    let k = 1.0 - eta * eta * (1.0 - cos_i * cos_i);
    (k >= 0.0)
        .then(|| std::array::from_fn(|i| eta * direction[i] + (eta * cos_i - k.sqrt()) * normal[i]))
}

fn fresnel(cos: f32, ior: f32) -> f32 {
    let f0 = ((ior - 1.0) / (ior + 1.0)).powi(2);
    f0 + (1.0 - f0) * (1.0 - cos.abs()).powi(5)
}

pub fn caustic_rect(toward: [f32; 3]) -> [f32; 4] {
    let toward = unit(toward);
    let along = (room::LENS[1] - room::TABLE_TOP) / toward[1].max(1e-3);
    let centre = [
        room::LENS[0] - toward[0] * along,
        room::LENS[2] - toward[2] * along,
    ];
    [
        centre[0] - CAUSTIC_SIDE * 0.5,
        centre[1] - CAUSTIC_SIDE * 0.5,
        CAUSTIC_SIDE,
        CAUSTIC_SIDE,
    ]
}

pub fn caustic(toward: [f32; 3]) -> (Vec<f32>, [f32; 4]) {
    let toward = unit(toward);
    let down = toward.map(|v| -v);
    let rect = caustic_rect(toward);
    let side = CAUSTIC as usize;
    let mut energy = vec![0.0f32; side * side];
    let radius = room::LENS_RADIUS;
    let u = unit(super::cross(down, [0.0, 1.0, 0.0]));
    let v = super::cross(u, down);
    let step = 2.0 * radius / RAYS as f32;
    let ior = 1.5;
    let texel_area = (rect[2] / CAUSTIC as f32) * (rect[3] / CAUSTIC as f32);
    let unblocked = down[1].abs();
    for j in 0..RAYS {
        for i in 0..RAYS {
            let a = -radius + (i as f32 + 0.5) * step;
            let b = -radius + (j as f32 + 0.5) * step;
            if a * a + b * b >= radius * radius {
                continue;
            }
            let origin: [f32; 3] = std::array::from_fn(|k| {
                room::LENS[k] + toward[k] * 2.0 * radius + u[k] * a + v[k] * b
            });
            let Some(t) = sphere_hit(origin, down, room::LENS, radius, false) else {
                continue;
            };
            let hit: [f32; 3] = std::array::from_fn(|k| origin[k] + down[k] * t);
            let normal = unit(std::array::from_fn(|k| hit[k] - room::LENS[k]));
            let enter = 1.0 - fresnel(dot(down, normal), ior);
            let Some(inside) = refract(down, normal, 1.0 / ior) else {
                continue;
            };
            let Some(t) = sphere_hit(hit, inside, room::LENS, radius, true) else {
                continue;
            };
            let out_hit: [f32; 3] = std::array::from_fn(|k| hit[k] + inside[k] * t);
            let normal = unit(std::array::from_fn(|k| room::LENS[k] - out_hit[k]));
            let leave = 1.0 - fresnel(dot(inside, normal), 1.0 / ior);
            let Some(out) = refract(inside, normal, ior) else {
                continue;
            };
            if out[1] >= -1e-4 {
                continue;
            }
            let along = (room::TABLE_TOP - out_hit[1]) / out[1];
            if along < 0.0 {
                continue;
            }
            let x = out_hit[0] + out[0] * along;
            let z = out_hit[2] + out[2] * along;
            let px = ((x - rect[0]) / rect[2] * CAUSTIC as f32).floor();
            let pz = ((z - rect[1]) / rect[3] * CAUSTIC as f32).floor();
            if px < 0.0 || pz < 0.0 || px >= CAUSTIC as f32 || pz >= CAUSTIC as f32 {
                continue;
            }
            energy[pz as usize * side + px as usize] += step * step * enter * leave;
        }
    }
    let mut gain = vec![0.0f32; side * side];
    for y in 0..side {
        for x in 0..side {
            let mut sum = 0.0;
            let mut weight = 0.0;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (sx, sy) = (x as i32 + dx, y as i32 + dy);
                    if sx < 0 || sy < 0 || sx >= side as i32 || sy >= side as i32 {
                        continue;
                    }
                    let w = if dx == 0 && dy == 0 { 4.0 } else { 1.0 };
                    sum += energy[sy as usize * side + sx as usize] * w;
                    weight += w;
                }
            }
            let refracted = sum / weight / texel_area / unblocked;
            let centre = [
                rect[0] + (x as f32 + 0.5) / CAUSTIC as f32 * rect[2],
                room::TABLE_TOP,
                rect[1] + (y as f32 + 0.5) / CAUSTIC as f32 * rect[3],
            ];
            let shaded = sphere_hit(centre, toward, room::LENS, radius, false).is_some();
            let direct = if shaded { 0.0 } else { 1.0 };
            gain[y * side + x] = direct + refracted - 1.0;
        }
    }
    (gain, rect)
}

fn halves(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|&v| {
            let h = half::f16::from_f32(v).to_bits().to_le_bytes();
            let one = half::f16::from_f32(1.0).to_bits().to_le_bytes();
            [h, h, h, one].into_iter().flatten()
        })
        .collect()
}

impl Painted {
    pub fn new(lighting: &Lighting) -> Self {
        let mut atlas = ContentAtlas::new(ATLAS, ATLAS, ContentFormat::Srgb8, 4).unwrap();
        let card_pixels: Vec<Vec<u8>> = (0..CARDS).map(card).collect();
        let ink_pixels = ink();
        let decal_pixels = decal();
        let mut images: Vec<AtlasImage<'_>> = card_pixels
            .iter()
            .map(|texels| AtlasImage {
                width: CARD.0,
                height: CARD.1,
                texels,
            })
            .collect();
        images.push(AtlasImage {
            width: INK.0,
            height: INK.1,
            texels: &ink_pixels,
        });
        images.push(AtlasImage {
            width: DECAL,
            height: DECAL,
            texels: &decal_pixels,
        });
        let ids = atlas.add_all(&images).unwrap();
        let (caustic, caustic_rect) = caustic(lighting.sun(HOUR).direction);
        Self {
            atlas,
            cards: ids[..CARDS as usize].to_vec(),
            ink: ids[CARDS as usize],
            decal: ids[CARDS as usize + 1],
            photo: photo(),
            video: (0..VIDEO_FRAMES).map(video_frame).collect(),
            caustic,
            caustic_rect,
        }
    }

    pub fn upload(&mut self, renderer: &Renderer) -> Result<(), String> {
        let frame = renderer.frame();
        self.atlas.allocate(frame, ATLAS_SLOT)?;
        self.atlas.upload(frame, ATLAS_SLOT)?;
        frame.set_content(PHOTO_SLOT, PHOTO.0, PHOTO.1, ContentFormat::Srgb8)?;
        frame.write_content(PHOTO_SLOT, [0, 0, PHOTO.0, PHOTO.1], &self.photo)?;
        frame.set_content(VIDEO_SLOT, VIDEO.0, VIDEO.1, ContentFormat::Srgb8)?;
        frame.write_content(VIDEO_SLOT, [0, 0, VIDEO.0, VIDEO.1], &self.video[0])?;
        frame.set_content(CAUSTIC_SLOT, CAUSTIC, CAUSTIC, ContentFormat::Linear16)?;
        frame.write_content(
            CAUSTIC_SLOT,
            [0, 0, CAUSTIC, CAUSTIC],
            &halves(&self.caustic),
        )?;
        Ok(())
    }

    pub fn surfaces(&self, room: &Room) -> Vec<(usize, InstanceSurface)> {
        let mut out = Vec::new();
        for (order, index) in room::cards(room).into_iter().enumerate() {
            let id = self.cards[(order * 9) % self.cards.len()];
            out.push((index, self.atlas.surface(id).unwrap()));
        }
        out.push((
            room::part(room, "sheet new"),
            self.atlas.surface(self.ink).unwrap(),
        ));
        out.push((
            room::part(room, "decal box"),
            self.atlas.surface(self.decal).unwrap(),
        ));
        out
    }
}
