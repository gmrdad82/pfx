use super::Value;
use std::collections::BTreeSet;
use toml::{Table, Value as Toml};

pub(crate) struct Fields<'a> {
    table: &'a Table,
    used: BTreeSet<&'a str>,
    at: String,
}

impl<'a> Fields<'a> {
    pub(crate) fn new(table: &'a Table, at: String) -> Self {
        Self {
            table,
            used: BTreeSet::new(),
            at,
        }
    }

    fn get(&mut self, key: &'a str) -> Option<&'a Toml> {
        let value = self.table.get(key)?;
        self.used.insert(key);
        Some(value)
    }

    fn wrong(&self, key: &str, want: &str) -> String {
        format!("{}: `{key}` must be {want}", self.at)
    }

    pub(crate) fn number(&mut self, key: &'a str) -> Result<Option<f32>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(value) => number(value)
                .map(Some)
                .ok_or_else(|| self.wrong(key, "a number")),
        }
    }

    pub(crate) fn number_or(&mut self, key: &'a str, default: f32) -> Result<f32, String> {
        Ok(self.number(key)?.unwrap_or(default))
    }

    pub(crate) fn whole(&mut self, key: &'a str) -> Result<Option<u64>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Toml::Integer(n)) if *n >= 0 => Ok(Some(*n as u64)),
            Some(_) => Err(self.wrong(key, "a whole number of 0 or more")),
        }
    }

    pub(crate) fn text(&mut self, key: &'a str) -> Result<Option<String>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Toml::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(self.wrong(key, "a string")),
        }
    }

    pub(crate) fn value(&mut self, key: &'a str, default: f32) -> Result<Value, String> {
        match self.get(key) {
            None => Ok(Value::Fixed(default)),
            Some(Toml::String(s)) => Ok(Value::Curve(s.clone())),
            Some(value) => number(value)
                .map(Value::Fixed)
                .ok_or_else(|| self.wrong(key, "a number or a curve name")),
        }
    }

    pub(crate) fn numbers(&mut self, key: &'a str) -> Result<Option<Vec<f32>>, String> {
        let Some(value) = self.get(key) else {
            return Ok(None);
        };
        let want = || self.wrong(key, "an array of numbers");
        let Toml::Array(items) = value else {
            return Err(want());
        };
        items
            .iter()
            .map(|item| number(item).ok_or_else(want))
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    pub(crate) fn pairs(&mut self, key: &'a str) -> Result<Option<Vec<[f32; 2]>>, String> {
        let Some(value) = self.get(key) else {
            return Ok(None);
        };
        let want = || self.wrong(key, "an array of [a, b] pairs");
        let Toml::Array(items) = value else {
            return Err(want());
        };
        items
            .iter()
            .map(|item| match item {
                Toml::Array(pair) if pair.len() == 2 => {
                    match (number(&pair[0]), number(&pair[1])) {
                        (Some(a), Some(b)) => Ok([a, b]),
                        _ => Err(want()),
                    }
                }
                _ => Err(want()),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    pub(crate) fn pair(&mut self, key: &'a str, default: [f32; 2]) -> Result<[f32; 2], String> {
        match self.numbers(key)? {
            None => Ok(default),
            Some(v) if v.len() == 2 => Ok([v[0], v[1]]),
            Some(_) => Err(self.wrong(key, "a pair [a, b]")),
        }
    }

    pub(crate) fn flag(&mut self, key: &'a str) -> Result<Option<bool>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Toml::Boolean(b)) => Ok(Some(*b)),
            Some(_) => Err(self.wrong(key, "true or false")),
        }
    }

    pub(crate) fn triples(&mut self, key: &'a str) -> Result<Option<Vec<[f32; 3]>>, String> {
        let Some(value) = self.get(key) else {
            return Ok(None);
        };
        let want = || self.wrong(key, "an array of [a, b, c] triples");
        let Toml::Array(items) = value else {
            return Err(want());
        };
        items
            .iter()
            .map(|item| match item {
                Toml::Array(row) if row.len() == 3 => {
                    match (number(&row[0]), number(&row[1]), number(&row[2])) {
                        (Some(a), Some(b), Some(c)) => Ok([a, b, c]),
                        _ => Err(want()),
                    }
                }
                _ => Err(want()),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    pub(crate) fn stamps(&mut self, key: &'a str) -> Result<Option<Vec<(f32, String)>>, String> {
        let Some(value) = self.get(key) else {
            return Ok(None);
        };
        let want = || self.wrong(key, "a name or an array of [t, name] pairs");
        match value {
            Toml::String(s) => Ok(Some(vec![(0.0, s.clone())])),
            Toml::Array(items) if !items.is_empty() => items
                .iter()
                .map(|item| match item {
                    Toml::Array(pair) if pair.len() == 2 => match (number(&pair[0]), &pair[1]) {
                        (Some(t), Toml::String(s)) => Ok((t, s.clone())),
                        _ => Err(want()),
                    },
                    _ => Err(want()),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
            _ => Err(want()),
        }
    }

    pub(crate) fn mark(&mut self, key: &'a str) {
        if self.table.contains_key(key) {
            self.used.insert(key);
        }
    }

    pub(crate) fn finish(self) -> Result<(), String> {
        match self.table.keys().find(|k| !self.used.contains(k.as_str())) {
            Some(key) => Err(format!("{}: unknown key `{key}`", self.at)),
            None => Ok(()),
        }
    }
}

fn number(value: &Toml) -> Option<f32> {
    let n = match value {
        Toml::Float(f) => *f as f32,
        Toml::Integer(i) => *i as f32,
        _ => return None,
    };
    n.is_finite().then_some(n)
}

pub(crate) fn tables<'a>(root: &'a Table, key: &str) -> Result<Vec<&'a Table>, String> {
    match root.get(key) {
        None => Ok(Vec::new()),
        Some(Toml::Array(items)) => items
            .iter()
            .map(|item| match item {
                Toml::Table(t) => Ok(t),
                _ => Err(format!("`[[{key}]]` must be tables")),
            })
            .collect(),
        Some(_) => Err(format!("`{key}` must be written as `[[{key}]]` tables")),
    }
}
