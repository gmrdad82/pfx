use crate::paths::{dirname, join, normpath};
use regex::{Captures, Match, Regex};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const TAG: &str = r"<[A-Za-z][^>]*>";
const ATTR: &str =
    r#"(?i)\s(?:href|src)\s*=\s*(?:"([^"]*)"|'([^']*)'|\\"([^"\\]*)\\"|([^\s"'=<>`\\]+))"#;
const ID: &str =
    r#"(?i)\s(?:id|name)\s*=\s*(?:"([^"]*)"|'([^']*)'|\\"([^"\\]*)\\"|([^\s"'=<>`\\]+))"#;
const SCHEME: &str = r"^[A-Za-z][A-Za-z0-9+.-]*:(?:[^:]|$)";
const IMPL: &str = r#"<section id="impl-"#;
const HEADER: &str = r#"<h3 class="code-header">"#;
const DEREF: &str = r#"<h2 id="deref-methods-"#;
const TRAIT: &str = r#"class="trait" href=""#;
const IMPLEMENTORS: &str = "(function() {\n    const implementors = Object.fromEntries([]);\n    if (window.register_implementors) {\n        window.register_implementors(implementors);\n    } else {\n        window.pending_implementors = implementors;\n    }\n})()\n";

const NO_FILE: &str = "no such file in the bundle";
const NO_ANCHOR: &str = "no such anchor in the page";

pub struct Broken {
    pub page: String,
    pub target: String,
    pub why: &'static str,
}

pub struct Mended {
    pub scripts: usize,
    pub links: usize,
}

fn value<'t>(c: &Captures<'t>) -> Option<Match<'t>> {
    (1..=4).find_map(|group| c.get(group))
}

fn href_in<'t>(text: &'t str, marker: &str) -> Option<&'t str> {
    let rest = &text[text.find(marker)? + marker.len()..];
    rest.find('"').map(|end| &rest[..end])
}

fn regex(pattern: &str) -> Result<Regex, String> {
    Regex::new(pattern).map_err(|error| format!("pattern {pattern}: {error}"))
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
}

fn files_under(dir: &Path, base: &Path, out: &mut Vec<(PathBuf, String)>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, base, out)?;
        } else if let Ok(relative) = path.strip_prefix(base) {
            let name = relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push((path, name));
        }
    }
    Ok(())
}

fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    text.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn percent(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = bytes
            .get(at + 1..at + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[at], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                at += 3;
            }
            (byte, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

struct Checker {
    dir: PathBuf,
    pages: Vec<(PathBuf, String)>,
    files: HashSet<String>,
    tag: Regex,
    attr: Regex,
    id: Regex,
    scheme: Regex,
}

impl Checker {
    fn new(dir: &Path) -> Result<Self, String> {
        let mut all = Vec::new();
        files_under(dir, dir, &mut all)?;
        all.sort_by(|a, b| a.1.cmp(&b.1));
        let files = all.iter().map(|(_, name)| name.clone()).collect();
        Ok(Self {
            dir: dir.to_path_buf(),
            pages: all
                .into_iter()
                .filter(|(_, name)| name.ends_with(".html"))
                .collect(),
            files,
            tag: regex(TAG)?,
            attr: regex(ATTR)?,
            id: regex(ID)?,
            scheme: regex(SCHEME)?,
        })
    }

    fn links<'t>(&'t self, text: &'t str) -> impl Iterator<Item = (Match<'t>, String)> + 't {
        self.tag.find_iter(text).flat_map(move |tag| {
            self.attr
                .captures_iter(tag.as_str())
                .filter_map(|c| value(&c))
                .map(move |found| (tag, unescape(found.as_str().trim())))
        })
    }

    fn local(&self, raw: &str) -> bool {
        !(raw.is_empty() || raw == "#" || raw.starts_with("//") || self.scheme.is_match(raw))
    }

    fn target(&self, page: &str, path: &str) -> Option<String> {
        if path.is_empty() {
            return Some(page.to_string());
        }
        if path.starts_with('/') {
            return None;
        }
        let resolved = normpath(&join(dirname(page), &percent(path)));
        if resolved == ".." || resolved.starts_with("../") {
            return None;
        }
        if self.files.contains(&resolved) {
            return Some(resolved);
        }
        let index = normpath(&join(&resolved, "index.html"));
        self.files.contains(&index).then_some(index)
    }

    fn ids<'a>(
        &self,
        cache: &'a mut HashMap<String, HashSet<String>>,
        file: &str,
    ) -> Result<&'a HashSet<String>, String> {
        if !cache.contains_key(file) {
            let text = read(&self.dir.join(file))?;
            let found = self
                .tag
                .find_iter(&text)
                .flat_map(|tag| self.id.captures_iter(tag.as_str()))
                .filter_map(|c| value(&c))
                .map(|m| unescape(m.as_str()))
                .collect();
            cache.insert(file.to_string(), found);
        }
        Ok(&cache[file])
    }

    fn broken(
        &self,
        cache: &mut HashMap<String, HashSet<String>>,
        page: &str,
        raw: &str,
    ) -> Result<Option<&'static str>, String> {
        let (path, fragment) = raw.split_once('#').unwrap_or((raw, ""));
        let path = path.split_once('?').map_or(path, |(path, _)| path);
        let Some(file) = self.target(page, path) else {
            return Ok(Some(NO_FILE));
        };
        if fragment.is_empty() || !file.ends_with(".html") {
            return Ok(None);
        }
        let decoded = percent(fragment);
        let line = decoded
            .split_once('-')
            .map_or(decoded.as_str(), |(first, _)| first);
        let ids = self.ids(cache, &file)?;
        let lines = line.bytes().all(|b| b.is_ascii_digit()) && ids.contains(line);
        let found = ids.contains(fragment) || ids.contains(&decoded) || lines;
        Ok((!found).then_some(NO_ANCHOR))
    }

    fn check_pages(&self, pages: &[(PathBuf, String)]) -> Result<Vec<Broken>, String> {
        let mut cache = HashMap::new();
        let mut broken = Vec::new();
        for (path, page) in pages {
            let text = read(path)?;
            let mut seen: HashSet<String> = HashSet::new();
            for (_, raw) in self.links(&text) {
                if !self.local(&raw) || !seen.insert(raw.clone()) {
                    continue;
                }
                if let Some(why) = self.broken(&mut cache, page, &raw)? {
                    broken.push(Broken {
                        page: page.clone(),
                        target: raw,
                        why,
                    });
                }
            }
        }
        Ok(broken)
    }

    fn foreign(&self, href: Option<&str>) -> bool {
        href.is_none_or(|href| self.scheme.is_match(href))
    }

    fn foreign_spans(&self, text: &str) -> Vec<(usize, usize)> {
        let mut starts: Vec<usize> = text
            .match_indices(IMPL)
            .chain(text.match_indices(DEREF))
            .map(|(at, _)| at)
            .collect();
        starts.sort_unstable();
        let mut spans = Vec::new();
        for (index, &start) in starts.iter().enumerate() {
            let next = starts.get(index + 1).copied().unwrap_or(text.len());
            let end = text[start + 1..next]
                .find("<h2 ")
                .map_or(next, |cut| start + 1 + cut);
            let span = &text[start..end];
            let source = if span.starts_with(DEREF) {
                let header = &span[..span.find("</h2>").unwrap_or(span.len())];
                let Some((_, target)) = header.split_once("Target = ") else {
                    continue;
                };
                href_in(target, "href=\"")
            } else {
                let Some(header) = span.find(HEADER).map(|at| &span[at + HEADER.len()..]) else {
                    continue;
                };
                let header = &header[..header.find("</h3>").unwrap_or(header.len())];
                let Some((head, _)) = header.split_once(" for ") else {
                    continue;
                };
                head.rfind(TRAIT).and_then(|at| href_in(&head[at..], TRAIT))
            };
            if self.foreign(source) {
                spans.push((start, end));
            }
        }
        spans
    }

    fn mend_page(
        &self,
        cache: &mut HashMap<String, HashSet<String>>,
        page: &str,
        text: &str,
    ) -> Result<Option<(String, usize)>, String> {
        let spans = self.foreign_spans(text);
        let mut cuts: Vec<(usize, usize)> = Vec::new();
        for (tag, raw) in self.links(text) {
            let at = tag.start();
            if !tag.as_str().starts_with("<a ")
                || !spans.iter().any(|&(start, end)| start <= at && at < end)
                || !self.local(&raw)
                || self.broken(cache, page, &raw)?.is_none()
            {
                continue;
            }
            if let Some(close) = text[tag.end()..].find("</a>") {
                cuts.push((at, tag.end()));
                cuts.push((tag.end() + close, tag.end() + close + "</a>".len()));
            }
        }
        if cuts.is_empty() {
            return Ok(None);
        }
        cuts.sort_unstable();
        cuts.dedup();
        let mut out = String::with_capacity(text.len());
        let mut from = 0;
        for &(start, end) in &cuts {
            out.push_str(&text[from..start]);
            from = end;
        }
        out.push_str(&text[from..]);
        Ok(Some((out, cuts.len() / 2)))
    }
}

fn split<T: Send>(
    pages: &[(PathBuf, String)],
    work: impl Fn(&[(PathBuf, String)]) -> Result<Vec<T>, String> + Sync,
) -> Result<Vec<T>, String> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let size = pages.len().div_ceil(threads).max(1);
    let parts: Vec<Result<Vec<T>, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = pages
            .chunks(size)
            .map(|chunk| scope.spawn(|| work(chunk)))
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err("a link-check thread panicked".to_string()))
            })
            .collect()
    });
    let mut out = Vec::new();
    for part in parts {
        out.extend(part?);
    }
    Ok(out)
}

pub fn check(dir: &Path) -> Result<Vec<Broken>, String> {
    let checker = Checker::new(dir)?;
    split(&checker.pages, |chunk| checker.check_pages(chunk))
}

pub fn mend(dir: &Path) -> Result<Mended, String> {
    let mut checker = Checker::new(dir)?;
    let mut stubs: Vec<String> = Vec::new();
    for (path, page) in &checker.pages {
        for (tag, raw) in checker.links(&read(path)?) {
            if !tag.as_str().starts_with("<script") || !checker.local(&raw) {
                continue;
            }
            let resolved = normpath(&join(dirname(page), &percent(&raw)));
            if resolved.starts_with("trait.impl/")
                && resolved.ends_with(".js")
                && !checker.files.contains(&resolved)
                && !stubs.contains(&resolved)
            {
                stubs.push(resolved);
            }
        }
    }
    for stub in &stubs {
        let full = dir.join(stub);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        std::fs::write(&full, IMPLEMENTORS)
            .map_err(|error| format!("{}: {error}", full.display()))?;
        checker.files.insert(stub.clone());
    }
    let links = split(&checker.pages, |chunk| {
        let mut cache = HashMap::new();
        let mut mended = Vec::new();
        for (path, page) in chunk {
            let text = read(path)?;
            if let Some((out, count)) = checker.mend_page(&mut cache, page, &text)? {
                std::fs::write(path, out)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                mended.push(count);
            }
        }
        Ok(mended)
    })?;
    Ok(Mended {
        scripts: stubs.len(),
        links: links.into_iter().sum(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/docs-bundle-links")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("api/a")).unwrap();
        dir
    }

    fn page(dir: &Path, name: &str, body: &str) {
        std::fs::write(
            dir.join(name),
            format!("<!doctype html><html><body>{body}</body></html>"),
        )
        .unwrap();
    }

    fn found(dir: &Path) -> Vec<(String, String)> {
        check(dir)
            .unwrap()
            .into_iter()
            .map(|b| (b.page, b.target))
            .collect()
    }

    #[test]
    fn a_clean_pair_passes() {
        let dir = scratch("clean");
        page(
            &dir,
            "api/a/one.html",
            r##"<h1 id="top">one</h1><a href="two.html#method.year">year</a> <a href="../a/">up</a> <a href="https://example.com/x.html#nope">out</a> <a href="#top">top</a> <img src="two.html"><pre>let src = load(x);</pre>"##,
        );
        page(
            &dir,
            "api/a/two.html",
            r#"<h1 id=top>t</h1><section id="method.year"></section><a href='one.html?search=x#top'>back</a><a href=#12-14>lines</a><a href=#12 id=12>12</a>"#,
        );
        page(&dir, "api/a/index.html", r#"<a href="one.html">one</a>"#);
        assert!(found(&dir).is_empty(), "{:?}", found(&dir));
    }

    #[test]
    fn a_broken_file_link_and_a_broken_anchor_fail() {
        let dir = scratch("broken");
        page(
            &dir,
            "api/a/one.html",
            r#"<a href="./struct.NaiveDate.html#method.and_time">gone</a><a href="super::Subscriber">path</a>"#,
        );
        page(
            &dir,
            "api/a/two.html",
            r#"<span id="calendar"></span><a href="one.html#calendar-date">missing</a>"#,
        );
        assert_eq!(
            found(&dir),
            vec![
                (
                    "api/a/one.html".to_string(),
                    "./struct.NaiveDate.html#method.and_time".to_string()
                ),
                (
                    "api/a/one.html".to_string(),
                    "super::Subscriber".to_string()
                ),
                (
                    "api/a/two.html".to_string(),
                    "one.html#calendar-date".to_string()
                ),
            ]
        );
    }

    #[test]
    fn mending_unlinks_only_an_undocumented_traits_impl_and_stubs_its_scripts() {
        let dir = scratch("mend");
        page(
            &dir,
            "api/a/struct.S.html",
            r##"<script src="../trait.impl/a/trait.T.js" async></script><h2 id="impls">Impls</h2><section id="impl-Mine-for-T" class="impl"><h3 class="code-header">impl&lt;T&gt; <a class="trait" href="trait.Mine.html">Mine</a> for T</h3></section><div class='docblock'><a href="dispatcher#own">own</a></div><section id="impl-Theirs-for-T" class="impl"><h3 class="code-header">impl&lt;T&gt; Theirs for T</h3></section><div class='docblock'>The <a href="dispatcher#default">default</a> and <a href="#impls">here</a></div><h2 id="deref-methods-Map" class="section-header"><span>Methods from <a class="trait" href="https://doc.rust-lang.org/core/ops/trait.Deref.html">Deref</a>&lt;Target = <a class="struct" href="https://doc.rust-lang.org/alloc/struct.Map.html">Map</a>&gt;</span></h2><div class="docblock">See <a href="index.html#keys">keys</a></div><h2 id="next">Next</h2><section id="impl-S" class="impl"><h3 class="code-header">impl S</h3></section><a href="gone.html">gone</a>"##,
        );
        page(&dir, "api/a/trait.Mine.html", "");
        let mended = mend(&dir.join("api")).unwrap();
        assert_eq!((mended.scripts, mended.links), (1, 2));
        let text = std::fs::read_to_string(dir.join("api/a/struct.S.html")).unwrap();
        assert!(text.contains("The default and <a href=\"#impls\">here</a>"));
        assert!(text.contains("See keys</div>"));
        assert_eq!(
            found(&dir),
            vec![
                (
                    "api/a/struct.S.html".to_string(),
                    "dispatcher#own".to_string()
                ),
                ("api/a/struct.S.html".to_string(), "gone.html".to_string()),
            ]
        );
    }
}
