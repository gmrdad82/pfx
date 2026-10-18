use crate::{Part, maps};
use pfx_glb::{Image, Looks, Map, Mime, Node};
use pfx_recipe::{Kind, Recipe};
use std::collections::BTreeMap;
use std::path::Path;

pub const CARD: &str = "card";

#[derive(Clone, Debug)]
pub(crate) struct Art {
    image: Image,
    cutoff: Option<f32>,
}

pub(crate) fn read(
    recipe: &Recipe,
    base: &Path,
    parts: &[Part],
    notes: &mut Vec<String>,
) -> BTreeMap<usize, Art> {
    let mut out = BTreeMap::new();
    let mut files: BTreeMap<&str, Image> = BTreeMap::new();
    for part in parts.iter().filter(|p| !p.hidden) {
        let object = &recipe.objects[part.object];
        let Kind::Card {
            image: Some(path),
            alpha_cutoff,
            ..
        } = &object.kind
        else {
            continue;
        };
        if out.contains_key(&part.object) {
            continue;
        }
        let image = match files.get(path.as_str()) {
            Some(image) => image.clone(),
            None => {
                let file = base.join(path);
                let read = pfx_recipe::files::read(&file)
                    .map_err(|e| e.to_string())
                    .and_then(maps::image);
                match read {
                    Ok(image) => {
                        files.insert(path, image.clone());
                        image
                    }
                    Err(e) => {
                        notes.push(format!(
                            "object[{}].image: {}: {e}; the GLB carries the plain quad",
                            object.name,
                            file.display()
                        ));
                        continue;
                    }
                }
            }
        };
        let cutoff = (image.mime == Mime::Png && *alpha_cutoff > 0.0)
            .then_some(alpha_cutoff.min(1.0) as f32);
        out.insert(part.object, Art { image, cutoff });
    }
    out
}

pub(crate) fn dress(node: &mut Node, art: &Art, looks: &mut Looks) {
    if node.mesh.slots.is_empty() {
        node.mesh.set_material(CARD);
    }
    for slot in &mut node.mesh.slots {
        let name = format!("{}+{}", slot.material, node.name);
        let mut look = looks.get(&slot.material).cloned().unwrap_or_default();
        look.base_color = Some([1.0; 4]);
        look.base = Some(Map::new(art.image.clone()));
        look.alpha_cutoff = art.cutoff;
        looks.insert(name.clone(), look);
        slot.material = name;
    }
}
