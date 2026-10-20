use pfx_input::{ActionSpec, Binding, Button, Input, Key as Pressed};
use pfx_live::flat::{Draw, Srgba};
use pfx_settings::{Key, Preferences, Rate, Section};

use crate::settings::Settings;
use crate::ui::{Anchor, Label, Ui, UiFrame};

pub const OPEN: &str = "settings.open";
pub const UP: &str = "settings.up";
pub const DOWN: &str = "settings.down";
pub const LEFT: &str = "settings.left";
pub const RIGHT: &str = "settings.right";
pub const CONTEXT: &str = "settings";
const FADE: f32 = 0.18;
const RISE: f32 = 12.0;

pub fn actions() -> Vec<ActionSpec> {
    let step = |name: &str, key: Pressed, button: Button| {
        ActionSpec::digital(name)
            .context(CONTEXT)
            .repeat(18, 6)
            .key(Binding::key(key))
            .pad(Binding::button(button))
    };
    vec![
        ActionSpec::digital(OPEN)
            .key(Binding::key(Pressed::Escape))
            .pad(Binding::button(Button::Menu)),
        step(UP, Pressed::Up, Button::DPadUp),
        step(DOWN, Pressed::Down, Button::DPadDown),
        step(LEFT, Pressed::Left, Button::DPadLeft),
        step(RIGHT, Pressed::Right, Button::DPadRight),
    ]
}

pub fn rows() -> Vec<Key> {
    Section::ALL
        .into_iter()
        .flat_map(Section::keys)
        .filter(|key| *key != Key::Stats)
        .collect()
}

pub fn preferences(settings: &Settings) -> Preferences {
    Preferences {
        vrr: settings.pacing.vrr,
        frame_rate: Some(Rate::of(settings.pacing.frame_rate)),
        stats: false,
        graphics: settings.graphics,
    }
}

pub fn apply(settings: &mut Settings, preferences: &Preferences) {
    settings.pacing.vrr = preferences.vrr;
    settings.pacing.frame_rate = preferences.rate().frame_rate();
    settings.graphics = preferences.graphics;
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuLook {
    pub family: String,
    pub size: f32,
    pub ink: Srgba,
    pub dim: Srgba,
    pub accent: Srgba,
    pub panel: Srgba,
    pub scrim: Srgba,
}

impl MenuLook {
    pub fn new(family: impl Into<String>) -> MenuLook {
        MenuLook {
            family: family.into(),
            size: 26.0,
            ink: Srgba::hex(0xc8c8c8),
            dim: Srgba::hex(0x6c6c6c),
            accent: Srgba::hex(0xccf040),
            panel: Srgba::hex(0x0d0e0e),
            scrim: Srgba([0.0, 0.0, 0.0, 0.6]),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsMenu {
    open: bool,
    focus: usize,
    opened: Option<f32>,
}

impl SettingsMenu {
    pub fn new() -> SettingsMenu {
        SettingsMenu::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn focus(&self) -> Key {
        rows()[self.focus]
    }

    pub fn open(&mut self) {
        if !self.open {
            self.opened = None;
        }
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn tick(&mut self, input: &Input, settings: &mut Settings) -> bool {
        if input.action(OPEN).pressed {
            self.open = !self.open;
            self.opened = None;
            return false;
        }
        if !self.open {
            return false;
        }
        let rows = rows();
        if input.action(UP).fired() {
            self.focus = (self.focus + rows.len() - 1) % rows.len();
        }
        if input.action(DOWN).fired() {
            self.focus = (self.focus + 1) % rows.len();
        }
        let step = match (input.action(LEFT).fired(), input.action(RIGHT).fired()) {
            (true, false) => -1,
            (false, true) => 1,
            _ => return false,
        };
        self.step(settings, step)
    }

    pub fn step(&mut self, settings: &mut Settings, by: i32) -> bool {
        let key = self.focus();
        let mut chosen = preferences(settings);
        let choices = key.choices();
        let at = choices
            .iter()
            .position(|choice| choice.name == key.chosen(&chosen))
            .unwrap_or(0) as i32;
        let next = (at + by).rem_euclid(choices.len() as i32) as usize;
        if key.set(&mut chosen, choices[next].name).is_err() {
            return false;
        }
        let before = settings.clone();
        apply(settings, &chosen);
        *settings != before
    }

    pub fn move_focus(&mut self, by: i32) {
        let count = rows().len() as i32;
        self.focus = (self.focus as i32 + by).rem_euclid(count) as usize;
    }

    pub fn ui(&mut self, frame: &UiFrame, settings: &Settings, look: &MenuLook) -> Ui {
        let mut ui = Ui::new();
        if !self.open {
            return ui;
        }
        let opened = *self.opened.get_or_insert(frame.seconds);
        let shown = pfx_core::motion::cubic_out((frame.seconds - opened) / FADE);
        let fade = |colour: Srgba| {
            let Srgba([r, g, b, a]) = colour;
            Srgba([r, g, b, a * shown])
        };
        let [left, top, right, bottom] = frame.safe;
        let centre = [(left + right) / 2.0, (top + bottom) / 2.0];
        let line = look.size * 1.6;
        let rows = rows();
        let width = (look.size * 30.0).min(right - left);
        let height = line * (rows.len() as f32 + 2.5);
        let lift = RISE * (1.0 - shown);
        ui.draw(Draw::rect(centre, [right - left, bottom - top], 0.0).fill(fade(look.scrim)));
        ui.draw(
            Draw::rect(
                [centre[0], centre[1] + lift],
                [width, height],
                look.size * 0.2,
            )
            .fill(fade(look.panel)),
        );
        let x = centre[0] - width / 2.0 + look.size;
        let value_x = centre[0] + width / 2.0 - look.size;
        let mut y = centre[1] - height / 2.0 + lift + line * 0.75;
        ui.label(
            Label::new("SETTINGS", look.family.clone(), look.size * 1.1, [x, y])
                .weight(300)
                .colour(fade(look.ink)),
        );
        y += line * 1.25;
        let chosen = preferences(settings);
        for (index, key) in rows.into_iter().enumerate() {
            let focused = index == self.focus;
            let ink = if focused { look.accent } else { look.ink };
            ui.label(
                Label::new(key.label(), look.family.clone(), look.size, [x, y]).colour(fade(ink)),
            );
            let value = key
                .choices()
                .into_iter()
                .find(|choice| choice.name == key.chosen(&chosen))
                .map_or("", |choice| choice.label);
            let shown_value = if focused {
                format!("< {value} >")
            } else {
                value.to_string()
            };
            ui.label(
                Label::new(shown_value, look.family.clone(), look.size, [value_x, y])
                    .anchor(Anchor::End)
                    .colour(fade(if focused { look.accent } else { look.dim })),
            );
            y += line;
        }
        ui
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_settings::{Effect, Quality};

    #[test]
    fn the_menu_steps_each_row_through_its_choices_and_changes_the_games_settings() {
        let mut menu = SettingsMenu::new();
        let mut settings = Settings::default();
        assert_eq!(menu.focus(), Key::Vrr);
        assert!(menu.step(&mut settings, 1));
        assert!(!settings.pacing.vrr);
        menu.move_focus(1);
        assert_eq!(menu.focus(), Key::FrameRate);
        assert!(menu.step(&mut settings, -1));
        assert_eq!(
            settings.pacing.frame_rate,
            pfx_gpu::window::FrameRate::Cap120
        );
        menu.move_focus(1);
        assert_eq!(menu.focus(), Key::Quality);
        menu.step(&mut settings, 1);
        assert_eq!(settings.graphics.quality, Some(Quality::Performance));
        menu.move_focus(2);
        assert_eq!(menu.focus(), Key::Effect(Effect::Ssr));
        menu.step(&mut settings, 1);
        assert!(!settings.graphics.effects.ssr);
        assert!(!rows().contains(&Key::Stats));
        menu.move_focus(-5);
        assert_eq!(menu.focus(), rows()[rows().len() - 1]);
    }

    #[test]
    fn the_menu_draws_only_while_open_and_fades_in_eased() {
        let mut menu = SettingsMenu::new();
        let settings = Settings::default();
        let look = MenuLook::new("Body");
        let mut frame = UiFrame::default();
        assert!(menu.ui(&frame, &settings, &look).is_empty());
        menu.open();
        let first = menu.ui(&frame, &settings, &look);
        assert_eq!(first.labels.len(), 1 + 2 * rows().len());
        assert_eq!(first.labels[0].colour.0[3], 0.0);
        frame.seconds = 0.09;
        let half = menu.ui(&frame, &settings, &look).labels[0].colour.0[3];
        assert!(half > 0.5 && half < 1.0, "{half}");
        frame.seconds = 1.0;
        assert_eq!(menu.ui(&frame, &settings, &look).labels[0].colour.0[3], 1.0);
        menu.close();
        assert!(menu.ui(&frame, &settings, &look).is_empty());
    }
}
