use std::io::IsTerminal;

pub(super) fn width() -> Option<usize> {
    let out = std::io::stdout();
    if !out.is_terminal() {
        return None;
    }
    terminal_size::terminal_size_of(out).map(|(w, _)| usize::from(w.0))
}

pub(super) fn change(file: &str, key: &str, new: &str, was: &str, brief: bool) -> String {
    if brief {
        format!("{key} = {new}")
    } else {
        format!("{file}: {key} = {new}  (was {was})")
    }
}

fn pieces(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b' ' && (i + 1 == bytes.len() || bytes[i + 1] != b' ') {
            out.push(&line[start..=i]);
            start = i + 1;
        }
    }
    if start < line.len() {
        out.push(&line[start..]);
    }
    out
}

fn indent(line: &str, width: usize) -> usize {
    let at = line
        .find(" = ")
        .map_or(0, |i| line[..i].chars().count() + 3);
    if at * 2 <= width { at } else { 2 }
}

pub(super) fn wrapped(line: &str, width: Option<usize>) -> String {
    let Some(width) = width.filter(|&w| w > 0 && line.chars().count() > w) else {
        return line.to_string();
    };
    let lead = " ".repeat(indent(line, width));
    let mut rows: Vec<String> = Vec::new();
    let mut row = String::new();
    for piece in pieces(line) {
        let fits = row.chars().count() + piece.trim_end().chars().count() <= width;
        if fits || row.trim().is_empty() {
            row.push_str(piece);
            continue;
        }
        rows.push(row.trim_end().to_string());
        row = format!("{lead}{piece}");
    }
    rows.push(row.trim_end().to_string());
    rows.join("\n")
}

pub(super) fn emit(line: &str, width: Option<usize>) {
    println!("{}", wrapped(line, width));
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: &str =
        "scene.pitofx: materials.slate.base = [0.085, 0.09, 0.097]  (was [0.5, 0.5, 0.5])";

    #[test]
    fn the_brief_form_drops_the_file_and_the_old_value() {
        assert_eq!(
            change("scene.pitofx", "look.exposure", "2.5", "3.0", true),
            "look.exposure = 2.5"
        );
        assert_eq!(
            change("scene.pitofx", "look.exposure", "2.5", "absent", false),
            "scene.pitofx: look.exposure = 2.5  (was absent)"
        );
    }

    #[test]
    fn a_line_wider_than_the_terminal_breaks_at_commas_and_spaces_only() {
        let text = wrapped(LONG, Some(40));
        let rows: Vec<&str> = text.lines().collect();
        assert!(rows.len() > 1, "{text}");
        assert!(rows.iter().all(|r| r.chars().count() <= 40), "{text}");
        let joined: Vec<&str> = text.split_whitespace().collect();
        assert_eq!(joined, LONG.split_whitespace().collect::<Vec<_>>());
    }

    #[test]
    fn a_brief_value_wraps_under_its_first_number() {
        let line = "object.hero.at = [1.25, 2.5, 3.75, 4.125]";
        assert_eq!(
            wrapped(line, Some(40)),
            "object.hero.at = [1.25, 2.5, 3.75,\n                 4.125]"
        );
    }

    #[test]
    fn piped_or_short_lines_stay_one_line() {
        assert_eq!(wrapped(LONG, None), LONG);
        assert_eq!(wrapped(LONG, Some(200)), LONG);
    }

    #[test]
    fn a_word_longer_than_the_width_is_never_split() {
        let line = "k = averyveryverylongwordwithoutbreaks";
        assert_eq!(
            wrapped(line, Some(10)),
            "k =\n    averyveryverylongwordwithoutbreaks"
        );
    }
}
