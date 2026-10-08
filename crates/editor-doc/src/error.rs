use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocError {
    pub file: PathBuf,
    pub key: String,
    pub message: String,
}

impl DocError {
    pub fn new(file: impl AsRef<Path>, key: impl Into<String>, message: impl Into<String>) -> Self {
        DocError {
            file: file.as_ref().to_path_buf(),
            key: key.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for DocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let file = self.file.display();
        if self.key.is_empty() {
            write!(f, "{file}: {}", self.message)
        } else {
            write!(f, "{file}: {}: {}", self.key, self.message)
        }
    }
}

impl std::error::Error for DocError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_names_its_file_and_key() {
        let error = DocError::new("a.toml", "object[crate].at", "no key `at`");
        assert_eq!(error.to_string(), "a.toml: object[crate].at: no key `at`");
        let bare = DocError::new("a.toml", "", "does not parse");
        assert_eq!(bare.to_string(), "a.toml: does not parse");
    }
}
