use pfx_gpu::wgpu;
use pfx_gpu::wgpu::util::DeviceExt;

pub const ATLAS_WIDTH: u32 = 1024;
pub const CELL_TEXELS: f32 = 160.0;
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
pub const RAW_MAGIC: [u8; 4] = *b"PFXI";
pub const RAW_VERSION: u32 = 1;
const RAW_HEADER: usize = 20;
const RAW_ENTRY: usize = 36;
const RAW_MAX_SIDE: u32 = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Icon(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconKind {
    Fill,
    Line,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IconShape {
    pub polygons: Vec<Vec<[f32; 2]>>,
    pub lines: Vec<Vec<[f32; 2]>>,
    pub circles: Vec<([f32; 2], f32)>,
    pub reach: f32,
}

impl IconShape {
    pub fn polygon(points: &[[f32; 2]], reach: f32) -> Self {
        Self {
            polygons: vec![points.to_vec()],
            reach,
            ..Self::default()
        }
    }

    pub fn lines(lines: &[&[[f32; 2]]], reach: f32) -> Self {
        Self {
            lines: lines.iter().map(|line| line.to_vec()).collect(),
            reach,
            ..Self::default()
        }
    }

    pub fn with_circle(mut self, centre: [f32; 2], radius: f32) -> Self {
        self.circles.push((centre, radius));
        self
    }

    pub fn kind(&self) -> IconKind {
        if self.polygons.is_empty() {
            IconKind::Line
        } else {
            IconKind::Fill
        }
    }

    fn bounds(&self) -> Option<[f32; 4]> {
        let mut points = self
            .polygons
            .iter()
            .chain(&self.lines)
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        for &(centre, radius) in &self.circles {
            points.push([centre[0] - radius, centre[1] - radius]);
            points.push([centre[0] + radius, centre[1] + radius]);
        }
        let first = *points.first()?;
        Some(points.iter().fold(
            [first[0], first[1], first[0], first[1]],
            |[x0, y0, x1, y1], p| [x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1])],
        ))
    }

    pub fn distance(&self, p: [f32; 2]) -> f32 {
        let mut nearest = f32::MAX;
        let mut inside = false;
        for polygon in &self.polygons {
            for (index, &a) in polygon.iter().enumerate() {
                let b = polygon[(index + 1) % polygon.len()];
                nearest = nearest.min(segment(p, a, b));
                if (a[1] > p[1]) != (b[1] > p[1])
                    && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
                {
                    inside = !inside;
                }
            }
        }
        for line in &self.lines {
            if line.len() == 1 {
                nearest = nearest.min(segment(p, line[0], line[0]));
            }
            for pair in line.windows(2) {
                nearest = nearest.min(segment(p, pair[0], pair[1]));
            }
        }
        for &(centre, radius) in &self.circles {
            let offset = [p[0] - centre[0], p[1] - centre[1]];
            nearest = nearest
                .min(((offset[0] * offset[0] + offset[1] * offset[1]).sqrt() - radius).abs());
        }
        if inside { -nearest } else { nearest }
    }
}

pub(crate) fn segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let span = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if span > 0.0 {
        ((ap[0] * ab[0] + ap[1] * ab[1]) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let d = [ap[0] - ab[0] * t, ap[1] - ab[1] * t];
    (d[0] * d[0] + d[1] * d[1]).sqrt()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconEntry {
    pub kind: IconKind,
    pub bounds: [f32; 4],
    pub uv: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Icons {
    pub width: u32,
    pub height: u32,
    texels: Vec<u16>,
    entries: Vec<IconEntry>,
}

impl Icons {
    pub fn bake(shapes: &[IconShape]) -> Result<Self, String> {
        let mut cells = Vec::with_capacity(shapes.len());
        let (mut x, mut y, mut row) = (0u32, 0u32, 0u32);
        for shape in shapes {
            let [x0, y0, x1, y1] = shape.bounds().ok_or("an icon has no geometry")?;
            if !(shape.reach.is_finite() && shape.reach >= 0.0) {
                return Err("an icon's reach must be finite and nonnegative".into());
            }
            let bounds = [
                x0 - shape.reach,
                y0 - shape.reach,
                x1 + shape.reach,
                y1 + shape.reach,
            ];
            let span = (bounds[2] - bounds[0]).max(bounds[3] - bounds[1]);
            if !(span.is_finite() && span > 0.0) {
                return Err("an icon has no extent".into());
            }
            let density = CELL_TEXELS / span;
            let size = [
                ((bounds[2] - bounds[0]) * density).ceil() as u32 + 1,
                ((bounds[3] - bounds[1]) * density).ceil() as u32 + 1,
            ];
            if x + size[0] > ATLAS_WIDTH {
                x = 0;
                y += row;
                row = 0;
            }
            cells.push((bounds, density, [x, y], size));
            x += size[0];
            row = row.max(size[1]);
        }
        let width = ATLAS_WIDTH;
        let height = (y + row).max(1).next_power_of_two();
        if height > 8192 {
            return Err("the icon atlas is taller than 8192 texels".into());
        }
        let mut texels = vec![half::f16::from_f32(1.0e4).to_bits(); (width * height) as usize];
        let mut entries = Vec::with_capacity(shapes.len());
        for (shape, &(bounds, density, origin, size)) in shapes.iter().zip(&cells) {
            for row in 0..size[1] {
                for column in 0..size[0] {
                    let p = [
                        bounds[0] + column as f32 / density,
                        bounds[1] + row as f32 / density,
                    ];
                    let at = ((origin[1] + row) * width + origin[0] + column) as usize;
                    texels[at] = half::f16::from_f32(shape.distance(p)).to_bits();
                }
            }
            let scale = [density / width as f32, density / height as f32];
            entries.push(IconEntry {
                kind: shape.kind(),
                bounds,
                uv: [
                    (origin[0] as f32 + 0.5) / width as f32 - bounds[0] * scale[0],
                    (origin[1] as f32 + 0.5) / height as f32 - bounds[1] * scale[1],
                    scale[0],
                    scale[1],
                ],
            });
        }
        Ok(Self {
            width,
            height,
            texels,
            entries,
        })
    }

    pub fn to_raw(&self) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(RAW_HEADER + self.entries.len() * RAW_ENTRY + self.texels.len() * 2);
        out.extend_from_slice(&RAW_MAGIC);
        out.extend_from_slice(&RAW_VERSION.to_le_bytes());
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for entry in &self.entries {
            let kind: u32 = match entry.kind {
                IconKind::Fill => 0,
                IconKind::Line => 1,
            };
            out.extend_from_slice(&kind.to_le_bytes());
            for value in entry.bounds.iter().chain(&entry.uv) {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        for texel in &self.texels {
            out.extend_from_slice(&texel.to_le_bytes());
        }
        out
    }

    pub fn from_raw(bytes: &[u8]) -> Result<Self, String> {
        let u32_at = |at: usize| {
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        if bytes.len() < RAW_HEADER {
            return Err("an icon atlas is shorter than its header".into());
        }
        if bytes[..4] != RAW_MAGIC {
            return Err("not a baked icon atlas".into());
        }
        let version = u32_at(4);
        if version != RAW_VERSION {
            return Err(format!(
                "icon atlas format {version}, this build reads {RAW_VERSION}"
            ));
        }
        let (width, height, count) = (u32_at(8), u32_at(12), u32_at(16));
        if !(1..=RAW_MAX_SIDE).contains(&width) || !(1..=RAW_MAX_SIDE).contains(&height) {
            return Err(format!(
                "an icon atlas of {width}x{height} texels is outside 1 to {RAW_MAX_SIDE}"
            ));
        }
        let texels_at = RAW_HEADER as u64 + u64::from(count) * RAW_ENTRY as u64;
        let need = texels_at + u64::from(width) * u64::from(height) * 2;
        if bytes.len() as u64 != need {
            return Err(format!(
                "a {width}x{height} icon atlas of {count} icons is {need} bytes, not {}",
                bytes.len()
            ));
        }
        let mut entries = Vec::with_capacity(count as usize);
        for index in 0..count as usize {
            let at = RAW_HEADER + index * RAW_ENTRY;
            let kind = match u32_at(at) {
                0 => IconKind::Fill,
                1 => IconKind::Line,
                other => return Err(format!("icon {index} has kind {other}")),
            };
            let value = |k: usize| {
                f32::from_le_bytes(bytes[at + 4 + k * 4..at + 8 + k * 4].try_into().unwrap())
            };
            let bounds = [value(0), value(1), value(2), value(3)];
            let uv = [value(4), value(5), value(6), value(7)];
            if !bounds.iter().chain(&uv).all(|value| value.is_finite()) {
                return Err(format!("icon {index} has a value that is not finite"));
            }
            entries.push(IconEntry { kind, bounds, uv });
        }
        let texels = bytes[texels_at as usize..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        if texels.iter().any(|&bits| bits & 0x7c00 == 0x7c00) {
            return Err("an icon atlas texel is not finite".into());
        }
        Ok(Self {
            width,
            height,
            texels,
            entries,
        })
    }

    pub fn entry(&self, icon: Icon) -> Option<&IconEntry> {
        self.entries.get(icon.0 as usize)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn texels(&self) -> &[u16] {
        &self.texels
    }

    fn texel(&self, x: i64, y: i64) -> f32 {
        let x = x.clamp(0, i64::from(self.width) - 1) as u32;
        let y = y.clamp(0, i64::from(self.height) - 1) as u32;
        half::f16::from_bits(self.texels[(y * self.width + x) as usize]).to_f32()
    }

    pub fn sample(&self, uv: [f32; 2]) -> f32 {
        let x = uv[0] * self.width as f32 - 0.5;
        let y = uv[1] * self.height as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let top = self.texel(x0, y0) * (1.0 - fx) + self.texel(x0 + 1, y0) * fx;
        let bottom = self.texel(x0, y0 + 1) * (1.0 - fx) + self.texel(x0 + 1, y0 + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    pub fn upload(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> GpuIcons {
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("flat icon fields"),
                size: wgpu::Extent3d {
                    width: self.width,
                    height: self.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            bytemuck::cast_slice(&self.texels),
        );
        let view = texture.create_view(&Default::default());
        GpuIcons {
            _texture: texture,
            view,
        }
    }
}

pub struct GpuIcons {
    _texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

pub fn pointer() -> IconShape {
    IconShape::polygon(
        &[
            [0.0, 0.0],
            [0.0, 52.0],
            [12.0, 41.0],
            [21.0, 60.0],
            [30.0, 56.0],
            [21.0, 37.0],
            [37.0, 37.0],
        ],
        6.0,
    )
}

pub fn tick() -> IconShape {
    IconShape::lines(&[&[[-10.0, 1.0], [-3.0, 9.0], [11.0, -8.0]]], 6.0)
}

pub fn plus() -> IconShape {
    IconShape::lines(
        &[&[[-16.0, 0.0], [16.0, 0.0]], &[[0.0, -16.0], [0.0, 16.0]]],
        7.0,
    )
}

pub fn cross() -> IconShape {
    IconShape::lines(
        &[
            &[[-14.0, -14.0], [14.0, 14.0]],
            &[[14.0, -14.0], [-14.0, 14.0]],
        ],
        7.0,
    )
}

pub fn arrows() -> IconShape {
    IconShape::lines(
        &[
            &[[-20.0, 0.0], [-70.0, 0.0]],
            &[[-52.0, -16.0], [-70.0, 0.0], [-52.0, 16.0]],
            &[[20.0, 0.0], [70.0, 0.0]],
            &[[52.0, -16.0], [70.0, 0.0], [52.0, 16.0]],
        ],
        6.0,
    )
}

pub fn menu() -> IconShape {
    IconShape::lines(
        &[
            &[[-25.0, -20.0], [25.0, -20.0]],
            &[[-25.0, 0.0], [25.0, 0.0]],
            &[[-25.0, 20.0], [25.0, 20.0]],
        ],
        6.0,
    )
}

pub fn search() -> IconShape {
    IconShape::lines(&[&[[6.0, 7.0], [14.0, 15.0]]], 4.0).with_circle([0.0, 0.0], 9.0)
}

pub fn page() -> IconShape {
    IconShape::polygon(
        &[
            [0.0, 0.0],
            [32.0, 0.0],
            [50.0, 18.0],
            [50.0, 64.0],
            [0.0, 64.0],
        ],
        3.0,
    )
}

pub fn fold() -> IconShape {
    IconShape::polygon(&[[32.0, 0.0], [32.0, 18.0], [50.0, 18.0]], 3.0)
}

pub fn funnel() -> IconShape {
    IconShape::polygon(
        &[
            [-12.0, -10.0],
            [12.0, -10.0],
            [3.0, 2.0],
            [3.0, 12.0],
            [-3.0, 9.0],
            [-3.0, 2.0],
        ],
        3.0,
    )
}

pub fn standard() -> Vec<(&'static str, IconShape)> {
    vec![
        ("pointer", pointer()),
        ("tick", tick()),
        ("plus", plus()),
        ("cross", cross()),
        ("arrows", arrows()),
        ("menu", menu()),
        ("search", search()),
        ("page", page()),
        ("fold", fold()),
        ("funnel", funnel()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygon_fields_are_signed_and_line_fields_are_not() {
        let pointer = pointer();
        assert_eq!(pointer.kind(), IconKind::Fill);
        assert!(pointer.distance([10.0, 30.0]) < 0.0);
        assert!((pointer.distance([-3.0, 30.0]) - 3.0).abs() < 1e-5);
        assert!((pointer.distance([2.0, 30.0]) + 2.0).abs() < 1e-5);
        let tick = tick();
        assert_eq!(tick.kind(), IconKind::Line);
        assert!((tick.distance([-10.0, -2.0]) - 3.0).abs() < 1e-5);
        assert!(tick.distance([-3.0, 9.0]).abs() < 1e-5);
        let search = search();
        assert!((search.distance([0.0, 0.0]) - 9.0).abs() < 1e-5);
        assert!(search.distance([9.0, 0.0]).abs() < 1e-5);
    }

    #[test]
    fn the_atlas_samples_back_its_distances() {
        let shapes = standard()
            .into_iter()
            .map(|(_, shape)| shape)
            .collect::<Vec<_>>();
        let icons = Icons::bake(&shapes).unwrap();
        assert_eq!(icons.len(), shapes.len());
        assert_eq!(icons.width, ATLAS_WIDTH);
        assert!(icons.height.is_power_of_two());
        for (index, shape) in shapes.iter().enumerate() {
            let entry = *icons.entry(Icon(index as u32)).unwrap();
            assert_eq!(entry.kind, shape.kind());
            let [x0, y0, x1, y1] = entry.bounds;
            for i in 1..20 {
                for j in 1..20 {
                    let p = [
                        x0 + (x1 - x0) * i as f32 / 20.0,
                        y0 + (y1 - y0) * j as f32 / 20.0,
                    ];
                    let uv = [
                        entry.uv[0] + p[0] * entry.uv[2],
                        entry.uv[1] + p[1] * entry.uv[3],
                    ];
                    let texel = (x1 - x0).max(y1 - y0) / CELL_TEXELS;
                    let exact = shape.distance(p);
                    let sampled = icons.sample(uv);
                    assert!(
                        (sampled - exact).abs() < texel * 0.75 + exact.abs() * 2e-3,
                        "icon {index} at {p:?}: {sampled} vs {exact}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_baked_atlas_round_trips_through_raw_bytes() {
        let shapes = standard()
            .into_iter()
            .map(|(_, shape)| shape)
            .collect::<Vec<_>>();
        let icons = Icons::bake(&shapes).unwrap();
        let raw = icons.to_raw();
        assert_eq!(Icons::from_raw(&raw).unwrap(), icons);
    }

    #[test]
    fn crafted_raw_atlases_are_refused() {
        let raw = Icons::bake(&[pointer(), tick()]).unwrap().to_raw();
        for cut in [0, 3, 19, 20, 55, raw.len() - 1] {
            assert!(Icons::from_raw(&raw[..cut]).is_err(), "cut at {cut}");
        }
        let mut longer = raw.clone();
        longer.push(0);
        assert!(Icons::from_raw(&longer).is_err());
        let mut huge = raw.clone();
        huge[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Icons::from_raw(&huge).is_err());
        let mut wide = raw.clone();
        wide[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        wide[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Icons::from_raw(&wide).is_err());
        let mut kind = raw.clone();
        kind[20..24].copy_from_slice(&7u32.to_le_bytes());
        assert!(Icons::from_raw(&kind).is_err());
        let mut uv = raw.clone();
        uv[40..44].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(Icons::from_raw(&uv).is_err());
        let mut texel = raw.clone();
        let last = texel.len() - 2;
        texel[last..].copy_from_slice(&0x7c00u16.to_le_bytes());
        assert!(Icons::from_raw(&texel).is_err());
        let mut version = raw;
        version[4] = 2;
        assert!(Icons::from_raw(&version).is_err());
    }

    #[test]
    fn an_empty_icon_is_refused() {
        assert!(Icons::bake(&[IconShape::default()]).is_err());
    }
}
