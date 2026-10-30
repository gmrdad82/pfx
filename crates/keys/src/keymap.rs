use std::borrow::Cow;
use std::fmt;

use crate::chord::Chord;

type Text = Cow<'static, str>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Mode(pub &'static str);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct When(pub &'static str);

pub(crate) const HELP_OPEN: When = When("while this help is open");

#[derive(Debug, Clone, Copy)]
pub struct Now<'a> {
    pub mode: Mode,
    pub when: &'a [When],
}

impl Now<'static> {
    pub const fn new(mode: Mode) -> Now<'static> {
        Now { mode, when: &[] }
    }
}

impl<'a> Now<'a> {
    pub const fn when<'b>(self, when: &'b [When]) -> Now<'b> {
        Now {
            mode: self.mode,
            when,
        }
    }

    fn holds(&self, when: Option<When>) -> bool {
        when.is_none_or(|when| self.when.contains(&when))
    }

    fn holds_open(&self, when: Option<When>) -> bool {
        when.is_none_or(|when| when == HELP_OPEN || self.when.contains(&when))
    }
}

#[derive(Debug, Clone)]
pub struct Bind<A> {
    chords: Vec<Chord>,
    shown: Option<Text>,
    keys: String,
    does: Text,
    note: Option<Text>,
    modes: Vec<Mode>,
    modes_text: String,
    hay: String,
    when: Option<When>,
    rank: u8,
    quiet: bool,
    action: Option<A>,
}

pub fn bind<A>(keys: &str, does: impl Into<Text>, action: A) -> Bind<A> {
    let mut made = Bind::bare(keys, does);
    made.action = Some(action);
    made
}

impl<A> Bind<A> {
    fn bare(keys: &str, does: impl Into<Text>) -> Bind<A> {
        let chords = keys
            .split_whitespace()
            .map(|word| Chord::parse(word).unwrap_or_else(|unknown| panic!("pfx-keys: {unknown}")))
            .collect::<Vec<_>>();
        assert!(!chords.is_empty(), "pfx-keys: a bind needs a key");
        Bind {
            chords,
            shown: None,
            keys: String::new(),
            does: does.into(),
            note: None,
            modes: Vec::new(),
            modes_text: String::new(),
            hay: String::new(),
            when: None,
            rank: 0,
            quiet: false,
            action: None,
        }
    }

    pub fn shown(mut self, keys: impl Into<Text>) -> Self {
        self.shown = Some(keys.into());
        self
    }

    pub fn note(mut self, note: impl Into<Text>) -> Self {
        self.note = Some(note.into());
        self
    }

    pub fn on(mut self, mode: Mode) -> Self {
        if !self.modes.contains(&mode) {
            self.modes.push(mode);
        }
        self
    }

    pub fn modes(mut self, modes: &[Mode]) -> Self {
        for mode in modes {
            self = self.on(*mode);
        }
        self
    }

    pub fn when(mut self, when: When) -> Self {
        self.when = Some(when);
        self
    }

    pub fn rank(mut self, rank: u8) -> Self {
        self.rank = rank;
        self
    }

    pub fn quiet(mut self) -> Self {
        self.quiet = true;
        self
    }

    pub fn chords(&self) -> &[Chord] {
        &self.chords
    }

    pub fn action(&self) -> Option<&A> {
        self.action.as_ref()
    }

    pub(crate) fn applies(&self, mode: Mode) -> bool {
        self.modes.is_empty() || self.modes.contains(&mode)
    }

    pub(crate) fn live(&self, now: &Now) -> bool {
        self.applies(now.mode) && now.holds(self.when)
    }

    pub(crate) fn live_open(&self, now: &Now) -> bool {
        self.applies(now.mode) && now.holds_open(self.when)
    }

    pub(crate) fn found(&self, terms: &[String]) -> bool {
        terms.iter().all(|term| self.hay.contains(term.as_str()))
    }

    fn settle(&mut self, joiner: &str, group: &str) {
        self.keys = match &self.shown {
            Some(shown) => shown.to_string(),
            None => self
                .chords
                .iter()
                .map(Chord::to_string)
                .collect::<Vec<_>>()
                .join(joiner),
        };
        self.modes_text = self
            .modes
            .iter()
            .map(|mode| mode.0)
            .collect::<Vec<_>>()
            .join(", ");
        self.hay = [
            group,
            &self.keys,
            &self.does,
            self.note.as_deref().unwrap_or(""),
            &self.modes_text,
            self.when.map_or("", |when| when.0),
        ]
        .join(" ")
        .to_lowercase();
    }

    pub(crate) fn row<'k>(&'k self, group: &'k str, live: bool) -> Row<'k> {
        Row {
            group,
            keys: &self.keys,
            does: &self.does,
            note: self.note.as_deref(),
            modes: &self.modes_text,
            when: self.when.map(|when| when.0),
            rank: self.rank,
            live,
        }
    }

    fn hint(&self) -> Hint<'_> {
        Hint {
            keys: &self.keys,
            does: &self.does,
            rank: self.rank,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Group<A> {
    pub(crate) name: Text,
    pub(crate) binds: Vec<Bind<A>>,
}

impl<A> Group<A> {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn binds(&self) -> &[Bind<A>] {
        &self.binds
    }
}

#[derive(Debug, Clone)]
pub struct Words {
    pub group: Text,
    pub open: Text,
    pub close: Text,
    pub search: Text,
    pub hint: Text,
    pub button: Text,
}

impl Default for Words {
    fn default() -> Self {
        Words {
            group: "Help".into(),
            open: "opens and closes this help".into(),
            close: "closes it".into(),
            search: "searches it".into(),
            hint: "help".into(),
            button: "? Help".into(),
        }
    }
}

type Spot = (usize, usize);

#[derive(Debug, Clone)]
struct Ranked {
    mode: Mode,
    here: Vec<Spot>,
    plain: Vec<Spot>,
}

#[derive(Debug, Clone)]
pub struct Keymap<A> {
    pub(crate) groups: Vec<Group<A>>,
    pub(crate) help: Group<A>,
    order: Vec<Mode>,
    joiner: Text,
    words: Words,
    ranked: Vec<Ranked>,
    anywhere: Vec<Spot>,
}

impl<A> Default for Keymap<A> {
    fn default() -> Self {
        Keymap::new()
    }
}

impl<A> Keymap<A> {
    pub fn new() -> Self {
        let keymap = Keymap {
            groups: Vec::new(),
            help: Group {
                name: Text::Borrowed(""),
                binds: Vec::new(),
            },
            order: Vec::new(),
            joiner: " / ".into(),
            words: Words::default(),
            ranked: Vec::new(),
            anywhere: Vec::new(),
        };
        keymap.words(Words::default())
    }

    pub fn words(mut self, words: Words) -> Self {
        self.help = Group {
            name: words.group.clone(),
            binds: vec![
                Bind::bare("?", words.open.clone()),
                Bind::bare("esc", words.close.clone()).when(HELP_OPEN),
                Bind::bare("/", words.search.clone()).when(HELP_OPEN),
            ],
        };
        self.words = words;
        self.settle();
        self
    }

    pub fn joiner(mut self, joiner: impl Into<Text>) -> Self {
        self.joiner = joiner.into();
        self.settle();
        self
    }

    pub fn modes(mut self, order: &[Mode]) -> Self {
        self.order.clear();
        for mode in order {
            self.learn(*mode);
        }
        self.rank_hints();
        self
    }

    pub fn group(
        mut self,
        name: impl Into<Text>,
        binds: impl IntoIterator<Item = Bind<A>>,
    ) -> Self {
        let name = name.into();
        let mut binds: Vec<Bind<A>> = binds.into_iter().collect();
        for bind in &mut binds {
            bind.settle(&self.joiner, &name);
            for mode in bind.modes.clone() {
                self.learn(mode);
            }
        }
        self.groups.push(Group { name, binds });
        self.rank_hints();
        self
    }

    pub fn groups(&self) -> &[Group<A>] {
        &self.groups
    }

    pub fn order(&self) -> &[Mode] {
        &self.order
    }

    pub fn button(&self) -> &str {
        &self.words.button
    }

    pub fn action(&self, now: Now, chord: Chord) -> Option<&A> {
        self.groups
            .iter()
            .flat_map(|group| &group.binds)
            .filter(|bind| bind.live(&now) && bind.chords.contains(&chord))
            .find_map(|bind| bind.action.as_ref())
    }

    pub fn hints(&self, now: Now) -> Vec<Hint<'_>> {
        let spots = self
            .ranked
            .iter()
            .find(|ranked| ranked.mode == now.mode)
            .map_or(&self.anywhere, |ranked| &ranked.here);
        let mut hints = Vec::with_capacity(spots.len() + 1);
        let mut help = Some(self.help_hint());
        for bind in spots.iter().map(|spot| self.at(*spot)) {
            if !now.holds(bind.when) {
                continue;
            }
            if bind.rank > 0
                && let Some(help) = help.take()
            {
                hints.push(help);
            }
            hints.push(bind.hint());
        }
        hints.extend(help);
        hints
    }

    pub fn ticker(&self, now: Now) -> Vec<Line<'_>> {
        let mut lines = Vec::with_capacity(self.ranked.len() + 1);
        lines.push(Line {
            mode: now.mode,
            now: true,
            hints: self.hints(now),
        });
        for ranked in &self.ranked {
            if ranked.mode == now.mode || ranked.plain.is_empty() {
                continue;
            }
            lines.push(Line {
                mode: ranked.mode,
                now: false,
                hints: ranked
                    .plain
                    .iter()
                    .map(|spot| self.at(*spot).hint())
                    .collect(),
            });
        }
        lines
    }

    pub fn clashes(&self) -> Vec<Clash<'_>> {
        let all: Vec<(&str, &Bind<A>)> = self
            .groups
            .iter()
            .flat_map(|group| {
                group
                    .binds
                    .iter()
                    .map(move |bind| (group.name.as_ref(), bind))
            })
            .chain(std::iter::once((
                self.help.name.as_ref(),
                &self.help.binds[0],
            )))
            .collect();
        let mut clashes = Vec::new();
        for (at, (first_group, first)) in all.iter().enumerate() {
            for (second_group, second) in &all[at + 1..] {
                let overlap = first.modes.is_empty()
                    || second.modes.is_empty()
                    || first.modes.iter().any(|mode| second.modes.contains(mode));
                let shadows = first.when == second.when || first.when.is_none();
                if !overlap || !shadows {
                    continue;
                }
                if let Some(chord) = first
                    .chords
                    .iter()
                    .find(|chord| second.chords.contains(chord))
                {
                    clashes.push(Clash {
                        chord: *chord,
                        first: first.row(first_group, false),
                        second: second.row(second_group, false),
                    });
                }
            }
        }
        clashes
    }

    fn help_hint(&self) -> Hint<'_> {
        Hint {
            keys: self.help.binds[0].keys.as_str(),
            does: &self.words.hint,
            rank: 0,
        }
    }

    fn at(&self, (group, bind): Spot) -> &Bind<A> {
        &self.groups[group].binds[bind]
    }

    fn spots(&self, keep: impl Fn(&Bind<A>) -> bool) -> Vec<Spot> {
        let mut spots: Vec<Spot> = self
            .groups
            .iter()
            .enumerate()
            .flat_map(|(group, binds)| {
                binds
                    .binds
                    .iter()
                    .enumerate()
                    .filter(|(_, bind)| !bind.quiet && keep(bind))
                    .map(move |(bind, _)| (group, bind))
            })
            .collect();
        spots.sort_by_key(|spot| self.at(*spot).rank);
        spots
    }

    fn rank_hints(&mut self) {
        let mut modes = self.order.clone();
        for mode in self
            .groups
            .iter()
            .flat_map(|group| &group.binds)
            .flat_map(|bind| &bind.modes)
        {
            if !modes.contains(mode) {
                modes.push(*mode);
            }
        }
        self.ranked = modes
            .iter()
            .map(|mode| Ranked {
                mode: *mode,
                here: self.spots(|bind| bind.applies(*mode)),
                plain: match self.order.contains(mode) {
                    true => self.spots(|bind| bind.when.is_none() && bind.modes.contains(mode)),
                    false => Vec::new(),
                },
            })
            .collect();
        self.anywhere = self.spots(|bind| bind.modes.is_empty());
    }

    fn learn(&mut self, mode: Mode) {
        if !self.order.contains(&mode) {
            self.order.push(mode);
        }
    }

    fn settle(&mut self) {
        let joiner = self.joiner.clone();
        for group in self
            .groups
            .iter_mut()
            .chain(std::iter::once(&mut self.help))
        {
            for bind in &mut group.binds {
                bind.settle(&joiner, &group.name);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row<'k> {
    pub group: &'k str,
    pub keys: &'k str,
    pub does: &'k str,
    pub note: Option<&'k str>,
    pub modes: &'k str,
    pub when: Option<&'k str>,
    pub rank: u8,
    pub live: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hint<'k> {
    pub keys: &'k str,
    pub does: &'k str,
    pub rank: u8,
}

impl fmt::Display for Hint<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.keys, self.does)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line<'k> {
    pub mode: Mode,
    pub now: bool,
    pub hints: Vec<Hint<'k>>,
}

impl fmt::Display for Line<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (at, hint) in self.hints.iter().enumerate() {
            if at > 0 {
                f.write_str(" · ")?;
            }
            write!(f, "{hint}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clash<'k> {
    pub chord: Chord,
    pub first: Row<'k>,
    pub second: Row<'k>,
}

impl fmt::Display for Clash<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: '{}' ({}) and '{}' ({}) answer it in the same place, and the first wins",
            self.chord, self.first.does, self.first.group, self.second.does, self.second.group
        )
    }
}
