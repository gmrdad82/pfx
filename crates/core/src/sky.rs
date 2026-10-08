use crate::daylight::Daylight;
use std::f32::consts::PI;

pub const SUN_RADIUS: f32 = 0.004_675;

pub fn adapt_exposure(previous: f32, luminances: &[f32], frame_seconds: f32) -> f32 {
    let previous = if previous.is_finite() && previous > 0.0 {
        previous
    } else {
        1.0
    };
    let mut log_sum = 0.0_f64;
    let mut count = 0;
    for &luminance in luminances {
        if luminance.is_finite() && luminance >= 0.0 {
            log_sum += luminance.max(0.001).ln() as f64;
            count += 1;
        }
    }
    if count == 0 || !frame_seconds.is_finite() || frame_seconds <= 0.0 {
        return previous;
    }
    let average = (log_sum / count as f64).exp() as f32;
    let target = (0.18 / average).clamp(0.25, 8.0);
    let blend = 1.0 - (-frame_seconds / 0.5).exp();
    previous + (target - previous) * blend
}

#[derive(Clone, Copy, Debug)]
pub struct AnalyticSky {
    pub sun: [f32; 3],
    pub sun_colour: [f32; 3],
    pub sun_intensity: f32,
    pub ambient: f32,
    pub turbidity: f32,
    pub ground_albedo: [f32; 3],
}

impl AnalyticSky {
    pub fn new(
        daylight: Daylight,
        reference_hour: f64,
        turbidity: f32,
        ground_albedo: [f32; 3],
    ) -> Self {
        let sun = daylight.sun();
        let light = daylight.light(reference_hour);
        Self {
            sun: sun.y_up.map(|v| v as f32),
            sun_colour: light.colour.map(|v| v as f32),
            sun_intensity: light.intensity as f32,
            ambient: light.ambient as f32,
            turbidity: turbidity.clamp(1.0, 10.0),
            ground_albedo: ground_albedo.map(|v| v.clamp(0.0, 1.0)),
        }
    }

    pub fn diffuse(&self, direction: [f32; 3]) -> [f32; 3] {
        let direction = unit(direction);
        if direction[1] < 0.0 {
            return self.ground_albedo.map(|v| v * self.ambient * 0.12);
        }
        let cos_theta = direction[1].max(0.001);
        let cos_gamma = dot(direction, self.sun).clamp(-1.0, 1.0);
        let gamma = cos_gamma.acos();
        let t = self.turbidity;
        let a = 0.1787 * t - 1.4630;
        let b = -0.3554 * t + 0.4275;
        let c = -0.0227 * t + 5.3251;
        let d = 0.1206 * t - 2.5771;
        let e = -0.0670 * t + 0.3703;
        let shape = (1.0 + a * (b / cos_theta).exp())
            * (1.0 + c * (d * gamma).exp() + e * cos_gamma * cos_gamma);
        let zenith_gamma = self.sun[1].clamp(-1.0, 1.0).acos();
        let zenith_cos = zenith_gamma.cos();
        let norm = (1.0 + a * b.exp())
            * (1.0 + c * (d * zenith_gamma).exp() + e * zenith_cos * zenith_cos);
        let haze = (t - 1.0) / 9.0;
        let warm = (1.0 - self.sun[1].max(0.0)).powi(2) * (-gamma * 2.0).exp();
        let base = [0.34 + 0.24 * haze, 0.50 + 0.18 * haze, 1.05 - 0.30 * haze];
        let tint = [1.0 + 0.75 * warm, 1.0 + 0.15 * warm, 1.0 - 0.45 * warm];
        let rise = self.sun[1].max(0.0);
        let twilight = ((rise - 0.2) / 0.4).clamp(0.0, 1.0);
        let twilight = twilight * twilight * (3.0 - 2.0 * twilight);
        let scale = 0.215 * rise.powf(2.5) + 0.05 * (1.0 - twilight);
        let strength = (shape / norm.max(0.01)).max(0.0) * self.ambient * scale;
        [0, 1, 2].map(|i| base[i] * tint[i] * strength)
    }

    pub fn radiance(&self, direction: [f32; 3]) -> [f32; 3] {
        let direction = unit(direction);
        let mut value = self.diffuse(direction);
        let cosine = dot(direction, self.sun).clamp(-1.0, 1.0);
        let edge = SUN_RADIUS.cos();
        if self.sun[1] > 0.0 && cosine >= edge {
            let radius = (2.0 * (1.0 - cosine)).sqrt() / SUN_RADIUS;
            let limb = 0.4 + 0.6 * (1.0 - radius * radius).max(0.0).sqrt();
            let radiance = self.sun_intensity * limb / (PI * SUN_RADIUS * SUN_RADIUS * 0.8);
            for (channel, solar) in value.iter_mut().zip(self.sun_colour) {
                *channel += solar * radiance;
            }
        }
        value
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(a: [f32; 3]) -> [f32; 3] {
    let scale = 1.0 / dot(a, a).sqrt().max(1e-8);
    a.map(|v| v * scale)
}

pub const SKY_MODEL_WGSL: &str = r#"
struct SkyUniform {
    sun: vec4f,
    sun_colour_intensity: vec4f,
    ambient_turbidity_albedo: vec4f,
    albedo_mode: vec4f,
    forward: vec4f,
    right: vec4f,
    up: vec4f,
};
@group(1) @binding(0) var<uniform> sky: SkyUniform;
@group(1) @binding(1) var hdr: texture_2d<f32>;
@group(1) @binding(2) var hdr_sampler: sampler;
fn sky_diffuse(direction: vec3f) -> vec3f {
    let d = normalize(direction);
    if d.y < 0.0 {
        return vec3f(sky.ambient_turbidity_albedo.zw, sky.albedo_mode.x) * sky.ambient_turbidity_albedo.x * 0.12;
    }
    let cos_theta = max(d.y, 0.001);
    let cos_gamma = clamp(dot(d, sky.sun.xyz), -1.0, 1.0);
    let gamma = acos(cos_gamma);
    let t = sky.ambient_turbidity_albedo.y;
    let a = 0.1787*t-1.4630;
    let b = -0.3554*t+0.4275;
    let c = -0.0227*t+5.3251;
    let dd = 0.1206*t-2.5771;
    let e = -0.0670*t+0.3703;
    let shape = (1.0+a*exp(b/cos_theta))*(1.0+c*exp(dd*gamma)+e*cos_gamma*cos_gamma);
    let zenith_gamma = acos(clamp(sky.sun.y, -1.0, 1.0));
    let zenith_cos = cos(zenith_gamma);
    let norm = (1.0+a*exp(b))*(1.0+c*exp(dd*zenith_gamma)+e*zenith_cos*zenith_cos);
    let haze = (t-1.0)/9.0;
    let warm = pow(1.0-max(sky.sun.y, 0.0), 2.0)*exp(-gamma*2.0);
    let base = vec3f(0.34+0.24*haze, 0.50+0.18*haze, 1.05-0.30*haze);
    let tint = vec3f(1.0+0.75*warm, 1.0+0.15*warm, 1.0-0.45*warm);
    let rise = max(sky.sun.y, 0.0);
    let scale = 0.215*pow(rise, 2.5)+0.05*(1.0-smoothstep(0.2, 0.6, rise));
    return base*tint*max(shape/max(norm, 0.01), 0.0)*sky.ambient_turbidity_albedo.x*scale;
}
fn sky_analytic(direction: vec3f) -> vec3f {
    let d = normalize(direction);
    var value = sky_diffuse(d);
    let cosine = clamp(dot(d, sky.sun.xyz), -1.0, 1.0);
    let radius = 0.004675;
    if sky.sun.y > 0.0 && cosine >= cos(radius) {
        let radial = sqrt(2.0*(1.0-cosine))/radius;
        let limb = 0.4+0.6*sqrt(max(1.0-radial*radial, 0.0));
        value += sky.sun_colour_intensity.xyz*(sky.sun_colour_intensity.w*limb/(3.14159265*radius*radius*0.8));
    }
    return value;
}
fn sky_radiance(direction: vec3f) -> vec3f {
    let d = normalize(direction);
    if sky.albedo_mode.y < 0.5 {
        return sky_analytic(d);
    }
    let u = fract(atan2(d.x, -d.z)/6.28318531+0.5);
    let v = acos(clamp(d.y, -1.0, 1.0))/3.14159265;
    return textureSampleLevel(hdr, hdr_sampler, vec2f(u, v), 0.0).rgb;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daylight::REFERENCE_HOUR;

    #[test]
    fn exposure_uses_the_frame_log_average_and_caller_time() {
        let first = adapt_exposure(1.0, &[0.04, 0.16], 0.5);
        let target = 0.18 / 0.08;
        let expected = 1.0 + (target - 1.0) * (1.0 - (-1.0_f32).exp());
        assert!((first - expected).abs() < 1e-6);
        assert_eq!(adapt_exposure(first, &[0.04, 0.16], 0.0), first);
        assert_eq!(adapt_exposure(first, &[f32::NAN], 1.0), first);
        assert_eq!(adapt_exposure(1.0, &[0.04, 0.16], 0.5), first);
    }

    #[test]
    fn model_keeps_the_solar_disc_and_calibrates_diffuse() {
        let expected = [
            ([0.0237, 0.0326, 0.0607], [0.1190, 0.1560, 0.2770]),
            ([0.0684, 0.0952, 0.1792], [0.0506, 0.0706, 0.1330]),
            ([0.0237, 0.0326, 0.0607], [0.0262, 0.0366, 0.0690]),
        ];
        for (hour, (zenith, side)) in [8.0, 12.0, 16.0].into_iter().zip(expected) {
            let sky = AnalyticSky::new(
                Daylight {
                    hour,
                    day: 172.0,
                    latitude: 45.0,
                    heading: 180.0,
                },
                REFERENCE_HOUR,
                2.5,
                [0.2, 0.25, 0.2],
            );
            for (actual, reference) in [
                (sky.diffuse([0.0, 1.0, 0.0]), zenith),
                (sky.diffuse([1.0, 0.5, 0.0]), side),
            ] {
                for (value, expected) in actual.into_iter().zip(reference) {
                    assert!(
                        (value - expected).abs() <= 0.001,
                        "{hour}: {value} != {expected}"
                    );
                }
            }
            let direct = sky.radiance(sky.sun);
            let diffuse = sky.diffuse(sky.sun);
            for channel in 0..3 {
                let expected = sky.sun_colour[channel] * sky.sun_intensity
                    / (PI * SUN_RADIUS * SUN_RADIUS * 0.8);
                assert!((direct[channel] - diffuse[channel] - expected).abs() < 0.01);
            }
        }
    }

    #[test]
    fn clear_afternoon_has_a_sun_dominated_horizontal_irradiance() {
        fn luminance(rgb: [f32; 3]) -> f64 {
            rgb.into_iter()
                .zip([0.2126, 0.7152, 0.0722])
                .map(|(channel, weight)| channel as f64 * weight)
                .sum()
        }

        fn sky_share(hour: f64) -> f64 {
            let sky = AnalyticSky::new(
                Daylight {
                    hour,
                    day: 172.0,
                    latitude: 45.0,
                    heading: 180.0,
                },
                16.0,
                2.5,
                [0.2, 0.25, 0.2],
            );
            let mut diffuse = 0.0;
            for row in 0..96 {
                let theta = (row as f64 + 0.5) * std::f64::consts::FRAC_PI_2 / 96.0;
                for column in 0..192 {
                    let phi = (column as f64 + 0.5) * std::f64::consts::TAU / 192.0;
                    let direction = [
                        (theta.sin() * phi.cos()) as f32,
                        theta.cos() as f32,
                        (theta.sin() * phi.sin()) as f32,
                    ];
                    let solid_angle = theta.sin() * std::f64::consts::FRAC_PI_2 / 96.0
                        * std::f64::consts::TAU
                        / 192.0;
                    diffuse += luminance(sky.diffuse(direction)) * theta.cos() * solid_angle;
                }
            }
            let direct =
                luminance(sky.sun_colour) * sky.sun_intensity as f64 * sky.sun[1].max(0.0) as f64;
            diffuse / (diffuse + direct)
        }

        for hour in [10.0, 12.0, 14.0, 16.0] {
            let share = sky_share(hour);
            assert!((0.15..=0.25).contains(&share), "{hour}: {share}");
        }
        assert!(sky_share(6.0) > sky_share(8.0));
        assert!(sky_share(19.0) > sky_share(16.0));
    }
}
