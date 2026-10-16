use std::sync::Arc;

use pfx_materials::Material;

use crate::image::Image;
use crate::surface::Surface;

use super::Scene;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Textures {
    pub colour: Vec<Arc<Image>>,
    pub linear: Vec<Arc<Image>>,
    pub orm: Vec<Arc<Image>>,
}

pub fn surface_name(mesh: &str, surface: &Surface) -> String {
    format!("gltf {mesh}/{}", surface.name)
}

fn slot(list: &mut Vec<Arc<Image>>, image: &Option<Arc<Image>>) -> i32 {
    let Some(image) = image else {
        return -1;
    };
    let index = match list
        .iter()
        .position(|seen| Arc::ptr_eq(seen, image) || **seen == **image)
    {
        Some(index) => index,
        None => {
            list.push(image.clone());
            list.len() - 1
        }
    };
    index as i32
}

impl Textures {
    pub fn is_empty(&self) -> bool {
        self.colour.is_empty() && self.linear.is_empty() && self.orm.is_empty()
    }

    pub fn material(&mut self, surface: &Surface) -> Material {
        let mut material = surface.material;
        let maps = &surface.maps;
        material.textures.base = slot(&mut self.colour, &maps.base);
        material.textures.normal = slot(&mut self.linear, &maps.normal);
        material.textures.detail = slot(&mut self.linear, &maps.detail);
        material.textures.anisotropy = slot(&mut self.linear, &maps.anisotropy);
        material.textures.orm = slot(&mut self.orm, &maps.orm);
        material
    }
}

pub(super) fn tangents_for_lobes(scene: &mut Scene) {
    let mut wanted = Vec::new();
    for object in &scene.objects {
        let Some(mesh) = scene.meshes.get(&object.mesh) else {
            continue;
        };
        for (index, part) in mesh.parts.iter().enumerate() {
            let name = super::draws::pick(scene, object, part);
            let material = scene
                .library
                .get(&name)
                .copied()
                .or_else(|| part.surface.as_ref().map(|surface| surface.material));
            if material.is_some_and(|material| !material.anisotropy.off())
                && part.geometry.placeholder_tangents()
            {
                wanted.push((object.mesh.clone(), index));
            }
        }
    }
    for (mesh, index) in wanted {
        if let Some(part) = scene
            .meshes
            .get_mut(&mesh)
            .and_then(|mesh| mesh.parts.get_mut(index))
            && part.geometry.placeholder_tangents()
        {
            part.geometry = Arc::new((*part.geometry).clone().with_uv_tangents());
        }
    }
}
