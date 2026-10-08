use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::material::{Material, Recipe};

pub const FIXTURE: &str = include_str!("../fixtures/library.toml");

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Library {
    materials: BTreeMap<String, Material>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct File {
    materials: BTreeMap<String, Recipe>,
}

impl Library {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fixture() -> Self {
        Self::from_toml(FIXTURE).expect("the fixture library parses")
    }

    pub fn from_toml(text: &str) -> Result<Self, String> {
        let file: File = toml::from_str(text).map_err(|err| err.to_string())?;
        let mut materials = BTreeMap::new();
        for (name, recipe) in file.materials {
            if !valid_name(&name) {
                return Err(format!(
                    "material name {name:?} needs printable characters without spaces at its ends"
                ));
            }
            let material = recipe
                .into_material()
                .map_err(|err| format!("materials.{name}: {err}"))?;
            materials.insert(name, material);
        }
        Ok(Self { materials })
    }

    pub fn to_toml(&self) -> Result<String, String> {
        let file = File {
            materials: self
                .materials
                .iter()
                .map(|(name, material)| (name.clone(), Recipe::from(*material)))
                .collect(),
        };
        toml::to_string(&file).map_err(|err| err.to_string())
    }

    pub fn get(&self, name: &str) -> Option<&Material> {
        self.materials.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.materials.contains_key(name)
    }

    pub fn insert(&mut self, name: &str, material: Material) -> Result<Option<Material>, String> {
        if !valid_name(name) {
            return Err(format!(
                "material name {name:?} needs printable characters without spaces at its ends"
            ));
        }
        Ok(self.materials.insert(name.to_owned(), material))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.materials.keys().map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Material)> {
        self.materials
            .iter()
            .map(|(name, material)| (name.as_str(), material))
    }

    pub fn len(&self) -> usize {
        self.materials.len()
    }

    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.trim() == name && !name.chars().any(char::is_control)
}
