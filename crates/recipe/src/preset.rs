use std::path::Path;

use pfx_motion::{Rate, Timeline};
use toml_edit::{DocumentMut, Item, Table, TableLike, Value};

use crate::Error;
use crate::include::Sources;

mod camera;
mod lighting;

pub(crate) type Rule = (&'static str, fn(f64) -> bool);

fn finite(v: f64) -> bool {
    v.is_finite()
}

fn above_zero(v: f64) -> bool {
    v.is_finite() && v > 0.0
}

fn from_zero(v: f64) -> bool {
    v.is_finite() && v >= 0.0
}

fn below_ninety(v: f64) -> bool {
    v.is_finite() && (-90.0..=90.0).contains(&v)
}

fn roll(v: f64) -> bool {
    v.is_finite() && (0.0..90.0).contains(&v)
}

pub(crate) const ANY: Rule = ("a number", finite);
pub(crate) const ABOVE_0: Rule = ("a number above 0", above_zero);
pub(crate) const FROM_0: Rule = ("a number of 0 or more", from_zero);
pub(crate) const PITCH: Rule = ("degrees from -90 to 90", below_ninety);
pub(crate) const ROLL: Rule = ("degrees from 0 up to 90", roll);

pub(crate) struct Params<'a> {
    table: &'a dyn TableLike,
    path: String,
    line: Option<usize>,
    src: &'a Sources,
    read: Vec<&'static str>,
}

fn rate_of(item: &Item) -> Option<Rate> {
    if let Some(text) = item.as_str() {
        return text.parse().ok();
    }
    Rate::from_f64(number_of(item)?).ok()
}

fn number_of(item: &Item) -> Option<f64> {
    item.as_float()
        .or_else(|| item.as_integer().map(|i| i as f64))
}

impl<'a> Params<'a> {
    pub(crate) fn new(
        src: &'a Sources,
        table: &'a dyn TableLike,
        path: String,
        line: Option<usize>,
    ) -> Params<'a> {
        Params {
            table,
            path,
            line,
            src,
            read: Vec::new(),
        }
    }

    fn line_of(&self, item: &Item) -> Option<usize> {
        item.span()
            .map(|span| self.src.line(span.start))
            .or(self.line)
    }

    pub(crate) fn refuse(&self, key: &str, item: &Item, text: impl Into<String>) -> Error {
        Error::new(format!("{}.{key}", self.path), self.line_of(item), text)
    }

    pub(crate) fn missing(&self, key: &str, what: &str) -> Error {
        Error::new(
            format!("{}.{key}", self.path),
            self.line,
            format!("missing, expected {what}"),
        )
    }

    fn get(&mut self, key: &'static str) -> Option<&'a Item> {
        self.read.push(key);
        self.table.get(key)
    }

    pub(crate) fn number(
        &mut self,
        key: &'static str,
        default: f64,
        rule: Rule,
    ) -> Result<f64, Error> {
        let Some(item) = self.get(key) else {
            return Ok(default);
        };
        match number_of(item) {
            Some(v) if (rule.1)(v) => Ok(v),
            Some(v) => Err(self.refuse(key, item, format!("expected {}, found {v}", rule.0))),
            None => Err(self.refuse(key, item, format!("expected {}", rule.0))),
        }
    }

    pub(crate) fn maybe(&mut self, key: &'static str, rule: Rule) -> Result<Option<f64>, Error> {
        if self.table.get(key).is_none() {
            self.read.push(key);
            return Ok(None);
        }
        self.number(key, 0.0, rule).map(Some)
    }

    pub(crate) fn whole(&mut self, key: &'static str) -> Result<Option<u64>, Error> {
        let Some(item) = self.get(key) else {
            return Ok(None);
        };
        match item.as_integer().and_then(|i| u64::try_from(i).ok()) {
            Some(whole) => Ok(Some(whole)),
            None => Err(self.refuse(key, item, "expected a whole number from 0 up")),
        }
    }

    pub(crate) fn pair(&mut self, key: &'static str) -> Result<Option<[f64; 2]>, Error> {
        let Some(item) = self.get(key) else {
            return Ok(None);
        };
        if let Some(v) = number_of(item).filter(|v| v.is_finite()) {
            return Ok(Some([v, v]));
        }
        let numbers: Option<Vec<f64>> = item.as_array().and_then(|list| {
            list.iter()
                .map(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
                .collect()
        });
        match numbers.as_deref() {
            Some(&[a, b]) if a.is_finite() && b.is_finite() => Ok(Some([a, b])),
            _ => Err(self.refuse(key, item, "expected a number or [from, to]")),
        }
    }

    pub(crate) fn point(
        &mut self,
        key: &'static str,
        default: [f64; 3],
    ) -> Result<[f64; 3], Error> {
        let Some(item) = self.get(key) else {
            return Ok(default);
        };
        let numbers: Option<Vec<f64>> = item.as_array().and_then(|list| {
            list.iter()
                .map(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
                .collect()
        });
        match numbers.as_deref() {
            Some(&[x, y, z]) if [x, y, z].iter().all(|v| v.is_finite()) => Ok([x, y, z]),
            _ => Err(self.refuse(key, item, "expected an array of 3 numbers")),
        }
    }

    pub(crate) fn text(&mut self, key: &'static str) -> Result<Option<&'a str>, Error> {
        let Some(item) = self.get(key) else {
            return Ok(None);
        };
        match item.as_str() {
            Some(text) => Ok(Some(text)),
            None => Err(self.refuse(key, item, "expected text")),
        }
    }

    pub(crate) fn name(&mut self, key: &'static str, what: &str) -> Result<&'a str, Error> {
        match self.text(key)? {
            Some(text) if !text.is_empty() => Ok(text),
            Some(_) => Err(self.refuse_key(key, format!("expected {what}"))),
            None => Err(self.missing(key, what)),
        }
    }

    fn refuse_key(&self, key: &str, text: String) -> Error {
        let line = self
            .table
            .get(key)
            .map_or(self.line, |item| self.line_of(item));
        Error::new(format!("{}.{key}", self.path), line, text)
    }

    pub(crate) fn choice(
        &mut self,
        key: &'static str,
        default: &'static str,
        options: &[&'static str],
    ) -> Result<&'static str, Error> {
        let Some(text) = self.text(key)? else {
            return Ok(default);
        };
        match options.iter().find(|o| **o == text) {
            Some(found) => Ok(found),
            None => Err(self.refuse_key(
                key,
                format!(
                    "no value named `{text}`, expected one of: {}",
                    options.join(", ")
                ),
            )),
        }
    }

    pub(crate) fn finish(self) -> Result<(), Error> {
        for (key, item) in self.table.iter() {
            if !self.read.contains(&key) {
                return Err(self.refuse(
                    key,
                    item,
                    format!(
                        "no key named `{key}`, expected one of: {}",
                        self.read.join(", ")
                    ),
                ));
            }
        }
        Ok(())
    }
}

pub(crate) fn num(v: f64) -> String {
    format!("{v:?}")
}

pub(crate) fn point(v: [f64; 3]) -> String {
    format!("[{}, {}, {}]", num(v[0]), num(v[1]), num(v[2]))
}

pub(crate) fn quote(text: &str) -> String {
    Value::from(text).to_string().trim().to_string()
}

pub(crate) fn negated(v: f64) -> f64 {
    0.0 - v
}

pub(crate) fn parsed(text: &str, path: &str) -> Result<DocumentMut, Error> {
    text.parse::<DocumentMut>().map_err(|e| {
        Error::new(
            path,
            None,
            format!("the preset wrote invalid TOML: {}", e.message()),
        )
    })
}

pub(crate) fn fill(target: &mut dyn TableLike, from: &Table) {
    for (key, item) in from.iter() {
        match target.get_mut(key) {
            None => {
                target.insert(key, item.clone());
            }
            Some(have) => {
                if let (Some(have), Some(want)) = (have.as_table_like_mut(), item.as_table()) {
                    fill(have, want);
                }
            }
        }
    }
}

pub(crate) struct Clock {
    pub seconds: Option<f64>,
}

impl Clock {
    fn of(root: &Table) -> Clock {
        let Some(table) = root.get("timeline").and_then(Item::as_table_like) else {
            return Clock { seconds: None };
        };
        let fps = table.get("fps").and_then(rate_of).unwrap_or(Timeline::FPS);
        let seconds = table.get("seconds").and_then(number_of).or_else(|| {
            table
                .get("frames")
                .and_then(number_of)
                .map(|frames| frames * f64::from(fps.den()) / f64::from(fps.num()))
        });
        Clock {
            seconds: seconds.filter(|s| s.is_finite() && *s > 0.0),
        }
    }
}

pub(crate) fn expand(table: &mut Table, src: &Sources, base: Option<&Path>) -> Result<(), Error> {
    let clock = Clock::of(table);
    camera::expand(table, src, &clock).map_err(|e| src.error(e))?;
    lighting::expand(table, src, base).map_err(|e| src.error(e))
}
