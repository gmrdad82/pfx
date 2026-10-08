use half::f16;
use serde::{Deserialize, Serialize};

pub const AXES: [[f32; 3]; 6] = [
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
];
const MAGIC: &[u8; 8] = b"PITOPRB1";
const HEADER: usize = 64;
const STRIDE_V1: usize = 60;
const STRIDE_V2: usize = 80;

fn packed(value: f32) -> f16 {
    f16::from_f32(value.clamp(0.0, 65504.0))
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridSpec {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub spacing: f32,
}

impl GridSpec {
    pub fn dimensions(self) -> Result<[u32; 3], String> {
        if !self.spacing.is_finite() || self.spacing <= 0.0 {
            return Err("invalid probe spacing".into());
        }
        let mut dims = [0; 3];
        for (axis, dim) in dims.iter_mut().enumerate() {
            if !self.min[axis].is_finite()
                || !self.max[axis].is_finite()
                || self.max[axis] < self.min[axis]
            {
                return Err("invalid probe bounds".into());
            }
            let ratio = (self.max[axis] as f64 - self.min[axis] as f64) / self.spacing as f64;
            let steps = ratio.round();
            if (ratio - steps).abs() > 1e-5 {
                return Err("probe bounds must be a multiple of spacing".into());
            }
            if steps >= u32::MAX as f64 {
                return Err("probe dimensions overflow".into());
            }
            *dim = steps as u32 + 1;
        }
        let count = u64::from(dims[0]) * u64::from(dims[1]) * u64::from(dims[2]);
        if count > u32::MAX as u64 || count > usize::MAX as u64 / STRIDE_V2 as u64 {
            return Err("probe count overflow".into());
        }
        Ok(dims)
    }

    pub fn index(dims: [u32; 3], xyz: [u32; 3]) -> usize {
        (xyz[2] as usize * dims[1] as usize + xyz[1] as usize) * dims[0] as usize + xyz[0] as usize
    }

    pub fn position(self, dims: [u32; 3], index: usize) -> [f32; 3] {
        let xyz = [
            index as u32 % dims[0],
            index as u32 / dims[0] % dims[1],
            index as u32 / dims[0] / dims[1],
        ];
        [0, 1, 2].map(|axis| self.min[axis] + xyz[axis] as f32 * self.spacing)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    pub lobes: [[f32; 3]; 6],
    pub visibility: [f32; 6],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProbeExtra {
    pub second: [f32; 6],
    pub offset: [f32; 3],
    pub enabled: bool,
    pub backfaces: u16,
}

impl Default for Probe {
    fn default() -> Self {
        Self {
            lobes: [[0.0; 3]; 6],
            visibility: [65504.0; 6],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub spec: GridSpec,
    pub dims: [u32; 3],
    pub probes: Vec<Probe>,
    pub extras: Option<Vec<ProbeExtra>>,
}

impl Grid {
    pub fn new(spec: GridSpec, probes: Vec<Probe>) -> Result<Self, String> {
        let dims = spec.dimensions()?;
        if probes.len() != dims.iter().map(|&v| v as usize).product::<usize>() {
            return Err("probe count does not match grid".into());
        }
        if probes.iter().any(|probe| {
            probe
                .lobes
                .iter()
                .flatten()
                .any(|value| !value.is_finite() || *value < 0.0)
                || probe
                    .visibility
                    .iter()
                    .any(|value| !value.is_finite() || *value < 0.0)
        }) {
            return Err("probe values must be finite and nonnegative".into());
        }
        Ok(Self {
            spec,
            dims,
            probes,
            extras: None,
        })
    }

    pub fn with_extras(
        spec: GridSpec,
        probes: Vec<Probe>,
        extras: Vec<ProbeExtra>,
    ) -> Result<Self, String> {
        let mut grid = Self::new(spec, probes)?;
        if extras.len() != grid.probes.len()
            || extras.iter().any(|extra| {
                extra
                    .second
                    .iter()
                    .any(|value| !value.is_finite() || *value < 0.0)
                    || extra
                        .offset
                        .iter()
                        .any(|value| !value.is_finite() || value.abs() > spec.spacing * 0.5)
                    || extra.backfaces > 32767
            })
        {
            return Err("invalid probe depth moments or state".into());
        }
        grid.extras = Some(extras);
        Ok(grid)
    }

    pub fn version(&self) -> u32 {
        if self.extras.is_some() { 2 } else { 1 }
    }

    pub fn sample(&self, position: [f32; 3], normal: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
        let length = normal.iter().map(|v| v * v).sum::<f32>().sqrt();
        if !length.is_finite() || length < 1e-6 || position.iter().any(|v| !v.is_finite()) {
            return fallback;
        }
        let n = normal.map(|v| v / length);
        let point: [f32; 3] = std::array::from_fn(|axis| position[axis] + n[axis] * 0.006);
        let mut base = [0; 3];
        let mut t = [0.0; 3];
        let last: [f32; 3] = std::array::from_fn(|axis| (self.dims[axis] - 1) as f32);
        let outside = |at: [f32; 3]| {
            (0..3).any(|axis| {
                let coord = (at[axis] - self.spec.min[axis]) / self.spec.spacing;
                coord < 0.0 || coord > last[axis]
            })
        };
        if outside(position) && outside(point) {
            return fallback;
        }
        for axis in 0..3 {
            let coord =
                ((point[axis] - self.spec.min[axis]) / self.spec.spacing).clamp(0.0, last[axis]);
            base[axis] = (coord.floor() as u32).min(self.dims[axis].saturating_sub(2));
            t[axis] = if self.dims[axis] == 1 {
                0.0
            } else {
                (coord - base[axis] as f32).clamp(0.0, 1.0)
            };
        }
        let mut color = [0.0; 3];
        let mut total = 0.0;
        for corner in 0..8 {
            let mut xyz = base;
            let mut weight = 1.0;
            for axis in 0..3 {
                let high = corner & (1 << axis) != 0;
                if high && self.dims[axis] > 1 {
                    xyz[axis] += 1;
                }
                weight *= if high { t[axis] } else { 1.0 - t[axis] };
            }
            if weight == 0.0 {
                continue;
            }
            let index = GridSpec::index(self.dims, xyz);
            let probe = &self.probes[index];
            let extra = self.extras.as_ref().map(|extras| &extras[index]);
            if extra.is_some_and(|extra| !extra.enabled) {
                continue;
            }
            let mut center = self.spec.position(self.dims, index);
            if let Some(extra) = extra {
                for (axis, value) in center.iter_mut().enumerate() {
                    *value += packed(extra.offset[axis].abs() / self.spec.spacing).to_f32()
                        * self.spec.spacing
                        * extra.offset[axis].signum();
                }
            }
            let toward_probe: [f32; 3] =
                std::array::from_fn(|axis| center[axis] - position[axis] + n[axis] * 0.0001);
            let side_length = toward_probe.iter().map(|v| v * v).sum::<f32>().sqrt();
            let facing = ((toward_probe
                .iter()
                .zip(n)
                .map(|(v, axis)| v * axis)
                .sum::<f32>()
                / side_length.max(1e-6))
                + 1.0)
                .clamp(0.0, 2.0)
                * 0.5;
            weight *= facing * facing * facing;
            if weight == 0.0 {
                continue;
            }
            let ray: [f32; 3] = std::array::from_fn(|axis| point[axis] - center[axis]);
            let distance = ray.iter().map(|v| v * v).sum::<f32>().sqrt();
            let mut irradiance = [0.0; 3];
            let mut visibility = 0.0;
            let mut mean = 0.0;
            let mut second = 0.0;
            for (lobe, axis_direction) in AXES.iter().enumerate() {
                let nw = (0..3)
                    .map(|axis| n[axis] * axis_direction[axis])
                    .sum::<f32>()
                    .max(0.0)
                    .powi(2);
                for (channel, value) in irradiance.iter_mut().enumerate() {
                    *value += packed(probe.lobes[lobe][channel]).to_f32() * nw;
                }
                let direction_weight = (0..3)
                    .map(|axis| ray[axis] * axis_direction[axis])
                    .sum::<f32>()
                    .max(0.0)
                    / distance.max(1e-6);
                let ray_weight = direction_weight * direction_weight;
                if let Some(extra) = extra {
                    mean += ray_weight
                        * packed(probe.visibility[lobe] / self.spec.spacing).to_f32()
                        * self.spec.spacing;
                    second += ray_weight
                        * packed(extra.second[lobe] / self.spec.spacing.powi(2)).to_f32()
                        * self.spec.spacing.powi(2);
                } else {
                    let reach = packed(probe.visibility[lobe]).to_f32() + self.spec.spacing * 0.25;
                    visibility +=
                        ray_weight * ((reach - distance) / self.spec.spacing).clamp(0.0, 1.0);
                }
            }
            if extra.is_some() {
                let variance = (second - mean * mean).max((self.spec.spacing * 0.1).powi(2));
                let delta = (distance - mean - self.spec.spacing * 0.1).max(0.0);
                let chance = variance / (variance + delta * delta);
                weight *= chance * chance;
            } else {
                weight *= if distance > 1e-6 { visibility } else { 1.0 };
            }
            for channel in 0..3 {
                color[channel] += irradiance[channel] * weight;
            }
            total += weight;
        }
        if total <= 1e-6 {
            fallback
        } else {
            color.map(|v| v / total)
        }
    }

    pub fn bytes(&self) -> Vec<u8> {
        let version = self.version();
        let stride = if version == 2 { STRIDE_V2 } else { STRIDE_V1 };
        let mut bytes = Vec::with_capacity(HEADER + self.probes.len() * stride);
        bytes.extend_from_slice(MAGIC);
        for value in [version, self.dims[0], self.dims[1], self.dims[2]] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in self
            .spec
            .min
            .into_iter()
            .chain(self.spec.max)
            .chain([self.spec.spacing])
        {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.resize(HEADER, 0);
        for (index, probe) in self.probes.iter().enumerate() {
            for lobe in probe.lobes {
                for value in lobe.into_iter().chain([0.0]) {
                    bytes.extend_from_slice(&packed(value).to_bits().to_le_bytes());
                }
            }
            for value in probe.visibility {
                let stored = if version == 2 {
                    value / self.spec.spacing
                } else {
                    value
                };
                bytes.extend_from_slice(&packed(stored).to_bits().to_le_bytes());
            }
            if let Some(extras) = &self.extras {
                let extra = &extras[index];
                for value in extra.second {
                    bytes.extend_from_slice(
                        &packed(value / self.spec.spacing.powi(2))
                            .to_bits()
                            .to_le_bytes(),
                    );
                }
                for value in extra.offset {
                    bytes.extend_from_slice(
                        &f16::from_f32((value / self.spec.spacing).clamp(-0.5, 0.5))
                            .to_bits()
                            .to_le_bytes(),
                    );
                }
                let flags = (extra.backfaces << 1) | u16::from(extra.enabled);
                bytes.extend_from_slice(&flags.to_le_bytes());
            }
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < HEADER || &bytes[..8] != MAGIC {
            return Err("invalid probe header".into());
        }
        let word = |at| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let version = word(8);
        if !matches!(version, 1 | 2) || bytes[52..64].iter().any(|&v| v != 0) {
            return Err("unsupported probe version".into());
        }
        let dims = [word(12), word(16), word(20)];
        let float = |at| f32::from_bits(word(at));
        let spec = GridSpec {
            min: [float(24), float(28), float(32)],
            max: [float(36), float(40), float(44)],
            spacing: float(48),
        };
        if spec.dimensions()? != dims {
            return Err("invalid probe dimensions".into());
        }
        let count = dims.iter().map(|&v| v as usize).product::<usize>();
        let stride = if version == 2 { STRIDE_V2 } else { STRIDE_V1 };
        if bytes.len() != HEADER + count * stride {
            return Err("invalid probe byte length".into());
        }
        let mut probes = Vec::with_capacity(count);
        let mut extras = Vec::with_capacity(count);
        for chunk in bytes[HEADER..].chunks_exact(stride) {
            let half = |at| f16::from_bits(u16::from_le_bytes([chunk[at], chunk[at + 1]])).to_f32();
            let mut probe = Probe::default();
            for lobe in 0..6 {
                for channel in 0..3 {
                    probe.lobes[lobe][channel] = half(lobe * 8 + channel * 2);
                }
                let stored = half(48 + lobe * 2);
                probe.visibility[lobe] = if version == 2 {
                    stored * spec.spacing
                } else {
                    stored
                };
            }
            probes.push(probe);
            if version == 2 {
                let flags = u16::from_le_bytes([chunk[78], chunk[79]]);
                extras.push(ProbeExtra {
                    second: std::array::from_fn(|lobe| half(60 + lobe * 2) * spec.spacing.powi(2)),
                    offset: std::array::from_fn(|axis| half(72 + axis * 2) * spec.spacing),
                    enabled: flags & 1 != 0,
                    backfaces: flags >> 1,
                });
            }
        }
        if version == 2 {
            Grid::with_extras(spec, probes, extras)
        } else {
            Grid::new(spec, probes)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnchorBlend {
    pub lower: usize,
    pub upper: usize,
    pub upper_weight: f32,
}

pub fn blend_anchors(hours: &[f32], hour: f32) -> Result<AnchorBlend, String> {
    if hours.is_empty()
        || !hour.is_finite()
        || hours.iter().any(|v| !v.is_finite())
        || hours.windows(2).any(|v| v[0] >= v[1])
    {
        return Err("anchors must be finite, sorted and distinct".into());
    }
    if hour <= hours[0] {
        return Ok(AnchorBlend {
            lower: 0,
            upper: 0,
            upper_weight: 0.0,
        });
    }
    for upper in 1..hours.len() {
        if hour < hours[upper] {
            return Ok(AnchorBlend {
                lower: upper - 1,
                upper,
                upper_weight: (hour - hours[upper - 1]) / (hours[upper] - hours[upper - 1]),
            });
        }
    }
    let last = hours.len() - 1;
    Ok(AnchorBlend {
        lower: last,
        upper: last,
        upper_weight: 0.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexing_and_binary_round_trip() {
        let spec = GridSpec {
            min: [-1.0, 0.0, 2.0],
            max: [1.0, 2.0, 4.0],
            spacing: 1.0,
        };
        let dims = spec.dimensions().unwrap();
        assert_eq!(dims, [3, 3, 3]);
        assert_eq!(GridSpec::index(dims, [2, 1, 1]), 14);
        assert_eq!(spec.position(dims, 14), [1.0, 1.0, 3.0]);
        let grid = Grid::new(spec, vec![Probe::default(); 27]).unwrap();
        assert_eq!(Grid::from_bytes(&grid.bytes()).unwrap(), grid);
        assert_eq!(
            Grid::from_bytes(&grid.bytes()).unwrap().bytes(),
            grid.bytes()
        );
        assert_eq!(&grid.bytes()[..8], b"PITOPRB1");
    }
    #[test]
    fn anchors_and_sampling() {
        assert_eq!(
            blend_anchors(&[8.0, 12.0, 16.0, 20.0], 14.0).unwrap(),
            AnchorBlend {
                lower: 1,
                upper: 2,
                upper_weight: 0.5
            }
        );
        for (hour, lower) in [(9.0, 0), (20.0, 1), (22.0, 2)] {
            let blend = blend_anchors(&[9.0, 20.0, 22.0], hour).unwrap();
            assert_eq!((blend.lower, blend.upper_weight), (lower, 0.0));
        }
        let spec = GridSpec {
            min: [0.0; 3],
            max: [1.0; 3],
            spacing: 1.0,
        };
        let mut probes = vec![Probe::default(); 8];
        for (index, probe) in probes.iter_mut().enumerate() {
            probe.lobes[0] = [index as f32; 3];
        }
        let grid = Grid::new(spec, probes).unwrap();
        let sample = grid.sample([0.5; 3], [1.0, 0.0, 0.0], [9.0; 3]);
        assert!((3.95..4.0).contains(&sample[0]));
        assert_eq!(grid.sample([2.0; 3], [1.0, 0.0, 0.0], [9.0; 3]), [9.0; 3]);
    }

    #[test]
    fn thin_wall_keeps_the_unlit_side_dark_at_each_spacing() {
        for spacing in [0.02, 0.03, 0.05] {
            let spec = GridSpec {
                min: [0.0, 0.0, -spacing],
                max: [0.0, 0.0, spacing],
                spacing,
            };
            let probes = [-spacing, 0.0, spacing]
                .map(|z| Probe {
                    lobes: [[if z > 0.0 { 1.0 } else { 0.0 }; 3]; 6],
                    ..Probe::default()
                })
                .to_vec();
            let grid = Grid::new(spec, probes).unwrap();
            assert_eq!(
                grid.sample([0.0, 0.0, -0.001], [0.0, 0.0, -1.0], [0.0; 3]),
                [0.0; 3]
            );
            assert!(grid.sample([0.0, 0.0, 0.001], [0.0, 0.0, 1.0], [0.0; 3])[0] > 0.9);
        }
    }

    #[test]
    fn a_surface_inside_a_thin_grid_keeps_its_probe_after_the_normal_offset() {
        let spec = GridSpec {
            min: [0.0, 0.01, 0.0],
            max: [0.0, 0.01, 0.0],
            spacing: 1.0,
        };
        let mut lit = Probe::default();
        lit.lobes[2] = [0.5; 3];
        lit.visibility = [4.0; 6];
        let grid = Grid::new(spec, vec![lit]).unwrap();
        assert_eq!(
            grid.sample([0.0, 0.01, 0.0], [0.0, 1.0, 0.0], [9.0; 3]),
            [0.5; 3]
        );
        assert_eq!(
            grid.sample([0.0, 0.5, 0.0], [0.0, 1.0, 0.0], [9.0; 3]),
            [9.0; 3]
        );
    }

    #[test]
    fn visibility_uses_the_probe_to_point_direction() {
        let spec = GridSpec {
            min: [0.0, 0.006, 0.0],
            max: [1.0, 0.006, 0.0],
            spacing: 1.0,
        };
        let mut blocked = Probe::default();
        blocked.lobes[2] = [1.0; 3];
        blocked.visibility[0] = 0.1;
        let grid = Grid::new(spec, vec![blocked, Probe::default()]).unwrap();
        assert_eq!(
            grid.sample([0.5, 0.0, 0.0], [0.0, 1.0, 0.0], [9.0; 3]),
            [0.0; 3]
        );
    }

    #[test]
    fn version_two_round_trip_keeps_probe_state_and_moments() {
        let spec = GridSpec {
            min: [0.0; 3],
            max: [1.0, 0.0, 0.0],
            spacing: 1.0,
        };
        let probe = Probe {
            visibility: [4.0; 6],
            ..Probe::default()
        };
        let extra = ProbeExtra {
            second: [16.0; 6],
            offset: [0.25, 0.0, 0.0],
            enabled: true,
            backfaces: 3,
        };
        let grid = Grid::with_extras(spec, vec![probe; 2], vec![extra; 2]).unwrap();
        let bytes = grid.bytes();
        assert_eq!(bytes.len(), 64 + 2 * 80);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 2);
        let decoded = Grid::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.version(), 2);
        assert_eq!(decoded.bytes(), bytes);
        assert_eq!(decoded.extras.unwrap()[0].backfaces, 3);
    }

    #[test]
    fn chebyshev_moments_reject_a_blocked_probe() {
        let spec = GridSpec {
            min: [0.0, 0.006, 0.0],
            max: [1.0, 0.006, 0.0],
            spacing: 1.0,
        };
        let mut blocked = Probe::default();
        blocked.lobes[2] = [1.0; 3];
        blocked.visibility = [0.1; 6];
        let open = Probe {
            visibility: [4.0; 6],
            ..Probe::default()
        };
        let visible = ProbeExtra {
            second: [0.01; 6],
            offset: [0.0; 3],
            enabled: true,
            backfaces: 0,
        };
        let clear = ProbeExtra {
            second: [16.0; 6],
            ..visible.clone()
        };
        let grid = Grid::with_extras(
            spec,
            vec![blocked.clone(), open.clone()],
            vec![visible.clone(), clear.clone()],
        )
        .unwrap();
        let sample = grid.sample([0.5, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0; 3]);
        assert!(sample[0] < 0.01);
        let disabled = Grid::with_extras(
            spec,
            vec![blocked, open],
            vec![
                ProbeExtra {
                    enabled: false,
                    ..visible
                },
                clear,
            ],
        )
        .unwrap();
        assert_eq!(
            disabled.sample([0.5, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0; 3]),
            [0.0; 3]
        );
    }
}
