#[cfg(test)]
mod tests;

use super::{
    Draw, Fill, Fit, FlatPass, FlatScene, FlatText, Shadow, ShadowCurve, Shape, Srgba, TokenKey,
};
use pfx_gpu::{GpuProfiler, wgpu};
pub use pfx_post::look::{
    Aberration, Backdrop, Blend, Crt, Curvature, Dither, Glow, Gradient, GradientShape, Grid,
    Interpolation, Major, Palette, Place, Quantise, Scanlines, Upscale, UpscaleMap, Vignette,
};
use pfx_post::look::{LookGpu, gpu::Surface, mix_oklab};
use std::collections::BTreeMap;

pub const LOW_RES_MAX: u32 = 4096;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TokenSet {
    tokens: BTreeMap<String, Srgba>,
}

impl TokenSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, key: impl Into<String>, colour: Srgba) -> Self {
        self.insert(key, colour);
        self
    }

    pub fn insert(&mut self, key: impl Into<String>, colour: Srgba) {
        self.tokens.insert(key.into(), colour);
    }

    pub fn get(&self, key: &str) -> Option<Srgba> {
        self.tokens.get(key).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, Srgba)> {
        self.tokens
            .iter()
            .map(|(key, colour)| (key.as_str(), *colour))
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    pub fn mix(&self, other: &TokenSet, t: f32) -> TokenSet {
        let t = t.clamp(0.0, 1.0);
        let mut out = self.clone();
        for (key, colour) in &other.tokens {
            let mixed = match self.tokens.get(key) {
                Some(start) if t <= 0.0 => *start,
                Some(_) if t >= 1.0 => *colour,
                Some(start) => Srgba(mix_oklab(start.0, colour.0, t)),
                None => *colour,
            };
            out.tokens.insert(key.clone(), mixed);
        }
        out
    }
}

impl<K: Into<String>> FromIterator<(K, Srgba)> for TokenSet {
    fn from_iter<I: IntoIterator<Item = (K, Srgba)>>(iter: I) -> Self {
        let mut set = Self::new();
        for (key, colour) in iter {
            set.insert(key, colour);
        }
        set
    }
}

fn byte_key(colour: Srgba) -> [u8; 3] {
    std::array::from_fn(|i| (colour.0[i].clamp(0.0, 1.0) * 255.0).round() as u8)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recolour {
    map: Vec<([u8; 3], Srgba, f32)>,
    keyed: BTreeMap<TokenKey, (Srgba, f32)>,
}

fn swapped(colour: Srgba, to: Srgba, base_alpha: f32) -> Srgba {
    let alpha = if base_alpha > 0.0 {
        colour.0[3] * to.0[3] / base_alpha
    } else {
        to.0[3]
    };
    Srgba([to.0[0], to.0[1], to.0[2], alpha.clamp(0.0, 1.0)])
}

impl Recolour {
    pub fn new(base: &TokenSet, swap: &TokenSet) -> Self {
        let mut map: Vec<([u8; 3], Srgba, f32)> = Vec::new();
        for (key, from) in base.iter() {
            let Some(to) = swap.get(key) else {
                continue;
            };
            let at = byte_key(from);
            if map.iter().any(|(existing, _, _)| *existing == at) || from == to {
                continue;
            }
            map.push((at, to, from.0[3]));
        }
        map.sort_by_key(|entry| entry.0);
        let mut keyed = BTreeMap::new();
        for (key, to) in swap.iter() {
            let from = base.get(key);
            if from == Some(to) {
                continue;
            }
            keyed.insert(TokenKey::new(key), (to, from.map_or(1.0, |from| from.0[3])));
        }
        Self { map, keyed }
    }

    pub fn is_identity(&self) -> bool {
        self.map.is_empty() && self.keyed.is_empty()
    }

    pub fn pick(&self, colour: Srgba, key: Option<TokenKey>) -> Srgba {
        match key {
            None => self.colour(colour),
            Some(key) => match self.keyed.get(&key) {
                Some(&(to, base_alpha)) => swapped(colour, to, base_alpha),
                None => colour,
            },
        }
    }

    pub fn colour(&self, colour: Srgba) -> Srgba {
        let at = byte_key(colour);
        match self.map.binary_search_by_key(&at, |entry| entry.0) {
            Ok(index) => {
                let (_, to, base_alpha) = self.map[index];
                swapped(colour, to, base_alpha)
            }
            Err(_) => colour,
        }
    }

    pub fn gradient(&self, gradient: &Gradient) -> Gradient {
        let stops: Vec<(f32, [f32; 4])> = gradient
            .stops()
            .map(|(offset, colour)| (offset, self.colour(Srgba(colour)).0))
            .collect();
        Gradient::new(gradient.shape, &stops)
            .map(|recoloured| recoloured.space(gradient.space))
            .unwrap_or(*gradient)
    }

    pub fn draw(&self, draw: &mut Draw) {
        let roles = draw.roles;
        draw.fill = match draw.fill {
            Fill::None => Fill::None,
            Fill::Solid(colour) => Fill::Solid(self.pick(colour, roles.fill)),
            Fill::Vertical(top, bottom) => Fill::Vertical(
                self.pick(top, roles.fill),
                self.pick(bottom, roles.fill_end),
            ),
            Fill::Gradient(gradient) => Fill::Gradient(self.gradient(&gradient)),
            Fill::Image(image) => Fill::Image(image),
        };
        if let Some(stroke) = draw.stroke.as_mut() {
            stroke.colour = self.pick(stroke.colour, roles.stroke);
        }
        if let Some(pattern) = draw.pattern.as_mut() {
            pattern.colour = self.pick(pattern.colour, roles.pattern);
        }
        if let Shadow::Glow(glow) = &mut draw.shadow {
            glow.colour = self.pick(glow.colour, roles.glow);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LowText {
    #[default]
    Pixelated,
    Crisp,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LowRes {
    pub size: [u32; 2],
    pub upscale: Upscale,
    pub text: LowText,
}

impl LowRes {
    pub fn new(size: [u32; 2]) -> Self {
        Self {
            size,
            upscale: Upscale::Integer,
            text: LowText::Pixelated,
        }
    }

    pub fn upscale(mut self, upscale: Upscale) -> Self {
        self.upscale = upscale;
        self
    }

    pub fn text(mut self, text: LowText) -> Self {
        self.text = text;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowTint {
    pub colour: Option<Srgba>,
    pub alpha: f32,
}

impl Default for ShadowTint {
    fn default() -> Self {
        Self {
            colour: None,
            alpha: 1.0,
        }
    }
}

impl ShadowTint {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn colour(mut self, colour: Srgba) -> Self {
        self.colour = Some(colour);
        self
    }

    pub fn alpha(mut self, alpha: f32) -> Self {
        self.alpha = alpha;
        self
    }

    pub fn apply(&self, curve: Srgba) -> Srgba {
        let colour = self.colour.unwrap_or(curve);
        colour.alpha((colour.0[3] * self.alpha).clamp(0.0, 1.0))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LookPass {
    LowRes(LowRes),
    Tokens(TokenSet),
    Shadow(ShadowTint),
    Backdrop(Backdrop),
    Quantise(Quantise),
    Vignette(Vignette),
    Crt(Crt),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub passes: Vec<LookPass>,
}

impl Look {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, pass: LookPass) -> Self {
        self.passes.push(pass);
        self
    }

    pub fn is_identity(&self) -> bool {
        self.passes.is_empty()
    }

    pub fn low_res(&self) -> Option<&LowRes> {
        self.passes.iter().find_map(|pass| match pass {
            LookPass::LowRes(low) => Some(low),
            _ => None,
        })
    }

    pub fn tokens(&self) -> Option<&TokenSet> {
        self.passes.iter().find_map(|pass| match pass {
            LookPass::Tokens(tokens) => Some(tokens),
            _ => None,
        })
    }

    pub fn shadow(&self) -> Option<&ShadowTint> {
        self.passes.iter().find_map(|pass| match pass {
            LookPass::Shadow(tint) => Some(tint),
            _ => None,
        })
    }

    pub fn backdrop(&self) -> Option<&Backdrop> {
        self.passes.iter().find_map(|pass| match pass {
            LookPass::Backdrop(backdrop) => Some(backdrop),
            _ => None,
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        let count = |test: fn(&LookPass) -> bool| self.passes.iter().filter(|p| test(p)).count();
        if count(|p| matches!(p, LookPass::LowRes(_))) > 1
            || count(|p| matches!(p, LookPass::Tokens(_))) > 1
            || count(|p| matches!(p, LookPass::Backdrop(_))) > 1
            || count(|p| matches!(p, LookPass::Shadow(_))) > 1
        {
            return Err(
                "a look holds at most one low resolution, token set, shadow tint and backdrop"
                    .into(),
            );
        }
        if let Some(tint) = self.shadow()
            && !(tint.alpha.is_finite() && tint.alpha >= 0.0)
        {
            return Err("a shadow tint's alpha scale is finite and zero or more".into());
        }
        if let Some(low) = self.low_res()
            && (low.size.contains(&0) || low.size.iter().any(|&side| side > LOW_RES_MAX))
        {
            return Err(format!(
                "a low resolution is 1 to {LOW_RES_MAX} pixels a side"
            ));
        }
        Ok(())
    }

    fn same_pixels(&self, other: &Look) -> bool {
        let pixels = |look: &Look| {
            look.passes
                .iter()
                .filter(|pass| !matches!(pass, LookPass::Tokens(_) | LookPass::Shadow(_)))
                .cloned()
                .collect::<Vec<_>>()
        };
        pixels(self) == pixels(other)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Easing {
    #[default]
    Linear,
    In,
    Out,
    InOut,
    Steps(u32),
}

impl Easing {
    pub fn at(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::In => t * t * t,
            Easing::Out => 1.0 - (1.0 - t).powi(3),
            Easing::InOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) * 0.5
                }
            }
            Easing::Steps(steps) => {
                let steps = steps.max(1) as f32;
                (t * steps).floor().min(steps) / steps
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    pub duration: f32,
    pub easing: Easing,
    pub blend: Blend,
}

impl Transition {
    pub const INSTANT: Self = Self {
        duration: 0.0,
        easing: Easing::Linear,
        blend: Blend::Crossfade,
    };

    pub fn crossfade(duration: f32, easing: Easing) -> Self {
        Self {
            duration,
            easing,
            blend: Blend::Crossfade,
        }
    }

    pub fn wipe(duration: f32, easing: Easing, angle: f32, softness: f32) -> Self {
        Self {
            duration,
            easing,
            blend: Blend::Wipe { angle, softness },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Motion {
    #[default]
    Full,
    Reduced {
        longest: f32,
    },
}

impl Motion {
    pub fn apply(self, transition: Transition) -> Transition {
        match self {
            Motion::Full => transition,
            Motion::Reduced { longest } => Transition {
                duration: transition.duration.min(longest.max(0.0)),
                easing: Easing::Linear,
                blend: Blend::Crossfade,
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct LookStack {
    base: TokenSet,
    current: Look,
    previous: Option<Look>,
    transition: Transition,
    start: f64,
    pub motion: Motion,
}

#[derive(Clone, Copy, Debug)]
pub struct LookFrame<'a> {
    pub from: &'a Look,
    pub to: &'a Look,
    pub progress: f32,
    pub blend: Blend,
    pub base: &'a TokenSet,
}

impl<'a> LookFrame<'a> {
    pub fn still(look: &'a Look, base: &'a TokenSet) -> Self {
        Self {
            from: look,
            to: look,
            progress: 1.0,
            blend: Blend::Crossfade,
            base,
        }
    }

    pub fn settled(&self) -> bool {
        self.progress >= 1.0 || std::ptr::eq(self.from, self.to) || self.from == self.to
    }

    pub fn is_identity(&self) -> bool {
        self.to.is_identity() && (self.settled() || self.from.is_identity())
    }
}

impl LookStack {
    pub fn new(look: Look) -> Result<Self, String> {
        look.validate()?;
        Ok(Self {
            base: TokenSet::new(),
            current: look,
            previous: None,
            transition: Transition::INSTANT,
            start: 0.0,
            motion: Motion::Full,
        })
    }

    pub fn base(mut self, tokens: TokenSet) -> Self {
        self.base = tokens;
        self
    }

    pub fn motion(mut self, motion: Motion) -> Self {
        self.motion = motion;
        self
    }

    pub fn current(&self) -> &Look {
        &self.current
    }

    pub fn tokens(&self) -> &TokenSet {
        &self.base
    }

    pub fn progress(&self, time: f64) -> f32 {
        let transition = self.motion.apply(self.transition);
        if self.previous.is_none() || transition.duration <= 0.0 {
            return 1.0;
        }
        let t = ((time - self.start) / f64::from(transition.duration)).clamp(0.0, 1.0) as f32;
        transition.easing.at(t)
    }

    pub fn set(&mut self, look: Look) -> Result<(), String> {
        look.validate()?;
        self.current = look;
        self.previous = None;
        Ok(())
    }

    pub fn switch(&mut self, look: Look, time: f64, transition: Transition) -> Result<(), String> {
        look.validate()?;
        if !(transition.duration.is_finite() && transition.duration >= 0.0) || !time.is_finite() {
            return Err("a transition's time and duration must be finite and nonnegative".into());
        }
        let progress = self.progress(time);
        let shown = match self.previous.take() {
            Some(previous) if progress < 0.5 => previous,
            _ => std::mem::take(&mut self.current),
        };
        self.previous = Some(shown);
        self.current = look;
        self.transition = transition;
        self.start = time;
        Ok(())
    }

    pub fn frame(&self, time: f64) -> LookFrame<'_> {
        let progress = self.progress(time);
        match &self.previous {
            Some(previous) if progress < 1.0 => LookFrame {
                from: previous,
                to: &self.current,
                progress,
                blend: self.motion.apply(self.transition).blend,
                base: &self.base,
            },
            _ => LookFrame::still(&self.current, &self.base),
        }
    }
}

pub fn text_keys(draws: &[Draw], count: usize) -> Vec<Option<TokenKey>> {
    let mut keys = vec![None; count];
    for draw in draws {
        if let Shape::Text(index) = draw.shape
            && let Some(slot) = keys.get_mut(index)
            && slot.is_none()
        {
            *slot = draw.roles.fill;
        }
    }
    keys
}

pub struct SceneCopy<'a> {
    pub draws: Vec<Draw>,
    pub text: Vec<FlatText<'a>>,
    pub curve: ShadowCurve,
    pub clear: Option<Srgba>,
}

impl<'a> SceneCopy<'a> {
    pub fn recoloured(scene: &FlatScene<'a>, recolour: &Recolour) -> Self {
        let mut draws = scene.draws.to_vec();
        let mut text = scene.text.to_vec();
        let mut curve = scene.curve.clone();
        let mut clear = scene.clear;
        if !recolour.is_identity() {
            let keys = text_keys(&draws, text.len());
            for draw in &mut draws {
                recolour.draw(draw);
            }
            for (item, key) in text.iter_mut().zip(keys) {
                item.colour = recolour.pick(item.colour, key);
            }
            curve.colour = recolour.colour(curve.colour);
            clear = clear.map(|colour| recolour.colour(colour));
        }
        Self {
            draws,
            text,
            curve,
            clear,
        }
    }

    pub fn scene<'b>(&'b self, base: &FlatScene<'a>) -> FlatScene<'b>
    where
        'a: 'b,
    {
        FlatScene {
            layout: base.layout,
            clear: self.clear,
            curve: &self.curve,
            light: base.light,
            draws: &self.draws,
            groups: base.groups,
            text: &self.text,
            icons: base.icons,
            sprites: None,
            environment: base.environment,
            post: base.post,
            frame: base.frame,
            seed: base.seed,
        }
    }

    fn keep(&mut self, text: bool) {
        for draw in &mut self.draws {
            if matches!(draw.shape, Shape::Text(_)) != text {
                draw.opacity = 0.0;
            }
        }
    }

    fn place_text(&mut self, layout_axes: [f32; 2], shift: [f32; 2]) {
        let affine = super::multiply(
            super::translation(shift[0], shift[1]),
            super::scaling(layout_axes[0], layout_axes[1]),
        );
        for draw in &mut self.draws {
            if matches!(draw.shape, Shape::Text(_)) {
                draw.transform = super::multiply(affine, draw.transform);
            }
        }
    }
}

fn place_of(fit: &Fit) -> Place {
    Place {
        layout: fit.layout,
        scale: fit.scale,
        offset: fit.offset,
    }
}

fn side_tokens(frame: &LookFrame<'_>, look: &Look, mixed: bool) -> TokenSet {
    let own = |look: &Look| {
        let mut tokens = frame.base.clone();
        for (key, colour) in look.tokens().into_iter().flat_map(TokenSet::iter) {
            tokens.insert(key, colour);
        }
        tokens
    };
    if mixed {
        own(frame.from).mix(&own(frame.to), frame.progress)
    } else {
        own(look)
    }
}

fn side_shadow(frame: &LookFrame<'_>, look: &Look, mixed: bool, curve: Srgba) -> Srgba {
    let own = |look: &Look| look.shadow().map_or(curve, |tint| tint.apply(curve));
    if !mixed {
        return own(look);
    }
    let (from, to) = (own(frame.from), own(frame.to));
    if from == to {
        return to;
    }
    let t = frame.progress.clamp(0.0, 1.0);
    if t <= 0.0 {
        from
    } else if t >= 1.0 {
        to
    } else {
        Srgba(mix_oklab(from.0, to.0, t))
    }
}

fn warm_slots<'a>(
    looks: impl IntoIterator<Item = &'a Look>,
    size: [u32; 2],
) -> Result<Vec<(usize, [u32; 2])>, String> {
    let mut slots: Vec<(usize, [u32; 2])> = Vec::new();
    let mut want = |slot: usize, render: [u32; 2]| {
        if !slots.contains(&(slot, render)) {
            slots.push((slot, render));
        }
    };
    for look in looks {
        look.validate()?;
        if look.is_identity() {
            continue;
        }
        let low = look.low_res().copied();
        let render = low.map_or(size, |low| low.size);
        let crisp = low.is_some_and(|low| low.text == LowText::Crisp);
        want(0, render);
        if crisp {
            want(1, size);
        }
        if low.is_some() {
            want(2, render);
            if crisp {
                want(3, size);
            }
        }
    }
    Ok(slots)
}

pub struct FlatLooks {
    gpu: LookGpu,
    passes: Vec<Option<FlatPass>>,
    stages: u32,
}

enum Stage<'l> {
    Backdrop(&'l Backdrop),
    Quantise(&'l Quantise),
    Vignette(&'l Vignette),
    Upscale(UpscaleMap),
    Text,
    Crt(&'l Crt),
}

impl FlatLooks {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self {
            gpu: LookGpu::new(device, queue),
            passes: Vec::new(),
            stages: 0,
        }
    }

    pub fn stages(&self) -> u32 {
        self.stages
    }

    pub fn pipelines_made(&self) -> usize {
        self.gpu.pipelines()
            + self
                .passes
                .iter()
                .flatten()
                .map(FlatPass::pipelines_made)
                .sum::<usize>()
    }

    pub fn warm<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        looks: impl IntoIterator<Item = &'a Look>,
        size: [u32; 2],
    ) -> Result<(), String> {
        let slots = warm_slots(looks, size)?;
        for (slot, render) in slots {
            self.pass(device, queue, slot)
                .warm(device, render, &[], false);
        }
        Ok(())
    }

    fn pass(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, slot: usize) -> &mut FlatPass {
        if self.passes.len() <= slot {
            self.passes.resize_with(slot + 1, || None);
        }
        self.passes[slot].get_or_insert_with(|| FlatPass::new(device, queue))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        main: &mut FlatPass,
        scene: &FlatScene<'_>,
        frame: &LookFrame<'_>,
        size: [u32; 2],
        target: &wgpu::TextureView,
        mut profiler: Option<&mut GpuProfiler>,
    ) -> Result<Fit, String> {
        frame.from.validate()?;
        frame.to.validate()?;
        self.gpu.begin();
        self.stages = 0;
        let settled = frame.settled();
        let shared =
            !settled && frame.blend == Blend::Crossfade && frame.from.same_pixels(frame.to);
        let sides: Vec<&Look> = if settled || shared {
            vec![frame.to]
        } else {
            vec![frame.from, frame.to]
        };
        let mixed = !settled && frame.blend == Blend::Crossfade;
        let copies: Vec<SceneCopy<'_>> = sides
            .iter()
            .map(|look| {
                let tokens = side_tokens(frame, look, mixed);
                let mut copy = SceneCopy::recoloured(scene, &Recolour::new(frame.base, &tokens));
                if look.backdrop().is_some() {
                    copy.clear = None;
                }
                copy.curve.colour = side_shadow(frame, look, mixed, copy.curve.colour);
                copy
            })
            .collect();
        let last = sides.len() - 1;
        let fit = main.prepare(device, queue, &copies[last].scene(scene), size)?;
        let single = sides.len() == 1;
        let mut results = Vec::new();
        for (index, look) in sides.iter().enumerate() {
            let output = single.then_some(target);
            let result = self.encode_side(
                device,
                queue,
                encoder,
                main,
                scene,
                &copies[index],
                look,
                index,
                index == last,
                &fit,
                size,
                output,
                profiler.as_deref_mut(),
            )?;
            results.push(result);
        }
        if !single {
            self.gpu.mix(
                Surface {
                    view: &results[0],
                    size,
                },
                Surface {
                    view: &results[1],
                    size,
                },
                Surface { view: target, size },
                &frame.blend,
                frame.progress,
                &place_of(&fit),
            );
            self.stages += 1;
        }
        self.gpu.flush(device, queue, encoder, profiler);
        Ok(fit)
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_side(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        main: &mut FlatPass,
        scene: &FlatScene<'_>,
        copy: &SceneCopy<'_>,
        look: &Look,
        index: usize,
        main_side: bool,
        fit: &Fit,
        size: [u32; 2],
        output: Option<&wgpu::TextureView>,
        mut profiler: Option<&mut GpuProfiler>,
    ) -> Result<wgpu::TextureView, String> {
        let low = look.low_res().copied();
        let render = low.map_or(size, |low| low.size);
        let low_fit = Fit::new(scene.layout, render)?;
        let map = low
            .map(|low| UpscaleMap::new(low.size, size, low.upscale))
            .transpose()?;
        let crisp = low.is_some_and(|low| low.text == LowText::Crisp);
        let mut stages = Vec::new();
        if let Some(backdrop) = look.backdrop() {
            stages.push(Stage::Backdrop(backdrop));
        }
        let mut upscaled = map.is_none();
        for pass in &look.passes {
            match pass {
                LookPass::Quantise(quantise) => stages.push(Stage::Quantise(quantise)),
                LookPass::Vignette(vignette) => stages.push(Stage::Vignette(vignette)),
                LookPass::Crt(crt) => {
                    if !upscaled {
                        stages.push(Stage::Upscale(map.unwrap()));
                        if crisp {
                            stages.push(Stage::Text);
                        }
                        upscaled = true;
                    }
                    if !crt.is_empty() {
                        stages.push(Stage::Crt(crt));
                    }
                }
                _ => {}
            }
        }
        if !upscaled {
            stages.push(Stage::Upscale(map.unwrap()));
            if crisp {
                stages.push(Stage::Text);
            }
        }
        let clear = copy.clear.map_or([0.0; 4], |colour| colour.premultiplied());
        let base_slot = index * 4;
        let first_view = match (output, stages.is_empty()) {
            (Some(output), true) => output.clone(),
            _ => self.gpu.target(device, base_slot, render),
        };
        let reuse_main = main_side && low.is_none();
        self.gpu
            .flush(device, queue, encoder, profiler.as_deref_mut());
        let flat_scene;
        let flat: &mut FlatPass = if reuse_main {
            main
        } else {
            let mut colour = SceneCopy {
                draws: copy.draws.clone(),
                text: copy.text.clone(),
                curve: copy.curve.clone(),
                clear: copy.clear,
            };
            if crisp {
                colour.keep(false);
            }
            flat_scene = colour;
            let pass = self.pass(device, queue, index * 2);
            pass.prepare(device, queue, &flat_scene.scene(scene), render)?;
            pass
        };
        flat.encode_colour(encoder, &first_view, Some(clear), profiler.as_deref_mut());
        let mut current = first_view;
        let mut current_size = render;
        let mut place = place_of(&low_fit);
        let mut toggle = 1;
        let count = stages.len();
        for (at, stage) in stages.into_iter().enumerate() {
            let next_size = match stage {
                Stage::Upscale(map) => map.target,
                _ => current_size,
            };
            let next = match (output, at + 1 == count) {
                (Some(output), true) => output.clone(),
                _ => {
                    let slot = base_slot
                        + if next_size == render && low.is_some() {
                            toggle
                        } else {
                            2 + toggle
                        };
                    toggle ^= 1;
                    self.gpu.target(device, slot, next_size)
                }
            };
            let from = Surface {
                view: &current,
                size: current_size,
            };
            let to = Surface {
                view: &next,
                size: next_size,
            };
            match stage {
                Stage::Backdrop(backdrop) => self.gpu.backdrop(from, to, backdrop, &place),
                Stage::Quantise(quantise) => self.gpu.quantise(from, to, quantise),
                Stage::Vignette(vignette) => self.gpu.vignette(from, to, vignette, &place),
                Stage::Crt(crt) => self.gpu.crt(device, from, to, crt, &place),
                Stage::Upscale(map) => {
                    self.gpu.upscale(from, to, &map, clear);
                    let k = map.scale();
                    place = Place {
                        layout: scene.layout,
                        scale: low_fit.scale * k[0],
                        offset: [
                            map.offset[0] as f32 + low_fit.offset[0] * k[0],
                            map.offset[1] as f32 + low_fit.offset[1] * k[1],
                        ],
                    };
                    let mut text = SceneCopy {
                        draws: copy.draws.clone(),
                        text: copy.text.clone(),
                        curve: copy.curve.clone(),
                        clear: None,
                    };
                    if crisp {
                        text.keep(true);
                        text.place_text(
                            [
                                low_fit.scale * k[0] / fit.scale,
                                low_fit.scale * k[1] / fit.scale,
                            ],
                            [
                                (map.offset[0] as f32 + low_fit.offset[0] * k[0] - fit.offset[0])
                                    / fit.scale,
                                (map.offset[1] as f32 + low_fit.offset[1] * k[1] - fit.offset[1])
                                    / fit.scale,
                            ],
                        );
                        let pass = self.pass(device, queue, index * 2 + 1);
                        pass.prepare(device, queue, &text.scene(scene), size)?;
                    }
                }
                Stage::Text => {
                    self.gpu
                        .flush(device, queue, encoder, profiler.as_deref_mut());
                    if let Some(Some(pass)) = self.passes.get(index * 2 + 1) {
                        pass.encode_colour(encoder, &current, None, profiler.as_deref_mut());
                    }
                    self.stages += 1;
                    continue;
                }
            }
            self.stages += 1;
            current = next;
            current_size = next_size;
        }
        if let Some(output) = output
            && current_size == size
            && count > 0
            && &current != output
        {
            self.gpu.copy(
                Surface {
                    view: &current,
                    size,
                },
                Surface { view: output, size },
            );
            current = output.clone();
        }
        Ok(current)
    }
}
