pub mod font;
pub mod keycap;
pub mod shape;

use std::collections::{BTreeMap, BTreeSet};

use pfx_text::{Atlas, Baseline, Error, Representation, Style, TextEngine};
use sha2::{Digest, Sha256};

use crate::device::{Button, Control, Key, MouseButton, Stick, Wheel};
use crate::family::{Family, GlyphStyle};
use shape::{Contour, Dir, Shape};

pub const ATLAS_DIGEST: &str = "8a1f27a3b01ad6cb8ef376d2b373d528411f03db37f86c72a165e44cc21e587c";
pub const BOX: f32 = 48.0;
pub const PPEM: f32 = 64.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mark {
    Text(&'static str),
    Cross,
    Circle,
    Square,
    Triangle,
    Spot(Dir),
    Bars,
    Windows,
    Rays,
    Plus,
    Minus,
    Forward,
    Back,
    Home,
    Share,
    Capture,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MouseMark {
    Button(MouseButton),
    Wheel(Wheel),
    Move,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Glyph {
    Disc(Mark),
    Pill(Mark),
    Stick(&'static str),
    Bumper(&'static str),
    Trigger(&'static str),
    Grip(&'static str),
    DPad(Option<Dir>),
    Touchpad,
    Keycap(u16),
    Arrow(Dir),
    Mouse(MouseMark),
}

pub fn glyph(family: Family, control: Control) -> Glyph {
    match control {
        Control::Key(key) => key_glyph(key),
        Control::Mouse(button) => Glyph::Mouse(MouseMark::Button(button)),
        Control::Wheel(wheel) => Glyph::Mouse(MouseMark::Wheel(wheel)),
        Control::MouseMove => Glyph::Mouse(MouseMark::Move),
        Control::Stick(stick) => Glyph::Stick(match stick {
            Stick::Left => "L",
            Stick::Right => "R",
        }),
        Control::DPad => Glyph::DPad(None),
        Control::Button(button) => button_glyph(family, button),
    }
}

pub fn glyph_for(style: GlyphStyle, control: Control) -> Glyph {
    glyph(style.family(), control)
}

fn key_glyph(key: Key) -> Glyph {
    match key {
        Key::Up => Glyph::Arrow(Dir::Up),
        Key::Down => Glyph::Arrow(Dir::Down),
        Key::Left => Glyph::Arrow(Dir::Left),
        Key::Right => Glyph::Arrow(Dir::Right),
        _ => keycap::keycap_for(keycap::estimate(key.label())),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    Letters,
    Shapes,
    Nintendo,
    Positions,
}

fn look(family: Family) -> Look {
    match family {
        Family::DualSense | Family::DualShock4 => Look::Shapes,
        Family::SwitchPro => Look::Nintendo,
        Family::Generic => Look::Positions,
        Family::Xbox | Family::SteamController | Family::SteamDeck => Look::Letters,
    }
}

fn numbered(family: Family) -> bool {
    matches!(
        family,
        Family::DualSense | Family::DualShock4 | Family::SteamDeck
    )
}

fn button_glyph(family: Family, button: Button) -> Glyph {
    let look = look(family);
    let face = |letters: &'static str, shape: Mark, nintendo: &'static str, spot: Dir| match look {
        Look::Letters => Glyph::Disc(Mark::Text(letters)),
        Look::Shapes => Glyph::Disc(shape),
        Look::Nintendo => Glyph::Disc(Mark::Text(nintendo)),
        Look::Positions => Glyph::Disc(Mark::Spot(spot)),
    };
    let numbered = numbered(family);
    match button {
        Button::South => face("A", Mark::Cross, "B", Dir::Down),
        Button::East => face("B", Mark::Circle, "A", Dir::Right),
        Button::West => face("X", Mark::Square, "Y", Dir::Left),
        Button::North => face("Y", Mark::Triangle, "X", Dir::Up),
        Button::LeftBumper => Glyph::Bumper(match family {
            Family::SwitchPro => "L",
            _ if numbered => "L1",
            _ => "LB",
        }),
        Button::RightBumper => Glyph::Bumper(match family {
            Family::SwitchPro => "R",
            _ if numbered => "R1",
            _ => "RB",
        }),
        Button::LeftTrigger => Glyph::Trigger(match family {
            Family::SwitchPro => "ZL",
            _ if numbered => "L2",
            _ => "LT",
        }),
        Button::RightTrigger => Glyph::Trigger(match family {
            Family::SwitchPro => "ZR",
            _ if numbered => "R2",
            _ => "RT",
        }),
        Button::LeftStick => Glyph::Disc(Mark::Text(if numbered { "L3" } else { "LS" })),
        Button::RightStick => Glyph::Disc(Mark::Text(if numbered { "R3" } else { "RS" })),
        Button::DPadUp => Glyph::DPad(Some(Dir::Up)),
        Button::DPadDown => Glyph::DPad(Some(Dir::Down)),
        Button::DPadLeft => Glyph::DPad(Some(Dir::Left)),
        Button::DPadRight => Glyph::DPad(Some(Dir::Right)),
        Button::Menu => match family {
            Family::DualSense | Family::DualShock4 => Glyph::Pill(Mark::Bars),
            Family::SwitchPro => Glyph::Disc(Mark::Plus),
            Family::SteamController => Glyph::Disc(Mark::Forward),
            Family::Xbox | Family::SteamDeck | Family::Generic => Glyph::Disc(Mark::Bars),
        },
        Button::View => match family {
            Family::DualSense | Family::DualShock4 => Glyph::Pill(Mark::Rays),
            Family::SwitchPro => Glyph::Disc(Mark::Minus),
            Family::SteamController => Glyph::Disc(Mark::Back),
            Family::Xbox | Family::SteamDeck | Family::Generic => Glyph::Disc(Mark::Windows),
        },
        Button::Guide => Glyph::Disc(Mark::Home),
        Button::Touchpad => Glyph::Touchpad,
        Button::LeftGrip => Glyph::Grip(grip(family, 0)),
        Button::RightGrip => Glyph::Grip(grip(family, 1)),
        Button::LeftGripLower => Glyph::Grip(grip(family, 2)),
        Button::RightGripLower => Glyph::Grip(grip(family, 3)),
        Button::Misc => match family {
            Family::Xbox => Glyph::Disc(Mark::Share),
            Family::SwitchPro => Glyph::Disc(Mark::Capture),
            _ => Glyph::Disc(Mark::Text("M")),
        },
    }
}

fn grip(family: Family, index: usize) -> &'static str {
    match family {
        Family::SteamController => ["LG", "RG", "LG", "RG"][index],
        Family::Xbox => ["P3", "P1", "P4", "P2"][index],
        _ => ["L4", "R4", "L5", "R5"][index],
    }
}

pub fn catalogue() -> Vec<Glyph> {
    let mut set = BTreeSet::new();
    for family in Family::ALL {
        for button in Button::ALL {
            set.insert(glyph(family, Control::Button(button)));
        }
        for stick in [Stick::Left, Stick::Right] {
            set.insert(glyph(family, Control::Stick(stick)));
        }
        set.insert(glyph(family, Control::DPad));
    }
    for key in Key::ALL {
        set.insert(glyph(Family::Generic, Control::Key(key)));
    }
    for width in keycap::KEYCAP_WIDTHS {
        set.insert(Glyph::Keycap(width));
    }
    for button in [
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
        MouseButton::Back,
        MouseButton::Forward,
    ] {
        set.insert(glyph(Family::Generic, Control::Mouse(button)));
    }
    for wheel in [Wheel::Up, Wheel::Down, Wheel::Left, Wheel::Right] {
        set.insert(glyph(Family::Generic, Control::Wheel(wheel)));
    }
    set.insert(glyph(Family::Generic, Control::MouseMove));
    set.into_iter().collect()
}

const R: f32 = 22.0;
const C: f32 = BOX / 2.0;

fn mark(mark: Mark, cx: f32, cy: f32, size: f32) -> Vec<Contour> {
    let s = size;
    match mark {
        Mark::Text(text) => {
            let cells = shape::label_width(text, 1.0);
            let max_w = if cells > 5.0 { s * 1.5 } else { s * 1.1 };
            shape::label(text, cx, cy, max_w, s * 1.15, s / 4.5)
        }
        Mark::Cross => vec![shape::cross(cx, cy, s * 0.62, s * 0.11)],
        Mark::Circle => vec![
            shape::circle(cx, cy, s * 0.5),
            shape::circle(cx, cy, s * 0.5 - s * 0.16),
        ],
        Mark::Square => vec![
            shape::rect(cx - s * 0.45, cy - s * 0.45, s * 0.9, s * 0.9, 0.0),
            shape::rect(
                cx - s * 0.45 + s * 0.15,
                cy - s * 0.45 + s * 0.15,
                s * 0.9 - s * 0.3,
                s * 0.9 - s * 0.3,
                0.0,
            ),
        ],
        Mark::Triangle => vec![
            shape::triangle(cx, cy + s * 0.08, s * 0.6, Dir::Up),
            shape::triangle(cx, cy + s * 0.08, s * 0.6 - s * 0.3, Dir::Up),
        ],
        Mark::Spot(dir) => {
            let d = s * 0.42;
            let r = s * 0.17;
            let mut contours = Vec::new();
            for (spot, dx, dy) in [
                (Dir::Up, 0.0, -d),
                (Dir::Down, 0.0, d),
                (Dir::Left, -d, 0.0),
                (Dir::Right, d, 0.0),
            ] {
                contours.push(shape::circle(cx + dx, cy + dy, r));
                if spot != dir {
                    contours.push(shape::circle(cx + dx, cy + dy, r * 0.45));
                }
            }
            contours
        }
        Mark::Bars => (0..3)
            .map(|i| {
                let y = cy - s * 0.32 + i as f32 * s * 0.32;
                shape::rect(cx - s * 0.42, y - s * 0.07, s * 0.84, s * 0.14, s * 0.07)
            })
            .collect(),
        Mark::Windows => {
            let w = s * 0.6;
            let t = s * 0.13;
            let back_x = cx - s * 0.5;
            let back_y = cy - s * 0.5;
            let front_x = cx - s * 0.1;
            let front_y = cy - s * 0.1;
            vec![
                shape::polygon(&[
                    [back_x, back_y],
                    [back_x + w, back_y],
                    [back_x + w, front_y - t * 0.8],
                    [back_x + w - t, front_y - t * 0.8],
                    [back_x + w - t, back_y + t],
                    [back_x + t, back_y + t],
                    [back_x + t, back_y + w - t],
                    [front_x - t * 0.8, back_y + w - t],
                    [front_x - t * 0.8, back_y + w],
                    [back_x, back_y + w],
                ]),
                shape::rect(front_x, front_y, w, w, 0.0),
                shape::rect(front_x + t, front_y + t, w - 2.0 * t, w - 2.0 * t, 0.0),
            ]
        }
        Mark::Rays => (0..3)
            .map(|i| {
                let x = cx - s * 0.3 + i as f32 * s * 0.3;
                let tall = if i == 1 { s * 0.7 } else { s * 0.5 };
                shape::rect(x - s * 0.07, cy - tall / 2.0, s * 0.14, tall, s * 0.07)
            })
            .collect(),
        Mark::Plus => vec![shape::plus(cx, cy, s * 0.5, s * 0.1)],
        Mark::Minus => vec![shape::rect(cx - s * 0.5, cy - s * 0.1, s, s * 0.2, 0.0)],
        Mark::Forward => vec![shape::triangle(cx + s * 0.08, cy, s * 0.48, Dir::Right)],
        Mark::Back => vec![shape::triangle(cx - s * 0.08, cy, s * 0.48, Dir::Left)],
        Mark::Home => vec![shape::polygon(&[
            [cx, cy - s * 0.5],
            [cx + s * 0.5, cy - s * 0.05],
            [cx + s * 0.34, cy - s * 0.05],
            [cx + s * 0.34, cy + s * 0.45],
            [cx - s * 0.34, cy + s * 0.45],
            [cx - s * 0.34, cy - s * 0.05],
            [cx - s * 0.5, cy - s * 0.05],
        ])],
        Mark::Share => vec![shape::polygon(&[
            [cx, cy - s * 0.5],
            [cx + s * 0.4, cy - s * 0.1],
            [cx + s * 0.12, cy - s * 0.1],
            [cx + s * 0.12, cy + s * 0.45],
            [cx - s * 0.12, cy + s * 0.45],
            [cx - s * 0.12, cy - s * 0.1],
            [cx - s * 0.4, cy - s * 0.1],
        ])],
        Mark::Capture => vec![
            shape::rect(cx - s * 0.45, cy - s * 0.45, s * 0.9, s * 0.9, s * 0.12),
            shape::circle(cx, cy, s * 0.25),
        ],
    }
}

fn arrow(cx: f32, cy: f32, size: f32, dir: Dir) -> Contour {
    let (w, h) = (size * 0.5, size * 0.42);
    let points: [[f32; 2]; 7] = [
        [0.0, -size * 0.5],
        [w, -size * 0.5 + h],
        [w * 0.38, -size * 0.5 + h],
        [w * 0.38, size * 0.5],
        [-w * 0.38, size * 0.5],
        [-w * 0.38, -size * 0.5 + h],
        [-w, -size * 0.5 + h],
    ];
    let turn = |p: [f32; 2]| match dir {
        Dir::Up => [cx + p[0], cy + p[1]],
        Dir::Down => [cx - p[0], cy - p[1]],
        Dir::Left => [cx + p[1], cy - p[0]],
        Dir::Right => [cx - p[1], cy + p[0]],
    };
    shape::polygon(&points.map(turn))
}

pub fn shape_of(glyph: Glyph) -> Shape {
    draw(glyph).oriented()
}

fn draw(glyph: Glyph) -> Shape {
    match glyph {
        Glyph::Disc(inner) => {
            let mut shape = Shape::new(BOX, BOX);
            shape.add(shape::circle(C, C, R));
            shape.extend(mark(inner, C, C, 22.0));
            shape
        }
        Glyph::Pill(inner) => {
            let mut shape = Shape::new(BOX, BOX);
            shape.add(shape::pill(C, C, 44.0, 28.0));
            shape.extend(mark(inner, C, C, 17.0));
            shape
        }
        Glyph::Stick(text) => {
            let mut shape = Shape::new(BOX, BOX);
            shape.add(shape::circle(C, C, R));
            shape.add(shape::circle(C, C, R - 4.5));
            shape.extend(shape::label(text, C, C, 14.0, 18.0, 3.0));
            shape
        }
        Glyph::Bumper(text) => {
            let width = 64.0;
            let mut shape = Shape::new(width, BOX);
            shape.add(shape::rounded(
                2.0,
                10.0,
                width - 4.0,
                28.0,
                [14.0, 14.0, 5.0, 5.0],
            ));
            shape.extend(shape::label(text, width / 2.0, 24.0, 40.0, 17.0, 2.6));
            shape
        }
        Glyph::Trigger(text) => {
            let mut shape = Shape::new(BOX, BOX);
            shape.add(shape::rounded(5.0, 2.0, 38.0, 44.0, [17.0, 17.0, 5.0, 5.0]));
            shape.extend(shape::label(text, C, 26.0, 28.0, 18.0, 2.6));
            shape
        }
        Glyph::Grip(text) => {
            let width = 56.0;
            let mut shape = Shape::new(width, BOX);
            shape.add(shape::polygon(&[
                [8.0, 8.0],
                [width - 2.0, 8.0],
                [width - 2.0, 40.0],
                [8.0, 40.0],
                [2.0, 24.0],
            ]));
            shape.extend(shape::label(text, width / 2.0 + 3.0, 24.0, 36.0, 18.0, 2.6));
            shape
        }
        Glyph::DPad(dir) => {
            let mut shape = Shape::new(BOX, BOX);
            shape.add(shape::plus(C, C, 22.0, 8.0));
            match dir {
                Some(dir) => {
                    let (dx, dy) = match dir {
                        Dir::Up => (0.0, -13.0),
                        Dir::Down => (0.0, 13.0),
                        Dir::Left => (-13.0, 0.0),
                        Dir::Right => (13.0, 0.0),
                    };
                    shape.add(shape::triangle(C + dx, C + dy, 5.0, dir));
                }
                None => {
                    shape.add(shape::circle(C, C, 4.0));
                }
            }
            shape
        }
        Glyph::Touchpad => {
            let width = 64.0;
            let mut shape = Shape::new(width, BOX);
            shape.add(shape::rect(2.0, 8.0, width - 4.0, 32.0, 8.0));
            shape.add(shape::rect(6.0, 12.0, width - 12.0, 24.0, 5.0));
            shape.add(shape::circle(width / 2.0, 24.0, 5.0));
            shape
        }
        Glyph::Keycap(width) => {
            let width = f32::from(width);
            let mut shape = Shape::new(width, BOX);
            shape.add(shape::rect(2.0, 2.0, width - 4.0, BOX - 4.0, 8.0));
            shape.add(shape::rect(6.0, 6.0, width - 12.0, BOX - 12.0, 5.0));
            shape
        }
        Glyph::Arrow(dir) => {
            let mut shape = Shape::new(BOX, BOX);
            shape.add(shape::rect(2.0, 2.0, BOX - 4.0, BOX - 4.0, 8.0));
            shape.add(shape::rect(6.0, 6.0, BOX - 12.0, BOX - 12.0, 5.0));
            shape.add(arrow(C, C, 24.0, dir));
            shape
        }
        Glyph::Mouse(inner) => mouse(inner),
    }
}

fn mouse(inner: MouseMark) -> Shape {
    let width = BOX;
    let (x, y, w, h) = (7.0, 2.0, 34.0, 44.0);
    let t = 3.5;
    let radius = 11.0;
    let mut shape = Shape::new(width, BOX);
    shape.add(shape::rect(x, y, w, h, radius));
    shape.add(shape::rect(
        x + t,
        y + t,
        w - 2.0 * t,
        h - 2.0 * t,
        radius - t,
    ));
    let gap = 2.5;
    let (ix, iy, iw) = (x + t + gap, y + t + gap, w - 2.0 * t - 2.0 * gap);
    let split = y + 19.0;
    let half = iw / 2.0 - gap / 2.0;
    let wheel = |shape: &mut Shape| {
        shape.add(shape::pill(x + w / 2.0, y + 13.0, 4.0, 9.0));
    };
    match inner {
        MouseMark::Button(MouseButton::Left) => {
            shape.add(shape::rounded(
                ix,
                iy,
                half,
                split - iy,
                [radius - t - gap, 0.0, 0.0, 0.0],
            ));
        }
        MouseMark::Button(MouseButton::Right) => {
            shape.add(shape::rounded(
                ix + half + gap,
                iy,
                half,
                split - iy,
                [0.0, radius - t - gap, 0.0, 0.0],
            ));
        }
        MouseMark::Button(MouseButton::Middle) => wheel(&mut shape),
        MouseMark::Button(MouseButton::Back) => {
            shape.add(shape::triangle(x + w / 2.0 - 1.0, y + 31.0, 6.0, Dir::Left));
        }
        MouseMark::Button(MouseButton::Forward) => {
            shape.add(shape::triangle(
                x + w / 2.0 + 1.0,
                y + 31.0,
                6.0,
                Dir::Right,
            ));
        }
        MouseMark::Wheel(dir) => {
            wheel(&mut shape);
            let (cx, cy, dir) = match dir {
                Wheel::Up => (x + w / 2.0, y + 28.0, Dir::Up),
                Wheel::Down => (x + w / 2.0, y + 30.0, Dir::Down),
                Wheel::Left => (x + w / 2.0 - 1.0, y + 31.0, Dir::Left),
                Wheel::Right => (x + w / 2.0 + 1.0, y + 31.0, Dir::Right),
            };
            shape.add(shape::triangle(cx, cy, 5.0, dir));
        }
        MouseMark::Move => {
            let cx = x + w / 2.0;
            let cy = y + 29.0;
            for (dx, dy, dir) in [
                (0.0, -6.5, Dir::Up),
                (0.0, 6.5, Dir::Down),
                (-6.0, 0.0, Dir::Left),
                (6.0, 0.0, Dir::Right),
            ] {
                shape.add(shape::triangle(cx + dx, cy + dy, 3.4, dir));
            }
        }
    }
    shape
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphCell {
    pub rect: [u32; 4],
    pub uv: [f32; 4],
    pub offset: [f32; 2],
    pub size: [f32; 2],
    pub box_size: [f32; 2],
}

const TILE: f32 = 80.0;

fn stretched(base: &GlyphCell, width: f32, from: f32) -> Vec<GlyphCell> {
    let [ox, oy] = base.offset;
    let [sx, sy] = base.size;
    let [u0, v0, u1, v1] = base.uv;
    let u = |x: f32| u0 + (x - ox) / sx * (u1 - u0);
    let px = |x: f32| base.rect[0] as f32 + (x - ox) / sx * base.rect[2] as f32;
    let piece = |at: f32, from: f32, to: f32| {
        let left = px(from).round().max(0.0) as u32;
        let right = px(to).round().max(0.0) as u32;
        GlyphCell {
            rect: [left, base.rect[1], right.saturating_sub(left), base.rect[3]],
            uv: [u(from), v0, u(to), v1],
            offset: [at, oy],
            size: [to - from, sy],
            box_size: [width, BOX],
        }
    };
    let middle = from / 2.0;
    let shift = width - from;
    let mut pieces = vec![piece(ox, ox, middle)];
    let mut at = middle;
    while at < middle + shift {
        let span = TILE.min(middle + shift - at);
        let start = middle - TILE / 2.0;
        pieces.push(piece(at, start, start + span));
        at += span;
    }
    pieces.push(piece(middle + shift, middle, ox + sx));
    pieces
}

#[derive(Clone, Debug)]
pub struct GlyphAtlas {
    pub atlas: Atlas,
    cells: BTreeMap<Glyph, GlyphCell>,
}

impl GlyphAtlas {
    pub fn build() -> Result<Self, Error> {
        let glyphs = catalogue();
        let shapes: Vec<Shape> = glyphs.iter().map(|glyph| shape_of(*glyph)).collect();
        let bytes = font::build(&shapes);
        let mut engine = TextEngine::new(&bytes)?;
        let text: String = (0..glyphs.len() as u32)
            .filter_map(|index| char::from_u32(font::FIRST_CHAR + index))
            .collect();
        let paragraph = engine.layout(
            &text,
            Style {
                family: font::FAMILY,
                size: PPEM,
                line_height: PPEM * 1.25,
                wrap_width: None,
                pixels_per_unit: 1.0,
                representation: Representation::Msdf,
            },
            &Baseline::Straight {
                origin: [0.0, 0.0],
                direction: [1.0, 0.0],
            },
        )?;
        let mut cells = BTreeMap::new();
        for instance in &paragraph.glyphs {
            let Some(index) = usize::from(instance.glyph_id).checked_sub(1) else {
                continue;
            };
            let Some(glyph) = glyphs.get(index) else {
                continue;
            };
            let shape = &shapes[index];
            cells.insert(
                *glyph,
                GlyphCell {
                    rect: instance.atlas_rect,
                    uv: instance.uv,
                    offset: [
                        instance.local_rect[0],
                        instance.local_rect[1] + shape.height,
                    ],
                    size: [instance.local_rect[2], instance.local_rect[3]],
                    box_size: [shape.width, shape.height],
                },
            );
        }
        if cells.len() != glyphs.len() {
            return Err(Error::UnsupportedGlyph);
        }
        Ok(Self {
            atlas: paragraph.atlas,
            cells,
        })
    }

    pub fn get(&self, glyph: Glyph) -> Option<&GlyphCell> {
        self.cells.get(&glyph)
    }

    pub fn pieces(&self, glyph: Glyph) -> Option<Vec<GlyphCell>> {
        if let Some(cell) = self.cells.get(&glyph) {
            return Some(vec![*cell]);
        }
        let Glyph::Keycap(width) = glyph else {
            return None;
        };
        let widest = keycap::widest();
        if width < widest {
            return None;
        }
        let base = self.cells.get(&Glyph::Keycap(widest))?;
        Some(stretched(base, f32::from(width), f32::from(widest)))
    }

    pub fn cell(&self, family: Family, control: Control) -> Option<&GlyphCell> {
        self.get(glyph(family, control))
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Glyph, &GlyphCell)> {
        self.cells.iter()
    }

    pub fn digest(&self) -> String {
        let mut hash = Sha256::new();
        let level = &self.atlas.levels[0];
        hash.update(self.atlas.channels.to_le_bytes());
        hash.update(level.width.to_le_bytes());
        hash.update(level.height.to_le_bytes());
        hash.update(&level.bytes);
        for cell in self.cells.values() {
            for value in cell.rect {
                hash.update(value.to_le_bytes());
            }
            for value in cell.offset.iter().chain(&cell.size).chain(&cell.box_size) {
                hash.update(value.to_bits().to_le_bytes());
            }
        }
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}
