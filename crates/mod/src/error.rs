use std::fmt;
use std::path::PathBuf;

use wasmtime::Trap;

use crate::manifest::{ID_CHARS, MANIFEST};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostError(pub String);

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "mod host: {}", self.0)
    }
}

impl std::error::Error for HostError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    Read(String),
    Manifest(String),
    Id {
        found: String,
    },
    Folder {
        id: String,
        folder: String,
    },
    Entry {
        found: String,
    },
    Api {
        wanted: String,
        world: String,
        version: String,
    },
    Size {
        what: &'static str,
        bytes: u64,
        cap: usize,
    },
    NotComponent,
    Invalid(String),
    Start,
    Import {
        name: String,
    },
    Types(String),
    Module(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(e) => write!(f, "cannot be read: {e}"),
            Self::Manifest(e) => write!(f, "has a bad {MANIFEST}: {e}"),
            Self::Id { found } => write!(
                f,
                "has id `{found}`; an id is 1 to {ID_CHARS} lowercase letters, digits, `-` or `_`, starting with a letter or digit"
            ),
            Self::Folder { id, folder } => write!(
                f,
                "has id `{id}` in folder `{folder}`; the folder must be named after the id"
            ),
            Self::Entry { found } => write!(
                f,
                "has entry `{found}`; the entry is a file name ending in .wasm beside {MANIFEST}, with no folders"
            ),
            Self::Api {
                wanted,
                world,
                version,
            } => write!(
                f,
                "was made for version {wanted} of the game's `{world}` mod API, but this game has {version}; it needs a mod made for a version compatible with {version}"
            ),
            Self::Size { what, bytes, cap } => {
                write!(f, "has a {what} of {bytes} bytes, over the cap of {cap}")
            }
            Self::NotComponent => write!(
                f,
                "is not a WebAssembly component; core modules are refused, build the mod as a component"
            ),
            Self::Invalid(e) => write!(f, "is not a valid component: {e}"),
            Self::Start => write!(
                f,
                "has a component start function, which this host refuses; run setup from an export the game calls"
            ),
            Self::Import { name } => write!(
                f,
                "imports `{name}`, which is not part of the game's mod API"
            ),
            Self::Types(e) => write!(f, "does not match the game's mod API: {e}"),
            Self::Module(e) => write!(
                f,
                "is a core module that cannot be made a component: {e}; build it with wit-bindgen, or as a component"
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadError {
    pub folder: PathBuf,
    pub id: Option<String>,
    pub refusal: Box<Refusal>,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.id {
            Some(id) => write!(f, "mod `{id}` ({}) {}", self.folder.display(), self.refusal),
            None => write!(f, "mod at {} {}", self.folder.display(), self.refusal),
        }
    }
}

impl std::error::Error for LoadError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Limit {
    Memory { bytes: usize },
    Table { elements: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cause {
    Fuel,
    Depth,
    Stack,
    Trap(Trap),
    Limit(Limit),
    Error(String),
    Disabled,
}

impl fmt::Display for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fuel => write!(f, "ran out of fuel"),
            Self::Depth => write!(f, "called deeper than its call-depth limit"),
            Self::Stack => write!(f, "exhausted its stack"),
            Self::Trap(t) => write!(f, "trapped ({t})"),
            Self::Limit(Limit::Memory { bytes }) => {
                write!(f, "tried to grow its memory to {bytes} bytes, over the cap")
            }
            Self::Limit(Limit::Table { elements }) => {
                write!(
                    f,
                    "tried to grow a table to {elements} elements, over the cap"
                )
            }
            Self::Error(e) => write!(f, "failed: {e}"),
            Self::Disabled => write!(f, "is disabled for this run"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Frame {
    pub func: u32,
    pub offset: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModError {
    pub id: String,
    pub cause: Cause,
    pub fuel: u64,
    pub at: Option<Frame>,
}

impl fmt::Display for ModError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "mod `{}` {}", self.id, self.cause)?;
        if self.cause != Cause::Disabled {
            write!(f, " after {} fuel", self.fuel)?;
        }
        if let Some(at) = self.at {
            write!(f, " in function {}", at.func)?;
            if let Some(offset) = at.offset {
                write!(f, " at offset {offset:#x}")?;
            }
        }
        if self.cause != Cause::Disabled {
            write!(f, "; it is stopped and disabled for this run")?;
        }
        Ok(())
    }
}

impl std::error::Error for ModError {}
