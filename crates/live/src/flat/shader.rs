use pfx_post::look::shader::COLOUR_WGSL;

pub const SDF_WGSL: &str = r#"
const FLAT_RECT: u32 = 0u;
const FLAT_CIRCLE: u32 = 1u;
const FLAT_RING: u32 = 2u;
const FLAT_ARC: u32 = 3u;
const FLAT_ICON_FILL: u32 = 4u;
const FLAT_ICON_LINE: u32 = 5u;
const FLAT_TAU: f32 = 6.283185307179586;

fn flat_sign(value: f32) -> f32 {
    return select(-1.0, 1.0, value >= 0.0);
}

fn flat_direction(p: vec2f) -> vec2f {
    let length_p = length(p);
    return select(vec2f(1.0, 0.0), p / length_p, length_p > 0.0);
}

fn flat_corner(p: vec2f, radii: vec4f) -> f32 {
    let right = p.x >= 0.0;
    if (p.y >= 0.0) {
        return select(radii.w, radii.z, right);
    }
    return select(radii.x, radii.y, right);
}

fn flat_rect(p: vec2f, half: vec2f, radii: vec4f) -> vec3f {
    let radius = flat_corner(p, radii);
    let s = vec2f(flat_sign(p.x), flat_sign(p.y));
    let q = abs(p) - half + vec2f(radius);
    if (q.x > 0.0 && q.y > 0.0) {
        let l = length(q);
        return vec3f(l - radius, s * q / l);
    }
    if (q.x > q.y) {
        return vec3f(q.x - radius, s.x, 0.0);
    }
    return vec3f(q.y - radius, 0.0, s.y);
}

fn flat_circle(p: vec2f, radius: f32) -> vec3f {
    return vec3f(length(p) - radius, flat_direction(p));
}

fn flat_ring(p: vec2f, radius: f32, width: f32) -> vec3f {
    let offset = length(p) - radius;
    return vec3f(abs(offset) - width * 0.5, flat_sign(offset) * flat_direction(p));
}

fn flat_turn(p: vec2f, angle: f32) -> vec2f {
    let c = cos(angle);
    let s = sin(angle);
    return vec2f(c * p.x - s * p.y, s * p.x + c * p.y);
}

fn flat_arc(p: vec2f, radius: f32, width: f32, start: f32, sweep: f32) -> vec3f {
    if (sweep >= FLAT_TAU) {
        return flat_ring(p, radius, width);
    }
    let half = sweep * 0.5;
    let middle = start + half;
    let q = flat_turn(p, -middle);
    let angle = atan2(q.y, q.x);
    if (abs(angle) <= half) {
        return flat_ring(p, radius, width);
    }
    let end = radius * vec2f(cos(half), flat_sign(q.y) * sin(half));
    let away = q - end;
    return vec3f(length(away) - width * 0.5, flat_turn(flat_direction(away), middle));
}

struct FlatAlong {
    s: f32,
    tangent: vec2f,
}

fn flat_rect_along(p: vec2f, half: vec2f, radii: vec4f) -> FlatAlong {
    let arc = radii * (FLAT_TAU * 0.25);
    let top = (half.x - radii.x) + (half.x - radii.y);
    let right = (half.y - radii.y) + (half.y - radii.z);
    let bottom = (half.x - radii.w) + (half.x - radii.z);
    let left = (half.y - radii.x) + (half.y - radii.w);
    let at_right = top + arc.y;
    let at_bottom_arc = at_right + right;
    let at_bottom = at_bottom_arc + arc.z;
    let at_left_arc = at_bottom + bottom;
    let at_left = at_left_arc + arc.w;
    let at_top_arc = at_left + left;
    let radius = flat_corner(p, radii);
    let inner = half - vec2f(radius);
    let q = abs(p) - inner;
    if (q.x > 0.0 && q.y > 0.0) {
        let s = vec2f(flat_sign(p.x), flat_sign(p.y));
        let centre = s * inner;
        let offset = p - centre;
        let rho = max(length(offset), 1e-6);
        let tangent = vec2f(-offset.y, offset.x) / rho * (radius / rho);
        var start = top;
        var angle = atan2(offset.y, offset.x) + FLAT_TAU * 0.25;
        if (s.x > 0.0 && s.y > 0.0) {
            start = at_bottom_arc;
            angle = atan2(offset.y, offset.x);
        } else if (s.x < 0.0 && s.y > 0.0) {
            start = at_left_arc;
            angle = atan2(offset.y, offset.x) - FLAT_TAU * 0.25;
        } else if (s.x < 0.0 && s.y < 0.0) {
            start = at_top_arc;
            angle = atan2(-offset.y, -offset.x);
        }
        return FlatAlong(start + radius * clamp(angle, 0.0, FLAT_TAU * 0.25), tangent);
    }
    if (q.x > q.y) {
        if (p.x > 0.0) {
            return FlatAlong(at_right + p.y + (half.y - radii.y), vec2f(0.0, 1.0));
        }
        return FlatAlong(at_left + (half.y - radii.w) - p.y, vec2f(0.0, -1.0));
    }
    if (p.y < 0.0) {
        return FlatAlong(p.x + (half.x - radii.x), vec2f(1.0, 0.0));
    }
    return FlatAlong(at_bottom + (half.x - radii.z) - p.x, vec2f(-1.0, 0.0));
}

fn flat_circle_along(p: vec2f, radius: f32) -> FlatAlong {
    let rho = max(length(p), 1e-6);
    var angle = atan2(p.y, p.x);
    if (angle < 0.0) {
        angle += FLAT_TAU;
    }
    return FlatAlong(radius * angle, vec2f(-p.y, p.x) / rho * (radius / rho));
}

fn flat_dash(along: FlatAlong, on: f32, off: f32, phase: f32) -> vec3f {
    let period = on + off;
    let u = along.s + phase - on * 0.5;
    let centred = u - period * floor(u / period + 0.5);
    return vec3f(abs(centred) - on * 0.5, flat_sign(centred) * along.tangent);
}

fn flat_coverage(field: vec3f, inverse: mat2x2f) -> f32 {
    let screen = vec2f(dot(inverse[0], field.yz), dot(inverse[1], field.yz));
    return clamp(0.5 - field.x / max(length(screen), 1e-6), 0.0, 1.0);
}

fn flat_coverage_even(distance: f32, inverse: mat2x2f) -> f32 {
    let unit = sqrt(abs(determinant(inverse)));
    return clamp(0.5 - distance / max(unit, 1e-6), 0.0, 1.0);
}

const FLAT_SHADOW: u32 = 6u;
const FLAT_SHADOW_STEPS: u32 = 8u;
const FLAT_SHADOW_REACH: f32 = 4.0;

fn flat_erf(x: f32) -> f32 {
    let t = 1.0 / (1.0 + 0.3275911 * abs(x));
    let y = 1.0 - (((((1.0614054 * t - 1.4531521) * t) + 1.4214138) * t - 0.28449674) * t + 0.2548296) * t * exp(-x * x);
    return select(-y, y, x >= 0.0);
}

fn flat_shadow_reach(half: vec2f, radius: f32, y: f32) -> f32 {
    let delta = min(half.y - radius - abs(y), 0.0);
    return half.x - radius + sqrt(max(radius * radius - delta * delta, 0.0));
}

fn flat_shadow_row(x: f32, y: f32, half: vec2f, radii: vec4f, sigma: f32) -> f32 {
    var left = radii.w;
    var right = radii.z;
    if (y < 0.0) {
        left = radii.x;
        right = radii.y;
    }
    let k = 0.70710678 / sigma;
    return 0.5 * (flat_erf((x + flat_shadow_reach(half, left, y)) * k) - flat_erf((x - flat_shadow_reach(half, right, y)) * k));
}

fn flat_rounded_shadow(p: vec2f, half: vec2f, radii: vec4f, sigma: vec2f) -> f32 {
    let k = 0.70710678 / sigma.y;
    let low = max(-half.y, p.y - FLAT_SHADOW_REACH * sigma.y);
    let high = min(half.y, p.y + FLAT_SHADOW_REACH * sigma.y);
    if (high <= low) {
        return 0.0;
    }
    let first = asin(clamp(low / half.y, -1.0, 1.0));
    let last = asin(clamp(high / half.y, -1.0, 1.0));
    let step = (last - first) / f32(FLAT_SHADOW_STEPS);
    var total = 0.0;
    var edge = flat_erf((p.y - low) * k);
    for (var index = 0u; index < FLAT_SHADOW_STEPS; index += 1u) {
        let a = first + step * f32(index);
        let b = a + step;
        let next = flat_erf((p.y - half.y * sin(b)) * k);
        let weight = 0.5 * (edge - next);
        edge = next;
        total += weight * flat_shadow_row(p.x, half.y * sin((a + b) * 0.5), half, radii, sigma.x);
    }
    return total;
}

fn flat_stripe(u: f32, width: f32) -> vec2f {
    let c = u - floor(u + 0.5);
    return vec2f(abs(c) - width * 0.5, flat_sign(c));
}

fn flat_pattern_field(kind: u32, params: vec4f, p: vec2f) -> vec3f {
    let scale = params.x;
    let width = params.z;
    let q = flat_turn(p, -params.y) / scale;
    var d = 0.0;
    var g = vec2f(1.0, 0.0);
    if (kind == 1u) {
        let s = flat_stripe(q.x, width);
        d = s.x;
        g = vec2f(s.y, 0.0);
    } else if (kind == 2u) {
        let f = q - floor(q + vec2f(0.5));
        d = length(f) - width * 0.5;
        g = flat_direction(f);
    } else if (kind == 3u) {
        let a = flat_stripe(q.x, width);
        let b = flat_stripe(q.y, width);
        if (a.x <= b.x) {
            d = a.x;
            g = vec2f(a.y, 0.0);
        } else {
            d = b.x;
            g = vec2f(0.0, b.y);
        }
    } else {
        let t = q.y - floor(q.y + 0.5);
        let s = flat_stripe(q.x + abs(t), width);
        d = s.x * 0.70710678;
        g = vec2f(s.y, s.y * flat_sign(t)) * 0.70710678;
    }
    return vec3f(d * scale, flat_turn(g, params.y));
}

fn flat_pattern_over(base: vec4f, kind: u32, params: vec4f, packed: u32, p: vec2f, inverse: mat2x2f) -> vec4f {
    let colour = unpack4x8unorm(packed);
    let cover = flat_coverage(flat_pattern_field(kind, params, p), inverse);
    let paint = vec4f(colour.rgb * colour.a, colour.a) * cover;
    return paint + base * (1.0 - paint.a);
}
"#;

pub const PASS_WGSL: &str = r#"
const FLAT_FILL_SHIFT: u32 = 4u;
const FLAT_DASHED: u32 = 256u;

struct FlatView {
    size: vec4f,
    light: vec4f,
    environment: vec4f,
}

@group(0) @binding(0) var<uniform> flat_view: FlatView;
@group(0) @binding(1) var flat_icons: texture_2d<f32>;
@group(0) @binding(2) var flat_icon_sampler: sampler;
@group(0) @binding(3) var flat_environment: texture_2d<f32>;
@group(0) @binding(4) var flat_environment_sampler: sampler;
@group(0) @binding(5) var flat_gradients: texture_2d<f32>;
@group(0) @binding(6) var flat_sprites: texture_2d<f32>;
@group(0) @binding(7) var flat_sprite_sampler: sampler;

struct FlatInstance {
    @location(0) axes: vec4f,
    @location(1) origin: vec4f,
    @location(2) bounds: vec4f,
    @location(3) shape: vec4f,
    @location(4) stroke: vec4f,
    @location(5) fill_top: vec4f,
    @location(6) fill_bottom: vec4f,
    @location(7) stroke_colour: vec4f,
    @location(8) icon: vec4f,
    @location(9) info: vec4u,
    @location(10) material: vec4f,
    @location(11) frame_x: vec4f,
    @location(12) frame_y: vec4f,
    @location(13) frame_z: vec4f,
    @location(14) corners: vec4f,
}

struct FlatVaryings {
    @builtin(position) position: vec4f,
    @location(0) local: vec2f,
    @location(1) @interpolate(flat) inverse: vec4f,
    @location(2) @interpolate(flat) origin: vec4f,
    @location(3) @interpolate(flat) shape: vec4f,
    @location(4) @interpolate(flat) stroke: vec4f,
    @location(5) @interpolate(flat) fill_top: vec4f,
    @location(6) @interpolate(flat) fill_bottom: vec4f,
    @location(7) @interpolate(flat) stroke_colour: vec4f,
    @location(8) @interpolate(flat) icon: vec4f,
    @location(9) @interpolate(flat) info: vec4u,
    @location(10) @interpolate(flat) material: vec4f,
    @location(11) @interpolate(flat) frame_x: vec4f,
    @location(12) @interpolate(flat) frame_y: vec4f,
    @location(13) @interpolate(flat) frame_z: vec4f,
    @location(14) @interpolate(flat) corners: vec4f,
}

fn flat_place(instance: FlatInstance, index: u32) -> FlatVaryings {
    let corner = vec2f(f32(index & 1u), f32((index >> 1u) & 1u)) * 2.0 - 1.0;
    let axes = mat2x2f(instance.axes.xy, instance.axes.zw);
    let inverse = mat2x2f(vec2f(axes[1].y, -axes[0].y), vec2f(-axes[1].x, axes[0].x))
        * (1.0 / determinant(axes));
    let margin = 1.5 * vec2f(
        length(vec2f(inverse[0].x, inverse[1].x)),
        length(vec2f(inverse[0].y, inverse[1].y)),
    );
    let local = instance.bounds.xy + corner * (instance.bounds.zw + margin);
    let pixel = axes * local + instance.origin.xy;
    var out: FlatVaryings;
    out.position = vec4f(
        pixel.x * flat_view.size.z * 2.0 - 1.0,
        1.0 - pixel.y * flat_view.size.w * 2.0,
        0.0,
        1.0,
    );
    out.local = local;
    out.inverse = vec4f(inverse[0], inverse[1]);
    out.origin = instance.origin;
    out.shape = instance.shape;
    out.stroke = instance.stroke;
    out.fill_top = instance.fill_top;
    out.fill_bottom = instance.fill_bottom;
    out.stroke_colour = instance.stroke_colour;
    out.icon = instance.icon;
    out.info = instance.info;
    out.material = instance.material;
    out.frame_x = instance.frame_x;
    out.frame_y = instance.frame_y;
    out.frame_z = instance.frame_z;
    out.corners = instance.corners;
    return out;
}

@vertex
fn flat_vertex(instance: FlatInstance, @builtin(vertex_index) index: u32) -> FlatVaryings {
    return flat_place(instance, index);
}

@vertex
fn flat_id_vertex(instance: FlatInstance, @builtin(vertex_index) index: u32) -> FlatVaryings {
    var out = flat_place(instance, index);
    if ((instance.info.x & 15u) == FLAT_SHADOW) {
        out.position = vec4f(2.0, 2.0, 2.0, 1.0);
    }
    return out;
}

struct FlatPaint {
    colour: vec4f,
    footprint: f32,
}

fn flat_field(v: FlatVaryings, kind: u32, p: vec2f) -> vec3f {
    if (kind == FLAT_RECT) {
        return flat_rect(p, v.shape.xy, v.corners);
    } else if (kind == FLAT_CIRCLE) {
        return flat_circle(p, v.shape.x);
    } else if (kind == FLAT_RING) {
        return flat_ring(p, v.shape.x, v.shape.y);
    } else if (kind == FLAT_ARC) {
        return flat_arc(p, v.shape.x, v.shape.y, v.shape.z, v.shape.w);
    }
    return vec3f(textureSampleLevel(flat_icons, flat_icon_sampler, v.icon.xy + p * v.icon.zw, 0.0).r, 0.0, 0.0);
}

fn flat_stroke_cover(v: FlatVaryings, kind: u32, p: vec2f, field: vec3f, inverse: mat2x2f) -> f32 {
    let icon = kind == FLAT_ICON_FILL || kind == FLAT_ICON_LINE;
    let band = vec3f(abs(field.x) - v.stroke.x * 0.5, flat_sign(field.x) * field.yz);
    var cover = select(flat_coverage(band, inverse), flat_coverage_even(band.x, inverse), icon);
    if ((v.info.x & FLAT_DASHED) != 0u) {
        var along = flat_circle_along(p, v.shape.x);
        if (kind == FLAT_RECT) {
            along = flat_rect_along(p, v.shape.xy, v.corners);
        }
        cover *= flat_coverage(flat_dash(along, v.stroke.y, v.stroke.z, v.stroke.w), inverse);
    }
    return cover;
}

fn flat_gradient_offset(row: i32, index: u32) -> f32 {
    return textureLoad(flat_gradients, vec2i(8 + i32(index / 4u), row), 0)[index % 4u];
}

fn flat_gradient_stop(row: i32, index: u32) -> vec4f {
    return textureLoad(flat_gradients, vec2i(i32(index), row), 0);
}

fn flat_gradient(v: FlatVaryings, p: vec2f) -> vec4f {
    let row = i32(v.fill_bottom.x);
    let count = u32(v.fill_bottom.y);
    let mode = u32(v.fill_bottom.z);
    let oklab = (mode & 2u) != 0u;
    let t = look_gradient_position(f32(mode & 1u), v.fill_top, p);
    var colour = flat_gradient_stop(row, count - 1u);
    if (t <= flat_gradient_offset(row, 0u)) {
        colour = flat_gradient_stop(row, 0u);
    } else {
        for (var i = 0u; i + 1u < count; i += 1u) {
            let a = flat_gradient_offset(row, i);
            let b = flat_gradient_offset(row, i + 1u);
            if (t < b) {
                colour = look_stop_mix(flat_gradient_stop(row, i), flat_gradient_stop(row, i + 1u), (t - a) / (b - a), oklab);
                break;
            }
        }
    }
    return vec4f(colour.rgb * colour.a, colour.a);
}

fn flat_fill_base(v: FlatVaryings, fill_mode: u32, p: vec2f) -> vec4f {
    if (fill_mode == 3u) {
        return flat_gradient(v, p);
    }
    if (fill_mode == 4u) {
        let texel = textureSampleLevel(flat_sprites, flat_sprite_sampler, v.icon.xy + p * v.icon.zw, 0.0) * v.fill_top;
        return vec4f(texel.rgb * texel.a, texel.a);
    }
    var t = 0.0;
    if (fill_mode == 2u) {
        t = clamp((p.y - v.origin.z) / (v.origin.w - v.origin.z), 0.0, 1.0);
    }
    let colour = mix(v.fill_top, v.fill_bottom, t);
    return vec4f(colour.rgb * colour.a, colour.a);
}

fn flat_fill_alpha(v: FlatVaryings, fill_mode: u32) -> f32 {
    return select(max(v.fill_top.a, v.fill_bottom.a), v.fill_bottom.a, fill_mode == 3u);
}

fn flat_paint(v: FlatVaryings) -> FlatPaint {
    let p = v.local;
    let inverse = mat2x2f(v.inverse.xy, v.inverse.zw);
    let kind = v.info.x & 15u;
    if (kind == FLAT_SHADOW) {
        let cover = flat_rounded_shadow(p - v.icon.zw, v.shape.xy, v.corners, v.icon.xy);
        return FlatPaint(vec4f(v.fill_top.rgb * v.fill_top.a, v.fill_top.a) * cover, 0.0);
    }
    let fill_mode = (v.info.x >> FLAT_FILL_SHIFT) & 15u;
    let opacity = bitcast<f32>(v.info.w);
    let icon = kind == FLAT_ICON_FILL || kind == FLAT_ICON_LINE;
    let field = flat_field(v, kind, p);
    var fill = vec4f(0.0);
    var fill_cover = 0.0;
    if (fill_mode != 0u) {
        fill_cover = select(flat_coverage(field, inverse), flat_coverage_even(field.x, inverse), icon);
        var base = flat_fill_base(v, fill_mode, p);
        let pattern = (v.info.x >> 12u) & 7u;
        if (pattern != 0u) {
            base = flat_pattern_over(base, pattern, v.icon, v.info.z, p, inverse);
        }
        fill = base * fill_cover;
        fill = flat_bevel_over(v, kind, p, field, inverse, fill, fill_cover);
    }
    var stroke = vec4f(0.0);
    var stroke_cover = 0.0;
    if (v.stroke.x > 0.0) {
        stroke_cover = flat_stroke_cover(v, kind, p, field, inverse);
        stroke = vec4f(v.stroke_colour.rgb * v.stroke_colour.a, v.stroke_colour.a) * stroke_cover;
    }
    let colour = (stroke + fill * (1.0 - stroke.a)) * opacity;
    let fill_seen = select(0.0, fill_cover, fill_mode != 0u && flat_fill_alpha(v, fill_mode) > 0.0);
    let stroke_seen = select(0.0, stroke_cover, v.stroke_colour.a > 0.0);
    let footprint = select(0.0, max(fill_seen, stroke_seen), opacity > 0.0);
    return FlatPaint(colour, footprint);
}

@fragment
fn flat_colour(v: FlatVaryings) -> @location(0) vec4f {
    return flat_paint(v).colour;
}

@fragment
fn flat_id(v: FlatVaryings) -> @location(0) u32 {
    if (flat_paint(v).footprint < 0.5) {
        discard;
    }
    return v.info.y;
}

struct FlatLayerVaryings {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) opacity: f32,
}

@vertex
fn flat_layer_vertex(@location(0) rect: vec4f, @location(1) opacity: vec4f, @builtin(vertex_index) index: u32) -> FlatLayerVaryings {
    let corner = vec2f(f32(index & 1u), f32((index >> 1u) & 1u));
    let pixel = mix(rect.xy, rect.zw, corner);
    var out: FlatLayerVaryings;
    out.position = vec4f(pixel.x * flat_view.size.z * 2.0 - 1.0, 1.0 - pixel.y * flat_view.size.w * 2.0, 0.0, 1.0);
    out.opacity = opacity.x;
    return out;
}

@group(1) @binding(0) var flat_layer: texture_2d<f32>;

@fragment
fn flat_layer_clear(v: FlatLayerVaryings) -> @location(0) vec4f {
    return vec4f(0.0);
}

@fragment
fn flat_layer_composite(v: FlatLayerVaryings) -> @location(0) vec4f {
    return textureLoad(flat_layer, vec2i(v.position.xy), 0) * v.opacity;
}
"#;

pub const LIT_WGSL: &str = r#"
const FLAT_SLAB: u32 = 512u;
const FLAT_LIT: u32 = 1024u;
const FLAT_PI: f32 = 3.141592653589793;

fn flat_decode(c: vec3f) -> vec3f {
    return select(pow((c + 0.055) / 1.055, vec3f(2.4)), c / 12.92, c <= vec3f(0.04045));
}

fn flat_encode(c: vec3f) -> vec3f {
    let x = clamp(c, vec3f(0.0), vec3f(1.0));
    return select(1.055 * pow(x, vec3f(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3f(0.0031308));
}

fn flat_footprint(p: vec2f, v: FlatVaryings, kind: u32) -> f32 {
    if (kind == FLAT_RECT) {
        return flat_rect(p, v.shape.xy, v.corners).x;
    } else if (kind == FLAT_ICON_FILL || kind == FLAT_ICON_LINE) {
        return textureSampleLevel(flat_icons, flat_icon_sampler, v.icon.xy + p * v.icon.zw, 0.0).r
            - v.stroke.x * 0.5;
    }
    return length(p) - v.shape.x;
}

fn flat_footprint_field(p: vec2f, v: FlatVaryings, kind: u32, unit: f32) -> vec3f {
    if (kind == FLAT_RECT) {
        return flat_rect(p, v.shape.xy, v.corners);
    } else if (kind == FLAT_CIRCLE) {
        return flat_circle(p, v.shape.x);
    }
    let e = 0.5 * unit;
    let d = flat_footprint(p, v, kind);
    let gradient = vec2f(
        flat_footprint(p + vec2f(e, 0.0), v, kind) - flat_footprint(p - vec2f(e, 0.0), v, kind),
        flat_footprint(p + vec2f(0.0, e), v, kind) - flat_footprint(p - vec2f(0.0, e), v, kind),
    );
    return vec3f(d, flat_direction(gradient));
}

fn flat_slab(q: vec3f, v: FlatVaryings, kind: u32, thickness: f32, bevel: f32) -> f32 {
    let w = vec2f(flat_footprint(q.xy, v, kind) + bevel, abs(q.z + thickness * 0.5) - thickness * 0.5 + bevel);
    return min(max(w.x, w.y), 0.0) + length(max(w, vec2f(0.0))) - bevel;
}

fn flat_ggx(n: vec3f, l: vec3f, v: vec3f, roughness: f32) -> f32 {
    let h = normalize(l + v);
    let nl = max(dot(n, l), 0.0);
    let nv = max(dot(n, v), 1e-4);
    let nh = max(dot(n, h), 0.0);
    let vh = max(dot(v, h), 0.0);
    let a = roughness * roughness;
    let a2 = a * a;
    let denominator = nh * nh * (a2 - 1.0) + 1.0;
    let d = a2 / (FLAT_PI * denominator * denominator);
    let k = a * 0.5;
    let g = nl / (nl * (1.0 - k) + k) * nv / (nv * (1.0 - k) + k);
    let f = 0.04 + 0.96 * pow(1.0 - vh, 5.0);
    return d * g * f / (4.0 * nv);
}

fn flat_reflected(direction: vec3f, roughness: f32) -> vec3f {
    let u = 0.5 + atan2(direction.x, -direction.y) / (2.0 * FLAT_PI);
    let w = acos(clamp(direction.z, -1.0, 1.0)) / FLAT_PI;
    let level = roughness * max(flat_view.environment.x - 1.0, 0.0);
    return textureSampleLevel(flat_environment, flat_environment_sampler, vec2f(u, w), level).rgb
        * flat_view.environment.y;
}

fn flat_lit(v: FlatVaryings) -> FlatPaint {
    let inverse = mat2x2f(v.inverse.xy, v.inverse.zw);
    let kind = v.info.x & 15u;
    let fill_mode = (v.info.x >> FLAT_FILL_SHIFT) & 15u;
    let opacity = bitcast<f32>(v.info.w);
    let icon = kind == FLAT_ICON_FILL || kind == FLAT_ICON_LINE;
    let slab = (v.info.x & FLAT_SLAB) != 0u;
    let unit = sqrt(abs(determinant(inverse)));
    let local_from_world = mat3x3f(v.frame_x.xyz, v.frame_y.xyz, v.frame_z.xyz);
    let world_from_local = transpose(local_from_world);
    let down = -v.frame_z.xyz;
    var hit = vec3f(v.local, 0.0);
    var normal = vec3f(0.0, 0.0, 1.0);
    var cover = 1.0;
    var settled = false;
    if (slab && down.z < -0.99999) {
        let footprint = flat_footprint_field(v.local, v, kind, unit);
        if (footprint.x >= 1.5 * unit) {
            return FlatPaint(vec4f(0.0), 0.0);
        }
        let bevel = v.material.y;
        let across = footprint.x + bevel;
        if (across > 0.0) {
            let x = min(across, bevel);
            let height = sqrt(max(bevel * bevel - x * x, 0.0));
            normal = normalize(vec3f(footprint.yz * max(x, 1e-6), height));
            hit = vec3f(v.local, height - bevel);
        }
        cover = clamp(0.5 - footprint.x / unit, 0.0, 1.0);
        settled = true;
    }
    if (slab && !settled) {
        let thickness = v.material.x;
        let bevel = v.material.y;
        let reach = thickness / max(-down.z, 0.05) + 2.0 * unit;
        let start = hit;
        var s = 0.0;
        var best = 1e9;
        var closest = hit;
        var found = false;
        for (var step = 0u; step < 32u; step += 1u) {
            let q = start + down * s;
            let distance = flat_slab(q, v, kind, thickness, bevel);
            if (distance < best) {
                best = distance;
                closest = q;
            }
            if (distance < 0.02 * unit) {
                found = true;
                break;
            }
            s += max(distance, 0.1 * unit);
            if (s > reach) {
                break;
            }
        }
        hit = closest;
        if (found) {
            let probe = min(unit, thickness * 0.5);
            let inside = flat_slab(hit + down * probe, v, kind, thickness, bevel);
            cover = clamp(0.5 - inside / max(probe, 1e-6), 0.0, 1.0);
        } else {
            cover = clamp(0.5 - best / unit, 0.0, 1.0);
        }
        let e = 0.25 * unit;
        normal = normalize(vec3f(
            flat_slab(hit + vec3f(e, 0.0, 0.0), v, kind, thickness, bevel)
                - flat_slab(hit - vec3f(e, 0.0, 0.0), v, kind, thickness, bevel),
            flat_slab(hit + vec3f(0.0, e, 0.0), v, kind, thickness, bevel)
                - flat_slab(hit - vec3f(0.0, e, 0.0), v, kind, thickness, bevel),
            flat_slab(hit + vec3f(0.0, 0.0, e), v, kind, thickness, bevel)
                - flat_slab(hit - vec3f(0.0, 0.0, e), v, kind, thickness, bevel),
        ));
    }
    let p = hit.xy;
    let field = flat_field(v, kind, p);
    var fill = vec4f(0.0);
    var fill_cover = 0.0;
    if (fill_mode != 0u) {
        fill_cover = select(select(flat_coverage(field, inverse), flat_coverage_even(field.x, inverse), icon), 1.0, slab);
        var base = flat_fill_base(v, fill_mode, p);
        let pattern = (v.info.x >> 12u) & 7u;
        if (pattern != 0u) {
            base = flat_pattern_over(base, pattern, v.icon, v.info.z, p, inverse);
        }
        fill = base * fill_cover;
    }
    var stroke = vec4f(0.0);
    var stroke_cover = 0.0;
    if (v.stroke.x > 0.0) {
        stroke_cover = select(flat_stroke_cover(v, kind, p, field, inverse), 1.0, slab && kind == FLAT_ICON_LINE);
        stroke = vec4f(v.stroke_colour.rgb * v.stroke_colour.a, v.stroke_colour.a) * stroke_cover;
    }
    let painted = stroke + fill * (1.0 - stroke.a);
    let alpha = painted.a;
    if (alpha <= 0.0) {
        return FlatPaint(vec4f(0.0), 0.0);
    }
    let albedo = flat_decode(painted.rgb / alpha);
    let world_normal = normalize(world_from_local * normal);
    let key = flat_view.light.xyz;
    let view = vec3f(0.0, 0.0, 1.0);
    let lean = length(world_normal.xy);
    var shade = 1.0;
    if (slab) {
        let ambient = flat_view.light.w;
        let ratio = clamp(dot(world_normal, key) / max(key.z, 0.05), 0.0, 1.5);
        shade = 1.0 + (1.0 - ambient) * (ratio - 1.0);
    }
    var linear = albedo * shade;
    let roughness = v.frame_y.w;
    if (lean > 0.0) {
        linear += vec3f(lean * v.material.z * flat_ggx(world_normal, key, view, roughness) * max(dot(world_normal, key), 0.0));
        if (v.frame_x.w > 0.0 && flat_view.environment.z > 0.0) {
            let nv = max(dot(world_normal, view), 0.0);
            let fresnel = 0.04 + 0.96 * pow(1.0 - nv, 5.0);
            linear += lean * v.frame_x.w * fresnel * flat_reflected(reflect(-view, world_normal), roughness);
        }
    }
    let coverage = alpha * cover * opacity;
    var colour = painted * (cover * opacity);
    if (any(linear != albedo)) {
        colour += vec4f((flat_encode(linear) - flat_encode(albedo)) * coverage, 0.0);
    }
    let footprint = select(0.0, select(max(fill_cover, stroke_cover), cover, slab), opacity > 0.0);
    return FlatPaint(colour, footprint);
}

fn flat_rich(v: FlatVaryings) -> FlatPaint {
    var paint: FlatPaint;
    if ((v.info.x & FLAT_BLUR) != 0u) {
        paint = flat_blur(v);
    } else if ((v.info.x & FLAT_LIT) != 0u) {
        paint = flat_lit(v);
    } else {
        paint = flat_paint(v);
    }
    if ((v.info.x & FLAT_ADDITIVE) != 0u) {
        paint.colour.a = 0.0;
    }
    return paint;
}

@fragment
fn flat_colour_lit(v: FlatVaryings) -> @location(0) vec4f {
    return flat_rich(v).colour;
}

@fragment
fn flat_id_lit(v: FlatVaryings) -> @location(0) u32 {
    if ((v.info.x & FLAT_UNPICKED) != 0u) {
        discard;
    }
    var footprint = 0.0;
    if ((v.info.x & FLAT_BLUR) != 0u) {
        footprint = flat_paint(v).footprint;
    } else {
        footprint = flat_rich(v).footprint;
    }
    if (footprint < 0.5) {
        discard;
    }
    return v.info.y;
}
"#;

pub const FX_WGSL: &str = r#"
const FLAT_BLUR: u32 = 2048u;
const FLAT_ADDITIVE: u32 = 32768u;
const FLAT_UNPICKED: u32 = 65536u;
const FLAT_BLUR_ROWS: u32 = 4u;
const FLAT_BLUR_STEPS: u32 = 48u;
const FLAT_BLUR_FLOOR: f32 = 0.35;

@vertex
fn flat_id_vertex_rich(instance: FlatInstance, @builtin(vertex_index) index: u32) -> FlatVaryings {
    var out = flat_place(instance, index);
    if ((instance.info.x & 15u) == FLAT_SHADOW || (instance.info.x & FLAT_UNPICKED) != 0u) {
        out.position = vec4f(2.0, 2.0, 2.0, 1.0);
    }
    return out;
}

fn flat_blur_distance(v: FlatVaryings, kind: u32, p: vec2f) -> f32 {
    if (kind == FLAT_ICON_FILL || kind == FLAT_ICON_LINE) {
        let c = clamp(p, v.shape.xy - v.shape.zw, v.shape.xy + v.shape.zw);
        let outside = length(p - c);
        if (outside > 0.0) {
            return outside + 0.5 * v.stroke.x;
        }
        return textureSampleLevel(flat_icons, flat_icon_sampler, v.icon.xy + c * v.icon.zw, 0.0).r;
    }
    return flat_field(v, kind, p).x;
}

fn flat_blur_band(v: FlatVaryings, kind: u32, stroke: bool, p: vec2f) -> f32 {
    let d = flat_blur_distance(v, kind, p);
    if (!stroke) {
        return d;
    }
    let band = abs(d) - v.stroke.x * 0.5;
    if ((v.info.x & FLAT_DASHED) == 0u) {
        return band;
    }
    var along = flat_circle_along(p, v.shape.x);
    if (kind == FLAT_RECT) {
        along = flat_rect_along(p, v.shape.xy, v.corners);
    }
    return max(band, flat_dash(along, v.stroke.y, v.stroke.z, v.stroke.w).x);
}

fn flat_blur_cdf(x: f32, span: f32) -> f32 {
    let hi = 0.5 * (span + 1.0);
    let lo = 0.5 * abs(span - 1.0);
    let ramp = min(span, 1.0);
    let height = 1.0 / max(span, 1.0);
    if (x <= -hi) {
        return 0.0;
    }
    if (x >= hi) {
        return 1.0;
    }
    if (x < -lo) {
        let u = x + hi;
        return 0.5 * height / ramp * u * u;
    }
    if (x > lo) {
        let u = hi - x;
        return 1.0 - 0.5 * height / ramp * u * u;
    }
    return 0.5 * height * ramp + height * (x + lo);
}

fn flat_blur_line(v: FlatVaryings, kind: u32, stroke: bool, start: vec2f, along: vec2f, unit: f32, span: f32) -> f32 {
    let hi = 0.5 * (span + 1.0);
    let least = max(FLAT_BLUR_FLOOR, 2.0 * hi / f32(FLAT_BLUR_STEPS - 8u));
    var x = -hi;
    var s0 = flat_blur_band(v, kind, stroke, start + along * x) / unit;
    var total = 0.0;
    for (var step = 0u; step < FLAT_BLUR_STEPS; step += 1u) {
        if (x >= hi) {
            break;
        }
        let x1 = min(x + max(abs(s0), least), hi);
        let s1 = flat_blur_band(v, kind, stroke, start + along * x1) / unit;
        if (s0 <= 0.0 && s1 <= 0.0) {
            total += flat_blur_cdf(x1, span) - flat_blur_cdf(x, span);
        } else if (s0 <= 0.0) {
            let r = x + (x1 - x) * s0 / (s0 - s1);
            total += flat_blur_cdf(r, span) - flat_blur_cdf(x, span);
        } else if (s1 <= 0.0) {
            let r = x + (x1 - x) * s0 / (s0 - s1);
            total += flat_blur_cdf(x1, span) - flat_blur_cdf(r, span);
        }
        x = x1;
        s0 = s1;
    }
    if (x < hi && s0 <= 0.0) {
        total += 1.0 - flat_blur_cdf(x, span);
    }
    return total;
}

fn flat_line_empty() -> vec2f {
    return vec2f(1.0, -1.0);
}

fn flat_line_merge(a: vec2f, b: vec2f) -> vec2f {
    if (a.x >= a.y) {
        return b;
    }
    if (b.x >= b.y) {
        return a;
    }
    return vec2f(min(a.x, b.x), max(a.y, b.y));
}

fn flat_line_disc(start: vec2f, along: vec2f, centre: vec2f, radius: f32) -> vec2f {
    if (radius <= 0.0) {
        return flat_line_empty();
    }
    let o = start - centre;
    let aa = dot(along, along);
    let b = dot(o, along);
    let c = dot(o, o) - radius * radius;
    let d = b * b - aa * c;
    if (d <= 0.0) {
        return flat_line_empty();
    }
    let root = sqrt(d);
    return vec2f((-b - root) / aa, (-b + root) / aa);
}

fn flat_line_box(start: vec2f, along: vec2f, half: vec2f) -> vec2f {
    if (half.x < 0.0 || half.y < 0.0) {
        return flat_line_empty();
    }
    var low = -1e30;
    var high = 1e30;
    for (var axis = 0; axis < 2; axis += 1) {
        if (abs(along[axis]) < 1e-12) {
            if (abs(start[axis]) > half[axis]) {
                return flat_line_empty();
            }
        } else {
            let a = (-half[axis] - start[axis]) / along[axis];
            let b = (half[axis] - start[axis]) / along[axis];
            low = max(low, min(a, b));
            high = min(high, max(a, b));
        }
    }
    return vec2f(low, high);
}

fn flat_line_rounded(start: vec2f, along: vec2f, half: vec2f, radius: f32) -> vec2f {
    if (half.x <= 0.0 || half.y <= 0.0) {
        return flat_line_empty();
    }
    let r = clamp(radius, 0.0, min(half.x, half.y));
    var hit = flat_line_box(start, along, vec2f(half.x, half.y - r));
    hit = flat_line_merge(hit, flat_line_box(start, along, vec2f(half.x - r, half.y)));
    if (r > 0.0) {
        let c = half - vec2f(r);
        hit = flat_line_merge(hit, flat_line_disc(start, along, c, r));
        hit = flat_line_merge(hit, flat_line_disc(start, along, vec2f(-c.x, c.y), r));
        hit = flat_line_merge(hit, flat_line_disc(start, along, vec2f(c.x, -c.y), r));
        hit = flat_line_merge(hit, flat_line_disc(start, along, -c, r));
    }
    return hit;
}

fn flat_blur_weight(hit: vec2f, span: f32) -> f32 {
    if (hit.x >= hit.y) {
        return 0.0;
    }
    return flat_blur_cdf(hit.y, span) - flat_blur_cdf(hit.x, span);
}

fn flat_blur_closed(v: FlatVaryings, kind: u32, stroke: bool) -> bool {
    if (kind == FLAT_RING) {
        return !stroke;
    }
    if (kind != FLAT_RECT && kind != FLAT_CIRCLE) {
        return false;
    }
    if (kind == FLAT_RECT && any(v.corners != vec4f(v.corners.x))) {
        return false;
    }
    return !stroke || (v.info.x & FLAT_DASHED) == 0u;
}

fn flat_blur_row(v: FlatVaryings, kind: u32, stroke: bool, start: vec2f, a: vec2f, span: f32) -> f32 {
    let w = select(0.0, v.stroke.x * 0.5, stroke);
    var outer = flat_line_empty();
    var inner = flat_line_empty();
    if (kind == FLAT_RECT) {
        outer = flat_line_rounded(start, a, v.shape.xy + vec2f(w), v.corners.x + w);
        if (stroke) {
            inner = flat_line_rounded(start, a, v.shape.xy - vec2f(w), max(v.corners.x - w, 0.0));
        }
    } else if (kind == FLAT_CIRCLE) {
        outer = flat_line_disc(start, a, vec2f(0.0), v.shape.x + w);
        if (stroke) {
            inner = flat_line_disc(start, a, vec2f(0.0), v.shape.x - w);
        }
    } else {
        outer = flat_line_disc(start, a, vec2f(0.0), v.shape.x + 0.5 * v.shape.y);
        inner = flat_line_disc(start, a, vec2f(0.0), v.shape.x - 0.5 * v.shape.y);
    }
    return flat_blur_weight(outer, span) - flat_blur_weight(inner, span);
}

fn flat_blur_cover(v: FlatVaryings, kind: u32, stroke: bool, p: vec2f, inverse: mat2x2f) -> f32 {
    let axes = mat2x2f(vec2f(inverse[1].y, -inverse[0].y), vec2f(-inverse[1].x, inverse[0].x))
        * (1.0 / determinant(inverse));
    let screen = axes * v.material.xy;
    let span = max(length(screen), 1e-6);
    let direction = screen / span;
    let along = inverse * direction;
    let across = inverse * vec2f(-direction.y, direction.x);
    let unit = max(length(along), 1e-12);
    let closed = flat_blur_closed(v, kind, stroke);
    var total = 0.0;
    for (var row = 0u; row < FLAT_BLUR_ROWS; row += 1u) {
        let u = (f32(row) + 0.5) / f32(FLAT_BLUR_ROWS) - 0.5;
        if (closed) {
            total += flat_blur_row(v, kind, stroke, p + across * u, along, span);
        } else {
            total += flat_blur_line(v, kind, stroke, p + across * u, along, unit, span);
        }
    }
    return clamp(total / f32(FLAT_BLUR_ROWS), 0.0, 1.0);
}

fn flat_blur(v: FlatVaryings) -> FlatPaint {
    let p = v.local;
    let inverse = mat2x2f(v.inverse.xy, v.inverse.zw);
    let kind = v.info.x & 15u;
    let fill_mode = (v.info.x >> FLAT_FILL_SHIFT) & 15u;
    let opacity = bitcast<f32>(v.info.w);
    var fill = vec4f(0.0);
    var fill_cover = 0.0;
    if (fill_mode != 0u) {
        fill_cover = flat_blur_cover(v, kind, false, p, inverse);
        var base = flat_fill_base(v, fill_mode, p);
        let pattern = (v.info.x >> 12u) & 7u;
        if (pattern != 0u) {
            base = flat_pattern_over(base, pattern, v.icon, v.info.z, p, inverse);
        }
        fill = base * fill_cover;
    }
    var stroke = vec4f(0.0);
    var stroke_cover = 0.0;
    if (v.stroke.x > 0.0) {
        stroke_cover = flat_blur_cover(v, kind, true, p, inverse);
        stroke = vec4f(v.stroke_colour.rgb * v.stroke_colour.a, v.stroke_colour.a) * stroke_cover;
    }
    let colour = (stroke + fill * (1.0 - stroke.a)) * opacity;
    let fill_seen = select(0.0, fill_cover, fill_mode != 0u && flat_fill_alpha(v, fill_mode) > 0.0);
    let stroke_seen = select(0.0, stroke_cover, v.stroke_colour.a > 0.0);
    let footprint = select(0.0, max(fill_seen, stroke_seen), opacity > 0.0);
    return FlatPaint(colour, footprint);
}
"#;

pub const FINISH_WGSL: &str = r#"
@group(0) @binding(0) var flat_source: texture_2d<f32>;

@vertex
fn flat_finish_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let xy = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
    return vec4f(xy[index], 0.0, 1.0);
}

fn flat_finish_decode(c: vec3f) -> vec3f {
    return select(pow((c + 0.055) / 1.055, vec3f(2.4)), c / 12.92, c <= vec3f(0.04045));
}

@fragment
fn flat_finish_copy(@builtin(position) position: vec4f) -> @location(0) vec4f {
    return textureLoad(flat_source, vec2i(position.xy), 0);
}

@fragment
fn flat_finish_linear(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let c = textureLoad(flat_source, vec2i(position.xy), 0);
    if (c.a <= 0.0) {
        return vec4f(0.0);
    }
    return vec4f(flat_finish_decode(c.rgb / c.a) * c.a, c.a);
}
"#;

pub use super::style::{STYLE_CALL, STYLE_WGSL};

pub fn flat_source() -> String {
    format!(
        "{COLOUR_WGSL}
{SDF_WGSL}
{STYLE_WGSL}
{PASS_WGSL}"
    )
}

pub fn source() -> String {
    format!(
        "{COLOUR_WGSL}
{SDF_WGSL}
{STYLE_WGSL}
{PASS_WGSL}
{LIT_WGSL}
{FX_WGSL}"
    )
}

pub fn unstyled_source() -> String {
    format!(
        "{COLOUR_WGSL}
{SDF_WGSL}
{}
{LIT_WGSL}
{FX_WGSL}",
        PASS_WGSL.replace(STYLE_CALL, "")
    )
}
