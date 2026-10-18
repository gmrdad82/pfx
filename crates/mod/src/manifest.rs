use serde::Deserialize;

use crate::error::Refusal;

pub const MANIFEST: &str = "mod.toml";
pub const ID_CHARS: usize = 64;
pub const DATA_CHARS: usize = 255;
pub const DATA_DEPTH: usize = 8;

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
    #[serde(default)]
    pub data: Vec<String>,
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
        for (at, name) in manifest.data.iter().enumerate() {
            if !valid_data(name) {
                return Err(Refusal::Data {
                    name: name.clone(),
                    problem: format!(
                        "a data file is a relative path of up to {DATA_DEPTH} folders and file names, each of a-z, A-Z, 0-9, `-`, `_` and `.` and none starting with `.`, at most {DATA_CHARS} bytes"
                    ),
                });
            }
            if *name == manifest.entry || name == MANIFEST {
                return Err(Refusal::Data {
                    name: name.clone(),
                    problem: format!("{MANIFEST} and the entry are not data files"),
                });
            }
            if manifest.data[..at].contains(name) {
                return Err(Refusal::Data {
                    name: name.clone(),
                    problem: "it is named twice".to_string(),
                });
            }
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

pub fn valid_data(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= DATA_CHARS
        && name.split('/').count() <= DATA_DEPTH
        && name.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        })
}
