use pfx_gpu::wgpu;
use pfx_gpu::wgpu::util::DeviceExt;

pub const ATLAS_WIDTH: u32 = 1024;
pub const CELL_TEXELS: f32 = 160.0;
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

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

fn segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
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
    fn an_empty_icon_is_refused() {
        assert!(Icons::bake(&[IconShape::default()]).is_err());
    }
}
