use crate::layers::{Style, Text};
use pfx_core::ease::Ease;
use std::collections::BTreeMap;

const SMASH: f64 = 0.18;

pub struct Motion {
    pub opacity: f32,
    pub scale: f32,
    pub progress: f32,
    pub offset: [f32; 2],
    pub age: f64,
}

pub fn motion(at: f64, end: f64, s: f64, style: &Style) -> Option<Motion> {
    if s < at || s >= end {
        return None;
    }
    let age = s - at;
    let life = (end - at).max(1e-6);
    let fade = style.fade.unwrap_or(0.25);
    let out = style.out.unwrap_or(0.25);
    let rise = if fade > 0.0 {
        (age / fade).min(1.0)
    } else {
        1.0
    };
    let fall = if out > 0.0 {
        ((end - s) / out).min(1.0)
    } else {
        1.0
    };
    let progress = (age / life) as f32;
    let mut scale = 1.0 + style.grow.unwrap_or(0.0) * progress;
    if let Some(smash) = style.smash.filter(|v| *v != 0.0) {
        let k = Ease::CubicOut.at((age / SMASH).min(1.0) as f32);
        scale *= 1.0 + smash * (1.0 - k);
    }
    let slide = style.slide.unwrap_or([0.0; 2]);
    let k = 1.0 - Ease::CubicOut.at((age / fade.max(0.3)).min(1.0) as f32);
    Some(Motion {
        opacity: (rise.min(fall) as f32).clamp(0.0, 1.0),
        scale,
        progress,
        offset: [slide[0] * k, slide[1] * k],
        age,
    })
}

pub fn merged(text: &Text, styles: &BTreeMap<String, Style>) -> Style {
    let base = text
        .style
        .as_ref()
        .and_then(|n| styles.get(n))
        .cloned()
        .unwrap_or_default();
    let own = text.style();
    Style {
        size: own.size.or(base.size),
        weight: own.weight.or(base.weight),
        tracking: own.tracking.or(base.tracking),
        spread: own.spread.or(base.spread),
        leading: own.leading.or(base.leading),
        colour: own.colour.or(base.colour),
        align: own.align.or(base.align),
        caps: own.caps.or(base.caps),
        place: own.place.or(base.place),
        fade: own.fade.or(base.fade),
        out: own.out.or(base.out),
        grow: own.grow.or(base.grow),
        smash: own.smash.or(base.smash),
        slide: own.slide.or(base.slide),
        outline: own.outline.or(base.outline),
        edge: own.edge.or(base.edge),
    }
}
