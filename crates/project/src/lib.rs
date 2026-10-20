pub mod marks;
pub mod project;
#[cfg(feature = "scaffold")]
pub mod scaffold;
#[cfg(test)]
mod tests;
#[cfg(feature = "scaffold")]
pub mod title;

pub use project::{Folder, Module, Project};
#[cfg(feature = "scaffold")]
pub use scaffold::{check_name, scaffold};
#[doc(no_inline)]
pub use toml::Table;
