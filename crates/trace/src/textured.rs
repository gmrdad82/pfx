use std::sync::Arc;

use pfx_load::Image;
use pfx_load::scene::textures::Textures as SceneTextures;
use pfx_load::surface::rgba8;
use pfx_materials::PackedMaterial;

use crate::detail::{ContentImage, Textures};
use crate::gpu::GpuContent;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Slots {
    pub colour: (usize, usize),
    pub linear: (usize, usize),
    pub orm: (usize, usize),
}

fn slot(index: f32, (first, count): (usize, usize)) -> f32 {
    if index < 0.0 || index as usize >= count {
        return -1.0;
    }
    (first + index as usize) as f32
}

impl Slots {
    pub fn remap(&self, packed: &mut PackedMaterial) {
        let [base, normal, orm, detail] = packed.textures;
        packed.textures = [
            slot(base, self.colour),
            slot(normal, self.linear),
            slot(orm, self.orm),
            slot(detail, self.linear),
        ];
        packed.texture_extra[0] = slot(packed.texture_extra[0], self.linear);
    }
}

pub(crate) fn place(
    textures: &Textures,
    info: &mut Vec<GpuContent>,
    texels: &mut Vec<[f32; 4]>,
) -> Slots {
    let mut push = |images: &[ContentImage]| {
        let first = info.len();
        for image in images {
            info.push(GpuContent {
                offset: texels.len() as u32,
                width: image.width,
                height: image.height,
                kind: 0,
            });
            texels.extend_from_slice(&image.texels);
        }
        (first, images.len())
    };
    Slots {
        colour: push(&textures.colour),
        linear: push(&textures.linear),
        orm: push(&textures.orm),
    }
}

pub(crate) fn wants_uvs1(packed: &PackedMaterial) -> bool {
    let sets = packed.texture_scales[3].round() as u32;
    let main = packed.textures[0] >= 0.0
        || packed.textures[1] >= 0.0
        || packed.textures[2] >= 0.0
        || packed.texture_extra[0] >= 0.0;
    (main && sets & 1 != 0) || (packed.textures[3] >= 0.0 && sets & 2 != 0)
}

pub(crate) fn place_uvs1(
    triangles: impl Iterator<Item = [[f32; 2]; 3]>,
    texels: &mut Vec<[f32; 4]>,
) -> Result<u32, String> {
    let first = texels.len() + 1;
    for [a, b, c] in triangles {
        texels.push([a[0], a[1], b[0], b[1]]);
        texels.push([c[0], c[1], 0.0, 0.0]);
    }
    u32::try_from(first).map_err(|_| "trace content is too large".to_string())
}

fn content(image: &Image, srgb: bool) -> ContentImage {
    let decode = |byte: u8| {
        let value = f32::from(byte) / 255.0;
        if srgb {
            pfx_materials::linear_channel(value)
        } else {
            value
        }
    };
    ContentImage {
        width: image.width,
        height: image.height,
        texels: rgba8(image)
            .into_iter()
            .map(|[r, g, b, a]| [decode(r), decode(g), decode(b), f32::from(a) / 255.0])
            .collect(),
    }
}

pub fn from_scene(textures: &SceneTextures) -> Textures {
    let list = |images: &[Arc<Image>], srgb: bool| {
        images
            .iter()
            .map(|image| content(image, srgb))
            .collect::<Vec<_>>()
    };
    Textures {
        colour: list(&textures.colour, true),
        linear: list(&textures.linear, false),
        orm: list(&textures.orm, false),
    }
}
