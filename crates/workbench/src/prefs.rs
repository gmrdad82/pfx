use egui::epaint::RectShape;
use egui::{
    Align, Align2, Button, Color32, Context, CornerRadius, Frame, Id, Key, Layout, Margin, Pos2,
    Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, TextEdit, Ui, Vec2,
};
use pfx_settings::{Effect, Key as Setting, Preferences, Section, Store};
use pfx_splash::{Licence, Refusal, Tier};

use crate::shell::{code, small};
use crate::theme::*;
use crate::widgets::KeyCap;

pub const SALE_OPEN: bool = false;
pub const BUY: &str = "https://pitomd.com/pfx/license";
const WIDTH: f32 = 760.0;
const HEIGHT: f32 = 540.0;
const NAV: f32 = 168.0;
const LABEL: f32 = 190.0;
const UNLOCK_WIDTH: f32 = 560.0;
const FADE: f64 = 0.18;
const RISE: f32 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Display,
    Quality,
    Licence,
}

impl Page {
    const ALL: [Page; 3] = [Page::Display, Page::Quality, Page::Licence];

    fn label(self) -> &'static str {
        match self {
            Page::Display => "Display",
            Page::Quality => "Quality",
            Page::Licence => "Licence",
        }
    }

    fn about(self) -> &'static str {
        match self {
            Page::Display => "how the window presents its frames",
            Page::Quality => "how the viewport renders, and which costly effects it runs",
            Page::Licence => "your pfx licence, checked here and offline",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Preferences(Preferences),
    Licence(Option<String>),
}

#[derive(Clone, Debug)]
struct Unlock {
    text: String,
    seen: String,
    outcome: Option<Result<Licence, Refusal>>,
    since: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct Prefs {
    shown: bool,
    page: Page,
    since: Option<f64>,
    unlock: Option<Unlock>,
    public_key: Option<String>,
    build_date: Option<String>,
    licence: Option<Licence>,
    shed: Vec<Effect>,
    scale: String,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs::new(
            pfx_splash::PUBLIC_KEY.map(str::to_string),
            pfx_splash::built(),
        )
    }
}

pub fn status(public_key: Option<&str>, key: Option<&str>, built: Option<&str>) -> Option<Licence> {
    pfx_splash::unlock(public_key?, key?, built).ok()
}

impl Prefs {
    pub fn new(public_key: Option<String>, build_date: Option<String>) -> Prefs {
        Prefs {
            shown: false,
            page: Page::Display,
            since: None,
            unlock: None,
            public_key,
            build_date,
            licence: None,
            shed: Vec::new(),
            scale: String::new(),
        }
    }

    pub fn read(store: &Store) -> Prefs {
        let mut prefs = Prefs::default();
        prefs.read_licence(store);
        prefs
    }

    pub fn read_licence(&mut self, store: &Store) {
        let key = store.licence_key();
        self.licence = status(
            self.public_key.as_deref(),
            key.as_deref(),
            self.build_date.as_deref(),
        );
    }

    pub fn licence(&self) -> Option<&Licence> {
        self.licence.as_ref()
    }

    pub fn set_scale(&mut self, scale: String) {
        self.scale = scale;
    }

    pub fn set_shed(&mut self, shed: &[Effect]) {
        self.shed = shed.to_vec();
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    pub fn open(&mut self, page: Page) {
        if !self.shown {
            self.since = None;
        }
        self.shown = true;
        self.page = page;
    }

    pub fn close(&mut self) {
        self.shown = false;
        self.unlock = None;
    }

    pub fn open_unlock(&mut self) {
        self.open(Page::Licence);
        self.unlock = Some(Unlock {
            text: String::new(),
            seen: String::new(),
            outcome: None,
            since: None,
        });
    }

    pub fn paste(&mut self, key: &str) {
        if let Some(unlock) = self.unlock.as_mut() {
            unlock.text = key.to_string();
        }
    }

    pub fn unlocking(&self) -> bool {
        self.unlock.is_some()
    }

    pub fn draw(&mut self, ctx: &Context, preferences: &Preferences) -> Option<Change> {
        if !self.shown {
            return None;
        }
        let now = ctx.input(|input| input.time);
        let opened = *self.since.get_or_insert(now);
        let shown = eased(now - opened);
        if shown < 1.0 {
            ctx.request_repaint();
        }
        scrim(
            ctx,
            "settings scrim",
            shown,
            egui::Order::Middle,
            egui::Order::Middle,
        );
        let mut change = None;
        let mut close = false;
        egui::Area::new(Id::new("settings panel"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_CENTER, Vec2::new(0.0, RISE * (1.0 - shown)))
            .fade_in(false)
            .show(ctx, |ui| {
                ui.multiply_opacity(shown);
                card(ui).show(ui, |ui| {
                    ui.set_min_size(Vec2::new(WIDTH, HEIGHT));
                    ui.set_max_width(WIDTH);
                    ui.spacing_mut().item_spacing = Vec2::new(ITEM_SPACING, ITEM_SPACING);
                    self.header(ui);
                    rule(ui);
                    ui.horizontal_top(|ui| {
                        ui.set_min_height(HEIGHT - 96.0);
                        self.nav(ui);
                        ui.vertical(|ui| {
                            ui.set_width(WIDTH - NAV - SECTION_GAP);
                            change = match self.page {
                                Page::Display => display(ui, preferences),
                                Page::Quality => quality(ui, preferences, &self.shed, &self.scale),
                                Page::Licence => {
                                    self.licence_page(ui);
                                    None
                                }
                            };
                        });
                    });
                    rule(ui);
                    let (reset, closed) = footer(ui, self.page);
                    if reset {
                        change = Some(Change::Preferences(reset_page(self.page, preferences)));
                    }
                    close |= closed;
                });
            });
        if let Some(unlock) = self.unlock.as_mut() {
            match unlock_dialog(
                ctx,
                unlock,
                self.public_key.as_deref(),
                self.build_date.as_deref(),
                now,
            ) {
                Some(Some(key)) => {
                    self.licence = status(
                        self.public_key.as_deref(),
                        Some(&key),
                        self.build_date.as_deref(),
                    );
                    self.unlock = None;
                    change = Some(Change::Licence(Some(key)));
                }
                Some(None) => self.unlock = None,
                None => {}
            }
        } else if pressed(ctx, Key::Escape) {
            close = true;
        }
        if close {
            self.close();
        }
        change
    }

    fn header(&self, ui: &mut Ui) {
        let theme = Theme::of(ui.ctx());
        let palette = Palette::of(ui.ctx());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(theme.titles.text("Settings"))
                    .font(theme.title_font(SIZE_HEADING))
                    .color(palette.text_primary),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let shown = badge(ui, self.licence.as_ref(), true);
                shown.on_hover_text(match &self.licence {
                    Some(licence) => format!(
                        "{} licence for {}, covering builds through {}",
                        licence.tier.label(),
                        licence.licensee,
                        licence.covered_through
                    ),
                    None => "no key: pfx runs under the AGPL-3.0".to_string(),
                });
            });
        });
    }

    fn nav(&mut self, ui: &mut Ui) {
        let palette = Palette::of(ui.ctx());
        ui.vertical(|ui| {
            ui.set_width(NAV);
            ui.add_space(4.0);
            for page in Page::ALL {
                let chosen = page == self.page;
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(NAV, ROW_HEIGHT + 8.0), Sense::click());
                let fill = if chosen {
                    palette.bg_selected
                } else if response.hovered() {
                    palette.bg_hover
                } else {
                    Color32::TRANSPARENT
                };
                let painter = ui.painter();
                painter.rect_filled(rect, CornerRadius::same(RADIUS), fill);
                if chosen {
                    painter.rect_filled(
                        Rect::from_min_size(rect.left_top(), Vec2::new(2.0, rect.height())),
                        CornerRadius::ZERO,
                        palette.accent,
                    );
                }
                painter.text(
                    rect.left_center() + Vec2::new(12.0, 0.0),
                    Align2::LEFT_CENTER,
                    page.label(),
                    Theme::of(ui.ctx()).font(SIZE_BODY),
                    if chosen {
                        palette.accent
                    } else {
                        palette.text_secondary
                    },
                );
                mark(ui, page.label(), rect);
                if response.clicked() {
                    self.page = page;
                }
            }
        });
        let rect = ui.min_rect();
        ui.painter().vline(
            rect.left() + NAV + ITEM_SPACING,
            rect.y_range(),
            Stroke::new(STROKE, Palette::of(ui.ctx()).line_subtle),
        );
        ui.add_space(SECTION_GAP);
    }

    fn licence_page(&mut self, ui: &mut Ui) {
        let palette = Palette::of(ui.ctx());
        page_title(ui, Page::Licence);
        ui.add_space(4.0);
        badge(ui, self.licence.as_ref(), false);
        ui.add_space(4.0);
        match &self.licence {
            Some(licence) => {
                for (label, shown) in [
                    ("Tier", licence.tier.label().to_string()),
                    ("Licensed to", licence.licensee.clone()),
                    ("Paid through", licence.paid_through.clone()),
                    (
                        "Covered through",
                        format!(
                            "{} ({} days' grace)",
                            licence.covered_through, licence.grace_days
                        ),
                    ),
                    ("Key", licence.id.clone()),
                ] {
                    fact(ui, label, &shown);
                }
                ui.add_space(4.0);
                ui.label(small(
                    ui,
                    "a game built while the key is paid keeps it: a Pro key hides the splash in builds made through the covered date",
                    palette.text_muted,
                ));
            }
            None => {
                fact(ui, "Tier", "Free");
                ui.label(small(
                    ui,
                    "pfx is free under the AGPL-3.0 for open-source projects, learning and hobby work; a closed product needs a commercial licence",
                    palette.text_muted,
                ));
            }
        }
        if let Some(built) = &self.build_date {
            fact(ui, "This pfx built", built);
        }
        ui.add_space(SECTION_GAP);
        ui.horizontal(|ui| {
            let unlock = if self.licence.is_some() {
                "Enter another key…"
            } else {
                "Unlock…"
            };
            let unlocked = ui.add(action(ui, unlock, true));
            mark(ui, "unlock", unlocked.rect);
            if unlocked.clicked() {
                self.open_unlock();
            }
            if SALE_OPEN {
                ui.hyperlink_to(small(ui, "Buy a licence", palette.info), BUY);
            }
        });
    }
}

pub fn eased(seconds: f64) -> f32 {
    pfx_core::motion::cubic_out((seconds / FADE) as f32)
}

fn scrim(ctx: &Context, id: &str, shown: f32, shade: egui::Order, block: egui::Order) {
    let screen = ctx.content_rect();
    ctx.layer_painter(egui::LayerId::new(shade, Id::new((id, "shade"))))
        .rect_filled(
            screen,
            CornerRadius::ZERO,
            Color32::from_black_alpha((150.0 * shown) as u8),
        );
    egui::Area::new(Id::new((id, "block")))
        .order(block)
        .fixed_pos(screen.min)
        .fade_in(false)
        .show(ctx, |ui| {
            ui.allocate_exact_size(screen.size(), Sense::click());
        });
}

fn card(ui: &Ui) -> Frame {
    let palette = Palette::of(ui.ctx());
    Frame::NONE
        .fill(palette.bg_base)
        .stroke(Stroke::new(STROKE, palette.line_border))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::same(PANEL_PADDING * 2))
}

fn rule(ui: &mut Ui) {
    let palette = Palette::of(ui.ctx());
    ui.add_space(2.0);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(STROKE, palette.line_subtle),
    );
    ui.add_space(2.0);
}

fn page_title(ui: &mut Ui, page: Page) {
    let theme = Theme::of(ui.ctx());
    let palette = Palette::of(ui.ctx());
    ui.add_space(4.0);
    ui.label(
        RichText::new(theme.labels.text(page.label()))
            .font(theme.label_font(SIZE_BODY))
            .color(palette.text_primary),
    );
    ui.label(small(ui, page.about(), palette.text_muted));
    ui.add_space(4.0);
}

fn group(ui: &mut Ui, title: &str, about: &str) {
    let theme = Theme::of(ui.ctx());
    let palette = Palette::of(ui.ctx());
    ui.add_space(SECTION_GAP * 0.5);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(theme.labels.text(title))
                .font(theme.label_font(SIZE_SMALL))
                .color(palette.text_secondary),
        );
        ui.label(small(ui, about, palette.text_dim));
    });
}

fn fact(ui: &mut Ui, label: &str, shown: &str) {
    let palette = Palette::of(ui.ctx());
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(LABEL * 0.7, ROW_HEIGHT),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_min_width(LABEL * 0.7);
                ui.label(small(ui, label, palette.text_secondary));
            },
        );
        ui.label(code(ui, shown, palette.text_primary));
    });
}

fn action(ui: &Ui, text: &str, enabled: bool) -> Button<'static> {
    let palette = Palette::of(ui.ctx());
    let ink = if enabled {
        palette.text_primary
    } else {
        palette.text_dim
    };
    Button::new(small(ui, text, ink))
        .fill(palette.bg_chip)
        .stroke(Stroke::new(STROKE, palette.line_border))
        .corner_radius(CornerRadius::same(RADIUS))
        .min_size(Vec2::new(0.0, ROW_HEIGHT + 4.0))
}

fn primary(ui: &Ui, text: &str, enabled: bool) -> Button<'static> {
    let palette = Palette::of(ui.ctx());
    let (fill, ink) = if enabled {
        (palette.accent, palette.accent_text)
    } else {
        (palette.bg_chip, palette.text_dim)
    };
    Button::new(small(ui, text, ink))
        .fill(fill)
        .stroke(Stroke::new(STROKE, fill))
        .corner_radius(CornerRadius::same(RADIUS))
        .min_size(Vec2::new(0.0, ROW_HEIGHT + 4.0))
}

fn segmented(ui: &mut Ui, setting: Setting, chosen: &str) -> Option<&'static str> {
    let palette = Palette::of(ui.ctx());
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for choice in setting.choices() {
            let on = choice.name == chosen;
            let ink = if on {
                palette.accent
            } else {
                palette.text_secondary
            };
            let response = ui
                .add(
                    Button::new(code(ui, choice.label, ink))
                        .wrap_mode(egui::TextWrapMode::Extend)
                        .fill(if on {
                            palette.bg_selected
                        } else {
                            palette.bg_chip
                        })
                        .stroke(Stroke::new(
                            STROKE,
                            if on {
                                palette.accent
                            } else {
                                palette.line_border
                            },
                        ))
                        .corner_radius(CornerRadius::same(RADIUS))
                        .min_size(Vec2::new(54.0, ROW_HEIGHT + 2.0)),
                )
                .on_hover_text(if choice.about.is_empty() {
                    setting.about()
                } else {
                    choice.about
                });
            mark(
                ui,
                &format!("{}={}", setting.name(), choice.name),
                response.rect,
            );
            if response.clicked() && !on {
                picked = Some(choice.name);
            }
        }
    });
    picked
}

fn row(ui: &mut Ui, preferences: &Preferences, setting: Setting) -> Option<Preferences> {
    let palette = Palette::of(ui.ctx());
    let mut changed = None;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(LABEL, ROW_HEIGHT + 2.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_min_width(LABEL);
                ui.label(
                    RichText::new(setting.label())
                        .font(Theme::of(ui.ctx()).font(SIZE_BODY))
                        .color(palette.text_primary),
                );
            },
        );
        if let Some(name) = segmented(ui, setting, setting.chosen(preferences)) {
            let mut next = *preferences;
            if setting.set(&mut next, name).is_ok() {
                changed = Some(next);
            }
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(LABEL + ITEM_SPACING);
        ui.label(small(ui, setting.about(), palette.text_muted));
    });
    changed
}

fn display(ui: &mut Ui, preferences: &Preferences) -> Option<Change> {
    page_title(ui, Page::Display);
    let mut changed = None;
    for setting in Section::Display.keys() {
        changed = row(ui, preferences, setting).or(changed);
    }
    if !preferences.measured() {
        let palette = Palette::of(ui.ctx());
        ui.add_space(4.0);
        ui.label(small(
            ui,
            "not measured yet: the first frames pick the frame rate and the quality for this machine",
            palette.text_dim,
        ));
    }
    changed.map(Change::Preferences)
}

fn switch(ui: &mut Ui, on: bool) -> Response {
    let palette = Palette::of(ui.ctx());
    let size = Vec2::new(28.0, 14.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let at = ui.ctx().animate_bool_with_time_and_easing(
        response.id,
        on,
        0.14,
        egui::emath::easing::cubic_out,
    );
    let track = lerp_colour(palette.bg_chip, palette.bg_selected, at);
    let edge = lerp_colour(palette.line_border, palette.accent, at);
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(7),
        track,
        Stroke::new(STROKE, edge),
        StrokeKind::Inside,
    );
    let x = rect.left() + 7.0 + (rect.width() - 14.0) * at;
    painter.circle_filled(
        Pos2::new(x, rect.center().y),
        4.5,
        lerp_colour(palette.text_muted, palette.accent, at),
    );
    response
}

fn lerp_colour(from: Color32, to: Color32, t: f32) -> Color32 {
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
        mix(from.a(), to.a()),
    )
}

fn quality(ui: &mut Ui, preferences: &Preferences, shed: &[Effect], scale: &str) -> Option<Change> {
    let palette = Palette::of(ui.ctx());
    page_title(ui, Page::Quality);
    let mut changed = row(ui, preferences, Setting::Quality);
    if !scale.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(LABEL + ITEM_SPACING);
            ui.label(code(ui, format!("render scale {scale}"), palette.info))
                .on_hover_text("the scale chip under the viewport picks auto, 100%, 75% or 50%; 100% is Best appearance");
        });
    }
    group(
        ui,
        "Effects",
        "each pass runs where the scene asks for it; off skips it everywhere",
    );
    let half = (ui.available_width() - SECTION_GAP) / 2.0;
    egui::Grid::new("settings effects")
        .num_columns(2)
        .spacing(Vec2::new(SECTION_GAP, ITEM_SPACING + 2.0))
        .min_col_width(half)
        .show(ui, |ui| {
            for (index, effect) in Effect::ALL.into_iter().enumerate() {
                let on = preferences.graphics.effects.get(effect);
                let dropped = on && shed.contains(&effect);
                ui.horizontal(|ui| {
                    let response = switch(ui, on).on_hover_text(effect.about());
                    let ink = if dropped {
                        palette.text_dim
                    } else if on {
                        palette.text_primary
                    } else {
                        palette.text_muted
                    };
                    ui.label(code(ui, effect.label(), ink))
                        .on_hover_text(effect.about());
                    if dropped {
                        ui.label(code(ui, "held off", palette.info))
                            .on_hover_text("Best performance switched it off to hold 8.33 ms");
                    }
                    mark(ui, &Setting::Effect(effect).name(), response.rect);
                    if response.clicked() {
                        let mut next = *preferences;
                        next.graphics.effects.set(effect, !on);
                        changed = Some(next);
                    }
                });
                if index % 2 == 1 {
                    ui.end_row();
                }
            }
        });
    ui.add_space(4.0);
    let note = match preferences.graphics.quality() {
        pfx_settings::Quality::Appearance => "Best appearance keeps every effect that is on",
        pfx_settings::Quality::Performance => {
            "Best performance lowers the render scale first, then holds the costliest effects off until the frame fits 8.33 ms"
        }
    };
    ui.label(small(ui, note, palette.text_muted));
    changed.map(Change::Preferences)
}

fn reset_page(page: Page, preferences: &Preferences) -> Preferences {
    let fresh = Preferences::default();
    match page {
        Page::Display => Preferences {
            vrr: fresh.vrr,
            frame_rate: preferences.frame_rate.map(|_| fresh.rate()),
            ..*preferences
        },
        Page::Quality => Preferences {
            graphics: pfx_settings::Graphics {
                quality: preferences.graphics.quality.map(|_| Default::default()),
                effects: fresh.graphics.effects,
            },
            ..*preferences
        },
        Page::Licence => *preferences,
    }
}

fn footer(ui: &mut Ui, page: Page) -> (bool, bool) {
    let palette = Palette::of(ui.ctx());
    let mut reset = false;
    let mut close = false;
    ui.horizontal(|ui| {
        ui.label(small(
            ui,
            "changes apply at once and are kept in settings.pitofx",
            palette.text_dim,
        ));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            close = ui.add(primary(ui, "Close", true)).clicked();
            if page != Page::Licence {
                reset = ui.add(action(ui, "Reset this page", true)).clicked();
            }
            ui.add(KeyCap::new("esc"));
        });
    });
    (reset, close)
}

pub fn badge(ui: &mut Ui, licence: Option<&Licence>, compact: bool) -> Response {
    let palette = Palette::of(ui.ctx());
    let theme = Theme::of(ui.ctx());
    let tier = licence.map_or(Tier::Free, |licence| licence.tier);
    let (fill, ink, edge) = match tier {
        Tier::Free => (palette.bg_chip, palette.text_secondary, palette.line_border),
        Tier::Indie => (palette.bg_hover, palette.info, palette.info),
        Tier::Pro => (palette.accent, palette.accent_text, palette.accent),
    };
    let mark = theme.labels.text(tier.label()).into_owned();
    let mark_font = theme.bold_font(SIZE_CODE);
    let detail = licence.map(|licence| {
        if compact {
            format!("{} · through {}", licence.licensee, licence.covered_through)
        } else {
            format!(
                "{}  ·  paid through {}",
                licence.licensee, licence.paid_through
            )
        }
    });
    let mark_galley = ui.painter().layout_no_wrap(mark, mark_font, ink);
    let detail_galley = detail.map(|detail| {
        ui.painter()
            .layout_no_wrap(detail, theme.font(SIZE_CODE), palette.text_secondary)
    });
    let pad = 8.0;
    let mark_width = mark_galley.size().x + 2.0 * pad;
    let detail_width = detail_galley
        .as_ref()
        .map_or(0.0, |galley| galley.size().x + 2.0 * pad);
    let height = ROW_HEIGHT + 3.0;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(mark_width + detail_width, height), Sense::click());
    let painter = ui.painter();
    let back = painter.add(Shape::Noop);
    painter.set(
        back,
        RectShape::new(
            rect,
            CornerRadius::same(RADIUS),
            palette.bg_deep,
            Stroke::new(STROKE, edge),
            StrokeKind::Inside,
        ),
    );
    let mark_rect = Rect::from_min_size(rect.min, Vec2::new(mark_width, height));
    painter.rect_filled(mark_rect, CornerRadius::same(RADIUS), fill);
    painter.galley(
        mark_rect.center() - mark_galley.size() / 2.0,
        mark_galley,
        ink,
    );
    if let Some(galley) = detail_galley {
        painter.galley(
            Pos2::new(
                mark_rect.right() + pad,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            palette.text_secondary,
        );
    }
    response
}

fn unlock_dialog(
    ctx: &Context,
    unlock: &mut Unlock,
    public_key: Option<&str>,
    build_date: Option<&str>,
    now: f64,
) -> Option<Option<String>> {
    if unlock.since.is_none() {
        ctx.memory_mut(|memory| memory.request_focus(Id::new("unlock key")));
    }
    let opened = *unlock.since.get_or_insert(now);
    let shown = eased(now - opened);
    if shown < 1.0 {
        ctx.request_repaint();
    }
    if unlock.text != unlock.seen {
        unlock.seen.clone_from(&unlock.text);
        unlock.outcome = (!unlock.text.trim().is_empty()).then(|| match public_key {
            Some(public_key) => pfx_splash::unlock(public_key, &unlock.text, build_date),
            None => Err(Refusal::Key(pfx_splash::KeyError::PublicKey)),
        });
    }
    scrim(
        ctx,
        "unlock scrim",
        shown * 0.6,
        egui::Order::Tooltip,
        egui::Order::Foreground,
    );
    let mut done = None;
    egui::Area::new(Id::new("unlock dialog"))
        .order(egui::Order::Tooltip)
        .anchor(Align2::CENTER_CENTER, Vec2::new(0.0, RISE * (1.0 - shown)))
        .fade_in(false)
        .show(ctx, |ui| {
            ui.multiply_opacity(shown);
            let palette = Palette::of(ui.ctx());
            let theme = Theme::of(ui.ctx());
            card(ui)
                .stroke(Stroke::new(STROKE, palette.line_strong))
                .show(ui, |ui| {
                    ui.set_width(UNLOCK_WIDTH);
                    ui.spacing_mut().item_spacing = Vec2::new(ITEM_SPACING, ITEM_SPACING);
                    ui.label(
                        RichText::new(theme.titles.text("Unlock pfx"))
                            .font(theme.title_font(SIZE_HEADING))
                            .color(palette.text_primary),
                    );
                    ui.label(small(
                        ui,
                        "paste your pfx key; it is checked here, offline, against pfx's public key",
                        palette.text_muted,
                    ));
                    ui.add_space(4.0);
                    let back = ui.painter().add(Shape::Noop);
                    let field = ui.add(
                        TextEdit::multiline(&mut unlock.text)
                            .id(Id::new("unlock key"))
                            .frame(Frame::NONE)
                            .margin(Margin::same(8))
                            .desired_rows(5)
                            .desired_width(UNLOCK_WIDTH - 16.0)
                            .font(theme.font(SIZE_CODE))
                            .text_color(palette.text_code)
                            .hint_text(code(
                                ui,
                                "----- BEGIN PITO MD LICENSE -----\nPITO1.…\n------ END PITO MD LICENSE ------",
                                palette.text_dim,
                            )),
                    );
                    ui.painter().set(
                        back,
                        RectShape::new(
                            field.rect,
                            CornerRadius::same(RADIUS),
                            palette.bg_deep,
                            Stroke::new(
                                STROKE,
                                if field.has_focus() {
                                    palette.accent
                                } else {
                                    palette.line_border
                                },
                            ),
                            StrokeKind::Inside,
                        ),
                    );
                    ui.add_space(4.0);
                    match &unlock.outcome {
                        None => {
                            ui.label(small(ui, "waiting for a key", palette.text_dim));
                        }
                        Some(Ok(licence)) => {
                            ui.horizontal(|ui| {
                                ui.label(code(ui, "verified", palette.ok));
                                badge(ui, Some(licence), false);
                            });
                            ui.label(small(
                                ui,
                                format!(
                                    "{} · {} · covers builds through {}",
                                    licence.tier.label(),
                                    licence.licensee,
                                    licence.covered_through
                                ),
                                palette.text_secondary,
                            ));
                        }
                        Some(Err(refusal)) => {
                            ui.label(code(ui, "refused", palette.danger));
                            ui.label(small(ui, refusal.to_string(), palette.danger));
                        }
                    }
                    ui.add_space(SECTION_GAP * 0.5);
                    let verified = matches!(unlock.outcome, Some(Ok(_)));
                    ui.horizontal(|ui| {
                        if SALE_OPEN {
                            ui.hyperlink_to(small(ui, "Buy a licence", palette.info), BUY);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let save = ui.add_enabled(verified, primary(ui, "Save key", verified));
                            let cancel = ui.add(action(ui, "Cancel", true));
                            mark(ui, "save", save.rect);
                            mark(ui, "cancel", cancel.rect);
                            if save.clicked() {
                                done = Some(Some(pfx_splash::token(&unlock.text)));
                            }
                            if cancel.clicked() {
                                done = Some(None);
                            }
                        });
                    });
                });
        });
    if done.is_none() && pressed(ctx, Key::Escape) {
        done = Some(None);
    }
    done
}

fn mark(ui: &Ui, name: &str, rect: Rect) {
    ui.ctx()
        .data_mut(|data| data.insert_temp(Id::new(("settings target", name)), rect));
}

#[cfg(test)]
pub fn target(ctx: &Context, name: &str) -> Option<Rect> {
    ctx.data(|data| data.get_temp(Id::new(("settings target", name))))
}

fn pressed(ctx: &Context, key: Key) -> bool {
    ctx.input(|input| input.key_pressed(key))
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;
    use crate::{Script, Shell, Workbench};

    const SIZE: [u32; 2] = [1600, 1000];
    const BUILT: &str = "2026-10-20";

    pub(crate) fn pair() -> (SigningKey, String) {
        let signing = SigningKey::from_bytes(&[5u8; 32]);
        let public = STANDARD.encode(signing.verifying_key().to_bytes());
        (signing, public)
    }

    pub(crate) fn mint(signing: &SigningKey, tier: &str, through: &str) -> String {
        let payload = format!(
            r#"{{"v":2,"id":"k-1042","email":"hello@example.com","licensee":"Example Games","app":"pfx","tier":"{tier}","major":3,"issued":"2026-10-01","type":"timed","paid_through":"{through}","grace_days":14}}"#
        );
        let signed = format!("PITO1.{}", URL_SAFE_NO_PAD.encode(payload.as_bytes()));
        let signature = signing.sign(signed.as_bytes());
        format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
    }

    fn checked(shell: &mut Shell, public: &str, key: Option<&str>) {
        let prefs = shell.prefs();
        prefs.public_key = Some(public.to_string());
        prefs.build_date = Some(BUILT.to_string());
        prefs.licence = status(Some(public), key, Some(BUILT));
    }

    fn frames(count: usize) -> Script {
        Script::parse(&vec!["frame"; count].join("\n")).unwrap()
    }

    fn drafts() -> std::path::PathBuf {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/settings-drafts");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn drive(ctx: &egui::Context, shell: &mut Shell, script: &str) {
        pfx_editor_shell::drive(ctx, SIZE, &Script::parse(script).unwrap(), |ui| {
            shell.ui(ui)
        });
    }

    fn click(ctx: &egui::Context, shell: &mut Shell, name: &str) {
        let at = target(ctx, name)
            .unwrap_or_else(|| panic!("{name} is not drawn"))
            .center();
        drive(
            ctx,
            shell,
            &format!("click {} {}\nframe\nframe", at.x, at.y),
        );
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx, &crate::theme::workbench());
        ctx
    }

    #[test]
    fn every_setting_in_the_panel_applies_live_and_is_kept() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/ui-tests/settings-live");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = context();
        let mut shell = Shell::new().with_book(crate::open::Book::at(&dir));
        drive(&ctx, &mut shell, "key ctrl+Comma\nframe\nframe");
        assert!(shell.prefs().shown());
        click(&ctx, &mut shell, "vrr=off");
        assert!(!shell.present().vrr);
        click(&ctx, &mut shell, "frame_rate=60");
        assert_eq!(
            shell.present().frame_rate,
            pfx_editor_shell::FrameRate::Cap60
        );
        click(&ctx, &mut shell, "frame_rate=auto");
        assert_eq!(shell.preferences().rate(), pfx_settings::Rate::Auto);
        click(&ctx, &mut shell, "stats=on");
        assert!(shell.preferences().stats);
        assert!(shell.stats());
        click(&ctx, &mut shell, "Quality");
        click(&ctx, &mut shell, "quality=performance");
        click(&ctx, &mut shell, "quality=appearance");
        assert_eq!(shell.view_scale(), crate::scale::ViewScale::Percent(100));
        click(&ctx, &mut shell, "quality=performance");
        assert_eq!(
            shell.preferences().graphics.quality(),
            pfx_settings::Quality::Performance
        );
        assert_eq!(shell.view_scale(), crate::scale::ViewScale::Auto);
        assert_eq!(
            crate::open::Book::at(&dir).view_scale(),
            crate::scale::ViewScale::Auto
        );
        for effect in Effect::ALL {
            click(&ctx, &mut shell, &Setting::Effect(effect).name());
            assert!(
                !shell.preferences().graphics.effects.get(effect),
                "{effect:?}"
            );
        }
        let kept = crate::open::Book::at(&dir).preferences();
        assert_eq!(kept, shell.preferences());
        assert!(
            !kept.vrr
                && kept.graphics.effects
                    == pfx_settings::Effects {
                        taa: false,
                        ssr: false,
                        scene_reflections: false,
                        ambient_occlusion: false,
                        bloom: false,
                        flare: false,
                        motion_blur: false,
                        depth_of_field: false,
                        fog: false,
                    }
        );
        drive(&ctx, &mut shell, "key Escape\nframe");
        assert!(!shell.prefs().shown());
        drive(&ctx, &mut shell, "key F3\nframe");
        assert!(!shell.stats() && !shell.preferences().stats);
        assert!(!crate::open::Book::at(&dir).preferences().stats);
    }

    #[test]
    fn the_unlock_dialog_keeps_a_verified_key_and_refuses_a_forged_one() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/ui-tests/settings-unlock");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (signing, public) = pair();
        let ctx = context();
        let book = crate::open::Book::at(&dir);
        let mut shell = Shell::new().with_book(book.clone());
        checked(&mut shell, &public, None);
        shell.prefs().open(Page::Licence);
        drive(&ctx, &mut shell, "frame\nframe");
        click(&ctx, &mut shell, "unlock");
        assert!(shell.prefs().unlocking());
        let other = SigningKey::from_bytes(&[6u8; 32]);
        shell.prefs().paste(&mint(&other, "pro", "2026-12-31"));
        drive(&ctx, &mut shell, "frame\nframe");
        click(&ctx, &mut shell, "save");
        assert!(shell.prefs().unlocking());
        assert_eq!(book.store().licence_key(), None);
        let pro = mint(&signing, "pro", "2026-12-31");
        shell.prefs().paste(&format!(
            "----- BEGIN PITO MD LICENSE -----\n{pro}\n------ END PITO MD LICENSE ------\n"
        ));
        drive(&ctx, &mut shell, "frame\nframe");
        click(&ctx, &mut shell, "save");
        assert!(!shell.prefs().unlocking());
        assert_eq!(book.store().licence_key().as_deref(), Some(pro.as_str()));
        let licence = shell.prefs().licence().unwrap();
        assert_eq!(licence.tier, Tier::Pro);
        assert_eq!(licence.licensee, "Example Games");
        assert_eq!(licence.covered_through, "2027-01-14");
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn the_settings_panel_and_the_unlock_dialog_draw_offscreen() {
        let since = std::time::Instant::now();
        let (signing, public) = pair();
        let pro = mint(&signing, "pro", "2026-12-31");
        let dir = drafts();
        let mut shell = Shell::new();
        checked(&mut shell, &public, Some(&pro));
        let mut preferences = shell.preferences();
        preferences.measure(11.0);
        preferences.graphics.effects.set(Effect::Flare, false);
        preferences.graphics.effects.set(Effect::Fog, false);
        shell.set_preferences(preferences);
        shell.prefs().set_shed(&[Effect::MotionBlur]);
        for (page, name) in [
            (Page::Quality, "settings-quality"),
            (Page::Display, "settings-display"),
            (Page::Licence, "settings-licence"),
        ] {
            shell.prefs().close();
            shell.prefs().open(page);
            let png =
                crate::shot(&mut Workbench::new(&mut shell, None), SIZE, &frames(16)).unwrap();
            std::fs::write(dir.join(format!("{name}.png")), png).unwrap();
        }
        for (key, name) in [
            (pro.clone(), "unlock-verified"),
            (format!("{}x", pro), "unlock-refused"),
            (mint(&signing, "pro", "2026-09-01"), "unlock-lapsed"),
        ] {
            shell.prefs().close();
            shell.prefs().open_unlock();
            shell.prefs().paste(&key);
            let png =
                crate::shot(&mut Workbench::new(&mut shell, None), SIZE, &frames(16)).unwrap();
            std::fs::write(dir.join(format!("{name}.png")), png).unwrap();
        }
        crate::gpu_turn(since);
    }
}
