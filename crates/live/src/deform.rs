use bytemuck::{Pod, Zeroable};
use pfx_geom::strip::{ROLL_WGSL, Roller};
use pfx_physics::BEND_WGSL;
use std::sync::Arc;
use wgpu::util::DeviceExt;

pub const NONE: u32 = u32::MAX;
pub const ROLLER: u32 = 1;
pub const WIND: u32 = 2;
pub const CURVED_ROLLER: u32 = 3;

pub const DEFORM_WGSL: &str = r#"
struct DeformerData {
    kind: vec4u,
    current_a: vec4f,
    current_b: vec4f,
    previous_a: vec4f,
    previous_b: vec4f,
}
struct DeformedVertex {
    position: vec3f,
    normal: vec3f,
    tangent: vec4f,
    previous_position: vec3f,
}
@group(2) @binding(0) var<storage, read> deformers: array<DeformerData>;
@group(2) @binding(1) var<storage, read> roller_plan: array<vec4f>;

fn plan_sample(s: f32, offset: u32, count: u32) -> vec4f {
    var low = 0u;
    var high = count - 1u;
    let clamped = clamp(s, roller_plan[offset].x, roller_plan[offset + high].x);
    while (high - low > 1u) {
        let middle = (low + high) / 2u;
        if (roller_plan[offset + middle].x < clamped) {
            low = middle;
        } else {
            high = middle;
        }
    }
    let a = roller_plan[offset + low];
    let b = roller_plan[offset + high];
    let before = roller_plan[offset + max(low, 1u) - 1u];
    let after = roller_plan[offset + min(high + 1u, count - 1u)];
    let span = b.x - a.x;
    let u = (clamped - a.x) / span;
    let u2 = u * u;
    let u3 = u2 * u;
    let ta = normalize(vec2f(b.y - before.y, b.z - before.z));
    let tb = normalize(vec2f(after.y - a.y, after.z - a.z));
    let p = (2.0 * u3 - 3.0 * u2 + 1.0) * a.yz
        + (u3 - 2.0 * u2 + u) * span * ta
        + (-2.0 * u3 + 3.0 * u2) * b.yz
        + (u3 - u2) * span * tb;
    let derivative = (6.0 * u2 - 6.0 * u) * a.yz / span
        + (3.0 * u2 - 4.0 * u + 1.0) * ta
        + (-6.0 * u2 + 6.0 * u) * b.yz / span
        + (3.0 * u2 - 2.0 * u) * tb;
    return vec4f(p, normalize(derivative));
}

fn curved_keep(rest: vec3f, data: DeformerData, previous: bool) -> f32 {
    let state = select(data.current_a, data.previous_a, previous);
    return 1.0 - smoothstep(0.0, 3.14159265 * state.y, rest.x - state.x);
}

fn flattened(rest: vec3f, data: DeformerData, normal: vec3f, tangent: vec3f) -> array<vec3f, 2> {
    let keep = curved_keep(rest, data, false);
    if (keep >= 1.0) {
        return array<vec3f, 2>(normal, tangent);
    }
    let side = select(1.0, -1.0, normal.y < 0.0);
    let n = normalize(mix(vec3f(0.0, side, 0.0), normal, keep));
    let along = tangent - n * dot(tangent, n);
    let t = select(normalize(along), vec3f(1.0, 0.0, 0.0), dot(along, along) < 1e-12);
    return array<vec3f, 2>(n, t);
}

fn curved_roll(rest: vec3f, data: DeformerData, previous: bool) -> Rolled {
    let state = select(data.current_a, data.previous_a, previous);
    if (data.kind.w == 1u) {
        return roll_place(rest, state.x, state.y, state.z);
    }
    let sample = plan_sample(min(rest.x, state.x), data.kind.y, data.kind.z);
    let direction = sample.zw;
    let normal = vec2f(-direction.y, direction.x);
    let rolled = roll_place(rest, state.x, state.y, state.z);
    var out: Rolled;
    if (rest.x <= state.x) {
        out.position = vec3f(sample.xy + normal * rest.y, rest.z);
        out.tangent = vec3f(direction, 0.0);
        out.normal = vec3f(normal, 0.0);
    } else {
        let travel = rest.x - state.x;
        let shrink = state.z / 6.28318531;
        let fade = exp(-travel / state.y);
        let planar = rolled.position.xy - vec2f(state.x, shrink * (1.0 - fade));
        let tangent = normalize(rolled.tangent.xy - vec2f(0.0, shrink / state.y * fade));
        out.position = vec3f(sample.xy + direction * planar.x + normal * planar.y, rest.z);
        out.tangent = vec3f(direction * tangent.x + normal * tangent.y, 0.0);
        out.normal = vec3f(direction * -tangent.y + normal * tangent.x, 0.0);
        out.position += out.normal * rest.y * curved_keep(rest, data, previous);
    }
    return out;
}

fn deform_wind_position(rest: vec3f, data: DeformerData, previous: bool) -> vec3f {
    let a = select(data.current_a, data.previous_a, previous);
    let b = select(data.current_b, data.previous_b, previous);
    return rest + bend(rest, data.kind.y, a.x, a.y, b.xyz, f32(data.kind.z));
}

fn deform_position(rest: vec3f, deformer_id: u32, previous: bool) -> vec3f {
    if (deformer_id == 0xffffffffu) {
        return rest;
    }
    let data = deformers[deformer_id];
    if (data.kind.x == 1u) {
        let a = select(data.current_a, data.previous_a, previous);
        return roll_place(rest, a.x, a.y, a.z).position;
    }
    if (data.kind.x == 3u) {
        return curved_roll(rest, data, previous).position;
    }
    if (data.kind.x == 2u) {
        return deform_wind_position(rest, data, previous);
    }
    return rest;
}

fn deform_vertex(rest: vec3f, normal: vec3f, tangent: vec4f, deformer_id: u32) -> DeformedVertex {
    var out: DeformedVertex;
    out.position = rest;
    out.normal = normal;
    out.tangent = tangent;
    out.previous_position = rest;
    if (deformer_id == 0xffffffffu) {
        return out;
    }
    let data = deformers[deformer_id];
    if (data.kind.x == 1u || data.kind.x == 3u) {
        var current: Rolled;
        var n = normal;
        var t = tangent.xyz;
        if (data.kind.x == 3u) {
            current = curved_roll(rest, data, false);
            if (data.kind.w == 0u) {
                let flat = flattened(rest, data, normal, tangent.xyz);
                n = flat[0];
                t = flat[1];
            }
        } else {
            current = roll_place(rest, data.current_a.x, data.current_a.y, data.current_a.z);
        }
        let axis = vec3f(0.0, 0.0, 1.0);
        out.position = current.position;
        out.previous_position = deform_position(rest, deformer_id, true);
        out.normal = normalize(current.tangent * n.x + current.normal * n.y + axis * n.z);
        out.tangent = vec4f(normalize(current.tangent * t.x + current.normal * t.y + axis * t.z), tangent.w);
    } else if (data.kind.x == 2u) {
        out.position = deform_wind_position(rest, data, false);
        out.previous_position = deform_position(rest, deformer_id, true);
        let step = 0.001;
        let dx = (deform_wind_position(rest + vec3f(step, 0.0, 0.0), data, false) - out.position) / step;
        let dy = (deform_wind_position(rest + vec3f(0.0, step, 0.0), data, false) - out.position) / step;
        let dz = (deform_wind_position(rest + vec3f(0.0, 0.0, step), data, false) - out.position) / step;
        let cofactor = mat3x3f(cross(dy, dz), cross(dz, dx), cross(dx, dy));
        let jacobian = mat3x3f(dx, dy, dz);
        out.normal = normalize(cofactor * normal);
        out.tangent = vec4f(normalize(jacobian * tangent.xyz), tangent.w);
    }
    return out;
}
"#;

pub fn shader_source() -> String {
    format!("{ROLL_WGSL}\n{BEND_WGSL}\n{DEFORM_WGSL}")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollerFrame {
    pub at: f32,
    pub outer: f32,
    pub thickness: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindFrame {
    pub time: f32,
    pub strength: f32,
    pub direction: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub enum Deformer {
    Roller {
        current: RollerFrame,
        previous: RollerFrame,
    },
    CurvedRoller {
        current: RollerFrame,
        previous: RollerFrame,
        plan: Arc<[[f32; 4]]>,
    },
    Wind {
        current: WindFrame,
        previous: WindFrame,
        object: u32,
        seed: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeformerId(pub u32);

impl DeformerId {
    pub const NONE: Self = Self(NONE);
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PackedDeformer {
    pub kind: [u32; 4],
    pub current_a: [f32; 4],
    pub current_b: [f32; 4],
    pub previous_a: [f32; 4],
    pub previous_b: [f32; 4],
}

impl From<&Deformer> for PackedDeformer {
    fn from(value: &Deformer) -> Self {
        match value {
            Deformer::Roller { current, previous } => Self {
                kind: [ROLLER, 0, 0, 0],
                current_a: [current.at, current.outer, current.thickness, 0.0],
                current_b: [0.0; 4],
                previous_a: [previous.at, previous.outer, previous.thickness, 0.0],
                previous_b: [0.0; 4],
            },
            Deformer::CurvedRoller {
                current, previous, ..
            } => Self {
                kind: [CURVED_ROLLER, 0, 0, 0],
                current_a: [current.at, current.outer, current.thickness, 0.0],
                current_b: [0.0; 4],
                previous_a: [previous.at, previous.outer, previous.thickness, 0.0],
                previous_b: [0.0; 4],
            },
            Deformer::Wind {
                current,
                previous,
                object,
                seed,
            } => Self {
                kind: [WIND, *object, *seed, 0],
                current_a: [current.time, current.strength, 0.0, 0.0],
                current_b: [
                    current.direction[0],
                    current.direction[1],
                    current.direction[2],
                    0.0,
                ],
                previous_a: [previous.time, previous.strength, 0.0, 0.0],
                previous_b: [
                    previous.direction[0],
                    previous.direction[1],
                    previous.direction[2],
                    0.0,
                ],
            },
        }
    }
}

pub fn pack(deformers: &[Deformer]) -> Result<Vec<PackedDeformer>, String> {
    pack_with_plans(deformers).map(|(packed, _)| packed)
}

fn pack_with_plans(deformers: &[Deformer]) -> Result<(Vec<PackedDeformer>, Vec<[f32; 4]>), String> {
    if deformers.len() >= NONE as usize {
        return Err("too many deformers".into());
    }
    let mut packed = Vec::with_capacity(deformers.len().max(1));
    let mut plans = Vec::new();
    for deformer in deformers {
        match deformer {
            Deformer::Roller { current, previous }
            | Deformer::CurvedRoller {
                current, previous, ..
            } => {
                for state in [*current, *previous] {
                    if !state.at.is_finite()
                        || !state.outer.is_finite()
                        || state.outer <= 0.0
                        || !state.thickness.is_finite()
                        || state.thickness < 0.0
                    {
                        return Err("roller parameters must be finite with positive radius and nonnegative thickness".into());
                    }
                }
            }
            Deformer::Wind {
                current, previous, ..
            } => {
                for state in [current, previous] {
                    if !state.time.is_finite()
                        || !state.strength.is_finite()
                        || !state.direction.iter().all(|value| value.is_finite())
                    {
                        return Err("wind parameters must be finite".into());
                    }
                }
            }
        }
        let mut item = PackedDeformer::from(deformer);
        if let Deformer::CurvedRoller { plan, .. } = deformer {
            if plan.len() < 2 || plan.len() > u32::MAX as usize {
                return Err("roller plan needs at least two samples".into());
            }
            for (index, point) in plan.iter().enumerate() {
                if !point.iter().all(|value| value.is_finite())
                    || (index > 0 && point[0] <= plan[index - 1][0])
                    || (index > 0
                        && (point[1] - plan[index - 1][1]).hypot(point[2] - plan[index - 1][2])
                            <= 1e-9)
                {
                    return Err("roller plan must have finite, increasing arc lengths and distinct positions".into());
                }
            }
            if plans.len() > u32::MAX as usize - plan.len() {
                return Err("roller plan buffer is too large".into());
            }
            item.kind[1] = plans.len() as u32;
            item.kind[2] = plan.len() as u32;
            item.kind[3] = u32::from(
                plan.iter()
                    .all(|point| point[1] == point[0] && point[2] == 0.0),
            );
            plans.extend_from_slice(plan);
        }
        packed.push(item);
    }
    if packed.is_empty() {
        packed.push(PackedDeformer::zeroed());
    }
    if plans.is_empty() {
        plans.push([0.0; 4]);
    }
    Ok((packed, plans))
}

pub struct DeformerBuffer {
    pub layout: wgpu::BindGroupLayout,
    pub group: wgpu::BindGroup,
    buffer: wgpu::Buffer,
    capacity: usize,
    plan_buffer: wgpu::Buffer,
    plan_capacity: usize,
    pub len: usize,
}

impl DeformerBuffer {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live deformers"),
            entries: &deformer_entries(),
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live deformer parameters"),
            size: std::mem::size_of::<PackedDeformer>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let plan_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live roller plan"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = Self::group(device, &layout, &buffer, &plan_buffer);
        Self {
            layout,
            group,
            buffer,
            capacity: 1,
            plan_buffer,
            plan_capacity: 1,
            len: 0,
        }
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    pub fn plan_buffer(&self) -> &wgpu::Buffer {
        &self.plan_buffer
    }

    fn group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        buffer: &wgpu::Buffer,
        plan_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live deformers"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: plan_buffer.as_entire_binding(),
                },
            ],
        })
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        deformers: &[Deformer],
    ) -> Result<(), String> {
        let (packed, plans) = pack_with_plans(deformers)?;
        let mut regroup = false;
        if packed.len() > self.capacity {
            self.buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("live deformer parameters"),
                contents: bytemuck::cast_slice(&packed),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });
            self.capacity = packed.len();
            regroup = true;
        } else {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&packed));
        }
        if plans.len() > self.plan_capacity {
            self.plan_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("live roller plan"),
                contents: bytemuck::cast_slice(&plans),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });
            self.plan_capacity = plans.len();
            regroup = true;
        } else {
            queue.write_buffer(&self.plan_buffer, 0, bytemuck::cast_slice(&plans));
        }
        if regroup {
            self.group = Self::group(device, &self.layout, &self.buffer, &self.plan_buffer);
        }
        self.len = deformers.len();
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeformedRollerVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
    pub previous_position: [f32; 3],
    pub uv: [f32; 2],
}

pub fn roller_vertex(
    rest: [f32; 3],
    normal: [f32; 3],
    tangent: [f32; 4],
    uv: [f32; 2],
    current: RollerFrame,
    previous: RollerFrame,
) -> DeformedRollerVertex {
    let make = |state: RollerFrame| {
        Roller {
            at: state.at,
            core: state.outer,
            thickness: state.thickness,
        }
        .place(rest, state.outer)
    };
    let now = make(current);
    let before = make(previous);
    let rotate = |v: [f32; 3]| {
        [
            now.tangent[0] * v[0] + now.normal[0] * v[1],
            now.tangent[1] * v[0] + now.normal[1] * v[1],
            v[2],
        ]
    };
    let n = rotate(normal);
    let t = rotate([tangent[0], tangent[1], tangent[2]]);
    DeformedRollerVertex {
        position: now.position,
        normal: n,
        tangent: [t[0], t[1], t[2], tangent[3]],
        previous_position: before.position,
        uv,
    }
}

pub fn plan_sample(plan: &[[f32; 4]], s: f32) -> ([f32; 2], [f32; 2]) {
    let s = s.clamp(plan[0][0], plan[plan.len() - 1][0]);
    let high = plan
        .partition_point(|point| point[0] < s)
        .clamp(1, plan.len() - 1);
    let low = high - 1;
    let a = plan[low];
    let b = plan[high];
    let before = plan[low.saturating_sub(1)];
    let after = plan[(high + 1).min(plan.len() - 1)];
    let span = b[0] - a[0];
    let u = (s - a[0]) / span;
    let u2 = u * u;
    let u3 = u2 * u;
    let unit = |x: f32, y: f32| {
        let length = x.hypot(y);
        [x / length, y / length]
    };
    let ta = unit(b[1] - before[1], b[2] - before[2]);
    let tb = unit(after[1] - a[1], after[2] - a[2]);
    let position = std::array::from_fn(|i| {
        let k = i + 1;
        (2.0 * u3 - 3.0 * u2 + 1.0) * a[k]
            + (u3 - 2.0 * u2 + u) * span * ta[i]
            + (-2.0 * u3 + 3.0 * u2) * b[k]
            + (u3 - u2) * span * tb[i]
    });
    let derivative: [f32; 2] = std::array::from_fn(|i| {
        let k = i + 1;
        (6.0 * u2 - 6.0 * u) * a[k] / span
            + (3.0 * u2 - 4.0 * u + 1.0) * ta[i]
            + (-6.0 * u2 + 6.0 * u) * b[k] / span
            + (3.0 * u2 - 2.0 * u) * tb[i]
    });
    (position, unit(derivative[0], derivative[1]))
}

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn flattened(keep: f32, normal: [f32; 3], tangent: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    if keep >= 1.0 {
        return (normal, tangent);
    }
    let side = if normal[1] < 0.0 { -1.0 } else { 1.0 };
    let mixed = [
        normal[0] * keep,
        side + (normal[1] - side) * keep,
        normal[2] * keep,
    ];
    let length = mixed.iter().map(|v| v * v).sum::<f32>().sqrt();
    let n = mixed.map(|v| v / length);
    let along_n = tangent.iter().zip(n).map(|(a, b)| a * b).sum::<f32>();
    let along: [f32; 3] = std::array::from_fn(|i| tangent[i] - n[i] * along_n);
    let squared = along.iter().map(|v| v * v).sum::<f32>();
    let t = if squared < 1e-12 {
        [1.0, 0.0, 0.0]
    } else {
        along.map(|v| v / squared.sqrt())
    };
    (n, t)
}

pub fn curved_roller_vertex(
    rest: [f32; 3],
    normal: [f32; 3],
    tangent: [f32; 4],
    uv: [f32; 2],
    current: RollerFrame,
    previous: RollerFrame,
    plan: &[[f32; 4]],
) -> DeformedRollerVertex {
    if plan
        .iter()
        .all(|point| point[1] == point[0] && point[2] == 0.0)
    {
        return roller_vertex(rest, normal, tangent, uv, current, previous);
    }
    let keep = |state: RollerFrame| {
        1.0 - smoothstep(0.0, std::f32::consts::PI * state.outer, rest[0] - state.at)
    };
    let place = |state: RollerFrame| {
        let (anchor, direction) = plan_sample(plan, rest[0].min(state.at));
        let side = [-direction[1], direction[0]];
        let roll = Roller {
            at: state.at,
            core: state.outer,
            thickness: state.thickness,
        }
        .place(rest, state.outer);
        if rest[0] <= state.at {
            (
                [
                    anchor[0] + side[0] * rest[1],
                    anchor[1] + side[1] * rest[1],
                    rest[2],
                ],
                [direction[0], direction[1]],
                side,
            )
        } else {
            let dx = roll.position[0] - state.at;
            let travel = rest[0] - state.at;
            let shrink = state.thickness / std::f32::consts::TAU;
            let fade = (-travel / state.outer).exp();
            let dy = roll.position[1] - shrink * (1.0 - fade);
            let raw = [
                roll.tangent[0],
                roll.tangent[1] - shrink / state.outer * fade,
            ];
            let length = raw[0].hypot(raw[1]);
            let local_tangent = [raw[0] / length, raw[1] / length];
            let tangent = [
                direction[0] * local_tangent[0] + side[0] * local_tangent[1],
                direction[1] * local_tangent[0] + side[1] * local_tangent[1],
            ];
            let normal = [-tangent[1], tangent[0]];
            let lift = rest[1] * keep(state);
            (
                [
                    anchor[0] + direction[0] * dx + side[0] * dy + normal[0] * lift,
                    anchor[1] + direction[1] * dx + side[1] * dy + normal[1] * lift,
                    rest[2],
                ],
                tangent,
                normal,
            )
        }
    };
    let now = place(current);
    let before = place(previous);
    let rotate = |v: [f32; 3]| {
        [
            now.1[0] * v[0] + now.2[0] * v[1],
            now.1[1] * v[0] + now.2[1] * v[1],
            v[2],
        ]
    };
    let (flat_normal, flat_tangent) =
        flattened(keep(current), normal, [tangent[0], tangent[1], tangent[2]]);
    let n = rotate(flat_normal);
    let t = rotate(flat_tangent);
    DeformedRollerVertex {
        position: now.0,
        normal: n,
        tangent: [t[0], t[1], t[2], tangent[3]],
        previous_position: before.0,
        uv,
    }
}

pub fn deformer_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_geom::strip::Strip;

    #[test]
    fn parameter_packing() {
        assert_eq!(std::mem::size_of::<PackedDeformer>(), 80);
        assert_eq!(DeformerId::NONE.0, NONE);
        let roller = Deformer::Roller {
            current: RollerFrame {
                at: 0.3,
                outer: 0.2,
                thickness: 0.01,
            },
            previous: RollerFrame {
                at: 0.4,
                outer: 0.22,
                thickness: 0.01,
            },
        };
        let wind = Deformer::Wind {
            current: WindFrame {
                time: 1.5,
                strength: 2.0,
                direction: [1.0, 0.0, 0.0],
            },
            previous: WindFrame {
                time: 1.0,
                strength: 1.5,
                direction: [0.0, 0.0, 1.0],
            },
            object: 42,
            seed: 7,
        };
        let packed = pack(&[roller, wind]).unwrap();
        assert_eq!(packed[0].kind, [ROLLER, 0, 0, 0]);
        assert_eq!(packed[0].current_a, [0.3, 0.2, 0.01, 0.0]);
        assert_eq!(packed[0].previous_a, [0.4, 0.22, 0.01, 0.0]);
        assert_eq!(packed[1].kind, [WIND, 42, 7, 0]);
        assert_eq!(packed[1].current_b, [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(packed[1].previous_b, [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(bytemuck::bytes_of(&packed[0]).len(), 80);
        assert!(
            pack(&[Deformer::Roller {
                current: RollerFrame {
                    at: 0.3,
                    outer: 0.0,
                    thickness: 0.01
                },
                previous: RollerFrame {
                    at: 0.4,
                    outer: 0.2,
                    thickness: 0.01
                },
            }])
            .is_err()
        );
    }

    #[test]
    fn roller_matches_geom_and_keeps_uv() {
        let current = RollerFrame {
            at: 0.4,
            outer: 0.17,
            thickness: 0.008,
        };
        let previous = RollerFrame {
            at: 0.48,
            outer: 0.18,
            thickness: 0.008,
        };
        let roller = Roller {
            at: current.at,
            core: current.outer,
            thickness: current.thickness,
        };
        let before = Roller {
            at: previous.at,
            core: previous.outer,
            thickness: previous.thickness,
        };
        for rest in [
            [0.1, 0.0, 0.2],
            [0.4, 0.0, 0.5],
            [0.51, 0.0, 0.5],
            [0.8, 0.0, 0.8],
        ] {
            let uv = [rest[0], rest[2]];
            let got = roller_vertex(
                rest,
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0, 1.0],
                uv,
                current,
                previous,
            );
            let reference = roller.place(rest, current.outer);
            let previous_reference = before.place(rest, previous.outer);
            assert_eq!(got.position, reference.position);
            assert_eq!(got.normal, reference.normal);
            assert_eq!(
                got.tangent,
                [
                    reference.tangent[0],
                    reference.tangent[1],
                    reference.tangent[2],
                    1.0
                ]
            );
            assert_eq!(got.previous_position, previous_reference.position);
            assert_eq!(got.uv, uv);
            if rest[0] > current.at {
                assert_ne!(got.position, got.previous_position);
            }
        }
    }

    #[test]
    fn straight_plan_matches_flat_roller() {
        let plan = [
            [0.0, 0.0, 0.0, 0.0],
            [0.5, 0.5, 0.0, 0.0],
            [1.0, 1.0, 0.0, 0.0],
        ];
        let current = RollerFrame {
            at: 0.4,
            outer: 0.17,
            thickness: 0.008,
        };
        let previous = RollerFrame {
            at: 0.48,
            outer: 0.18,
            thickness: 0.008,
        };
        let packed = pack(&[Deformer::CurvedRoller {
            current,
            previous,
            plan: plan.to_vec().into(),
        }])
        .unwrap();
        assert_eq!(packed[0].kind[3], 1);
        for s in [0.0, 0.25, 0.4, 0.5, 0.8, 1.0] {
            let rest = [s, 0.0, 0.2];
            let flat = roller_vertex(
                rest,
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0, 1.0],
                [s, 0.2],
                current,
                previous,
            );
            let curved = curved_roller_vertex(
                rest,
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0, 1.0],
                [s, 0.2],
                current,
                previous,
                &plan,
            );
            for (a, b) in flat.position.iter().zip(curved.position) {
                assert!((a - b).abs() < 1e-6);
            }
            for (a, b) in flat.previous_position.iter().zip(curved.previous_position) {
                assert!((a - b).abs() < 1e-6);
            }
            assert_eq!(flat.normal, curved.normal);
            assert_eq!(flat.tangent, curved.tangent);
        }
    }

    #[test]
    fn curved_plan_packs_arc_length_samples() {
        let plan: Arc<[[f32; 4]]> = vec![[0.0, 0.0, 0.0, 0.0], [0.5, 0.49, 0.05, 0.0]].into();
        let state = RollerFrame {
            at: 0.4,
            outer: 0.02,
            thickness: 0.002,
        };
        let roller = Deformer::CurvedRoller {
            current: state,
            previous: state,
            plan: plan.clone(),
        };
        let (packed, samples) = pack_with_plans(&[roller.clone(), roller]).unwrap();
        assert_eq!(packed[0].kind, [CURVED_ROLLER, 0, 2, 0]);
        assert_eq!(packed[1].kind, [CURVED_ROLLER, 2, 2, 0]);
        assert_eq!(samples, [&plan[..], &plan[..]].concat());
        let bad: Arc<[[f32; 4]]> = vec![[0.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]].into();
        assert!(
            pack(&[Deformer::CurvedRoller {
                current: state,
                previous: state,
                plan: bad
            }])
            .is_err()
        );
    }

    #[test]
    fn curved_plan_samples_and_join_are_continuous() {
        let plan = [
            [0.0, 0.0, 0.0, 0.0],
            [0.2, 0.19, 0.05, 0.0],
            [0.4, 0.38, 0.02, 0.0],
            [0.6, 0.58, 0.0, 0.0],
        ];
        for knot in plan.iter().skip(1).take(2) {
            let left = plan_sample(&plan, knot[0] - 1e-5);
            let middle = plan_sample(&plan, knot[0]);
            let right = plan_sample(&plan, knot[0] + 1e-5);
            assert!((middle.0[0] - knot[1]).abs() < 1e-6);
            assert!((middle.0[1] - knot[2]).abs() < 1e-6);
            for i in 0..2 {
                assert!((left.1[i] - right.1[i]).abs() < 1e-3);
            }
        }
        let state = RollerFrame {
            at: 0.3,
            outer: 0.02,
            thickness: 0.002,
        };
        let get = |s| {
            curved_roller_vertex(
                [s, 0.0, 0.1],
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0, 1.0],
                [s, 0.1],
                state,
                state,
                &plan,
            )
        };
        let left = get(state.at - 1e-6);
        let right = get(state.at + 1e-6);
        for i in 0..3 {
            assert!((left.position[i] - right.position[i]).abs() < 1e-5);
            assert!((left.normal[i] - right.normal[i]).abs() < 1e-3);
            assert!((left.tangent[i] - right.tangent[i]).abs() < 1e-3);
        }
    }

    #[test]
    fn moving_fold_has_previous_position_and_attached_glyphs() {
        let plan = [
            [0.0, 0.0, 0.0, 0.0],
            [0.3, 0.29, 0.04, 0.0],
            [0.6, 0.58, 0.0, 0.0],
        ];
        let current = RollerFrame {
            at: 0.32,
            outer: 0.018,
            thickness: 0.002,
        };
        let previous = RollerFrame {
            at: 0.42,
            outer: 0.016,
            thickness: 0.002,
        };
        let glyph = [0.36, 0.0, 0.14];
        let ink = curved_roller_vertex(
            glyph,
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0, 1.0],
            [glyph[0], glyph[2]],
            current,
            previous,
            &plan,
        );
        let offset = ((ink.position[0] - ink.previous_position[0]).powi(2)
            + (ink.position[1] - ink.previous_position[1]).powi(2))
        .sqrt();
        assert!(offset > 0.001);
        assert_eq!(ink.uv, [glyph[0], glyph[2]]);
        let lifted = |s: f32, lift: f32| {
            curved_roller_vertex(
                [s, lift, glyph[2]],
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0, 1.0],
                [s, glyph[2]],
                current,
                previous,
                &plan,
            )
        };
        let open = lifted(0.2, 0.0);
        let raised = lifted(0.2, 0.004);
        for i in 0..3 {
            assert!((raised.position[i] - open.position[i] - open.normal[i] * 0.004).abs() < 1e-6);
        }
        let wound = current.at + std::f32::consts::PI * current.outer;
        for s in [wound, wound + 0.05] {
            let flat = lifted(s, 0.0);
            let raised = lifted(s, 0.004);
            assert_eq!(raised.position, flat.position);
        }
        let joined = |lift: f32| {
            let left = lifted(current.at - 1e-6, lift);
            let right = lifted(current.at + 1e-6, lift);
            (0..3).all(|i| (left.position[i] - right.position[i]).abs() < 1e-5)
        };
        assert!(joined(0.004));
        let tilted = curved_roller_vertex(
            [wound + 0.01, 0.004, glyph[2]],
            [0.6, 0.8, 0.0],
            [0.8, -0.6, 0.0, 1.0],
            ink.uv,
            current,
            previous,
            &plan,
        );
        let flat = lifted(wound + 0.01, 0.0);
        for i in 0..3 {
            assert!((tilted.normal[i] - flat.normal[i]).abs() < 1e-5);
            assert!((tilted.tangent[i] - flat.tangent[i]).abs() < 1e-5);
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn curved_glyph_shader_matches_cpu_mirror() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let device = &gpu.device;
        let plan: Arc<[[f32; 4]]> = vec![
            [0.0, 0.0, 0.0, 0.0],
            [0.2, 0.19, 0.04, 0.0],
            [0.4, 0.38, 0.02, 0.0],
            [0.6, 0.58, 0.0, 0.0],
        ]
        .into();
        let current = RollerFrame {
            at: 0.32,
            outer: 0.018,
            thickness: 0.002,
        };
        let previous = RollerFrame {
            at: 0.42,
            outer: 0.016,
            thickness: 0.002,
        };
        let mut deformers = DeformerBuffer::new(device);
        deformers
            .upload(
                device,
                &gpu.queue,
                &[Deformer::CurvedRoller {
                    current,
                    previous,
                    plan: plan.clone(),
                }],
            )
            .unwrap();
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("curved glyph positions"),
            size: 192,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("curved glyph readback"),
            size: 192,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let result_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let result_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &result_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: output.as_entire_binding(),
            }],
        });
        let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[],
        });
        let empty_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &empty,
            entries: &[],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    shader_source(),
                    r#"
@group(0) @binding(0) var<storage, read_write> result: array<vec4f>;
@compute @workgroup_size(3)
fn main(@builtin(local_invocation_index) index: u32) {
    let rest = vec3f(0.30 + f32(index) * 0.04, 0.004, 0.14);
    let shape = deform_vertex(rest, normalize(vec3f(0.3, 1.0, 0.2)), vec4f(normalize(vec3f(1.0, -0.3, 0.0)), 1.0), 0u);
    result[index * 4u] = vec4f(shape.position, 1.0);
    result[index * 4u + 1u] = vec4f(shape.previous_position, 1.0);
    result[index * 4u + 2u] = vec4f(shape.normal, 0.0);
    result[index * 4u + 3u] = shape.tangent;
}
"#
                )
                .into(),
            ),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&result_layout, &empty, &deformers.layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: Some(&layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &result_group, &[]);
            pass.set_bind_group(1, &empty_group, &[]);
            pass.set_bind_group(2, &deformers.group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 192);
        gpu.queue.submit(Some(encoder.finish()));
        let (send, receive) = std::sync::mpsc::channel();
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        receive.recv().unwrap().unwrap();
        let mapped = slice.get_mapped_range();
        let values: &[[f32; 4]] = bytemuck::cast_slice(&mapped[..]);
        let unit = |v: [f32; 3]| {
            let length = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            v.map(|x| x / length)
        };
        let normal = unit([0.3, 1.0, 0.2]);
        let tangent = unit([1.0, -0.3, 0.0]);
        for index in 0..3 {
            let s = 0.30 + index as f32 * 0.04;
            let mirror = curved_roller_vertex(
                [s, 0.004, 0.14],
                normal,
                [tangent[0], tangent[1], tangent[2], 1.0],
                [s, 0.14],
                current,
                previous,
                &plan,
            );
            let close = |actual: &[f32], expected: &[f32], tolerance: f32| {
                actual
                    .iter()
                    .zip(expected)
                    .all(|(a, e)| (a - e).abs() < tolerance)
            };
            assert!(close(&values[index * 4][..3], &mirror.position, 2e-5));
            assert!(close(
                &values[index * 4 + 1][..3],
                &mirror.previous_position,
                2e-5
            ));
            assert!(close(&values[index * 4 + 2][..3], &mirror.normal, 1e-4));
            assert!(close(&values[index * 4 + 3], &mirror.tangent, 1e-4));
        }
        drop(mapped);
        readback.unmap();
    }

    #[test]
    fn shader_has_shared_deformation() {
        let source = format!(
            "{}\n@vertex fn main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {{ let d = deform_vertex(vec3f(f32(index), 0.0, 0.0), vec3f(0.0, 1.0, 0.0), vec4f(1.0, 0.0, 0.0, 1.0), 0u); return vec4f(d.position, 1.0); }}",
            shader_source()
        );
        naga::front::wgsl::parse_str(&source).unwrap();
    }

    #[repr(C)]
    #[derive(Clone, Copy, Pod, Zeroable)]
    struct TestVertex {
        rest: [f32; 4],
        normal: [f32; 4],
        tangent: [f32; 4],
        uv: [f32; 4],
    }

    const TEST_SHADER: &str = r#"
struct Input {
    @location(0) rest: vec4f,
    @location(1) normal: vec4f,
    @location(2) tangent: vec4f,
    @location(3) uv: vec4f,
}
struct Output {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
    @location(1) height: f32,
    @location(2) shadow_difference: f32,
}
@vertex fn vs_main(input: Input) -> Output {
    let v = deform_vertex(input.rest.xyz, input.normal.xyz, input.tangent, 0u);
    let shadow_position = deform_position(input.rest.xyz, 0u, false);
    var out: Output;
    out.clip = vec4f(v.position.x * 2.0 - 1.0, v.position.z * 2.0 - 1.0, 0.8 - v.position.y, 1.0);
    out.uv = input.uv.xy;
    out.height = v.position.y;
    out.shadow_difference = distance(v.position, shadow_position);
    return out;
}
@fragment fn fs_main(input: Output) -> @location(0) vec4f {
    if (input.shadow_difference > 0.00001) {
        return vec4f(1.0, 0.0, 1.0, 1.0);
    }
    let ink = abs(fract(input.uv.x * 10.0) - 0.5) < 0.12 && input.uv.y > 0.25 && input.uv.y < 0.75;
    let colour = select(vec3f(0.95, 0.9, 0.8), vec3f(0.0), ink);
    return vec4f(colour, clamp(input.height * 2.0, 0.0, 1.0));
}
"#;

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn rolled_strip_keeps_ink_on_raised_paper() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let device = &gpu.device;
        let strip = Strip {
            length: 1.0,
            width: 1.0,
        };
        let mesh = strip.mesh(0.17, 0.0003, None, 12);
        let vertices: Vec<TestVertex> = mesh
            .positions
            .iter()
            .enumerate()
            .map(|(index, rest)| TestVertex {
                rest: [rest[0], rest[1], rest[2], 1.0],
                normal: [0.0, 1.0, 0.0, 0.0],
                tangent: [1.0, 0.0, 0.0, 1.0],
                uv: [mesh.uvs[index][0], mesh.uvs[index][1], 0.0, 0.0],
            })
            .collect();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("deform test strip"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("deform test indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let mut deformers = DeformerBuffer::new(device);
        let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[],
        });
        let empty_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &empty,
            entries: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("deform test shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{}\n{}", shader_source(), TEST_SHADER).into(),
            ),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&empty, &empty, &deformers.layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("deform test pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<TestVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4],
                }],
            },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        let target = gpu
            .offscreen(256, 256, wgpu::TextureFormat::Rgba8UnormSrgb)
            .unwrap();
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 256,
                height: 256,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&Default::default());
        for at in [0.2, 0.4, 0.6] {
            let current = RollerFrame {
                at,
                outer: 0.17,
                thickness: 0.004,
            };
            deformers
                .upload(
                    device,
                    &gpu.queue,
                    &[Deformer::Roller {
                        current,
                        previous: current,
                    }],
                )
                .unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("deform test render"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::GREEN),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &empty_group, &[]);
                pass.set_bind_group(1, &empty_group, &[]);
                pass.set_bind_group(2, &deformers.group, &[]);
                pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.indices.len() as u32, 0, 0..1);
            }
            gpu.queue.submit(Some(encoder.finish()));
            let pixels = gpu.readback_rgba8(&target).unwrap();
            let mut raised_paper = 0;
            let mut raised_ink = 0;
            let mut flat_ink = 0;
            let mut flat_paper = 0;
            let mut shadow_mismatch = 0;
            for pixel in pixels.chunks_exact(4) {
                if pixel[0] > 200 && pixel[1] < 50 && pixel[2] > 200 {
                    shadow_mismatch += 1;
                }
                if pixel[0] < 50 && pixel[1] < 50 && pixel[2] < 50 {
                    if pixel[3] > 30 {
                        raised_ink += 1;
                    }
                    if pixel[3] == 0 {
                        flat_ink += 1;
                    }
                }
                if pixel[0] > 150 && pixel[1] > 150 && pixel[2] > 100 && pixel[3] > 30 {
                    raised_paper += 1;
                }
                if pixel[0] > 150 && pixel[1] > 150 && pixel[2] > 100 && pixel[3] == 0 {
                    flat_paper += 1;
                }
            }
            assert!(raised_paper > 100, "no raised paper at {at}");
            assert!(raised_ink > 10, "no attached ink at {at}");
            assert!(flat_paper > 100, "no flat paper at {at}");
            assert_eq!(shadow_mismatch, 0, "shadow deformation differs at {at}");
            if at > 0.5 {
                assert!(flat_ink > 10, "no ink on flat paper at {at}");
            }
        }
    }
}
