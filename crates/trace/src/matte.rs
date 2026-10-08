use crate::gpu::Trace;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Matte {
    pub transparent: bool,
    pub catcher: Option<u32>,
}

impl Matte {
    pub fn transparent() -> Self {
        Self {
            transparent: true,
            catcher: None,
        }
    }

    pub fn catcher(material: u32) -> Self {
        Self {
            transparent: true,
            catcher: Some(material),
        }
    }

    pub fn lane(self) -> f32 {
        match self.catcher {
            Some(material) => material as f32 + 2.0,
            None if self.transparent => 1.0,
            None => 0.0,
        }
    }
}

impl Trace {
    pub fn set_matte(&mut self, matte: Matte) -> Result<(), String> {
        if let Some(material) = matte.catcher {
            if material >= self.materials {
                return Err(format!(
                    "the shadow catcher names material {material}, the scene has {}",
                    self.materials
                ));
            }
            if material >= 1 << 23 {
                return Err("the shadow catcher's material is past 2^23".into());
            }
            if !matte.transparent {
                return Err("a shadow catcher needs the transparent film".into());
            }
        }
        self.base.forward[3] = matte.lane();
        self.samples = 0;
        Ok(())
    }

    pub fn matte(&self) -> Matte {
        let lane = self.base.forward[3];
        Matte {
            transparent: lane > 0.0,
            catcher: (lane > 1.5).then(|| lane as u32 - 2),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub texels: Vec<[f32; 4]>,
}

impl Rgba {
    pub fn premultiplied(width: u32, height: u32, color: &[u8]) -> Result<Self, String> {
        let texels = crate::probe::texels(color);
        if texels.len() != width as usize * height as usize {
            return Err("the colour readback does not match the size".into());
        }
        Ok(Self {
            width,
            height,
            texels,
        })
    }

    pub fn straight(&self) -> Vec<[f32; 4]> {
        self.texels.iter().map(|&texel| straight(texel)).collect()
    }
}

pub fn straight(texel: [f32; 4]) -> [f32; 4] {
    let alpha = texel[3].clamp(0.0, 1.0);
    if alpha <= 1.0 / 65535.0 {
        return [0.0, 0.0, 0.0, 0.0];
    }
    [texel[0] / alpha, texel[1] / alpha, texel[2] / alpha, alpha]
}

pub fn over(texel: [f32; 4], background: [f32; 3]) -> [f32; 4] {
    let alpha = texel[3].clamp(0.0, 1.0);
    [
        texel[0] + background[0] * (1.0 - alpha),
        texel[1] + background[1] * (1.0 - alpha),
        texel[2] + background[2] * (1.0 - alpha),
        1.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lanes_round_trip() {
        assert_eq!(Matte::default().lane(), 0.0);
        assert_eq!(Matte::transparent().lane(), 1.0);
        assert_eq!(Matte::catcher(0).lane(), 2.0);
        assert_eq!(Matte::catcher(7).lane(), 9.0);
    }

    #[test]
    fn straight_alpha_undoes_premultiplying() {
        assert_eq!(straight([0.2, 0.1, 0.05, 0.5]), [0.4, 0.2, 0.1, 0.5]);
        assert_eq!(straight([0.0, 0.0, 0.0, 0.0]), [0.0; 4]);
        assert_eq!(straight([0.3, 0.3, 0.3, 1.0]), [0.3, 0.3, 0.3, 1.0]);
        assert_eq!(
            over([0.0, 0.0, 0.0, 0.25], [1.0, 0.5, 0.0]),
            [0.75, 0.375, 0.0, 1.0]
        );
        let rgba =
            Rgba::premultiplied(1, 1, bytemuck::cast_slice(&[0.1f32, 0.2, 0.3, 0.5])).unwrap();
        assert!((rgba.straight()[0][2] - 0.6).abs() < 1e-6);
        assert!(Rgba::premultiplied(2, 1, &[0u8; 16]).is_err());
    }
}
