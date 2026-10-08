use std::hash::{Hash, Hasher};

use cosmic_text::{Attrs, Buffer, LayoutGlyph, Metrics, Shaping, SubpixelBin, Wrap};

use crate::{
    Anchor, Error, Face, Pen, Placed, Placement, RasterKey, Representation, Span, Spot, TextEngine,
    face_attrs, field_ppem, raster_flags, raster_key,
};

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum Figures {
    #[default]
    Tabular,
    TabularDigits,
    Proportional,
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum Ligatures {
    #[default]
    Kept,
    Off,
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub struct NumberForms {
    pub figures: Figures,
    pub ligatures: Ligatures,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Columns {
    widest: f32,
    solo: [f32; 10],
}

struct NumberGlyph {
    spot: Spot,
    advance: f32,
    coverage: [RasterKey; 4],
    msdf: RasterKey,
}

pub(crate) struct NumberFace {
    pub(crate) stamp: u64,
    pub(crate) bytes: u64,
    chars: Vec<char>,
    glyphs: Vec<Option<NumberGlyph>>,
    kern: Vec<Option<f32>>,
    columns: Option<Columns>,
    line_y: f32,
    height: f32,
}

fn pair_sequence(count: usize) -> Vec<usize> {
    let mut next = vec![0usize; count];
    let mut stack = vec![0usize];
    let mut circuit = Vec::with_capacity(count * count + 1);
    while let Some(&node) = stack.last() {
        if next[node] < count {
            stack.push(next[node]);
            next[node] += 1;
        } else {
            circuit.push(node);
            stack.pop();
        }
    }
    circuit.reverse();
    circuit
}

fn face_key(face: &Face, chars: &str, scale: f32, forms: NumberForms) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    chars.hash(&mut hasher);
    face.family.hash(&mut hasher);
    face.size.to_bits().hash(&mut hasher);
    face.line.to_bits().hash(&mut hasher);
    face.weight.hash(&mut hasher);
    face.italic.hash(&mut hasher);
    face.spacing.to_bits().hash(&mut hasher);
    scale.to_bits().hash(&mut hasher);
    forms.hash(&mut hasher);
    hasher.finish()
}

fn valid(face: &Face, color: [f32; 4], scale: f32) -> bool {
    scale.is_finite()
        && scale > 0.0
        && face.size.is_finite()
        && face.size > 0.0
        && face.line.is_finite()
        && face.line > 0.0
        && face.spacing.is_finite()
        && color.iter().all(|c| c.is_finite())
}

fn same_glyph(solo: Option<&LayoutGlyph>, glyph: &LayoutGlyph) -> bool {
    solo.is_some_and(|solo| solo.glyph_id == glyph.glyph_id && solo.font_id == glyph.font_id)
}

fn single(glyphs: Vec<LayoutGlyph>) -> Option<LayoutGlyph> {
    <[LayoutGlyph; 1]>::try_from(glyphs)
        .ok()
        .map(|[glyph]| glyph)
}

pub(crate) fn char_lines(chars: &[char]) -> String {
    chars
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn unique_chars(chars: &str) -> Vec<char> {
    let mut set = Vec::new();
    for c in chars.chars() {
        if !set.contains(&c) {
            set.push(c);
        }
    }
    set
}

pub(crate) fn bin_keys(key: &RasterKey) -> [RasterKey; 4] {
    [
        SubpixelBin::Zero,
        SubpixelBin::One,
        SubpixelBin::Two,
        SubpixelBin::Three,
    ]
    .map(|x_bin| RasterKey {
        x_bin,
        ..key.clone()
    })
}

fn unwrapped(fonts: &mut cosmic_text::FontSystem, face: &Face, scale: f32) -> Buffer {
    let mut buffer = Buffer::new(fonts, Metrics::new(face.size * scale, face.line * scale));
    buffer.set_wrap(fonts, Wrap::None);
    buffer.set_size(fonts, None, None);
    buffer
}

fn shape_lines(
    fonts: &mut cosmic_text::FontSystem,
    buffer: &mut Buffer,
    text: &str,
    attrs: &Attrs,
) -> Vec<(f32, f32, Vec<LayoutGlyph>)> {
    buffer.set_text(fonts, text, attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(fonts, false);
    let mut lines = vec![(0.0, 0.0, Vec::new()); text.split('\n').count()];
    for run in buffer.layout_runs() {
        if let Some(line) = lines.get_mut(run.line_i) {
            *line = (
                run.line_y - run.line_top,
                run.line_height,
                run.glyphs.to_vec(),
            );
        }
    }
    lines
}

impl TextEngine {
    pub fn place_number(
        &mut self,
        span: &Span,
        chars: &str,
        anchor: Anchor,
        placement: &Placement,
    ) -> Result<Placed, Error> {
        self.place_number_with(span, chars, NumberForms::default(), anchor, placement)
    }

    pub fn place_number_with(
        &mut self,
        span: &Span,
        chars: &str,
        forms: NumberForms,
        anchor: Anchor,
        placement: &Placement,
    ) -> Result<Placed, Error> {
        if !valid(&span.face, span.color, placement.scale) {
            return Err(Error::InvalidStyle);
        }
        let mut columns = None;
        if !span.text.is_empty() && span.text.chars().all(|c| chars.contains(c)) {
            let key = self.number_face(&span.face, chars, placement.scale, forms)?;
            if let Some(face) = self.numbers.remove(&key) {
                let indices = span
                    .text
                    .chars()
                    .filter_map(|c| face.chars.iter().position(|&known| known == c))
                    .collect::<Vec<_>>();
                let placed = self.place_fast(&face, &indices, span.color, anchor, placement);
                columns = face.columns;
                self.numbers.insert(key, face);
                if let Some(placed) = placed? {
                    self.shared.stats.fast += 1;
                    return Ok(placed);
                }
            }
        } else if forms.figures != Figures::Proportional
            && span.text.bytes().any(|b| b.is_ascii_digit())
        {
            columns = self
                .number_face(&span.face, chars, placement.scale, forms)
                .ok()
                .and_then(|key| self.numbers[&key].columns);
        }
        let spans = std::slice::from_ref(span);
        let key = self.layout_block(
            spans,
            None,
            placement.scale,
            Anchor::Start,
            Some(forms),
            columns,
        )?;
        let width = self.blocks[&key].width;
        let placement = Placement {
            origin: anchored(placement.origin, width, anchor),
            ..*placement
        };
        self.place_block(key, spans, &placement)
    }

    pub fn cached_number_faces(&self) -> usize {
        self.numbers.len()
    }

    pub fn ligating_pairs(
        &mut self,
        face: &Face,
        chars: &str,
        forms: NumberForms,
        scale: f32,
    ) -> Result<Vec<(char, char)>, Error> {
        if !valid(face, [0.0; 4], scale) {
            return Err(Error::InvalidStyle);
        }
        let key = self.number_face(face, chars, scale, forms)?;
        let face = &self.numbers[&key];
        let count = face.chars.len();
        let mut pairs = Vec::new();
        for i in 0..count {
            for j in 0..count {
                if face.glyphs[i].is_some()
                    && face.glyphs[j].is_some()
                    && face.kern[i * count + j].is_none()
                {
                    pairs.push((face.chars[i], face.chars[j]));
                }
            }
        }
        Ok(pairs)
    }

    fn place_fast(
        &mut self,
        face: &NumberFace,
        indices: &[usize],
        color: [f32; 4],
        anchor: Anchor,
        placement: &Placement,
    ) -> Result<Option<Placed>, Error> {
        let count = face.chars.len();
        if indices.iter().any(|&i| face.glyphs[i].is_none())
            || indices
                .windows(2)
                .any(|pair| face.kern[pair[0] * count + pair[1]].is_none())
        {
            return Ok(None);
        }
        let mut pens = Vec::with_capacity(indices.len());
        let mut x = 0.0f32;
        for (at, &i) in indices.iter().enumerate() {
            let glyph = face.glyphs[i].as_ref().unwrap();
            pens.push(x + glyph.spot.x);
            x += glyph.advance;
            if let Some(&next) = indices.get(at + 1) {
                x += if face.columns.is_some()
                    && (face.chars[i].is_ascii_digit() || face.chars[next].is_ascii_digit())
                {
                    0.0
                } else {
                    face.kern[i * count + next].unwrap_or_default()
                };
            }
        }
        let scale = placement.scale;
        let placement = Placement {
            origin: anchored(placement.origin, x / scale, anchor),
            ..*placement
        };
        let mut quads = Vec::with_capacity(indices.len());
        for (&i, pen_x) in indices.iter().zip(pens) {
            let glyph = face.glyphs[i].as_ref().unwrap();
            let spot = Spot {
                x: pen_x,
                color,
                ..glyph.spot.clone()
            };
            let pen = Pen::new(&spot, &placement);
            let key = match placement.representation {
                Representation::Coverage => &glyph.coverage[pen.bin as usize],
                Representation::Msdf => &glyph.msdf,
            };
            self.place_glyph(&spot, key, pen, &placement, &mut quads)?;
        }
        Ok(Some(Placed {
            quads,
            width: x / scale,
            height: face.height / scale,
            baseline: face.line_y / scale,
        }))
    }

    pub(crate) fn column_shifts(
        &mut self,
        buffer: &Buffer,
        columns: Columns,
        face: &Face,
        scale: f32,
        forms: NumberForms,
    ) -> (Vec<f32>, f32) {
        let mut lines = Vec::new();
        let mut wanted: Vec<char> = Vec::new();
        for run in buffer.layout_runs() {
            let glyphs = run
                .glyphs
                .iter()
                .map(|glyph| {
                    let mut chars = run.text[glyph.start..glyph.end].chars();
                    let one = chars.next().filter(|_| chars.next().is_none());
                    (one, glyph.w, glyph.glyph_id, glyph.font_id)
                })
                .collect::<Vec<_>>();
            for pair in glyphs.windows(2) {
                if let (Some(c), Some(next)) = (pair[0].0, pair[1].0)
                    && !c.is_ascii_digit()
                    && next.is_ascii_digit()
                    && !wanted.contains(&c)
                {
                    wanted.push(c);
                }
            }
            lines.push((run.line_w, glyphs));
        }
        let mut solo = Vec::new();
        if !wanted.is_empty() {
            let attrs = face_attrs(face, scale, 0, Some(forms));
            let mut alone = unwrapped(&mut self.fonts, face, scale);
            let text = char_lines(&wanted);
            solo = shape_lines(&mut self.fonts, &mut alone, &text, &attrs)
                .into_iter()
                .map(|(_, _, glyphs)| single(glyphs))
                .collect();
        }
        let digit = |c: Option<char>| c.and_then(|c| c.to_digit(10)).map(|d| d as usize);
        let mut shifts = Vec::new();
        let mut width = 0.0f32;
        for (line_w, glyphs) in lines {
            let mut shift = 0.0f32;
            for (at, &(c, w, glyph_id, font_id)) in glyphs.iter().enumerate() {
                let next = glyphs.get(at + 1).and_then(|next| digit(next.0));
                let target = match digit(c) {
                    Some(d) => {
                        shifts.push(shift + (columns.widest - columns.solo[d]) * 0.5);
                        columns.widest
                    }
                    None => {
                        shifts.push(shift);
                        c.filter(|_| next.is_some())
                            .and_then(|c| wanted.iter().position(|&known| known == c))
                            .and_then(|at| solo.get(at).and_then(Option::as_ref))
                            .filter(|alone| alone.glyph_id == glyph_id && alone.font_id == font_id)
                            .map_or(w, |alone| alone.w)
                    }
                };
                shift += target - w;
            }
            width = width.max(line_w + shift);
        }
        (shifts, width)
    }

    pub(crate) fn number_glyph_keys(
        &mut self,
        face: &Face,
        chars: &str,
        scale: f32,
        forms: NumberForms,
        representation: Representation,
    ) -> Result<Vec<(Spot, Vec<RasterKey>)>, Error> {
        if !valid(face, [0.0; 4], scale) {
            return Err(Error::InvalidStyle);
        }
        let key = self.number_face(face, chars, scale, forms)?;
        let face = &self.numbers[&key];
        Ok(face
            .glyphs
            .iter()
            .flatten()
            .map(|glyph| {
                let keys = match representation {
                    Representation::Coverage => glyph.coverage.to_vec(),
                    Representation::Msdf => vec![glyph.msdf.clone()],
                };
                (glyph.spot.clone(), keys)
            })
            .collect())
    }

    pub(crate) fn number_face(
        &mut self,
        face: &Face,
        chars: &str,
        scale: f32,
        forms: NumberForms,
    ) -> Result<u64, Error> {
        let key = face_key(face, chars, scale, forms);
        let stamp = self.stamp();
        if let Some(found) = self.numbers.get_mut(&key) {
            found.stamp = stamp;
            return Ok(key);
        }
        self.shared.stats.shaped += 1;
        let set = unique_chars(chars);
        let count = set.len();
        let mut solo_chars = set.clone();
        for digit in '0'..='9' {
            if !solo_chars.contains(&digit) {
                solo_chars.push(digit);
            }
        }
        let attrs = face_attrs(face, scale, 0, Some(forms));
        let mut buffer = unwrapped(&mut self.fonts, face, scale);
        let solo = char_lines(&solo_chars);
        let lines = shape_lines(&mut self.fonts, &mut buffer, &solo, &attrs);
        let (line_y, height) = lines
            .iter()
            .find(|line| line.1 > 0.0)
            .map(|line| (line.0, line.1))
            .ok_or(Error::UnsupportedGlyph)?;
        let found = lines
            .into_iter()
            .map(|(_, _, glyphs)| single(glyphs))
            .collect::<Vec<_>>();
        let digits = ('0'..='9')
            .map(|digit| {
                solo_chars
                    .iter()
                    .position(|&c| c == digit)
                    .and_then(|at| found[at].as_ref())
                    .map(|glyph| glyph.w)
            })
            .collect::<Option<Vec<_>>>();
        let columns = digits.and_then(|digits| {
            let widest = digits.iter().copied().fold(0.0f32, f32::max);
            (forms.figures != Figures::Proportional
                && digits.iter().any(|&w| (w - widest).abs() > 1e-3))
            .then(|| Columns {
                widest,
                solo: std::array::from_fn(|d| digits[d]),
            })
        });
        let found = &found[..count];
        let kern = self.pair_kerns(&mut buffer, &attrs, &set, found);
        let mut glyphs = Vec::with_capacity(count);
        for (i, glyph) in found.iter().enumerate() {
            let Some(glyph) = glyph else {
                glyphs.push(None);
                continue;
            };
            let font = self
                .fonts
                .get_font(glyph.font_id, glyph.font_weight)
                .ok_or(Error::UnsupportedGlyph)?;
            let flags = raster_flags(
                glyph.cache_key_flags,
                face.italic,
                self.fonts.db().face(glyph.font_id).map(|face| face.style),
            );
            let template = |ppem: f32, representation: Representation| RasterKey {
                representation,
                ..raster_key(
                    glyph.font_id,
                    glyph.glyph_id,
                    ppem,
                    glyph.font_weight,
                    glyph.font_size / scale,
                    flags,
                    font.as_swash(),
                )
            };
            let coverage = template(glyph.font_size, Representation::Coverage);
            let bins = bin_keys(&coverage);
            let msdf = template(field_ppem(glyph.font_size), Representation::Msdf);
            let column = columns.filter(|_| set[i].is_ascii_digit());
            let pad = column.map_or(0.0, |columns| (columns.widest - glyph.w) * 0.5);
            glyphs.push(Some(NumberGlyph {
                spot: Spot {
                    font_id: glyph.font_id,
                    font_weight: glyph.font_weight,
                    glyph_id: glyph.glyph_id,
                    flags,
                    font_size: glyph.font_size,
                    x: glyph.x_offset * glyph.font_size + pad,
                    y: glyph.y - glyph.y_offset * glyph.font_size,
                    line_y,
                    line_index: 0,
                    color: [0.0; 4],
                },
                advance: column.map_or(glyph.w, |columns| columns.widest),
                coverage: bins,
                msdf,
            }));
        }
        let bytes = (std::mem::size_of::<NumberFace>()
            + count * std::mem::size_of::<char>()
            + kern.len() * std::mem::size_of::<Option<f32>>()
            + glyphs.len() * std::mem::size_of::<Option<NumberGlyph>>()
            + glyphs
                .iter()
                .flatten()
                .map(|glyph| {
                    glyph
                        .coverage
                        .iter()
                        .chain([&glyph.msdf])
                        .map(|key| key.font_id.capacity())
                        .sum::<usize>()
                })
                .sum::<usize>()) as u64;
        self.numbers.insert(
            key,
            NumberFace {
                stamp,
                bytes,
                chars: set,
                glyphs,
                kern,
                columns,
                line_y,
                height,
            },
        );
        Ok(key)
    }

    fn pair_kerns(
        &mut self,
        buffer: &mut Buffer,
        attrs: &Attrs,
        set: &[char],
        solo: &[Option<LayoutGlyph>],
    ) -> Vec<Option<f32>> {
        let count = set.len();
        let mut kern = vec![None; count * count];
        if count == 0 {
            return kern;
        }
        let order = pair_sequence(count);
        let mut starts = Vec::with_capacity(order.len());
        let mut sequence = String::new();
        for &i in &order {
            starts.push(sequence.len());
            sequence.push(set[i]);
        }
        let laid = shape_lines(&mut self.fonts, buffer, &sequence, attrs);
        let mut cover = vec![0u32; order.len()];
        let mut x = vec![None; order.len()];
        if let [(_, _, glyphs)] = laid.as_slice() {
            for glyph in glyphs {
                let Ok(first) = starts.binary_search(&glyph.start) else {
                    continue;
                };
                let last = starts.partition_point(|&start| start < glyph.end);
                for slot in &mut cover[first..last.max(first + 1)] {
                    *slot += 1;
                }
                if last == first + 1 && same_glyph(solo[order[first]].as_ref(), glyph) {
                    x[first] = Some(glyph.x);
                }
            }
        }
        let clean = |at: usize| if cover[at] == 1 { x[at] } else { None };
        let advance = |i: usize| solo[i].as_ref().map_or(0.0, |glyph| glyph.w);
        let mut unresolved = Vec::new();
        for (at, pair) in order.windows(2).enumerate() {
            match (clean(at), clean(at + 1)) {
                (Some(a), Some(b)) => {
                    kern[pair[0] * count + pair[1]] = Some(b - a - advance(pair[0]))
                }
                _ => unresolved.push((pair[0], pair[1])),
            }
        }
        if unresolved.is_empty() {
            return kern;
        }
        let text = unresolved
            .iter()
            .map(|&(i, j)| format!("{}{}", set[i], set[j]))
            .collect::<Vec<_>>()
            .join("\n");
        let lines = shape_lines(&mut self.fonts, buffer, &text, attrs);
        for (&(i, j), (_, _, glyphs)) in unresolved.iter().zip(lines) {
            if let [a, b] = glyphs.as_slice()
                && a.start == 0
                && b.start == set[i].len_utf8()
                && same_glyph(solo[i].as_ref(), a)
                && same_glyph(solo[j].as_ref(), b)
            {
                kern[i * count + j] = Some(b.x - a.x - advance(i));
            }
        }
        kern
    }
}

pub(crate) fn anchored(origin: [f32; 2], width: f32, anchor: Anchor) -> [f32; 2] {
    match anchor {
        Anchor::Start => origin,
        Anchor::Center => [origin[0] - width * 0.5, origin[1]],
        Anchor::End => [origin[0] - width, origin[1]],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pair_sequence_holds_every_ordered_pair_once() {
        let count = 17;
        let order = pair_sequence(count);
        assert_eq!(order.len(), count * count + 1);
        let mut seen = vec![0; count * count];
        for pair in order.windows(2) {
            seen[pair[0] * count + pair[1]] += 1;
        }
        assert!(seen.iter().all(|&count| count == 1));
    }
}
