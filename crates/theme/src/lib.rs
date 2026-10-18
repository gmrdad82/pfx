use egui::{Color32, RichText, WidgetText};
use pfx_editor_style::Theme;

pub use pfx_editor_style as style;
pub use pfx_editor_style::tokens::*;

pub fn pfx() -> Theme {
    theme()
}

pub fn label(theme: &Theme, text: &str, colour: Color32) -> WidgetText {
    theme
        .labels
        .job(text, theme.label_font(theme.size_code), colour)
        .into()
}

pub fn small(theme: &Theme, text: impl Into<String>, colour: Color32) -> RichText {
    RichText::new(text)
        .font(theme.font(theme.size_small))
        .color(colour)
}

pub fn code(theme: &Theme, text: impl Into<String>, colour: Color32) -> RichText {
    RichText::new(text)
        .font(theme.font(theme.size_code))
        .color(colour)
}

pub fn title(theme: &Theme, text: &str, size: f32, colour: Color32) -> WidgetText {
    theme
        .titles
        .job(text, theme.title_font(size), colour)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_and_labels_are_uppercase_tracked_jost_at_their_weights() {
        let theme = pfx();
        let ctx = egui::Context::default();
        pfx_editor_style::theme::apply_theme(&ctx, &theme);
        let job = theme
            .titles
            .job("outliner", theme.title_font(12.0), theme.text);
        assert_eq!(job.text, "OUTLINER");
        assert!((job.sections[0].format.extra_letter_spacing - 3.6).abs() < 1e-5);
        assert_eq!(
            job.sections[0].format.font_id.family,
            pfx_editor_style::theme::display()
        );
        let data = theme
            .labels
            .job("roughness", theme.label_font(11.0), theme.text2);
        assert_eq!(data.text, "ROUGHNESS");
        assert_eq!(data.sections[0].format.coords.as_ref()[0].1, LABEL_WEIGHT);
        assert_eq!(theme.display.as_ref().unwrap().weight, Some(TITLE_WEIGHT));
        ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.label(title(&theme, "pfx gui", 13.0, theme.text));
                ui.label(label(&theme, "roughness", theme.text2));
            });
        })
        .textures_delta
        .clear();
        assert_eq!(*Theme::of(&ctx), theme);
        assert_eq!(Theme::of(&ctx).bg, BG);
    }
}
