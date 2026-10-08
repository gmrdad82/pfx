use std::path::{Path, PathBuf};

use pfx_editor_doc::toml_edit::DocumentMut;
use pfx_editor_toml::{Check, Diagnostic, Location, Severity};
use pfx_scene::Project;

pub struct SceneCheck<'a> {
    project: &'a Project,
}

fn severity(found: pfx_scene::Severity) -> Severity {
    match found {
        pfx_scene::Severity::Error => Severity::Error,
        pfx_scene::Severity::Warning => Severity::Warning,
    }
}

impl<'a> SceneCheck<'a> {
    pub fn new(project: &'a Project) -> SceneCheck<'a> {
        SceneCheck { project }
    }

    fn place(&self, file: &Path) -> PathBuf {
        if file.is_absolute() {
            file.to_path_buf()
        } else {
            self.project.root().join(file)
        }
    }

    pub fn convert(&self, found: pfx_scene::Diagnostic, file: Option<&Path>) -> Diagnostic {
        Diagnostic {
            file: file.map_or_else(|| self.place(&found.file), Path::to_path_buf),
            line: found.line,
            column: found.column,
            end_line: found.end_line,
            end_column: found.end_column,
            severity: severity(found.severity),
            code: found.code.to_string(),
            key: found.key,
            message: found.message,
            related: found
                .related
                .into_iter()
                .map(|related| Location {
                    file: self.place(&related.file),
                    line: related.line,
                    column: related.column,
                })
                .collect(),
        }
    }

    pub fn all(&self) -> Vec<Diagnostic> {
        self.project
            .check()
            .into_iter()
            .map(|found| self.convert(found, None))
            .collect()
    }
}

impl Check for SceneCheck<'_> {
    fn check(&self, file: &Path, text: &str, _doc: &DocumentMut) -> Vec<Diagnostic> {
        pfx_scene::check(text, file, self.project)
            .into_iter()
            .map(|found| self.convert(found, Some(file)))
            .collect()
    }
}

pub fn worded(problem: &Diagnostic, root: &Path) -> String {
    let file = crate::outline::relative(root, &problem.file);
    let kind = match problem.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    format!(
        "{file}:{}:{}: {kind}: {}",
        problem.line, problem.column, problem.message
    )
}
