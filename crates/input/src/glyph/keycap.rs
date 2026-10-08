use pfx_text::{Anchor, Error, Face, Placed, Placement, Span, TextEngine};

use super::{BOX, Glyph, glyph};
use crate::device::{Control, Key};
use crate::family::Family;
use crate::layout::KeyLabels;

pub const KEYCAP_WIDTHS: [u16; 6] = [48, 64, 80, 96, 128, 160];
pub const INSET: f32 = 10.0;
pub const LETTER_EM: f32 = 26.0;
pub const WORD_EM: f32 = 18.0;
pub const CAP_HEIGHT: f32 = 0.7;

pub fn room(width: u16) -> f32 {
    f32::from(width) - 2.0 * INSET
}

pub fn keycap_for(label_width: f32) -> Glyph {
    Glyph::Keycap(
        KEYCAP_WIDTHS
            .into_iter()
            .find(|width| room(*width) >= label_width)
            .unwrap_or(widest()),
    )
}

pub fn label_em(text: &str) -> f32 {
    if text.chars().count() == 1 {
        LETTER_EM
    } else {
        WORD_EM
    }
}

pub fn estimate(text: &str) -> f32 {
    text.chars().count() as f32 * 0.6 * label_em(text)
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeycapLabel {
    pub glyph: Glyph,
    pub text: String,
    pub size: f32,
    pub origin: [f32; 2],
}

impl KeycapLabel {
    pub fn key(
        engine: &mut TextEngine,
        face: &Face,
        labels: &KeyLabels,
        key: Key,
    ) -> Result<Self, Error> {
        Self::key_with_floor(engine, face, labels, key, 0.0)
    }

    pub fn key_with_floor(
        engine: &mut TextEngine,
        face: &Face,
        labels: &KeyLabels,
        key: Key,
        floor: f32,
    ) -> Result<Self, Error> {
        match glyph(Family::Generic, Control::Key(key)) {
            Glyph::Keycap(_) => Self::fit_with_floor(engine, face, labels.label(key), floor),
            other => Ok(Self {
                glyph: other,
                text: String::new(),
                size: 0.0,
                origin: [0.0, 0.0],
            }),
        }
    }

    pub fn fit(engine: &mut TextEngine, face: &Face, text: &str) -> Result<Self, Error> {
        Self::fit_with_floor(engine, face, text, 0.0)
    }

    pub fn fit_with_floor(
        engine: &mut TextEngine,
        face: &Face,
        text: &str,
        floor: f32,
    ) -> Result<Self, Error> {
        if !floor.is_finite() || floor < 0.0 {
            return Err(Error::InvalidStyle);
        }
        let em = label_em(text);
        let span = span(text, face, em, [0.0; 4]);
        let block = engine.layout_spans(&[span], None, 1.0, Anchor::Start)?;
        let (width, baseline) = (block.width, block.baseline);
        let Glyph::Keycap(fitted) = keycap_for(width) else {
            return Err(Error::InvalidStyle);
        };
        let mut cap = fitted;
        let mut shrink = if width > room(cap) {
            room(cap) / width
        } else {
            1.0
        };
        if em * shrink < floor {
            shrink = floor.min(em) / em;
            cap = widened(width * shrink).max(fitted);
        }
        let glyph = Glyph::Keycap(cap);
        let size = em * shrink;
        Ok(Self {
            glyph,
            text: text.to_owned(),
            size,
            origin: [
                (f32::from(cap) - width * shrink) / 2.0,
                BOX / 2.0 + CAP_HEIGHT * size / 2.0 - baseline * shrink,
            ],
        })
    }

    pub fn place(
        &self,
        engine: &mut TextEngine,
        face: &Face,
        color: [f32; 4],
        origin: [f32; 2],
        height: f32,
        placement: &Placement,
    ) -> Result<Placed, Error> {
        if self.text.is_empty() {
            return Ok(Placed::default());
        }
        let k = height / BOX;
        let span = span(&self.text, face, self.size * k, color);
        engine.place_spans(
            &[span],
            None,
            Anchor::Start,
            &Placement {
                origin: [
                    origin[0] + self.origin[0] * k,
                    origin[1] + self.origin[1] * k,
                ],
                ..*placement
            },
        )
    }
}

pub fn widened(label_width: f32) -> u16 {
    (label_width + 2.0 * INSET)
        .ceil()
        .clamp(0.0, f32::from(u16::MAX)) as u16
}

pub fn widest() -> u16 {
    KEYCAP_WIDTHS[KEYCAP_WIDTHS.len() - 1]
}

fn span(text: &str, face: &Face, size: f32, color: [f32; 4]) -> Span {
    Span {
        text: text.to_owned(),
        face: Face {
            size,
            line: face.line / face.size * size,
            ..face.clone()
        },
        color,
    }
}
