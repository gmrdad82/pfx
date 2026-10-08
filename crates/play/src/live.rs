use std::collections::BTreeMap;

use pfx_load::scene::{Target, Value};

use crate::tunables::TunableValue;

#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Scene {
        target: Target,
        path: Vec<String>,
        value: Value,
    },
    Tunable {
        name: String,
        value: TunableValue,
    },
}

impl Edit {
    pub fn scene(target: Target, path: &[&str], value: impl Into<Value>) -> Edit {
        Edit::Scene {
            target,
            path: path.iter().map(|key| key.to_string()).collect(),
            value: value.into(),
        }
    }

    pub fn tunable(name: &str, value: TunableValue) -> Edit {
        Edit::Tunable {
            name: name.to_string(),
            value,
        }
    }

    pub fn key(&self) -> String {
        match self {
            Edit::Scene { target, path, .. } => format!("{target} {}", path.join(".")),
            Edit::Tunable { name, .. } => format!("tunable {name}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stamped {
    pub tick: u64,
    pub edit: Edit,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pending {
    edits: BTreeMap<String, Edit>,
}

impl Pending {
    pub(crate) fn hold(&mut self, edit: Edit) {
        self.edits.insert(edit.key(), edit);
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    pub fn len(&self) -> usize {
        self.edits.len()
    }

    pub fn get(&self, key: &str) -> Option<&Edit> {
        self.edits.get(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.edits.contains_key(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Edit> {
        self.edits.values()
    }

    pub fn scene(&self) -> impl Iterator<Item = (&Target, &[String], &Value)> {
        self.edits.values().filter_map(|edit| match edit {
            Edit::Scene {
                target,
                path,
                value,
            } => Some((target, path.as_slice(), value)),
            Edit::Tunable { .. } => None,
        })
    }

    pub fn tunables(&self) -> impl Iterator<Item = (&str, TunableValue)> {
        self.edits.values().filter_map(|edit| match edit {
            Edit::Tunable { name, value } => Some((name.as_str(), *value)),
            Edit::Scene { .. } => None,
        })
    }
}
