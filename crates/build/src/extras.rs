use crate::Part;
use pfx_glb::{MoverFacts, mover_json};
use pfx_recipe::{Recipe, Shadow};
use serde_json::{Map, Value, json};

pub(crate) fn of(recipe: &Recipe, parts: &[Part], movers: &[MoverFacts]) -> Vec<Option<Value>> {
    let mut rooted = vec![false; recipe.movers.len()];
    parts
        .iter()
        .map(|part| {
            let mut pfx = Map::new();
            let name = &recipe.objects[part.object].name;
            let owner = recipe.movers.iter().position(|m| m.owns(name));
            if let Some(at) = owner {
                let facts = &movers[at];
                pfx.insert("group".into(), json!(facts.group));
                if !part.hidden && !rooted[at] {
                    rooted[at] = true;
                    pfx.extend(mover_json(facts));
                }
            }
            match part.shadow {
                Shadow::Cast => {}
                Shadow::Only => {
                    pfx.insert("shadow".into(), json!("only"));
                }
                Shadow::None => {
                    pfx.insert("shadow".into(), json!("none"));
                }
            }
            (!pfx.is_empty()).then(|| json!({ "pfx": pfx }))
        })
        .collect()
}
