use std::ops::Range;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, TomlError};

pub const PARSE: &str = "parse";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Location {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub severity: Severity,
    pub code: String,
    pub key: String,
    pub message: String,
    pub related: Vec<Location>,
}

pub trait Check {
    fn check(&self, file: &Path, text: &str, doc: &DocumentMut) -> Vec<Diagnostic>;
}

pub struct NoCheck;

impl Check for NoCheck {
    fn check(&self, _: &Path, _: &str, _: &DocumentMut) -> Vec<Diagnostic> {
        Vec::new()
    }
}

impl<F: Fn(&Path, &str, &DocumentMut) -> Vec<Diagnostic>> Check for F {
    fn check(&self, file: &Path, text: &str, doc: &DocumentMut) -> Vec<Diagnostic> {
        self(file, text, doc)
    }
}

fn floor(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

pub fn position(text: &str, byte: usize) -> (u32, u32) {
    let byte = floor(text, byte);
    let before = &text[..byte];
    let line_start = before.rfind('\n').map_or(0, |at| at + 1);
    let line = before.bytes().filter(|b| *b == b'\n').count() + 1;
    let column = before[line_start..].chars().count() + 1;
    (line as u32, column as u32)
}

pub fn offset(text: &str, line: u32, column: u32) -> usize {
    let mut start = 0;
    for _ in 1..line.max(1) {
        match text[start..].find('\n') {
            Some(at) => start += at + 1,
            None => return text.len(),
        }
    }
    let end = text[start..].find('\n').map_or(text.len(), |at| start + at);
    text[start..end]
        .char_indices()
        .nth(column.max(1) as usize - 1)
        .map_or(end, |(at, _)| start + at)
}

pub fn parse(file: &Path, text: &str) -> Result<DocumentMut, Box<Diagnostic>> {
    text.parse::<DocumentMut>()
        .map_err(|error| Box::new(Diagnostic::parse(file, text, &error)))
}

impl Diagnostic {
    pub fn at(
        file: impl Into<PathBuf>,
        text: &str,
        range: Range<usize>,
        severity: Severity,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Diagnostic {
        let (line, column) = position(text, range.start);
        let (end_line, end_column) = position(text, range.end.max(range.start));
        Diagnostic {
            file: file.into(),
            line,
            column,
            end_line,
            end_column,
            severity,
            code: code.into(),
            key: String::new(),
            message: message.into(),
            related: Vec::new(),
        }
    }

    pub fn parse(file: &Path, text: &str, error: &TomlError) -> Diagnostic {
        let range = error.span().unwrap_or(0..0);
        Diagnostic::at(
            file,
            text,
            range,
            Severity::Error,
            PARSE,
            error.message().trim(),
        )
    }

    pub fn start(&self, text: &str) -> usize {
        offset(text, self.line, self.column)
    }

    pub fn span(&self, text: &str) -> Range<usize> {
        let start = self.start(text);
        let end = offset(text, self.end_line, self.end_column).max(start);
        if start < end {
            return start..end;
        }
        match text[start..].chars().next() {
            Some(next) if next != '\n' => start..start + next.len_utf8(),
            _ => match text[..start].chars().next_back() {
                Some(previous) if previous != '\n' => start - previous.len_utf8()..start,
                _ => start..start,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "scene.toml";

    #[test]
    fn positions_count_lines_and_characters_from_one() {
        let text = "a = \"héllo wörld\"\nb = 'ünï' # ✓\n";
        assert_eq!(position(text, 0), (1, 1));
        let b = text.find('b').unwrap();
        assert_eq!(position(text, b), (2, 1));
        let tick = text.find('✓').unwrap();
        assert_eq!(position(text, tick), (2, 13));
        assert_eq!(position(text, tick + 1), (2, 13));
        assert_eq!(position(text, text.len()), (3, 1));
        for byte in 0..=text.len() {
            if text.is_char_boundary(byte) {
                let (line, column) = position(text, byte);
                assert_eq!(offset(text, line, column), byte, "{byte}");
            }
        }
        assert_eq!(offset(text, 1, 99), text.find('\n').unwrap());
        assert_eq!(offset(text, 9, 1), text.len());
    }

    #[test]
    fn a_parse_error_lands_on_its_line_and_character_column() {
        let text = "name = \"héllo wörld\"\n[obj]\nbad = = 1\n";
        let error = parse(Path::new(FILE), text).unwrap_err();
        assert_eq!(error.severity, Severity::Error);
        assert_eq!(error.code, PARSE);
        assert_eq!(error.file, Path::new(FILE));
        assert_eq!((error.line, error.column), (3, 7));
        assert!(!error.message.is_empty());
        let span = error.span(text);
        assert_eq!(&text[span], "=");
    }

    #[test]
    fn a_parse_error_after_wide_characters_counts_characters() {
        let text = "ключ = \"значение\" junk\n";
        let error = parse(Path::new(FILE), text).unwrap_err();
        assert_eq!(error.line, 1);
        assert_eq!(error.column, 19);
        let span = error.span(text);
        assert!(text[span.clone()].starts_with('j'), "{:?}", &text[span]);
    }

    #[test]
    fn an_app_diagnostic_maps_back_to_its_bytes() {
        let text = "[obj]\nnäme = \"ä\"\ncolor = [1, 2]\n";
        let diagnostic = Diagnostic {
            file: FILE.into(),
            line: 3,
            column: 9,
            end_line: 3,
            end_column: 15,
            severity: Severity::Warning,
            code: "short-color".into(),
            key: "obj.color".into(),
            message: "a colour has three channels".into(),
            related: Vec::new(),
        };
        assert_eq!(&text[diagnostic.span(text)], "[1, 2]");
        let wide = Diagnostic {
            line: 2,
            column: 2,
            end_line: 2,
            end_column: 3,
            ..diagnostic.clone()
        };
        assert_eq!(&text[wide.span(text)], "ä");
        let range = text.find("\"ä\"").unwrap()..text.find("\"ä\"").unwrap() + "\"ä\"".len();
        let made = Diagnostic::at(FILE, text, range.clone(), Severity::Error, "x", "y");
        assert_eq!((made.line, made.column, made.end_column), (2, 8, 11));
        assert_eq!(made.span(text), range);
    }

    #[test]
    fn an_empty_span_widens_to_one_character() {
        let text = "a = \nb = 1";
        let at = |line, column| Diagnostic {
            line,
            column,
            end_line: line,
            end_column: column,
            ..Diagnostic::at(FILE, text, 0..0, Severity::Error, PARSE, "")
        };
        assert_eq!(&text[at(1, 1).span(text)], "a");
        assert_eq!(&text[at(1, 5).span(text)], " ");
        assert_eq!(&text[at(2, 6).span(text)], "1");
        assert_eq!(at(1, 1).span(""), 0..0);
    }

    #[test]
    fn untouched_text_round_trips_byte_for_byte() {
        let text = "# a scene\n\n[object.lamp]   # the lamp\nname   =   \"lamp\"\nat = [ 0.0,1.5 ,  -2 ]\n\n[[light]] \n  color = { r = 1,g=0 }\ndoc = '''\nraw\n'''\n";
        let doc = parse(Path::new(FILE), text).unwrap();
        assert_eq!(doc.to_string(), text);
    }
}
