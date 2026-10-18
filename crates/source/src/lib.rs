pub mod ambientcg;
pub mod catalog;
pub mod credits;
pub mod fetch;
pub mod http;
pub mod licence;
pub mod polyhaven;
pub mod record;
pub mod secret;
pub mod settings;
pub mod sketchfab;
pub mod zip;

use std::io::Write;
use std::path::PathBuf;

use serde_json::Value;

pub use record::{RECORD, Record};
pub use settings::Settings;

pub const USER_AGENT: &str = concat!("pfx/", env!("CARGO_PKG_VERSION"));

pub type Headers = Vec<(String, String)>;

pub trait Net {
    fn fetch(&self, url: &str, headers: &Headers, sink: &mut dyn Write) -> Result<u64, String>;

    fn get(&self, url: &str, headers: &Headers) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        self.fetch(url, headers, &mut out)?;
        Ok(out)
    }

    fn json(&self, url: &str, headers: &Headers) -> Result<Value, String> {
        let bytes = self.get(url, headers)?;
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: not JSON: {e}", shown(url)))
    }
}

pub fn shown(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Hdri,
    Texture,
    Model,
    Pack,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Hdri, Kind::Texture, Kind::Model, Kind::Pack];

    pub fn word(self) -> &'static str {
        match self {
            Kind::Hdri => "hdri",
            Kind::Texture => "texture",
            Kind::Model => "model",
            Kind::Pack => "pack",
        }
    }

    pub fn parse(word: &str) -> Result<Kind, String> {
        Kind::ALL
            .into_iter()
            .find(|kind| kind.word() == word || format!("{}s", kind.word()) == word)
            .ok_or_else(|| {
                format!(
                    "unknown asset kind `{word}`, expected one of: {}",
                    Kind::ALL.map(Kind::word).join(", ")
                )
            })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub kind: Option<Kind>,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub source: String,
    pub id: String,
    pub name: String,
    pub kind: Option<Kind>,
    pub author: Option<String>,
    pub licence: Option<String>,
    pub url: Option<String>,
}

impl Listing {
    pub fn json(&self) -> Value {
        serde_json::json!({
            "ref": format!("{}:{}", self.source, self.id),
            "source": self.source,
            "id": self.id,
            "name": self.name,
            "kind": self.kind.map(Kind::word),
            "author": self.author,
            "licence": self.licence,
            "url": self.url,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Want {
    pub resolution: Option<String>,
    pub format: Option<String>,
    pub from: Option<PathBuf>,
    pub policy: licence::Policy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Url(String),
    File(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    pub origin: Origin,
    pub path: String,
    pub size: Option<u64>,
    pub md5: Option<String>,
    pub sha256: Option<String>,
    pub unpack: bool,
    pub headers: Headers,
}

impl Download {
    pub fn url(url: impl Into<String>, path: impl Into<String>) -> Download {
        Download {
            origin: Origin::Url(url.into()),
            path: path.into(),
            size: None,
            md5: None,
            sha256: None,
            unpack: false,
            headers: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub source: String,
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub author: Option<String>,
    pub licence: String,
    pub credit: String,
    pub url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub asset: Asset,
    pub downloads: Vec<Download>,
}

pub trait Source {
    fn name(&self) -> &str;

    fn about(&self) -> &str;

    fn powered_by(&self) -> Option<&'static str> {
        None
    }

    fn search(&self, net: &dyn Net, query: &Query) -> Result<Vec<Listing>, String>;

    fn plan(&self, net: &dyn Net, id: &str, want: &Want) -> Result<Plan, String>;
}

pub fn builtin() -> Vec<Box<dyn Source>> {
    vec![
        Box::new(polyhaven::PolyHaven),
        Box::new(ambientcg::AmbientCg),
        Box::new(catalog::Catalog::kenney()),
        Box::new(catalog::Catalog::quaternius()),
    ]
}

pub fn sources(settings: &Settings) -> Vec<Box<dyn Source>> {
    let mut out = builtin();
    for account in &settings.accounts {
        out.push(account.source());
    }
    out
}

pub fn split_ref(text: &str) -> Result<(&str, &str), String> {
    match text.split_once(':') {
        Some((source, id)) if !source.is_empty() && !id.is_empty() => Ok((source, id)),
        _ => Err(format!(
            "expected <source>:<id>, as polyhaven:<id>, found \"{text}\""
        )),
    }
}

pub fn check_id(id: &str) -> Result<(), String> {
    let fine = !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
    if fine {
        Ok(())
    } else {
        Err(format!(
            "`{id}` is not an asset id: expected letters, digits, `_`, `-` and `.`"
        ))
    }
}

pub fn find<'a>(sources: &'a [Box<dyn Source>], name: &str) -> Result<&'a dyn Source, String> {
    sources
        .iter()
        .find(|s| s.name() == name)
        .map(|s| s.as_ref())
        .ok_or_else(|| {
            let names: Vec<&str> = sources.iter().map(|s| s.name()).collect();
            format!(
                "no source is named `{name}`, expected one of: {}",
                names.join(", ")
            )
        })
}

pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn text_of(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
pub(crate) mod mock {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub struct Mock {
        pub pages: BTreeMap<String, Vec<u8>>,
        pub asked: RefCell<Vec<(String, Headers)>>,
    }

    impl Mock {
        pub fn with(mut self, url: &str, body: impl Into<Vec<u8>>) -> Mock {
            self.pages.insert(url.to_string(), body.into());
            self
        }
    }

    pub fn scratch(label: &str) -> PathBuf {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/source-tests")
            .join(format!("{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    impl Net for Mock {
        fn fetch(&self, url: &str, headers: &Headers, sink: &mut dyn Write) -> Result<u64, String> {
            self.asked
                .borrow_mut()
                .push((url.to_string(), headers.clone()));
            let body = self
                .pages
                .get(url)
                .ok_or_else(|| format!("{url}: 404 Not Found"))?;
            sink.write_all(body).map_err(|e| e.to_string())?;
            Ok(body.len() as u64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_and_ids_are_checked_before_any_request() {
        assert_eq!(split_ref("polyhaven:sky_01"), Ok(("polyhaven", "sky_01")));
        assert!(split_ref("sky_01").is_err());
        assert!(split_ref(":x").is_err());
        assert!(check_id("Rock_064-a.b").is_ok());
        for bad in ["", "../up", "a/b", "a?b=c", ".hidden", "a b"] {
            assert!(check_id(bad).is_err(), "{bad}");
        }
        assert_eq!(encode("red brick & tile"), "red%20brick%20%26%20tile");
    }

    #[test]
    fn every_builtin_source_has_a_distinct_name() {
        let all = builtin();
        let mut names: Vec<&str> = all.iter().map(|s| s.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all.len());
        assert!(find(&all, "polyhaven").is_ok());
        assert!(find(&all, "nowhere").err().unwrap().contains("polyhaven"));
    }
}
