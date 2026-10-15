use crate::marks::{Badge, Marks};
use crate::script::Drawn;
use crate::sheet::Sheet;
use crate::titles::{Setter, Words};
use pfx_live::flat::{Draw, Fill, Gradient, Shadow, Srgba, multiply, scaling, translation};

pub const LIFT: f32 = 12.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Art<D> {
    Words(Words),
    Rule {
        centre: [f32; 2],
        size: [f32; 2],
        colour: Srgba,
    },
    Band {
        centre: [f32; 2],
        size: [f32; 2],
        soft: f32,
        colour: Srgba,
    },
    Glow {
        centre: [f32; 2],
        radius: [f32; 2],
        colour: Srgba,
    },
    Shade {
        centre: [f32; 2],
        radius: [f32; 2],
        colour: Srgba,
    },
    Spot {
        centre: [f32; 2],
        radius: [f32; 2],
        colour: Srgba,
    },
    Mark(Box<Badge>),
    App(D),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Piece<D> {
    pub art: Art<D>,
    pub opacity: f32,
    pub scale: f32,
}

pub struct Kit<'a> {
    pub marks: &'a Marks,
    pub setter: &'a mut Setter,
    pub glow: bool,
}

pub fn glow(b: &mut Vec<Draw>, centre: [f32; 2], radius: [f32; 2], colour: Srgba) {
    soft(b, centre, radius, colour, true);
}

pub fn soft(b: &mut Vec<Draw>, centre: [f32; 2], radius: [f32; 2], colour: Srgba, light: bool) {
    let [r, g, bl, a] = colour.0;
    let stops = [
        (0.0, [r, g, bl, a]),
        (0.45, [r, g, bl, a * 0.35]),
        (1.0, [r, g, bl, 0.0]),
    ];
    let Ok(gradient) = Gradient::radial([0.0, 0.0], radius, &stops) else {
        return;
    };
    let mut draw = Draw::rect(centre, [radius[0] * 2.0, radius[1] * 2.0], 0.0)
        .shadow(Shadow::None)
        .elevation(LIFT - 1.0);
    draw.fx.additive = light;
    draw.fill = Fill::Gradient(gradient);
    b.push(draw);
}

fn band(b: &mut Vec<Draw>, centre: [f32; 2], size: [f32; 2], soft: f32, colour: Srgba) {
    let soft = soft.max(0.0);
    let tall = size[1].max(0.0) + 2.0 * soft;
    if tall <= 0.0 {
        return;
    }
    let mut draw = Draw::rect(centre, [size[0], tall], 0.0)
        .shadow(Shadow::None)
        .elevation(LIFT);
    if soft <= 0.0 {
        b.push(draw.fill(colour));
        return;
    }
    let [r, g, bl, a] = colour.0;
    let edge = soft / tall;
    let ramp = |t: f32| t * t * (3.0 - 2.0 * t);
    let mut stops = Vec::with_capacity(8);
    for i in 0..=3 {
        let t = i as f32 / 3.0;
        stops.push((edge * t, [r, g, bl, a * ramp(t)]));
    }
    for i in (0..=3).rev() {
        let t = i as f32 / 3.0;
        stops.push((1.0 - edge * t, [r, g, bl, a * ramp(t)]));
    }
    let Ok(gradient) = Gradient::linear([0.0, -tall * 0.5], [0.0, tall * 0.5], &stops) else {
        return;
    };
    draw.fill = Fill::Gradient(gradient);
    b.push(draw);
}

fn spot(b: &mut Vec<Draw>, size: [f32; 2], centre: [f32; 2], radius: [f32; 2], colour: Srgba) {
    let [r, g, bl, a] = colour.0;
    let reach = [
        centre[0].max(size[0] - centre[0]),
        centre[1].max(size[1] - centre[1]),
    ];
    let k = (reach[0] / radius[0].max(1.0))
        .max(reach[1] / radius[1].max(1.0))
        .max(1.0)
        * 1.5;
    let stops = [
        (0.0, [r, g, bl, 0.0]),
        (0.68 / k, [r, g, bl, 0.0]),
        (1.0 / k, [r, g, bl, a]),
        (1.0, [r, g, bl, a]),
    ];
    let mid = [size[0] * 0.5, size[1] * 0.5];
    let at = [centre[0] - mid[0], centre[1] - mid[1]];
    let Ok(gradient) = Gradient::radial(at, [radius[0] * k, radius[1] * k], &stops) else {
        return;
    };
    let mut draw = Draw::rect(mid, size, 0.0)
        .shadow(Shadow::None)
        .elevation(LIFT - 1.0);
    draw.fill = Fill::Gradient(gradient);
    b.push(draw);
}

fn draw<S: Sheet>(b: &mut S, d: &mut Kit<'_>, art: &Art<S::Drawn>) {
    match art {
        Art::Words(w) => d.setter.set(b, w, d.glow),
        Art::Rule {
            centre,
            size,
            colour,
        } => b.add(
            Draw::rect(*centre, *size, size[1].min(size[0]) * 0.5)
                .fill(*colour)
                .elevation(LIFT)
                .shadow(Shadow::None),
        ),
        Art::Band {
            centre,
            size,
            soft,
            colour,
        } => band(b.draws(), *centre, *size, *soft, *colour),
        Art::Glow {
            centre,
            radius,
            colour,
        } => glow(b.draws(), *centre, *radius, *colour),
        Art::Shade {
            centre,
            radius,
            colour,
        } => soft(b.draws(), *centre, *radius, *colour, false),
        Art::Spot {
            centre,
            radius,
            colour,
        } => {
            let size = b.size();
            spot(b.draws(), size, *centre, *radius, *colour)
        }
        Art::Mark(m) => d.marks.draw(b, d.setter, m, d.glow),
        Art::App(a) => b.art(a),
    }
}

pub fn place<S: Sheet>(b: &mut S, d: &mut Kit<'_>, piece: &Piece<S::Drawn>) {
    if piece.opacity <= 0.0 {
        return;
    }
    let from = b.draws().len();
    let mut art = piece.art.clone();
    if let Art::Words(w) = &mut art {
        w.opacity *= piece.opacity;
        w.scale *= piece.scale;
        d.setter.set(b, w, d.glow);
        return;
    }
    draw(b, d, &art);
    if d.glow && !matches!(art, Art::Mark(_)) {
        for draw in &mut b.draws()[from..] {
            draw.fx.bloom = true;
        }
    }
    if let Some(c) = anchor(&art).filter(|_| (piece.scale - 1.0).abs() > 1e-4) {
        let s = piece.scale.max(0.0);
        let m = multiply(
            translation(c[0], c[1]),
            multiply(scaling(s, s), translation(-c[0], -c[1])),
        );
        for draw in &mut b.draws()[from..] {
            draw.transform = multiply(m, draw.transform);
        }
    }
    let flat = match &art {
        Art::Glow { .. }
        | Art::Shade { .. }
        | Art::Spot { .. }
        | Art::Rule { .. }
        | Art::Band { .. } => true,
        Art::App(a) => a.flat(),
        Art::Words(_) | Art::Mark(_) => false,
    };
    if flat {
        for draw in &mut b.draws()[from..] {
            draw.opacity *= piece.opacity;
        }
        return;
    }
    let group = b.group(piece.opacity);
    for draw in &mut b.draws()[from..] {
        draw.group = group;
    }
}

fn anchor<D: Drawn>(art: &Art<D>) -> Option<[f32; 2]> {
    match art {
        Art::Words(_) | Art::Spot { .. } => None,
        Art::Rule { centre, .. }
        | Art::Band { centre, .. }
        | Art::Glow { centre, .. }
        | Art::Shade { centre, .. } => Some(*centre),
        Art::Mark(m) => Some(m.centre),
        Art::App(a) => a.anchor(),
    }
}
