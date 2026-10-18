use std::fmt;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error,
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Error => "error",
            Self::Warning => "warning",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Location {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file.display(), self.line, self.column)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub severity: Severity,
    pub code: &'static str,
    pub key: String,
    pub message: String,
    pub related: Vec<Location>,
}

impl Diagnostic {
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    pub fn location(&self) -> Location {
        Location {
            file: self.file.clone(),
            line: self.line,
            column: self.column,
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {} [{}]",
            self.file.display(),
            self.line,
            self.column,
            self.severity,
            self.code
        )?;
        if !self.key.is_empty() {
            write!(f, " {}:", self.key)?;
        }
        write!(f, " {}", self.message)?;
        for related in &self.related {
            write!(f, " (see {related})")?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostic {}

pub mod code {
    pub const SYNTAX: &str = "syntax";
    pub const UNREADABLE: &str = "unreadable";
    pub const NOT_UTF8: &str = "not-utf8";
    pub const UNKNOWN_KEY: &str = "unknown-key";
    pub const MISSING_KEY: &str = "missing-key";
    pub const BAD_TYPE: &str = "bad-type";
    pub const BAD_VALUE: &str = "bad-value";
    pub const BAD_ID: &str = "bad-id";
    pub const MISSING_ID: &str = "missing-id";
    pub const DUPLICATE_ID: &str = "duplicate-id";
    pub const BAD_NAME: &str = "bad-name";
    pub const DUPLICATE_NAME: &str = "duplicate-name";
    pub const BAD_REFERENCE: &str = "bad-reference";
    pub const AMBIGUOUS_REFERENCE: &str = "ambiguous-reference";
    pub const BAD_PATH: &str = "bad-path";
    pub const OUTSIDE_ROOT: &str = "outside-root";
    pub const MISSING_FILE: &str = "missing-file";
    pub const PATH_CASE: &str = "path-case";
    pub const INCLUDE_CYCLE: &str = "include-cycle";
    pub const PREFAB_CYCLE: &str = "prefab-cycle";
    pub const INCLUDE_DEPTH: &str = "include-depth";
    pub const PREFAB_DEPTH: &str = "prefab-depth";
    pub const PARENT_CYCLE: &str = "parent-cycle";
    pub const HELD_TWICE: &str = "held-twice";
    pub const BAD_OVERRIDE: &str = "bad-override";
    pub const PLACEMENT_KEY: &str = "placement-key";
    pub const OLD_FORMAT: &str = "old-format";
    pub const NEWER_FORMAT: &str = "newer-format";
    pub const MIGRATION: &str = "migration";

    pub const ALL: &[&str] = &[
        SYNTAX,
        UNREADABLE,
        NOT_UTF8,
        UNKNOWN_KEY,
        MISSING_KEY,
        BAD_TYPE,
        BAD_VALUE,
        BAD_ID,
        MISSING_ID,
        DUPLICATE_ID,
        BAD_NAME,
        DUPLICATE_NAME,
        BAD_REFERENCE,
        AMBIGUOUS_REFERENCE,
        BAD_PATH,
        OUTSIDE_ROOT,
        MISSING_FILE,
        PATH_CASE,
        INCLUDE_CYCLE,
        PREFAB_CYCLE,
        INCLUDE_DEPTH,
        PREFAB_DEPTH,
        PARENT_CYCLE,
        HELD_TWICE,
        BAD_OVERRIDE,
        PLACEMENT_KEY,
        OLD_FORMAT,
        NEWER_FORMAT,
        MIGRATION,
    ];
}
