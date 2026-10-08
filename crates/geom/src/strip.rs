use crate::mesh::{Builder, Mesh};
use std::f32::consts::{PI, TAU};

pub const ROLL_WGSL: &str = r#"
struct Rolled {
    position: vec3<f32>,
    tangent: vec3<f32>,
    normal: vec3<f32>,
}

fn roll_outer(core: f32, thickness: f32, wound: f32) -> f32 {
    return sqrt(core * core + max(wound, 0.0) * thickness / 3.14159265);
}

fn roll_place(rest: vec3<f32>, at: f32, outer: f32, thickness: f32) -> Rolled {
    var out: Rolled;
    let s = rest.x - at;
    if (s <= 0.0) {
        out.position = vec3<f32>(rest.x, 0.0, rest.z);
        out.tangent = vec3<f32>(1.0, 0.0, 0.0);
        out.normal = vec3<f32>(0.0, 1.0, 0.0);
        return out;
    }
    var theta = s / outer;
    if (thickness > 1e-7) {
        let inside = max(outer * outer - thickness * s / 3.14159265, 0.0);
        theta = (outer - sqrt(inside)) * 6.28318531 / thickness;
    }
    let shrink = thickness / 6.28318531;
    let r = outer - shrink * theta;
    let around = vec2<f32>(sin(theta), -cos(theta));
    let along = vec2<f32>(cos(theta), sin(theta));
    let flat = -shrink * around + r * along;
    let t = normalize(flat);
    out.position = vec3<f32>(at + r * around.x, outer + r * around.y, rest.z);
    out.tangent = vec3<f32>(t.x, t.y, 0.0);
    out.normal = vec3<f32>(-t.y, t.x, 0.0);
    return out;
}
"#;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strip {
    pub length: f32,
    pub width: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Roller {
    pub at: f32,
    pub core: f32,
    pub thickness: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rolled {
    pub position: [f32; 3],
    pub tangent: [f32; 3],
    pub normal: [f32; 3],
}

impl Roller {
    pub fn outer(&self, strip: &Strip) -> f32 {
        let wound = (strip.length - self.at).max(0.0);
        (self.core * self.core + wound * self.thickness / PI).sqrt()
    }

    pub fn place(&self, rest: [f32; 3], outer: f32) -> Rolled {
        let s = rest[0] - self.at;
        if s <= 0.0 {
            return Rolled {
                position: [rest[0], 0.0, rest[2]],
                tangent: [1.0, 0.0, 0.0],
                normal: [0.0, 1.0, 0.0],
            };
        }
        let theta = if self.thickness > 1e-7 {
            let inside = (outer * outer - self.thickness * s / PI).max(0.0);
            (outer - inside.sqrt()) * TAU / self.thickness
        } else {
            s / outer
        };
        let shrink = self.thickness / TAU;
        let r = outer - shrink * theta;
        let around = [theta.sin(), -theta.cos()];
        let along = [theta.cos(), theta.sin()];
        let flat = [
            -shrink * around[0] + r * along[0],
            -shrink * around[1] + r * along[1],
        ];
        let len = (flat[0] * flat[0] + flat[1] * flat[1]).sqrt().max(1e-12);
        let t = [flat[0] / len, flat[1] / len];
        Rolled {
            position: [self.at + r * around[0], outer + r * around[1], rest[2]],
            tangent: [t[0], t[1], 0.0],
            normal: [-t[1], t[0], 0.0],
        }
    }
}

impl Strip {
    pub fn columns(&self, core: f32, error: f32, band: Option<(f32, f32)>) -> Vec<f32> {
        let step = column_step(core, error);
        let (from, to) = band
            .map(|(a, b)| (a.clamp(0.0, self.length), b.clamp(0.0, self.length)))
            .unwrap_or((0.0, self.length));
        let (from, to) = (from.min(to), from.max(to));
        let mut out = vec![0.0];
        if from > 0.0 {
            out.push(from);
        }
        let count = ((to - from) / step).ceil().max(1.0) as usize;
        for i in 1..=count {
            out.push(from + (to - from) * i as f32 / count as f32);
        }
        if to < self.length {
            out.push(self.length);
        }
        out.dedup_by(|a, b| (*a - *b).abs() <= 1e-7);
        out
    }

    pub fn mesh(&self, core: f32, error: f32, band: Option<(f32, f32)>, across: usize) -> Mesh {
        let xs = self.columns(core, error, band);
        let across = across.max(1);
        let mut builder = Builder::new();
        let mut grid = Vec::with_capacity(xs.len());
        for x in &xs {
            let mut column = Vec::with_capacity(across + 1);
            for j in 0..=across {
                let z = self.width * j as f32 / across as f32;
                let uv = [x / self.length.max(1e-6), z / self.width.max(1e-6)];
                column.push(builder.vertex([*x, 0.0, z], [0.0, 1.0, 0.0], uv));
            }
            grid.push(column);
        }
        for i in 0..xs.len() - 1 {
            for j in 0..across {
                let (a, b, c, d) = (
                    grid[i][j],
                    grid[i + 1][j],
                    grid[i + 1][j + 1],
                    grid[i][j + 1],
                );
                builder.tri(a, b, c);
                builder.tri(a, c, d);
            }
        }
        builder.build()
    }
}

pub fn column_step(core: f32, error: f32) -> f32 {
    let core = core.max(1e-6);
    let error = error.clamp(core * 1e-5, core);
    let alpha = 2.0 * (1.0 - error / core).acos();
    (core * alpha).max(1e-6)
}
