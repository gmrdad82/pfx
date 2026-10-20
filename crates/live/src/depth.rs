pub const DEPTH_WGSL: &str = "fn pfx_standard_depth(stored: f32) -> f32 {
    return 1.0 - stored;
}
fn pfx_is_sky(stored: f32) -> bool {
    return stored <= 0.0;
}
fn pfx_projection_distance(z_ndc: f32, z_scale: f32, z_offset: f32, orthographic: bool) -> f32 {
    return select(z_offset / (z_ndc + z_scale), (z_offset - z_ndc) / z_scale, orthographic);
}
fn pfx_linear_depth(stored: f32, near: f32, far: f32) -> f32 {
    return near * far / (near + stored * (far - near));
}
fn pfx_linear_depth_orthographic(stored: f32, near: f32, far: f32) -> f32 {
    return far - stored * (far - near);
}
";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthConvention {
    pub format: wgpu::TextureFormat,
    pub reversed: bool,
    pub clear: f32,
    pub near: f32,
    pub far: f32,
    pub sky: f32,
    pub write_compare: wgpu::CompareFunction,
    pub test_compare: wgpu::CompareFunction,
}

impl DepthConvention {
    pub const REVERSED: Self = Self {
        format: wgpu::TextureFormat::Depth32Float,
        reversed: true,
        clear: 0.0,
        near: 1.0,
        far: 0.0,
        sky: 0.0,
        write_compare: wgpu::CompareFunction::Greater,
        test_compare: wgpu::CompareFunction::GreaterEqual,
    };

    pub fn standard_depth(self, stored: f32) -> f32 {
        1.0 - stored
    }

    pub fn stored_depth(self, standard: f32) -> f32 {
        1.0 - standard
    }

    pub fn is_sky(self, stored: f32) -> bool {
        stored <= 0.0
    }

    pub fn projection_distance(self, stored: f32, projection: &crate::frame::Matrix) -> f32 {
        let z = self.standard_depth(stored);
        let (scale, offset) = (projection[2][2], projection[3][2]);
        if projection[2][3] == 0.0 {
            (offset - z) / scale
        } else {
            offset / (z + scale)
        }
    }

    pub fn linear_depth(self, stored: f32, near: f32, far: f32) -> f32 {
        near * far / (near + stored * (far - near))
    }

    pub fn linear_depth_orthographic(self, stored: f32, near: f32, far: f32) -> f32 {
        far - stored * (far - near)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Matrix, reversed, transform};

    const NEAR: f32 = 0.1;
    const FAR: f32 = 400.0;

    fn perspective() -> Matrix {
        let tan = 0.5f32.tan();
        [
            [1.0 / (tan * 1.5), 0.0, 0.0, 0.0],
            [0.0, 1.0 / tan, 0.0, 0.0],
            [0.0, 0.0, FAR / (NEAR - FAR), -1.0],
            [0.0, 0.0, FAR * NEAR / (NEAR - FAR), 0.0],
        ]
    }

    fn orthographic() -> Matrix {
        [
            [0.2, 0.0, 0.0, 0.0],
            [0.0, 0.3, 0.0, 0.0],
            [0.0, 0.0, 1.0 / (NEAR - FAR), 0.0],
            [0.0, 0.0, NEAR / (NEAR - FAR), 1.0],
        ]
    }

    fn stored(projection: Matrix, distance: f32) -> (f32, f32) {
        let point = [0.3, -0.2, -distance, 1.0];
        let standard = transform(projection, point);
        let flipped = transform(reversed(projection), point);
        (1.0 - standard[2] / standard[3], flipped[2] / flipped[3])
    }

    fn validate(source: &str) {
        let module = naga::front::wgsl::parse_str(source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn the_renderers_linear_depth_pass_reads_through_the_helpers() {
        let source = crate::renderer::linear_depth_wgsl();
        assert!(source.starts_with(DEPTH_WGSL));
        validate(&source);
    }

    #[test]
    fn helpers_validate_in_an_app_shader() {
        let source = format!(
            "{DEPTH_WGSL}@group(0) @binding(0) var depth: texture_depth_2d;
@group(0) @binding(1) var<uniform> camera: vec4f;
@fragment fn main(@builtin(position) at: vec4f) -> @location(0) vec4f {{
    let d = textureLoad(depth, vec2i(at.xy), 0);
    let near = pfx_linear_depth(d, camera.x, camera.y);
    let flat_near = pfx_linear_depth_orthographic(d, camera.x, camera.y);
    let terms = pfx_projection_distance(pfx_standard_depth(d), camera.z, camera.w, false);
    return select(vec4f(near, flat_near, terms, 1.0), vec4f(0.0), pfx_is_sky(d));
}}
"
        );
        validate(&source);
    }

    #[test]
    fn a_stored_depth_is_one_minus_the_callers_ndc_and_gives_back_the_distance() {
        let convention = DepthConvention::REVERSED;
        for projection in [perspective(), orthographic()] {
            let ortho = projection[2][3] == 0.0;
            for distance in [NEAR, 0.5, 2.0, 37.0, 250.0, FAR] {
                let (classic, reversed) = stored(projection, distance);
                assert!(
                    (classic - reversed).abs() < 2e-6,
                    "{distance}: {classic} {reversed}"
                );
                let linear = if ortho {
                    convention.linear_depth_orthographic(reversed, NEAR, FAR)
                } else {
                    convention.linear_depth(reversed, NEAR, FAR)
                };
                let through = convention.projection_distance(reversed, &projection);
                let tolerance = distance * if ortho { 1e-3 } else { 2e-3 } + 1e-4;
                assert!(
                    (linear - distance).abs() < tolerance,
                    "{distance}: {linear}"
                );
                assert!(
                    (through - distance).abs() < tolerance,
                    "{distance}: {through}"
                );
            }
        }
        assert!((stored(perspective(), NEAR).1 - convention.near).abs() < 1e-6);
        assert!(stored(perspective(), FAR).1.abs() < 1e-6);
        assert!(convention.is_sky(convention.clear));
        assert!(convention.is_sky(convention.sky));
        assert!(!convention.is_sky(convention.near));
        assert_eq!(convention.linear_depth(convention.sky, NEAR, FAR), FAR);
        assert_eq!(convention.linear_depth(convention.near, NEAR, FAR), NEAR);
        assert_eq!(
            convention.standard_depth(convention.stored_depth(0.25)),
            0.25
        );
    }
}
