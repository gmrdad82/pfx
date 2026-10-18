use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PathError {
    Empty,
    Absolute,
    Backslash,
    Outside(String),
}

impl PathError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Outside(_) => crate::code::OUTSIDE_ROOT,
            _ => crate::code::BAD_PATH,
        }
    }

    pub(crate) fn message(&self, path: &str) -> String {
        match self {
            Self::Empty => "a path is empty".to_string(),
            Self::Absolute => {
                format!("path {path} is absolute; paths are relative to the project root")
            }
            Self::Backslash => format!("path {path} has a backslash; paths use /"),
            Self::Outside(_) => format!("path {path} leaves the project root"),
        }
    }
}

fn absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || path.starts_with('\\')
        || (bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic())
}

pub(crate) fn join(base: &str, path: &str) -> Result<String, PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    if path.contains('\\') {
        return Err(PathError::Backslash);
    }
    if absolute(path) {
        return Err(PathError::Absolute);
    }
    let mut parts: Vec<&str> = Vec::new();
    let mut above = 0usize;
    for part in base.split('/').chain(path.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    above += 1;
                }
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if above > 0 {
        let up = vec![".."; above].join("/");
        let shown = if joined.is_empty() {
            up
        } else {
            format!("{up}/{joined}")
        };
        return Err(PathError::Outside(shown));
    }
    if joined.is_empty() {
        return Err(PathError::Empty);
    }
    Ok(joined)
}

pub(crate) fn rooted(path: &str) -> Result<String, PathError> {
    join("", path)
}

pub(crate) fn folder(file: &Path) -> String {
    file.parent().map(slashed).unwrap_or_default()
}

pub(crate) fn slashed(path: &Path) -> String {
    path.components()
        .filter_map(|part| match part {
            std::path::Component::Normal(name) => name.to_str().map(str::to_string),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Found {
    Exact,
    Case(String),
    Missing,
}

#[derive(Default)]
pub(crate) struct Listing {
    folders: BTreeMap<PathBuf, Option<Vec<String>>>,
}

impl Listing {
    fn names(&mut self, folder: &Path) -> Option<&Vec<String>> {
        self.folders
            .entry(folder.to_path_buf())
            .or_insert_with(|| {
                std::fs::read_dir(folder).ok().map(|entries| {
                    entries
                        .filter_map(|entry| entry.ok())
                        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
                        .collect()
                })
            })
            .as_ref()
    }

    pub(crate) fn find(&mut self, root: &Path, path: &str, extra: &dyn Fn(&str) -> bool) -> Found {
        if extra(path) {
            return Found::Exact;
        }
        let mut folder = root.to_path_buf();
        let mut actual = Vec::new();
        let mut exact = true;
        for part in path.split('/') {
            let Some(names) = self.names(&folder) else {
                return Found::Missing;
            };
            if names.iter().any(|name| name == part) {
                actual.push(part.to_string());
                folder.push(part);
            } else if let Some(other) = names
                .iter()
                .find(|name| name.to_lowercase() == part.to_lowercase())
            {
                exact = false;
                actual.push(other.clone());
                folder.push(other);
            } else {
                return Found::Missing;
            }
        }
        if exact {
            Found::Exact
        } else {
            Found::Case(actual.join("/"))
        }
    }
}

pub(crate) fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

pub(crate) fn deepest_common(folders: &[PathBuf]) -> PathBuf {
    let mut iter = folders.iter();
    let Some(first) = iter.next() else {
        return PathBuf::new();
    };
    let mut common: Vec<_> = first.components().collect();
    for folder in iter {
        let parts: Vec<_> = folder.components().collect();
        let same = common
            .iter()
            .zip(&parts)
            .take_while(|(a, b)| a == b)
            .count();
        common.truncate(same);
    }
    common.iter().collect()
}

pub(crate) fn up_to(from: &Path, ancestor: &Path) -> String {
    let below = from.components().count();
    let above = ancestor.components().count();
    let steps = below.saturating_sub(above);
    if steps == 0 {
        ".".to_string()
    } else {
        vec![".."; steps].join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_join_and_refuse() {
        assert_eq!(join("scenes", "a.gltf"), Ok("scenes/a.gltf".into()));
        assert_eq!(
            join("scenes", "../meshes/a.gltf"),
            Ok("meshes/a.gltf".into())
        );
        assert_eq!(join("", "./a//b.gltf"), Ok("a/b.gltf".into()));
        assert_eq!(join("", "a/../b"), Ok("b".into()));
        assert_eq!(join("", "../a"), Err(PathError::Outside("../a".into())));
        assert_eq!(
            join("scenes", "../../x/a"),
            Err(PathError::Outside("../x/a".into()))
        );
        assert_eq!(join("", ""), Err(PathError::Empty));
        assert_eq!(join("", "/a"), Err(PathError::Absolute));
        assert_eq!(join("", "C:/a"), Err(PathError::Absolute));
        assert_eq!(join("", "a\\b"), Err(PathError::Backslash));
        assert_eq!(join("", "."), Err(PathError::Empty));
    }

    #[test]
    fn the_deepest_common_folder_is_found() {
        let found = deepest_common(&[
            lexical(Path::new("/w/a/b/../b/c")),
            "/w/a/b/d".into(),
            "/w/a/b".into(),
        ]);
        assert_eq!(found, PathBuf::from("/w/a/b"));
        assert_eq!(up_to(Path::new("/w/a/b"), Path::new("/w")), "../..");
        assert_eq!(up_to(Path::new("/w"), Path::new("/w")), ".");
    }
}
