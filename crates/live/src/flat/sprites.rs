use wgpu::util::DeviceExt;

pub const SPRITE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpriteFilter {
    #[default]
    Linear,
    Nearest,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpriteImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub filter: SpriteFilter,
}

impl SpriteImage {
    pub fn new(
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        filter: SpriteFilter,
    ) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("a sprite atlas needs a size".into());
        }
        if rgba.len() != width as usize * height as usize * 4 {
            return Err(format!(
                "a {width}x{height} sprite atlas needs {} bytes, not {}",
                width as usize * height as usize * 4,
                rgba.len()
            ));
        }
        Ok(Self {
            width,
            height,
            rgba,
            filter,
        })
    }

    pub fn from_premultiplied(
        width: u32,
        height: u32,
        pixels: &[[f32; 4]],
        filter: SpriteFilter,
    ) -> Result<Self, String> {
        if pixels.len() != width as usize * height as usize {
            return Err(format!(
                "a {width}x{height} sprite atlas needs {} pixels, not {}",
                width as usize * height as usize,
                pixels.len()
            ));
        }
        let (w, h) = (width as usize, height as usize);
        let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        let mut rgba = Vec::with_capacity(pixels.len() * 4);
        for y in 0..h {
            for x in 0..w {
                let [r, g, b, a] = pixels[y * w + x];
                let (colour, alpha) = if a > 0.0 {
                    ([r, g, b], a)
                } else {
                    let mut sum = [0.0f32; 4];
                    for ny in y.saturating_sub(1)..(y + 2).min(h) {
                        for nx in x.saturating_sub(1)..(x + 2).min(w) {
                            let near = pixels[ny * w + nx];
                            for k in 0..4 {
                                sum[k] += near[k];
                            }
                        }
                    }
                    ([sum[0], sum[1], sum[2]], sum[3])
                };
                let scale = if alpha > 0.0 { 1.0 / alpha } else { 0.0 };
                rgba.extend(colour.map(|value| byte(value * scale)));
                rgba.push(byte(a));
            }
        }
        Self::new(width, height, rgba, filter)
    }

    fn texel(&self, x: i64, y: i64) -> [f32; 4] {
        let x = x.clamp(0, i64::from(self.width) - 1) as usize;
        let y = y.clamp(0, i64::from(self.height) - 1) as usize;
        let at = (y * self.width as usize + x) * 4;
        std::array::from_fn(|channel| f32::from(self.rgba[at + channel]) / 255.0)
    }

    pub fn sample(&self, uv: [f32; 2]) -> [f32; 4] {
        let x = uv[0] * self.width as f32;
        let y = uv[1] * self.height as f32;
        match self.filter {
            SpriteFilter::Nearest => self.texel(x.floor() as i64, y.floor() as i64),
            SpriteFilter::Linear => {
                let fx = x - 0.5;
                let fy = y - 0.5;
                let x0 = fx.floor();
                let y0 = fy.floor();
                let tx = fx - x0;
                let ty = fy - y0;
                let (x0, y0) = (x0 as i64, y0 as i64);
                let a = self.texel(x0, y0);
                let b = self.texel(x0 + 1, y0);
                let c = self.texel(x0, y0 + 1);
                let d = self.texel(x0 + 1, y0 + 1);
                std::array::from_fn(|k| {
                    let top = a[k] + (b[k] - a[k]) * tx;
                    let bottom = c[k] + (d[k] - c[k]) * tx;
                    top + (bottom - top) * ty
                })
            }
        }
    }
}

pub struct FlatSprites {
    pub image: SpriteImage,
    texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) sampler: wgpu::Sampler,
}

impl FlatSprites {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, image: SpriteImage) -> Self {
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("flat sprites"),
                size: wgpu::Extent3d {
                    width: image.width,
                    height: image.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: SPRITE_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &image.rgba,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = sampler(device, image.filter);
        Self {
            image,
            texture,
            view,
            sampler,
        }
    }

    pub fn live(device: &wgpu::Device, size: [u32; 2], filter: SpriteFilter) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("flat live sprites"),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SPRITE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            image: SpriteImage {
                width: 1,
                height: 1,
                rgba: vec![0; 4],
                filter,
            },
            texture,
            view,
            sampler: sampler(device, filter),
        }
    }

    pub fn size(&self) -> [u32; 2] {
        [self.texture.width(), self.texture.height()]
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
}

pub(crate) fn sampler(device: &wgpu::Device, filter: SpriteFilter) -> wgpu::Sampler {
    let filter = match filter {
        SpriteFilter::Linear => wgpu::FilterMode::Linear,
        SpriteFilter::Nearest => wgpu::FilterMode::Nearest,
    };
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("flat sprites"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: filter,
        min_filter: filter,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    })
}
