use std::collections::{BTreeMap, BTreeSet};

use pfx_gpu::window::Viewport;
use serde::{Deserialize, Serialize};

use crate::action::{ActionId, ActionKind, Actions, Binding, BindingMap, Set, Source};
use crate::backend::Backend;
use crate::device::{
    Axis, Button, Control, DeviceKind, Key, Motors, MouseButton, PadId, PadInfo, Sign, Wheel,
};
use crate::event::InputEvent;
use crate::family::{Family, GlyphStyle};
use crate::record::{Recording, Start};
use crate::rumble::{MotorCommand, Rumble, Rumbler};

pub const PRESS: f32 = 0.5;
pub const POINTER_SWITCH: f32 = 4.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionState {
    pub held: bool,
    pub pressed: bool,
    pub released: bool,
    pub repeated: bool,
    pub held_steps: u32,
    pub value: f32,
    pub vector: [f32; 2],
}

impl ActionState {
    pub fn fired(&self) -> bool {
        self.pressed || self.repeated
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Active {
    pub kind: DeviceKind,
    pub pad: Option<PadId>,
    pub family: Option<Family>,
}

impl Active {
    pub fn keyboard() -> Self {
        Self {
            kind: DeviceKind::Keyboard,
            pad: None,
            family: None,
        }
    }

    fn mouse() -> Self {
        Self {
            kind: DeviceKind::Mouse,
            pad: None,
            family: None,
        }
    }

    pub fn set(&self) -> Set {
        if self.kind.is_gamepad() {
            Set::Gamepad
        } else {
            Set::Keyboard
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pointer {
    pub layout: Option<[f32; 2]>,
    pub inside: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub index: u64,
    pub changed: Option<Active>,
    pub style: Option<GlyphStyle>,
    pub motors: Vec<MotorCommand>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Level {
    value: f32,
    tapped: bool,
}

impl Level {
    fn tap(&self) -> bool {
        self.tapped && self.value < PRESS
    }

    fn set(&mut self, value: f32) -> bool {
        let rising = self.value < PRESS && value >= PRESS;
        self.value = value;
        if rising {
            self.tapped = true;
        }
        rising
    }
}

#[derive(Clone, Debug)]
struct Pad {
    info: PadInfo,
    buttons: BTreeMap<Button, Level>,
    axes: BTreeMap<Axis, f32>,
    actions: BTreeMap<String, [f32; 2]>,
    origins: BTreeMap<String, Vec<Control>>,
}

impl Pad {
    fn new(info: PadInfo) -> Self {
        Self {
            info,
            buttons: BTreeMap::new(),
            axes: BTreeMap::new(),
            actions: BTreeMap::new(),
            origins: BTreeMap::new(),
        }
    }

    fn source(&self, source: Source) -> Level {
        match source {
            Source::Button(button) => self.buttons.get(&button).copied().unwrap_or_default(),
            Source::Axis(axis, sign) => {
                let value = self.axes.get(&axis).copied().unwrap_or(0.0);
                let value = match sign {
                    Sign::Positive => value,
                    Sign::Negative => -value,
                };
                Level {
                    value: value.max(0.0),
                    tapped: false,
                }
            }
            _ => Level::default(),
        }
    }

    fn axis(&self, axis: Axis) -> f32 {
        self.axes.get(&axis).copied().unwrap_or(0.0)
    }
}

pub struct Input {
    actions: Actions,
    map: BindingMap,
    resolved: Vec<[Vec<Binding>; 2]>,
    keys: BTreeMap<Source, Level>,
    wheel_pending: [f32; 2],
    wheel: [f32; 2],
    pads: BTreeMap<PadId, Pad>,
    states: Vec<ActionState>,
    pointer: Pointer,
    outside: BTreeSet<MouseButton>,
    anchor: Option<[f32; 2]>,
    viewport: Option<Viewport>,
    active: Active,
    reported: Active,
    reported_style: GlyphStyle,
    last_pad: Option<PadId>,
    step: u64,
    step_us: u32,
    rumbler: Rumbler,
    recording: Option<Recording>,
    pending: Vec<InputEvent>,
}

impl Input {
    pub fn new(actions: Actions, step_us: u32) -> Self {
        let map = BindingMap::defaults(&actions);
        let states = vec![ActionState::default(); actions.len()];
        let mut input = Self {
            actions,
            map,
            resolved: Vec::new(),
            keys: BTreeMap::new(),
            wheel_pending: [0.0; 2],
            wheel: [0.0; 2],
            pads: BTreeMap::new(),
            states,
            pointer: Pointer::default(),
            outside: BTreeSet::new(),
            anchor: None,
            viewport: None,
            active: Active::keyboard(),
            reported: Active::keyboard(),
            reported_style: GlyphStyle::Standard,
            last_pad: None,
            step: 0,
            step_us: step_us.max(1),
            rumbler: Rumbler::new(step_us, 100),
            recording: None,
            pending: Vec::new(),
        };
        input.resolve();
        input
    }

    pub fn actions(&self) -> &Actions {
        &self.actions
    }

    pub fn bindings(&self) -> &BindingMap {
        &self.map
    }

    pub fn set_bindings(&mut self, mut map: BindingMap) {
        map.fit(&self.actions);
        self.map = map;
        self.resolve();
    }

    pub fn edit_bindings<R>(&mut self, edit: impl FnOnce(&mut BindingMap, &Actions) -> R) -> R {
        let result = edit(&mut self.map, &self.actions);
        self.map.fit(&self.actions);
        self.resolve();
        result
    }

    fn resolve(&mut self) {
        self.resolved = self
            .actions
            .iter()
            .map(|(_, spec)| {
                [
                    self.map.bindings(Set::Keyboard, &spec.name).to_vec(),
                    self.map.bindings(Set::Gamepad, &spec.name).to_vec(),
                ]
            })
            .collect();
    }

    pub fn set_viewport(&mut self, viewport: Option<Viewport>) {
        self.viewport = viewport;
    }

    pub fn step_index(&self) -> u64 {
        self.step
    }

    pub fn step_us(&self) -> u32 {
        self.step_us
    }

    pub fn state(&self, id: ActionId) -> ActionState {
        self.states[usize::from(id.0)]
    }

    pub fn action(&self, name: &str) -> ActionState {
        self.actions
            .id(name)
            .map_or_else(ActionState::default, |id| self.state(id))
    }

    pub fn states(&self) -> &[ActionState] {
        &self.states
    }

    pub fn pointer(&self) -> Pointer {
        self.pointer
    }

    pub fn wheel(&self) -> [f32; 2] {
        self.wheel
    }

    pub fn active(&self) -> Active {
        self.active
    }

    pub fn last_family(&self) -> Option<Family> {
        self.last_pad
            .and_then(|pad| self.pads.get(&pad))
            .map(|pad| pad.info.family)
    }

    pub fn pads(&self) -> impl Iterator<Item = (PadId, &PadInfo)> {
        self.pads.iter().map(|(id, pad)| (*id, &pad.info))
    }

    pub fn key_held(&self, key: Key) -> bool {
        self.keys
            .get(&Source::Key(key))
            .is_some_and(|level| level.value >= PRESS)
    }

    pub fn mouse_held(&self, button: MouseButton) -> bool {
        self.keys
            .get(&Source::Mouse(button))
            .is_some_and(|level| level.value >= PRESS)
    }

    pub fn mouse_outside(&self, button: MouseButton) -> bool {
        self.outside.contains(&button)
    }

    pub fn feed(&mut self, event: InputEvent) {
        let event = self.normalize(event);
        self.apply(&event);
        if self.recording.is_some() {
            self.pending.push(event);
        }
    }

    fn apply(&mut self, event: &InputEvent) {
        match event {
            InputEvent::Key { key, pressed } => {
                let rising = self
                    .keys
                    .entry(Source::Key(*key))
                    .or_default()
                    .set(if *pressed { 1.0 } else { 0.0 });
                if rising {
                    self.active = Active::keyboard();
                }
            }
            InputEvent::MouseButton { button, pressed } => {
                if *pressed && self.pointer.layout.is_some() && !self.pointer.inside {
                    self.outside.insert(*button);
                } else {
                    self.outside.remove(button);
                }
                let rising = self
                    .keys
                    .entry(Source::Mouse(*button))
                    .or_default()
                    .set(if *pressed { 1.0 } else { 0.0 });
                if rising {
                    self.active = Active::mouse();
                }
            }
            InputEvent::Wheel { x, y } => {
                if x.is_finite() && y.is_finite() {
                    self.wheel_pending[0] += x;
                    self.wheel_pending[1] += y;
                    for (wheel, on) in [
                        (Wheel::Up, *y > 0.0),
                        (Wheel::Down, *y < 0.0),
                        (Wheel::Left, *x < 0.0),
                        (Wheel::Right, *x > 0.0),
                    ] {
                        if on {
                            self.keys.entry(Source::Wheel(wheel)).or_default().tapped = true;
                            self.active = Active::mouse();
                        }
                    }
                }
            }
            InputEvent::PointerMoved { .. } => {}
            InputEvent::PointerAt { layout, inside } => {
                match self.anchor {
                    Some(anchor) if self.active.kind != DeviceKind::Mouse => {
                        let dx = layout[0] - anchor[0];
                        let dy = layout[1] - anchor[1];
                        if dx * dx + dy * dy >= POINTER_SWITCH * POINTER_SWITCH {
                            self.active = Active::mouse();
                            self.anchor = Some(*layout);
                        }
                    }
                    _ => self.anchor = Some(*layout),
                }
                self.pointer = Pointer {
                    layout: Some(*layout),
                    inside: *inside,
                };
            }
            InputEvent::PointerLeft => {
                self.pointer = Pointer::default();
                self.anchor = None;
            }
            InputEvent::FocusLost => {
                for level in self.keys.values_mut() {
                    level.value = 0.0;
                }
                self.outside.clear();
            }
            InputEvent::PadConnected { pad, info } => {
                self.pads.insert(*pad, Pad::new(info.clone()));
                if self.active.pad == Some(*pad) {
                    self.active.family = Some(info.family);
                }
            }
            InputEvent::PadDisconnected { pad } => {
                self.pads.remove(pad);
                self.rumbler.forget(*pad);
                if self.last_pad == Some(*pad) {
                    self.last_pad = None;
                }
                if self.active.pad == Some(*pad) {
                    self.active = Active::keyboard();
                }
            }
            InputEvent::PadButton { pad, button, value } => {
                let value = finite_unit(*value);
                let rising = self
                    .pad_mut(*pad)
                    .buttons
                    .entry(*button)
                    .or_default()
                    .set(value);
                if rising {
                    self.touch_pad(*pad);
                }
            }
            InputEvent::PadAxis { pad, axis, value } => {
                let value = if value.is_finite() {
                    value.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
                let entry = self.pad_mut(*pad);
                let before = entry.axis(*axis).abs();
                entry.axes.insert(*axis, value);
                if before < PRESS && value.abs() >= PRESS {
                    self.touch_pad(*pad);
                }
            }
            InputEvent::PadAction { pad, action, value } => {
                let value = [finite_signed(value[0]), finite_signed(value[1])];
                let entry = self.pad_mut(*pad);
                let before = entry.actions.get(action).copied().unwrap_or([0.0; 2]);
                entry.actions.insert(action.clone(), value);
                if magnitude(before) < PRESS && magnitude(value) >= PRESS {
                    self.touch_pad(*pad);
                }
            }
            InputEvent::PadOrigins {
                pad,
                action,
                controls,
            } => {
                self.pad_mut(*pad)
                    .origins
                    .insert(action.clone(), controls.clone());
            }
        }
    }

    fn normalize(&self, event: InputEvent) -> InputEvent {
        match event {
            InputEvent::PointerMoved { window } => match self.viewport {
                Some(viewport) => InputEvent::PointerAt {
                    layout: viewport.pointer_to_layout(window[0], window[1]),
                    inside: viewport.pointer_inside(window[0], window[1]),
                },
                None => InputEvent::PointerAt {
                    layout: window,
                    inside: true,
                },
            },
            InputEvent::Key { .. }
            | InputEvent::MouseButton { .. }
            | InputEvent::Wheel { .. }
            | InputEvent::PointerAt { .. }
            | InputEvent::PointerLeft
            | InputEvent::FocusLost
            | InputEvent::PadConnected { .. }
            | InputEvent::PadDisconnected { .. }
            | InputEvent::PadButton { .. }
            | InputEvent::PadAxis { .. }
            | InputEvent::PadAction { .. }
            | InputEvent::PadOrigins { .. } => event,
        }
    }

    fn pad_mut(&mut self, pad: PadId) -> &mut Pad {
        self.pads.entry(pad).or_insert_with(|| {
            Pad::new(PadInfo {
                name: String::new(),
                vendor: None,
                product: None,
                family: Family::Generic,
                motors: Motors::None,
                resolves_actions: false,
            })
        })
    }

    fn touch_pad(&mut self, pad: PadId) {
        let family = self.pads.get(&pad).map(|pad| pad.info.family);
        self.last_pad = Some(pad);
        self.active = Active {
            kind: DeviceKind::Gamepad,
            pad: Some(pad),
            family,
        };
    }

    pub fn step(&mut self) -> Step {
        for index in 0..self.states.len() {
            let id = ActionId(index as u16);
            let (held, value, vector) = self.evaluate(id);
            let spec = self.actions.spec(id);
            let repeat = spec.repeat;
            let state = &mut self.states[index];
            let was = state.held;
            state.pressed = held && !was;
            state.released = !held && was;
            state.held_steps = if held && was {
                state.held_steps.saturating_add(1)
            } else {
                0
            };
            state.repeated = held
                && was
                && repeat.is_some_and(|repeat| {
                    state.held_steps >= repeat.delay
                        && (state.held_steps - repeat.delay).is_multiple_of(repeat.interval)
                });
            state.held = held;
            state.value = value;
            state.vector = vector;
        }
        for level in self.keys.values_mut() {
            level.tapped = false;
        }
        for pad in self.pads.values_mut() {
            for level in pad.buttons.values_mut() {
                level.tapped = false;
            }
        }
        self.wheel = self.wheel_pending;
        self.wheel_pending = [0.0; 2];
        let changed = (self.active != self.reported).then_some(self.active);
        self.reported = self.active;
        let style = self.glyph_style();
        let style_changed = (style != self.reported_style).then_some(style);
        self.reported_style = style;
        let motors = self.rumbler.step();
        if let Some(recording) = &mut self.recording {
            recording.steps.push(std::mem::take(&mut self.pending));
        }
        let index = self.step;
        self.step += 1;
        Step {
            index,
            changed,
            style: style_changed,
            motors,
        }
    }

    pub fn update(&mut self, backend: &mut impl Backend) -> Step {
        let mut events = Vec::new();
        backend.poll(&mut events);
        for event in events {
            self.feed(event);
        }
        let step = self.step();
        for command in &step.motors {
            backend.set_motors(command);
        }
        step
    }

    fn level(&self, source: Source, set: Set) -> Level {
        match set {
            Set::Keyboard => match source {
                Source::Mouse(button) if self.outside.contains(&button) => Level::default(),
                _ => self.keys.get(&source).copied().unwrap_or_default(),
            },
            Set::Gamepad => {
                let mut best = Level::default();
                for pad in self.pads.values() {
                    if pad.info.resolves_actions {
                        continue;
                    }
                    let level = pad.source(source);
                    best.value = best.value.max(level.value);
                    best.tapped |= level.tapped;
                }
                best
            }
        }
    }

    fn stick_axes(&self, x: Axis, y: Axis) -> [f32; 2] {
        let mut best = [0.0f32; 2];
        for pad in self.pads.values() {
            if pad.info.resolves_actions {
                continue;
            }
            let v = [pad.axis(x), pad.axis(y)];
            if magnitude(v) > magnitude(best) {
                best = v;
            }
        }
        best
    }

    fn evaluate(&self, id: ActionId) -> (bool, f32, [f32; 2]) {
        let spec = self.actions.spec(id);
        let dead_zone = spec.dead_zone;
        let mut held = false;
        let mut best = [0.0f32; 2];
        for (set, bindings) in [Set::Keyboard, Set::Gamepad]
            .into_iter()
            .zip(&self.resolved[usize::from(id.0)])
        {
            for binding in bindings {
                let (tapped, vector) = self.binding_value(binding, set, dead_zone);
                held |= tapped;
                if magnitude(vector) > magnitude(best) {
                    best = vector;
                }
            }
        }
        for pad in self.pads.values() {
            if !pad.info.resolves_actions {
                continue;
            }
            if let Some(value) = pad.actions.get(&spec.name) {
                let vector = match spec.kind {
                    ActionKind::Digital | ActionKind::Axis => [value[0], 0.0],
                    ActionKind::Stick => radial(*value, dead_zone),
                };
                if magnitude(vector) > magnitude(best) {
                    best = vector;
                }
            }
        }
        held |= magnitude(best) >= PRESS;
        match spec.kind {
            ActionKind::Digital => {
                let value = magnitude(best).min(1.0);
                (held, value, [value, 0.0])
            }
            ActionKind::Axis => (held, best[0], [best[0], 0.0]),
            ActionKind::Stick => (held, magnitude(best), best),
        }
    }

    fn binding_value(&self, binding: &Binding, set: Set, dead_zone: f32) -> (bool, [f32; 2]) {
        match *binding {
            Binding::Press(source) => {
                let level = self.level(source, set);
                (level.tap(), [axial(level.value, dead_zone), 0.0])
            }
            Binding::Axis { negative, positive } => {
                let negative = self.level(negative, set);
                let positive = self.level(positive, set);
                (
                    negative.tap() || positive.tap(),
                    [axial(positive.value - negative.value, dead_zone), 0.0],
                )
            }
            Binding::PadAxis(axis) => {
                if set != Set::Gamepad {
                    return (false, [0.0; 2]);
                }
                let (x, y) = axis.stick().axes();
                let v = self.stick_axes(x, y);
                let value = if axis == x { v[0] } else { v[1] };
                let vector = match axis {
                    Axis::LeftX | Axis::RightX => [axial(value, dead_zone), 0.0],
                    Axis::LeftY | Axis::RightY => [0.0, axial(value, dead_zone)],
                };
                (false, vector)
            }
            Binding::Stick {
                up,
                down,
                left,
                right,
            } => {
                let up = self.level(up, set);
                let down = self.level(down, set);
                let left = self.level(left, set);
                let right = self.level(right, set);
                let vector = radial([right.value - left.value, up.value - down.value], dead_zone);
                (up.tap() || down.tap() || left.tap() || right.tap(), vector)
            }
            Binding::PadStick(stick) => {
                if set != Set::Gamepad {
                    return (false, [0.0; 2]);
                }
                let (x, y) = stick.axes();
                (false, radial(self.stick_axes(x, y), dead_zone))
            }
        }
    }

    pub fn prompt(&self, name: &str) -> Vec<Control> {
        let set = self.active.set();
        if set == Set::Gamepad
            && let Some(pad) = self.active.pad.and_then(|pad| self.pads.get(&pad))
            && pad.info.resolves_actions
        {
            return pad.origins.get(name).cloned().unwrap_or_default();
        }
        let bindings = self.map.bindings(set, name);
        let chosen = match self.active.kind {
            DeviceKind::Mouse => bindings
                .iter()
                .find(|binding| binding.sources().iter().any(|source| source.is_mouse()))
                .or_else(|| bindings.first()),
            DeviceKind::Keyboard => bindings
                .iter()
                .find(|binding| binding.sources().iter().all(|source| source.is_keyboard()))
                .or_else(|| bindings.first()),
            DeviceKind::Gamepad => bindings.first(),
        };
        chosen.map(Binding::controls).unwrap_or_default()
    }

    pub fn prompt_family(&self) -> Family {
        self.active
            .family
            .or_else(|| self.last_family())
            .unwrap_or(Family::Generic)
    }

    pub fn glyph_style(&self) -> GlyphStyle {
        self.prompt_family().style()
    }

    pub fn rumble_intensity(&self) -> u8 {
        self.rumbler.intensity()
    }

    pub fn set_rumble_intensity(&mut self, percent: u8) {
        self.rumbler.set_intensity(percent);
    }

    pub fn rumble(&mut self, rumble: Rumble) -> bool {
        match (self.active.kind, self.active.pad) {
            (DeviceKind::Gamepad, Some(pad)) => self.rumble_pad(pad, rumble),
            _ => false,
        }
    }

    pub fn rumble_pad(&mut self, pad: PadId, rumble: Rumble) -> bool {
        let Some(motors) = self.pads.get(&pad).map(|pad| pad.info.motors) else {
            return false;
        };
        self.rumbler.play(pad, motors, rumble)
    }

    pub fn start_recording(&mut self) {
        let mut prelude = Vec::new();
        for (pad, state) in &self.pads {
            prelude.push(InputEvent::PadConnected {
                pad: *pad,
                info: state.info.clone(),
            });
            for (button, level) in &state.buttons {
                if level.value != 0.0 {
                    prelude.push(InputEvent::PadButton {
                        pad: *pad,
                        button: *button,
                        value: level.value,
                    });
                }
            }
            for (axis, value) in &state.axes {
                if *value != 0.0 {
                    prelude.push(InputEvent::PadAxis {
                        pad: *pad,
                        axis: *axis,
                        value: *value,
                    });
                }
            }
            for (action, value) in &state.actions {
                prelude.push(InputEvent::PadAction {
                    pad: *pad,
                    action: action.clone(),
                    value: *value,
                });
            }
            for (action, controls) in &state.origins {
                prelude.push(InputEvent::PadOrigins {
                    pad: *pad,
                    action: action.clone(),
                    controls: controls.clone(),
                });
            }
        }
        for (source, level) in &self.keys {
            if level.value < PRESS {
                continue;
            }
            match source {
                Source::Key(key) => prelude.push(InputEvent::Key {
                    key: *key,
                    pressed: true,
                }),
                Source::Mouse(button) => prelude.push(InputEvent::MouseButton {
                    button: *button,
                    pressed: true,
                }),
                Source::Wheel(_) | Source::Button(_) | Source::Axis(..) => {}
            }
        }
        if let Some(layout) = self.pointer.layout {
            prelude.push(InputEvent::PointerAt {
                layout,
                inside: self.pointer.inside,
            });
        }
        self.pending.clear();
        self.recording = Some(Recording::new(
            self.step_us,
            Start {
                events: prelude,
                states: self.states.clone(),
                active: self.active,
                last_pad: self.last_pad,
                anchor: self.anchor,
                outside: self.outside.iter().copied().collect(),
            },
        ));
    }

    pub fn begin_replay(&mut self, recording: &Recording) {
        let start = &recording.start;
        for event in &start.events {
            self.apply(event);
        }
        if start.states.len() == self.states.len() {
            self.states.clone_from(&start.states);
        }
        self.active = start.active;
        self.reported = start.active;
        self.last_pad = start.last_pad;
        self.reported_style = self.glyph_style();
        self.anchor = start.anchor;
        self.outside = start.outside.iter().copied().collect();
        for level in self.keys.values_mut() {
            level.tapped = false;
        }
        for pad in self.pads.values_mut() {
            for level in pad.buttons.values_mut() {
                level.tapped = false;
            }
        }
    }

    pub fn stop_recording(&mut self) -> Option<Recording> {
        self.pending.clear();
        self.recording.take()
    }

    pub fn recording(&self) -> Option<&Recording> {
        self.recording.as_ref()
    }

    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }
}

fn finite_unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn finite_signed(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

fn magnitude(v: [f32; 2]) -> f32 {
    v[0].hypot(v[1])
}

pub fn axial(value: f32, dead_zone: f32) -> f32 {
    let value = finite_signed(value);
    let dead_zone = dead_zone.clamp(0.0, 0.95);
    let size = value.abs();
    if size <= dead_zone {
        return 0.0;
    }
    value.signum() * ((size - dead_zone) / (1.0 - dead_zone)).min(1.0)
}

pub fn radial(v: [f32; 2], dead_zone: f32) -> [f32; 2] {
    let v = [finite_signed(v[0]), finite_signed(v[1])];
    let dead_zone = dead_zone.clamp(0.0, 0.95);
    let size = magnitude(v);
    if size <= dead_zone {
        return [0.0; 2];
    }
    let scaled = ((size - dead_zone) / (1.0 - dead_zone)).min(1.0);
    [v[0] / size * scaled, v[1] / size * scaled]
}
