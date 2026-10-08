use std::rc::Rc;

use pfx_core::clock::Tick;
use pfx_gpu::screens::{Device, Display, ScreenPolicy, ScreenReport};
use pfx_gpu::window::{Attention, Clock, FramePacer, Host, Pace, Point, PresentPreference, Size};
use pfx_input::{Backend, BindingMap, InputEvent, KeyLabels, LayoutSource};
use pfx_play::{Exit, Notice, PlaySession, Settings, Ui, UiFrame, Warm, Warmer, Warming};

use crate::prompt::Part;

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Input(InputEvent),
    Attention(Attention),
    Resized(Size),
    Refresh(Option<u32>),
    Monitor(Option<String>),
    Moved {
        monitor: Option<String>,
        position: Point,
    },
}

pub fn present_preference(vrr: bool, host: Host) -> PresentPreference {
    if vrr {
        PresentPreference::vrr(host)
    } else {
        PresentPreference::game()
    }
}

pub struct Driver {
    session: PlaySession,
    pacer: FramePacer,
    applied: Settings,
    device: Device,
    display: Display,
    display_changed: bool,
    present_changed: bool,
    policy: ScreenPolicy,
    window: Size,
    report: Option<ScreenReport>,
    last_ns: Option<u64>,
    refresh: Option<u32>,
    pads: Option<Box<dyn Backend>>,
    scratch: Vec<InputEvent>,
    labels: KeyLabels,
    layout: Option<Box<dyn LayoutSource>>,
    ui: Rc<Ui>,
    parts: Vec<Vec<Part>>,
    motors: u64,
    frames: u64,
    seconds: f32,
    notices: Vec<Notice>,
}

impl Driver {
    pub fn new(
        session: PlaySession,
        settings: Settings,
        policy: ScreenPolicy,
        device: Device,
        window: Size,
        pads: Option<Box<dyn Backend>>,
    ) -> Self {
        let mut driver = Self {
            session,
            pacer: FramePacer::new(settings.pacing.caps()),
            display: Display::new(device, settings.display.clone()),
            applied: settings.clone(),
            device,
            display_changed: false,
            present_changed: false,
            policy,
            window,
            report: None,
            last_ns: None,
            refresh: None,
            pads,
            scratch: Vec::new(),
            labels: KeyLabels::new(),
            layout: None,
            ui: Rc::default(),
            parts: Vec::new(),
            motors: 0,
            frames: 0,
            seconds: 0.0,
            notices: Vec::new(),
        };
        driver.apply(&settings, true);
        driver.refit();
        driver.session.set_display(driver.display.settings());
        driver
    }

    pub fn feed(&mut self, event: Event) {
        match event {
            Event::Input(event) => self.session.feed(event),
            Event::Attention(attention) => self.pacer.on(attention),
            Event::Resized(size) => {
                self.window = size;
                self.display.resized(size);
                self.refit();
            }
            Event::Refresh(refresh) => {
                self.refresh = refresh;
                self.pace();
            }
            Event::Monitor(name) => self.display.moved_to(name),
            Event::Moved { monitor, position } => self.display.placed(monitor, position),
        }
    }

    pub fn poll(&mut self, clock: &impl Clock) -> Pace {
        let pace = self.pacer.poll(clock);
        if pace == Pace::Paused {
            self.last_ns = None;
        }
        pace
    }

    pub fn start_clock(&mut self, now_ns: u64) {
        self.last_ns = Some(now_ns);
    }

    pub fn update(&mut self, now_ns: u64) -> Tick {
        if let Some(pads) = &mut self.pads {
            pads.poll(&mut self.scratch);
            for event in self.scratch.drain(..) {
                self.session.feed(event);
            }
        }
        let seconds = self
            .last_ns
            .map_or(0.0, |last| now_ns.saturating_sub(last) as f64 / 1e9);
        self.last_ns = Some(now_ns);
        self.seconds = seconds as f32;
        self.session.set_display(self.display.settings());
        let tick = self.session.advance(self.seconds);
        self.frames += 1;
        self.settle(tick.alpha);
        tick
    }

    pub(crate) fn warm(
        &mut self,
        warmer: &mut dyn Warmer,
        pass: u32,
        elapsed: f64,
        budget: f64,
    ) -> Result<Warming, String> {
        let mut warm = Warm::new(warmer, pass, elapsed, budget);
        self.session
            .game_mut()
            .warm(&mut warm)
            .map_err(|error| error.to_string())
    }

    pub fn step(&mut self) -> bool {
        if !self.session.step() {
            return false;
        }
        self.seconds = 0.0;
        self.settle(0.0);
        true
    }

    fn settle(&mut self, alpha: f32) {
        if self.session.exit().is_some() {
            return;
        }
        for command in self.session.take_motors() {
            if let Some(pads) = &mut self.pads {
                pads.set_motors(&command);
                self.motors += 1;
            }
        }
        if let Some(layout) = &mut self.layout {
            self.labels.refresh(layout.as_mut());
        }
        if let Some(modules) = self.session.game_mut().modules() {
            self.notices.extend(modules.notices());
        }
        let changed = self
            .session
            .game()
            .settings()
            .filter(|settings| **settings != self.applied)
            .cloned();
        if let Some(settings) = changed {
            self.apply(&settings, false);
        }
        let frame = self.ui_frame(alpha);
        self.ui = Rc::new(self.session.ui(&frame));
        self.parts = self.resolve();
    }

    fn resolve(&self) -> Vec<Vec<Part>> {
        let input = self.session.input();
        self.ui
            .prompts
            .iter()
            .map(|prompt| crate::prompt::parts(input, &prompt.of))
            .collect()
    }

    fn apply(&mut self, settings: &Settings, first: bool) {
        if first || settings.pacing != self.applied.pacing {
            self.pacer.set_caps(settings.pacing.caps());
        }
        if first || settings.rumble != self.applied.rumble {
            self.session.set_rumble_intensity(settings.rumble.min(100));
        }
        if first || settings.audio != self.applied.audio {
            settings
                .audio
                .apply(self.session.world_mut().sounds.mixer());
        }
        if settings.binds != self.applied.binds || (first && settings.binds.is_some()) {
            let map = settings
                .binds
                .clone()
                .unwrap_or_else(|| BindingMap::defaults(self.session.input().actions()));
            self.session.set_bindings(map);
        }
        if !first
            && settings.display != self.applied.display
            && settings.display != *self.display.settings()
        {
            self.display = Display::new(self.device, settings.display.clone());
            self.display_changed = true;
        }
        if !first && settings.pacing.vrr != self.applied.pacing.vrr {
            self.present_changed = true;
        }
        self.applied = settings.clone();
        self.pace();
    }

    fn pace(&mut self) {
        let vrr = self.applied.pacing.vrr.then_some(self.refresh).flatten();
        self.pacer.set_vrr(vrr);
    }

    fn refit(&mut self) {
        self.report = self.policy.fit(self.device, self.window);
        self.session
            .set_viewport(self.report.as_ref().and_then(ScreenReport::viewport));
    }

    pub fn ui_frame(&self, alpha: f32) -> UiFrame {
        let window = [self.window.width, self.window.height];
        let glyph_style = self.session.input().glyph_style();
        let deck = self.device.is_deck();
        match &self.report {
            Some(report) => {
                let safe = report.safe_layout;
                let content = report.content_units();
                UiFrame {
                    layout: report.layout_units(),
                    safe: [safe.x, safe.y, safe.width, safe.height],
                    ui_scale: report.ui_scale,
                    pixels_per_unit: report.pixels_per_unit(),
                    window,
                    window_units: report.window_units(),
                    content: [content.x, content.y, content.width, content.height],
                    backdrop: report.backdrop,
                    deck,
                    glyph_style,
                    alpha,
                    seconds: self.seconds,
                }
            }
            None => UiFrame {
                window,
                deck,
                glyph_style,
                alpha,
                seconds: self.seconds,
                ..UiFrame::default()
            },
        }
    }

    pub fn take_display(&mut self) -> Option<&Display> {
        if !self.display_changed {
            return None;
        }
        self.display_changed = false;
        Some(&self.display)
    }

    pub fn take_present(&mut self) -> Option<bool> {
        if !self.present_changed {
            return None;
        }
        self.present_changed = false;
        Some(self.applied.pacing.vrr)
    }

    pub fn exit(&self) -> Option<&Exit> {
        self.session.exit()
    }

    pub fn finish(self) -> Result<u8, String> {
        let exit = self.session.exit().cloned();
        self.session.stop();
        match exit {
            None => Ok(0),
            Some(Exit::Quit(code)) => Ok(code),
            Some(Exit::Failed(error)) => Err(error),
        }
    }

    pub fn seconds(&self) -> f32 {
        self.seconds
    }

    pub fn display(&self) -> &Display {
        &self.display
    }

    pub fn device(&self) -> Device {
        self.device
    }

    pub fn session(&self) -> &PlaySession {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut PlaySession {
        &mut self.session
    }

    pub fn into_session(self) -> PlaySession {
        self.session
    }

    pub fn settings(&self) -> &Settings {
        &self.applied
    }

    pub fn report(&self) -> Option<&ScreenReport> {
        self.report.as_ref()
    }

    pub fn window(&self) -> Size {
        self.window
    }

    pub fn ui(&self) -> &Ui {
        &self.ui
    }

    pub(crate) fn shared_ui(&self) -> Rc<Ui> {
        Rc::clone(&self.ui)
    }

    pub fn prompt_parts(&self) -> &[Vec<Part>] {
        &self.parts
    }

    pub fn labels(&self) -> &KeyLabels {
        &self.labels
    }

    pub fn labels_mut(&mut self) -> &mut KeyLabels {
        &mut self.labels
    }

    pub fn set_layout(&mut self, layout: Option<Box<dyn LayoutSource>>) {
        self.layout = layout;
    }

    pub fn motors_sent(&self) -> u64 {
        self.motors
    }

    pub fn effective_fps(&self) -> Option<u32> {
        self.pacer.effective_fps()
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn ticks(&self) -> u64 {
        self.session.world().ticks()
    }

    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }

    pub fn take_audio(&mut self) -> Vec<f32> {
        self.session.take_audio()
    }
}
