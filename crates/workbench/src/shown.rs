use std::path::{Component, Path, PathBuf};

const OPENS: [char; 11] = [' ', '\t', '\n', '(', '[', '"', '\'', '`', ',', '=', ':'];
const CLOSES: [char; 9] = [' ', '\t', '\n', ')', ']', '"', '\'', '`', ','];

fn plain(path: &Path) -> bool {
    path.is_relative()
        && !path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::Prefix(_)))
}

fn slashed(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn named(path: &Path) -> String {
    let typed = path.to_string_lossy();
    typed
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_string()
}

fn project_of(path: &Path) -> Option<&Path> {
    path.ancestors()
        .skip(1)
        .find(|folder| folder.join(pfx_project::project::FILE).is_file())
}

pub fn path(path: &Path) -> String {
    let typed = path.to_string_lossy();
    if typed.is_empty() || typed.starts_with('~') || typed.contains('\\') {
        return named(path);
    }
    if plain(path) {
        return slashed(path);
    }
    let whole = pfx_recipe::files::clean(path);
    let inside = |folder: &Path| {
        whole
            .strip_prefix(folder)
            .ok()
            .filter(|rest| !rest.as_os_str().is_empty())
            .map(slashed)
    };
    if let Some(found) = project_of(&whole).and_then(inside) {
        return found;
    }
    let here = std::env::current_dir().map(|here| pfx_recipe::files::clean(&here));
    if let Some(found) = here.ok().as_deref().and_then(inside) {
        return found;
    }
    named(&whole)
}

fn starts(text: &str, at: usize) -> bool {
    let rest = &text[at..];
    let mut chars = rest.chars();
    let absolute = match chars.next() {
        Some('/') => chars.next().is_some_and(|c| !c.is_whitespace() && c != '/'),
        Some('~') => chars.next() == Some('/'),
        Some(drive) if drive.is_ascii_alphabetic() => {
            chars.next() == Some(':') && matches!(chars.next(), Some('\\' | '/'))
        }
        _ => false,
    };
    absolute
        && text[..at]
            .chars()
            .next_back()
            .is_none_or(|before| OPENS.contains(&before))
}

fn tail(token: &str) -> usize {
    let mut end = token.len();
    loop {
        let kept = &token[..end];
        let digits = kept.trim_end_matches(|c: char| c.is_ascii_digit());
        if kept.ends_with(['.', ':', ';']) {
            end -= 1;
        } else if digits.len() < kept.len() && digits.ends_with(':') {
            end = digits.len() - 1;
        } else {
            return end;
        }
    }
}

pub fn text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        if starts(text, at) {
            let length = text[at..].find(CLOSES).unwrap_or(text.len() - at);
            let token = &text[at..at + length];
            let end = tail(token);
            out.push_str(&path(&PathBuf::from(&token[..end])));
            out.push_str(&token[end..]);
            at += length;
            continue;
        }
        let next = text[at..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&text[at..at + next]);
        at += next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_shows_as_typed_project_relative_or_by_its_name_alone() {
        let dir = crate::scratch("shown-project").with_file_name("game");
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        std::fs::write(dir.join(pfx_project::project::FILE), "").unwrap();
        assert_eq!(path(&dir.join("scenes/room.pitofx")), "scenes/room.pitofx");
        assert_eq!(path(Path::new("samples/car.toml")), "samples/car.toml");
        assert_eq!(path(Path::new("/nowhere/at/all/car.toml")), "car.toml");
        assert_eq!(path(Path::new("~/models/car.glb")), "car.glb");
        assert_eq!(path(Path::new("../../../car.glb")), "car.glb");
    }

    #[test]
    fn absolute_paths_in_a_line_lose_their_folders_and_keep_what_follows() {
        assert_eq!(
            text("../a.glb is not a file in any of /x/y/doc, /x/z/roots"),
            "../a.glb is not a file in any of doc, roots"
        );
        assert_eq!(
            text("error: /x/y/r.toml:7:3: no such key"),
            "error: r.toml:7:3: no such key"
        );
        assert_eq!(
            text("cannot read \"/x/m.glb\" (C:\\x\\n.glb)."),
            "cannot read \"m.glb\" (n.glb)."
        );
        assert_eq!(
            text("kept as ~/m/a.pitofx; 1/2 W/m² a / b"),
            "kept as a.pitofx; 1/2 W/m² a / b"
        );
    }
}
