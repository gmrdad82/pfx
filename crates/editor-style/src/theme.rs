use std::borrow::Cow;
use std::sync::Arc;

use egui::style::{Selection, WidgetVisuals, Widgets};
use egui::text::LayoutJob;
use egui::{
    Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Id, Margin,
    Shadow, Stroke, Style, TextStyle, ThemePreference, Vec2, Visuals,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Face {
    pub name: Cow<'static, str>,
    pub regular: Cow<'static, [u8]>,
    pub bold: Cow<'static, [u8]>,
}

impl Face {
    pub fn regular_name(&self) -> String {
        format!("{}-Regular", self.name)
    }

    pub fn bold_name(&self) -> String {
        format!("{}-Bold", self.name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Display {
    pub name: Cow<'static, str>,
    pub bytes: Cow<'static, [u8]>,
    pub licence: Cow<'static, str>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caps {
    pub upper: bool,
    pub tracking: f32,
}

impl Caps {
    pub const PLAIN: Caps = Caps {
        upper: false,
        tracking: 0.0,
    };

    pub fn text<'a>(self, text: &'a str) -> Cow<'a, str> {
        if self.upper {
            Cow::Owned(text.to_uppercase())
        } else {
            Cow::Borrowed(text)
        }
    }

    pub fn job(self, text: &str, font: FontId, colour: Color32) -> LayoutJob {
        let spacing = self.tracking * font.size;
        let mut job = LayoutJob::simple(self.text(text).into_owned(), font, colour, f32::INFINITY);
        for section in &mut job.sections {
            section.format.extra_letter_spacing = spacing;
        }
        job
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Syntax {
    pub key: Color32,
    pub table: Color32,
    pub string: Color32,
    pub number: Color32,
    pub boolean: Color32,
    pub date: Color32,
    pub comment: Color32,
    pub punctuation: Color32,
    pub plain: Color32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Diff {
    pub add: Color32,
    pub del: Color32,
    pub change: Color32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: Cow<'static, str>,
    pub bg: Color32,
    pub surface: Color32,
    pub well: Color32,
    pub line: Color32,
    pub track: Color32,
    pub text: Color32,
    pub text2: Color32,
    pub muted: Color32,
    pub dim: Color32,
    pub code: Color32,
    pub on_accent: Color32,
    pub accent: Color32,
    pub accent_bright: Color32,
    pub accent_deep: Color32,
    pub secondary: Color32,
    pub secondary_soft: Color32,
    pub hot: Color32,
    pub hot_dim: Color32,
    pub ok: Color32,
    pub ok_tint: Color32,
    pub error: Color32,
    pub error_tint: Color32,
    pub warning: Color32,
    pub syntax: Syntax,
    pub diff: Diff,
    pub size_code: f32,
    pub size_small: f32,
    pub size_body: f32,
    pub size_heading: f32,
    pub size_title: f32,
    pub radius: u8,
    pub stroke: f32,
    pub panel_padding: i8,
    pub item_spacing: f32,
    pub section_gap: f32,
    pub dash: f32,
    pub row_height: f32,
    pub body: Face,
    pub display: Option<Display>,
    pub titles: Caps,
    pub labels: Caps,
}

impl Theme {
    pub fn of(ctx: &Context) -> Arc<Theme> {
        ctx.data(|data| data.get_temp::<Arc<Theme>>(key()))
            .unwrap_or_else(|| panic!("{UNTHEMED}"))
    }

    pub fn font(&self, size: f32) -> FontId {
        FontId::new(size, FontFamily::Monospace)
    }

    pub fn bold_font(&self, size: f32) -> FontId {
        FontId::new(size, bold())
    }

    pub fn title_font(&self, size: f32) -> FontId {
        match self.display {
            Some(_) => FontId::new(size, display()),
            None => self.font(size),
        }
    }

    pub fn heading_font(&self, size: f32) -> FontId {
        match self.display {
            Some(_) => FontId::new(size, display()),
            None => self.bold_font(size),
        }
    }
}

pub const UNTHEMED: &str = "no theme applied: call theme::apply_theme first";

fn key() -> Id {
    Id::new("editor style theme")
}

pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn display() -> FontFamily {
    FontFamily::Name("display".into())
}

pub fn code() -> TextStyle {
    TextStyle::Name("code".into())
}

pub fn title() -> TextStyle {
    TextStyle::Name("title".into())
}

fn data(bytes: Cow<'static, [u8]>) -> Arc<FontData> {
    Arc::new(match bytes {
        Cow::Borrowed(bytes) => FontData::from_static(bytes),
        Cow::Owned(bytes) => FontData::from_owned(bytes),
    })
}

pub fn fonts(theme: &Theme) -> FontDefinitions {
    let regular = theme.body.regular_name();
    let bold_name = theme.body.bold_name();
    let mut fonts = FontDefinitions::empty();
    fonts
        .font_data
        .insert(regular.clone(), data(theme.body.regular.clone()));
    fonts
        .font_data
        .insert(bold_name.clone(), data(theme.body.bold.clone()));
    fonts
        .families
        .insert(FontFamily::Proportional, vec![regular.clone()]);
    fonts
        .families
        .insert(FontFamily::Monospace, vec![regular.clone()]);
    fonts
        .families
        .insert(bold(), vec![bold_name.clone(), regular.clone()]);
    if let Some(face) = &theme.display {
        let name = face.name.to_string();
        fonts
            .font_data
            .insert(name.clone(), data(face.bytes.clone()));
        fonts
            .families
            .insert(display(), vec![name, bold_name, regular]);
    }
    fonts
}

fn widget(theme: &Theme, fill: Color32, border: Color32, text: Color32) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(theme.stroke, border),
        corner_radius: CornerRadius::same(theme.radius),
        fg_stroke: Stroke::new(theme.stroke, text),
        expansion: 0.0,
    }
}

pub fn visuals(theme: &Theme) -> Visuals {
    let mut visuals = Visuals::dark();
    visuals.dark_mode = true;
    visuals.override_text_color = None;
    visuals.weak_text_color = Some(theme.muted);
    visuals.widgets = Widgets {
        noninteractive: widget(theme, theme.bg, theme.line, theme.text),
        inactive: widget(theme, theme.surface, theme.line, theme.text),
        hovered: widget(theme, theme.accent_deep, theme.accent, theme.text),
        active: widget(
            theme,
            theme.accent_bright,
            theme.accent_bright,
            theme.on_accent,
        ),
        open: widget(theme, theme.accent_deep, theme.accent, theme.text),
    };
    visuals.selection = Selection {
        bg_fill: theme.accent_bright,
        stroke: Stroke::new(theme.stroke, theme.on_accent),
    };
    visuals.hyperlink_color = theme.secondary;
    visuals.faint_bg_color = theme.accent_deep;
    visuals.extreme_bg_color = theme.well;
    visuals.text_edit_bg_color = None;
    visuals.code_bg_color = theme.surface;
    visuals.warn_fg_color = theme.warning;
    visuals.error_fg_color = theme.error;
    visuals.window_corner_radius = CornerRadius::same(theme.radius);
    visuals.window_shadow = Shadow::NONE;
    visuals.window_fill = theme.bg;
    visuals.window_stroke = Stroke::new(theme.stroke, theme.line);
    visuals.window_highlight_topmost = false;
    visuals.menu_corner_radius = CornerRadius::same(theme.radius);
    visuals.panel_fill = theme.bg;
    visuals.popup_shadow = Shadow::NONE;
    visuals.text_cursor.stroke = Stroke::new(2.0_f32, theme.hot);
    visuals.text_cursor.blink = false;
    visuals.collapsing_header_frame = false;
    visuals.indent_has_left_vline = true;
    visuals.striped = true;
    visuals.slider_trailing_fill = true;
    visuals
}

pub fn style(theme: &Theme) -> Style {
    let mut style = Style {
        visuals: visuals(theme),
        ..Style::default()
    };
    style.text_styles = [
        (TextStyle::Small, theme.font(theme.size_small)),
        (TextStyle::Body, theme.font(theme.size_body)),
        (TextStyle::Button, theme.font(theme.size_body)),
        (TextStyle::Monospace, theme.font(theme.size_code)),
        (TextStyle::Heading, theme.heading_font(theme.size_heading)),
        (code(), theme.font(theme.size_code)),
        (title(), theme.heading_font(theme.size_title)),
    ]
    .into();
    style.spacing.item_spacing = Vec2::splat(theme.item_spacing);
    style.spacing.window_margin = Margin::same(theme.panel_padding);
    style.spacing.menu_margin = Margin::same(theme.item_spacing as i8);
    style.spacing.button_padding = Vec2::new(8.0, 3.0);
    style.spacing.interact_size = Vec2::new(40.0, theme.row_height);
    style.spacing.indent = theme.section_gap;
    style.spacing.combo_height = 240.0;
    style.interaction.selectable_labels = false;
    style.animation_time = 0.0;
    style
}

pub fn apply_theme(ctx: &Context, theme: &Theme) {
    ctx.set_fonts(fonts(theme));
    ctx.options_mut(|options| {
        options.theme_preference = ThemePreference::Dark;
        options.fallback_theme = egui::Theme::Dark;
        options.zoom_with_keyboard = false;
    });
    ctx.set_style_of(egui::Theme::Dark, style(theme));
    ctx.set_style_of(egui::Theme::Light, style(theme));
    ctx.data_mut(|data| data.insert_temp(key(), Arc::new(theme.clone())));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{self, BOLD, REGULAR};

    const REGULAR_NAME: &str = "Inconsolata-Regular";
    const BOLD_NAME: &str = "Inconsolata-Bold";

    #[test]
    fn the_theme_applies_without_panicking() {
        let theme = fixture::theme();
        let ctx = Context::default();
        apply_theme(&ctx, &theme);
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("OUTLINER");
                ui.label("mv_knob");
                ui.monospace("0.012");
            });
        });
        assert!(!output.shapes.is_empty());
        let style = ctx.style();
        assert_eq!(style.visuals.panel_fill, theme.bg);
        assert_eq!(style.visuals.extreme_bg_color, theme.well);
        assert_eq!(style.visuals.widgets.active.bg_fill, theme.accent_bright);
        assert_eq!(style.visuals.window_shadow, Shadow::NONE);
        assert_eq!(style.text_styles[&TextStyle::Body].size, theme.size_body);
        assert_eq!(style.text_styles[&title()].family, bold());
    }

    #[test]
    fn one_family_carries_both_generic_families() {
        let fonts = fonts(&fixture::theme());
        assert_eq!(
            fonts.families[&FontFamily::Proportional],
            fonts.families[&FontFamily::Monospace]
        );
        assert_eq!(fonts.font_data.len(), 2);
        assert!(!fonts.families.contains_key(&display()));
    }

    #[test]
    fn apply_installs_only_the_themes_fonts_and_its_visuals_on_a_fresh_context() {
        let theme = fixture::theme();
        let ctx = Context::default();
        apply_theme(&ctx, &theme);
        let mut seen = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            ctx.fonts(|fonts| seen = Some(fonts.definitions().clone()));
        });
        let installed = seen.unwrap();
        assert_eq!(installed.font_data.len(), 2);
        assert_eq!(installed.font_data[REGULAR_NAME].font, REGULAR);
        assert_eq!(installed.font_data[BOLD_NAME].font, BOLD);
        assert_eq!(installed.families[&bold()][0], BOLD_NAME);
        assert_eq!(installed.families[&FontFamily::Monospace], [REGULAR_NAME]);
        for mode in [egui::Theme::Dark, egui::Theme::Light] {
            let style = ctx.style_of(mode);
            assert_eq!(style.visuals.panel_fill, theme.bg);
            assert_eq!(style.visuals.widgets.inactive.bg_fill, theme.surface);
            assert_eq!(style.visuals.selection.bg_fill, theme.accent_bright);
        }
        assert_eq!(
            ctx.options(|options| options.theme_preference),
            ThemePreference::Dark
        );
    }

    #[test]
    #[should_panic(expected = "no theme applied: call theme::apply_theme first")]
    fn reading_a_theme_before_one_is_applied_panics() {
        Theme::of(&Context::default());
    }

    #[test]
    fn a_context_reads_back_the_last_theme_applied() {
        let ctx = Context::default();
        let first = fixture::theme();
        apply_theme(&ctx, &first);
        assert_eq!(*Theme::of(&ctx), first);
        let theme = Theme {
            name: Cow::Borrowed("test"),
            hot: Color32::from_rgb(1, 2, 3),
            size_body: 15.0,
            ..fixture::theme()
        };
        apply_theme(&ctx, &theme);
        assert_eq!(*Theme::of(&ctx), theme);
        assert_eq!(ctx.style().text_styles[&TextStyle::Body].size, 15.0);
        assert_eq!(
            ctx.style().visuals.text_cursor.stroke.color,
            Color32::from_rgb(1, 2, 3)
        );
        apply_theme(&ctx, &first);
        assert_eq!(*Theme::of(&ctx), first);
    }

    #[test]
    fn a_display_face_carries_titles_and_headings() {
        let theme = Theme {
            display: Some(Display {
                name: Cow::Borrowed("Display"),
                bytes: Cow::Borrowed(BOLD),
                licence: Cow::Borrowed(fixture::LICENCE),
            }),
            ..fixture::theme()
        };
        let fonts = fonts(&theme);
        assert_eq!(fonts.font_data.len(), 3);
        assert_eq!(fonts.families[&display()][0], "Display");
        let style = style(&theme);
        assert_eq!(style.text_styles[&title()].family, display());
        assert_eq!(style.text_styles[&TextStyle::Heading].family, display());
        assert_eq!(theme.title_font(12.0).family, display());
        assert_eq!(
            fixture::theme().title_font(12.0).family,
            FontFamily::Monospace
        );
        let ctx = Context::default();
        apply_theme(&ctx, &theme);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label(egui::RichText::new("TITLE").text_style(title()));
            });
        });
    }

    #[test]
    fn caps_upper_and_track_by_the_font_size() {
        let theme = fixture::theme();
        let caps = Caps {
            upper: true,
            tracking: 0.3,
        };
        let job = caps.job("outliner", theme.font(10.0), Color32::WHITE);
        assert_eq!(job.text, "OUTLINER");
        assert_eq!(job.sections[0].format.extra_letter_spacing, 3.0);
        let plain = Caps::PLAIN.job("outliner", theme.font(10.0), Color32::WHITE);
        assert_eq!(
            plain,
            LayoutJob::simple(
                "outliner".into(),
                theme.font(10.0),
                Color32::WHITE,
                f32::INFINITY
            )
        );
    }
}
