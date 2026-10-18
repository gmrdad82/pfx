#![forbid(unsafe_code)]

mod diagnostic;
mod edit;
mod id;
mod migrate;
mod mount;
mod paths;
mod project;
mod read;
mod rules;
mod scene;
#[cfg(feature = "schema")]
mod schema;
mod sha256;
mod spans;
pub mod types;

pub use diagnostic::{Diagnostic, Location, Severity, code};
pub use edit::{EditError, Kind, Patch, PatchGroup, SceneEdit, Target, Value};
pub use id::{ID_BITS, ID_LENGTH, Id, IdError};
pub use mount::{GLTF_PATH, pack, unpack};
pub use pfx_file;
pub use project::{MAX_DEPTH, Project, check, migrate};
pub use scene::{Entry, Scene};
#[cfg(feature = "schema")]
pub use schema::json_schema;
pub use types::{
    FORMAT, File, FileKind, MaterialsFile, PrefabFile, ProjectFile, ProjectTable, ProxiesFile,
    SceneFile,
};
