use std::ops::Range;
use std::path::PathBuf;

use crate::splice::TextSplice;

pub const MARK: &str = "#";
pub const PREFIX: &str = "# ";
pub const COMMENT: &str = "comment";
pub const UNCOMMENT: &str = "uncomment";

struct Line {
    old: usize,
    new: usize,
    at: usize,
    added: usize,
    removed: usize,
}

fn line_start(text: &str, byte: usize) -> usize {
    text[..byte].rfind('\n').map_or(0, |at| at + 1)
}

fn line_end(text: &str, byte: usize) -> usize {
    text[byte..].find('\n').map_or(text.len(), |at| byte + at)
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

fn blank(line: &str) -> bool {
    line.trim().is_empty()
}

fn floor(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

pub fn toggle_comment(
    file: impl Into<PathBuf>,
    text: &str,
    selection: Range<usize>,
) -> Option<TextSplice> {
    let low = floor(text, selection.start.min(selection.end));
    let high = floor(text, selection.start.max(selection.end));
    let start = line_start(text, low);
    let mut last = high;
    if high > low && line_start(text, high) == high {
        last = high - 1;
    }
    let end = line_end(text, last.max(start));
    let region = &text[start..end];
    let lines: Vec<&str> = region.split('\n').collect();
    let filled: Vec<&&str> = lines.iter().filter(|line| !blank(line)).collect();
    if filled.is_empty() {
        return None;
    }
    let remove = filled
        .iter()
        .all(|line| line[indent(line)..].starts_with(MARK));
    let column = filled.iter().map(|line| indent(line)).min().unwrap_or(0);
    let mut new = String::with_capacity(region.len() + lines.len() * PREFIX.len());
    let mut map = Vec::with_capacity(lines.len());
    let mut old_at = start;
    for (n, line) in lines.iter().enumerate() {
        if n > 0 {
            new.push('\n');
        }
        let new_at = start + new.len();
        let mut entry = Line {
            old: old_at,
            new: new_at,
            at: 0,
            added: 0,
            removed: 0,
        };
        if blank(line) {
            new.push_str(line);
        } else if remove {
            let at = indent(line);
            let cut = if line[at..].starts_with(PREFIX) {
                PREFIX.len()
            } else {
                MARK.len()
            };
            new.push_str(&line[..at]);
            new.push_str(&line[at + cut..]);
            entry.at = at;
            entry.removed = cut;
        } else {
            new.push_str(&line[..column]);
            new.push_str(PREFIX);
            new.push_str(&line[column..]);
            entry.at = column;
            entry.added = PREFIX.len();
        }
        map.push(entry);
        old_at += line.len() + 1;
    }
    let shift = new.len() as isize - region.len() as isize;
    let moved = |byte: usize, left: bool| -> usize {
        if byte < start {
            return byte;
        }
        if byte > end {
            return (byte as isize + shift) as usize;
        }
        let line = map
            .iter()
            .rev()
            .find(|line| line.old <= byte)
            .expect("the region starts a line");
        let column = byte - line.old;
        let column = if line.added > 0 && (column > line.at || (column == line.at && !left)) {
            column + line.added
        } else if line.removed > 0 && column >= line.at + line.removed {
            column - line.removed
        } else if line.removed > 0 && column > line.at {
            line.at
        } else {
            column
        };
        line.new + column
    };
    let wide = selection.start != selection.end;
    let caret = moved(selection.start, wide && selection.start < selection.end)
        ..moved(selection.end, wide && selection.end < selection.start);
    Some(
        TextSplice::new(file, start..end, region, new)
            .labelled(if remove { UNCOMMENT } else { COMMENT })
            .with_carets(selection, caret),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::edit::Edit;
    use crate::files::Files;

    fn toggled(text: &str, selection: Range<usize>) -> (String, Range<usize>, String) {
        let splice = toggle_comment("a.toml", text, selection).expect("a splice");
        let mut files = Files::new();
        files.open("a.toml", text);
        let mut edit = splice.clone();
        edit.apply(&mut files).unwrap();
        (
            files.text(Path::new("a.toml")).unwrap().to_string(),
            splice.caret_after.clone(),
            splice.label.clone(),
        )
    }

    #[test]
    fn mixed_indented_and_blank_lines_comment_at_the_least_indent() {
        let text = "[a]\n  x = 1\n\n    y = 2\n  # z = 3\nw = 4\n";
        let (out, _, label) = toggled(text, 4..25);
        assert_eq!(out, "[a]\n  # x = 1\n\n  #   y = 2\n  # # z = 3\nw = 4\n");
        assert_eq!(label, COMMENT);
    }

    #[test]
    fn lines_all_commented_are_uncommented_keeping_their_indent() {
        let text = "  # x = 1\n\n    #y = 2\n  #   z = 3\n";
        let (out, _, label) = toggled(text, 0..text.len());
        assert_eq!(out, "  x = 1\n\n    y = 2\n    z = 3\n");
        assert_eq!(label, UNCOMMENT);
    }

    #[test]
    fn toggling_twice_restores_the_text() {
        let text = "a = 1\n\tb = 2\n\n  c = \"ä\"\n";
        let (once, caret, _) = toggled(text, 0..text.len() - 1);
        let (twice, _, _) = toggled(&once, caret);
        assert_eq!(twice, text);
    }

    #[test]
    fn a_caret_toggles_its_own_line_and_moves_with_the_text() {
        let text = "a = 1\n  b = 2\nc = 3\n";
        let (out, caret, _) = toggled(text, 9..9);
        assert_eq!(out, "a = 1\n  # b = 2\nc = 3\n");
        assert_eq!(caret, 11..11);
        let (back, caret, _) = toggled(&out, 11..11);
        assert_eq!(back, text);
        assert_eq!(caret, 9..9);
    }

    #[test]
    fn a_selection_ending_at_a_line_start_leaves_that_line() {
        let text = "a = 1\nb = 2\n";
        let (out, caret, _) = toggled(text, 0..6);
        assert_eq!(out, "# a = 1\nb = 2\n");
        assert_eq!(caret, 0..8);
    }

    #[test]
    fn blank_lines_alone_toggle_nothing() {
        assert!(toggle_comment("a.toml", "\n  \n\n", 0..4).is_none());
    }
}
