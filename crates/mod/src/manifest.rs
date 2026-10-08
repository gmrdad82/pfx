use serde::Deserialize;

use crate::error::Refusal;

pub const MANIFEST: &str = "mod.toml";
pub const ID_CHARS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Api {
    pub world: String,
    pub version: String,
    pub imports: Vec<String>,
}

impl Api {
    pub fn new(world: &str, version: &str) -> Self {
        Self {
            world: world.to_string(),
            version: version.to_string(),
            imports: Vec::new(),
        }
    }

    pub fn import(mut self, name: &str) -> Self {
        self.imports.push(name.to_string());
        self
    }

    pub fn accepts(&self, wanted: &str) -> Result<(), Refusal> {
        let refuse = || Refusal::Api {
            wanted: wanted.to_string(),
            world: self.world.clone(),
            version: self.version.clone(),
        };
        let game = semver::Version::parse(&self.version).map_err(|_| refuse())?;
        let need = semver::Version::parse(wanted).map_err(|_| refuse())?;
        let req = semver::VersionReq::parse(&format!("^{need}")).map_err(|_| refuse())?;
        if req.matches(&game) {
            Ok(())
        } else {
            Err(refuse())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub api: String,
    pub entry: String,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, Refusal> {
        let manifest: Self =
            toml::from_str(text).map_err(|e| Refusal::Manifest(e.message().to_string()))?;
        if !valid_id(&manifest.id) {
            return Err(Refusal::Id { found: manifest.id });
        }
        if semver::Version::parse(&manifest.version).is_err() {
            return Err(Refusal::Manifest(format!(
                "version `{}` is not a semantic version such as 1.0.0",
                manifest.version
            )));
        }
        if semver::Version::parse(&manifest.api).is_err() {
            return Err(Refusal::Manifest(format!(
                "api `{}` is not a semantic version such as 1.0.0",
                manifest.api
            )));
        }
        if !valid_entry(&manifest.entry) {
            return Err(Refusal::Entry {
                found: manifest.entry,
            });
        }
        Ok(manifest)
    }
}

pub fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    id.len() <= ID_CHARS
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub fn valid_entry(entry: &str) -> bool {
    let Some(stem) = entry.strip_suffix(".wasm") else {
        return false;
    };
    !stem.is_empty()
        && entry.len() <= ID_CHARS + 5
        && stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && !stem.starts_with('.')
}
