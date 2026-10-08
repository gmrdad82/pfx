use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use pfx_load::scene::{EditError, SceneError};

pub const KEEP: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub level: Level,
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
    pub text: String,
}

impl Entry {
    pub fn place(&self, root: &Path) -> Option<String> {
        let file = self.file.as_ref()?;
        let name = crate::outline::relative(root, file);
        Some(match self.line {
            Some(line) => format!("{name}:{line}"),
            None => name,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Log {
    pub entries: VecDeque<Entry>,
}

impl Log {
    pub fn push(&mut self, entry: Entry) {
        if self.entries.len() == KEEP {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.say(Level::Info, text);
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        self.say(Level::Warn, text);
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.say(Level::Error, text);
    }

    fn say(&mut self, level: Level, text: impl Into<String>) {
        self.push(Entry {
            level,
            file: None,
            line: None,
            text: text.into(),
        });
    }

    fn see(&mut self, file: &Path, line: usize) {
        self.push(Entry {
            level: Level::Error,
            file: Some(file.to_path_buf()),
            line: Some(line),
            text: "see here".to_string(),
        });
    }

    pub fn scene_error(&mut self, error: &SceneError) {
        self.push(Entry {
            level: Level::Error,
            file: Some(error.file.clone()),
            line: error.line,
            text: error.message.clone(),
        });
        for place in &error.related {
            self.see(&place.file, place.line);
        }
    }

    pub fn edit_error(&mut self, error: &EditError) {
        self.push(Entry {
            level: Level::Error,
            file: Some(error.file.clone()),
            line: error.line.map(|line| line as usize),
            text: format!("{}: {}", error.key, error.message),
        });
        let root = pfx_scene::Project::root_of(&error.file);
        for found in error.diagnostics.iter().filter(|found| found.is_error()) {
            for place in &found.related {
                self.see(&root.join(&place.file), place.line as usize);
            }
        }
    }

    pub fn last(&self) -> Option<&Entry> {
        self.entries.back()
    }

    pub fn errors(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.level == Level::Error)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_refused_with_a_chain_logs_each_place_of_it() {
        let mut log = Log::default();
        let diagnostic = |file: &str, related: Vec<pfx_scene::Location>| pfx_scene::Diagnostic {
            file: PathBuf::from(file),
            line: 3,
            column: 12,
            end_line: 3,
            end_column: 20,
            severity: pfx_scene::Severity::Error,
            code: pfx_scene::code::INCLUDE_CYCLE,
            key: "include.0".to_string(),
            message: "the scene includes itself".to_string(),
            related,
        };
        let place = |file: &str| pfx_scene::Location {
            file: PathBuf::from(file),
            line: 3,
            column: 12,
        };
        log.edit_error(&EditError {
            file: PathBuf::from("/game/content/other.scene.toml"),
            key: "include".to_string(),
            line: Some(3),
            code: pfx_scene::code::INCLUDE_CYCLE,
            message: "the scene includes itself".to_string(),
            diagnostics: vec![diagnostic(
                "content/other.scene.toml",
                vec![
                    place("content/main.scene.toml"),
                    place("content/other.scene.toml"),
                ],
            )],
        });
        let seen: Vec<(Option<PathBuf>, Option<usize>, &str)> = log
            .entries
            .iter()
            .map(|entry| (entry.file.clone(), entry.line, entry.text.as_str()))
            .collect();
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert_eq!(seen[0].2, "include: the scene includes itself");
        assert_eq!(seen[1].2, "see here");
        assert_eq!(seen[1].1, Some(3));
        assert!(
            seen[1]
                .0
                .as_ref()
                .unwrap()
                .ends_with("content/main.scene.toml")
        );
        assert!(
            seen[2]
                .0
                .as_ref()
                .unwrap()
                .ends_with("content/other.scene.toml")
        );
    }

    #[test]
    fn errors_keep_their_file_and_line_and_the_log_keeps_its_last_entries() {
        let mut log = Log::default();
        log.scene_error(&SceneError::new(
            Path::new("/scenes/room.scene.toml"),
            Some(14),
            "unknown field 'colour'",
        ));
        let entry = log.last().unwrap();
        assert_eq!(
            entry.place(Path::new("/scenes/room.scene.toml")).unwrap(),
            "room.scene.toml:14"
        );
        for index in 0..KEEP + 3 {
            log.info(format!("line {index}"));
        }
        assert_eq!(log.entries.len(), KEEP);
        assert_eq!(log.last().unwrap().text, format!("line {}", KEEP + 2));
        assert_eq!(log.errors(), 0);
    }
}
