use std::collections::BTreeMap;
use std::ops::Range;

use toml::Spanned;
use toml::de::{DeTable, DeValue};

pub(crate) type KeyPath = Vec<String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    Key,
    Value,
}

#[derive(Clone, Debug)]
struct Place {
    key: Option<Range<usize>>,
    value: Range<usize>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Spans {
    places: BTreeMap<KeyPath, Place>,
    ids: BTreeMap<KeyPath, String>,
    starts: Vec<usize>,
    text: String,
}

impl Spans {
    pub(crate) fn new(text: &str, table: Option<&Spanned<DeTable<'_>>>) -> Self {
        let mut spans = Self {
            places: BTreeMap::new(),
            ids: BTreeMap::new(),
            starts: std::iter::once(0)
                .chain(
                    text.bytes()
                        .enumerate()
                        .filter(|(_, byte)| *byte == b'\n')
                        .map(|(at, _)| at + 1),
                )
                .collect(),
            text: text.to_string(),
        };
        if let Some(table) = table {
            spans.places.insert(
                Vec::new(),
                Place {
                    key: None,
                    value: 0..0,
                },
            );
            spans.table(&mut Vec::new(), table.get_ref());
        }
        spans
    }

    fn table(&mut self, path: &mut KeyPath, table: &DeTable<'_>) {
        for (key, value) in table.iter() {
            path.push(key.get_ref().to_string());
            self.places.insert(
                path.clone(),
                Place {
                    key: Some(key.span()),
                    value: value.span(),
                },
            );
            if key.get_ref() == "id"
                && let Some(text) = value.get_ref().as_str()
            {
                let mut owner = path.clone();
                owner.pop();
                self.ids.insert(owner, text.to_string());
            }
            self.value(path, value);
            path.pop();
        }
    }

    fn value(&mut self, path: &mut KeyPath, value: &Spanned<DeValue<'_>>) {
        match value.get_ref() {
            DeValue::Table(table) => self.table(path, table),
            DeValue::Array(items) => {
                for (place, item) in items.iter().enumerate() {
                    path.push(place.to_string());
                    self.places.insert(
                        path.clone(),
                        Place {
                            key: None,
                            value: item.span(),
                        },
                    );
                    self.value(path, item);
                    path.pop();
                }
            }
            _ => {}
        }
    }

    pub(crate) fn has(&self, path: &[String]) -> bool {
        self.places.contains_key(path)
    }

    pub(crate) fn span(&self, path: &[String], part: Part) -> Range<usize> {
        let mut path = path.to_vec();
        let mut part = part;
        loop {
            if let Some(place) = self.places.get(&path) {
                return match (part, &place.key) {
                    (Part::Key, Some(key)) => key.clone(),
                    _ => place.value.clone(),
                };
            }
            if path.pop().is_none() {
                return 0..0;
            }
            part = Part::Value;
        }
    }

    pub(crate) fn path_at(&self, span: &Range<usize>) -> Option<(KeyPath, Part)> {
        let mut best: Option<(KeyPath, Part, usize)> = None;
        for (path, place) in &self.places {
            for (part, range) in [
                (Part::Key, place.key.clone()),
                (Part::Value, Some(place.value.clone())),
            ] {
                let Some(range) = range else { continue };
                let inside = range.start <= span.start
                    && span.end <= range.end.max(range.start + 1)
                    && !(range.is_empty() && !span.is_empty());
                if inside
                    && best
                        .as_ref()
                        .is_none_or(|(_, _, size)| range.len() <= *size)
                {
                    best = Some((path.clone(), part, range.len()));
                }
            }
        }
        best.map(|(path, part, _)| (path, part))
    }

    pub(crate) fn id_of(&self, entry: &[String]) -> Option<&str> {
        self.ids.get(entry).map(String::as_str)
    }

    pub(crate) fn position(&self, offset: usize) -> (u32, u32) {
        let offset = offset.min(self.text.len());
        let line = match self.starts.binary_search(&offset) {
            Ok(line) => line,
            Err(next) => next - 1,
        };
        let start = self.starts[line];
        let column = self
            .text
            .get(start..offset)
            .map_or(offset - start, |part| part.chars().count());
        (line as u32 + 1, column as u32 + 1)
    }
}

pub(crate) fn dotted(path: &[String], ids: &Spans) -> String {
    let mut out = Vec::new();
    for (place, segment) in path.iter().enumerate() {
        let is_index = segment.parse::<usize>().is_ok();
        match ids.id_of(&path[..=place]) {
            Some(id) if is_index => out.push(id.to_string()),
            _ => out.push(segment.clone()),
        }
    }
    out.join(".")
}
