use std::any::Any;
use std::fmt::Debug;
use std::ops::Range;
use std::path::PathBuf;

use crate::error::DocError;
use crate::files::Files;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caret {
    pub file: PathBuf,
    pub selection: Range<usize>,
}

pub trait Edit: Any + Debug {
    fn label(&self) -> &str;

    fn files(&self) -> Vec<PathBuf>;

    fn apply(&mut self, files: &mut Files) -> Result<(), DocError>;

    fn inverse(&self) -> Box<dyn Edit>;

    fn merge(&self, _next: &dyn Edit) -> Option<Box<dyn Edit>> {
        None
    }

    fn caret(&self) -> Option<Caret> {
        None
    }
}

pub fn downcast<T: Edit>(edit: &dyn Edit) -> Option<&T> {
    let any: &dyn Any = edit;
    any.downcast_ref::<T>()
}
