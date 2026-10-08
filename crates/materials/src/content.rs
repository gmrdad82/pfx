use serde::{Deserialize, Serialize};

use crate::brdf::Shaded;
use crate::material::Content;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ContentLook {
    pub ink_roughness: f32,
    pub ink_emboss_coated: f32,
    pub ink_emboss_bare: f32,
    pub photo_gain: f32,
    pub screen_gain: f32,
    pub photo_window: [f32; 4],
}

impl Default for ContentLook {
    fn default() -> Self {
        Self {
            ink_roughness: -1.0,
            ink_emboss_coated: 0.0,
            ink_emboss_bare: 0.0,
            photo_gain: 1.0,
            screen_gain: 1.0,
            photo_window: [0.0, 0.0, 1.0, 1.0],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blend {
    #[default]
    Over,
    Multiply,
    Emit,
}

impl Blend {
    pub fn id(self) -> u32 {
        match self {
            Self::Over => 0,
            Self::Multiply => 1,
            Self::Emit => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ContentLayer {
    pub slot: i32,
    pub blend: Blend,
    pub ink_roughness: f32,
    pub emboss: f32,
    pub strength: f32,
}

impl Default for ContentLayer {
    fn default() -> Self {
        Self {
            slot: -1,
            blend: Blend::Over,
            ink_roughness: -1.0,
            emboss: 0.0,
            strength: 1.0,
        }
    }
}

impl ContentLayer {
    pub fn active(&self) -> bool {
        self.slot >= 0
    }

    pub fn from_kind(content: Content, clearcoat: f32, slot: i32, look: &ContentLook) -> Self {
        let layer = Self {
            slot,
            ..Self::default()
        };
        match content {
            Content::None => Self::default(),
            Content::Ink => Self {
                ink_roughness: look.ink_roughness,
                emboss: if clearcoat > 0.0 {
                    look.ink_emboss_coated
                } else {
                    look.ink_emboss_bare
                },
                ..layer
            },
            Content::Decal | Content::Scroll | Content::Print | Content::Field => layer,
            Content::Photo => Self {
                blend: Blend::Multiply,
                strength: look.photo_gain,
                ..layer
            },
            Content::Screen => Self {
                blend: Blend::Emit,
                strength: look.screen_gain,
                ..layer
            },
        }
    }

    pub fn composite(&self, surface: &Shaded, texel: [f32; 4]) -> Shaded {
        let mut out = *surface;
        let a = texel[3].clamp(0.0, 1.0);
        let rgb = [texel[0], texel[1], texel[2]];
        match self.blend {
            Blend::Over => {
                out.base = std::array::from_fn(|c| surface.base[c] * (1.0 - a) + rgb[c]);
                out.emission = surface.emission.map(|value| value * (1.0 - a));
                if self.ink_roughness >= 0.0 {
                    let alpha = surface.roughness * surface.roughness;
                    let ink = self.ink_roughness * self.ink_roughness;
                    out.roughness = (alpha + (ink - alpha) * a).sqrt();
                }
                out.subsurface = surface.subsurface * (1.0 - a).powi(4);
            }
            Blend::Multiply => {
                out.base =
                    std::array::from_fn(|c| surface.base[c] * (1.0 - a + rgb[c] * self.strength));
            }
            Blend::Emit => {
                out.emission =
                    std::array::from_fn(|c| surface.emission[c] + rgb[c] * self.strength);
            }
        }
        out
    }
}

pub fn content_uv(
    uv: [f32; 2],
    offset: [f32; 2],
    scale: [f32; 2],
    crop: [f32; 4],
) -> Option<[f32; 2]> {
    let at = [uv[0] * scale[0] + offset[0], uv[1] * scale[1] + offset[1]];
    (at[0] >= crop[0] && at[1] >= crop[1] && at[0] <= crop[2] && at[1] <= crop[3]).then_some(at)
}

pub fn emboss(
    normal: [f32; 3],
    tangent: [f32; 3],
    bitangent: [f32; 3],
    gradient: [f32; 2],
    strength: f32,
) -> [f32; 3] {
    let tilt: [f32; 3] =
        std::array::from_fn(|k| (tangent[k] * gradient[0] + bitangent[k] * gradient[1]) * strength);
    let along = tilt.iter().zip(normal).map(|(t, n)| t * n).sum::<f32>();
    let bent: [f32; 3] = std::array::from_fn(|k| normal[k] - (tilt[k] - along * normal[k]));
    let length = bent.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-8);
    bent.map(|v| v / length)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::Material;

    const LOOK: ContentLook = ContentLook {
        ink_roughness: 0.5,
        ink_emboss_coated: 0.3,
        ink_emboss_bare: -0.2,
        photo_gain: 1.25,
        screen_gain: 2.0,
        photo_window: [0.25, 0.0, 0.5, 1.0],
    };

    fn surface() -> Shaded {
        Shaded::from_material(&Material {
            base: [0.8, 0.6, 0.4],
            roughness: 0.9,
            subsurface: 0.5,
            emission: [0.2, 0.1, 0.0],
            ..Material::default()
        })
    }

    #[test]
    fn over_is_premultiplied_ink_with_the_looks_roughness() {
        let layer = ContentLayer::from_kind(Content::Ink, 1.0, 0, &LOOK);
        let base = surface();
        let ink = [0.05, 0.02, 0.01, 0.75];
        let out = layer.composite(&base, ink);
        for c in 0..3 {
            let over = base.base[c] * (1.0 - ink[3]) + ink[c];
            assert!((out.base[c] - over).abs() < 1e-6);
            assert!((out.emission[c] - base.emission[c] * 0.25).abs() < 1e-6);
        }
        let alpha = 0.9f32 * 0.9;
        let mixed = alpha + (0.25 - alpha) * 0.75;
        assert!((out.roughness * out.roughness - mixed).abs() < 1e-5);
        assert!((out.subsurface - 0.5 * 0.25f32.powi(4)).abs() < 1e-7);
        let clear = layer.composite(&base, [0.0; 4]);
        assert_eq!(clear.base, base.base);
        assert!((clear.roughness - base.roughness).abs() < 1e-6);
        let decal = ContentLayer::from_kind(Content::Decal, 1.0, 1, &LOOK).composite(&base, ink);
        assert_eq!(decal.roughness, base.roughness);
        let neutral = ContentLayer::from_kind(Content::Ink, 1.0, 0, &ContentLook::default());
        assert_eq!(neutral.composite(&base, ink).roughness, base.roughness);
        assert_eq!(neutral.emboss, 0.0);
    }

    #[test]
    fn multiply_and_emit_follow_the_photo_and_screen_gains() {
        let base = surface();
        let frame = [0.5, 0.25, 0.125, 1.0];
        let photo = ContentLayer::from_kind(Content::Photo, 1.0, 3, &LOOK).composite(&base, frame);
        for ((out, before), value) in photo.base.iter().zip(base.base).zip(frame) {
            assert!((out - before * value * LOOK.photo_gain).abs() < 1e-6);
        }
        assert_eq!(photo.emission, base.emission);
        let screen =
            ContentLayer::from_kind(Content::Screen, 1.0, 3, &LOOK).composite(&base, frame);
        for ((out, before), value) in screen.emission.iter().zip(base.emission).zip(frame) {
            assert!((out - (before + value * LOOK.screen_gain)).abs() < 1e-6);
        }
        assert_eq!(screen.base, base.base);
        let dark =
            ContentLayer::from_kind(Content::Screen, 1.0, 3, &LOOK).composite(&base, [0.0; 4]);
        assert_eq!(dark.emission, base.emission);
        let plain = ContentLayer::from_kind(Content::Photo, 1.0, 3, &ContentLook::default());
        assert_eq!(plain.strength, 1.0);
    }

    #[test]
    fn ink_rises_out_of_the_surface() {
        let n = [0.0, 0.0, 1.0];
        let t = [1.0, 0.0, 0.0];
        let b = [0.0, 1.0, 0.0];
        let rising = emboss(n, t, b, [0.5, 0.0], LOOK.ink_emboss_coated);
        assert!(rising[0] < 0.0 && rising[1].abs() < 1e-7 && rising[2] > 0.9);
        let pressed = emboss(n, t, b, [0.5, 0.0], LOOK.ink_emboss_bare);
        assert!(pressed[0] > 0.0);
        let down = emboss(n, t, b, [0.0, 0.5], LOOK.ink_emboss_coated);
        assert!(down[1] < 0.0 && down[0].abs() < 1e-7);
        assert_eq!(emboss(n, t, b, [0.0, 0.0], 1.0), n);
        let length = rising.iter().map(|v| v * v).sum::<f32>();
        assert!((length - 1.0).abs() < 1e-5);
    }

    #[test]
    fn uv_offset_and_crop() {
        let [x, y, w, h] = LOOK.photo_window;
        let centre = content_uv([0.5, 0.5], [x, y], [w, h], [0.0, 0.0, 1.0, 1.0]).unwrap();
        assert!((centre[0] - 0.5).abs() < 1e-6 && (centre[1] - 0.5).abs() < 1e-6);
        let left = content_uv([0.0, 0.2], [x, y], [w, h], [0.0, 0.0, 1.0, 1.0]).unwrap();
        assert!((left[0] - 0.25).abs() < 1e-6);
        let tape = [0.0, 0.0, 1.0, 2000.0 / 2048.0];
        let scrolled = content_uv([0.4, 0.5], [0.0, 0.25], [1.0, 1.0], tape).unwrap();
        assert!((scrolled[1] - 0.75).abs() < 1e-6);
        assert!(content_uv([0.4, 0.8], [0.0, 0.25], [1.0, 1.0], tape).is_none());
        assert!(content_uv([-0.01, 0.5], [0.0; 2], [1.0; 2], [0.0, 0.0, 1.0, 1.0]).is_none());
    }

    #[test]
    fn layers_by_content_kind() {
        assert!(!ContentLayer::from_kind(Content::None, 0.0, 2, &LOOK).active());
        let bare = ContentLayer::from_kind(Content::Ink, 0.0, 0, &LOOK);
        assert_eq!(bare.emboss, LOOK.ink_emboss_bare);
        assert_eq!(bare.ink_roughness, LOOK.ink_roughness);
        assert_eq!(
            ContentLayer::from_kind(Content::Ink, 0.4, 0, &LOOK).emboss,
            LOOK.ink_emboss_coated
        );
        assert_eq!(
            ContentLayer::from_kind(Content::Photo, 1.0, 3, &LOOK).blend,
            Blend::Multiply
        );
        assert_eq!(
            ContentLayer::from_kind(Content::Screen, 1.0, 3, &LOOK).blend,
            Blend::Emit
        );
        for kind in [
            Content::Decal,
            Content::Scroll,
            Content::Print,
            Content::Field,
        ] {
            assert_eq!(
                ContentLayer::from_kind(kind, 0.0, 1, &LOOK).blend,
                Blend::Over
            );
        }
        assert_eq!(ContentLayer::default().slot, -1);
    }

    #[test]
    fn looks_read_from_toml() {
        let look: ContentLook = toml::from_str("screen_gain = 3.0").unwrap();
        assert_eq!(look.screen_gain, 3.0);
        assert_eq!(look.photo_gain, 1.0);
        assert!(toml::from_str::<ContentLook>("ink_gain = 1.0").is_err());
    }
}
