use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;

use crate::lex::{Kind, Lexed};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub path: Vec<String>,
    pub index: Option<usize>,
    pub header: Range<usize>,
}

impl Table {
    pub fn same(&self, other: &Table) -> bool {
        self.path == other.path && self.index == other.index
    }
}

fn header(text: &str) -> Option<(Vec<String>, bool)> {
    let (inner, array) = match text.strip_prefix("[[").and_then(|t| t.strip_suffix("]]")) {
        Some(inner) => (inner, true),
        None => (text.strip_prefix('[')?.strip_suffix(']')?, false),
    };
    let keys = toml_edit::Key::parse(inner.trim()).ok()?;
    Some((
        keys.iter().map(|key| key.get().to_string()).collect(),
        array,
    ))
}

type Entry = Option<(Vec<String>, Option<usize>)>;

fn headers(lexed: &Lexed) -> Vec<Range<usize>> {
    lexed
        .lines()
        .iter()
        .filter_map(|line| {
            let token = line
                .tokens
                .iter()
                .find(|token| token.kind == Kind::Header)?;
            Some(token.range.start + line.start..token.range.end + line.start)
        })
        .collect()
}

fn entries(text: &str, headers: &[Range<usize>]) -> Vec<Entry> {
    let mut counts: BTreeMap<Vec<String>, usize> = BTreeMap::new();
    headers
        .iter()
        .map(|range| {
            let (path, array) = header(&text[range.clone()])?;
            counts.retain(|known, _| !(known.len() > path.len() && known.starts_with(&path)));
            let index = array.then(|| {
                let count = counts.entry(path.clone()).or_default();
                *count += 1;
                *count - 1
            });
            Some((path, index))
        })
        .collect()
}

#[derive(Debug, Default)]
pub struct TableCache {
    key: Option<u64>,
    entries: Vec<Entry>,
    parsed: u64,
}

impl TableCache {
    pub fn at(&mut self, lexed: &Lexed, byte: usize) -> Option<Table> {
        let text = lexed.text();
        let headers = headers(lexed);
        let mut hasher = DefaultHasher::new();
        for range in &headers {
            text[range.clone()].hash(&mut hasher);
        }
        let key = hasher.finish();
        if self.key != Some(key) {
            self.entries = entries(text, &headers);
            self.key = Some(key);
            self.parsed += 1;
        }
        let line = lexed.line_of(byte.min(text.len()));
        let end = lexed
            .lines()
            .get(line + 1)
            .map_or(usize::MAX, |next| next.start);
        let before = headers.partition_point(|range| range.start < end);
        (0..before).rev().find_map(|index| {
            let (path, at) = self.entries[index].clone()?;
            Some(Table {
                path,
                index: at,
                header: headers[index].clone(),
            })
        })
    }

    pub fn parsed(&self) -> u64 {
        self.parsed
    }
}

pub fn tables(lexed: &Lexed) -> Vec<Table> {
    let headers = headers(lexed);
    entries(lexed.text(), &headers)
        .into_iter()
        .zip(headers)
        .filter_map(|(entry, header)| {
            let (path, index) = entry?;
            Some(Table {
                path,
                index,
                header,
            })
        })
        .collect()
}

pub fn table_at(lexed: &Lexed, byte: usize) -> Option<Table> {
    TableCache::default().at(lexed, byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "top = 1\n[[object]]\nname = \"a\"\n[[object]]\nname = \"b\"\n[[object.part]]\nx = 1\n[[object]]\nname = \"c\"\n[[object.part]]\nx = 2\n[materials.\"red one\"]\nshine = 0.5\n";

    fn at(needle: &str) -> Option<Table> {
        table_at(&Lexed::new(TEXT), TEXT.find(needle).unwrap())
    }

    #[test]
    fn each_header_names_its_table_and_its_place_in_an_array() {
        let found = tables(&Lexed::new(TEXT));
        let summary: Vec<(String, Option<usize>)> = found
            .iter()
            .map(|table| (table.path.join("."), table.index))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("object".into(), Some(0)),
                ("object".into(), Some(1)),
                ("object.part".into(), Some(0)),
                ("object".into(), Some(2)),
                ("object.part".into(), Some(0)),
                ("materials.red one".into(), None),
            ]
        );
        assert_eq!(&TEXT[found[5].header.clone()], "[materials.\"red one\"]");
    }

    #[test]
    fn a_byte_belongs_to_the_table_above_it() {
        assert_eq!(at("top"), None);
        assert_eq!(at("name = \"c\"").unwrap().index, Some(2));
        assert_eq!(at("x = 2").unwrap().path, vec!["object", "part"]);
        let header = TEXT.find("[[object]]\nname = \"b\"").unwrap();
        assert_eq!(
            table_at(&Lexed::new(TEXT), header + 3).unwrap().index,
            Some(1)
        );
        assert_eq!(table_at(&Lexed::new(TEXT), header).unwrap().index, Some(1));
        assert_eq!(at("shine").unwrap().path, vec!["materials", "red one"]);
        assert!(at("shine").unwrap().same(&at("shine").unwrap()));
    }

    #[test]
    fn the_cache_parses_headers_again_only_when_one_changes() {
        let mut cache = TableCache::default();
        let typed = TEXT.replace("x = 2", "x = 22");
        let renamed = TEXT.replace("[materials.\"red one\"]", "[materials.blue]");
        for (text, needle) in [(TEXT, "x = 2"), (&typed, "x = 22"), (&renamed, "shine")] {
            let lexed = Lexed::new(text);
            let at = text.find(needle).unwrap();
            assert_eq!(cache.at(&lexed, at), table_at(&lexed, at));
        }
        assert_eq!(cache.parsed(), 2);
    }
}
