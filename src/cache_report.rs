use pfx::harness::tools;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const STAMP: &str = ".used";
const PRUNE_DAYS: u64 = 30;
const LOCK_LIMIT: Duration = Duration::from_secs(4 * 3600);
const ROOT_VARIABLE: &str = "PFX_CACHE";

const GROUPS: &[(&str, &str)] = &[
    ("ffmpeg", "ffmpeg"),
    ("adapters", "adapters"),
    ("src", "checkouts"),
    ("archive-queue", "archive-queue"),
];

pub enum Request {
    Show,
    Prune { dry_run: bool },
}

pub struct Entry {
    pub section: &'static str,
    pub name: String,
    pub repo: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub used: SystemTime,
    pub in_use: bool,
}

impl Entry {
    fn prunable(&self) -> bool {
        matches!(self.section, "ffmpeg" | "adapters" | "checkouts" | "target")
    }

    fn stale(&self, now: SystemTime) -> bool {
        self.prunable()
            && now.duration_since(self.used).unwrap_or_default()
                > Duration::from_secs(PRUNE_DAYS * 86_400)
    }
}

pub struct Pruned {
    pub freed: u64,
    pub removed: Vec<(&'static str, String, u64)>,
    pub kept_in_use: usize,
}

fn short(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(url)
        .trim_end_matches(".git")
        .to_string()
}

fn origins(root: &Path) -> BTreeMap<String, String> {
    children(&root.join("src"))
        .into_iter()
        .filter(|path| path.join(".git").exists())
        .filter_map(|path| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&path)
                .args(["remote", "get-url", "origin"])
                .output()
                .ok()?;
            out.status.success().then(|| {
                (
                    name_of(&path),
                    short(String::from_utf8_lossy(&out.stdout).trim()),
                )
            })
        })
        .collect()
}

fn millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn bytes_of(path: &Path) -> u64 {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => tools::size(path),
        Ok(meta) => meta.len(),
        Err(_) => 0,
    }
}

fn children(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    out.sort();
    out
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn locks(root: &Path, now: SystemTime) -> Vec<String> {
    children(&root.join("src"))
        .into_iter()
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "lock"))
        .filter(|p| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .is_ok_and(|t| now.duration_since(t).unwrap_or_default() <= LOCK_LIMIT)
        })
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect()
}

fn held(section: &str, name: &str, locks: &[String]) -> bool {
    locks.iter().any(|stem| match section {
        "checkouts" => name == stem,
        "adapters" => name.starts_with(&format!("{stem}-")),
        "target" => name
            .strip_prefix("target-")
            .is_some_and(|app| stem.starts_with(&format!("{app}-"))),
        _ => false,
    })
}

fn repo_of(section: &str, name: &str, origins: &BTreeMap<String, String>) -> String {
    let found = match section {
        "target" => name.strip_prefix("target-").and_then(|app| {
            origins
                .iter()
                .find(|(checkout, _)| checkout.starts_with(&format!("{app}-")))
                .map(|(_, repo)| repo.clone())
        }),
        "checkouts" => origins.get(name).cloned(),
        "adapters" => origins
            .iter()
            .filter(|(checkout, _)| name.starts_with(&format!("{checkout}-")))
            .max_by_key(|(checkout, _)| checkout.len())
            .map(|(_, repo)| repo.clone()),
        "ffmpeg" | "archive-queue" => Some(section.to_string()),
        _ => None,
    };
    found.unwrap_or_else(|| name.to_string())
}

fn used_of(path: &Path, now: SystemTime) -> SystemTime {
    std::fs::metadata(path.join(STAMP))
        .or_else(|_| std::fs::symlink_metadata(path))
        .and_then(|m| m.modified())
        .unwrap_or(now)
}

fn entry(
    section: &'static str,
    path: PathBuf,
    origins: &BTreeMap<String, String>,
    locks: &[String],
    now: SystemTime,
) -> Entry {
    let name = name_of(&path);
    Entry {
        section,
        repo: repo_of(section, &name, origins),
        in_use: held(section, &name, locks),
        bytes: bytes_of(&path),
        used: used_of(&path, now),
        name,
        path,
    }
}

pub fn scan(root: &Path, now: SystemTime) -> Result<Vec<Entry>, String> {
    let origins = origins(root);
    let locks = locks(root, now);
    let mut out = Vec::new();
    for (group, section) in GROUPS {
        for path in children(&root.join(group)) {
            if path.is_dir() {
                out.push(entry(section, path, &origins, &locks, now));
            }
        }
    }
    for path in children(root) {
        let name = name_of(&path);
        if GROUPS.iter().any(|(group, _)| *group == name) {
            continue;
        }
        let section = if path.is_dir() && name.starts_with("target") {
            "target"
        } else {
            "other"
        };
        out.push(entry(section, path, &origins, &locks, now));
    }
    Ok(out)
}

pub fn prune(root: &Path, now: SystemTime, dry_run: bool) -> Result<Pruned, String> {
    let mut done = Pruned {
        freed: 0,
        removed: Vec::new(),
        kept_in_use: 0,
    };
    for entry in scan(root, now)? {
        if !entry.stale(now) {
            continue;
        }
        if entry.in_use {
            done.kept_in_use += 1;
            continue;
        }
        if !dry_run {
            std::fs::remove_dir_all(&entry.path)
                .map_err(|e| format!("{}: {e}", entry.path.display()))?;
        }
        done.freed += entry.bytes;
        done.removed.push((entry.section, entry.name, entry.bytes));
    }
    Ok(done)
}

pub fn report(root: &Path, set_by: Option<&str>, now: SystemTime) -> Result<Value, String> {
    let entries = scan(root, now)?;
    let stale: u64 = entries
        .iter()
        .filter(|e| e.stale(now) && !e.in_use)
        .map(|e| e.bytes)
        .sum();
    let queue = children(&root.join("archive-queue"));
    let queued: Vec<&PathBuf> = queue.iter().filter(|p| p.is_dir()).collect();
    let queue_bytes: u64 = queued.iter().map(|p| bytes_of(p)).sum();
    Ok(json!({
        "v": 1,
        "tool": "pfx",
        "tool_version": env!("CARGO_PKG_VERSION"),
        "root": root.to_string_lossy(),
        "root_set_by": set_by,
        "bytes": tools::size(root),
        "at": millis(now),
        "entries": entries
            .iter()
            .map(|e| json!({
                "section": e.section,
                "name": e.name,
                "repo": e.repo,
                "path": e.path.to_string_lossy(),
                "bytes": e.bytes,
                "used": millis(e.used),
                "in_use": e.in_use,
            }))
            .collect::<Vec<_>>(),
        "prune": {"after_days": PRUNE_DAYS, "stale_bytes": stale},
        "archive_queue": {"entries": queued.len(), "bytes": queue_bytes},
    }))
}

pub fn pruned(done: &Pruned) -> Value {
    json!({
        "v": 1,
        "freed": done.freed,
        "removed": done
            .removed
            .iter()
            .map(|(section, name, bytes)| json!({"section": section, "name": name, "bytes": bytes}))
            .collect::<Vec<_>>(),
        "kept_in_use": done.kept_in_use,
    })
}

pub fn run(request: &Request) -> Result<(), String> {
    let root = tools::cache()?;
    let now = SystemTime::now();
    let value = match request {
        Request::Show => {
            let set_by = std::env::var_os(ROOT_VARIABLE).map(|_| ROOT_VARIABLE);
            report(&root, set_by, now)?
        }
        Request::Prune { dry_run } => pruned(&prune(&root, now, *dry_run)?),
    };
    println!("{value}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let tmp = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp");
        let root = tmp.join(format!("cache-report-{label}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn fill(dir: &Path, bytes: usize) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("blob"), vec![0u8; bytes]).unwrap();
    }

    fn stamp(dir: &Path, at: SystemTime) {
        let file = dir.join(STAMP);
        std::fs::write(&file, b"").unwrap();
        std::fs::File::options()
            .write(true)
            .open(file)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }

    fn synthetic(label: &str) -> PathBuf {
        let root = scratch(label);
        fill(&root.join("ffmpeg/0123456789abcdef"), 100);
        fill(
            &root.join("adapters/alpha-alpha-repo-abcdef123456-src"),
            200,
        );
        fill(&root.join("src/alpha-alpha-repo"), 300);
        fill(&root.join("target-alpha"), 400);
        fill(&root.join("target-beta"), 450);
        fill(&root.join("archive-queue/aaaaaaaaaaaaaaaaaaaaaaaa"), 500);
        fill(&root.join("archive-queue/bbbbbbbbbbbbbbbbbbbbbbbb"), 600);
        fill(&root.join("scratch"), 700);
        std::fs::write(root.join("note.txt"), vec![0u8; 50]).unwrap();
        root
    }

    fn find<'a>(entries: &'a [Value], name: &str) -> &'a Value {
        entries.iter().find(|e| e["name"] == name).unwrap()
    }

    #[test]
    fn the_report_has_every_key_of_the_shared_format_with_the_right_types() {
        let root = synthetic("shape");
        let now = SystemTime::now();
        let value = report(&root, None, now).unwrap();
        let top = value.as_object().unwrap();
        let mut keys: Vec<&str> = top.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "archive_queue",
                "at",
                "bytes",
                "entries",
                "prune",
                "root",
                "root_set_by",
                "tool",
                "tool_version",
                "v"
            ]
        );
        assert_eq!(value["v"], 1);
        assert_eq!(value["tool"], "pfx");
        assert_eq!(value["tool_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["root"], root.to_string_lossy().as_ref());
        assert!(value["root_set_by"].is_null());
        assert_eq!(value["at"].as_u64().unwrap(), millis(now));
        assert!(value["bytes"].as_u64().unwrap() >= 3300);
        assert_eq!(value["prune"]["after_days"], 30);
        assert_eq!(value["prune"]["stale_bytes"], 0);
        assert_eq!(value["archive_queue"]["entries"], 2);
        assert_eq!(value["archive_queue"]["bytes"], 1100);
        for e in value["entries"].as_array().unwrap() {
            let entry = e.as_object().unwrap();
            let mut keys: Vec<&str> = entry.keys().map(String::as_str).collect();
            keys.sort_unstable();
            assert_eq!(
                keys,
                ["bytes", "in_use", "name", "path", "repo", "section", "used"]
            );
            assert!(e["section"].is_string());
            assert!(e["name"].is_string());
            assert!(e["repo"].is_string());
            assert!(e["path"].is_string());
            assert!(e["bytes"].is_u64());
            assert!(e["used"].is_u64());
            assert!(e["in_use"].is_boolean());
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn root_set_by_names_the_variable_only_when_it_moved_the_root() {
        let root = scratch("setby");
        let now = SystemTime::now();
        assert!(report(&root, None, now).unwrap()["root_set_by"].is_null());
        assert_eq!(
            report(&root, Some(ROOT_VARIABLE), now).unwrap()["root_set_by"],
            "PFX_CACHE"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn entries_are_sorted_into_pfx_own_sections_with_their_repos() {
        let root = synthetic("sections");
        let value = report(&root, None, SystemTime::now()).unwrap();
        let entries = value["entries"].as_array().unwrap();
        let ffmpeg = find(entries, "0123456789abcdef");
        assert_eq!(ffmpeg["section"], "ffmpeg");
        assert_eq!(ffmpeg["bytes"], 100);
        let adapter = find(entries, "alpha-alpha-repo-abcdef123456-src");
        assert_eq!(adapter["section"], "adapters");
        assert_eq!(adapter["repo"], "alpha-alpha-repo-abcdef123456-src");
        let checkout = find(entries, "alpha-alpha-repo");
        assert_eq!(checkout["section"], "checkouts");
        assert_eq!(checkout["repo"], "alpha-alpha-repo");
        let alpha = find(entries, "target-alpha");
        assert_eq!(alpha["section"], "target");
        assert_eq!(alpha["repo"], "target-alpha");
        let beta = find(entries, "target-beta");
        assert_eq!(beta["section"], "target");
        assert_eq!(beta["repo"], "target-beta");
        let queued = find(entries, "aaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(queued["section"], "archive-queue");
        assert_eq!(queued["bytes"], 500);
        assert_eq!(find(entries, "scratch")["section"], "other");
        assert_eq!(find(entries, "note.txt")["section"], "other");
        assert_eq!(find(entries, "note.txt")["bytes"], 50);
        assert_eq!(entries.len(), 9);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn checkouts_name_their_own_repo() {
        let root = scratch("origins");
        let checkout = root.join("src/sample-sample-repo");
        fill(&checkout, 10);
        let init = std::process::Command::new("git")
            .arg("-C")
            .arg(&checkout)
            .args(["init", "--quiet"])
            .status()
            .unwrap();
        assert!(init.success());
        let remote = std::process::Command::new("git")
            .arg("-C")
            .arg(&checkout)
            .args(["remote", "add", "origin", "file:///repos/sample-repo.git"])
            .status()
            .unwrap();
        assert!(remote.success());
        fill(
            &root.join("adapters/sample-sample-repo-abcdef123456-src"),
            20,
        );
        fill(&root.join("target-sample"), 30);
        let value = report(&root, None, SystemTime::now()).unwrap();
        let entries = value["entries"].as_array().unwrap();
        assert_eq!(find(entries, "sample-sample-repo")["repo"], "sample-repo");
        assert_eq!(
            find(entries, "sample-sample-repo-abcdef123456-src")["repo"],
            "sample-repo"
        );
        assert_eq!(find(entries, "target-sample")["repo"], "sample-repo");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn used_comes_from_the_last_use_mark_or_the_mtime() {
        let root = scratch("used");
        let marked = root.join("target-alpha");
        fill(&marked, 10);
        let at = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        stamp(&marked, at);
        fill(&root.join("target-beta"), 10);
        let now = SystemTime::now();
        let value = report(&root, None, now).unwrap();
        let entries = value["entries"].as_array().unwrap();
        assert_eq!(find(entries, "target-alpha")["used"], 1_700_000_000_000u64);
        let unmarked = find(entries, "target-beta")["used"].as_u64().unwrap();
        assert!(unmarked > 1_700_000_000_000 && unmarked <= millis(now));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_held_lock_marks_the_checkout_its_adapters_and_its_target_in_use() {
        let root = synthetic("locks");
        let now = SystemTime::now();
        let value = report(&root, None, now).unwrap();
        for e in value["entries"].as_array().unwrap() {
            assert_eq!(e["in_use"], false, "{}", e["name"]);
        }
        std::fs::write(root.join("src/alpha-alpha-repo.lock"), b"").unwrap();
        let value = report(&root, None, now).unwrap();
        let entries = value["entries"].as_array().unwrap();
        for name in [
            "alpha-alpha-repo",
            "alpha-alpha-repo-abcdef123456-src",
            "target-alpha",
        ] {
            assert_eq!(find(entries, name)["in_use"], true, "{name}");
        }
        for name in ["target-beta", "0123456789abcdef", "scratch"] {
            assert_eq!(find(entries, name)["in_use"], false, "{name}");
        }
        std::fs::remove_file(root.join("src/alpha-alpha-repo.lock")).unwrap();
        let value = report(&root, None, now).unwrap();
        assert_eq!(
            find(value["entries"].as_array().unwrap(), "target-alpha")["in_use"],
            false
        );
        std::fs::remove_dir_all(&root).ok();
    }

    fn aged(label: &str) -> (PathBuf, SystemTime) {
        let root = synthetic(label);
        let long_ago = SystemTime::now() - Duration::from_secs(40 * 86_400);
        for dir in [
            "ffmpeg/0123456789abcdef",
            "adapters/alpha-alpha-repo-abcdef123456-src",
            "src/alpha-alpha-repo",
            "target-alpha",
            "archive-queue/aaaaaaaaaaaaaaaaaaaaaaaa",
            "scratch",
        ] {
            stamp(&root.join(dir), long_ago);
        }
        (root, SystemTime::now())
    }

    #[test]
    fn dry_run_reports_what_would_go_and_deletes_nothing() {
        let (root, now) = aged("dry");
        let value = report(&root, None, now).unwrap();
        assert_eq!(value["prune"]["stale_bytes"], 1000);
        let done = prune(&root, now, true).unwrap();
        assert_eq!(done.removed.len(), 4);
        assert_eq!(done.freed, 1000);
        assert_eq!(done.kept_in_use, 0);
        let answer = pruned(&done);
        assert_eq!(answer["v"], 1);
        assert_eq!(answer["freed"], done.freed);
        assert_eq!(answer["kept_in_use"], 0);
        assert!(answer["removed"][0]["name"].is_string());
        assert!(answer["removed"][0]["section"].is_string());
        assert!(answer["removed"][0]["bytes"].is_u64());
        for dir in [
            "ffmpeg/0123456789abcdef",
            "adapters/alpha-alpha-repo-abcdef123456-src",
            "src/alpha-alpha-repo",
            "target-alpha",
            "archive-queue/aaaaaaaaaaaaaaaaaaaaaaaa",
            "scratch",
            "target-beta",
        ] {
            assert!(root.join(dir).is_dir(), "{dir}");
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn prune_removes_what_went_unused_and_spares_the_rest_and_whatever_is_in_use() {
        let (root, now) = aged("prune");
        std::fs::write(root.join("src/alpha-alpha-repo.lock"), b"").unwrap();
        let done = prune(&root, now, false).unwrap();
        assert_eq!(done.removed.len(), 1);
        assert_eq!(done.removed[0].0, "ffmpeg");
        assert_eq!(done.removed[0].1, "0123456789abcdef");
        assert_eq!(done.kept_in_use, 3);
        assert!(!root.join("ffmpeg/0123456789abcdef").exists());
        for dir in [
            "adapters/alpha-alpha-repo-abcdef123456-src",
            "src/alpha-alpha-repo",
            "target-alpha",
            "archive-queue/aaaaaaaaaaaaaaaaaaaaaaaa",
            "scratch",
            "target-beta",
        ] {
            assert!(root.join(dir).is_dir(), "{dir}");
        }
        std::fs::remove_file(root.join("src/alpha-alpha-repo.lock")).unwrap();
        let done = prune(&root, now, false).unwrap();
        assert_eq!(done.removed.len(), 3);
        assert_eq!(done.kept_in_use, 0);
        assert!(root.join("scratch").is_dir() && root.join("target-beta").is_dir());
        std::fs::remove_dir_all(&root).ok();
    }
}
