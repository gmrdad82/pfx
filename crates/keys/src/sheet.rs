use crate::chord::{Chord, Key, Mods};
use crate::keymap::{Group, Keymap, Mode, Now, Row};

const QUERY_CHARS: usize = 64;
const QUERY_TERMS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Took,
    Pass,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sheet {
    open: bool,
    typing: bool,
    query: String,
    terms: Vec<String>,
}

impl Sheet {
    pub fn new() -> Sheet {
        Sheet::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn typing(&self) -> bool {
        self.typing
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn close(&mut self) {
        *self = Sheet::default();
    }

    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open();
        }
    }

    pub fn search(&mut self, query: impl Into<String>) {
        self.open = true;
        self.query = query.into();
        if let Some((cut, _)) = self.query.char_indices().nth(QUERY_CHARS) {
            self.query.truncate(cut);
        }
        self.find();
    }

    fn find(&mut self) {
        self.terms.clear();
        self.terms.extend(
            self.query
                .split_whitespace()
                .take(QUERY_TERMS)
                .map(str::to_lowercase),
        );
    }

    fn clear(&mut self) {
        self.query.clear();
        self.terms.clear();
    }

    fn push(&mut self, c: char) {
        if self.query.chars().count() < QUERY_CHARS {
            self.query.push(c);
            self.find();
        }
    }

    pub fn key(&mut self, chord: Chord) -> Outcome {
        if !self.open {
            if chord == Chord::char('?') {
                self.open();
                return Outcome::Took;
            }
            return Outcome::Pass;
        }
        if self.typing {
            return self.type_key(chord);
        }
        match (chord.key, chord.mods.is_empty()) {
            (Key::Char('?'), true) => self.close(),
            (Key::Esc, true) if !self.query.is_empty() => self.clear(),
            (Key::Esc, true) => self.close(),
            (Key::Char('/'), true) => self.typing = true,
            _ => return Outcome::Pass,
        }
        Outcome::Took
    }

    fn type_key(&mut self, chord: Chord) -> Outcome {
        let plain = chord.mods.without(Mods::SHIFT).is_empty();
        match chord.key {
            Key::Esc => {
                self.typing = false;
                self.clear();
            }
            Key::Enter => self.typing = false,
            Key::Backspace => {
                if self.query.pop().is_none() {
                    self.typing = false;
                }
                self.find();
            }
            Key::Char(c) if plain => self.push(c),
            Key::Space if plain => self.push(' '),
            _ => return Outcome::Pass,
        }
        Outcome::Took
    }

    pub fn view<'k, A>(&self, keymap: &'k Keymap<A>, now: Now) -> View<'k> {
        let mut view = View {
            mode: now.mode,
            here: Vec::new(),
            elsewhere: Vec::new(),
        };
        for group in keymap.groups.iter().chain(std::iter::once(&keymap.help)) {
            let (here, elsewhere) = split(group, &now, &self.terms);
            if !here.is_empty() {
                view.here.push(Section {
                    group: group.name(),
                    rows: here,
                });
            }
            if !elsewhere.is_empty() {
                view.elsewhere.push(Section {
                    group: group.name(),
                    rows: elsewhere,
                });
            }
        }
        view
    }
}

fn split<'k, A>(group: &'k Group<A>, now: &Now, terms: &[String]) -> (Vec<Row<'k>>, Vec<Row<'k>>) {
    let mut here = Vec::new();
    let mut elsewhere = Vec::new();
    for bind in group.binds() {
        if !bind.found(terms) {
            continue;
        }
        let row = bind.row(group.name(), bind.live_open(now));
        if bind.applies(now.mode) {
            here.push(row);
        } else {
            elsewhere.push(row);
        }
    }
    (here, elsewhere)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section<'k> {
    pub group: &'k str,
    pub rows: Vec<Row<'k>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View<'k> {
    pub mode: Mode,
    pub here: Vec<Section<'k>>,
    pub elsewhere: Vec<Section<'k>>,
}

impl View<'_> {
    pub fn is_empty(&self) -> bool {
        self.here.is_empty() && self.elsewhere.is_empty()
    }
}
