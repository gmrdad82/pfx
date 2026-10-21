use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2};
use pfx_editor_style::theme::{Caps, Theme};
use pfx_editor_style::tokens;
use pfx_splash::Badge;
use swash::scale::ScaleContext;
use swash::zeno::{Command, PathBuilder, Point};

pub const HEIGHT: f32 = 20.0;
pub const TYPE: f32 = 10.5;
pub const WEIGHT: f32 = 560.0;
pub const TRACKING: f32 = 0.14;
const EDGE: f32 = 1.0;
const GAP: f32 = 5.0;
const PLATE: f32 = 3.0;
const ROSE: Color32 = Color32::from_rgb(0xF4, 0x6A, 0x7E);
const ROSE_DEEP: Color32 = Color32::from_rgb(0x2A, 0x10, 0x16);

pub const CAPS: Caps = Caps {
    upper: true,
    tracking: TRACKING,
    weight: Some(WEIGHT),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Pill,
    Plate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Plain,
    Dot,
    Diamond,
    Rings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Art {
    pub form: Form,
    pub mark: Mark,
    pub ground: Color32,
    pub edge: Color32,
    pub ink: Color32,
    pub sign: Color32,
}

pub fn art(badge: Badge) -> Art {
    themed(badge, &tokens::theme())
}

pub fn themed(badge: Badge, theme: &Theme) -> Art {
    match badge {
        Badge::Free => Art {
            form: Form::Pill,
            mark: Mark::Plain,
            ground: theme.bg,
            edge: theme.card_line,
            ink: theme.text2,
            sign: theme.text2,
        },
        Badge::Indie => Art {
            form: Form::Pill,
            mark: Mark::Dot,
            ground: theme.accent_deep,
            edge: theme.secondary,
            ink: theme.secondary_soft,
            sign: theme.secondary,
        },
        Badge::Pro => Art {
            form: Form::Pill,
            mark: Mark::Plain,
            ground: theme.hot,
            edge: theme.hot,
            ink: theme.on_hot,
            sign: theme.on_hot,
        },
        Badge::Owner => Art {
            form: Form::Plate,
            mark: Mark::Diamond,
            ground: theme.accent,
            edge: theme.accent,
            ink: theme.title,
            sign: theme.hot,
        },
        Badge::FriendsFamily => Art {
            form: Form::Pill,
            mark: Mark::Rings,
            ground: ROSE_DEEP,
            edge: ROSE,
            ink: ROSE,
            sign: ROSE,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub mark: Option<Rect>,
    pub word: f32,
}

fn mark_width(mark: Mark) -> f32 {
    match mark {
        Mark::Plain => 0.0,
        Mark::Dot => 5.0,
        Mark::Diamond => 7.0,
        Mark::Rings => 10.0,
    }
}

pub fn layout(badge: Badge, word: f32) -> Layout {
    let art = art(badge);
    let pad = match art.form {
        Form::Pill => HEIGHT * 0.5,
        Form::Plate => 8.0,
    };
    let sign = mark_width(art.mark);
    let mark = (sign > 0.0).then(|| {
        Rect::from_min_size(
            Pos2::new(pad, (HEIGHT - sign.min(7.0)) * 0.5),
            Vec2::new(sign, sign.min(7.0)),
        )
    });
    let lead = if sign > 0.0 { sign + GAP } else { 0.0 };
    Layout {
        width: pad + lead + word + pad,
        mark,
        word: pad + lead,
    }
}

pub fn radius(form: Form) -> f32 {
    match form {
        Form::Pill => HEIGHT * 0.5,
        Form::Plate => PLATE,
    }
}

pub fn paint(
    painter: &Painter,
    at: Pos2,
    scale: f32,
    badge: Badge,
    art: Art,
    word: std::sync::Arc<egui::Galley>,
) -> Rect {
    let shape = layout(badge, word.size().x / scale);
    let rect = Rect::from_min_size(at, Vec2::new(shape.width, HEIGHT) * scale);
    let corner = CornerRadius::same((radius(art.form) * scale).round() as u8);
    painter.rect(
        rect,
        corner,
        art.ground,
        Stroke::new(EDGE * scale, art.edge),
        StrokeKind::Inside,
    );
    if let Some(mark) = shape.mark {
        let mark = Rect::from_min_size(at + mark.min.to_vec2() * scale, mark.size() * scale);
        match art.mark {
            Mark::Plain => {}
            Mark::Dot => {
                painter.circle_filled(mark.center(), mark.width() * 0.5, art.sign);
            }
            Mark::Diamond => {
                painter.add(Shape::convex_polygon(
                    vec![
                        Pos2::new(mark.center().x, mark.top()),
                        Pos2::new(mark.right(), mark.center().y),
                        Pos2::new(mark.center().x, mark.bottom()),
                        Pos2::new(mark.left(), mark.center().y),
                    ],
                    art.sign,
                    Stroke::NONE,
                ));
            }
            Mark::Rings => {
                let r = mark.height() * 0.5 - 0.5 * scale;
                for x in [
                    mark.left() + r + 0.5 * scale,
                    mark.right() - r - 0.5 * scale,
                ] {
                    painter.circle_stroke(
                        Pos2::new(x, mark.center().y),
                        r,
                        Stroke::new(1.2 * scale, art.sign),
                    );
                }
            }
        }
    }
    let top = rect.center().y - word.size().y * 0.5;
    painter.galley(Pos2::new(at.x + shape.word * scale, top), word, art.ink);
    rect
}

fn hex(colour: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", colour.r(), colour.g(), colour.b())
}

struct Piece {
    path: Vec<Command>,
    colour: Color32,
    ring: bool,
}

impl Piece {
    fn fill(path: Vec<Command>, colour: Color32) -> Piece {
        Piece {
            path,
            colour,
            ring: false,
        }
    }

    fn ring(path: Vec<Command>, colour: Color32) -> Piece {
        Piece {
            path,
            colour,
            ring: true,
        }
    }
}

struct Word {
    width: f32,
    glyphs: Vec<Vec<Command>>,
}

fn word(text: &str, size: f32) -> Word {
    let font = swash::FontRef::from_index(tokens::SANS, 0).expect("the display font reads");
    let coords: Vec<_> = font
        .variations()
        .normalized_coords([("wght", WEIGHT)])
        .collect();
    let metrics = font.glyph_metrics(&coords).scale(size);
    let mut context = ScaleContext::new();
    let mut scaler = context
        .builder(font)
        .size(size)
        .hint(false)
        .variations([("wght", WEIGHT)])
        .build();
    let tracking = TRACKING * size;
    let mut pen = 0.0;
    let mut glyphs = Vec::new();
    let caps = CAPS.text(text);
    let count = caps.chars().count();
    for (index, c) in caps.chars().enumerate() {
        let glyph = font.charmap().map(c);
        if let Some(outline) = scaler.scale_outline(glyph) {
            let mut commands = Vec::new();
            for command in swash::zeno::PathData::commands(&outline.path()) {
                commands.push(match command {
                    Command::MoveTo(p) => Command::MoveTo(Point::new(pen + p.x, -p.y)),
                    Command::LineTo(p) => Command::LineTo(Point::new(pen + p.x, -p.y)),
                    Command::QuadTo(a, p) => {
                        Command::QuadTo(Point::new(pen + a.x, -a.y), Point::new(pen + p.x, -p.y))
                    }
                    Command::CurveTo(a, b, p) => Command::CurveTo(
                        Point::new(pen + a.x, -a.y),
                        Point::new(pen + b.x, -b.y),
                        Point::new(pen + p.x, -p.y),
                    ),
                    Command::Close => Command::Close,
                });
            }
            glyphs.push(commands);
        }
        pen += metrics.advance_width(glyph);
        if index + 1 < count {
            pen += tracking;
        }
    }
    Word { width: pen, glyphs }
}

fn cap_height(size: f32) -> f32 {
    let font = swash::FontRef::from_index(tokens::SANS, 0).expect("the display font reads");
    let coords: Vec<_> = font
        .variations()
        .normalized_coords([("wght", WEIGHT)])
        .collect();
    font.metrics(&coords).scale(size).cap_height
}

fn moved(commands: &[Command], by: Vec2) -> Vec<Command> {
    let p = |p: Point| Point::new(p.x + by.x, p.y + by.y);
    commands
        .iter()
        .map(|command| match *command {
            Command::MoveTo(a) => Command::MoveTo(p(a)),
            Command::LineTo(a) => Command::LineTo(p(a)),
            Command::QuadTo(a, b) => Command::QuadTo(p(a), p(b)),
            Command::CurveTo(a, b, c) => Command::CurveTo(p(a), p(b), p(c)),
            Command::Close => Command::Close,
        })
        .collect()
}

fn round_rect(rect: Rect, radius: f32) -> Vec<Command> {
    let mut path: Vec<Command> = Vec::new();
    path.add_round_rect(
        [rect.left(), rect.top()],
        rect.width(),
        rect.height(),
        radius,
        radius,
    );
    path
}

fn pieces(badge: Badge) -> (f32, Vec<Piece>) {
    let art = art(badge);
    let text = word(badge.label(), TYPE);
    let shape = layout(badge, text.width);
    let whole = Rect::from_min_size(Pos2::ZERO, Vec2::new(shape.width, HEIGHT));
    let corner = radius(art.form);
    let mut rim = round_rect(whole, corner);
    rim.extend(round_rect(whole.shrink(EDGE), corner - EDGE));
    let mut out = vec![
        Piece::fill(round_rect(whole, corner), art.ground),
        Piece::ring(rim, art.edge),
    ];
    if let Some(mark) = shape.mark {
        let mut path: Vec<Command> = Vec::new();
        match art.mark {
            Mark::Plain => {}
            Mark::Dot => {
                path.add_circle([mark.center().x, mark.center().y], mark.width() * 0.5);
                out.push(Piece::fill(path, art.sign));
            }
            Mark::Diamond => {
                path.move_to([mark.center().x, mark.top()])
                    .line_to([mark.right(), mark.center().y])
                    .line_to([mark.center().x, mark.bottom()])
                    .line_to([mark.left(), mark.center().y])
                    .close();
                out.push(Piece::fill(path, art.sign));
            }
            Mark::Rings => {
                let r = mark.height() * 0.5 - 0.5;
                for x in [mark.left() + r + 0.5, mark.right() - r - 0.5] {
                    let mut ring: Vec<Command> = Vec::new();
                    ring.add_circle([x, mark.center().y], r + 0.6);
                    ring.add_circle([x, mark.center().y], r - 0.6);
                    out.push(Piece::ring(ring, art.sign));
                }
            }
        }
    }
    let baseline = (HEIGHT + cap_height(TYPE)) * 0.5;
    for glyph in &text.glyphs {
        out.push(Piece::fill(
            moved(glyph, Vec2::new(shape.word, baseline)),
            art.ink,
        ));
    }
    (shape.width, out)
}

fn d(commands: &[Command]) -> String {
    let mut out = String::new();
    let n = |v: f32| {
        let text = format!("{:.3}", v);
        let text = text.trim_end_matches('0').trim_end_matches('.').to_string();
        if text == "-0" { "0".to_string() } else { text }
    };
    for command in commands {
        match *command {
            Command::MoveTo(p) => out += &format!("M{} {}", n(p.x), n(p.y)),
            Command::LineTo(p) => out += &format!("L{} {}", n(p.x), n(p.y)),
            Command::QuadTo(a, p) => {
                out += &format!("Q{} {} {} {}", n(a.x), n(a.y), n(p.x), n(p.y))
            }
            Command::CurveTo(a, b, p) => {
                out += &format!(
                    "C{} {} {} {} {} {}",
                    n(a.x),
                    n(a.y),
                    n(b.x),
                    n(b.y),
                    n(p.x),
                    n(p.y)
                )
            }
            Command::Close => out += "Z",
        }
    }
    out
}

pub fn svg(badge: Badge) -> String {
    let (width, pieces) = pieces(badge);
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n<title>pfx {label}</title>\n",
        w = format!("{width:.3}")
            .trim_end_matches('0')
            .trim_end_matches('.'),
        h = HEIGHT,
        label = badge.label().replace('&', "&amp;"),
    );
    for piece in pieces {
        let rule = if piece.ring {
            " fill-rule=\"evenodd\""
        } else {
            ""
        };
        out += &format!(
            "<path d=\"{}\" fill=\"{}\"{rule}/>\n",
            d(&piece.path),
            hex(piece.colour)
        );
    }
    out += "</svg>\n";
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_badge_master_reads_as_the_asset_renderer_reads_a_logo() {
        for badge in Badge::ALL {
            let drawing = pfx_geom::svg::Drawing::parse(&svg(badge), 2e-4).unwrap();
            assert_eq!(drawing.view[3], f64::from(HEIGHT), "{badge:?}");
            assert!(drawing.view[2] > drawing.view[3], "{badge:?}");
            assert!(drawing.elements.len() > badge.label().len(), "{badge:?}");
            assert!(drawing.elements.iter().all(|element| !element.ground));
        }
    }

    #[test]
    fn the_tracked_render_masters_are_the_badges_the_window_draws() {
        let masters = [
            Badge::Free,
            Badge::Indie,
            Badge::Pro,
            Badge::Owner,
            Badge::FriendsFamily,
        ];
        for badge in masters {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../../render/badges/logos/badge-{}.svg",
                badge.name()
            ));
            let master = std::fs::read_to_string(&path).unwrap();
            assert!(
                svg(badge) == master,
                "render/badges/logos/badge-{}.svg",
                badge.name()
            );
        }
    }
}
