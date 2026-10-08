use pfx_input::{Control, Glyph, GlyphStyle};
use pfx_live::flat::effects::Effects;
use pfx_live::flat::look::{Blend, Look, LookFrame, TokenSet};
use pfx_live::flat::{Draw, Group, Light, Matrix, ShadowCurve, Srgba};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiFrame {
    pub layout: [f32; 2],
    pub safe: [f32; 4],
    pub ui_scale: f32,
    pub pixels_per_unit: f32,
    pub window: [u32; 2],
    pub window_units: [f32; 2],
    pub content: [f32; 4],
    pub backdrop: bool,
    pub deck: bool,
    pub glyph_style: GlyphStyle,
    pub alpha: f32,
    pub seconds: f32,
}

impl Default for UiFrame {
    fn default() -> Self {
        Self {
            layout: [1920.0, 1080.0],
            safe: [0.0, 0.0, 1920.0, 1080.0],
            ui_scale: 1.0,
            pixels_per_unit: 1.0,
            window: [1920, 1080],
            window_units: [1920.0, 1080.0],
            content: [0.0, 0.0, 1920.0, 1080.0],
            backdrop: false,
            deck: false,
            glyph_style: GlyphStyle::Standard,
            alpha: 0.0,
            seconds: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Anchor {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub text: String,
    pub family: String,
    pub size: f32,
    pub weight: u16,
    pub at: [f32; 2],
    pub anchor: Anchor,
    pub colour: Srgba,
    pub elevation: f32,
    pub id: u32,
    pub line: Option<f32>,
    pub wrap: Option<f32>,
    pub height: Option<f32>,
    pub vertical: Anchor,
    pub transform: Option<Matrix>,
    pub fallbacks: Vec<String>,
}

impl Label {
    pub fn new(
        text: impl Into<String>,
        family: impl Into<String>,
        size: f32,
        at: [f32; 2],
    ) -> Self {
        Self {
            text: text.into(),
            family: family.into(),
            size,
            weight: 400,
            at,
            anchor: Anchor::Start,
            colour: Srgba([1.0, 1.0, 1.0, 1.0]),
            elevation: 0.0,
            id: 0,
            line: None,
            wrap: None,
            height: None,
            vertical: Anchor::Start,
            transform: None,
            fallbacks: Vec::new(),
        }
    }

    pub fn line(mut self, height: f32) -> Self {
        self.line = Some(height);
        self
    }

    pub fn line_height(&self) -> f32 {
        self.line.unwrap_or(self.size * 1.25)
    }

    pub fn wrap(mut self, width: f32) -> Self {
        self.wrap = Some(width);
        self
    }

    pub fn boxed(mut self, size: [f32; 2], horizontal: Anchor, vertical: Anchor) -> Self {
        self.wrap = Some(size[0]);
        self.height = Some(size[1]);
        self.anchor = horizontal;
        self.vertical = vertical;
        self
    }

    pub fn transform(mut self, transform: Matrix) -> Self {
        self.transform = Some(transform);
        self
    }

    pub fn fallbacks<S: Into<String>>(mut self, families: impl IntoIterator<Item = S>) -> Self {
        self.fallbacks = families.into_iter().map(Into::into).collect();
        self
    }

    pub fn weight(mut self, weight: u16) -> Self {
        self.weight = weight;
        self
    }

    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn colour(mut self, colour: Srgba) -> Self {
        self.colour = colour;
        self
    }

    pub fn elevation(mut self, elevation: f32) -> Self {
        self.elevation = elevation;
        self
    }

    pub fn id(mut self, id: u32) -> Self {
        self.id = id;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PromptOf {
    Action(String),
    Control(Control),
    Glyph(Glyph),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    pub of: PromptOf,
    pub at: [f32; 2],
    pub height: f32,
    pub gap: f32,
    pub colour: Srgba,
    pub elevation: f32,
    pub family: String,
    pub weight: u16,
    pub floor: f32,
    pub id: u32,
}

impl Prompt {
    pub fn new(of: PromptOf, at: [f32; 2], height: f32) -> Self {
        Self {
            of,
            at,
            height,
            gap: height * 0.2,
            colour: Srgba([1.0, 1.0, 1.0, 1.0]),
            elevation: 0.0,
            family: String::new(),
            weight: 600,
            floor: 0.0,
            id: 0,
        }
    }

    pub fn floor(mut self, floor: f32) -> Self {
        self.floor = floor;
        self
    }

    pub fn action(name: impl Into<String>, at: [f32; 2], height: f32) -> Self {
        Self::new(PromptOf::Action(name.into()), at, height)
    }

    pub fn control(control: Control, at: [f32; 2], height: f32) -> Self {
        Self::new(PromptOf::Control(control), at, height)
    }

    pub fn glyph(glyph: Glyph, at: [f32; 2], height: f32) -> Self {
        Self::new(PromptOf::Glyph(glyph), at, height)
    }

    pub fn colour(mut self, colour: Srgba) -> Self {
        self.colour = colour;
        self
    }

    pub fn elevation(mut self, elevation: f32) -> Self {
        self.elevation = elevation;
        self
    }

    pub fn font(mut self, family: impl Into<String>, weight: u16) -> Self {
        self.family = family.into();
        self.weight = weight;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn id(mut self, id: u32) -> Self {
        self.id = id;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UiLook {
    pub from: Look,
    pub to: Look,
    pub progress: f32,
    pub blend: Blend,
    pub base: TokenSet,
}

impl UiLook {
    pub fn still(look: Look, base: TokenSet) -> Self {
        Self {
            from: look.clone(),
            to: look,
            progress: 1.0,
            blend: Blend::Crossfade,
            base,
        }
    }

    pub fn of(frame: &LookFrame<'_>) -> Self {
        Self {
            from: frame.from.clone(),
            to: frame.to.clone(),
            progress: frame.progress,
            blend: frame.blend,
            base: frame.base.clone(),
        }
    }

    pub fn frame(&self) -> LookFrame<'_> {
        LookFrame {
            from: &self.from,
            to: &self.to,
            progress: self.progress,
            blend: self.blend,
            base: &self.base,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ui {
    pub backdrop: Vec<Draw>,
    pub draws: Vec<Draw>,
    pub labels: Vec<Label>,
    pub prompts: Vec<Prompt>,
    pub groups: Vec<Group>,
    pub curve: Option<ShadowCurve>,
    pub light: Light,
    pub clear: Option<Srgba>,
    pub look: Option<UiLook>,
}

impl Ui {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.backdrop.is_empty()
            && self.draws.is_empty()
            && self.labels.is_empty()
            && self.prompts.is_empty()
    }

    pub fn backdrop(&mut self, draw: Draw) -> &mut Self {
        self.backdrop.push(draw);
        self
    }

    pub fn draw(&mut self, draw: Draw) -> &mut Self {
        self.draws.push(draw);
        self
    }

    pub fn label(&mut self, label: Label) -> &mut Self {
        self.labels.push(label);
        self
    }

    pub fn prompt(&mut self, prompt: Prompt) -> &mut Self {
        self.prompts.push(prompt);
        self
    }

    pub fn clear(&mut self, colour: Srgba) -> &mut Self {
        self.clear = Some(colour);
        self
    }

    pub fn look(&mut self, frame: &LookFrame<'_>) -> &mut Self {
        self.look = Some(UiLook::of(frame));
        self
    }

    pub fn effects(&mut self, effects: &mut Effects) -> &mut Self {
        let (draws, light) = effects.compose(&self.draws, self.light);
        self.draws = draws.to_vec();
        self.light = light;
        self
    }

    pub fn group(&mut self, opacity: f32) -> u16 {
        self.groups.push(Group { opacity });
        (self.groups.len() - 1) as u16
    }
}
