use std::collections::BTreeMap;
use std::f32::consts::{FRAC_PI_4, PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    pub on: bool,
}

pub type Contour = Vec<Point>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shape {
    pub width: f32,
    pub height: f32,
    pub contours: Vec<Contour>,
}

impl Shape {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            contours: Vec::new(),
        }
    }

    pub fn add(&mut self, contour: Contour) -> &mut Self {
        self.contours.push(contour);
        self
    }

    pub fn extend(&mut self, contours: Vec<Contour>) -> &mut Self {
        self.contours.extend(contours);
        self
    }

    pub fn polylines(&self) -> Vec<Vec<[f32; 2]>> {
        self.contours.iter().map(flatten).collect()
    }

    pub fn parents(&self) -> Vec<Option<usize>> {
        let lines = self.polylines();
        let areas: Vec<f32> = lines.iter().map(|line| signed_area(line).abs()).collect();
        lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                let probe = line[0];
                (0..lines.len())
                    .filter(|other| *other != index && areas[*other] > areas[index])
                    .filter(|other| contains(&lines[*other], probe))
                    .min_by(|a, b| areas[*a].total_cmp(&areas[*b]))
            })
            .collect()
    }

    pub fn depths(&self) -> Vec<usize> {
        let parents = self.parents();
        (0..parents.len())
            .map(|index| {
                let mut depth = 0;
                let mut at = parents[index];
                while let Some(parent) = at {
                    depth += 1;
                    at = parents[parent];
                }
                depth
            })
            .collect()
    }

    pub fn oriented(mut self) -> Self {
        let depths = self.depths();
        for (contour, depth) in self.contours.iter_mut().zip(depths) {
            let area = signed_area(&flatten(contour));
            if (depth % 2 == 0) != (area > 0.0) {
                reverse(contour);
            }
        }
        self
    }
}

fn reverse(contour: &mut Contour) {
    contour.reverse();
    if let Some(first) = contour.iter().position(|point| point.on) {
        contour.rotate_left(first);
    }
}

pub fn signed_area(line: &[[f32; 2]]) -> f32 {
    let mut sum = 0.0;
    for i in 0..line.len() {
        let a = line[i];
        let b = line[(i + 1) % line.len()];
        sum += a[0] * b[1] - b[0] * a[1];
    }
    sum / 2.0
}

pub fn contains(line: &[[f32; 2]], p: [f32; 2]) -> bool {
    let mut inside = false;
    for i in 0..line.len() {
        let a = line[i];
        let b = line[(i + 1) % line.len()];
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
    }
    inside
}

pub fn winding(lines: &[Vec<[f32; 2]>], p: [f32; 2]) -> i32 {
    let mut total = 0;
    for line in lines {
        for i in 0..line.len() {
            let a = line[i];
            let b = line[(i + 1) % line.len()];
            let side = (b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1]);
            if a[1] <= p[1] {
                if b[1] > p[1] && side > 0.0 {
                    total += 1;
                }
            } else if b[1] <= p[1] && side < 0.0 {
                total -= 1;
            }
        }
    }
    total
}

fn on(x: f32, y: f32) -> Point {
    Point { x, y, on: true }
}

fn off(x: f32, y: f32) -> Point {
    Point { x, y, on: false }
}

pub fn polygon(points: &[[f32; 2]]) -> Contour {
    points.iter().map(|p| on(p[0], p[1])).collect()
}

fn arc(contour: &mut Contour, cx: f32, cy: f32, r: f32, from: f32, to: f32) {
    let steps = (((to - from).abs() / FRAC_PI_4).ceil() as usize).max(1);
    let step = (to - from) / steps as f32;
    let reach = r / (step / 2.0).cos();
    if contour.is_empty() {
        contour.push(on(cx + r * from.cos(), cy + r * from.sin()));
    }
    for i in 0..steps {
        let a = from + step * i as f32;
        let mid = a + step / 2.0;
        let b = a + step;
        contour.push(off(cx + reach * mid.cos(), cy + reach * mid.sin()));
        contour.push(on(cx + r * b.cos(), cy + r * b.sin()));
    }
}

pub fn circle(cx: f32, cy: f32, r: f32) -> Contour {
    let mut contour = Vec::new();
    arc(&mut contour, cx, cy, r, 0.0, TAU);
    contour.pop();
    contour
}

pub fn rounded(x: f32, y: f32, w: f32, h: f32, radii: [f32; 4]) -> Contour {
    let limit = (w.min(h) / 2.0).max(0.0);
    let [tl, tr, br, bl] = radii.map(|r| r.clamp(0.0, limit));
    let mut contour = Vec::new();
    corner(&mut contour, x + tl, y + tl, tl, PI, 1.5 * PI, [x, y]);
    corner(
        &mut contour,
        x + w - tr,
        y + tr,
        tr,
        1.5 * PI,
        TAU,
        [x + w, y],
    );
    corner(
        &mut contour,
        x + w - br,
        y + h - br,
        br,
        0.0,
        0.5 * PI,
        [x + w, y + h],
    );
    corner(
        &mut contour,
        x + bl,
        y + h - bl,
        bl,
        0.5 * PI,
        PI,
        [x, y + h],
    );
    contour
}

fn corner(contour: &mut Contour, cx: f32, cy: f32, r: f32, from: f32, to: f32, sharp: [f32; 2]) {
    if r <= 0.0 {
        contour.push(on(sharp[0], sharp[1]));
        return;
    }
    contour.push(on(cx + r * from.cos(), cy + r * from.sin()));
    let mut tail = vec![on(cx + r * from.cos(), cy + r * from.sin())];
    arc(&mut tail, cx, cy, r, from, to);
    contour.extend(tail.into_iter().skip(1));
}

pub fn rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Contour {
    rounded(x, y, w, h, [r; 4])
}

pub fn pill(cx: f32, cy: f32, w: f32, h: f32) -> Contour {
    rect(cx - w / 2.0, cy - h / 2.0, w, h, h / 2.0)
}

pub fn regular(cx: f32, cy: f32, r: f32, sides: usize, turn: f32) -> Contour {
    (0..sides)
        .map(|i| {
            let a = turn + TAU * i as f32 / sides as f32;
            on(cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

pub fn triangle(cx: f32, cy: f32, r: f32, dir: Dir) -> Contour {
    let turn = match dir {
        Dir::Right => 0.0,
        Dir::Down => 0.5 * PI,
        Dir::Left => PI,
        Dir::Up => 1.5 * PI,
    };
    regular(cx, cy, r, 3, turn)
}

pub fn plus(cx: f32, cy: f32, arm: f32, half: f32) -> Contour {
    polygon(&[
        [cx - half, cy - arm],
        [cx + half, cy - arm],
        [cx + half, cy - half],
        [cx + arm, cy - half],
        [cx + arm, cy + half],
        [cx + half, cy + half],
        [cx + half, cy + arm],
        [cx - half, cy + arm],
        [cx - half, cy + half],
        [cx - arm, cy + half],
        [cx - arm, cy - half],
        [cx - half, cy - half],
    ])
}

pub fn cross(cx: f32, cy: f32, arm: f32, half: f32) -> Contour {
    plus(0.0, 0.0, arm, half)
        .into_iter()
        .map(|p| {
            let (s, c) = FRAC_PI_4.sin_cos();
            on(cx + p.x * c - p.y * s, cy + p.x * s + p.y * c)
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

pub fn flatten(contour: &Contour) -> Vec<[f32; 2]> {
    let mut points = Vec::new();
    let n = contour.len();
    let mut i = 0;
    while i < n {
        let p = contour[i];
        if p.on {
            points.push([p.x, p.y]);
            i += 1;
            continue;
        }
        let a = contour[(i + n - 1) % n];
        let b = contour[(i + 1) % n];
        for step in 1..16 {
            let t = step as f32 / 16.0;
            let u = 1.0 - t;
            points.push([
                u * u * a.x + 2.0 * u * t * p.x + t * t * b.x,
                u * u * a.y + 2.0 * u * t * p.y + t * t * b.y,
            ]);
        }
        i += 1;
    }
    points
}

const FONT: &[(char, [&str; 7])] = &[
    (
        'A',
        [
            ".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
    ),
    (
        'B',
        [
            "####.", "#...#", "#...#", "####.", "#...#", "#...#", "####.",
        ],
    ),
    (
        'C',
        [
            ".###.", "#...#", "#....", "#....", "#....", "#...#", ".###.",
        ],
    ),
    (
        'D',
        [
            "####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####.",
        ],
    ),
    (
        'E',
        [
            "#####", "#....", "#....", "####.", "#....", "#....", "#####",
        ],
    ),
    (
        'F',
        [
            "#####", "#....", "#....", "####.", "#....", "#....", "#....",
        ],
    ),
    (
        'G',
        [
            ".###.", "#...#", "#....", "#.###", "#...#", "#...#", ".###.",
        ],
    ),
    (
        'H',
        [
            "#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
    ),
    (
        'I',
        [
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "#####",
        ],
    ),
    (
        'J',
        [
            "..###", "...#.", "...#.", "...#.", "...#.", "#..#.", ".##..",
        ],
    ),
    (
        'K',
        [
            "#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#",
        ],
    ),
    (
        'L',
        [
            "#....", "#....", "#....", "#....", "#....", "#....", "#####",
        ],
    ),
    (
        'M',
        [
            "#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#",
        ],
    ),
    (
        'N',
        [
            "#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#",
        ],
    ),
    (
        'O',
        [
            ".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
    ),
    (
        'P',
        [
            "####.", "#...#", "#...#", "####.", "#....", "#....", "#....",
        ],
    ),
    (
        'Q',
        [
            ".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#",
        ],
    ),
    (
        'R',
        [
            "####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#",
        ],
    ),
    (
        'S',
        [
            ".####", "#....", "#....", ".###.", "....#", "....#", "####.",
        ],
    ),
    (
        'T',
        [
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#..",
        ],
    ),
    (
        'U',
        [
            "#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
    ),
    (
        'V',
        [
            "#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#..",
        ],
    ),
    (
        'W',
        [
            "#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#",
        ],
    ),
    (
        'X',
        [
            "#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#",
        ],
    ),
    (
        'Y',
        [
            "#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#..",
        ],
    ),
    (
        'Z',
        [
            "#####", "....#", "...#.", "..#..", ".#...", "#....", "#####",
        ],
    ),
    (
        '0',
        [
            ".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###.",
        ],
    ),
    (
        '1',
        [
            "..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###.",
        ],
    ),
    (
        '2',
        [
            ".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####",
        ],
    ),
    (
        '3',
        [
            "####.", "....#", "....#", ".###.", "....#", "....#", "####.",
        ],
    ),
    (
        '4',
        [
            "...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#.",
        ],
    ),
    (
        '5',
        [
            "#####", "#....", "####.", "....#", "....#", "#...#", ".###.",
        ],
    ),
    (
        '6',
        [
            ".###.", "#....", "#....", "####.", "#...#", "#...#", ".###.",
        ],
    ),
    (
        '7',
        [
            "#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#...",
        ],
    ),
    (
        '8',
        [
            ".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###.",
        ],
    ),
    (
        '9',
        [
            ".###.", "#...#", "#...#", ".####", "....#", "....#", ".###.",
        ],
    ),
    (
        '+',
        [
            ".....", "..#..", "..#..", "#####", "..#..", "..#..", ".....",
        ],
    ),
    (
        '-',
        [
            ".....", ".....", ".....", "#####", ".....", ".....", ".....",
        ],
    ),
    (
        '*',
        [
            ".....", "#.#.#", ".###.", "#####", ".###.", "#.#.#", ".....",
        ],
    ),
    (
        '/',
        [
            "....#", "....#", "...#.", "..#..", ".#...", "#....", "#....",
        ],
    ),
    (
        '\\',
        [
            "#....", "#....", ".#...", "..#..", "...#.", "....#", "....#",
        ],
    ),
    (
        '.',
        [
            ".....", ".....", ".....", ".....", ".....", ".##..", ".##..",
        ],
    ),
    (
        ',',
        [
            ".....", ".....", ".....", ".....", ".##..", "..#..", ".#...",
        ],
    ),
    (
        ';',
        [
            ".....", ".##..", ".##..", ".....", ".##..", "..#..", ".#...",
        ],
    ),
    (
        '\'',
        [
            "..#..", "..#..", ".#...", ".....", ".....", ".....", ".....",
        ],
    ),
    (
        '`',
        [
            ".#...", "..#..", "...#.", ".....", ".....", ".....", ".....",
        ],
    ),
    (
        '=',
        [
            ".....", ".....", "#####", ".....", "#####", ".....", ".....",
        ],
    ),
    (
        '[',
        [
            ".###.", ".#...", ".#...", ".#...", ".#...", ".#...", ".###.",
        ],
    ),
    (
        ']',
        [
            ".###.", "...#.", "...#.", "...#.", "...#.", "...#.", ".###.",
        ],
    ),
    (
        '?',
        [
            ".###.", "#...#", "....#", "...#.", "..#..", ".....", "..#..",
        ],
    ),
];

pub const FONT_COLUMNS: usize = 5;
pub const FONT_ROWS: usize = 7;

pub fn has_char(c: char) -> bool {
    FONT.iter().any(|(ch, _)| *ch == c)
}

fn rows(c: char) -> [&'static str; 7] {
    FONT.iter()
        .find(|(ch, _)| *ch == c)
        .or_else(|| FONT.iter().find(|(ch, _)| *ch == '?'))
        .map(|(_, rows)| *rows)
        .unwrap_or(["....."; 7])
}

pub fn bitmap(text: &str) -> (usize, usize, Vec<bool>) {
    let chars: Vec<char> = text.chars().collect();
    let width = if chars.is_empty() {
        0
    } else {
        chars.len() * (FONT_COLUMNS + 1) - 1
    };
    let mut cells = vec![false; width * FONT_ROWS];
    for (index, c) in chars.iter().enumerate() {
        let rows = rows(*c);
        for (y, row) in rows.iter().enumerate() {
            for (x, byte) in row.bytes().enumerate() {
                if byte == b'#' {
                    cells[y * width + index * (FONT_COLUMNS + 1) + x] = true;
                }
            }
        }
    }
    (width, FONT_ROWS, cells)
}

pub fn trace(width: usize, height: usize, cells: &[bool]) -> Vec<Vec<[i32; 2]>> {
    let filled = |x: i32, y: i32| -> bool {
        x >= 0
            && y >= 0
            && (x as usize) < width
            && (y as usize) < height
            && cells[y as usize * width + x as usize]
    };
    let mut segments: Vec<([i32; 2], [i32; 2])> = Vec::new();
    for j in -1..height as i32 {
        for i in -1..width as i32 {
            let tl = filled(i, j);
            let tr = filled(i + 1, j);
            let br = filled(i + 1, j + 1);
            let bl = filled(i, j + 1);
            let top = [2 * i + 1, 2 * j];
            let right = [2 * i + 2, 2 * j + 1];
            let bottom = [2 * i + 1, 2 * j + 2];
            let left = [2 * i, 2 * j + 1];
            match (tl, tr, br, bl) {
                (true, false, true, false) => {
                    segments.push((top, right));
                    segments.push((left, bottom));
                }
                (false, true, false, true) => {
                    segments.push((top, left));
                    segments.push((right, bottom));
                }
                _ => {
                    let mut ends = Vec::new();
                    if tl != tr {
                        ends.push(top);
                    }
                    if tr != br {
                        ends.push(right);
                    }
                    if br != bl {
                        ends.push(bottom);
                    }
                    if bl != tl {
                        ends.push(left);
                    }
                    if ends.len() == 2 {
                        segments.push((ends[0], ends[1]));
                    }
                }
            }
        }
    }
    let mut at: BTreeMap<[i32; 2], Vec<usize>> = BTreeMap::new();
    for (index, (a, b)) in segments.iter().enumerate() {
        at.entry(*a).or_default().push(index);
        at.entry(*b).or_default().push(index);
    }
    let mut used = vec![false; segments.len()];
    let mut loops = Vec::new();
    for start in 0..segments.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let (first, mut current) = segments[start];
        let mut points = vec![first];
        while current != first {
            points.push(current);
            let Some(next) = at[&current].iter().copied().find(|index| !used[*index]) else {
                break;
            };
            used[next] = true;
            let (a, b) = segments[next];
            current = if a == current { b } else { a };
        }
        loops.push(simplify(points));
    }
    loops
}

fn simplify(points: Vec<[i32; 2]>) -> Vec<[i32; 2]> {
    let n = points.len();
    let mut kept = Vec::new();
    for i in 0..n {
        let a = points[(i + n - 1) % n];
        let b = points[i];
        let c = points[(i + 1) % n];
        let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
        if cross != 0 {
            kept.push(b);
        }
    }
    kept
}

pub fn label(text: &str, cx: f32, cy: f32, max_w: f32, max_h: f32, max_cell: f32) -> Vec<Contour> {
    let (width, height, cells) = bitmap(text);
    if width == 0 {
        return Vec::new();
    }
    let cell = (max_w / width as f32)
        .min(max_h / height as f32)
        .min(max_cell);
    let x0 = cx - width as f32 * cell / 2.0;
    let y0 = cy - height as f32 * cell / 2.0;
    trace(width, height, &cells)
        .into_iter()
        .map(|points| {
            points
                .into_iter()
                .map(|p| {
                    on(
                        x0 + (p[0] as f32 / 2.0 + 0.5) * cell,
                        y0 + (p[1] as f32 / 2.0 + 0.5) * cell,
                    )
                })
                .collect()
        })
        .collect()
}

pub fn label_width(text: &str, cell: f32) -> f32 {
    let count = text.chars().count();
    if count == 0 {
        0.0
    } else {
        (count * (FONT_COLUMNS + 1) - 1) as f32 * cell
    }
}

pub fn crossings(shape: &Shape) -> usize {
    let lines = shape.polylines();
    let mut found = 0;
    for (ci, a) in lines.iter().enumerate() {
        for b in lines.iter().skip(ci + 1) {
            for i in 0..a.len() {
                let p1 = a[i];
                let p2 = a[(i + 1) % a.len()];
                for j in 0..b.len() {
                    let q1 = b[j];
                    let q2 = b[(j + 1) % b.len()];
                    if segments_touch(p1, p2, q1, q2) {
                        found += 1;
                    }
                }
            }
        }
    }
    found
}

fn segments_touch(p1: [f32; 2], p2: [f32; 2], q1: [f32; 2], q2: [f32; 2]) -> bool {
    let orient = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let d1 = orient(q1, q2, p1);
    let d2 = orient(q1, q2, p2);
    let d3 = orient(p1, p2, q1);
    let d4 = orient(p1, p2, q2);
    ((d1 > 0.0) != (d2 > 0.0) || d1 == 0.0 || d2 == 0.0)
        && ((d3 > 0.0) != (d4 > 0.0) || d3 == 0.0 || d4 == 0.0)
        && bounds_overlap(p1, p2, q1, q2)
}

fn bounds_overlap(p1: [f32; 2], p2: [f32; 2], q1: [f32; 2], q2: [f32; 2]) -> bool {
    p1[0].min(p2[0]) <= q1[0].max(q2[0])
        && q1[0].min(q2[0]) <= p1[0].max(p2[0])
        && p1[1].min(p2[1]) <= q1[1].max(q2[1])
        && q1[1].min(q2[1]) <= p1[1].max(p2[1])
}

pub fn min_gap(shape: &Shape) -> f32 {
    let lines = shape.polylines();
    let mut gap = f32::INFINITY;
    for (ci, a) in lines.iter().enumerate() {
        for b in lines.iter().skip(ci + 1) {
            for p in a {
                for j in 0..b.len() {
                    gap = gap.min(point_segment(*p, b[j], b[(j + 1) % b.len()]));
                }
            }
            for p in b {
                for i in 0..a.len() {
                    gap = gap.min(point_segment(*p, a[i], a[(i + 1) % a.len()]));
                }
            }
        }
    }
    gap
}

fn point_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let v = [b[0] - a[0], b[1] - a[1]];
    let len2 = v[0] * v[0] + v[1] * v[1];
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * v[0] + (p[1] - a[1]) * v[1]) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - t * v[0]).hypot(p[1] - a[1] - t * v[1])
}
