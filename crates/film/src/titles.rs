use crate::sheet::{Letter, Sheet};
use pfx_live::flat::{Srgba, multiply, scaling, translation};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    Start,
    #[default]
    Middle,
    End,
}

impl Align {
    pub fn share(self) -> f32 {
        match self {
            Align::Start => 0.0,
            Align::Middle => 0.5,
            Align::End => 1.0,
        }
    }
}

pub const CAP: f32 = 0.35;
const ELEVATION: f32 = 20.0;
const KEEP: usize = 4096;
const RING: usize = 16;
const INNER: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct Words {
    pub text: String,
    pub at: [f32; 2],
    pub align: Align,
    pub size: f32,
    pub weight: u16,
    pub tracking: f32,
    pub leading: f32,
    pub colour: Srgba,
    pub opacity: f32,
    pub scale: f32,
    pub outline: f32,
    pub edge: Srgba,
}

impl Words {
    pub fn new(text: impl Into<String>, at: [f32; 2], size: f32, colour: Srgba) -> Words {
        Words {
            text: text.into(),
            at,
            align: Align::Middle,
            size,
            weight: 700,
            tracking: 0.0,
            leading: 1.2,
            colour,
            opacity: 1.0,
            scale: 1.0,
            outline: 0.0,
            edge: Srgba([0.0, 0.0, 0.0, 1.0]),
        }
    }
}

type Key = (String, u32, u16);

#[derive(Default)]
pub struct Setter {
    advances: HashMap<Key, Vec<f32>>,
}

impl Setter {
    fn advances<S: Sheet>(
        &mut self,
        sheet: &mut S,
        line: &str,
        size: f32,
        weight: u16,
    ) -> Vec<f32> {
        let key = (line.to_string(), size.to_bits(), weight);
        if let Some(found) = self.advances.get(&key) {
            return found.clone();
        }
        let mut out = Vec::with_capacity(line.len() + 1);
        let mut prefix = String::new();
        out.push(0.0);
        for ch in line.chars() {
            prefix.push(ch);
            out.push(sheet.width(&prefix, size, weight));
        }
        if self.advances.len() >= KEEP {
            self.advances.clear();
        }
        self.advances.insert(key, out.clone());
        out
    }

    pub fn width<S: Sheet>(
        &mut self,
        sheet: &mut S,
        line: &str,
        size: f32,
        weight: u16,
        tracking: f32,
    ) -> f32 {
        let at = self.advances(sheet, line, size, weight);
        let n = line.chars().count();
        at.last().copied().unwrap_or(0.0) + tracking * size * n.saturating_sub(1) as f32
    }

    pub fn set<S: Sheet>(&mut self, b: &mut S, w: &Words, glow: bool) {
        if w.opacity <= 0.0 || w.text.trim().is_empty() || w.scale <= 0.0 {
            return;
        }
        let outline = w.outline.max(0.0);
        let (opacity, group) = if outline > 0.0 {
            match b.group(w.opacity) {
                Some(g) => (1.0, Some(g)),
                None => (w.opacity, None),
            }
        } else {
            (w.opacity, None)
        };
        if outline > 0.0 {
            for (count, reach) in [(RING, outline), (INNER, outline * 0.5)] {
                for k in 0..count {
                    let a = std::f32::consts::TAU * k as f32 / count as f32;
                    let shift = [a.cos() * reach, a.sin() * reach];
                    self.glyphs(b, w, w.edge, shift, opacity, group);
                }
            }
        }
        let from = b.draws().len();
        self.glyphs(b, w, w.colour, [0.0, 0.0], opacity, group);
        if glow {
            for d in &mut b.draws()[from..] {
                d.fx.bloom = true;
            }
        }
    }

    fn glyphs<S: Sheet>(
        &mut self,
        b: &mut S,
        w: &Words,
        colour: Srgba,
        shift: [f32; 2],
        opacity: f32,
        group: Option<u16>,
    ) {
        let lines: Vec<&str> = w.text.split('\n').collect();
        let step = w.size * w.leading;
        let first = -step * (lines.len() - 1) as f32 * 0.5 + CAP * w.size + shift[1];
        let frame = multiply(translation(w.at[0], w.at[1]), scaling(w.scale, w.scale));
        for (row, line) in lines.iter().enumerate() {
            let y = first + step * row as f32;
            let width = self.width(b, line, w.size, w.weight, w.tracking);
            let left = -w.align.share() * width + shift[0];
            let advances = self.advances(b, line, w.size, w.weight);
            let gap = w.tracking * w.size;
            for (i, ch) in line.chars().enumerate() {
                if ch.is_whitespace() {
                    continue;
                }
                let x = left + advances[i] + gap * i as f32;
                b.letter(Letter {
                    text: ch.to_string(),
                    size: w.size,
                    weight: w.weight,
                    colour,
                    at: [x, y],
                    frame,
                    elevation: ELEVATION,
                    opacity,
                    group,
                });
            }
        }
    }
}
