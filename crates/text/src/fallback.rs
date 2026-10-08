use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use cosmic_text::{Attrs, Family, Font, LayoutRun, Metrics, fontdb};
use unicode_script::Script;
use unicode_segmentation::UnicodeSegmentation;

use crate::{Error, Face, NumberForms, Span, TextEngine, face_attrs};

pub(crate) struct Explicit;

impl cosmic_text::Fallback for Explicit {
    fn common_fallback(&self) -> &[&'static str] {
        &[]
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        &[]
    }

    fn script_fallback(&self, _: Script, _: &str) -> &[&'static str] {
        &[]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fallback {
    pub family: String,
    pub weight: Option<u16>,
    pub scale: Option<f32>,
}

impl Fallback {
    pub fn new(family: &str) -> Self {
        Self {
            family: family.into(),
            weight: None,
            scale: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub range: Range<usize>,
    pub family: String,
    pub weight: u16,
    pub scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub cap_height: f32,
    pub x_height: f32,
}

#[derive(Default)]
pub(crate) struct Fallbacks {
    lists: BTreeMap<String, BTreeMap<Option<u16>, Vec<Fallback>>>,
}

impl Fallbacks {
    fn list(&self, family: &str, weight: u16) -> Option<&[Fallback]> {
        let lists = self.lists.get(family)?;
        lists
            .get(&Some(weight))
            .or_else(|| lists.get(&None))
            .map(Vec::as_slice)
    }
}

#[derive(Clone)]
struct Choice {
    id: fontdb::ID,
    family: String,
    weight: u16,
    scale: f32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Piece {
    pub span: usize,
    pub fallback: bool,
    pub scale: f32,
    pub ascent: f32,
    pub descent: f32,
}

pub(crate) struct Laid {
    pub texts: Vec<(usize, Range<usize>)>,
    pub faces: Vec<Face>,
    pub pieces: Vec<Piece>,
}

impl Laid {
    pub fn attrs(&self, scale: f32, forms: Option<NumberForms>) -> Vec<Attrs<'_>> {
        self.faces
            .iter()
            .zip(&self.pieces)
            .enumerate()
            .map(|(index, (face, piece))| {
                let attrs = face_attrs(face, scale, index, forms);
                if !piece.fallback {
                    attrs
                } else {
                    attrs.metrics(Metrics::new(
                        face.size * scale * piece.scale,
                        face.line * scale,
                    ))
                }
            })
            .collect()
    }
}

fn ignorable(c: char) -> bool {
    matches!(c, '\u{200c}' | '\u{200d}' | '\u{fe00}'..='\u{fe0f}' | '\u{e0100}'..='\u{e01ef}')
        || c.is_control()
}

fn em_metrics(font: &Font) -> (f32, f32) {
    let metrics = font.metrics();
    let em = f32::from(metrics.units_per_em);
    (metrics.ascent / em, -metrics.descent / em)
}

fn swash_metrics(font: &Font, weight: u16) -> FaceMetrics {
    let swash = font.as_swash();
    let coords = swash
        .variations()
        .normalized_coords([swash::Setting {
            tag: swash::Tag::from_be_bytes(*b"wght"),
            value: f32::from(weight),
        }])
        .collect::<Vec<_>>();
    let metrics = swash.metrics(&coords);
    let em = f32::from(metrics.units_per_em);
    FaceMetrics {
        ascent: metrics.ascent / em,
        descent: metrics.descent / em,
        cap_height: metrics.cap_height / em,
        x_height: metrics.x_height / em,
    }
}

pub fn matched_scale(primary: FaceMetrics, fallback: FaceMetrics) -> f32 {
    let ratio = if primary.cap_height > 0.0 && fallback.cap_height > 0.0 {
        primary.cap_height / fallback.cap_height
    } else if primary.x_height > 0.0 && fallback.x_height > 0.0 {
        primary.x_height / fallback.x_height
    } else {
        1.0
    };
    (ratio * 10_000.0).round() / 10_000.0
}

impl TextEngine {
    pub fn set_fallbacks(
        &mut self,
        primary: &str,
        weight: Option<u16>,
        faces: &[Fallback],
    ) -> Result<(), Error> {
        let known = |family: &str| {
            self.fonts
                .db()
                .faces()
                .any(|face| face.families.iter().any(|(name, _)| name == family))
        };
        if !known(primary) || faces.iter().any(|face| !known(&face.family)) {
            return Err(Error::InvalidFont);
        }
        if weight.is_some_and(|w| !(1..=1000).contains(&w))
            || faces.iter().any(|face| {
                face.weight.is_some_and(|w| !(1..=1000).contains(&w))
                    || face.scale.is_some_and(|s| !s.is_finite() || s <= 0.0)
            })
        {
            return Err(Error::InvalidStyle);
        }
        let lists = self.fallbacks.lists.entry(primary.into()).or_default();
        if faces.is_empty() {
            lists.remove(&weight);
        } else {
            lists.insert(weight, faces.to_vec());
        }
        if lists.is_empty() {
            self.fallbacks.lists.remove(primary);
        }
        self.blocks.clear();
        Ok(())
    }

    pub fn families(&self) -> Vec<String> {
        let mut families = self
            .fonts
            .db()
            .faces()
            .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
            .collect::<Vec<_>>();
        families.sort();
        families.dedup();
        families
    }

    pub fn face_metrics(&mut self, family: &str, weight: u16) -> Option<FaceMetrics> {
        let id = self.query(family, weight, false)?;
        let weight = self.resolved_weight(id, weight);
        let font = self.fonts.get_font(id, fontdb::Weight(weight))?;
        Some(swash_metrics(&font, weight))
    }

    pub fn picks(&mut self, span: &Span) -> Vec<Pick> {
        let laid = self.split(std::slice::from_ref(span));
        laid.texts
            .iter()
            .zip(&laid.faces)
            .zip(&laid.pieces)
            .map(|(((_, range), face), piece)| Pick {
                range: range.clone(),
                family: face.family.clone(),
                weight: face.weight,
                scale: piece.scale,
            })
            .collect()
    }

    fn query(&self, family: &str, weight: u16, italic: bool) -> Option<fontdb::ID> {
        self.fonts.db().query(&fontdb::Query {
            families: &[Family::Name(family)],
            weight: fontdb::Weight(weight),
            stretch: fontdb::Stretch::Normal,
            style: if italic {
                fontdb::Style::Italic
            } else {
                fontdb::Style::Normal
            },
        })
    }

    fn resolved_weight(&mut self, id: fontdb::ID, weight: u16) -> u16 {
        let face_weight = self
            .fonts
            .db()
            .face(id)
            .map_or(weight, |face| face.weight.0);
        let Some(font) = self.fonts.get_font(id, fontdb::Weight(face_weight)) else {
            return weight;
        };
        let axis = font
            .as_swash()
            .variations()
            .find_by_tag(swash::Tag::from_be_bytes(*b"wght"))
            .map(|axis| (axis.min_value(), axis.max_value()));
        match axis {
            Some((min, max)) => f32::from(weight).clamp(min, max).round() as u16,
            None => face_weight,
        }
    }

    fn choices(&mut self, face: &Face) -> Option<Vec<Choice>> {
        let list = self.fallbacks.list(&face.family, face.weight)?.to_vec();
        let primary = self.query(&face.family, face.weight, face.italic)?;
        let primary_metrics = {
            let weight = self.resolved_weight(primary, face.weight);
            let font = self.fonts.get_font(primary, fontdb::Weight(weight))?;
            swash_metrics(&font, weight)
        };
        let mut choices = vec![Choice {
            id: primary,
            family: face.family.clone(),
            weight: face.weight,
            scale: 1.0,
        }];
        for fallback in list {
            let wanted = fallback.weight.unwrap_or(face.weight);
            let Some(id) = self.query(&fallback.family, wanted, face.italic) else {
                continue;
            };
            let weight = self.resolved_weight(id, wanted);
            let Some(font) = self.fonts.get_font(id, fontdb::Weight(weight)) else {
                continue;
            };
            let scale = fallback
                .scale
                .unwrap_or_else(|| matched_scale(primary_metrics, swash_metrics(&font, weight)));
            choices.push(Choice {
                id,
                family: fallback.family,
                weight,
                scale,
            });
        }
        Some(choices)
    }

    fn fonts_of(&mut self, choices: &[Choice]) -> Vec<Option<Arc<Font>>> {
        choices
            .iter()
            .map(|choice| {
                self.fonts
                    .get_font(choice.id, fontdb::Weight(choice.weight))
            })
            .collect()
    }

    pub(crate) fn split(&mut self, spans: &[Span]) -> Laid {
        let mut laid = Laid {
            texts: Vec::new(),
            faces: Vec::new(),
            pieces: Vec::new(),
        };
        for (index, span) in spans.iter().enumerate() {
            let whole = |laid: &mut Laid| {
                laid.texts.push((index, 0..span.text.len()));
                laid.faces.push(span.face.clone());
                laid.pieces.push(Piece {
                    span: index,
                    fallback: false,
                    scale: 1.0,
                    ascent: 0.0,
                    descent: 0.0,
                });
            };
            let Some(choices) = self.choices(&span.face) else {
                whole(&mut laid);
                continue;
            };
            let fonts = self.fonts_of(&choices);
            let Some(primary) = fonts[0].as_ref() else {
                whole(&mut laid);
                continue;
            };
            let (ascent, descent) = em_metrics(primary);
            let charmaps = fonts
                .iter()
                .map(|font| font.as_ref().map(|font| font.as_swash().charmap()))
                .collect::<Vec<_>>();
            let covers = |slot: usize, text: &str| {
                charmaps[slot].as_ref().is_some_and(|map| {
                    text.chars()
                        .filter(|&c| !ignorable(c))
                        .all(|c| map.map(c) != 0)
                })
            };
            let base_covered = |slot: usize, text: &str| {
                charmaps[slot].as_ref().is_some_and(|map| {
                    text.chars()
                        .find(|&c| !ignorable(c))
                        .is_some_and(|c| map.map(c) != 0)
                })
            };
            let mut runs: Vec<(usize, Range<usize>)> = Vec::new();
            for (start, grapheme) in span.text.grapheme_indices(true) {
                let end = start + grapheme.len();
                let slot = if grapheme.chars().all(ignorable) {
                    runs.last().map_or(0, |run| run.0)
                } else if covers(0, grapheme) {
                    0
                } else {
                    (1..choices.len())
                        .find(|&slot| covers(slot, grapheme))
                        .or_else(|| (1..choices.len()).find(|&slot| base_covered(slot, grapheme)))
                        .unwrap_or(0)
                };
                match runs.last_mut() {
                    Some(run) if run.0 == slot => run.1.end = end,
                    _ => runs.push((slot, start..end)),
                }
            }
            if runs.is_empty() {
                runs.push((0, 0..0));
            }
            for (slot, range) in runs {
                let choice = &choices[slot];
                let face = if slot == 0 {
                    span.face.clone()
                } else {
                    Face {
                        family: choice.family.clone(),
                        weight: choice.weight,
                        spacing: span.face.spacing / choice.scale,
                        ..span.face.clone()
                    }
                };
                laid.texts.push((index, range));
                laid.faces.push(face);
                laid.pieces.push(Piece {
                    span: index,
                    fallback: slot > 0,
                    scale: if slot == 0 { 1.0 } else { choice.scale },
                    ascent,
                    descent,
                });
            }
        }
        laid
    }

    pub(crate) fn line_baselines<'a>(
        &mut self,
        runs: impl Iterator<Item = LayoutRun<'a>>,
        pieces: &[Piece],
    ) -> Vec<f32> {
        runs.map(|run| {
            let fallback = |metadata: usize| pieces.get(metadata).filter(|piece| piece.fallback);
            if !run
                .glyphs
                .iter()
                .any(|glyph| fallback(glyph.metadata).is_some())
            {
                return run.line_y;
            }
            let mut ascent = 0.0f32;
            let mut descent = 0.0f32;
            for glyph in run.glyphs {
                let (size, a, d) = match fallback(glyph.metadata) {
                    Some(piece) => (glyph.font_size / piece.scale, piece.ascent, piece.descent),
                    None => match self.fonts.get_font(glyph.font_id, glyph.font_weight) {
                        Some(font) => {
                            let (a, d) = em_metrics(&font);
                            (glyph.font_size, a, d)
                        }
                        None => continue,
                    },
                };
                ascent = ascent.max(size * a);
                descent = descent.max(size * d);
            }
            run.line_top + (run.line_height - (ascent + descent)) / 2.0 + ascent
        })
        .collect()
    }
}
