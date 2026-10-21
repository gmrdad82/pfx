use crate::Material;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tint {
    pub base: [f32; 3],
    pub replace: bool,
    pub roughness: Option<f32>,
    pub metalness: Option<f32>,
}

impl Default for Tint {
    fn default() -> Self {
        Self::NONE
    }
}

impl Tint {
    pub const NONE: Tint = Tint {
        base: [1.0; 3],
        replace: false,
        roughness: None,
        metalness: None,
    };

    pub fn multiply(base: [f32; 3]) -> Tint {
        Tint { base, ..Tint::NONE }
    }

    pub fn is_none(&self) -> bool {
        *self == Tint::NONE
    }

    pub fn base_of(&self, base: [f32; 3]) -> [f32; 3] {
        if self.replace {
            self.base
        } else {
            std::array::from_fn(|c| base[c] * self.base[c])
        }
    }

    pub fn apply(&self, material: Material) -> Material {
        Material {
            base: self.base_of(material.base),
            roughness: self.roughness.unwrap_or(material.roughness),
            metalness: self.metalness.unwrap_or(material.metalness),
            ..material
        }
    }

    pub fn gpu(&self) -> [[f32; 4]; 2] {
        let [r, g, b] = self.base;
        [
            [r, g, b, if self.replace { 1.0 } else { 0.0 }],
            [
                self.roughness.unwrap_or(-1.0),
                self.metalness.unwrap_or(-1.0),
                0.0,
                0.0,
            ],
        ]
    }
}
