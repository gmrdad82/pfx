use std::fmt::Write;

use crate::net::{Layer, OUT_BLOCKS_PER_GROUP};

pub const BLOCK_W: u32 = 32;
pub const BLOCK_H: u32 = 16;
const THREADS_X: u32 = 8;
const THREADS_Y: u32 = 8;
const PIXELS_X: u32 = 4;
const PIXELS_Y: u32 = 2;
const PATCH_W: u32 = BLOCK_W + 2;
const PATCH_H: u32 = BLOCK_H + 2;
const LANES: [char; 4] = ['x', 'y', 'z', 'w'];

pub fn process() -> &'static str {
    include_str!("process.wgsl")
}

pub struct ConvShape {
    pub width: u32,
    pub height: u32,
}

pub fn conv(layer: &Layer, shape: &ConvShape) -> String {
    let cb0 = layer.in0_blocks();
    let cb = cb0 + layer.in1_blocks();
    let ob = layer.out_blocks();
    let per = OUT_BLOCKS_PER_GROUP;
    let concat = layer.src1.is_some();
    let (w, h) = (shape.width, shape.height);
    let (w0, h0) = if layer.upsample {
        (w / 2, h / 2)
    } else {
        (w, h)
    };
    let mut s = String::new();
    writeln!(s, "struct Rows {{ row: u32 }}").unwrap();
    writeln!(s, "@group(0) @binding(0) var<uniform> rows: Rows;").unwrap();
    writeln!(
        s,
        "@group(0) @binding(1) var<storage, read> src0: array<vec4f>;"
    )
    .unwrap();
    if concat {
        writeln!(
            s,
            "@group(0) @binding(2) var<storage, read> src1: array<vec4f>;"
        )
        .unwrap();
    }
    writeln!(
        s,
        "@group(0) @binding(3) var<storage, read> weights: array<vec4f>;"
    )
    .unwrap();
    writeln!(
        s,
        "@group(0) @binding(4) var<storage, read> bias: array<vec4f>;"
    )
    .unwrap();
    writeln!(
        s,
        "@group(0) @binding(5) var<storage, read_write> dst: array<vec4f>;"
    )
    .unwrap();
    writeln!(
        s,
        "var<workgroup> slab: array<vec4f, {}>;",
        PATCH_W * PATCH_H
    )
    .unwrap();
    writeln!(s, "fn fetch(c: u32, y: i32, x: i32) -> vec4f {{").unwrap();
    writeln!(
        s,
        "  if (y < 0 || y >= {h} || x < 0 || x >= {w}) {{ return vec4f(0.0); }}"
    )
    .unwrap();
    if concat {
        writeln!(
            s,
            "  if (c >= {cb0}u) {{ return src1[((c - {cb0}u) * {h}u + u32(y)) * {w}u + u32(x)]; }}"
        )
        .unwrap();
    }
    if layer.upsample {
        writeln!(
            s,
            "  return src0[(c * {h0}u + u32(y >> 1u)) * {w0}u + u32(x >> 1u)];"
        )
        .unwrap();
    } else {
        writeln!(s, "  return src0[(c * {h}u + u32(y)) * {w}u + u32(x)];").unwrap();
    }
    writeln!(s, "}}").unwrap();
    writeln!(s, "@compute @workgroup_size({THREADS_X}, {THREADS_Y}, 1)").unwrap();
    writeln!(s, "fn main(@builtin(local_invocation_id) lid: vec3u, @builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wid: vec3u) {{").unwrap();
    writeln!(s, "  let g = wid.z;").unwrap();
    writeln!(s, "  let bx = i32(wid.x) * {BLOCK_W};").unwrap();
    writeln!(s, "  let by = i32(wid.y + rows.row) * {BLOCK_H};").unwrap();
    for o in 0..per {
        writeln!(s, "  let b{o} = bias[g * {per}u + {o}u];").unwrap();
    }
    for y in 0..PIXELS_Y {
        for x in 0..PIXELS_X {
            for o in 0..per {
                writeln!(s, "  var a{y}{x}{o} = b{o};").unwrap();
            }
        }
    }
    writeln!(
        s,
        "  let p = lid.y * {}u + lid.x * {PIXELS_X}u;",
        PIXELS_Y * PATCH_W
    )
    .unwrap();
    writeln!(s, "  for (var c = 0u; c < {cb}u; c++) {{").unwrap();
    writeln!(s, "    workgroupBarrier();").unwrap();
    writeln!(
        s,
        "    for (var i = li; i < {}u; i += {}u) {{",
        PATCH_W * PATCH_H,
        THREADS_X * THREADS_Y
    )
    .unwrap();
    writeln!(s, "      let py = i / {PATCH_W}u;").unwrap();
    writeln!(s, "      let px = i - py * {PATCH_W}u;").unwrap();
    writeln!(
        s,
        "      slab[i] = fetch(c, by + i32(py) - 1, bx + i32(px) - 1);"
    )
    .unwrap();
    writeln!(s, "    }}").unwrap();
    writeln!(s, "    workgroupBarrier();").unwrap();
    writeln!(s, "    let k = (g * {cb}u + c) * {}u;", 9 * 4 * per).unwrap();
    for r in 0..PIXELS_Y + 2 {
        for j in 0..PIXELS_X + 2 {
            writeln!(s, "    let v{r}{j} = slab[p + {}u];", r * PATCH_W + j).unwrap();
        }
    }
    for ky in 0..3 {
        for kx in 0..3 {
            for l in 0..4 {
                for o in 0..per {
                    let index = ((ky * 3 + kx) * 4 + l) * per + o;
                    writeln!(s, "    let w{o}_{ky}{kx}{l} = weights[k + {index}u];").unwrap();
                }
                for y in 0..PIXELS_Y {
                    for x in 0..PIXELS_X {
                        for o in 0..per {
                            writeln!(
                                s,
                                "    a{y}{x}{o} = fma(w{o}_{ky}{kx}{l}, vec4f(v{}{}.{}), a{y}{x}{o});",
                                y + ky,
                                x + kx,
                                LANES[l as usize]
                            )
                            .unwrap();
                        }
                    }
                }
            }
        }
    }
    writeln!(s, "  }}").unwrap();
    if layer.pool {
        let (wo, ho) = (w / 2, h / 2);
        for x in 0..PIXELS_X / 2 {
            writeln!(s, "  let oy{x} = by / 2 + i32(lid.y);").unwrap();
            writeln!(
                s,
                "  let ox{x} = bx / 2 + i32(lid.x) * {} + {x};",
                PIXELS_X / 2
            )
            .unwrap();
            writeln!(s, "  if (oy{x} < {ho} && ox{x} < {wo}) {{").unwrap();
            for o in 0..per {
                writeln!(s, "    if (g * {per}u + {o}u < {ob}u) {{").unwrap();
                writeln!(
                    s,
                    "      dst[((g * {per}u + {o}u) * {ho}u + u32(oy{x})) * {wo}u + u32(ox{x})] = max(max(max(a0{a}{o}, a0{b}{o}), max(a1{a}{o}, a1{b}{o})), vec4f(0.0));",
                    a = 2 * x,
                    b = 2 * x + 1
                )
                .unwrap();
                writeln!(s, "    }}").unwrap();
            }
            writeln!(s, "  }}").unwrap();
        }
    } else {
        for y in 0..PIXELS_Y {
            for x in 0..PIXELS_X {
                writeln!(s, "  let y{y}{x} = by + i32(lid.y) * {PIXELS_Y} + {y};").unwrap();
                writeln!(s, "  let x{y}{x} = bx + i32(lid.x) * {PIXELS_X} + {x};").unwrap();
                writeln!(s, "  if (y{y}{x} < {h} && x{y}{x} < {w}) {{").unwrap();
                for o in 0..per {
                    writeln!(s, "    if (g * {per}u + {o}u < {ob}u) {{").unwrap();
                    writeln!(
                        s,
                        "      dst[((g * {per}u + {o}u) * {h}u + u32(y{y}{x})) * {w}u + u32(x{y}{x})] = max(a{y}{x}{o}, vec4f(0.0));"
                    )
                    .unwrap();
                    writeln!(s, "    }}").unwrap();
                }
                writeln!(s, "  }}").unwrap();
            }
        }
    }
    writeln!(s, "}}").unwrap();
    s
}

pub fn groups(layer: &Layer, shape: &ConvShape) -> [u32; 3] {
    [
        shape.width.div_ceil(BLOCK_W),
        shape.height.div_ceil(BLOCK_H),
        layer.groups(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aux::Aux;

    #[test]
    fn every_generated_shader_validates() {
        let mut sources = vec![process().to_string()];
        for aux in [Aux::Noisy, Aux::Prefiltered, Aux::Clean] {
            for &filter in aux.filters() {
                let tensors = crate::tza::parse(filter.weights(aux)).unwrap();
                let net = crate::net::build(&tensors, filter.input_channels()).unwrap();
                for layer in &net.layers {
                    let shape = ConvShape {
                        width: 1920 >> layer.level,
                        height: 1088 >> layer.level,
                    };
                    let source = conv(layer, &shape);
                    if !sources.contains(&source) {
                        sources.push(source);
                    }
                }
            }
        }
        for source in sources {
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }
}
