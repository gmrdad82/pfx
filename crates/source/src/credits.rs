use std::collections::{BTreeMap, BTreeSet};

use pfx_file::Reader;
use sha2::{Digest, Sha256};
use toml::{Table, Value};

use crate::licence::credit_line;
use crate::record::Record;

const DEPTH: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credit {
    pub name: String,
    pub licence: String,
    pub credit: String,
    pub author: Option<String>,
    pub source: Option<String>,
    pub from: Option<String>,
    pub used_by: BTreeSet<String>,
}

impl Credit {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "licence": self.licence,
            "credit": self.credit,
            "author": self.author,
            "source": self.source,
            "from": self.from,
            "used_by": self.used_by,
        })
    }
}

struct Gather<'a> {
    own: &'a [&'a str],
    credits: Vec<Credit>,
    problems: Vec<String>,
}

fn facts(reader: &Reader<'_>) -> Table {
    reader
        .chunk(pfx_file::FACT)
        .ok()
        .flatten()
        .and_then(|bytes| std::str::from_utf8(&bytes).ok()?.parse::<Table>().ok())
        .unwrap_or_default()
}

fn field(table: &Table, key: &str) -> Option<String> {
    table
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Gather<'_> {
    fn add(
        &mut self,
        what: &str,
        name: String,
        table: &Table,
        from: Option<String>,
        used_by: BTreeSet<String>,
    ) {
        let licence = field(table, "licence").unwrap_or_default();
        if self.own.contains(&licence.as_str()) {
            return;
        }
        if licence.is_empty() {
            self.problems
                .push(format!("{what}: unlicensed, it records no licence"));
            return;
        }
        if let Err(text) = pfx_core::spdx::validate(&licence) {
            self.problems.push(format!(
                "{what}: licence \"{licence}\" is not an SPDX expression ({text})"
            ));
            return;
        }
        let author = field(table, "author");
        let source = field(table, "source");
        let credit = field(table, "credit")
            .unwrap_or_else(|| credit_line(&name, author.as_deref(), source.as_deref(), &licence));
        if let Some(same) = self
            .credits
            .iter_mut()
            .find(|c| c.licence == licence && c.credit == credit)
        {
            same.used_by.extend(used_by);
            return;
        }
        self.credits.push(Credit {
            name,
            licence,
            credit,
            author,
            source,
            from,
            used_by,
        });
    }

    fn container(&mut self, reader: &Reader<'_>, at: &str, depth: usize) {
        let table = facts(reader);
        if let Some(provenance) = table.get("provenance").and_then(Value::as_table) {
            for (object, value) in provenance {
                let Some(fields) = value.as_table() else {
                    continue;
                };
                let what = format!("{at}object `{object}`");
                self.add(
                    &what,
                    object.clone(),
                    fields,
                    None,
                    BTreeSet::from([what.clone()]),
                );
            }
        }
        let Ok(files) = reader.chunks(pfx_file::FILE) else {
            self.problems
                .push(format!("{at}: its embedded files cannot be read"));
            return;
        };
        let sums: BTreeMap<String, String> = files
            .iter()
            .map(|(name, bytes)| (sha256(bytes), format!("{at}{name}")))
            .collect();
        for (name, bytes) in &files {
            if !bytes.starts_with(&pfx_file::MAGIC) {
                continue;
            }
            let shown = format!("{at}{name}");
            let Ok(inner) = Reader::from_bytes(bytes) else {
                self.problems
                    .push(format!("{shown}: an embedded .pitofx that cannot be read"));
                continue;
            };
            if let Ok(record) = Record::from_reader(&inner) {
                let used: BTreeSet<String> = record
                    .files
                    .values()
                    .filter_map(|sha| sums.get(sha).cloned())
                    .collect();
                let used = if used.is_empty() {
                    BTreeSet::from([shown.clone()])
                } else {
                    used
                };
                let fields = facts(&inner);
                self.add(
                    &shown,
                    record.asset.name.clone(),
                    &fields,
                    Some(format!("{}:{}", record.asset.source, record.asset.id)),
                    used,
                );
                continue;
            }
            let fields = facts(&inner);
            let model = inner.find(pfx_file::GLTF).is_some();
            if model || fields.contains_key("licence") {
                let stem = name
                    .rsplit('/')
                    .next()
                    .unwrap_or(name)
                    .trim_end_matches(".pitofx")
                    .to_string();
                let label = field(&fields, "name").unwrap_or(stem);
                self.add(
                    &shown,
                    label,
                    &fields,
                    None,
                    BTreeSet::from([shown.clone()]),
                );
            } else if depth < DEPTH {
                self.container(&inner, &format!("{shown}/"), depth + 1);
            }
        }
    }
}

pub fn gather(reader: &Reader<'_>, own: &[&str]) -> Result<Vec<Credit>, Vec<String>> {
    let mut gather = Gather {
        own,
        credits: Vec::new(),
        problems: Vec::new(),
    };
    gather.container(reader, "", 0);
    if !gather.problems.is_empty() {
        gather.problems.sort();
        gather.problems.dedup();
        return Err(gather.problems);
    }
    let mut credits = gather.credits;
    credits.sort_by(|a, b| (&a.name, &a.credit).cmp(&(&b.name, &b.credit)));
    Ok(credits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_file::{Codec, Writer};

    const OWN: &str = "pfx's own original model";

    fn record_pfx(record: &Record) -> Vec<u8> {
        Writer::new()
            .chunk(pfx_file::FACT, record.facts().into_bytes(), Codec::Auto)
            .to_vec()
            .unwrap()
    }

    fn model_pfx(facts: &str) -> Vec<u8> {
        Writer::new()
            .chunk(pfx_file::GLTF, &b"{}"[..], Codec::Store)
            .chunk(pfx_file::FACT, facts.as_bytes(), Codec::Auto)
            .to_vec()
            .unwrap()
    }

    fn scene(files: &[(&str, Vec<u8>)], facts: &str) -> Vec<u8> {
        let mut writer = Writer::new().chunk(pfx_file::FACT, facts.as_bytes(), Codec::Auto);
        for (name, bytes) in files {
            writer = writer.named(pfx_file::FILE, *name, bytes.clone(), Codec::Auto);
        }
        writer.to_vec().unwrap()
    }

    #[test]
    fn a_scene_credits_its_fetched_assets_models_and_library_objects() {
        let hdr = b"#?RADIANCE sky".to_vec();
        let mut sky = crate::record::sample();
        sky.files = BTreeMap::from([("sky_01_1k.hdr".to_string(), sha256(&hdr))]);
        let facts = format!(
            "licence = \"MIT\"\n[provenance.bench]\nlicence = \"CC-BY-4.0\"\ncredit = \"Bench by B. Maker\"\n[provenance.wall]\nlicence = \"{OWN}\"\n"
        );
        let bytes = scene(
            &[
                ("assets/sky/asset.pitofx", record_pfx(&sky)),
                ("assets/sky/sky_01_1k.hdr", hdr),
                (
                    "library/prop/vase.pitofx",
                    model_pfx("name = \"vase\"\nlicence = \"CC0-1.0\"\nauthor = \"V. Potter\"\n"),
                ),
                ("textures/own.png", b"png".to_vec()),
            ],
            &facts,
        );
        let reader = Reader::from_bytes(&bytes).unwrap();
        let credits = gather(&reader, &[OWN]).unwrap();
        let names: Vec<&str> = credits.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Sky 01", "bench", "vase"]);
        let sky = &credits[0];
        assert_eq!(
            sky.credit,
            "\"Sky 01\" by A. Author is licensed under CC0 1.0"
        );
        assert_eq!(sky.from.as_deref(), Some("polyhaven:sky_01"));
        assert_eq!(
            sky.used_by,
            BTreeSet::from(["assets/sky/sky_01_1k.hdr".to_string()])
        );
        assert_eq!(credits[1].credit, "Bench by B. Maker");
        assert_eq!(
            credits[2].credit,
            "\"vase\" by V. Potter is licensed under CC0 1.0"
        );
    }

    #[test]
    fn unlicensed_assets_are_refused_by_name() {
        let bytes = scene(
            &[(
                "library/prop/mystery.pitofx",
                model_pfx("name = \"mystery\"\n"),
            )],
            "[provenance.chair]\nlicence = \"\"\n[provenance.lamp]\nlicence = \"Free\"\n",
        );
        let reader = Reader::from_bytes(&bytes).unwrap();
        let problems = gather(&reader, &[]).unwrap_err();
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(
            problems
                .iter()
                .any(|p| p.contains("object `chair`: unlicensed"))
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("object `lamp`") && p.contains("SPDX"))
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("library/prop/mystery.pitofx: unlicensed"))
        );
    }

    #[test]
    fn the_same_credit_used_twice_is_listed_once() {
        let a = model_pfx("name = \"tile\"\nlicence = \"CC0-1.0\"\ncredit = \"Tile, CC0\"\n");
        let bytes = scene(&[("a.pitofx", a.clone()), ("b/a.pitofx", a)], "");
        let reader = Reader::from_bytes(&bytes).unwrap();
        let credits = gather(&reader, &[]).unwrap();
        assert_eq!(credits.len(), 1);
        assert_eq!(credits[0].used_by.len(), 2);
    }
}
