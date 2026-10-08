use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const VARIABLE: &str = "PFX_RECIPES";
pub const SUBTREE: &str = "render/assets";
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Origin {
    Flag,
    Variable,
    Caller,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Root {
    pub dir: PathBuf,
    pub origin: Origin,
}

pub fn root(
    flag: Option<&Path>,
    variable: Option<PathBuf>,
    toplevel: Option<PathBuf>,
) -> Option<Root> {
    flag.map(|dir| Root {
        dir: dir.to_path_buf(),
        origin: Origin::Flag,
    })
    .or_else(|| {
        variable
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(|dir| Root {
                dir,
                origin: Origin::Variable,
            })
    })
    .or_else(|| {
        toplevel.map(|top| Root {
            dir: top.join(SUBTREE),
            origin: Origin::Caller,
        })
    })
}

pub fn slug(name: &str) -> Result<(), String> {
    let lower = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    if name.chars().next().is_some_and(lower) && name.chars().all(|c| lower(c) || c == '-') {
        Ok(())
    } else {
        Err(format!(
            "'{name}' is not a product slug: lowercase letters, digits and '-', starting with a letter or digit"
        ))
    }
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn toplevel(dir: &Path) -> Option<PathBuf> {
    git(dir, &["rev-parse", "--show-toplevel"])
        .filter(|top| !top.is_empty())
        .map(PathBuf::from)
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Clone, Debug)]
pub struct Recipes {
    root: Option<Root>,
    order: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Recipe {
    pub text: String,
    pub record: Value,
}

impl Recipe {
    pub fn pinned(&self) -> bool {
        self.record["pinned"].as_bool().unwrap_or(false)
    }

    pub fn sha256(&self) -> String {
        sha(self.text.as_bytes())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Listed {}

#[derive(Deserialize)]
struct Book {
    #[serde(default)]
    products: BTreeMap<String, toml::Spanned<Listed>>,
}

fn absolute(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}

fn names(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_file() && path.extension().is_some_and(|e| e == "toml"))
                .filter_map(|path| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

pub fn file(path: &Path) -> Result<Recipe, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(local_recipe(&absolute(path.to_path_buf()), text, true))
}

fn local_recipe(path: &Path, text: String, may_pin: bool) -> Recipe {
    let digest = sha(text.as_bytes());
    let shown = path.display().to_string();
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tracked = git(
        dir,
        &["ls-files", "--full-name", "--error-unmatch", "--", &name],
    );
    let record = match tracked {
        Some(full) => {
            let clean = git(dir, &["status", "--porcelain", "--", &name])
                .is_some_and(|status| status.is_empty());
            let repo = git(dir, &["remote", "get-url", "origin"])
                .or_else(|| toplevel(dir).map(|top| top.display().to_string()));
            json!({
                "path": full,
                "file": shown,
                "repo": repo,
                "commit": git(dir, &["rev-parse", "HEAD"]),
                "sha256": digest,
                "pinned": may_pin && clean,
            })
        }
        None => json!({
            "path": shown,
            "file": shown,
            "repo": Value::Null,
            "commit": Value::Null,
            "sha256": digest,
            "pinned": false,
        }),
    };
    Recipe { text, record }
}

impl Recipes {
    pub fn new(root: Option<Root>) -> Recipes {
        Recipes {
            root,
            order: Vec::new(),
        }
    }

    pub fn from_flags(
        values: &[String],
        variable: Option<PathBuf>,
        toplevel: impl FnOnce() -> Option<PathBuf>,
    ) -> Result<Recipes, String> {
        let dir = match values {
            [] => None,
            [one] if one.contains('=') => {
                return Err(format!(
                    "--recipes takes one folder, the caller's own; {one} names a product's folder elsewhere, and pfx reads no other repository"
                ));
            }
            [one] => Some(absolute(PathBuf::from(one))),
            _ => return Err("--recipes <dir> was given twice".into()),
        };
        let root = match (dir, variable.filter(|v| !v.as_os_str().is_empty())) {
            (Some(dir), _) => root(Some(&dir), None, None),
            (None, Some(var)) => root(None, Some(absolute(var)), None),
            (None, None) => root(None, None, toplevel()),
        };
        Ok(Recipes::new(root))
    }

    pub fn root(&self) -> Option<&Root> {
        self.root.as_ref()
    }

    pub fn path(&self, rel: &str) -> Option<PathBuf> {
        self.root.as_ref().map(|root| root.dir.join(rel))
    }

    pub fn find(&self, rel: &str) -> Result<Option<Recipe>, String> {
        if let Some(path) = self.path(rel) {
            match std::fs::read_to_string(&path) {
                Ok(text) => return Ok(Some(local_recipe(&path, text, true))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
        Ok(None)
    }

    pub fn read(&self, rel: &str) -> Result<Recipe, String> {
        self.find(rel)?.ok_or_else(|| self.missing("recipe", rel))
    }

    pub fn missing(&self, what: &str, rel: &str) -> String {
        match self.path(rel) {
            Some(path) => format!("no {what} at {}", path.display()),
            None => format!(
                "no recipes folder for {rel}: pass --recipes <dir>, set {VARIABLE}, or run inside the product's git tree, which keeps it at {SUBTREE}/{rel}"
            ),
        }
    }

    pub fn presets(&self) -> Vec<String> {
        self.root
            .as_ref()
            .map(|root| names(&root.dir.join("presets")))
            .unwrap_or_default()
    }

    pub fn books(&self) -> Vec<String> {
        self.root
            .as_ref()
            .map(|root| names(&root.dir.join("shots")))
            .unwrap_or_default()
    }

    pub fn icon_sets(&self, product: &str) -> Vec<String> {
        self.path(&format!("icons/{product}"))
            .map(|dir| names(&dir))
            .unwrap_or_default()
    }

    pub fn icon_products(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if let Some(root) = &self.root
            && let Ok(entries) = std::fs::read_dir(root.dir.join("icons"))
        {
            out = entries
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            out.sort();
        }
        out
    }

    pub fn universe(&self, own: &str) -> Vec<String> {
        let mut out = self.order.clone();
        if !out.iter().any(|p| p == own) {
            out.push(own.to_string());
        }
        out
    }

    pub fn list_products(&mut self, text: &str, what: &str) -> Result<(), String> {
        let book: Book = toml::from_str(text).map_err(|e| {
            format!(
                "{what}: {e}; [products.<p>] only orders the products of the caller's own folder"
            )
        })?;
        let mut listed: Vec<(usize, String)> = book
            .products
            .into_iter()
            .map(|(product, entry)| (entry.span().start, product))
            .collect();
        listed.sort_by_key(|(at, _)| *at);
        for (_, product) in listed {
            slug(&product).map_err(|e| format!("{what} [products.{product}]: {e}"))?;
            self.order.push(product);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/recipes")
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn named() -> Recipes {
        Recipes::from_flags(&[fixtures().display().to_string()], None, || None).unwrap()
    }

    #[test]
    fn the_root_is_the_flag_then_the_variable_then_the_callers_tree() {
        let flag = PathBuf::from("/flag");
        let var = Some(PathBuf::from("/var"));
        let top = Some(PathBuf::from("/top"));
        assert_eq!(
            root(Some(&flag), var.clone(), top.clone()),
            Some(Root {
                dir: flag.clone(),
                origin: Origin::Flag
            })
        );
        assert_eq!(
            root(None, var.clone(), top.clone()),
            Some(Root {
                dir: PathBuf::from("/var"),
                origin: Origin::Variable
            })
        );
        assert_eq!(
            root(None, Some(PathBuf::new()), top.clone()),
            Some(Root {
                dir: PathBuf::from("/top/render/assets"),
                origin: Origin::Caller
            })
        );
        assert_eq!(root(None, None, None), None);
        let from = |values: &[&str], var: Option<&str>| {
            let values: Vec<String> = values.iter().map(|v| v.to_string()).collect();
            Recipes::from_flags(&values, var.map(PathBuf::from), || {
                Some(PathBuf::from("/top"))
            })
            .unwrap()
            .root
            .unwrap()
        };
        assert_eq!(from(&["/flag"], Some("/var")).origin, Origin::Flag);
        assert_eq!(from(&[], Some("/var")).origin, Origin::Variable);
        assert_eq!(from(&[], None).dir, PathBuf::from("/top/render/assets"));
        assert!(Recipes::from_flags(&["/a".into(), "/b".into()], None, || None).is_err());
        let elsewhere = Recipes::from_flags(&["other=/o".into()], None, || None).unwrap_err();
        assert!(elsewhere.contains("no other repository"), "{elsewhere}");
    }

    #[test]
    fn product_slugs_take_no_list_but_only_safe_names() {
        for good in ["sample", "estate", "app-0", "a1"] {
            assert!(slug(good).is_ok(), "{good}");
        }
        for bad in ["", "Sample", "-x", "a b", "a/b", "..", "a_b"] {
            assert!(slug(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_named_root_reads_only_its_own_files_and_a_missing_one_names_the_path() {
        let recipes = named();
        let preset = recipes.read("presets/sample.toml").unwrap();
        assert!(preset.text.contains("[material]"));
        assert_eq!(preset.record["sha256"], json!(preset.sha256()));
        let missing = recipes.read("presets/other.toml").unwrap_err();
        assert_eq!(
            missing,
            format!(
                "no recipe at {}",
                fixtures().join("presets/other.toml").display()
            )
        );
        assert_eq!(recipes.presets(), ["sample"]);
        assert_eq!(recipes.books(), ["sample"]);
        assert_eq!(recipes.icon_sets("sample"), ["shapes"]);
        assert_eq!(recipes.icon_products(), ["sample"]);
        assert_eq!(recipes.universe("sample"), ["sample"]);
    }

    #[test]
    fn the_variable_is_strict_like_the_flag() {
        let recipes = Recipes::from_flags(&[], Some(fixtures()), || None).unwrap();
        assert_eq!(recipes.root().unwrap().origin, Origin::Variable);
        assert!(recipes.read("shots/sample.toml").is_ok());
        assert!(recipes.read("presets/other.toml").is_err());
    }

    #[test]
    fn the_callers_tree_reads_only_its_own_folder() {
        let top = scratch("caller");
        std::fs::create_dir_all(top.join("render/assets/presets")).unwrap();
        std::fs::write(
            top.join("render/assets/presets/sample.toml"),
            "colors = \"master\"\n",
        )
        .unwrap();
        let recipes = Recipes::from_flags(&[], None, || Some(top.clone())).unwrap();
        assert_eq!(recipes.root().unwrap().origin, Origin::Caller);
        let own = recipes.read("presets/sample.toml").unwrap();
        assert_eq!(own.text, "colors = \"master\"\n");
        assert_eq!(recipes.presets(), ["sample"]);
        assert!(recipes.books().is_empty());
        assert!(recipes.icon_products().is_empty());
        assert_eq!(recipes.universe("estate"), ["estate"]);
        assert!(
            recipes.read("presets/other.toml").unwrap_err().contains(
                &top.join("render/assets/presets/other.toml")
                    .display()
                    .to_string()
            )
        );
        let nowhere = Recipes::from_flags(&[], None, || None).unwrap();
        assert!(nowhere.presets().is_empty());
        assert!(
            nowhere
                .read("presets/sample.toml")
                .unwrap_err()
                .contains(VARIABLE)
        );
        std::fs::remove_dir_all(&top).ok();
    }

    #[test]
    fn a_shot_orders_its_products_and_reads_them_all_from_the_callers_folder() {
        let book = r#"
[products.other]

[products.sample]

[products.third]

[shots.all]
line = "x"
"#;
        let mut recipes = named();
        recipes.list_products(book, "shots/sample.toml").unwrap();
        assert_eq!(recipes.universe("sample"), ["other", "sample", "third"]);
        assert_eq!(
            recipes.universe("estate"),
            ["other", "sample", "third", "estate"]
        );
        assert!(recipes.read("presets/other.toml").is_err());
        for bad in [
            "[products.other]\nrepo = \"r\"\nrev = \"v1.0.0\"\n",
            "[products.other]\npath = \"render/assets\"\n",
            "[products.Other]\n",
        ] {
            let mut fresh = named();
            assert!(fresh.list_products(bad, "shots").is_err(), "{bad}");
        }
    }

    #[test]
    fn a_recipe_outside_git_is_recorded_but_unpinned() {
        let dir = scratch("loose");
        let path = dir.join("look.toml");
        std::fs::write(&path, "colors = \"master\"\n").unwrap();
        let recipe = file(&path).unwrap();
        assert_eq!(recipe.record["sha256"], json!(recipe.sha256()));
        assert!(!recipe.pinned());
        assert_eq!(recipe.record["commit"], Value::Null);
        std::fs::remove_dir_all(&dir).ok();
    }
}
