use std::collections::BTreeMap;

use pfx_physics::rigid;

pub const MAX: usize = 32;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layers {
    names: Vec<String>,
    masks: Vec<u32>,
}

impl Layers {
    pub fn from_scene(layers: &BTreeMap<String, Vec<String>>) -> Result<Layers, String> {
        if layers.len() > MAX {
            return Err(format!(
                "[layers] names {} layers; at most {MAX} can be named",
                layers.len()
            ));
        }
        let names: Vec<String> = layers.keys().cloned().collect();
        let mut masks = Vec::new();
        for (name, list) in layers {
            let mut mask = 0u32;
            for other in list {
                let at = names
                    .iter()
                    .position(|known| known == other)
                    .ok_or_else(|| {
                        format!("layer {name} lists {other:?}, which is not a layer in [layers]")
                    })?;
                mask |= 1 << at;
            }
            masks.push(mask);
        }
        Ok(Layers { names, masks })
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(String::as_str)
    }

    pub fn bit(&self, name: &str) -> Option<u32> {
        self.names
            .iter()
            .position(|known| known == name)
            .map(|at| 1 << at)
    }

    pub(crate) fn require(&self, name: &str) -> Result<u32, String> {
        self.bit(name).ok_or_else(|| self.unknown(name))
    }

    pub fn mask<S: AsRef<str>>(&self, names: &[S]) -> Result<u32, String> {
        let mut mask = 0;
        for name in names {
            mask |= self.require(name.as_ref())?;
        }
        Ok(mask)
    }

    pub fn names_of(&self, mask: u32) -> Vec<&str> {
        self.names
            .iter()
            .enumerate()
            .filter(|(at, _)| mask & (1 << at) != 0)
            .map(|(_, name)| name.as_str())
            .collect()
    }

    pub fn collides(&self, a: &str, b: &str) -> Option<bool> {
        let (bit_a, bit_b) = (self.bit(a)?, self.bit(b)?);
        Some(self.collision_mask(bit_a) & bit_b != 0 && self.collision_mask(bit_b) & bit_a != 0)
    }

    pub fn collision_mask(&self, layer: u32) -> u32 {
        self.masks
            .iter()
            .enumerate()
            .filter(|(at, _)| layer & (1 << at) != 0)
            .fold(0, |mask, (_, layers)| mask | layers)
    }

    pub fn groups(&self, layer: u32) -> rigid::Layers {
        rigid::Layers::new(layer, self.collision_mask(layer))
    }

    pub(crate) fn unknown(&self, name: &str) -> String {
        if self.names.is_empty() {
            format!("layer {name:?}: the project declares no [layers]")
        } else {
            let known: Vec<&str> = self.names().collect();
            format!(
                "layer {name:?} is not a layer in [layers] ({})",
                known.join(", ")
            )
        }
    }
}
