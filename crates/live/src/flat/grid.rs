use super::{Draw, Fill, IDENTITY, Order, Shadow, Shape, Srgba, Step};
use crate::text::RichQuad;
use pfx_text::{Anchor, Face, Representation, RichParagraph, Span, TextEngine};
use std::collections::BTreeMap;
use std::f32::consts::{FRAC_PI_2, PI};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub origin: [f32; 2],
    pub cell: [f32; 2],
    pub columns: u32,
    pub rows: u32,
}

impl Grid {
    pub fn new(origin: [f32; 2], cell: [f32; 2], columns: u32, rows: u32) -> Result<Self, String> {
        if !(cell[0] > 0.0 && cell[1] > 0.0 && cell[0].is_finite() && cell[1].is_finite()) {
            return Err("a grid cell must be finite and positive".into());
        }
        if !(origin[0].is_finite() && origin[1].is_finite()) {
            return Err("the grid origin must be finite".into());
        }
        if columns == 0 || rows == 0 {
            return Err("a grid needs at least one column and one row".into());
        }
        Ok(Self {
            origin,
            cell,
            columns,
            rows,
        })
    }

    pub fn cover(layout: [f32; 2], cell: [f32; 2]) -> Result<Self, String> {
        if !(cell[0] > 0.0 && cell[1] > 0.0) {
            return Err("a grid cell must be finite and positive".into());
        }
        let columns = (layout[0] / cell[0]).floor().max(0.0) as u32;
        let rows = (layout[1] / cell[1]).floor().max(0.0) as u32;
        let origin = [
            (layout[0] - columns as f32 * cell[0]) * 0.5,
            (layout[1] - rows as f32 * cell[1]) * 0.5,
        ];
        Self::new(origin, cell, columns, rows)
    }

    pub fn measure(engine: &mut TextEngine, face: &Face) -> Result<[f32; 2], String> {
        const SAMPLE: usize = 16;
        let block = engine
            .layout_spans(
                &[Span {
                    text: "0".repeat(SAMPLE),
                    face: face.clone(),
                    color: [1.0; 4],
                }],
                None,
                1.0,
                Anchor::Start,
            )
            .map_err(|error| format!("the grid font could not be measured: {error:?}"))?;
        let advance = block.width / SAMPLE as f32;
        if advance.is_nan() || advance <= 0.0 {
            return Err("the grid font has no advance".into());
        }
        Ok([advance, face.line])
    }

    pub fn cell_at(&self, point: [f32; 2]) -> Option<[u32; 2]> {
        let c = ((point[0] - self.origin[0]) / self.cell[0]).floor();
        let r = ((point[1] - self.origin[1]) / self.cell[1]).floor();
        (c >= 0.0 && r >= 0.0 && c < self.columns as f32 && r < self.rows as f32)
            .then_some([c as u32, r as u32])
    }

    pub fn snap(&self, point: [f32; 2]) -> [i64; 2] {
        [
            ((point[0] - self.origin[0]) / self.cell[0]).round() as i64,
            ((point[1] - self.origin[1]) / self.cell[1]).round() as i64,
        ]
    }

    pub fn corner(&self, cell: [u32; 2]) -> [f32; 2] {
        [
            self.origin[0] + cell[0] as f32 * self.cell[0],
            self.origin[1] + cell[1] as f32 * self.cell[1],
        ]
    }

    fn span(&self, low: f32, high: f32, axis: usize) -> (i64, i64) {
        let first = ((low - self.origin[axis]) / self.cell[axis] - 0.5).ceil() as i64;
        let last = ((high - self.origin[axis]) / self.cell[axis] - 0.5).floor() as i64;
        let count = i64::from(if axis == 0 { self.columns } else { self.rows });
        (first.max(0), last.min(count - 1))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Weight {
    #[default]
    Light,
    Heavy,
    Double,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interior {
    Blank,
    #[default]
    Background,
    Shade,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rules {
    pub line: Weight,
    pub raised: Option<(f32, Weight)>,
    pub rounded: f32,
    pub interior: Interior,
    pub dot: char,
    pub ink: Option<Srgba>,
    pub ground: Option<Srgba>,
    pub thickness: f32,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            line: Weight::Light,
            raised: None,
            rounded: f32::INFINITY,
            interior: Interior::Background,
            dot: '•',
            ink: None,
            ground: None,
            thickness: 0.12,
        }
    }
}

pub const UP: usize = 0;
pub const RIGHT: usize = 1;
pub const DOWN: usize = 2;
pub const LEFT: usize = 3;

pub type Arms = [Option<Weight>; 4];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Glyph {
    #[default]
    Empty,
    Char(char),
    Lines {
        arms: Arms,
        rounded: bool,
    },
    Shade(u8),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub glyph: Glyph,
    pub ink: Srgba,
    pub ground: Option<Srgba>,
    pub id: u32,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            glyph: Glyph::Empty,
            ink: Srgba::WHITE,
            ground: None,
            id: 0,
        }
    }
}

const LIGHT: [char; 16] = [
    ' ', '╵', '╶', '└', '╷', '│', '┌', '├', '╴', '┘', '─', '┴', '┐', '┤', '┬', '┼',
];
const HEAVY: [char; 16] = [
    ' ', '╹', '╺', '┗', '╻', '┃', '┏', '┣', '╸', '┛', '━', '┻', '┓', '┫', '┳', '╋',
];
const DOUBLE: [char; 16] = [
    ' ', '║', '═', '╚', '║', '║', '╔', '╠', '═', '╝', '═', '╩', '╗', '╣', '╦', '╬',
];
const SHADES: [char; 4] = ['░', '▒', '▓', '█'];

type Run = (i64, i64, [u32; 4], u32);

fn mask(arms: &Arms) -> usize {
    arms.iter()
        .enumerate()
        .filter(|(_, arm)| arm.is_some())
        .map(|(index, _)| 1 << index)
        .sum()
}

fn corner_mask(mask: usize) -> bool {
    matches!(mask, 3 | 6 | 9 | 12)
}

pub fn box_char(arms: &Arms, rounded: bool) -> char {
    let mask = mask(arms);
    let weight = arms.iter().flatten().max().copied();
    match weight {
        None => ' ',
        Some(Weight::Light) if rounded => match mask {
            6 => '╭',
            12 => '╮',
            9 => '╯',
            3 => '╰',
            _ => LIGHT[mask],
        },
        Some(Weight::Light) => LIGHT[mask],
        Some(Weight::Heavy) => HEAVY[mask],
        Some(Weight::Double) => DOUBLE[mask],
    }
}

impl Glyph {
    pub fn char(&self) -> char {
        match *self {
            Glyph::Empty => ' ',
            Glyph::Char(c) => c,
            Glyph::Lines { arms, rounded } => box_char(&arms, rounded),
            Glyph::Shade(level) => SHADES[usize::from(level.clamp(1, 4)) - 1],
        }
    }
}

pub struct CharGrid {
    pub grid: Grid,
    pub rules: Rules,
    cells: Vec<Cell>,
}

#[derive(Clone, Copy)]
struct Ink {
    colour: Srgba,
    id: u32,
}

fn fill_colour(fill: Fill) -> Option<Srgba> {
    match fill {
        Fill::None => None,
        Fill::Solid(colour) => Some(colour),
        Fill::Vertical(top, bottom) => Some(if top.0[3] >= bottom.0[3] { top } else { bottom }),
        Fill::Gradient(gradient) => gradient
            .stops()
            .map(|(_, colour)| Srgba(colour))
            .reduce(|kept, next| if next.0[3] > kept.0[3] { next } else { kept }),
        Fill::Image(image) => Some(image.tint),
    }
}

fn key(colour: Srgba) -> [u32; 4] {
    colour.0.map(f32::to_bits)
}

impl CharGrid {
    pub fn new(grid: Grid, rules: Rules) -> Self {
        Self {
            grid,
            rules,
            cells: vec![Cell::default(); (grid.columns * grid.rows) as usize],
        }
    }

    pub fn clear(&mut self) {
        self.cells.fill(Cell::default());
    }

    fn index(&self, c: i64, r: i64) -> Option<usize> {
        (c >= 0 && r >= 0 && c < i64::from(self.grid.columns) && r < i64::from(self.grid.rows))
            .then(|| (r * i64::from(self.grid.columns) + c) as usize)
    }

    pub fn cell(&self, c: u32, r: u32) -> Option<&Cell> {
        self.index(i64::from(c), i64::from(r))
            .map(|index| &self.cells[index])
    }

    pub fn put(&mut self, cell: [u32; 2], glyph: char, ink: Srgba, id: u32) {
        if let Some(index) = self.index(i64::from(cell[0]), i64::from(cell[1])) {
            let ink = self.rules.ink.unwrap_or(ink);
            let target = &mut self.cells[index];
            target.glyph = if glyph == ' ' {
                Glyph::Empty
            } else {
                Glyph::Char(glyph)
            };
            target.ink = ink;
            target.id = id;
        }
    }

    pub fn text(&mut self, at: [f32; 2], text: &str, ink: Srgba, id: u32) {
        let [start, mut r] = self.grid.snap(at);
        let mut c = start;
        for ch in text.chars() {
            if ch == '\n' {
                r += 1;
                c = start;
                continue;
            }
            if c >= 0 && r >= 0 {
                self.put([c as u32, r as u32], ch, ink, id);
            }
            c += 1;
        }
    }

    fn arms(&mut self, c: i64, r: i64, arms: Arms, rounded: bool, ink: Ink, merge: bool) {
        let Some(index) = self.index(c, r) else {
            return;
        };
        let cell = &mut self.cells[index];
        let (arms, rounded) = match (merge, cell.glyph) {
            (
                true,
                Glyph::Lines {
                    arms: old,
                    rounded: old_rounded,
                },
            ) => {
                let merged: Arms = std::array::from_fn(|i| old[i].max(arms[i]));
                let same = mask(&old) == mask(&arms) || mask(&old) == 0;
                (
                    merged,
                    rounded && (same || old_rounded) && corner_mask(mask(&merged)),
                )
            }
            _ => (arms, rounded),
        };
        cell.glyph = Glyph::Lines { arms, rounded };
        cell.ink = ink.colour;
        cell.id = ink.id;
    }

    fn line(&mut self, fixed: i64, from: i64, to: i64, horizontal: bool, weight: Weight, ink: Ink) {
        for at in from..=to {
            let mut arms: Arms = [None; 4];
            let (back, forward) = if horizontal {
                (LEFT, RIGHT)
            } else {
                (UP, DOWN)
            };
            if at > from || from == to {
                arms[back] = Some(weight);
            }
            if at < to || from == to {
                arms[forward] = Some(weight);
            }
            let (c, r) = if horizontal { (at, fixed) } else { (fixed, at) };
            self.arms(c, r, arms, false, ink, true);
        }
    }

    pub fn shape(&mut self, draw: &Draw) {
        let opacity = draw.opacity.clamp(0.0, 1.0);
        if opacity.is_nan() || opacity <= 0.0 {
            return;
        }
        let extent = match draw.shape {
            Shape::Rect { half, .. } => half,
            Shape::Circle { radius } => [radius; 2],
            Shape::Ring { radius, width } | Shape::Arc { radius, width, .. } => {
                [radius + width * 0.5; 2]
            }
            Shape::Icon(_) | Shape::Text(_) => return,
        };
        let stroke = draw
            .stroke
            .filter(|stroke| stroke.width > 0.0 && stroke.colour.0[3] > 0.0)
            .map(|stroke| stroke.colour);
        let fill = fill_colour(draw.fill).filter(|colour| colour.0[3] > 0.0);
        let Some(colour) = stroke.or(fill) else {
            return;
        };
        let filled = fill.is_some_and(|fill| fill.0[3] * opacity >= 0.5)
            && !matches!(draw.shape, Shape::Ring { .. } | Shape::Arc { .. });
        let m = draw.transform;
        let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for corner in [[-1.0, -1.0], [1.0, -1.0], [-1.0, 1.0], [1.0, 1.0]] {
            let local = [corner[0] * extent[0], corner[1] * extent[1]];
            let p = [
                m[0][0] * local[0] + m[1][0] * local[1] + m[3][0],
                m[0][1] * local[0] + m[1][1] * local[1] + m[3][1],
            ];
            bounds = [
                bounds[0].min(p[0]),
                bounds[1].min(p[1]),
                bounds[2].max(p[0]),
                bounds[3].max(p[1]),
            ];
        }
        if !bounds.iter().all(|value| value.is_finite()) {
            return;
        }
        let ink = Ink {
            colour: self.rules.ink.unwrap_or(colour),
            id: draw.id,
        };
        let weight = match self.rules.raised {
            Some((elevation, weight)) if draw.elevation >= elevation => weight,
            _ => self.rules.line,
        };
        let (c0, c1) = self.grid.span(bounds[0], bounds[2], 0);
        let (r0, r1) = self.grid.span(bounds[1], bounds[3], 1);
        let centre = [(bounds[0] + bounds[2]) * 0.5, (bounds[1] + bounds[3]) * 0.5];
        let home = [
            ((centre[0] - self.grid.origin[0]) / self.grid.cell[0]).floor() as i64,
            ((centre[1] - self.grid.origin[1]) / self.grid.cell[1]).floor() as i64,
        ];
        let columns = c1 - c0 + 1;
        let rows = r1 - r0 + 1;
        if columns >= 2 && rows >= 2 {
            let scale = (m[0][0] * m[1][1] - m[1][0] * m[0][1]).abs().sqrt();
            let rounded = match draw.shape {
                Shape::Rect { radii, .. } => {
                    radii.iter().copied().fold(0.0, f32::max) * scale >= self.rules.rounded
                }
                _ => true,
            };
            let ground = fill.map(|fill| self.rules.ground.unwrap_or(fill));
            let shade = fill.map_or(0, |fill| {
                (fill.0[3] * opacity * 4.0).ceil().clamp(1.0, 4.0) as u8
            });
            let tone = Ink {
                colour: self.rules.ink.or(fill).unwrap_or(ink.colour),
                id: ink.id,
            };
            self.boxed(
                [c0, c1, r0, r1],
                weight,
                rounded,
                (filled, ground, shade, tone),
                ink,
            );
        } else if columns >= 2 {
            self.line(home[1], c0, c1, true, weight, ink);
        } else if rows >= 2 {
            self.line(home[0], r0, r1, false, weight, ink);
        } else if let Some(index) = self.index(home[0], home[1]) {
            let cell = &mut self.cells[index];
            cell.glyph = Glyph::Char(self.rules.dot);
            cell.ink = ink.colour;
            cell.id = ink.id;
        }
    }

    fn boxed(
        &mut self,
        [c0, c1, r0, r1]: [i64; 4],
        weight: Weight,
        rounded: bool,
        (filled, ground, level, tone): (bool, Option<Srgba>, u8, Ink),
        ink: Ink,
    ) {
        let interior = self.rules.interior;
        for r in r0..=r1 {
            for c in c0..=c1 {
                let top = r == r0;
                let bottom = r == r1;
                let left = c == c0;
                let right = c == c1;
                let edge = top || bottom || left || right;
                if filled && let Some(index) = self.index(c, r) {
                    let cell = &mut self.cells[index];
                    cell.ground = match interior {
                        Interior::Background => ground,
                        Interior::Blank | Interior::Shade => None,
                    };
                    cell.glyph = if interior == Interior::Shade && !edge {
                        Glyph::Shade(level)
                    } else {
                        Glyph::Empty
                    };
                    cell.ink = tone.colour;
                    cell.id = tone.id;
                }
                if !edge {
                    continue;
                }
                let mut arms: Arms = [None; 4];
                if (left || right) && !top {
                    arms[UP] = Some(weight);
                }
                if (left || right) && !bottom {
                    arms[DOWN] = Some(weight);
                }
                if (top || bottom) && !left {
                    arms[LEFT] = Some(weight);
                }
                if (top || bottom) && !right {
                    arms[RIGHT] = Some(weight);
                }
                let corner = (top || bottom) && (left || right);
                self.arms(c, r, arms, rounded && corner, ink, !filled);
            }
        }
    }

    pub fn map(
        &mut self,
        draws: &[Draw],
        groups: usize,
        labels: &[(&str, Srgba)],
    ) -> Result<(), String> {
        let mut order = Order::default();
        order.build(draws, groups)?;
        for step in &order.steps {
            let Step::Draw(index) = *step else {
                continue;
            };
            let draw = &draws[index];
            match draw.shape {
                Shape::Text(label) => {
                    if let Some(&(text, colour)) = labels.get(label)
                        && draw.opacity > 0.0
                    {
                        self.text(
                            [draw.transform[3][0], draw.transform[3][1]],
                            text,
                            colour,
                            draw.id,
                        );
                    }
                }
                Shape::Icon(_) => {}
                _ => self.shape(draw),
            }
        }
        Ok(())
    }

    pub fn char_at(&self, c: u32, r: u32) -> char {
        self.cell(c, r).map_or(' ', |cell| cell.glyph.char())
    }

    pub fn lines(&self) -> Vec<String> {
        (0..self.grid.rows)
            .map(|r| {
                (0..self.grid.columns)
                    .map(|c| self.char_at(c, r))
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    pub fn spans(&self, face: &Face) -> Vec<Span> {
        let mut spans: Vec<Span> = Vec::new();
        for r in 0..self.grid.rows {
            let row = &self.cells[(r * self.grid.columns) as usize..][..self.grid.columns as usize];
            let end = row
                .iter()
                .rposition(|cell| matches!(cell.glyph, Glyph::Char(_)))
                .map_or(0, |last| last + 1);
            let mut current: Option<Span> = None;
            for cell in &row[..end] {
                let (ch, colour) = match cell.glyph {
                    Glyph::Char(ch) => (ch, cell.ink.0),
                    _ => (' ', current.as_ref().map_or(cell.ink.0, |span| span.color)),
                };
                match current.as_mut() {
                    Some(span) if span.color == colour => span.text.push(ch),
                    _ => {
                        spans.extend(current.take());
                        current = Some(Span {
                            text: ch.to_string(),
                            face: face.clone(),
                            color: colour,
                        });
                    }
                }
            }
            let mut last = current.unwrap_or_else(|| Span {
                text: String::new(),
                face: face.clone(),
                color: [1.0; 4],
            });
            last.text.push('\n');
            spans.push(last);
        }
        spans
    }

    pub fn snap(&self, rich: &RichParagraph, face: &Face) -> Vec<RichQuad> {
        let [cw, ch] = self.grid.cell;
        rich.quads
            .iter()
            .map(|quad| {
                let column = (quad.pen[0] / cw).round();
                let row = quad.line_index as f32;
                let dx = self.grid.origin[0] + column * cw - quad.pen[0];
                let dy = self.grid.origin[1] + (row + 0.5) * (ch - face.line);
                let mut placed = RichQuad::from(quad);
                placed.rect[0] += dx;
                placed.rect[1] += dy;
                placed
            })
            .collect()
    }

    pub fn render(
        &self,
        engine: &mut TextEngine,
        face: &Face,
        representation: Representation,
    ) -> Result<(RichParagraph, Vec<RichQuad>), String> {
        let rich = engine
            .render_spans(
                &self.spans(face),
                None,
                1.0,
                Anchor::Start,
                representation,
                [0.0, 0.0],
                1.0,
                [-f32::MAX, -f32::MAX, f32::MAX, f32::MAX],
                [0.0; 3],
            )
            .map_err(|error| format!("the grid text could not be laid out: {error:?}"))?;
        let quads = self.snap(&rich, face);
        Ok((rich, quads))
    }

    pub fn has_text(&self) -> bool {
        self.cells
            .iter()
            .any(|cell| matches!(cell.glyph, Glyph::Char(_)))
    }

    pub fn draws(&self, text: Option<usize>, out: &mut Vec<Draw>) {
        self.ground(out);
        self.shades(out);
        self.strokes(out);
        if let Some(text) = text
            && self.has_text()
        {
            out.push(
                Draw::new(Shape::Text(text))
                    .transform(IDENTITY)
                    .shadow(Shadow::None),
            );
        }
    }

    fn rect(&self, columns: [i64; 2], rows: [i64; 2]) -> ([f32; 2], [f32; 2]) {
        let [cw, ch] = self.grid.cell;
        let x0 = self.grid.origin[0] + columns[0] as f32 * cw;
        let x1 = self.grid.origin[0] + (columns[1] + 1) as f32 * cw;
        let y0 = self.grid.origin[1] + rows[0] as f32 * ch;
        let y1 = self.grid.origin[1] + (rows[1] + 1) as f32 * ch;
        ([(x0 + x1) * 0.5, (y0 + y1) * 0.5], [x1 - x0, y1 - y0])
    }

    fn close(
        &self,
        open: &mut BTreeMap<Run, (i64, Srgba)>,
        keep: &[Run],
        last_row: i64,
        out: &mut Vec<Draw>,
    ) {
        let done = open
            .keys()
            .filter(|run| !keep.contains(run))
            .copied()
            .collect::<Vec<_>>();
        for run in done {
            if let Some((first_row, colour)) = open.remove(&run) {
                let (centre, size) = self.rect([run.0, run.1], [first_row, last_row]);
                out.push(
                    Draw::rect(centre, size, 0.0)
                        .fill(colour)
                        .shadow(Shadow::None)
                        .id(run.3),
                );
            }
        }
    }

    fn blocks(&self, of: impl Fn(&Cell) -> Option<(Srgba, u32)>, out: &mut Vec<Draw>) {
        let mut open: BTreeMap<Run, (i64, Srgba)> = BTreeMap::new();
        let mut runs: Vec<(Run, Srgba)> = Vec::new();
        for r in 0..i64::from(self.grid.rows) {
            runs.clear();
            for c in 0..i64::from(self.grid.columns) {
                let Some((colour, id)) = self.index(c, r).and_then(|index| of(&self.cells[index]))
                else {
                    continue;
                };
                match runs.last_mut() {
                    Some((run, _)) if run.1 == c - 1 && run.2 == key(colour) && run.3 == id => {
                        run.1 = c;
                    }
                    _ => runs.push(((c, c, key(colour), id), colour)),
                }
            }
            let current = runs.iter().map(|(run, _)| *run).collect::<Vec<_>>();
            self.close(&mut open, &current, r - 1, out);
            for &(run, colour) in &runs {
                open.entry(run).or_insert((r, colour));
            }
        }
        self.close(&mut open, &[], i64::from(self.grid.rows) - 1, out);
    }

    fn ground(&self, out: &mut Vec<Draw>) {
        self.blocks(|cell| cell.ground.map(|ground| (ground, cell.id)), out);
    }

    fn shades(&self, out: &mut Vec<Draw>) {
        self.blocks(
            |cell| match cell.glyph {
                Glyph::Shade(level) => Some((
                    cell.ink
                        .alpha(cell.ink.0[3] * f32::from(level.clamp(1, 4)) / 4.0),
                    cell.id,
                )),
                _ => None,
            },
            out,
        );
    }

    fn thickness(&self) -> f32 {
        (self.rules.thickness * self.grid.cell[0]).max(0.0)
    }

    fn arms_at(&self, c: i64, r: i64) -> Option<(Arms, bool)> {
        let index = self.index(c, r)?;
        match self.cells[index].glyph {
            Glyph::Lines { arms, rounded } => Some((arms, rounded)),
            _ => None,
        }
    }

    fn radius(&self) -> f32 {
        self.grid.cell[0].min(self.grid.cell[1]) * 0.5
    }

    fn end(
        &self,
        c: i64,
        r: i64,
        horizontal: bool,
        start: bool,
        offset: f32,
        weight: Weight,
    ) -> f32 {
        let Some((arms, rounded)) = self.arms_at(c, r) else {
            return 0.0;
        };
        let t = self.thickness();
        let g = t;
        let sign = if start { -1.0 } else { 1.0 };
        let (low, high) = if horizontal {
            (UP, DOWN)
        } else {
            (LEFT, RIGHT)
        };
        let across = [arms[low], arms[high]];
        if rounded && weight == Weight::Light && corner_mask(mask(&arms)) {
            return -sign * self.radius();
        }
        if weight == Weight::Double {
            return match (across[0].is_some(), across[1].is_some()) {
                (false, true) => {
                    if start {
                        offset - t * 0.5
                    } else {
                        -offset + t * 0.5
                    }
                }
                (true, false) => {
                    if start {
                        -offset - t * 0.5
                    } else {
                        offset + t * 0.5
                    }
                }
                _ => sign * (g + t * 0.5),
            };
        }
        let half = across
            .iter()
            .flatten()
            .map(|weight| match weight {
                Weight::Light => t * 0.5,
                Weight::Heavy => t,
                Weight::Double => g + t * 0.5,
            })
            .fold(0.0f32, f32::max);
        sign * half
    }

    fn strokes(&self, out: &mut Vec<Draw>) {
        let t = self.thickness();
        if t.is_nan() || t <= 0.0 {
            return;
        }
        let [cw, ch] = self.grid.cell;
        let [ox, oy] = self.grid.origin;
        let columns = i64::from(self.grid.columns);
        let rows = i64::from(self.grid.rows);
        for horizontal in [true, false] {
            let (outer, inner) = if horizontal {
                (rows, columns)
            } else {
                (columns, rows)
            };
            let (back, forward) = if horizontal {
                (LEFT, RIGHT)
            } else {
                (UP, DOWN)
            };
            for fixed in 0..outer {
                let mut runs: Vec<(i64, i64, Weight, [u32; 4], u32, Srgba)> = Vec::new();
                for at in 0..inner {
                    let (c, r) = if horizontal { (at, fixed) } else { (fixed, at) };
                    let Some((arms, _)) = self.arms_at(c, r) else {
                        continue;
                    };
                    let cell = &self.cells[self.index(c, r).unwrap()];
                    for (side, half) in [(back, 0), (forward, 1)] {
                        let Some(weight) = arms[side] else {
                            continue;
                        };
                        let from = 2 * at + half;
                        let run_key = (weight, key(cell.ink));
                        match runs.last_mut() {
                            Some(run) if run.1 == from && (run.2, run.3) == run_key => {
                                run.1 = from + 1;
                            }
                            _ => runs.push((from, from + 1, weight, run_key.1, cell.id, cell.ink)),
                        }
                    }
                }
                for (from, to, weight, _, id, ink) in runs {
                    let lines: Vec<(f32, f32)> = match weight {
                        Weight::Light => vec![(0.0, t)],
                        Weight::Heavy => vec![(0.0, 2.0 * t)],
                        Weight::Double => vec![(-t, t), (t, t)],
                    };
                    for (offset, width) in lines {
                        let along = |lattice: i64, start: bool| {
                            let cell = lattice.div_euclid(2);
                            let base = if horizontal {
                                ox + lattice as f32 * cw * 0.5
                            } else {
                                oy + lattice as f32 * ch * 0.5
                            };
                            if lattice % 2 == 1 {
                                let (c, r) = if horizontal {
                                    (cell, fixed)
                                } else {
                                    (fixed, cell)
                                };
                                base + self.end(c, r, horizontal, start, offset, weight)
                            } else {
                                base
                            }
                        };
                        let a = along(from, true);
                        let b = along(to, false);
                        if b <= a || b.is_nan() || a.is_nan() {
                            continue;
                        }
                        let across = if horizontal {
                            oy + (fixed as f32 + 0.5) * ch + offset
                        } else {
                            ox + (fixed as f32 + 0.5) * cw + offset
                        };
                        let (centre, size) = if horizontal {
                            ([(a + b) * 0.5, across], [b - a, width])
                        } else {
                            ([across, (a + b) * 0.5], [width, b - a])
                        };
                        out.push(
                            Draw::rect(centre, size, 0.0)
                                .fill(ink)
                                .shadow(Shadow::None)
                                .id(id),
                        );
                    }
                }
            }
        }
        let radius = self.radius();
        for r in 0..rows {
            for c in 0..columns {
                let Some((arms, true)) = self.arms_at(c, r) else {
                    continue;
                };
                let mask = mask(&arms);
                if !corner_mask(mask) || arms.iter().flatten().any(|w| *w != Weight::Light) {
                    continue;
                }
                let cx = ox + (c as f32 + 0.5) * cw;
                let cy = oy + (r as f32 + 0.5) * ch;
                let (centre, start) = match mask {
                    6 => ([cx + radius, cy + radius], PI),
                    12 => ([cx - radius, cy + radius], -FRAC_PI_2),
                    9 => ([cx - radius, cy - radius], 0.0),
                    _ => ([cx + radius, cy - radius], FRAC_PI_2),
                };
                let cell = &self.cells[self.index(c, r).unwrap()];
                out.push(
                    Draw::new(Shape::Arc {
                        radius,
                        width: t,
                        start,
                        sweep: FRAC_PI_2,
                    })
                    .at(centre)
                    .fill(cell.ink)
                    .shadow(Shadow::None)
                    .id(cell.id),
                );
            }
        }
    }
}
