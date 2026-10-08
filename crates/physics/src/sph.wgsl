struct Particle {
    pos: vec2<f32>,
    vel: vec2<f32>,
    owner: u32,
    rho: f32,
    life: f32,
    born: f32,
}

struct Target {
    a: vec4<f32>,
    b: vec4<f32>,
    c: vec4<f32>,
    look: vec4<f32>,
    flow: vec4<f32>,
    form: vec4<f32>,
    carry: vec4<f32>,
}

struct Sim {
    grid: vec4<f32>,
    table: vec4<f32>,
    k: vec4<f32>,
    misc: vec4<f32>,
    apart: vec4<f32>,
    tension: vec4<f32>,
    clock: vec4<f32>,
}

@group(0) @binding(0) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(1) var<storage, read> targets: array<Target>;
@group(0) @binding(2) var<storage, read_write> counts: array<atomic<u32>>;
@group(0) @binding(3) var<storage, read_write> items: array<u32>;
@group(0) @binding(4) var<uniform> sim: Sim;
@group(0) @binding(5) var<storage, read_write> accel: array<vec2<f32>>;
@group(0) @binding(6) var height_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(7) var<storage, read_write> starts: array<u32>;
@group(0) @binding(8) var<storage, read_write> ranks: array<u32>;

const SLOTS: u32 = 32u;
const BLOCK: u32 = 256u;

var<workgroup> partial: array<u32, 256>;
const PI: f32 = 3.14159265;

fn cells() -> vec2<i32> {
    return vec2<i32>(sim.grid.xy);
}

fn cell_of(p: vec2<f32>) -> vec2<i32> {
    return clamp(vec2<i32>(floor(p / sim.grid.z)), vec2<i32>(0), cells() - vec2<i32>(1));
}

fn cell_id(c: vec2<i32>) -> u32 {
    return u32(c.y * cells().x + c.x);
}

fn cell_total() -> u32 {
    return u32(sim.grid.x * sim.grid.y);
}

fn binned(p: Particle) -> bool {
    return p.born >= 0.0 && p.life > 1e-3;
}

fn scan_partial(lane: u32, value: u32) -> u32 {
    partial[lane] = value;
    workgroupBarrier();
    for (var reach = 1u; reach < BLOCK; reach = reach << 1u) {
        var add = 0u;
        if (lane >= reach) {
            add = partial[lane - reach];
        }
        workgroupBarrier();
        partial[lane] += add;
        workgroupBarrier();
    }
    return partial[lane];
}

fn poly6(r2: f32) -> f32 {
    let h = sim.grid.z;
    let h2 = h * h;
    if (r2 >= h2) {
        return 0.0;
    }
    let d = h2 - r2;
    return 4.0 / (PI * pow(h, 8.0)) * d * d * d;
}

fn spiky(r: f32) -> f32 {
    let h = sim.grid.z;
    if (r >= h || r <= 1e-5) {
        return 0.0;
    }
    let d = h - r;
    return -30.0 / (PI * pow(h, 5.0)) * d * d;
}

fn harmonic(theta: f32, seed: f32) -> f32 {
    let square = fract(seed * 0.618) * 0.9;
    return 0.46 * sin(2.0 * theta + seed * 1.7) + 0.36 * sin(3.0 * theta + seed * 2.9 + 1.0) + square * 0.40 * cos(4.0 * theta + seed * 0.7) + 0.12 * sin(5.0 * theta + seed * 4.3 + 2.0);
}

fn turn(p: vec2<f32>, angle: f32) -> vec2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
}

fn segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let ba = b - a;
    let h = clamp(dot(p - a, ba) / max(dot(ba, ba), 1e-4), 0.0, 1.0);
    return vec2<f32>(length(p - a - ba * h), h);
}

fn bezier(t: Target, u: f32) -> vec2<f32> {
    let v = 1.0 - u;
    return v * v * v * t.a.xy + 3.0 * v * v * u * t.a.zw + 3.0 * v * u * u * t.b.xy + u * u * u * t.b.zw;
}

fn along(t: Target, p: vec2<f32>) -> vec4<f32> {
    var best = 1e9;
    var out = vec4<f32>(0.5, 1.0, 0.0, 1.0);
    var total = 0.0;
    var prev = bezier(t, 0.0);
    for (var i = 1; i <= 16; i++) {
        let next = bezier(t, f32(i) / 16.0);
        total += length(next - prev);
        prev = next;
    }
    prev = bezier(t, 0.0);
    for (var i = 1; i <= 16; i++) {
        let next = bezier(t, f32(i) / 16.0);
        let s = segment(p, prev, next);
        if (s.x < best) {
            best = s.x;
            let dir = normalize(next - prev + vec2<f32>(1e-5, 0.0));
            out = vec4<f32>((f32(i - 1) + s.y) / 16.0, dir, total);
        }
        prev = next;
    }
    return out;
}

fn region(t: Target, p: vec2<f32>) -> vec3<f32> {
    let kind = u32(t.flow.z);
    switch kind {
        case 1u: {
            let s = segment(p, t.a.xy, t.a.zw);
            let span = t.a.zw - t.a.xy;
            let normal = vec2<f32>(-span.y, span.x) / max(length(span), 1e-3);
            let u = s.y;
            let axis = mix(t.a.xy, t.a.zw, u) + normal * t.form.x * sin(3.14159265 * u);
            let drift = sim.clock.x * sim.clock.y * t.form.w;
            let wobble = 0.6 * sin(6.2831853 * u * 1.3 + t.form.z + drift) + 0.4 * sin(6.2831853 * u * 2.7 + t.form.z * 1.7 - drift * 0.7);
            let r = mix(t.b.x, t.b.y, 1.0 - (1.0 - u) * (1.0 - u)) * (1.0 + t.form.y * wobble);
            return vec3<f32>(length(p - axis) - r, axis);
        }
        case 2u: {
            var best = vec3<f32>(1e5, p);
            var prev = bezier(t, 0.0);
            for (var i = 1; i <= 16; i++) {
                let u1 = f32(i) / 16.0;
                let next = bezier(t, u1);
                let s = segment(p, prev, next);
                let u = (f32(i - 1) + s.y) / 16.0;
                let e = abs(2.0 * u - 1.0);
                let r = mix(t.c.z, mix(t.c.x, t.c.y, u), pow(e, select(2.0, t.form.x, t.form.x > 0.0)));
                if (s.x - r < best.x) {
                    best = vec3<f32>(s.x - r, mix(prev, next, s.y));
                }
                prev = next;
            }
            return best;
        }
        case 3u: {
            return vec3<f32>(length(p - t.a.xy) - t.a.z, t.a.xy);
        }
        case 4u: {
            let squash = select(1.0, t.a.w, t.a.w > 0.0);
            let d = (p - t.a.xy) * vec2<f32>(1.0, 1.0 / squash);
            let th = atan2(d.y, d.x);
            let wave = t.b.x * cos(2.0 * th - t.form.x) + t.b.y * cos(3.0 * th - t.form.y) + t.b.z * cos(4.0 * th - t.form.z) + t.b.w * cos(6.0 * th - t.form.w) + t.c.x * cos(7.0 * th - t.c.y);
            return vec3<f32>((length(d) - t.a.z * (1.0 + wave)) * min(squash, 1.0), t.a.xy);
        }
        default: {
            let local = turn(p - t.a.xy, -t.form.x);
            let r = max(t.a.zw, vec2<f32>(1.0));
            let theta = atan2(local.y / r.y, local.x / r.x);
            let c = max(cos(theta + t.form.x - t.form.w), 0.0);
            let s = 1.0 + t.form.y * harmonic(theta, t.form.z) + t.b.x * pow(c, max(t.b.y, 2.0));
            let shaped = r * s;
            let k0 = length(local / shaped);
            let k1 = length(local / (shaped * shaped));
            return vec3<f32>(k0 * (k0 - 1.0) / max(k1, 1e-5), t.a.xy);
        }
    }
}

@compute @workgroup_size(256)
fn clear_cells(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < cell_total()) {
        atomicStore(&counts[id.x], 0u);
    }
}

@compute @workgroup_size(256)
fn count(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= u32(sim.grid.w)) {
        return;
    }
    let p = particles[i];
    if (!binned(p)) {
        return;
    }
    ranks[i] = atomicAdd(&counts[cell_id(cell_of(p.pos))], 1u);
}

@compute @workgroup_size(256)
fn scan_cells(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_id) local: vec3<u32>, @builtin(workgroup_id) group: vec3<u32>) {
    let n = cell_total();
    var value = 0u;
    if (id.x < n) {
        value = atomicLoad(&counts[id.x]);
    }
    let inclusive = scan_partial(local.x, value);
    if (id.x < n) {
        starts[id.x] = inclusive - value;
    }
    if (local.x == BLOCK - 1u) {
        starts[n + group.x] = inclusive;
    }
}

@compute @workgroup_size(256)
fn scan_blocks(@builtin(local_invocation_id) local: vec3<u32>) {
    let n = cell_total();
    let total = (n + BLOCK - 1u) / BLOCK;
    var carry = 0u;
    for (var base = 0u; base < total; base += BLOCK) {
        let b = base + local.x;
        var value = 0u;
        if (b < total) {
            value = starts[n + b];
        }
        let inclusive = scan_partial(local.x, value);
        if (b < total) {
            starts[n + b] = carry + inclusive - value;
        }
        carry += partial[BLOCK - 1u];
        workgroupBarrier();
    }
}

@compute @workgroup_size(256)
fn scatter(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= u32(sim.grid.w)) {
        return;
    }
    let p = particles[i];
    if (!binned(p)) {
        return;
    }
    let cell = cell_id(cell_of(p.pos));
    items[starts[cell] + starts[cell_total() + cell / BLOCK] + ranks[i]] = i;
}

@compute @workgroup_size(256)
fn order(@builtin(global_invocation_id) id: vec3<u32>) {
    let cell = id.x;
    let n = cell_total();
    if (cell >= n) {
        return;
    }
    let base = starts[cell] + starts[n + cell / BLOCK];
    starts[cell] = base;
    let filled = atomicLoad(&counts[cell]);
    for (var k = 1u; k < filled; k++) {
        let item = items[base + k];
        var at = k;
        while (at > 0u && items[base + at - 1u] > item) {
            items[base + at] = items[base + at - 1u];
            at -= 1u;
        }
        items[base + at] = item;
    }
}

@compute @workgroup_size(256)
fn density(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x + u32(sim.apart.z);
    if (i >= u32(sim.grid.w)) {
        return;
    }
    var p = particles[i];
    if (p.born < 0.0) {
        return;
    }
    let home = cell_of(p.pos);
    var rho = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let c = home + vec2<i32>(dx, dy);
            if (any(c < vec2<i32>(0)) || any(c >= cells())) {
                continue;
            }
            let cell = cell_id(c);
            let n = min(atomicLoad(&counts[cell]), SLOTS);
            let base = starts[cell];
            for (var k = 0u; k < n; k++) {
                let j = items[base + k];
                let q = particles[j];
                let d = p.pos - q.pos;
                rho += q.life * poly6(dot(d, d));
            }
        }
    }
    particles[i].rho = max(rho, sim.table.z * 0.05);
}

@compute @workgroup_size(256)
fn forces(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x + u32(sim.apart.z);
    if (i >= u32(sim.grid.w)) {
        return;
    }
    let p = particles[i];
    if (p.born < 0.0) {
        return;
    }
    let t = targets[p.owner];
    let home = cell_of(p.pos);
    var a = vec2<f32>(0.0);
    var drag = vec2<f32>(0.0);
    let pressure = sim.k.x;
    let card = u32(t.flow.z) == 1u;
    let film = sim.grid.z * max(sim.apart.y, 1.0);
    let span = select(1, i32(ceil(max(sim.apart.y, 1.0))), card);
    for (var dy = -span; dy <= span; dy++) {
        for (var dx = -span; dx <= span; dx++) {
            let c = home + vec2<i32>(dx, dy);
            if (any(c < vec2<i32>(0)) || any(c >= cells())) {
                continue;
            }
            let cell = cell_id(c);
            let n = min(atomicLoad(&counts[cell]), SLOTS);
            let base = starts[cell];
            for (var k = 0u; k < n; k++) {
                let j = items[base + k];
                if (j == i) {
                    continue;
                }
                let q = particles[j];
                let d = p.pos - q.pos;
                let r = length(d);
                if (card && q.owner != p.owner && u32(targets[q.owner].flow.z) == 1u) {
                    if (r < film && r > 1e-4) {
                        let k = 1.0 - r / film;
                        a += sim.apart.x * q.life * p.life * k * k * d / r;
                    }
                    continue;
                }
                if (r >= sim.grid.z * 1.5) {
                    continue;
                }
                if (r < 1e-4) {
                    a += vec2<f32>(fract(f32(i) * 0.618) - 0.5, fract(f32(j) * 0.618) - 0.5) * 50.0;
                    continue;
                }
                let g = spiky(r) * d / r;
                a -= pressure * q.life * (1.0 / p.rho + 1.0 / q.rho) * g;
                drag += q.life * (q.vel - p.vel) * poly6(r * r) / q.rho;
            }
        }
    }
    let shape = region(t, p.pos);
    let out = p.pos - shape.yz;
    let dir = out / max(length(out), 1e-3);
    if (t.c.w > 0.5) {
        a -= sim.k.y * 0.35 * (shape.x + t.c.w - 1.0) * dir;
        if (u32(t.flow.z) == 2u) {
            let c = along(t, p.pos);
            let keep = clamp(c.x, t.form.z, 1.0 - t.form.z);
            a += sim.k.y * 0.5 * (keep - c.x) * c.w * c.yz;
        }
        if (u32(t.flow.z) == 0u && t.b.w > 0.0) {
            let rel = p.pos - t.a.xy;
            let angle = atan2(rel.y, rel.x);
            let off = atan2(sin(angle - t.b.z), cos(angle - t.b.z));
            if (abs(off) < t.b.w) {
                let tangent = vec2<f32>(-rel.y, rel.x) / max(length(rel), 1.0);
                a += sim.k.y * 2.0 * sign(off) * (t.b.w - abs(off)) * length(rel) * tangent;
            }
        }
    } else {
        if (shape.x > 0.0) {
            a -= sim.k.y * shape.x * dir;
        }
        var hold = 1.0;
        if (u32(t.flow.z) == 0u && sim.tension.x > 0.0) {
            hold = sim.tension.z + smoothstep(-sim.tension.x, 0.0, shape.x) * sim.tension.y;
        }
        a -= sim.k.z * t.flow.y * out * hold;
        if (u32(t.flow.z) == 2u && t.form.y != 0.0) {
            let c = along(t, p.pos);
            a += sim.k.z * t.flow.y * t.form.y * (2.0 * c.x - 1.0) * c.w * 0.5 * c.yz;
        }
    }
    a += drag * sim.misc.w;
    accel[i] = a;
}

@compute @workgroup_size(256)
fn integrate(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x + u32(sim.apart.z);
    if (i >= u32(sim.grid.w)) {
        return;
    }
    var p = particles[i];
    if (p.born < 0.0) {
        return;
    }
    let t = targets[p.owner];
    let dt = sim.table.w;
    p.vel = p.vel * exp(-sim.k.w * dt) + accel[i] * dt;
    let speed = length(p.vel);
    if (speed > 900.0) {
        p.vel = p.vel * (900.0 / speed);
    }
    p.pos = clamp(p.pos + (p.vel + t.carry.xy) * dt, vec2<f32>(1.0), sim.table.xy - vec2<f32>(1.0));
    var goal = select(0.0, t.flow.x, t.flow.w > 0.5);
    if (u32(t.flow.z) == 2u && t.carry.w > 0.5) {
        let c = along(t, p.pos);
        goal *= smoothstep(t.carry.z + 0.05, t.carry.z - 0.05, c.x);
    }
    p.life = clamp(p.life + clamp(goal - p.life, -sim.misc.x * dt, sim.misc.x * dt), 0.0, 4.0);
    particles[i] = p;
}

@compute @workgroup_size(8, 8)
fn gather(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<u32>(sim.table.xy);
    if (any(id.xy >= size)) {
        return;
    }
    let x = vec2<f32>(id.xy) + vec2<f32>(0.5);
    let home = cell_of(x);
    let reach = sim.grid.z * sim.misc.z;
    let keep = sim.grid.z * sim.grid.z / (reach * reach);
    var height = 0.0;
    var weight = 0.0;
    var tinted = 0.0;
    var chrome = 0.0;
    var milky = 0.0;
    for (var dy = -2; dy <= 2; dy++) {
        for (var dx = -2; dx <= 2; dx++) {
            let c = home + vec2<i32>(dx, dy);
            if (any(c < vec2<i32>(0)) || any(c >= cells())) {
                continue;
            }
            let cell = cell_id(c);
            let n = min(atomicLoad(&counts[cell]), SLOTS);
            let base = starts[cell];
            for (var k = 0u; k < n; k++) {
                let j = items[base + k];
                let q = particles[j];
                let d = x - q.pos;
                let r2 = dot(d, d);
                let t = targets[q.owner];
                let wide = reach * reach;
                if (r2 >= wide) {
                    continue;
                }
                let e = wide - r2;
                let w = q.life * 4.0 / (PI * pow(reach, 8.0)) * e * e * e * keep;
                if (w <= 0.0) {
                    continue;
                }
                height += w * t.look.x;
                weight += w;
                tinted += w * t.look.y;
                chrome += w * t.look.z;
                milky += w * t.look.w;
            }
        }
    }
    let h = height / sim.table.z;
    let inv = h / max(weight, 1e-6);
    textureStore(height_out, vec2<i32>(id.xy), vec4<f32>(h, tinted * inv, chrome * inv, milky * inv));
}
