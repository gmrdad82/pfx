mod chord;
#[cfg(feature = "egui")]
pub mod egui;
mod keymap;
mod sheet;
#[cfg(feature = "terminal")]
pub mod terminal;
#[cfg(feature = "winit")]
pub mod winit;

pub use chord::{Chord, Key, Mods, Unknown};
pub use keymap::{Bind, Clash, Group, Hint, Keymap, Line, Mode, Now, Row, When, Words, bind};
pub use sheet::{Outcome, Section, Sheet, View};

pub fn chords_of<A: PartialEq>(keymap: &Keymap<A>, action: &A) -> Vec<Chord> {
    keymap
        .groups()
        .iter()
        .flat_map(Group::binds)
        .filter(|bind| bind.action() == Some(action))
        .flat_map(|bind| bind.chords().iter().copied())
        .collect()
}

pub fn written<A: PartialEq>(keymap: &Keymap<A>, action: &A) -> Option<String> {
    chords_of(keymap, action).first().map(ToString::to_string)
}

pub fn group_names<A>(keymap: &Keymap<A>) -> Vec<&str> {
    keymap.groups().iter().map(Group::name).collect()
}

pub fn shown_groups<'k>(view: &View<'k>) -> Vec<&'k str> {
    let mut names: Vec<&str> = Vec::new();
    for section in view.here.iter().chain(&view.elsewhere) {
        if !names.contains(&section.group) {
            names.push(section.group);
        }
    }
    names
}

pub fn clash_lines<A>(keymap: &Keymap<A>) -> Vec<String> {
    keymap.clashes().iter().map(ToString::to_string).collect()
}

pub fn rows<'k>(view: &View<'k>) -> Vec<Row<'k>> {
    view.here
        .iter()
        .chain(&view.elsewhere)
        .flat_map(|section| section.rows.iter().copied())
        .collect()
}

pub fn hints_off_the_sheet<A>(keymap: &Keymap<A>, now: Now) -> Vec<String> {
    let view = Sheet::new().view(keymap, now);
    let rows = rows(&view);
    keymap
        .hints(now)
        .iter()
        .filter(|hint| {
            !rows
                .iter()
                .any(|row| row.keys == hint.keys && row.does == hint.does)
                && !(hint.keys == "?" && rows.iter().any(|row| row.keys == "?"))
        })
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod model_tests;
#[cfg(test)]
mod tests;
