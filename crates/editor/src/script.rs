use std::path::Path;

use pfx_editor_shell::{Script, drive};

use crate::editor::{Editor, Layout};

pub fn place(text: &str, layout: &Layout) -> Result<String, String> {
    let mut lines = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with('#') {
            lines.push(line.to_string());
            continue;
        }
        let mut words = Vec::new();
        for word in line.split_whitespace() {
            match word.strip_prefix('@') {
                Some(name) => {
                    let [x, y] = layout.find(name).ok_or_else(|| {
                        format!("line {}: nothing on screen is named {name}", index + 1)
                    })?;
                    words.push(format!("{x} {y}"));
                }
                None => words.push(word.to_string()),
            }
        }
        lines.push(words.join(" "));
    }
    Ok(lines.join("\n"))
}

pub fn script(text: &str, layout: &Layout) -> Result<Script, String> {
    Script::parse(&place(text, layout)?).map_err(|error| error.to_string())
}

pub fn resolve(path: &Path, size: [u32; 2], text: &str) -> Result<Script, String> {
    let mut done: Vec<String> = Vec::new();
    for line in text.lines() {
        if !line.contains('@') || line.trim_start().starts_with('#') {
            done.push(line.to_string());
            continue;
        }
        let mut editor = Editor::open(path)?;
        let ctx = egui::Context::default();
        pfx_editor_style::theme::apply_theme(&ctx, &crate::theme::pfx());
        let before = Script::parse(&done.join("\n")).map_err(|error| error.to_string())?;
        drive(&ctx, size, &before, |ctx| editor.ui(ctx));
        done.push(place(line, &editor.layout)?);
    }
    Script::parse(&done.join("\n")).map_err(|error| error.to_string())
}

pub fn frames(count: usize) -> String {
    "frame\n".repeat(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_points_become_coordinates_and_an_unknown_name_names_its_line() {
        let mut layout = Layout::default();
        layout.buttons.insert(
            "move",
            egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(20.0, 10.0)),
        );
        layout.viewport = Some(egui::Rect::from_min_size(
            egui::pos2(100.0, 0.0),
            egui::vec2(200.0, 100.0),
        ));
        assert_eq!(
            place(
                "# @button:move stays a comment\nclick @button:move\ndrag @viewport @viewport:0.1,0.9 middle\nkey ctrl+Z",
                &layout
            )
            .unwrap(),
            "# @button:move stays a comment\nclick 20 25\ndrag 200 50 120 90 middle\nkey ctrl+Z"
        );
        let error = place("frame\nclick @button:gone", &layout).unwrap_err();
        assert!(error.starts_with("line 2:"), "{error}");
        assert_eq!(
            script("click @button:move", &layout).unwrap().steps.len(),
            1
        );
        assert!(script("wiggle 3", &layout).is_err());
        assert_eq!(frames(2), "frame\nframe\n");
    }
}
