use pfx_input::glyph::BOX;
use pfx_input::{Control, Glyph, GlyphAtlas, Input, Key, KeyLabels, KeycapLabel, glyph_for};
use pfx_live::text::{RichQuad, page_quads};
use pfx_play::{Prompt, PromptOf};
use pfx_text::{Face, Placement, Representation, TextEngine};

pub const EVERYWHERE: [f32; 4] = [-f32::MAX, -f32::MAX, f32::MAX, f32::MAX];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Glyph(Glyph),
    Key(Key),
}

pub(crate) fn placement(pixels_per_unit: f32) -> Placement {
    Placement {
        scale: pixels_per_unit.max(f32::MIN_POSITIVE),
        representation: Representation::Msdf,
        origin: [0.0, 0.0],
        alpha: 1.0,
        clip: EVERYWHERE,
        turn: [0.0; 3],
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PromptQuads {
    pub glyphs: Vec<RichQuad>,
    pub labels: Vec<(u32, Vec<RichQuad>)>,
    pub width: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub family: String,
    pub weight: u16,
    pub gap: f32,
    pub floor: f32,
    pub pixels_per_unit: f32,
}

impl Style {
    pub fn of(prompt: &Prompt, pixels_per_unit: f32) -> Self {
        Self {
            family: prompt.family.clone(),
            weight: prompt.weight,
            gap: prompt.gap,
            floor: prompt.floor,
            pixels_per_unit,
        }
    }
}

pub struct Sources<'a> {
    pub atlas: &'a GlyphAtlas,
    pub text: Option<&'a mut TextEngine>,
    pub labels: &'a KeyLabels,
}

pub fn parts(input: &Input, of: &PromptOf) -> Vec<Part> {
    let style = input.glyph_style();
    let part = |control: Control| match control {
        Control::Key(key) => Part::Key(key),
        other => Part::Glyph(glyph_for(style, other)),
    };
    match of {
        PromptOf::Action(name) => input.prompt(name).into_iter().map(part).collect(),
        PromptOf::Control(control) => vec![part(*control)],
        PromptOf::Glyph(glyph) => vec![Part::Glyph(*glyph)],
    }
}

pub fn layout(
    prompt: &[Part],
    at: [f32; 2],
    size: f32,
    style: &Style,
    sources: &mut Sources<'_>,
) -> Result<PromptQuads, String> {
    let scale = size / BOX;
    let floor = if size > 0.0 {
        style.floor.max(0.0) * BOX / size
    } else {
        0.0
    };
    let face = Face {
        family: style.family.clone(),
        size: 1.0,
        line: 1.25,
        weight: style.weight,
        italic: false,
        spacing: 0.0,
    };
    let placement = placement(style.pixels_per_unit);
    let [mut x, y] = at;
    let mut quads = PromptQuads::default();
    for part in prompt {
        let (glyph, cap) = match part {
            Part::Glyph(glyph) => (*glyph, None),
            Part::Key(key) => {
                let Some(engine) = sources.text.as_deref_mut() else {
                    return Err("a key prompt needs a font for its keycap (Config::font)".into());
                };
                let cap = KeycapLabel::key_with_floor(engine, &face, sources.labels, *key, floor)
                    .map_err(|error| format!("the keycap of {key:?}: {error:?}"))?;
                (cap.glyph, Some(cap))
            }
        };
        let pieces = sources
            .atlas
            .pieces(glyph)
            .ok_or_else(|| format!("the input glyph atlas has no {glyph:?}"))?;
        for cell in &pieces {
            quads.glyphs.push(RichQuad {
                rect: [
                    x + cell.offset[0] * scale,
                    y + cell.offset[1] * scale,
                    cell.size[0] * scale,
                    cell.size[1] * scale,
                ],
                uv: cell.uv,
                origin: [x, y],
                alpha: 1.0,
                clip: EVERYWHERE,
                turn: [0.0; 3],
                color: [1.0; 4],
            });
        }
        if let (Some(cap), Some(engine)) = (cap, sources.text.as_deref_mut()) {
            let placed = cap
                .place(engine, &face, [1.0; 4], [x, y], size, &placement)
                .map_err(|error| format!("the keycap label {:?}: {error:?}", cap.text))?;
            quads.labels.extend(page_quads(&placed.quads));
        }
        let advance = pieces.first().map_or(BOX, |cell| cell.box_size[0]) * scale;
        quads.width = x + advance - at[0];
        x += advance + style.gap;
    }
    Ok(quads)
}
