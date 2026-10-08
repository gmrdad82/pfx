use crate::effects::ParticleInstance;
use crate::flat::FlatScene;
use crate::frame::{Matrix, Scene, multiply};
use crate::renderer::{Effects, Finish, Text, invert};
use crate::text::{PackedGlyph, TextSpace};
use pfx_gpu::{GpuProfiler, wgpu};
use std::sync::mpsc::{Receiver, TryRecvError};

pub const SETTLE: u32 = crate::taa::CYCLE;
pub const MARGIN: u32 = 4;
pub const DEPTH_PROBES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn whole(width: u32, height: u32) -> Self {
        Self::new(0, 0, width, height)
    }

    pub fn from_bounds(bounds: [f32; 4], width: u32, height: u32) -> Option<Self> {
        if !bounds.iter().all(|value| value.is_finite()) {
            return Some(Self::whole(width, height));
        }
        let left = bounds[0].floor().clamp(0.0, width as f32) as u32;
        let top = bounds[1].floor().clamp(0.0, height as f32) as u32;
        let right = bounds[2].ceil().clamp(0.0, width as f32) as u32;
        let bottom = bounds[3].ceil().clamp(0.0, height as f32) as u32;
        (right > left && bottom > top).then(|| Self::new(left, top, right - left, bottom - top))
    }

    pub fn right(&self) -> u32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> u32 {
        self.y + self.height
    }

    pub fn area(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    pub fn clip(self, width: u32, height: u32) -> Self {
        let x = self.x.min(width);
        let y = self.y.min(height);
        Self::new(
            x,
            y,
            self.right().min(width) - x,
            self.bottom().min(height) - y,
        )
    }

    pub fn grow(self, margin: u32, width: u32, height: u32) -> Self {
        let x = self.x.saturating_sub(margin);
        let y = self.y.saturating_sub(margin);
        Self::new(
            x,
            y,
            (self.right() + margin).min(width) - x,
            (self.bottom() + margin).min(height) - y,
        )
    }

    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }

    pub fn touches(&self, other: &Self, gap: u32) -> bool {
        self.x <= other.right() + gap
            && other.x <= self.right() + gap
            && self.y <= other.bottom() + gap
            && other.y <= self.bottom() + gap
    }
}

pub fn merge(regions: &[Region], width: u32, height: u32) -> Vec<Region> {
    let mut merged: Vec<Region> = regions
        .iter()
        .map(|region| region.clip(width, height))
        .filter(|region| !region.is_empty())
        .collect();
    loop {
        let mut joined = false;
        let mut index = 0;
        while index < merged.len() {
            let mut other = index + 1;
            while other < merged.len() {
                if merged[index].touches(&merged[other], 16) {
                    let region = merged.swap_remove(other);
                    merged[index] = merged[index].union(region);
                    joined = true;
                } else {
                    other += 1;
                }
            }
            index += 1;
        }
        if !joined {
            break;
        }
    }
    let covered: u64 = merged.iter().map(Region::area).sum();
    if covered * 5 > Region::whole(width, height).area() * 3 {
        return vec![Region::whole(width, height)];
    }
    merged.sort_by_key(|region| (region.y, region.x));
    merged
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Need {
    None,
    Reproject,
    Partial(Vec<Region>),
    Full,
}

impl Need {
    pub fn draws(&self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    Full,
    Reproject,
    Partial,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub angle: f32,
    pub zoom: f32,
    pub holes: f32,
    pub frames: u32,
    pub settle: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            angle: 0.6_f32.to_radians(),
            zoom: 0.03,
            holes: 0.005,
            frames: 90,
            settle: SETTLE,
        }
    }
}

pub struct Next<'a> {
    pub scene: &'a Scene<'a>,
    pub text: &'a Text<'a>,
    pub effects: &'a Effects<'a>,
    pub style: Finish,
    pub hud: Option<&'a FlatScene<'a>>,
    pub content: u64,
    pub overlay: u64,
    pub moving: &'a [Region],
}

impl<'a> Next<'a> {
    pub fn new(
        scene: &'a Scene<'a>,
        text: &'a Text<'a>,
        effects: &'a Effects<'a>,
        style: Finish,
    ) -> Self {
        Self {
            scene,
            text,
            effects,
            style,
            hud: None,
            content: 0,
            overlay: 0,
            moving: &[],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DemandStats {
    pub shown: Option<Shown>,
    pub since_full: u32,
    pub settle: u32,
    pub holes: f32,
    pub depth: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Summary {
    pub view: Matrix,
    pub projection: Matrix,
    pub position: [f32; 3],
    pub size: [u32; 2],
    pub world: u64,
    pub anchors: Vec<Matrix>,
    pub overlay: u64,
    pub ticking: bool,
    pub layered: Vec<Region>,
    pub moving: Vec<Region>,
}

impl Summary {
    fn same_camera(&self, other: &Self) -> bool {
        self.view == other.view && self.projection == other.projection
    }

    fn same_world(&self, other: &Self) -> bool {
        self.world == other.world
            && self.anchors.len() == other.anchors.len()
            && self
                .anchors
                .iter()
                .zip(&other.anchors)
                .all(|(a, b)| close(a, b))
    }
}

fn close(a: &Matrix, b: &Matrix) -> bool {
    a.iter()
        .flatten()
        .zip(b.iter().flatten())
        .all(|(x, y)| (x - y).abs() <= 1e-4 * x.abs().max(y.abs()).max(1.0))
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Book {
    pub cached: Option<Summary>,
    pub shown: Option<Summary>,
    pub kind: Option<Shown>,
    pub since_full: u32,
    pub settle: u32,
    pub holes: f32,
    pub depth: Option<f32>,
    pub still: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub angle: f32,
    pub zoom: f32,
}

fn rotation(view: &Matrix) -> [[f32; 3]; 3] {
    std::array::from_fn(|column| std::array::from_fn(|row| view[column][row]))
}

pub fn motion(
    from: (&Matrix, &Matrix, [f32; 3]),
    to: (&Matrix, &Matrix, [f32; 3]),
    depth: Option<f32>,
) -> Option<Motion> {
    let (view0, projection0, position0) = from;
    let (view1, projection1, position1) = to;
    if projection0[2][3] != -1.0 || projection0[3][3] != 0.0 {
        return None;
    }
    let scale0 = projection0[1][1];
    let scale1 = projection1[1][1];
    if !(scale0 > 0.0 && scale1 > 0.0) {
        return None;
    }
    let aspect0 = projection0[0][0] / scale0;
    let aspect1 = projection1[0][0] / scale1;
    if (aspect0 - aspect1).abs() > 1e-5 * aspect0.abs().max(1.0) {
        return None;
    }
    for (column, row) in [
        (2, 0),
        (2, 1),
        (2, 2),
        (2, 3),
        (3, 2),
        (3, 3),
        (3, 0),
        (3, 1),
    ] {
        let a = projection0[column][row];
        let b = projection1[column][row];
        if (a - b).abs() > 1e-5 * a.abs().max(b.abs()).max(1.0) {
            return None;
        }
    }
    let r0 = rotation(view0);
    let r1 = rotation(view1);
    let relative: [[f64; 3]; 3] = std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            (0..3)
                .map(|k| f64::from(r1[k][i]) * f64::from(r0[k][j]))
                .sum()
        })
    });
    let trace = relative[0][0] + relative[1][1] + relative[2][2];
    let skew = [
        relative[2][1] - relative[1][2],
        relative[0][2] - relative[2][0],
        relative[1][0] - relative[0][1],
    ];
    let sine = (skew[0] * skew[0] + skew[1] * skew[1] + skew[2] * skew[2]).sqrt() * 0.5;
    let turned = sine.atan2((trace - 1.0) * 0.5) as f32;
    let shift = {
        let d = [
            position1[0] - position0[0],
            position1[1] - position0[1],
            position1[2] - position0[2],
        ];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    };
    let shifted = if shift <= 1e-7 {
        0.0
    } else {
        match depth {
            Some(depth) if depth.is_finite() && depth > 0.0 => shift / depth,
            Some(_) => 0.0,
            None => return None,
        }
    };
    Some(Motion {
        angle: turned + shifted,
        zoom: (scale1 / scale0).ln().abs(),
    })
}

pub(crate) fn decide(book: &Book, next: &Summary, limits: &Limits) -> Need {
    let Some(cached) = &book.cached else {
        return Need::Full;
    };
    if next.size != cached.size
        || !next.same_world(cached)
        || next.ticking
        || book.settle > 0
        || (!next.layered.is_empty() && !book.still)
    {
        return Need::Full;
    }
    let shown = book.shown.as_ref().unwrap_or(cached);
    let kind = book.kind.unwrap_or(Shown::Full);
    let [width, height] = next.size;
    let mut layered: Vec<Region> = next.layered.clone();
    layered.extend(shown.layered.iter().copied());
    let overlay_changed = next.overlay != shown.overlay;
    if !next.same_camera(cached) {
        let Some(moved) = motion(
            (&cached.view, &cached.projection, cached.position),
            (&next.view, &next.projection, next.position),
            book.depth,
        ) else {
            return Need::Full;
        };
        if moved.angle > limits.angle
            || moved.zoom > limits.zoom
            || book.since_full >= limits.frames
            || book.holes > limits.holes
        {
            return Need::Full;
        }
        if kind == Shown::Reproject
            && next.same_camera(shown)
            && layered.is_empty()
            && !overlay_changed
            && next.moving.is_empty()
        {
            return Need::Full;
        }
        return Need::Reproject;
    }
    if kind == Shown::Reproject {
        return Need::Reproject;
    }
    let mut regions = layered;
    regions.extend(next.moving.iter().copied());
    regions.extend(shown.moving.iter().copied());
    if overlay_changed {
        regions.push(Region::whole(width, height));
    }
    if regions.is_empty() {
        Need::None
    } else {
        Need::Partial(merge(&regions, width, height))
    }
}

pub(crate) fn record(book: &mut Book, summary: Summary, kind: Shown, limits: &Limits) {
    match kind {
        Shown::Full => {
            let changed = book
                .cached
                .as_ref()
                .is_none_or(|cached| !cached.same_world(&summary) || cached.size != summary.size);
            book.settle = if changed {
                limits.settle
            } else {
                book.settle.saturating_sub(1)
            };
            book.cached = Some(summary.clone());
            book.since_full = 0;
            book.holes = 0.0;
        }
        Shown::Reproject => book.since_full = book.since_full.saturating_add(1),
        Shown::Partial => {}
    }
    book.shown = Some(summary);
    book.kind = Some(kind);
}

#[derive(Clone, Copy)]
pub(crate) struct Print(u64);

impl Print {
    pub fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub fn word(&mut self, value: u32) {
        self.0 = (self.0 ^ u64::from(value)).wrapping_mul(0x0000_0100_0000_01b3);
    }

    pub fn wide(&mut self, value: u64) {
        self.word(value as u32);
        self.word((value >> 32) as u32);
    }

    pub fn float(&mut self, value: f32) {
        self.word(value.to_bits());
    }

    pub fn floats(&mut self, values: &[f32]) {
        for value in values {
            self.float(*value);
        }
    }

    pub fn matrix(&mut self, matrix: &Matrix) {
        for column in matrix {
            self.floats(column);
        }
    }

    pub fn bytes(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(4);
        for chunk in &mut chunks {
            self.word(u32::from_le_bytes(chunk.try_into().unwrap()));
        }
        for byte in chunks.remainder() {
            self.word(u32::from(*byte) | 0x100);
        }
        self.word(bytes.len() as u32);
    }

    pub fn debug(&mut self, value: &impl std::fmt::Debug) {
        use std::fmt::Write;
        let _ = write!(self, "{value:?}");
        self.word(0xffff_ffff);
    }

    pub fn finish(self) -> u64 {
        self.0
    }
}

impl std::fmt::Write for Print {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.bytes(text.as_bytes());
        Ok(())
    }
}

pub(crate) fn print_scene(print: &mut Print, scene: &Scene<'_>) {
    print.word(scene.seed);
    print.floats(&scene.sun.direction);
    print.floats(&scene.sun.colour);
    print.float(scene.sun.intensity);
    print.float(scene.wind.current.strength);
    print.floats(&scene.wind.current.direction);
    print.word(scene.instances.len() as u32);
    for instance in scene.instances {
        print.word(instance.mesh.0);
        print.matrix(&instance.model);
        print.word(instance.material);
        print.word(instance.id);
        print.float(instance.opacity);
        print.float(instance.alpha_cutoff);
        print.word(instance.deformer.0);
        print.float(instance.age);
        print.float(instance.coverage);
        print.word(
            u32::from(instance.shadow_only)
                | u32::from(instance.casts_shadow) << 1
                | u32::from(instance.two_sided) << 2,
        );
    }
    print.word(scene.materials.len() as u32);
    for material in scene.materials {
        print.debug(material);
    }
    print.word(scene.deformers.len() as u32);
    for deformer in scene.deformers {
        print.debug(deformer);
    }
}

fn print_space(
    print: &mut Print,
    space: &TextSpace,
    anchors: &mut Vec<Matrix>,
    view_projection: &Matrix,
) {
    match space {
        TextSpace::Surface {
            model_view_projection,
        } => {
            print.word(1);
            anchors.push(anchor(view_projection, model_view_projection));
        }
        TextSpace::Deformed {
            model,
            layout,
            normal,
            lift,
            deformer,
            ..
        } => {
            print.word(2);
            print.matrix(model);
            print.matrix(layout);
            print.floats(normal);
            print.float(*lift);
            print.word(deformer.0);
        }
        TextSpace::Overlay { width, height } => {
            print.word(3);
            print.word(*width);
            print.word(*height);
        }
        TextSpace::LitSurface {
            model_view_projection,
            model,
        } => {
            print.word(4);
            print.matrix(model);
            anchors.push(anchor(view_projection, model_view_projection));
        }
        TextSpace::LitDeformed {
            model,
            layout,
            normal,
            lift,
            deformer,
            ..
        } => {
            print.word(5);
            print.matrix(model);
            print.matrix(layout);
            print.floats(normal);
            print.float(*lift);
            print.word(deformer.0);
        }
    }
}

fn anchor(view_projection: &Matrix, model_view_projection: &Matrix) -> Matrix {
    invert(*view_projection)
        .map(|inverse| multiply(inverse, *model_view_projection))
        .unwrap_or(*model_view_projection)
}

pub(crate) fn print_text(
    world: &mut Print,
    overlay: &mut Print,
    anchors: &mut Vec<Matrix>,
    text: &Text<'_>,
    view_projection: &Matrix,
) {
    for (print, items) in [(&mut *world, &text.surface), (&mut *overlay, &text.overlay)] {
        print.word(items.len() as u32);
        for item in items {
            print.wide(std::ptr::from_ref(item.atlas) as usize as u64);
            print.floats(&item.color);
            print.word(item.id);
            print_space(print, &item.space, anchors, view_projection);
            print.word(item.paragraph.glyphs.len() as u32);
            for glyph in &item.paragraph.glyphs {
                print.bytes(bytemuck::bytes_of(&PackedGlyph::from(glyph)));
            }
        }
    }
    for (print, items) in [
        (&mut *world, &text.surface_quads),
        (&mut *overlay, &text.overlay_quads),
    ] {
        print.word(items.len() as u32);
        for item in items {
            print.wide(std::ptr::from_ref(item.atlas) as usize as u64);
            print.word(u32::from(item.icons));
            print.word(item.id);
            print_space(print, &item.space, anchors, view_projection);
            print.word(item.quads.len() as u32);
            for quad in item.quads {
                print.bytes(bytemuck::bytes_of(&PackedGlyph::from(quad)));
            }
        }
    }
}

pub(crate) fn print_effects(print: &mut Print, effects: &Effects<'_>) {
    print.word(effects.glass.len() as u32);
    for surface in effects.glass {
        print.word(surface.mesh.0);
        print.matrix(&surface.model);
        print.debug(&surface.material);
        print.word(u32::from(surface.liquid));
        print.float(surface.fluid_height);
        print.float(surface.ripple_height);
        print.float(surface.caustic_strength);
        print.debug(&surface.tinted);
        print.word(surface.id);
        print.word(u32::from(surface.casts_shadow) | u32::from(surface.shadow_only) << 1);
        print.floats(surface.clip.as_flattened());
    }
    print.word(effects.props.len() as u32);
    for prop in effects.props {
        print.bytes(bytemuck::bytes_of(prop));
    }
}

fn transform(matrix: &Matrix, point: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|row| {
        (0..4)
            .map(|column| matrix[column][row] * point[column])
            .sum()
    })
}

pub fn box_region(
    view_projection: &Matrix,
    lo: [f32; 3],
    hi: [f32; 3],
    margin: u32,
    width: u32,
    height: u32,
) -> Option<Region> {
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for x in [lo[0], hi[0]] {
        for y in [lo[1], hi[1]] {
            for z in [lo[2], hi[2]] {
                let clip = transform(view_projection, [x, y, z, 1.0]);
                if clip[3] <= 0.0 || !clip.iter().all(|value| value.is_finite()) {
                    return Some(Region::whole(width, height));
                }
                let screen = [
                    (clip[0] / clip[3] * 0.5 + 0.5) * width as f32,
                    (0.5 - clip[1] / clip[3] * 0.5) * height as f32,
                ];
                bounds[0] = bounds[0].min(screen[0]);
                bounds[1] = bounds[1].min(screen[1]);
                bounds[2] = bounds[2].max(screen[0]);
                bounds[3] = bounds[3].max(screen[1]);
            }
        }
    }
    let margin = margin as f32;
    Region::from_bounds(
        [
            bounds[0] - margin,
            bounds[1] - margin,
            bounds[2] + margin,
            bounds[3] + margin,
        ],
        width,
        height,
    )
}

pub fn particle_region(
    view_projection: &Matrix,
    particles: &[ParticleInstance],
    margin: u32,
    width: u32,
    height: u32,
) -> Option<Region> {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for particle in particles {
        let reach = particle.position_size[3].abs() * 1.5;
        for axis in 0..3 {
            lo[axis] = lo[axis].min(particle.position_size[axis] - reach);
            hi[axis] = hi[axis].max(particle.position_size[axis] + reach);
        }
    }
    if particles.is_empty() {
        return None;
    }
    box_region(view_projection, lo, hi, margin, width, height)
}

pub fn local_post(chain: &pfx_post::Chain) -> bool {
    chain.is_local()
}

pub(crate) const WARP_WGSL: &str = r#"
struct Warp {
    across: vec4f,
    down: vec4f,
    corner: vec4f,
    origin: vec4f,
    size: vec4f,
    limits: vec4f,
}
@group(0) @binding(0) var<uniform> warp: Warp;
@group(0) @binding(1) var colour: texture_2d<f32>;
@group(0) @binding(2) var depth: texture_2d<f32>;
@group(0) @binding(3) var smooth_sampler: sampler;
@group(0) @binding(4) var<storage, read_write> holes: array<atomic<u32>, 4>;
@group(0) @binding(5) var tiles: texture_2d<f32>;
@group(0) @binding(6) var overlay: texture_2d<f32>;

@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let xy = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
    return vec4f(xy[index], 0.0, 1.0);
}

fn depth_at(uv: vec2f) -> f32 {
    let size = vec2i(warp.size.xy);
    let p = clamp(vec2i(floor(uv * warp.size.xy)), vec2i(0), size - vec2i(1));
    return textureLoad(depth, p, 0).r;
}

fn moved(uv: vec2f, d: f32) -> vec2f {
    let toward = warp.across * uv.x + warp.down * uv.y + warp.corner;
    let clip = select(toward * d + warp.origin, toward, d >= warp.limits.x);
    let ndc = clip.xy / clip.w;
    return vec2f(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

struct Found {
    uv: vec2f,
    miss: f32,
    depth: f32,
}

fn solve(goal: vec2f, start: vec2f) -> Found {
    var s = start;
    var found: Found;
    for (var step = 0; step < 4; step = step + 1) {
        found.depth = depth_at(s);
        let error = goal - moved(s, found.depth);
        found.miss = length(error * warp.size.xy);
        found.uv = s;
        if (found.miss < 0.05) {
            break;
        }
        s = s + error;
    }
    return found;
}

fn seed(goal: vec2f, d: f32) -> vec2f {
    return goal - (moved(goal, d) - goal);
}

fn pick(goal: vec2f) -> Found {
    let first = solve(goal, seed(goal, depth_at(goal)));
    let last = vec2i(textureDimensions(tiles)) - vec2i(1);
    let tile = textureLoad(tiles, clamp(vec2i(first.uv * warp.size.xy / warp.limits.z), vec2i(0), last), 0).rg;
    let near = min(tile.x, first.depth);
    let far = max(tile.y, first.depth);
    let tolerance = warp.limits.y;
    if (far <= near * 1.02 && first.miss < tolerance) {
        return first;
    }
    let front = solve(goal, seed(goal, near));
    let back = solve(goal, seed(goal, far));
    var best = back;
    var hit = back.miss < tolerance;
    if (first.miss < tolerance && (!hit || first.depth < best.depth)) {
        best = first;
        hit = true;
    }
    if (front.miss < tolerance && (!hit || front.depth < best.depth)) {
        best = front;
        hit = true;
    }
    if (!hit) {
        best.miss = 1.0e9;
    }
    return best;
}

fn fetch(uv: vec2f) -> vec4f {
    return textureSampleLevel(colour, smooth_sampler, uv, 0.0);
}

fn catmull_rom(uv: vec2f) -> vec4f {
    let size = warp.size.xy;
    let inv = warp.size.zw;
    let at = uv * size;
    let centre = floor(at - 0.5) + 0.5;
    let f = at - centre;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let offset = w2 / w12;
    let t0 = (centre - 1.0) * inv;
    let t3 = (centre + 2.0) * inv;
    let t12 = (centre + offset) * inv;
    var sum = vec4f(0.0);
    var weight = 0.0;
    let a = w12.x * w0.y;
    let ta = fetch(vec2f(t12.x, t0.y));
    sum += ta * a;
    let b = w0.x * w12.y;
    let tb = fetch(vec2f(t0.x, t12.y));
    sum += tb * b;
    let c = w12.x * w12.y;
    let tc = fetch(vec2f(t12.x, t12.y));
    sum += tc * c;
    let d = w3.x * w12.y;
    let td = fetch(vec2f(t3.x, t12.y));
    sum += td * d;
    let e = w12.x * w3.y;
    let te = fetch(vec2f(t12.x, t3.y));
    sum += te * e;
    weight = a + b + c + d + e;
    let lo = min(min(min(ta, tb), min(tc, td)), te);
    let hi = max(max(max(ta, tb), max(tc, td)), te);
    return clamp(sum / weight, lo, hi);
}

fn warped(position: vec4f) -> Found {
    let goal = position.xy * warp.size.zw;
    var found = pick(goal);
    let outside = any(found.uv < vec2f(0.0)) || any(found.uv > vec2f(1.0));
    if (found.miss >= warp.limits.y || outside) {
        atomicAdd(&holes[0], 1u);
    }
    found.uv = clamp(found.uv, vec2f(0.0), vec2f(1.0));
    return found;
}

fn resample(uv: vec2f) -> vec4f {
    let at = uv * warp.size.xy;
    let centre = floor(at) + 0.5;
    if (all(abs(at - centre) < vec2f(0.002))) {
        return textureLoad(colour, vec2i(floor(at)), 0);
    }
    return catmull_rom(uv);
}

@fragment fn fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let found = warped(position);
    let base = resample(found.uv);
    if (warp.limits.w > 0.5) {
        let over = textureLoad(overlay, vec2i(position.xy), 0);
        return over + base * (1.0 - over.a);
    }
    return base;
}

struct Layered {
    @location(0) colour: vec4f,
    @location(1) depth: f32,
}

@fragment fn layered(@builtin(position) position: vec4f) -> Layered {
    let found = warped(position);
    var out: Layered;
    out.colour = resample(found.uv);
    out.depth = found.depth;
    return out;
}
"#;

pub const WAKE_GROUPS: u32 = 1024;
pub const TILE: u32 = 4;

pub(crate) const TILES_WGSL: &str = r#"
@group(0) @binding(0) var depth: texture_2d<f32>;
@group(0) @binding(1) var raw: texture_storage_2d<rg32float, write>;
@compute @workgroup_size(8, 8)
fn reduce(@builtin(global_invocation_id) id: vec3u) {
    let tiles = textureDimensions(raw);
    if (id.x >= tiles.x || id.y >= tiles.y) {
        return;
    }
    let size = vec2i(textureDimensions(depth));
    let origin = vec2i(id.xy) * 4;
    var lo = 3.0e38;
    var hi = 0.0;
    for (var y = 0; y < 4; y = y + 1) {
        for (var x = 0; x < 4; x = x + 1) {
            let d = textureLoad(depth, min(origin + vec2i(x, y), size - vec2i(1)), 0).r;
            lo = min(lo, d);
            hi = max(hi, d);
        }
    }
    textureStore(raw, vec2i(id.xy), vec4f(lo, hi, 0.0, 0.0));
}
"#;

pub(crate) const SPREAD_WGSL: &str = r#"
@group(0) @binding(0) var raw: texture_2d<f32>;
@group(0) @binding(1) var tiles: texture_storage_2d<rg32float, write>;
@compute @workgroup_size(8, 8)
fn spread(@builtin(global_invocation_id) id: vec3u) {
    let size = vec2i(textureDimensions(raw));
    if (i32(id.x) >= size.x || i32(id.y) >= size.y) {
        return;
    }
    var lo = 3.0e38;
    var hi = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let p = clamp(vec2i(id.xy) + vec2i(x, y), vec2i(0), size - vec2i(1));
            let range = textureLoad(raw, p, 0).rg;
            lo = min(lo, range.x);
            hi = max(hi, range.y);
        }
    }
    textureStore(tiles, vec2i(id.xy), vec4f(lo, hi, 0.0, 0.0));
}
"#;

pub(crate) const WAKE_WGSL: &str = r#"
@group(0) @binding(0) var<storage, read_write> sink: array<f32, 4>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u) {
    var x = f32(id.x) * 1.0e-3;
    for (var step = 0u; step < 2048u; step = step + 1u) {
        x = fma(x, 1.0000001, 1.0e-7);
    }
    if (x == 12345.0) {
        sink[id.x & 3u] = x;
    }
}
"#;

pub(crate) const COMPOSITE_WGSL: &str = r#"
@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let xy = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
    return vec4f(xy[index], 0.0, 1.0);
}
@group(0) @binding(0) var base: texture_2d<f32>;
@group(0) @binding(1) var overlay: texture_2d<f32>;
@fragment fn fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let under = textureLoad(base, vec2i(position.xy), 0);
    let over = textureLoad(overlay, vec2i(position.xy), 0);
    return over + under * (1.0 - over.a);
}
"#;

pub(crate) const COPY_WGSL: &str = r#"
@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let xy = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
    return vec4f(xy[index], 0.0, 1.0);
}
@group(0) @binding(0) var source: texture_2d<f32>;
@fragment fn fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
    return textureLoad(source, vec2i(position.xy), 0);
}
"#;

#[derive(Clone, Copy)]
pub(crate) struct WarpUniform {
    pub across: [f32; 4],
    pub down: [f32; 4],
    pub corner: [f32; 4],
    pub origin: [f32; 4],
    pub size: [f32; 4],
    pub limits: [f32; 4],
}

impl WarpUniform {
    pub fn new(
        from: (&Matrix, &Matrix),
        to: (&Matrix, &Matrix),
        size: [u32; 2],
        far: f32,
    ) -> Option<Self> {
        let (view0, projection0) = from;
        let (view1, projection1) = to;
        let inverse_projection = invert(*projection0)?;
        let mapped = multiply(*projection1, multiply(*view1, invert(*view0)?));
        let ray = |u: f32, v: f32| {
            let h = transform(
                &inverse_projection,
                [u * 2.0 - 1.0, 1.0 - v * 2.0, 0.5, 1.0],
            );
            [h[0] / -h[2], h[1] / -h[2]]
        };
        let start = ray(0.0, 0.0);
        let right = ray(1.0, 0.0)[0] - start[0];
        let below = ray(0.0, 1.0)[1] - start[1];
        let column = |index: usize| mapped[index];
        let scaled = |vector: [f32; 4], by: f32| vector.map(|value| value * by);
        let corner: [f32; 4] = std::array::from_fn(|row| {
            column(0)[row] * start[0] + column(1)[row] * start[1] - column(2)[row]
        });
        Some(Self {
            across: scaled(column(0), right),
            down: scaled(column(1), below),
            corner,
            origin: column(3),
            size: [
                size[0] as f32,
                size[1] as f32,
                1.0 / size[0] as f32,
                1.0 / size[1] as f32,
            ],
            limits: [far * 0.999, 0.75, TILE as f32, 0.0],
        })
    }

    pub fn bytes(&self) -> Vec<u8> {
        let mut values: Vec<f32> = Vec::with_capacity(24);
        for vector in [
            self.across,
            self.down,
            self.corner,
            self.origin,
            self.size,
            self.limits,
        ] {
            values.extend(vector);
        }
        bytemuck::cast_slice(&values).to_vec()
    }
}

pub(crate) struct Readback {
    buffer: wgpu::Buffer,
    pending: Option<Receiver<Result<(), wgpu::BufferAsyncError>>>,
    encoded: bool,
}

impl Readback {
    fn new(device: &wgpu::Device, size: u64, label: &str) -> Self {
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            pending: None,
            encoded: false,
        }
    }

    pub fn free(&self) -> bool {
        self.pending.is_none() && !self.encoded
    }

    pub fn submitted(&mut self) {
        if !self.encoded {
            return;
        }
        self.encoded = false;
        let (sender, receiver) = std::sync::mpsc::channel();
        self.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.pending = Some(receiver);
    }

    pub fn take(&mut self) -> Option<Vec<u8>> {
        let receiver = self.pending.as_ref()?;
        match receiver.try_recv() {
            Ok(Ok(())) => {
                let bytes = self.buffer.slice(..).get_mapped_range().to_vec();
                self.buffer.unmap();
                self.pending = None;
                Some(bytes)
            }
            Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                self.pending = None;
                None
            }
            Err(TryRecvError::Empty) => None,
        }
    }
}

pub(crate) struct Image {
    pub view: wgpu::TextureView,
}

fn image(
    device: &wgpu::Device,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    label: &str,
) -> Image {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    Image {
        view: texture.create_view(&Default::default()),
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Last {
    Colour,
    Scratch,
    Warp(WarpUniform),
}

pub(crate) struct Cached {
    pub view: Matrix,
    pub projection: Matrix,
    pub far: f32,
    pub depth_index: usize,
    pub shadow: Vec<u8>,
    pub frame_number: u32,
    pub exposure: f32,
}

pub(crate) struct Cache {
    pub size: [u32; 2],
    pub colour: Image,
    pub still: Image,
    pub layer: Image,
    pub depth: Image,
    pub scratch: Image,
    pub tiles: Image,
    raw_tiles: Image,
    pub limits: Limits,
    pub book: Book,
    pub cached: Option<Cached>,
    pub exposure_moving: bool,
    pub colour_layers: bool,
    pub last: Last,
    pub overlaid: bool,
    pub overlay_print: Option<u64>,
    pub overlay: Image,
    layout: wgpu::BindGroupLayout,
    module: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    pipelines: Vec<(wgpu::TextureFormat, bool, wgpu::RenderPipeline)>,
    copy_layout: wgpu::BindGroupLayout,
    copy_pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    holes: wgpu::Buffer,
    wake: Option<(wgpu::ComputePipeline, wgpu::BindGroup)>,
    tiler: wgpu::ComputePipeline,
    spreader: wgpu::ComputePipeline,
    composite_layout: wgpu::BindGroupLayout,
    composite_pipeline_layout: wgpu::PipelineLayout,
    composite_module: wgpu::ShaderModule,
    composites: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    holes_back: Readback,
    depth_back: Readback,
}

const COLOUR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

impl Cache {
    pub fn new(device: &wgpu::Device, size: [u32; 2], limits: Limits) -> Self {
        let colour = image(
            device,
            size,
            COLOUR,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING,
            "demand colour",
        );
        let still = image(
            device,
            size,
            COLOUR,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            "demand still",
        );
        let layer = image(
            device,
            size,
            COLOUR,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING,
            "demand layer",
        );
        let depth = image(
            device,
            size,
            wgpu::TextureFormat::R32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            "demand warped depth",
        );
        let scratch = image(
            device,
            size,
            COLOUR,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            "demand post",
        );
        let tiles = image(
            device,
            [size[0].div_ceil(TILE), size[1].div_ceil(TILE)],
            wgpu::TextureFormat::Rg32Float,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            "demand depth tiles",
        );
        let raw_tiles = image(
            device,
            [size[0].div_ceil(TILE), size[1].div_ceil(TILE)],
            wgpu::TextureFormat::Rg32Float,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            "demand raw depth tiles",
        );
        let compute = |source: &str, entry: &str, label: &str| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let tiler = compute(TILES_WGSL, "reduce", "demand depth tiles");
        let spreader = compute(SPREAD_WGSL, "spread", "demand depth spread");
        let float = |binding: u32, filterable: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("demand warp"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                float(1, true),
                float(2, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                float(5, false),
                float(6, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("demand warp"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("demand warp"),
            source: wgpu::ShaderSource::Wgsl(WARP_WGSL.into()),
        });
        let overlay = image(
            device,
            size,
            COLOUR,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            "demand overlay",
        );
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("demand composite"),
            entries: &[float(0, false), float(1, false)],
        });
        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("demand composite"),
                bind_group_layouts: &[&composite_layout],
                push_constant_ranges: &[],
            });
        let composite_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("demand composite"),
            source: wgpu::ShaderSource::Wgsl(COMPOSITE_WGSL.into()),
        });
        let copy_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("demand copy"),
            entries: &[float(0, false)],
        });
        let copy_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("demand copy"),
            bind_group_layouts: &[&copy_layout],
            push_constant_ranges: &[],
        });
        let copy_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("demand copy"),
            source: wgpu::ShaderSource::Wgsl(COPY_WGSL.into()),
        });
        let copy_pipeline = pipeline(
            device,
            &copy_pipeline_layout,
            &copy_module,
            "fragment",
            &[COLOUR],
            "demand copy",
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("demand warp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let holes = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("demand holes"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            size,
            colour,
            still,
            layer,
            depth,
            scratch,
            tiles,
            raw_tiles,
            limits,
            book: Book::default(),
            cached: None,
            exposure_moving: false,
            colour_layers: false,
            last: Last::Colour,
            overlaid: false,
            overlay_print: None,
            overlay,
            layout,
            module,
            pipeline_layout,
            pipelines: Vec::new(),
            copy_layout,
            copy_pipeline,
            sampler,
            holes,
            wake: None,
            tiler,
            spreader,
            composite_layout,
            composite_pipeline_layout,
            composite_module,
            composites: Vec::new(),
            holes_back: Readback::new(device, 16, "demand holes readback"),
            depth_back: Readback::new(device, DEPTH_PROBES as u64 * 256, "demand depth readback"),
        }
    }

    pub fn stats(&self) -> DemandStats {
        DemandStats {
            shown: self.book.kind,
            since_full: self.book.since_full,
            settle: self.book.settle,
            holes: self.book.holes,
            depth: self.book.depth,
        }
    }

    pub fn gather(&mut self) {
        if let Some(bytes) = self.holes_back.take() {
            let count = u32::from_le_bytes(bytes[..4].try_into().unwrap());
            let pixels = (u64::from(self.size[0]) * u64::from(self.size[1])).max(1);
            self.book.holes = count as f32 / pixels as f32;
        }
        if let Some(bytes) = self.depth_back.take() {
            let far = self
                .cached
                .as_ref()
                .map_or(f32::INFINITY, |cached| cached.far);
            let mut depths: Vec<f32> = (0..DEPTH_PROBES)
                .map(|index| {
                    f32::from_le_bytes(bytes[index * 256..index * 256 + 4].try_into().unwrap())
                })
                .filter(|depth| depth.is_finite() && *depth > 0.0 && *depth < far * 0.999)
                .collect();
            depths.sort_by(f32::total_cmp);
            self.book.depth = Some(
                depths
                    .get(depths.len() / 2)
                    .copied()
                    .unwrap_or(f32::INFINITY),
            );
        }
    }

    pub fn submitted(&mut self) {
        self.holes_back.submitted();
        self.depth_back.submitted();
    }

    pub fn probe_depth(&mut self, encoder: &mut wgpu::CommandEncoder, depth: &wgpu::Texture) {
        if !self.depth_back.free() {
            return;
        }
        let [width, height] = self.size;
        for index in 0..DEPTH_PROBES {
            let column = (index % 4) as u32;
            let row = (index / 4) as u32;
            let x = ((2 * column + 1) * width / 8).min(width - 1);
            let y = ((2 * row + 1) * height / 8).min(height - 1);
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: depth,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &self.depth_back.buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: index as u64 * 256,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(1),
                    },
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.depth_back.encoded = true;
    }

    pub fn encode_clear(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("demand overlay clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.overlay.view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }

    pub fn encode_composite(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        base: &wgpu::TextureView,
        target: (&wgpu::TextureView, wgpu::TextureFormat),
        profiler: &mut GpuProfiler,
    ) {
        let index = match self
            .composites
            .iter()
            .position(|(format, _)| *format == target.1)
        {
            Some(index) => index,
            None => {
                let built = pipeline(
                    device,
                    &self.composite_pipeline_layout,
                    &self.composite_module,
                    "fragment",
                    &[target.1],
                    "demand composite",
                );
                self.composites.push((target.1, built));
                self.composites.len() - 1
            }
        };
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("demand composite"),
            layout: &self.composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(base),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.overlay.view),
                },
            ],
        });
        let timing = profiler.pass("composite");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("demand composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.0,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: profiler.render_writes(timing),
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.composites[index].1);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }

    pub fn encode_tiles(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        profiler: &mut GpuProfiler,
    ) {
        let [width, height] = [self.size[0].div_ceil(TILE), self.size[1].div_ceil(TILE)];
        let timing = profiler.pass("demand tiles");
        let writes = profiler.compute_writes(timing);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("demand depth tiles"),
            timestamp_writes: writes,
        });
        for (pipeline, source, target) in [
            (&self.tiler, depth, &self.raw_tiles.view),
            (&self.spreader, &self.raw_tiles.view, &self.tiles.view),
        ] {
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("demand depth tiles"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(target),
                    },
                ],
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
    }

    pub fn encode_wake(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        groups: u32,
        profiler: &mut GpuProfiler,
    ) {
        let (pipeline, bind) = self.wake.get_or_insert_with(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("demand wake"),
                source: wgpu::ShaderSource::Wgsl(WAKE_WGSL.into()),
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("demand wake"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            let sink = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("demand wake"),
                size: 16,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("demand wake"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: sink.as_entire_binding(),
                }],
            });
            (pipeline, bind)
        });
        let timing = profiler.pass("wake");
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("demand wake"),
            timestamp_writes: profiler.compute_writes(timing),
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &*bind, &[]);
        pass.dispatch_workgroups(groups.max(1), 1, 1);
    }

    fn warp_pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        layered: bool,
    ) -> usize {
        if let Some(index) = self
            .pipelines
            .iter()
            .position(|(known, kind, _)| *known == format && *kind == layered)
        {
            return index;
        }
        let built = if layered {
            pipeline(
                device,
                &self.pipeline_layout,
                &self.module,
                "layered",
                &[format, wgpu::TextureFormat::R32Float],
                "demand warp layered",
            )
        } else {
            pipeline(
                device,
                &self.pipeline_layout,
                &self.module,
                "fragment",
                &[format],
                "demand warp",
            )
        };
        self.pipelines.push((format, layered, built));
        self.pipelines.len() - 1
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode_warp(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        uniform: &WarpUniform,
        source: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        target: (&wgpu::TextureView, wgpu::TextureFormat),
        layered: bool,
        profiler: &mut GpuProfiler,
    ) {
        let index = self.warp_pipeline(device, target.1, layered);
        let buffer = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("demand warp"),
                contents: &uniform.bytes(),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("demand warp"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.holes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&self.tiles.view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&self.overlay.view),
                },
            ],
        });
        let measure = self.holes_back.free();
        if measure {
            encoder.clear_buffer(&self.holes, 0, None);
        }
        let timing = profiler.pass("reproject");
        {
            let mut attachments = vec![Some(wgpu::RenderPassColorAttachment {
                view: target.0,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })];
            if layered {
                attachments.push(Some(wgpu::RenderPassColorAttachment {
                    view: &self.depth.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }));
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("demand reproject"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: profiler.render_writes(timing),
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipelines[index].2);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        if measure {
            encoder.copy_buffer_to_buffer(&self.holes, 0, &self.holes_back.buffer, 0, 16);
            self.holes_back.encoded = true;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode_copy(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
        regions: &[Region],
        label: &'static str,
        profiler: &mut GpuProfiler,
    ) {
        if regions.is_empty() {
            return;
        }
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("demand copy"),
            layout: &self.copy_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            }],
        });
        let timing = profiler.pass(label);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: profiler.render_writes(timing),
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.copy_pipeline);
        pass.set_bind_group(0, &bind, &[]);
        for region in regions {
            let region = region.clip(self.size[0], self.size[1]);
            if region.is_empty() {
                continue;
            }
            pass.set_scissor_rect(region.x, region.y, region.width, region.height);
            pass.draw(0..3, 0..1);
        }
    }
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    entry: &str,
    formats: &[wgpu::TextureFormat],
    label: &str,
) -> wgpu::RenderPipeline {
    let targets: Vec<Option<wgpu::ColorTargetState>> = formats
        .iter()
        .map(|format| {
            Some(wgpu::ColorTargetState {
                format: *format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })
        })
        .collect();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vertex"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(entry),
            targets: &targets,
            compilation_options: Default::default(),
        }),
        multiview: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 1280;
    const H: u32 = 800;

    fn projection(scale: f32) -> Matrix {
        let near = 0.05;
        let far = 40.0;
        [
            [scale / 1.6, 0.0, 0.0, 0.0],
            [0.0, scale, 0.0, 0.0],
            [0.0, 0.0, far / (near - far), -1.0],
            [0.0, 0.0, far * near / (near - far), 0.0],
        ]
    }

    fn view(yaw: f32, position: [f32; 3]) -> Matrix {
        let (s, c) = yaw.sin_cos();
        let right = [c, 0.0, -s];
        let up = [0.0, 1.0, 0.0];
        let back = [s, 0.0, c];
        let dot = |a: [f32; 3]| a[0] * position[0] + a[1] * position[1] + a[2] * position[2];
        [
            [right[0], up[0], back[0], 0.0],
            [right[1], up[1], back[1], 0.0],
            [right[2], up[2], back[2], 0.0],
            [-dot(right), -dot(up), -dot(back), 1.0],
        ]
    }

    fn summary() -> Summary {
        Summary {
            view: view(0.0, [0.0, 1.0, 2.0]),
            projection: projection(1.5),
            position: [0.0, 1.0, 2.0],
            size: [W, H],
            world: 7,
            anchors: Vec::new(),
            overlay: 3,
            ticking: false,
            layered: Vec::new(),
            moving: Vec::new(),
        }
    }

    fn turned(degrees: f32) -> Summary {
        Summary {
            view: view(degrees.to_radians(), [0.0, 1.0, 2.0]),
            ..summary()
        }
    }

    fn settled(shown: Summary) -> Book {
        let limits = Limits {
            settle: 0,
            ..Limits::default()
        };
        let mut book = Book::default();
        record(&mut book, shown, Shown::Full, &limits);
        book.depth = Some(2.0);
        book.still = true;
        book
    }

    #[test]
    fn regions_clip_grow_and_merge() {
        let region = Region::new(10, 20, 30, 40);
        assert_eq!(region.grow(15, W, H), Region::new(0, 5, 55, 70));
        assert_eq!(
            Region::new(1270, 790, 50, 50).clip(W, H),
            Region::new(1270, 790, 10, 10)
        );
        assert!(Region::new(1300, 10, 5, 5).clip(W, H).is_empty());
        assert_eq!(
            Region::from_bounds([10.2, 20.9, 40.1, 60.0], W, H),
            Some(Region::new(10, 20, 31, 40))
        );
        assert_eq!(Region::from_bounds([50.0, 50.0, 40.0, 60.0], W, H), None);
        assert_eq!(
            Region::from_bounds([f32::NAN, 0.0, 1.0, 1.0], W, H),
            Some(Region::whole(W, H))
        );
        let merged = merge(
            &[
                Region::new(0, 0, 10, 10),
                Region::new(20, 0, 10, 10),
                Region::new(600, 400, 10, 10),
            ],
            W,
            H,
        );
        assert_eq!(
            merged,
            vec![Region::new(0, 0, 30, 10), Region::new(600, 400, 10, 10)]
        );
        assert_eq!(
            merge(&[Region::new(0, 0, 1000, 700)], W, H),
            vec![Region::whole(W, H)]
        );
        assert!(merge(&[Region::new(0, 0, 0, 10)], W, H).is_empty());
    }

    #[test]
    fn motion_measures_turns_shifts_and_zoom() {
        let base = summary();
        fn at(s: &Summary) -> (&Matrix, &Matrix, [f32; 3]) {
            (&s.view, &s.projection, s.position)
        }
        let still = motion(at(&base), at(&base), None).unwrap();
        assert!(still.angle.abs() < 1e-6 && still.zoom.abs() < 1e-6);
        let yaw = turned(0.25);
        let moved = motion(at(&base), at(&yaw), None).unwrap();
        assert!((moved.angle.to_degrees() - 0.25).abs() < 2e-3, "{moved:?}");
        let tiny = turned(0.002);
        let small = motion(at(&base), at(&tiny), None).unwrap();
        assert!((small.angle.to_degrees() - 0.002).abs() < 5e-4, "{small:?}");
        let pushed = Summary {
            view: view(0.0, [0.0, 1.0, 1.98]),
            position: [0.0, 1.0, 1.98],
            ..summary()
        };
        assert_eq!(motion(at(&base), at(&pushed), None), None);
        let shift = motion(at(&base), at(&pushed), Some(2.0)).unwrap();
        assert!((shift.angle - 0.01).abs() < 1e-4, "{shift:?}");
        let sky = motion(at(&base), at(&pushed), Some(f32::INFINITY)).unwrap();
        assert!(sky.angle < 1e-4);
        let zoomed = Summary {
            projection: projection(1.53),
            ..summary()
        };
        let zoom = motion(at(&base), at(&zoomed), None).unwrap();
        assert!((zoom.zoom - (1.53_f32 / 1.5).ln()).abs() < 1e-5);
        let mut squeezed = summary();
        squeezed.projection[0][0] *= 1.1;
        assert_eq!(motion(at(&base), at(&squeezed), None), None);
        let mut nearer = summary();
        nearer.projection[3][2] *= 0.5;
        assert_eq!(motion(at(&base), at(&nearer), None), None);
    }

    #[test]
    fn nothing_cached_needs_a_full_frame() {
        assert_eq!(
            decide(&Book::default(), &summary(), &Limits::default()),
            Need::Full
        );
    }

    #[test]
    fn an_unchanged_frame_needs_nothing() {
        let book = settled(summary());
        assert_eq!(decide(&book, &summary(), &Limits::default()), Need::None);
    }

    #[test]
    fn each_kind_of_change_is_seen() {
        let limits = Limits::default();
        let book = settled(summary());
        let world = Summary {
            world: 8,
            ..summary()
        };
        assert_eq!(decide(&book, &world, &limits), Need::Full);
        let ticking = Summary {
            ticking: true,
            ..summary()
        };
        assert_eq!(decide(&book, &ticking, &limits), Need::Full);
        let resized = Summary {
            size: [W, H + 2],
            ..summary()
        };
        assert_eq!(decide(&book, &resized, &limits), Need::Full);
        let mut anchored = settled(Summary {
            anchors: vec![view(0.0, [0.0, 0.0, 0.0])],
            ..summary()
        });
        anchored.depth = Some(2.0);
        let nudged = Summary {
            anchors: vec![view(0.0, [0.0, 0.0, 1e-6])],
            ..summary()
        };
        assert_eq!(decide(&anchored, &nudged, &limits), Need::None);
        let slid = Summary {
            anchors: vec![view(0.0, [0.0, 0.01, 0.0])],
            ..summary()
        };
        assert_eq!(decide(&anchored, &slid, &limits), Need::Full);
        let overlay = Summary {
            overlay: 4,
            ..summary()
        };
        assert_eq!(
            decide(&book, &overlay, &limits),
            Need::Partial(vec![Region::whole(W, H)])
        );
        let moving = Summary {
            moving: vec![Region::new(10, 10, 40, 20)],
            ..summary()
        };
        assert_eq!(
            decide(&book, &moving, &limits),
            Need::Partial(vec![Region::new(10, 10, 40, 20)])
        );
    }

    #[test]
    fn small_moves_reproject_and_large_ones_redraw() {
        let limits = Limits::default();
        let book = settled(summary());
        assert_eq!(decide(&book, &turned(0.3), &limits), Need::Reproject);
        assert_eq!(decide(&book, &turned(0.7), &limits), Need::Full);
        let zoomed = Summary {
            projection: projection(1.5 * 1.01),
            ..summary()
        };
        assert_eq!(decide(&book, &zoomed, &limits), Need::Reproject);
        let far_zoom = Summary {
            projection: projection(1.5 * 1.05),
            ..summary()
        };
        assert_eq!(decide(&book, &far_zoom, &limits), Need::Full);
        let mut unknown = book.clone();
        unknown.depth = None;
        let pushed = Summary {
            view: view(0.0, [0.0, 1.0, 1.995]),
            position: [0.0, 1.0, 1.995],
            ..summary()
        };
        assert_eq!(decide(&unknown, &pushed, &limits), Need::Full);
        assert_eq!(decide(&book, &pushed, &limits), Need::Reproject);
        let mut worn = book.clone();
        worn.since_full = limits.frames;
        assert_eq!(decide(&worn, &turned(0.1), &limits), Need::Full);
        let mut holed = book.clone();
        holed.holes = limits.holes * 2.0;
        assert_eq!(decide(&holed, &turned(0.1), &limits), Need::Full);
    }

    #[test]
    fn a_camera_at_rest_after_reprojecting_gets_a_fresh_frame() {
        let limits = Limits::default();
        let mut book = settled(summary());
        record(&mut book, turned(0.2), Shown::Reproject, &limits);
        assert_eq!(book.since_full, 1);
        assert_eq!(decide(&book, &turned(0.25), &limits), Need::Reproject);
        assert_eq!(decide(&book, &turned(0.2), &limits), Need::Full);
        assert_eq!(decide(&book, &summary(), &limits), Need::Reproject);
        let steaming = Summary {
            layered: vec![Region::new(100, 100, 50, 80)],
            ..turned(0.2)
        };
        assert_eq!(decide(&book, &steaming, &limits), Need::Reproject);
    }

    #[test]
    fn time_driven_layers_redraw_their_rects_and_where_they_were() {
        let limits = Limits::default();
        let steam = Region::new(100, 100, 50, 80);
        let mut book = settled(Summary {
            layered: vec![steam],
            ..summary()
        });
        let still = Summary {
            layered: vec![steam],
            ..summary()
        };
        assert_eq!(decide(&book, &still, &limits), Need::Partial(vec![steam]));
        let mut bare = book.clone();
        bare.still = false;
        assert_eq!(
            decide(&bare, &still, &limits),
            Need::Full,
            "no still layer was kept"
        );
        let gone = summary();
        assert_eq!(decide(&book, &gone, &limits), Need::Partial(vec![steam]));
        record(&mut book, gone.clone(), Shown::Partial, &limits);
        assert_eq!(decide(&book, &gone, &limits), Need::None);
        let elsewhere = Region::new(900, 500, 40, 40);
        record(
            &mut book,
            Summary {
                layered: vec![steam],
                ..summary()
            },
            Shown::Partial,
            &limits,
        );
        let moved = Summary {
            layered: vec![elsewhere],
            ..summary()
        };
        assert_eq!(
            decide(&book, &moved, &limits),
            Need::Partial(vec![steam, elsewhere])
        );
    }

    #[test]
    fn a_change_settles_the_history_before_going_idle() {
        let limits = Limits::default();
        let mut book = Book::default();
        record(&mut book, summary(), Shown::Full, &limits);
        book.depth = Some(2.0);
        assert_eq!(book.settle, limits.settle);
        let mut fulls = 0;
        while decide(&book, &summary(), &limits) == Need::Full {
            record(&mut book, summary(), Shown::Full, &limits);
            fulls += 1;
            assert!(fulls <= limits.settle);
        }
        assert_eq!(fulls, limits.settle);
        assert_eq!(decide(&book, &summary(), &limits), Need::None);
        record(&mut book, turned(0.2), Shown::Reproject, &limits);
        record(&mut book, turned(0.7), Shown::Full, &limits);
        assert_eq!(book.settle, 0);
        assert_eq!(book.since_full, 0);
        let changed = Summary {
            world: 99,
            ..summary()
        };
        record(&mut book, changed, Shown::Full, &limits);
        assert_eq!(book.settle, limits.settle);
    }

    #[test]
    fn prints_see_single_word_changes() {
        let print = |values: &[f32]| {
            let mut print = Print::new();
            print.floats(values);
            print.finish()
        };
        let base = print(&[1.0, 2.0, 3.0]);
        assert_eq!(base, print(&[1.0, 2.0, 3.0]));
        assert_ne!(base, print(&[1.0, 2.0, 3.0000002]));
        assert_ne!(base, print(&[1.0, 3.0, 2.0]));
        assert_ne!(base, print(&[1.0, 2.0]));
        let mut text = Print::new();
        text.debug(&"ab");
        let mut other = Print::new();
        other.debug(&"ab ");
        assert_ne!(text.finish(), other.finish());
    }

    #[test]
    fn layer_rects_cover_what_they_draw() {
        let projection = projection(1.5);
        let view = view(0.0, [0.0, 1.0, 2.0]);
        let view_projection = multiply(projection, view);
        let region = box_region(
            &view_projection,
            [-0.1, 0.9, -0.1],
            [0.1, 1.1, 0.1],
            8,
            W,
            H,
        )
        .unwrap();
        assert!(region.contains(W / 2, H / 2));
        assert!(region.width < W / 4 && region.height < H / 3, "{region:?}");
        let behind = box_region(&view_projection, [-0.1, 0.9, 2.5], [0.1, 1.1, 2.6], 8, W, H);
        assert_eq!(behind, Some(Region::whole(W, H)));
        let particle = ParticleInstance {
            position_size: [0.0, 1.0, 0.0, 0.05],
            velocity_alpha: [0.0; 4],
            color_rotation: [0.0; 4],
            detail: [0.0; 4],
        };
        let around = particle_region(&view_projection, &[particle], 4, W, H).unwrap();
        assert!(around.contains(W / 2, H / 2));
        assert_eq!(particle_region(&view_projection, &[], 4, W, H), None);
    }

    #[test]
    fn only_pointwise_finishes_redraw_in_rects() {
        use pfx_post::{Bloom, Chain, Pass, Style, Tone};
        let plain = Chain {
            passes: vec![
                Pass::Exposure(1.0),
                Pass::Bloom(Bloom::neutral(0.0)),
                Pass::Tone(Tone::aces()),
            ],
            frame: 0,
            seed: 0,
        };
        assert!(local_post(&plain));
        let glowing = Chain {
            passes: vec![Pass::Bloom(Bloom::neutral(0.4))],
            frame: 0,
            seed: 0,
        };
        assert!(!local_post(&glowing));
        let styled = Style::ALL
            .iter()
            .filter(|style| local_post(&style.chain()))
            .count();
        assert!(styled < Style::ALL.len());
    }

    #[test]
    fn warp_shaders_validate() {
        for source in [
            WARP_WGSL,
            COPY_WGSL,
            WAKE_WGSL,
            TILES_WGSL,
            SPREAD_WGSL,
            COMPOSITE_WGSL,
        ] {
            let module = naga::front::wgsl::parse_str(source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }

    #[test]
    fn warp_uniform_maps_the_cached_view_onto_itself_and_follows_a_turn() {
        let at = |uniform: &WarpUniform, uv: [f32; 2], depth: f32| {
            let toward: [f32; 4] = std::array::from_fn(|row| {
                uniform.across[row] * uv[0] + uniform.down[row] * uv[1] + uniform.corner[row]
            });
            let clip: [f32; 4] =
                std::array::from_fn(|row| toward[row] * depth + uniform.origin[row]);
            [clip[0] / clip[3] * 0.5 + 0.5, 0.5 - clip[1] / clip[3] * 0.5]
        };
        let still = view(0.1, [0.0, 1.0, 2.0]);
        let projection = projection(1.5);
        let same =
            WarpUniform::new((&still, &projection), (&still, &projection), [W, H], 40.0).unwrap();
        for uv in [[0.1, 0.2], [0.5, 0.5], [0.93, 0.71]] {
            for depth in [0.3, 2.0, 25.0] {
                let back = at(&same, uv, depth);
                assert!((back[0] - uv[0]).abs() < 1e-5 && (back[1] - uv[1]).abs() < 1e-5);
            }
        }
        let turned = view(0.1 + 0.5_f32.to_radians(), [0.0, 1.0, 2.0]);
        let warp =
            WarpUniform::new((&still, &projection), (&turned, &projection), [W, H], 40.0).unwrap();
        let near = at(&warp, [0.5, 0.5], 1.0);
        let far = at(&warp, [0.5, 0.5], 20.0);
        let shift = 0.5_f32.to_radians().tan() * 1.5 / 1.6 * 0.5;
        assert!((near[0] - 0.5 - shift).abs() < 1e-3, "{near:?}");
        assert!(
            (near[0] - far[0]).abs() < 1e-5,
            "a pure turn has no parallax"
        );
        assert_eq!(same.bytes().len(), 24 * 4);
    }
}
