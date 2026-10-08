pub mod project;
#[cfg(feature = "scaffold")]
pub mod scaffold;
#[cfg(test)]
mod tests;

pub use project::{Folder, Module, Project};
#[cfg(feature = "scaffold")]
pub use scaffold::{check_name, scaffold};
pub use toml::Table;
