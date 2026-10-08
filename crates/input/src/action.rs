use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::device::{Axis, Button, Control, Key, MouseButton, Sign, Stick, Wheel};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionKind {
    Digital,
    Axis,
    Stick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Repeat {
    pub delay: u32,
    pub interval: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActionSpec {
    pub name: String,
    pub kind: ActionKind,
    pub context: String,
    pub dead_zone: f32,
    pub repeat: Option<Repeat>,
    pub keyboard: Vec<Binding>,
    pub gamepad: Vec<Binding>,
}

impl ActionSpec {
    fn new(name: &str, kind: ActionKind, dead_zone: f32) -> Self {
        Self {
            name: name.to_owned(),
            kind,
            context: String::new(),
            dead_zone,
            repeat: None,
            keyboard: Vec::new(),
            gamepad: Vec::new(),
        }
    }

    pub fn digital(name: &str) -> Self {
        Self::new(name, ActionKind::Digital, 0.0)
    }

    pub fn axis(name: &str) -> Self {
        Self::new(name, ActionKind::Axis, 0.2)
    }

    pub fn stick(name: &str) -> Self {
        Self::new(name, ActionKind::Stick, 0.2)
    }

    pub fn context(mut self, context: &str) -> Self {
        context.clone_into(&mut self.context);
        self
    }

    pub fn dead_zone(mut self, dead_zone: f32) -> Self {
        self.dead_zone = dead_zone.clamp(0.0, 0.95);
        self
    }

    pub fn repeat(mut self, delay: u32, interval: u32) -> Self {
        self.repeat = Some(Repeat {
            delay: delay.max(1),
            interval: interval.max(1),
        });
        self
    }

    pub fn key(mut self, binding: Binding) -> Self {
        self.keyboard.push(binding);
        self
    }

    pub fn pad(mut self, binding: Binding) -> Self {
        self.gamepad.push(binding);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ActionId(pub u16);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Actions {
    specs: Vec<ActionSpec>,
    names: BTreeMap<String, ActionId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionsError {
    Duplicate(String),
    TooMany,
}

impl fmt::Display for ActionsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionsError::Duplicate(name) => write!(f, "action {name:?} is named twice"),
            ActionsError::TooMany => write!(f, "more than 65535 actions"),
        }
    }
}

impl std::error::Error for ActionsError {}

impl Actions {
    pub fn new(specs: Vec<ActionSpec>) -> Result<Self, ActionsError> {
        if specs.len() > usize::from(u16::MAX) {
            return Err(ActionsError::TooMany);
        }
        let mut names = BTreeMap::new();
        for (index, spec) in specs.iter().enumerate() {
            if names
                .insert(spec.name.clone(), ActionId(index as u16))
                .is_some()
            {
                return Err(ActionsError::Duplicate(spec.name.clone()));
            }
        }
        Ok(Self { specs, names })
    }

    pub fn id(&self, name: &str) -> Option<ActionId> {
        self.names.get(name).copied()
    }

    pub fn spec(&self, id: ActionId) -> &ActionSpec {
        &self.specs[usize::from(id.0)]
    }

    pub fn len(&self) -> usize {
        self.specs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (ActionId, &ActionSpec)> {
        self.specs
            .iter()
            .enumerate()
            .map(|(index, spec)| (ActionId(index as u16), spec))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Source {
    Key(Key),
    Mouse(MouseButton),
    Wheel(Wheel),
    Button(Button),
    Axis(Axis, Sign),
}

impl Source {
    pub fn is_keyboard(self) -> bool {
        matches!(self, Source::Key(_))
    }

    pub fn is_mouse(self) -> bool {
        matches!(self, Source::Mouse(_) | Source::Wheel(_))
    }

    pub fn is_gamepad(self) -> bool {
        matches!(self, Source::Button(_) | Source::Axis(..))
    }

    pub fn control(self) -> Control {
        match self {
            Source::Key(key) => Control::Key(key),
            Source::Mouse(button) => Control::Mouse(button),
            Source::Wheel(wheel) => Control::Wheel(wheel),
            Source::Button(button) => Control::Button(button),
            Source::Axis(axis, _) => Control::Stick(axis.stick()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Binding {
    Press(Source),
    Axis {
        negative: Source,
        positive: Source,
    },
    PadAxis(Axis),
    Stick {
        up: Source,
        down: Source,
        left: Source,
        right: Source,
    },
    PadStick(Stick),
}

impl Binding {
    pub fn key(key: Key) -> Self {
        Binding::Press(Source::Key(key))
    }

    pub fn mouse(button: MouseButton) -> Self {
        Binding::Press(Source::Mouse(button))
    }

    pub fn button(button: Button) -> Self {
        Binding::Press(Source::Button(button))
    }

    pub fn keys(negative: Key, positive: Key) -> Self {
        Binding::Axis {
            negative: Source::Key(negative),
            positive: Source::Key(positive),
        }
    }

    pub fn buttons(negative: Button, positive: Button) -> Self {
        Binding::Axis {
            negative: Source::Button(negative),
            positive: Source::Button(positive),
        }
    }

    pub fn key_stick(up: Key, down: Key, left: Key, right: Key) -> Self {
        Binding::Stick {
            up: Source::Key(up),
            down: Source::Key(down),
            left: Source::Key(left),
            right: Source::Key(right),
        }
    }

    pub fn dpad() -> Self {
        Binding::Stick {
            up: Source::Button(Button::DPadUp),
            down: Source::Button(Button::DPadDown),
            left: Source::Button(Button::DPadLeft),
            right: Source::Button(Button::DPadRight),
        }
    }

    pub fn sources(&self) -> Vec<Source> {
        match *self {
            Binding::Press(source) => vec![source],
            Binding::Axis { negative, positive } => vec![negative, positive],
            Binding::PadAxis(axis) => vec![
                Source::Axis(axis, Sign::Negative),
                Source::Axis(axis, Sign::Positive),
            ],
            Binding::Stick {
                up,
                down,
                left,
                right,
            } => vec![up, down, left, right],
            Binding::PadStick(stick) => {
                let (x, y) = stick.axes();
                vec![
                    Source::Axis(y, Sign::Positive),
                    Source::Axis(y, Sign::Negative),
                    Source::Axis(x, Sign::Negative),
                    Source::Axis(x, Sign::Positive),
                ]
            }
        }
    }

    pub fn part_count(&self) -> usize {
        match self {
            Binding::Press(_) => 1,
            Binding::Axis { .. } => 2,
            Binding::Stick { .. } => 4,
            Binding::PadAxis(_) | Binding::PadStick(_) => 0,
        }
    }

    fn part(&self, part: usize) -> Option<Source> {
        if part >= self.part_count() {
            return None;
        }
        self.sources().get(part).copied()
    }

    fn with_part(&self, part: usize, source: Source) -> Option<Binding> {
        let mut binding = *self;
        match &mut binding {
            Binding::Press(slot) if part == 0 => *slot = source,
            Binding::Axis { negative, .. } if part == 0 => *negative = source,
            Binding::Axis { positive, .. } if part == 1 => *positive = source,
            Binding::Stick { up, .. } if part == 0 => *up = source,
            Binding::Stick { down, .. } if part == 1 => *down = source,
            Binding::Stick { left, .. } if part == 2 => *left = source,
            Binding::Stick { right, .. } if part == 3 => *right = source,
            _ => return None,
        }
        Some(binding)
    }

    pub fn controls(&self) -> Vec<Control> {
        match *self {
            Binding::PadAxis(axis) => vec![Control::Stick(axis.stick())],
            Binding::PadStick(stick) => vec![Control::Stick(stick)],
            Binding::Stick {
                up: Source::Button(Button::DPadUp),
                down: Source::Button(Button::DPadDown),
                left: Source::Button(Button::DPadLeft),
                right: Source::Button(Button::DPadRight),
            } => vec![Control::DPad],
            _ => {
                let mut controls = Vec::new();
                for source in self.sources() {
                    let control = source.control();
                    if !controls.contains(&control) {
                        controls.push(control);
                    }
                }
                controls
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Set {
    Keyboard,
    Gamepad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Slot {
    pub set: Set,
    pub index: usize,
    pub part: usize,
}

impl Slot {
    pub fn keyboard(index: usize, part: usize) -> Self {
        Self {
            set: Set::Keyboard,
            index,
            part,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OnConflict {
    Refuse,
    Swap,
    Replace,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub action: String,
    pub slot: Slot,
    pub source: Source,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebindError {
    UnknownAction(String),
    NoSlot(Slot),
    WrongDevice(Source),
    Conflicts(Vec<Conflict>),
}

impl fmt::Display for RebindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RebindError::UnknownAction(name) => write!(f, "no action named {name:?}"),
            RebindError::NoSlot(slot) => write!(f, "no binding at {slot:?}"),
            RebindError::WrongDevice(source) => {
                write!(f, "{source:?} is not a keyboard or mouse control")
            }
            RebindError::Conflicts(conflicts) => {
                write!(f, "already bound to ")?;
                for (index, conflict) in conflicts.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", conflict.action)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for RebindError {}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingMap {
    pub keyboard: BTreeMap<String, Vec<Binding>>,
    pub gamepad: BTreeMap<String, Vec<Binding>>,
}

impl BindingMap {
    pub fn defaults(actions: &Actions) -> Self {
        let mut map = Self::default();
        for (_, spec) in actions.iter() {
            map.keyboard
                .insert(spec.name.clone(), spec.keyboard.clone());
            map.gamepad.insert(spec.name.clone(), spec.gamepad.clone());
        }
        map
    }

    pub fn fit(&mut self, actions: &Actions) {
        self.keyboard.retain(|name, _| actions.id(name).is_some());
        self.gamepad.retain(|name, _| actions.id(name).is_some());
        for (_, spec) in actions.iter() {
            self.keyboard
                .entry(spec.name.clone())
                .or_insert_with(|| spec.keyboard.clone());
            self.gamepad
                .entry(spec.name.clone())
                .or_insert_with(|| spec.gamepad.clone());
        }
    }

    pub fn reset(&mut self, actions: &Actions, name: &str) -> Result<(), RebindError> {
        let id = actions
            .id(name)
            .ok_or_else(|| RebindError::UnknownAction(name.to_owned()))?;
        let spec = actions.spec(id);
        self.keyboard
            .insert(spec.name.clone(), spec.keyboard.clone());
        self.gamepad.insert(spec.name.clone(), spec.gamepad.clone());
        Ok(())
    }

    pub fn set(&self, set: Set) -> &BTreeMap<String, Vec<Binding>> {
        match set {
            Set::Keyboard => &self.keyboard,
            Set::Gamepad => &self.gamepad,
        }
    }

    fn set_mut(&mut self, set: Set) -> &mut BTreeMap<String, Vec<Binding>> {
        match set {
            Set::Keyboard => &mut self.keyboard,
            Set::Gamepad => &mut self.gamepad,
        }
    }

    pub fn bindings(&self, set: Set, name: &str) -> &[Binding] {
        self.set(set).get(name).map_or(&[], Vec::as_slice)
    }

    pub fn source_at(&self, name: &str, slot: Slot) -> Option<Source> {
        self.bindings(slot.set, name)
            .get(slot.index)?
            .part(slot.part)
    }

    pub fn conflicts(
        &self,
        actions: &Actions,
        name: &str,
        slot: Slot,
        source: Source,
    ) -> Vec<Conflict> {
        let Some(id) = actions.id(name) else {
            return Vec::new();
        };
        let context = &actions.spec(id).context;
        let mut found = Vec::new();
        for (other_name, bindings) in self.set(slot.set) {
            let Some(other) = actions.id(other_name) else {
                continue;
            };
            if &actions.spec(other).context != context {
                continue;
            }
            for (index, binding) in bindings.iter().enumerate() {
                for part in 0..binding.part_count() {
                    let at = Slot {
                        set: slot.set,
                        index,
                        part,
                    };
                    if other_name == name && at == slot {
                        continue;
                    }
                    if binding.part(part) == Some(source) {
                        found.push(Conflict {
                            action: other_name.clone(),
                            slot: at,
                            source,
                        });
                    }
                }
            }
        }
        found
    }

    pub fn rebind_key(
        &mut self,
        actions: &Actions,
        name: &str,
        slot: Slot,
        source: Source,
        on_conflict: OnConflict,
    ) -> Result<Vec<Conflict>, RebindError> {
        if slot.set != Set::Keyboard || !(source.is_keyboard() || source.is_mouse()) {
            return Err(RebindError::WrongDevice(source));
        }
        self.rebind(actions, name, slot, source, on_conflict)
    }

    pub fn rebind(
        &mut self,
        actions: &Actions,
        name: &str,
        slot: Slot,
        source: Source,
        on_conflict: OnConflict,
    ) -> Result<Vec<Conflict>, RebindError> {
        if actions.id(name).is_none() {
            return Err(RebindError::UnknownAction(name.to_owned()));
        }
        let previous = self
            .source_at(name, slot)
            .ok_or(RebindError::NoSlot(slot))?;
        let conflicts = self.conflicts(actions, name, slot, source);
        let mut slot = slot;
        if !conflicts.is_empty() {
            match on_conflict {
                OnConflict::Refuse => return Err(RebindError::Conflicts(conflicts)),
                OnConflict::Swap => {
                    for conflict in &conflicts {
                        self.write(&conflict.action, conflict.slot, previous);
                    }
                }
                OnConflict::Replace => {
                    let mut dropped: Vec<(&str, usize)> = Vec::new();
                    for conflict in conflicts.iter().rev() {
                        if conflict.action == name && conflict.slot.index == slot.index {
                            self.write(name, conflict.slot, previous);
                            continue;
                        }
                        if dropped.contains(&(conflict.action.as_str(), conflict.slot.index)) {
                            continue;
                        }
                        dropped.push((conflict.action.as_str(), conflict.slot.index));
                        self.drop_binding(&conflict.action, conflict.slot);
                        if conflict.action == name && conflict.slot.index < slot.index {
                            slot.index -= 1;
                        }
                    }
                }
            }
        }
        self.write(name, slot, source);
        Ok(conflicts)
    }

    fn write(&mut self, name: &str, slot: Slot, source: Source) {
        if let Some(binding) = self
            .set_mut(slot.set)
            .get_mut(name)
            .and_then(|bindings| bindings.get_mut(slot.index))
            && let Some(next) = binding.with_part(slot.part, source)
        {
            *binding = next;
        }
    }

    fn drop_binding(&mut self, name: &str, slot: Slot) {
        if let Some(bindings) = self.set_mut(slot.set).get_mut(name)
            && slot.index < bindings.len()
        {
            bindings.remove(slot.index);
        }
    }

    pub fn add(&mut self, name: &str, set: Set, binding: Binding) {
        self.set_mut(set)
            .entry(name.to_owned())
            .or_default()
            .push(binding);
    }

    pub fn remove(&mut self, name: &str, set: Set, index: usize) -> Option<Binding> {
        let bindings = self.set_mut(set).get_mut(name)?;
        (index < bindings.len()).then(|| bindings.remove(index))
    }
}
