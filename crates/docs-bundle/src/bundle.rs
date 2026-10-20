use crate::lists::{FLAG_GROUPS, FORMATS, GUIDES, MOVED, SKIP};
use crate::paths::{dirname, join, normpath, relpath};
use regex::{Captures, Regex};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

const REPO: &str = "https://github.com/gmrdad82/pfx";

const INDEX_LEAD: &str = "Install it and make a first game: [Install](bundle:install.md). Every command of the `pfx` tool: [The pfx tool](bundle:reference/cli.md).";
const CRATES_LEAD: &str = "Each is a package in this workspace that a game or app can link by tag:";
const CRATES_LEAD_API: &str = "Each is a package in this workspace that a game or app can link by tag; its name links to its API reference ([all crates](bundle:api/index.html)):";
const CRATES_TAIL: &str = "\n\nEvery command and flag: [The pfx tool](bundle:reference/cli.md).";
const NEXT_STEPS: &str = "## Next steps\n\n- [Make your first game](docs/first-game.md): scaffold, play, edit and bench a game.\n- [Benchmark a scene](docs/benchmark-a-scene.md): a repeatable headless load with frame stats.\n- [Render a still with the path tracer](docs/render-a-still.md): an offline frame, traced and denoised.\n- [The pfx tool](bundle:reference/cli.md): every command and flag.";
const INSTALL_DESCRIPTION: &str = "Install the pfx tool on Linux, Windows or macOS, make a game and play it, and link the engine's crates from a game that builds on Linux, Windows and macOS.";
const FORMATS_LEAD: &str = "pfx promises the JSON outputs and file formats below; each section is taken from the guide that owns it, which says more. A schema version or format number changes only with its page.";
const FORMATS_DESCRIPTION: &str = "The JSON outputs and file formats pfx promises: scenes and projects, material libraries, frame stats, the bench summary, input journals and recordings, Steam depots and baked atlases.";
const FLAGS_LEAD: &str = "## Flags\n\nEach mode takes the flags below and refuses any other. A switch takes no value; any other flag takes one, as `--flag value` or `--flag=value`. `new`, `gui`, `bench`, `scene`, `font subset` and `archive` take what their usage lines above show.";
const ERRORS: &str = "## Errors\n\nInput the tool can't parse gets clap's answer on stderr with exit 2: the error, the command's usage line and `For more information, try '--help'.` A refusal after a successful parse is one line, `pfx: …`, with exit 1. `pfx bench`'s flags and summary are in [Benchmark a scene](bundle:guides/benchmark-a-scene.md) and [Frame stats](bundle:guides/frame-stats.md#bench); `pfx shot`'s in [Render a scene](bundle:guides/render-a-scene.md).";
const CLI_DESCRIPTION: &str =
    "Every command and flag of the pfx tool, generated from its usage text.";

pub fn fail(message: &str) -> ! {
    eprintln!("docs-bundle: {message}");
    std::process::exit(1)
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|error| fail(&format!("pattern {pattern}: {error}")))
}

struct Patterns {
    image: Regex,
    link_text: Regex,
    space: Regex,
    code_page: Regex,
    scheme: Regex,
    github: Regex,
    link: Regex,
    img: Regex,
    code_doc: Regex,
    page_name: Regex,
    picture: Regex,
    code: Regex,
    centre_open: Regex,
    centre_close: Regex,
    centre_image: Regex,
    slug: Regex,
    crate_row: Regex,
    blank_lines: Regex,
    target: Regex,
    shape: Regex,
    quoted: Regex,
}

impl Patterns {
    fn new() -> Self {
        Self {
            image: regex(r"!\[[^\]]*\]\([^)]*\)"),
            link_text: regex(r"\[([^\]]*)\]\([^)]*\)"),
            space: regex(r"\s+"),
            code_page: regex(r"`docs/([a-z0-9-]+)\.md`"),
            scheme: regex(r"^[a-z]+:"),
            github: regex(r"^https://github\.com/gmrdad82/([A-Za-z0-9_.-]+)"),
            link: regex(r"(!?)\[((?:[^\[\]]|\[[^\]]*\])*)\]\(([^)\s]+)\)"),
            img: regex(r"<img\s[^>]*>"),
            code_doc: regex(r"\A`(docs/[a-z0-9-]+\.md)`\z"),
            page_name: regex(r"\A(docs/)?[a-z0-9-]+\.md\z"),
            picture: regex(r"(?s)<picture>.*?</picture>\n*"),
            code: regex(r"`[^`]*`"),
            centre_open: regex(r#"(?m)^<p align="center">\s*$\n?"#),
            centre_close: regex(r"(?m)^</p>\s*$\n?"),
            centre_image: regex(r#"(?m)^<p align="center">(!\[.*\]\(.*\))</p>$"#),
            slug: regex(r"[^\w\- ]"),
            crate_row: regex(r"(?m)^\| `(pfx-[a-z-]+)`"),
            blank_lines: regex(r"\n{3,}"),
            target: regex(r"\]\(([^)\s]+)\)"),
            shape: regex(r#""([a-z]+)" => \{?\s*"([^"]+)""#),
            quoted: regex(r#""([^"]*)""#),
        }
    }
}

type Section = (String, usize, Vec<String>);

fn heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if (2..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
        Some((hashes, &line[hashes + 1..]))
    } else {
        None
    }
}

fn sections(text: &str) -> Vec<Section> {
    let mut found: Vec<Section> = Vec::new();
    let mut fenced = false;
    for line in text.split('\n') {
        if line.starts_with("```") {
            fenced = !fenced;
        }
        if !fenced && let Some((level, name)) = heading(line) {
            found.push((name.to_string(), level, Vec::new()));
        }
        if let Some(current) = found.last_mut() {
            current.2.push(line.to_string());
        }
    }
    found
}

fn strip_title(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(index) = lines.iter().position(|line| line.starts_with("# ")) else {
        return text.to_string();
    };
    let mut rest = &lines[index + 1..];
    while let Some(first) = rest.first()
        && first.trim().is_empty()
    {
        rest = &rest[1..];
    }
    lines[..index]
        .iter()
        .chain(rest)
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
}

fn demote(text: &str, level: usize, to: usize) -> String {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in text.split('\n') {
        if line.starts_with("```") {
            fenced = !fenced;
        }
        match heading(line).filter(|_| !fenced) {
            Some((hashes, name)) => {
                let depth = (hashes + to).saturating_sub(level).min(6);
                out.push(format!("{} {name}", "#".repeat(depth)));
            }
            None => out.push(line.to_string()),
        }
    }
    out.join("\n")
}

fn sentence(body: &str) -> Option<&str> {
    let chars: Vec<(usize, char)> = body.char_indices().collect();
    for (k, &(at, c)) in chars.iter().enumerate().skip(1) {
        if !matches!(c, '.' | '!' | '?') {
            continue;
        }
        let before = &body[..at];
        if before.ends_with("e.g") || before.ends_with("i.e") {
            continue;
        }
        if chars
            .get(k + 1)
            .is_none_or(|&(_, next)| next.is_whitespace())
        {
            return Some(&body[..at + c.len_utf8()]);
        }
    }
    None
}

fn attr(tag: &str, name: &str) -> String {
    let pattern = regex(&format!("{}=\"([^\"]*)\"", regex::escape(name)));
    pattern
        .captures(tag)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

fn json(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|error| fail(&error.to_string()))
}

fn front(title: &str, description: &str, order: u32) -> String {
    format!(
        "---\ntitle: {}\ndescription: {}\nsidebar:\n  order: {order}\n---\n\n",
        json(title),
        json(description)
    )
}

fn link_to(page_out: &str, target_out: &str) -> String {
    let start = dirname(page_out);
    relpath(target_out, if start.is_empty() { "." } else { start })
}

fn changelog_section<'a>(changelog: &'a str, tag: &str) -> Option<&'a str> {
    let mut start = 0;
    let found = loop {
        let line_end = changelog[start..].find('\n').map(|at| start + at);
        let line = &changelog[start..line_end.unwrap_or(changelog.len())];
        if let Some(after) = line.strip_prefix(&format!("## {tag}"))
            && after
                .chars()
                .next()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
        {
            break line_end? + 1;
        }
        start = line_end? + 1;
    };
    let mut at = found;
    loop {
        if changelog[at..].starts_with("## v") {
            return Some(&changelog[found..at]);
        }
        match changelog[at..].find('\n') {
            Some(next) => at += next + 1,
            None => return Some(&changelog[found..]),
        }
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|error| fail(&format!("{}: {error}", dir.display())));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

pub struct Bundle {
    root: PathBuf,
    dest: PathBuf,
    tag: String,
    api: bool,
    crates: Vec<String>,
    pages: HashMap<String, String>,
    titles: HashMap<String, String>,
    re: Patterns,
}

impl Bundle {
    pub fn new(root: &Path, dest: &Path, tag: &str, api: bool, crates: Vec<String>) -> Self {
        Self {
            root: root.to_path_buf(),
            dest: dest.to_path_buf(),
            tag: tag.to_string(),
            api,
            crates,
            pages: HashMap::new(),
            titles: HashMap::new(),
            re: Patterns::new(),
        }
    }

    fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.root.join(path))
            .unwrap_or_else(|error| fail(&format!("{path}: {error}")))
    }

    fn write(&self, path: &str, text: &str) {
        let full = self.dest.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)
                .unwrap_or_else(|error| fail(&format!("{}: {error}", parent.display())));
        }
        std::fs::write(&full, text)
            .unwrap_or_else(|error| fail(&format!("{}: {error}", full.display())));
    }

    fn plain(&self, text: &str) -> String {
        let text = self.re.image.replace_all(text, "");
        let text = self.re.link_text.replace_all(&text, "$1");
        let text = text.replace('`', "").replace("**", "");
        self.re.space.replace_all(&text, " ").trim().to_string()
    }

    fn heading_title(&self, text: &str) -> String {
        match text.lines().find_map(|line| line.strip_prefix("# ")) {
            Some(title) => self.plain(title),
            None => fail("a page has no title heading"),
        }
    }

    fn first_sentence(&self, text: &str) -> String {
        let mut para: Vec<&str> = Vec::new();
        let mut fenced = false;
        for line in text.lines() {
            if line.starts_with("```") {
                fenced = !fenced;
                continue;
            }
            if fenced || ["#", "|", "<", "!["].iter().any(|p| line.starts_with(p)) {
                if !para.is_empty() {
                    break;
                }
                continue;
            }
            if line.trim().is_empty() {
                if !para.is_empty() {
                    break;
                }
                continue;
            }
            para.push(line.trim());
        }
        let joined = para.join(" ");
        let body = self.re.code_page.replace_all(&joined, |c: &Captures| {
            self.titles
                .get(&format!("docs/{}.md", &c[1]))
                .cloned()
                .unwrap_or_else(|| c[0].to_string())
        });
        let body = self.plain(&body);
        sentence(&body).map(str::to_string).unwrap_or(body)
    }

    fn resolve(
        &self,
        target: &str,
        source: &str,
        page_out: &str,
        moved_from: Option<&str>,
    ) -> String {
        if let Some(rest) = target.strip_prefix("bundle:") {
            let (path, anchor) = rest.split_once('#').unwrap_or((rest, ""));
            let suffix = if anchor.is_empty() {
                String::new()
            } else {
                format!("#{anchor}")
            };
            return link_to(page_out, path) + &suffix;
        }
        if self.re.scheme.is_match(target) {
            if let Some(c) = self.re.github.captures(target)
                && &c[1] != "pfx"
            {
                return format!("https://docs.pitomd.com/{}/", c[1].to_lowercase());
            }
            return target.to_string();
        }
        let (path, anchor) = target.split_once('#').unwrap_or((target, ""));
        if path.is_empty() {
            if let Some(from) = moved_from {
                return link_to(page_out, &self.pages[from]) + "#" + anchor;
            }
            return target.to_string();
        }
        let repo_path = normpath(&join(dirname(source), path));
        let suffix = if anchor.is_empty() {
            String::new()
        } else {
            format!("#{anchor}")
        };
        if repo_path == "README.md"
            && let Some((_, to)) = MOVED.iter().find(|(from, _)| *from == anchor)
        {
            return link_to(page_out, to);
        }
        if let Some(out) = self.pages.get(&repo_path) {
            return link_to(page_out, out) + &suffix;
        }
        if let Some(image) = repo_path.strip_prefix("docs/images/") {
            return link_to(page_out, &format!("images/{image}"));
        }
        if repo_path.starts_with("..") {
            fail(&format!("{source} links outside the repository: {target}"));
        }
        let full = self.root.join(&repo_path);
        let kind = if full.is_dir() { "tree" } else { "blob" };
        if !full.exists() {
            fail(&format!("{source} links to a missing file: {target}"));
        }
        format!("{REPO}/{kind}/{}/{repo_path}{suffix}", self.tag)
    }

    fn label(&self, text: &str, target: &str, source: &str) -> String {
        let path = target.split_once('#').map_or(target, |(path, _)| path);
        if self.re.page_name.is_match(text) && !path.is_empty() {
            let repo_path = normpath(&join(dirname(source), path));
            if repo_path.starts_with("docs/")
                && let Some(title) = self.titles.get(&repo_path)
            {
                return title.clone();
            }
        }
        text.to_string()
    }

    fn spans<'a>(&self, text: &'a str) -> Vec<(&'a str, bool)> {
        let mut out = Vec::new();
        let mut last = 0;
        for m in self.re.code.find_iter(text) {
            out.push((&text[last..m.start()], false));
            out.push((m.as_str(), true));
            last = m.end();
        }
        out.push((&text[last..], false));
        out
    }

    fn rewrite_prose(
        &self,
        text: &str,
        source: &str,
        page_out: &str,
        moved_from: Option<&str>,
    ) -> String {
        let text = self.re.picture.replace_all(text, "");
        let text = self.re.img.replace_all(&text, |c: &Captures| {
            let alt = attr(&c[0], "alt").replace('[', "(").replace(']', ")");
            format!("![{alt}]({})", attr(&c[0], "src"))
        });
        let text: String = self
            .spans(&text)
            .into_iter()
            .map(|(part, code)| {
                if code && let Some(c) = self.re.code_doc.captures(part) {
                    let path = &c[1];
                    if self.pages.contains_key(path)
                        && let Some(title) = self.titles.get(path)
                    {
                        let link = if source.starts_with("docs/") {
                            &path["docs/".len()..]
                        } else {
                            path
                        };
                        return format!("[{title}]({link})");
                    }
                }
                part.to_string()
            })
            .collect();
        self.spans(&text)
            .into_iter()
            .map(|(part, code)| {
                if code {
                    return part.to_string();
                }
                self.re
                    .link
                    .replace_all(part, |c: &Captures| {
                        format!(
                            "{}[{}]({})",
                            &c[1],
                            self.label(&c[2], &c[3], source),
                            self.resolve(&c[3], source, page_out, moved_from)
                        )
                    })
                    .into_owned()
            })
            .collect()
    }

    fn rewrite(
        &self,
        text: &str,
        source: &str,
        page_out: &str,
        moved_from: Option<&str>,
    ) -> String {
        let mut out: Vec<String> = Vec::new();
        let mut fenced = false;
        let mut block: Vec<&str> = Vec::new();
        for line in text.split('\n') {
            if line.trim_start().starts_with("```") {
                if !fenced {
                    out.push(self.rewrite_prose(&block.join("\n"), source, page_out, moved_from));
                    block.clear();
                }
                fenced = !fenced;
                out.push(line.to_string());
                continue;
            }
            if fenced {
                out.push(line.to_string());
            } else {
                block.push(line);
            }
        }
        out.push(self.rewrite_prose(&block.join("\n"), source, page_out, moved_from));
        let text = out.join("\n");
        let text = self.re.centre_open.replace_all(&text, "");
        let text = self.re.centre_close.replace_all(&text, "");
        self.re.centre_image.replace_all(&text, "$1").into_owned()
    }

    fn section(&self, text: &str, wanted: Option<&str>) -> (String, usize) {
        let Some(wanted) = wanted else {
            let stripped = strip_title(text);
            let kept: Vec<&str> = stripped
                .split('\n')
                .take_while(|line| heading(line).is_none())
                .collect();
            return (kept.join("\n").trim_matches('\n').to_string(), 2);
        };
        let found = sections(text);
        for (index, (name, level, body)) in found.iter().enumerate() {
            if self.plain(name) == wanted {
                let mut collected = body.clone();
                for (_, later_level, later_body) in &found[index + 1..] {
                    if later_level <= level {
                        break;
                    }
                    collected.extend(later_body.iter().cloned());
                }
                return (collected.join("\n").trim_matches('\n').to_string(), *level);
            }
        }
        fail(&format!(
            "no section '{wanted}' to take into the formats page"
        ))
    }

    fn anchor(&self, heading: &str) -> String {
        let slug = self.plain(heading).to_lowercase();
        self.re.slug.replace_all(&slug, "").replace(' ', "-")
    }

    fn rust_str(&self, name: &str, text: &str) -> String {
        let pattern = regex(&format!(
            r#"(?s)const {}: &str = "((?:[^"\\]|\\.)*)";"#,
            regex::escape(name)
        ));
        match pattern.captures(text) {
            Some(c) => c[1].replace("\\\"", "\"").replace("\\\\", "\\"),
            None => fail(&format!("src/main.rs has no {name} string")),
        }
    }

    fn rust_list(&self, name: &str, text: &str) -> Vec<String> {
        let pattern = regex(&format!(
            r"(?s)const {}: &\[&str\] = &\[(.*?)\];",
            regex::escape(name)
        ));
        match pattern.captures(text) {
            Some(c) => self
                .re
                .quoted
                .captures_iter(&c[1])
                .map(|q| q[1].to_string())
                .collect(),
            None => fail(&format!("src/main.rs has no {name} list")),
        }
    }

    pub fn write_all(mut self, notes: &Path) {
        let docs_dir = self.root.join("docs");
        let mut docs: Vec<String> = std::fs::read_dir(&docs_dir)
            .unwrap_or_else(|error| fail(&format!("{}: {error}", docs_dir.display())))
            .flatten()
            .filter_map(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.strip_suffix(".md"))
                    .map(str::to_string)
            })
            .collect();
        docs.sort();
        let listed: Vec<&str> = GUIDES.iter().map(|(name, _, _)| *name).collect();
        let missing: Vec<String> = docs
            .iter()
            .filter(|name| !listed.contains(&name.as_str()) && !SKIP.contains(&name.as_str()))
            .map(|name| format!("docs/{name}.md"))
            .collect();
        if !missing.is_empty() {
            fail(&format!(
                "{} not placed in the bundle's GUIDES list (crates/docs-bundle/src/lists.rs)",
                missing.join(", ")
            ));
        }
        let absent: Vec<&str> = listed
            .iter()
            .filter(|name| !docs.iter().any(|doc| doc == *name))
            .copied()
            .collect();
        if !absent.is_empty() {
            fail(&format!(
                "GUIDES names pages that do not exist: {}",
                absent.join(", ")
            ));
        }

        for (from, to) in [
            ("README.md", "index.md"),
            ("CHANGELOG.md", "changelog.md"),
            ("docs/format.md", "reference/formats.md"),
            ("docs/formats.md", "reference/formats.md"),
        ] {
            self.pages.insert(from.to_string(), to.to_string());
        }
        for name in &listed {
            self.pages
                .insert(format!("docs/{name}.md"), format!("guides/{name}.md"));
        }
        for name in &listed {
            let source = format!("docs/{name}.md");
            let title = self.heading_title(&self.read(&source));
            self.titles.insert(source, title);
        }
        for (from, title) in [
            ("README.md", "pfx"),
            ("CHANGELOG.md", "Changelog"),
            ("docs/format.md", "Formats"),
            ("docs/formats.md", "Formats"),
        ] {
            self.titles.insert(from.to_string(), title.to_string());
        }

        self.index_and_install();
        self.guides();
        self.formats();
        self.cli();
        self.changelog(notes);
        if self.api {
            self.api_index();
        }
        self.check_links_and_copy_images();
    }

    fn index_and_install(&self) {
        let readme = self.read("README.md");
        let blocks = sections(&readme);
        let top = readme.split("\n## ").next().unwrap_or_default();
        let top_lines: Vec<&str> = top
            .split('\n')
            .filter(|line| !line.starts_with("# ") && !line.starts_with("[!["))
            .collect();
        let joined = top_lines.join("\n");
        let top = self.re.blank_lines.replace_all(&joined, "\n\n");
        let top = top.trim_matches('\n');
        let mut by_name: HashMap<&str, String> = HashMap::new();
        for (name, level, body) in &blocks {
            if *level == 2 {
                by_name.insert(name, body.join("\n").trim_matches('\n').to_string());
            }
        }
        for needed in ["Install and first use", "Build and test", "The crates"] {
            if !by_name.contains_key(needed) {
                fail(&format!("README has no '## {needed}' section"));
            }
        }

        let mut index_parts: Vec<String> = vec![top.to_string(), INDEX_LEAD.to_string()];
        for (name, level, body) in &blocks {
            if *level != 2 || name == "Install and first use" || name == "Build and test" {
                continue;
            }
            let mut text = body.join("\n").trim_matches('\n').to_string();
            if name == "The crates" && self.api {
                text = self
                    .re
                    .crate_row
                    .replace_all(&text, |c: &Captures| {
                        if self.crates.iter().any(|known| known == &c[1]) {
                            format!(
                                "| [`{}`](bundle:api/{}/index.html)",
                                &c[1],
                                c[1].replace('-', "_")
                            )
                        } else {
                            c[0].to_string()
                        }
                    })
                    .into_owned();
                text = text.replace(CRATES_LEAD, CRATES_LEAD_API);
            }
            if name == "The crates" {
                text.push_str(CRATES_TAIL);
            }
            index_parts.push(text);
        }
        let index_body = self.rewrite(&index_parts.join("\n\n"), "README.md", "index.md", None);
        let intro = self.first_sentence(top);
        self.write(
            "index.md",
            &(front("pfx", &intro, 1) + index_body.trim_matches('\n') + "\n"),
        );

        let after_heading = |name: &str| -> String {
            let text = &by_name[name];
            text.split_once('\n')
                .map(|(_, rest)| rest.trim_matches('\n').to_string())
                .unwrap_or_else(|| fail(&format!("README's '## {name}' section is empty")))
        };
        let install_body = [
            after_heading("Install and first use"),
            NEXT_STEPS.to_string(),
            format!(
                "## Build pfx from source\n\n{}",
                after_heading("Build and test")
            ),
        ]
        .join("\n\n");
        let install_body = self.rewrite(&install_body, "README.md", "install.md", None);
        self.write(
            "install.md",
            &(front("Install", INSTALL_DESCRIPTION, 2) + install_body.trim_matches('\n') + "\n"),
        );
    }

    fn guides(&self) {
        for (name, order, description) in GUIDES {
            let source = format!("docs/{name}.md");
            let text = self.read(&source);
            let stripped = strip_title(&text);
            let description = description
                .map(str::to_string)
                .unwrap_or_else(|| self.first_sentence(&stripped));
            let page = format!("guides/{name}.md");
            let body = self.rewrite(&stripped, &source, &page, None);
            self.write(
                &page,
                &(front(&self.titles[&source], &description, *order)
                    + body.trim_matches('\n')
                    + "\n"),
            );
        }
    }

    fn formats(&self) {
        let mut formats: Vec<String> = vec![FORMATS_LEAD.to_string()];
        for (heading, summary, picks) in FORMATS {
            formats.push(format!("## {heading}\n\n{summary}"));
            for (name, picked) in *picks {
                let source = format!("docs/{name}.md");
                let (text, level) = self.section(&self.read(&source), *picked);
                let place = match picked {
                    Some(picked) => format!("guides/{name}.md#{}", self.anchor(picked)),
                    None => format!("guides/{name}.md"),
                };
                let text = if picked.is_some() {
                    demote(&text, level, 3)
                } else {
                    text
                };
                let entry = format!("{text}\n\nFrom [{}](bundle:{place}).", self.titles[&source]);
                formats.push(self.rewrite(&entry, &source, "reference/formats.md", Some(&source)));
            }
        }
        self.write(
            "reference/formats.md",
            &(front("Formats", FORMATS_DESCRIPTION, 902)
                + formats.join("\n\n").trim_matches('\n')
                + "\n"),
        );
    }

    fn cli(&self) {
        let main_rs = self.read("src/main.rs");
        let about = self.rust_str("ABOUT", &main_rs);
        let usage = self.rust_str("USAGE", &main_rs);
        let switches = self.rust_list("SWITCHES", &main_rs);
        let mut shapes: HashMap<String, String> = HashMap::new();
        for c in self.re.shape.captures_iter(&main_rs) {
            shapes.insert(c[1].to_string(), c[2].to_string());
        }
        let commands: Vec<String> = self
            .rust_list("COMMANDS", &main_rs)
            .into_iter()
            .filter(|command| command != "help")
            .collect();
        let mut cli = vec![
            about.clone(),
            "`pfx --help` prints:".to_string(),
            format!("```\n{about}\n\n{usage}\n```"),
        ];
        let rows: Vec<String> = commands
            .iter()
            .filter_map(|command| {
                shapes
                    .get(command)
                    .map(|shape| format!("| `{command}` | `{}` |", shape.replace('|', "\\|")))
            })
            .collect();
        cli.push(format!(
            "## Commands\n\n| Command | Shape |\n|---|---|\n{}",
            rows.join("\n")
        ));
        cli.push(FLAGS_LEAD.to_string());
        for (name, line, constant) in FLAG_GROUPS {
            let rows: Vec<String> = self
                .rust_list(constant, &main_rs)
                .iter()
                .map(|flag| {
                    let kind = if switches.contains(flag) {
                        "switch"
                    } else {
                        "takes a value"
                    };
                    format!("| `{flag}` | {kind} |")
                })
                .collect();
            cli.push(format!(
                "### pfx {name}\n\n{line}\n\n| Flag | Kind |\n|---|---|\n{}",
                rows.join("\n")
            ));
        }
        cli.push(ERRORS.to_string());
        let body = self.rewrite(&cli.join("\n\n"), "src/main.rs", "reference/cli.md", None);
        self.write(
            "reference/cli.md",
            &(front("The pfx tool", CLI_DESCRIPTION, 901) + body.trim_matches('\n') + "\n"),
        );
    }

    fn changelog(&self, notes: &Path) {
        let changelog = self.read("CHANGELOG.md");
        let body = self.rewrite(
            &strip_title(&changelog),
            "CHANGELOG.md",
            "changelog.md",
            None,
        );
        self.write(
            "changelog.md",
            &(front("Changelog", "What changed in each pfx release.", 999)
                + body.trim_matches('\n')
                + "\n"),
        );
        let section = changelog_section(&changelog, &self.tag)
            .filter(|section| !section.trim().is_empty())
            .unwrap_or_else(|| fail(&format!("CHANGELOG.md has no section for {}", self.tag)));
        std::fs::write(notes, section.trim_matches('\n').to_string() + "\n")
            .unwrap_or_else(|error| fail(&format!("{}: {error}", notes.display())));
    }

    fn api_index(&self) {
        let items: Vec<String> = self
            .crates
            .iter()
            .map(|name| {
                format!(
                    "<li><a href=\"{}/index.html\">{name}</a></li>",
                    name.replace('-', "_")
                )
            })
            .collect();
        let tag = &self.tag;
        self.write(
            "api/index.html",
            &format!(
                "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>pfx {tag} API</title>\n</head>\n<body>\n<h1>pfx {tag} API</h1>\n<ul>\n{}\n</ul>\n</body>\n</html>\n",
                items.join("\n")
            ),
        );
    }

    fn check_links_and_copy_images(&self) {
        let mut files = Vec::new();
        walk(&self.dest, &mut files);
        let mut used: BTreeSet<String> = BTreeSet::new();
        for file in files {
            let Ok(relative) = file.strip_prefix(&self.dest) else {
                continue;
            };
            let relative: Vec<String> = relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            if relative.first().is_some_and(|first| first == "api") {
                continue;
            }
            let Some(name) = relative.last().filter(|name| name.ends_with(".md")) else {
                continue;
            };
            let folder = relative[..relative.len() - 1].join("/");
            let folder = if folder.is_empty() {
                ".".to_string()
            } else {
                folder
            };
            let text = std::fs::read_to_string(&file)
                .unwrap_or_else(|error| fail(&format!("{}: {error}", file.display())));
            for c in self.re.target.captures_iter(&text) {
                let target = &c[1];
                if self.re.scheme.is_match(target) {
                    continue;
                }
                let path = target.split_once('#').map_or(target, |(path, _)| path);
                if path.is_empty() {
                    continue;
                }
                let resolved = normpath(&join(&folder, path));
                if let Some(image) = resolved.strip_prefix("images/") {
                    used.insert(image.to_string());
                } else if resolved.starts_with("api/") {
                    if self.api && !self.dest.join(&resolved).exists() {
                        fail(&format!("{name} links to a missing API page: {target}"));
                    }
                } else if !self.dest.join(&resolved).exists() {
                    fail(&format!("{name} links to a missing page: {target}"));
                }
            }
        }
        let images = self.dest.join("images");
        std::fs::create_dir_all(&images)
            .unwrap_or_else(|error| fail(&format!("{}: {error}", images.display())));
        let notice = self.read("NOTICE.md");
        for image in used {
            let source = self.root.join("docs").join("images").join(&image);
            if !source.exists() {
                fail(&format!(
                    "a page uses docs/images/{image}, which does not exist"
                ));
            }
            if !notice.contains(&image) {
                fail(&format!("docs/images/{image} is not named in NOTICE.md"));
            }
            std::fs::copy(&source, images.join(&image))
                .unwrap_or_else(|error| fail(&format!("{}: {error}", source.display())));
        }
    }
}
