use bytemuck::{Pod, Zeroable};
#[cfg(test)]
use pfx_core::daylight::Daylight;
use pfx_core::daylight::{self, Probe};
pub use pfx_core::sky::AnalyticSky;
use pfx_core::sky::{SKY_MODEL_WGSL, SUN_RADIUS};
use pfx_load::Sky as HdrSky;
use std::f32::consts::{PI, TAU};
use std::sync::LazyLock;

#[derive(Clone, Debug)]
pub enum Environment {
    Analytic(AnalyticSky),
    Hdr(HdrSky),
}

impl From<HdrSky> for Environment {
    fn from(sky: HdrSky) -> Self {
        Self::Hdr(sky)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct EnvironmentUniform {
    pub sun: [f32; 4],
    pub sun_colour_intensity: [f32; 4],
    pub ambient_turbidity_albedo: [f32; 4],
    pub albedo_mode: [f32; 4],
    pub forward: [f32; 4],
    pub right: [f32; 4],
    pub up: [f32; 4],
}

impl Environment {
    pub fn uniform(&self) -> EnvironmentUniform {
        let (model, mode) = match self {
            Self::Analytic(model) => (*model, 0.0),
            Self::Hdr(_) => (
                AnalyticSky {
                    sun: [0.0, 1.0, 0.0],
                    sun_colour: [1.0; 3],
                    sun_intensity: 0.0,
                    ambient: 0.0,
                    turbidity: 2.0,
                    ground_albedo: [0.0; 3],
                },
                1.0,
            ),
        };
        EnvironmentUniform {
            sun: [model.sun[0], model.sun[1], model.sun[2], 0.0],
            sun_colour_intensity: [
                model.sun_colour[0],
                model.sun_colour[1],
                model.sun_colour[2],
                model.sun_intensity,
            ],
            ambient_turbidity_albedo: [
                model.ambient,
                model.turbidity,
                model.ground_albedo[0],
                model.ground_albedo[1],
            ],
            albedo_mode: [model.ground_albedo[2], mode, 0.0, 0.0],
            forward: [0.0; 4],
            right: [0.0; 4],
            up: [0.0; 4],
        }
    }

    pub fn from_probes(hour: f64, skies: &[HdrSky], probes: &[Probe]) -> Result<Self, String> {
        let first = skies.first().ok_or("sky probes need at least one HDR")?;
        if skies.iter().any(|s| {
            s.width != first.width
                || s.height != first.height
                || s.texels.len() != first.texels.len()
        }) {
            return Err("sky probes need matching HDR dimensions".into());
        }
        let weights = daylight::probe_weights(hour, probes, skies.len());
        if weights.iter().sum::<f64>() <= 0.0 {
            return Err("sky probes have no valid index".into());
        }
        let mut texels = vec![[0.0; 4]; first.texels.len()];
        for (sky, weight) in skies.iter().zip(weights) {
            for (out, input) in texels.iter_mut().zip(&sky.texels) {
                for (out_channel, input_channel) in out[..3].iter_mut().zip(&input[..3]) {
                    *out_channel += *input_channel * weight as f32;
                }
            }
        }
        for pixel in &mut texels {
            pixel[3] = 1.0;
        }
        Ok(Self::Hdr(HdrSky {
            width: first.width,
            height: first.height,
            texels,
        }))
    }

    pub fn radiance(&self, direction: [f32; 3]) -> [f32; 3] {
        match self {
            Self::Analytic(model) => model.radiance(direction),
            Self::Hdr(hdr) => {
                let d = unit(direction);
                let u = (d[0].atan2(-d[2]) / TAU + 0.5).rem_euclid(1.0);
                let v = d[1].clamp(-1.0, 1.0).acos() / PI;
                let x = u * hdr.width as f32 - 0.5;
                let y = v * hdr.height as f32 - 0.5;
                let x0 = x.floor();
                let y0 = y.floor();
                let tx = x - x0;
                let ty = y - y0;
                let at = |ix: f32, iy: f32| {
                    let col = (ix as i32).rem_euclid(hdr.width as i32) as usize;
                    let row = (iy as i32).clamp(0, hdr.height as i32 - 1) as usize;
                    hdr.texels[row * hdr.width as usize + col]
                };
                let a = at(x0, y0);
                let b = at(x0 + 1.0, y0);
                let c = at(x0, y0 + 1.0);
                let d = at(x0 + 1.0, y0 + 1.0);
                [0, 1, 2].map(|i| {
                    (a[i] * (1.0 - tx) + b[i] * tx) * (1.0 - ty)
                        + (c[i] * (1.0 - tx) + d[i] * tx) * ty
                })
            }
        }
    }

    fn diffuse(&self, direction: [f32; 3]) -> [f32; 3] {
        match self {
            Self::Analytic(model) => model.diffuse(direction),
            Self::Hdr(_) => self.radiance(direction),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SkyCdf {
    pub width: u32,
    pub height: u32,
    pub rows: Vec<f32>,
    pub columns: Vec<f32>,
    pub sun_probability: f32,
    pub total: f32,
    sun: Option<[f32; 3]>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SkyCdfUniform {
    pub width: u32,
    pub height: u32,
    pub sun_probability: f32,
    pub unused: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SkySample {
    pub direction: [f32; 3],
    pub radiance: [f32; 3],
    pub pdf: f32,
}

impl SkyCdf {
    pub fn without_sun(mut self) -> Self {
        self.sun_probability = 0.0;
        self.sun = None;
        self
    }

    pub fn uniform(&self) -> SkyCdfUniform {
        SkyCdfUniform {
            width: self.width,
            height: self.height,
            sun_probability: self.sun_probability,
            unused: 0.0,
        }
    }

    pub fn build(environment: &Environment, width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let mut rows = vec![0.0; height as usize];
        let mut columns = vec![0.0; (width * height) as usize];
        let mut total = 0.0_f64;
        for row in 0..height {
            let theta = PI * (row as f32 + 0.5) / height as f32;
            let sine = theta.sin();
            let mut row_total = 0.0_f64;
            for col in 0..width {
                let phi = TAU * (col as f32 + 0.5) / width as f32;
                let direction = [
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ];
                let color = environment.diffuse(direction);
                row_total += luminance(color).max(0.0) as f64 * sine as f64;
                columns[(row * width + col) as usize] = row_total as f32;
            }
            if row_total > 0.0 {
                for col in 0..width {
                    columns[(row * width + col) as usize] /= row_total as f32;
                }
            } else {
                for col in 0..width {
                    columns[(row * width + col) as usize] = (col + 1) as f32 / width as f32;
                }
            }
            total += row_total;
            rows[row as usize] = total as f32;
        }
        if total > 0.0 {
            for value in &mut rows {
                *value /= total as f32;
            }
        } else {
            for (index, value) in rows.iter_mut().enumerate() {
                *value = (index + 1) as f32 / height as f32;
            }
        }
        let (sun_probability, sun) = match environment {
            Environment::Analytic(model) if model.sun[1] > 0.0 && model.sun_intensity > 0.0 => {
                let sky_energy = total as f32 * (TAU / width as f32) * (PI / height as f32);
                let sun_energy = luminance(model.sun_colour) * model.sun_intensity;
                (
                    sun_energy / (sun_energy + sky_energy).max(1e-6),
                    Some(model.sun),
                )
            }
            _ => (0.0, None),
        };
        Self {
            width,
            height,
            rows,
            columns,
            sun_probability,
            total: total as f32,
            sun,
        }
    }

    pub fn sample(&self, environment: &Environment, u: f32, v: f32) -> SkySample {
        let u = u.clamp(0.0, 1.0 - f32::EPSILON);
        let v = v.clamp(0.0, 1.0 - f32::EPSILON);
        let mut cell_pdf = None;
        let direction = if let Some(sun) = self.sun.filter(|_| u < self.sun_probability) {
            let cosine = 1.0 - (u / self.sun_probability.max(1e-6)) * (1.0 - SUN_RADIUS.cos());
            let radial = cosine.acos();
            let angle = TAU * v;
            let up = if sun[1].abs() < 0.99 {
                [0.0, 1.0, 0.0]
            } else {
                [1.0, 0.0, 0.0]
            };
            let right = unit(cross(up, sun));
            let up = cross(sun, right);
            unit(add(
                add(
                    mul(sun, radial.cos()),
                    mul(right, radial.sin() * angle.cos()),
                ),
                mul(up, radial.sin() * angle.sin()),
            ))
        } else {
            let row = self
                .rows
                .partition_point(|&value| value <= v)
                .min(self.height as usize - 1);
            let row_start = if row == 0 { 0.0 } else { self.rows[row - 1] };
            let row_end = self.rows[row];
            let fy = (v - row_start) / (row_end - row_start).max(1e-6);
            let target =
                (u - self.sun_probability).max(0.0) / (1.0 - self.sun_probability).max(1e-6);
            let start = row * self.width as usize;
            let cdf = &self.columns[start..start + self.width as usize];
            let col = cdf
                .partition_point(|&value| value <= target)
                .min(self.width as usize - 1);
            let col_start = if col == 0 { 0.0 } else { cdf[col - 1] };
            let col_end = cdf[col];
            let fx = (target - col_start) / (col_end - col_start).max(1e-6);
            let theta = PI * (row as f32 + fy) / self.height as f32;
            let phi = TAU * (col as f32 + fx) / self.width as f32;
            let solid_angle =
                TAU / self.width as f32 * PI / self.height as f32 * theta.sin().max(1e-5);
            let sun_pdf = match self.sun {
                Some(sun) if dot(sun, unit_direction(theta, phi)) >= SUN_RADIUS.cos() => {
                    self.sun_probability / (2.0 * PI * (1.0 - SUN_RADIUS.cos()))
                }
                _ => 0.0,
            };
            cell_pdf = Some(
                (1.0 - self.sun_probability) * (row_end - row_start) * (col_end - col_start)
                    / solid_angle
                    + sun_pdf,
            );
            [
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            ]
        };
        SkySample {
            direction,
            radiance: environment.radiance(direction),
            pdf: cell_pdf.unwrap_or_else(|| self.pdf(direction)),
        }
    }

    pub fn pdf(&self, direction: [f32; 3]) -> f32 {
        let d = unit(direction);
        let theta = d[1].clamp(-1.0, 1.0).acos();
        let phi = d[2].atan2(d[0]).rem_euclid(TAU);
        let row = ((theta / PI * self.height as f32) as u32).min(self.height - 1) as usize;
        let col = ((phi / TAU * self.width as f32) as u32).min(self.width - 1) as usize;
        let start = row * self.width as usize;
        let row_mass = self.rows[row] - if row == 0 { 0.0 } else { self.rows[row - 1] };
        let col_mass = self.columns[start + col]
            - if col == 0 {
                0.0
            } else {
                self.columns[start + col - 1]
            };
        let solid_angle = TAU / self.width as f32 * PI / self.height as f32 * theta.sin().max(1e-5);
        let sky_pdf = (1.0 - self.sun_probability) * row_mass * col_mass / solid_angle;
        let sun_pdf = if self.sun.is_some_and(|sun| dot(sun, d) >= SUN_RADIUS.cos()) {
            self.sun_probability / (2.0 * PI * (1.0 - SUN_RADIUS.cos()))
        } else {
            0.0
        };
        sky_pdf + sun_pdf
    }
}

fn unit_direction(theta: f32, phi: f32) -> [f32; 3] {
    [
        theta.sin() * phi.cos(),
        theta.cos(),
        theta.sin() * phi.sin(),
    ]
}
fn luminance(v: [f32; 3]) -> f32 {
    0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: [f32; 3], s: f32) -> [f32; 3] {
    a.map(|v| v * s)
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: [f32; 3]) -> [f32; 3] {
    mul(a, 1.0 / dot(a, a).sqrt().max(1e-8))
}

pub static ENVIRONMENT_WGSL: LazyLock<String> =
    LazyLock::new(|| format!("{}\n{}", SKY_MODEL_WGSL, SKY_SAMPLING_WGSL));

const SKY_SAMPLING_WGSL: &str = r#"
fn sky_surface_radiance(direction: vec3f) -> vec3f {
    if sky.albedo_mode.y < 0.5 { return sky_diffuse(direction); }
    return sky_radiance(direction);
}
struct SkyCdfInfo {
    width: u32,
    height: u32,
    sun_probability: f32,
    unused: f32,
};
@group(1) @binding(3) var<storage, read> sky_rows: array<f32>;
@group(1) @binding(4) var<storage, read> sky_columns: array<f32>;
@group(1) @binding(5) var<uniform> sky_cdf: SkyCdfInfo;
struct SkyDirectionSample {
    direction: vec3f,
    radiance: vec3f,
    pdf: f32,
};
fn sky_find_row(needle: f32) -> u32 {
    var low = 0u;
    var high = sky_cdf.height;
    loop {
        if low >= high { break; }
        let mid = (low + high) / 2u;
        if sky_rows[mid] <= needle { low = mid + 1u; } else { high = mid; }
    }
    return min(low, sky_cdf.height - 1u);
}
fn sky_find_column(row: u32, needle: f32) -> u32 {
    var low = 0u;
    var high = sky_cdf.width;
    loop {
        if low >= high { break; }
        let mid = (low + high) / 2u;
        if sky_columns[row * sky_cdf.width + mid] <= needle { low = mid + 1u; } else { high = mid; }
    }
    return min(low, sky_cdf.width - 1u);
}
fn sky_pdf(direction: vec3f) -> f32 {
    let d = normalize(direction);
    let theta = acos(clamp(d.y, -1.0, 1.0));
    let phi = (atan2(d.z, d.x) + 6.28318531) % 6.28318531;
    let row = min(u32(theta / 3.14159265 * f32(sky_cdf.height)), sky_cdf.height - 1u);
    let col = min(u32(phi / 6.28318531 * f32(sky_cdf.width)), sky_cdf.width - 1u);
    let row_before = select(0.0, sky_rows[max(row, 1u) - 1u], row > 0u);
    let col_before = select(0.0, sky_columns[row * sky_cdf.width + max(col, 1u) - 1u], col > 0u);
    let mass = (sky_rows[row] - row_before) * (sky_columns[row * sky_cdf.width + col] - col_before);
    let area = 6.28318531 / f32(sky_cdf.width) * 3.14159265 / f32(sky_cdf.height) * max(sin(theta), 0.00001);
    let sun_pdf = select(0.0, sky_cdf.sun_probability / (6.28318531 * (1.0 - cos(0.004675))), sky.sun.y > 0.0 && dot(d, sky.sun.xyz) >= cos(0.004675));
    return (1.0 - sky_cdf.sun_probability) * mass / area + sun_pdf;
}
fn sky_sample(random: vec2f) -> SkyDirectionSample {
    let u = clamp(random.x, 0.0, 0.99999994);
    let v = clamp(random.y, 0.0, 0.99999994);
    var direction: vec3f;
    if sky_cdf.sun_probability > 0.0 && u < sky_cdf.sun_probability {
        let cosine = 1.0 - u / sky_cdf.sun_probability * (1.0 - cos(0.004675));
        let radial = acos(cosine);
        let angle = 6.28318531 * v;
        let axis = select(vec3f(1.0, 0.0, 0.0), vec3f(0.0, 1.0, 0.0), abs(sky.sun.y) < 0.99);
        let right = normalize(cross(axis, sky.sun.xyz));
        let up = cross(sky.sun.xyz, right);
        direction = normalize(sky.sun.xyz * cos(radial) + (right * cos(angle) + up * sin(angle)) * sin(radial));
    } else {
        let row = sky_find_row(v);
        let row_before = select(0.0, sky_rows[max(row, 1u) - 1u], row > 0u);
        let fy = (v - row_before) / max(sky_rows[row] - row_before, 0.000001);
        let needle = max(u - sky_cdf.sun_probability, 0.0) / max(1.0 - sky_cdf.sun_probability, 0.000001);
        let col = sky_find_column(row, needle);
        let col_before = select(0.0, sky_columns[row * sky_cdf.width + max(col, 1u) - 1u], col > 0u);
        let fx = (needle - col_before) / max(sky_columns[row * sky_cdf.width + col] - col_before, 0.000001);
        let theta = 3.14159265 * (f32(row) + fy) / f32(sky_cdf.height);
        let phi = 6.28318531 * (f32(col) + fx) / f32(sky_cdf.width);
        direction = vec3f(sin(theta) * cos(phi), cos(theta), sin(theta) * sin(phi));
        let area = 6.28318531 / f32(sky_cdf.width) * 3.14159265 / f32(sky_cdf.height) * max(sin(theta), 0.00001);
        let mass = (sky_rows[row] - row_before) * (sky_columns[row * sky_cdf.width + col] - col_before);
        let sun_pdf = select(0.0, sky_cdf.sun_probability / (6.28318531 * (1.0 - cos(0.004675))), sky.sun.y > 0.0 && dot(direction, sky.sun.xyz) >= cos(0.004675));
        return SkyDirectionSample(direction, sky_radiance(direction), (1.0 - sky_cdf.sun_probability) * mass / area + sun_pdf);
    }
    return SkyDirectionSample(direction, sky_radiance(direction), sky_pdf(direction));
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn model(hour: f64) -> AnalyticSky {
        AnalyticSky::new(
            Daylight {
                hour,
                day: 172.0,
                latitude: 45.0,
                heading: 180.0,
            },
            daylight::REFERENCE_HOUR,
            2.5,
            [0.2, 0.25, 0.2],
        )
    }

    #[test]
    fn cdf_sums_to_one_and_samples_sun() {
        let environment = Environment::Analytic(model(12.0));
        let cdf = SkyCdf::build(&environment, 64, 32);
        let mut mass = 0.0;
        for row in 0..cdf.height as usize {
            let row_mass = cdf.rows[row] - if row == 0 { 0.0 } else { cdf.rows[row - 1] };
            for col in 0..cdf.width as usize {
                let start = row * cdf.width as usize;
                let col_mass = cdf.columns[start + col]
                    - if col == 0 {
                        0.0
                    } else {
                        cdf.columns[start + col - 1]
                    };
                assert!(col_mass >= -1e-5 && row_mass >= -1e-5);
                mass += row_mass * col_mass;
            }
        }
        assert!((mass - 1.0).abs() < 1e-4);
        assert!((cdf.rows.last().unwrap() - 1.0).abs() < 1e-6);
        assert!(cdf.sun_probability > 0.0);
        let solar = cdf.sample(&environment, cdf.sun_probability * 0.5, 0.25);
        assert!(dot(solar.direction, model(12.0).sun) > SUN_RADIUS.cos());
        assert!(solar.pdf.is_finite() && solar.pdf > 0.0);
    }

    #[test]
    fn night_is_darker_and_hdr_probe_blends() {
        let noon = model(12.0);
        let night = model(0.0);
        assert!(noon.radiance([0.0, 1.0, 0.0])[2] > night.radiance([0.0, 1.0, 0.0])[2] * 5.0);
        let skies = [
            HdrSky {
                width: 1,
                height: 1,
                texels: vec![[2.0, 0.0, 0.0, 1.0]],
            },
            HdrSky {
                width: 1,
                height: 1,
                texels: vec![[0.0, 0.0, 4.0, 1.0]],
            },
        ];
        let probes = [
            Probe {
                hour: 6.0,
                index: 0,
            },
            Probe {
                hour: 18.0,
                index: 1,
            },
        ];
        let environment = Environment::from_probes(12.0, &skies, &probes).unwrap();
        assert_eq!(environment.radiance([1.0, 0.0, 0.0]), [1.0, 0.0, 2.0]);
        let cdf = SkyCdf::build(&environment, 16, 8);
        assert_eq!(cdf.sun_probability, 0.0);
        assert!((cdf.rows.last().unwrap() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn wgsl_parses() {
        let module = naga::front::wgsl::parse_str(&ENVIRONMENT_WGSL).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn sky_sampling_converges_to_analytic_integral() {
        let model = model(16.0);
        let environment = Environment::Analytic(model);
        let full_cdf = SkyCdf::build(&environment, 256, 128);
        let cdf = full_cdf.clone().without_sun();
        let mut reference = [0.0_f64; 3];
        for row in 0..256 {
            let theta = PI * (row as f32 + 0.5) / 256.0;
            for col in 0..512 {
                let phi = TAU * (col as f32 + 0.5) / 512.0;
                let direction = [
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ];
                let value = model.diffuse(direction);
                for channel in 0..3 {
                    reference[channel] += (value[channel] * theta.sin()) as f64;
                }
            }
        }
        let area = (PI * TAU / (256 * 512) as f32) as f64;
        let reference = reference.map(|v| v * area);
        let mut estimate = [0.0_f64; 3];
        let count = 65536_u32;
        for i in 0..count {
            let u = (i as f32 + 0.5) / count as f32;
            let v = (i.reverse_bits() as f32 + 0.5) * 2.328_306_4e-10;
            let sample = cdf.sample(&environment, u, v);
            let value = model.diffuse(sample.direction);
            for channel in 0..3 {
                estimate[channel] += (value[channel] / sample.pdf) as f64;
            }
        }
        for channel in 0..3 {
            let measured = estimate[channel] / count as f64;
            assert!(
                (measured / reference[channel] - 1.0).abs() < 0.02,
                "{measured} vs {}",
                reference[channel]
            );
        }
        let mut full_estimate = [0.0_f64; 3];
        for i in 0..count {
            let u = (i as f32 + 0.5) / count as f32;
            let v = (i.reverse_bits() as f32 + 0.5) * 2.328_306_4e-10;
            let sample = full_cdf.sample(&environment, u, v);
            for (sum, radiance) in full_estimate.iter_mut().zip(sample.radiance) {
                *sum += (radiance / sample.pdf) as f64;
            }
        }
        for channel in 0..3 {
            let measured = full_estimate[channel] / count as f64;
            let expected =
                reference[channel] + (model.sun_intensity * model.sun_colour[channel]) as f64;
            assert!(
                (measured / expected - 1.0).abs() < 0.02,
                "{measured} vs {expected}"
            );
        }
    }
}

#[cfg(test)]
mod density_tests {
    use super::*;

    fn speckled(width: u32, height: u32) -> HdrSky {
        let mut state = 0x2545f491_u32;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as f32 / u32::MAX as f32
        };
        let texels = (0..width * height)
            .map(|_| {
                let value = if next() < 0.11 { 0.0 } else { 20.0 * next() };
                [value, value * 0.9, value * 0.8, 1.0]
            })
            .collect();
        HdrSky {
            width,
            height,
            texels,
        }
    }

    #[test]
    fn the_pdf_is_the_density_of_what_the_sampler_draws() {
        let (width, height) = (128_usize, 64_usize);
        let environment = Environment::Hdr(speckled(width as u32, height as u32));
        let cdf = SkyCdf::build(&environment, width as u32, height as u32);
        let mut state = 0x1234567_u32;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 8) as f32 / (1u32 << 24) as f32
        };
        let draws = 2_000_000;
        let mut counts = vec![0u32; width * height];
        let mut no_pdf = 0;
        let mut wrong_cell = 0;
        for _ in 0..draws {
            let sample = cdf.sample(&environment, next(), next());
            let d = sample.direction;
            let theta = d[1].clamp(-1.0, 1.0).acos();
            let phi = d[2].atan2(d[0]).rem_euclid(TAU);
            let row = ((theta / PI * height as f32) as usize).min(height - 1);
            let col = ((phi / TAU * width as f32) as usize).min(width - 1);
            counts[row * width + col] += 1;
            if sample.pdf <= 0.0 {
                no_pdf += 1;
                continue;
            }
            let area = (TAU / width as f32 * PI / height as f32 * theta.sin()) as f64;
            let row_mass = cdf.rows[row] - if row == 0 { 0.0 } else { cdf.rows[row - 1] };
            let at = row * width + col;
            let col_mass = cdf.columns[at] - if col == 0 { 0.0 } else { cdf.columns[at - 1] };
            let density = row_mass as f64 * col_mass as f64 / area;
            if (cdf.pdf(d) as f64 / density - 1.0).abs() > 1e-3 {
                wrong_cell += 1;
            }
            assert!(sample.pdf as f64 > 0.0);
        }
        println!(
            "no pdf {no_pdf}, direction lookup in the neighbouring cell {wrong_cell}, of {draws}"
        );
        assert_eq!(no_pdf, 0);
        assert!(wrong_cell < draws / 1_000);
        for row in 0..height {
            let row_mass = cdf.rows[row] - if row == 0 { 0.0 } else { cdf.rows[row - 1] };
            for col in 0..width {
                let at = row * width + col;
                let col_mass = cdf.columns[at] - if col == 0 { 0.0 } else { cdf.columns[at - 1] };
                let expected = row_mass as f64 * col_mass as f64 * draws as f64;
                if expected < 200.0 {
                    continue;
                }
                let got = counts[at] as f64;
                assert!(
                    (got - expected).abs() < 5.0 * expected.sqrt(),
                    "cell {row},{col}: {got} drawn, {expected} expected"
                );
            }
        }
    }

    #[test]
    fn a_sun_adds_its_density_to_every_sample() {
        let daylight = Daylight {
            hour: 16.0,
            day: 172.0,
            latitude: 45.0,
            heading: 180.0,
        };
        let environment = Environment::Analytic(AnalyticSky::new(
            daylight,
            daylight::REFERENCE_HOUR,
            2.5,
            [0.2, 0.25, 0.2],
        ));
        let cdf = SkyCdf::build(&environment, 128, 64);
        assert!(cdf.sun_probability > 0.0);
        let mut state = 0x7654321_u32;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 8) as f32 / (1u32 << 24) as f32
        };
        let draws = 300_000;
        let mut apart = 0;
        let mut sun_draws = 0;
        for _ in 0..draws {
            let sample = cdf.sample(&environment, next(), next());
            assert!(sample.pdf > 0.0);
            let looked_up = cdf.pdf(sample.direction);
            if (sample.pdf / looked_up - 1.0).abs() > 1e-3 {
                apart += 1;
            }
            if dot(sample.direction, cdf.sun.unwrap()) >= SUN_RADIUS.cos() {
                sun_draws += 1;
            }
        }
        assert!(sun_draws > draws / 100);
        assert!(apart < draws / 1_000, "{apart}");
    }
}
