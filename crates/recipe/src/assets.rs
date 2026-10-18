use crate::{Recipe, files};
use std::path::{Path, PathBuf};

pub const LIBRARY_PREFIX: &str = "lib:";

fn clean(path: &Path) -> PathBuf {
    files::clean(path)
}

fn joined(base: &Path, folder: &str) -> PathBuf {
    clean(&base.join(folder))
}

impl Recipe {
    pub fn asset_roots_at(&self, base: &Path) -> Vec<PathBuf> {
        let mut folders: Vec<PathBuf> = Vec::new();
        for root in &self.asset_roots {
            let folder = joined(base, root);
            if !folders.contains(&folder) {
                folders.push(folder);
            }
        }
        folders
    }

    pub fn asset_folders(&self, base: &Path) -> Vec<PathBuf> {
        let here = clean(base);
        let mut folders = vec![here.clone()];
        folders.extend(
            self.asset_roots_at(base)
                .into_iter()
                .filter(|folder| *folder != here),
        );
        folders
    }

    pub fn project_folder(&self, base: &Path) -> Option<PathBuf> {
        self.project_root.as_deref().map(|root| joined(base, root))
    }

    pub fn asset(&self, base: &Path, reference: &str) -> Result<PathBuf, String> {
        self.asset_where(base, reference, "a file", &|path| files::is_file(path))
    }

    pub fn asset_where(
        &self,
        base: &Path,
        reference: &str,
        what: &str,
        found: &dyn Fn(&Path) -> bool,
    ) -> Result<PathBuf, String> {
        let given = Path::new(reference);
        if given.is_absolute() {
            return if found(given) {
                Ok(given.to_path_buf())
            } else {
                Err(format!("{reference} is not {what}"))
            };
        }
        let folders = self.asset_folders(base);
        if found(&base.join(reference)) {
            return Ok(base.join(reference));
        }
        for folder in &folders[1..] {
            let path = folder.join(reference);
            if found(&path) {
                return Ok(path);
            }
        }
        let searched = folders
            .iter()
            .map(|folder| folder.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        Err(if folders.len() == 1 {
            format!("{reference} is not {what} under {searched}")
        } else {
            format!("{reference} is not {what} in any of {searched}")
        })
    }

    pub fn asset_root_of(&self, base: &Path, file: &Path) -> Option<PathBuf> {
        let file = clean(file);
        self.asset_folders(base)
            .into_iter()
            .filter(|folder| file.starts_with(folder))
            .max_by_key(|folder| folder.components().count())
    }
}

pub struct Library<'a> {
    pub file: &'a str,
    pub entry: &'a str,
}

pub fn library_reference(reference: &str) -> Option<Library<'_>> {
    let rest = reference.strip_prefix(LIBRARY_PREFIX)?;
    let end = rest.find(".pitofx/")? + ".pitofx".len();
    Some(Library {
        file: &rest[..end],
        entry: &rest[end + 1..],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/recipe-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        files::clean(&dir)
    }

    #[test]
    fn a_reference_resolves_in_the_document_folder_then_each_root_in_order() {
        let dir = scratch("assets-order");
        for (folder, file) in [
            ("doc", "a.png"),
            ("one", "a.png"),
            ("one", "b.png"),
            ("two", "b.png"),
            ("two", "c.png"),
        ] {
            std::fs::create_dir_all(dir.join(folder)).unwrap();
            std::fs::write(dir.join(folder).join(file), folder).unwrap();
        }
        let recipe =
            crate::parse("recipe = 1\nname = \"t\"\nasset_roots = [\"../one\", \"../two\"]\n")
                .unwrap();
        let base = dir.join("doc");
        let found = |name: &str| recipe.asset(&base, name).unwrap();
        assert_eq!(found("a.png"), base.join("a.png"));
        assert_eq!(found("b.png"), dir.join("one/b.png"));
        assert_eq!(found("c.png"), dir.join("two/c.png"));
        let error = recipe.asset(&base, "d.png").unwrap_err();
        assert!(
            error.starts_with("d.png is not a file in any of "),
            "{error}"
        );
        assert!(error.contains("one") && error.contains("two"), "{error}");
        assert!(!error.contains('\n'), "{error}");
        assert_eq!(
            recipe.asset_root_of(&base, &dir.join("two/c.png")),
            Some(dir.join("two"))
        );
        assert_eq!(
            recipe.asset_root_of(&base, &dir.join("elsewhere/c.png")),
            None
        );
    }

    #[test]
    fn a_recipe_without_roots_searches_only_its_own_folder() {
        let dir = scratch("assets-plain");
        let recipe = crate::parse("recipe = 1\nname = \"t\"\n").unwrap();
        let error = recipe.asset(&dir, "x.png").unwrap_err();
        assert!(error.starts_with("x.png is not a file under "), "{error}");
        assert!(recipe.project_folder(&dir).is_none());
        let recipe = crate::parse("recipe = 1\nname = \"t\"\nproject_root = \"..\"\n").unwrap();
        assert_eq!(
            recipe.project_folder(&dir),
            Some(files::clean(&dir.join("..")))
        );
    }

    #[test]
    fn a_library_reference_splits_at_the_pfx_file() {
        let found = library_reference("lib:shared/props.pitofx/prop/crate").unwrap();
        assert_eq!(found.file, "shared/props.pitofx");
        assert_eq!(found.entry, "prop/crate");
        assert!(library_reference("prop/crate").is_none());
        assert!(library_reference("lib:props/crate").is_none());
    }
}
