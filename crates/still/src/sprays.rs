use pfx_build::{Built, Emitted, Rate};
use pfx_load::scene::sprays::Sprays;
use pfx_trace::sprites::Puff;

use pfx_live::frame::Matrix;

use crate::staging::IDENTITY;

pub fn puffs(emitted: &[Emitted], built: &Built, fps: Rate, time: f64) -> Vec<Puff> {
    if emitted.is_empty() {
        return Vec::new();
    }
    let mut sprays = Sprays::of_emitters(
        emitted
            .iter()
            .map(|emitter| {
                pfx_core::sim::particles::Emitter::new(emitter.recipe.clone(), emitter.at)
            })
            .collect(),
    );
    sprays.seek(time as f32, |t, out| {
        out.extend(
            emitted
                .iter()
                .map(|emitter| node_world(built, fps, emitter.node.as_deref(), t)),
        );
    });
    let mut puffs = Vec::with_capacity(sprays.live());
    sprays.each(|recipe, sprites| {
        puffs.extend(sprites.iter().map(|&sprite| Puff {
            sprite,
            stretch: recipe.stretch,
            blend: recipe.blend,
            lit: recipe.lit,
        }));
    });
    puffs
}

fn single(columns: [[f64; 4]; 4]) -> Matrix {
    columns.map(|column| column.map(|v| v as f32))
}

pub fn node_world(built: &Built, fps: Rate, node: Option<&str>, time: f32) -> Matrix {
    let Some(node) = node else {
        return IDENTITY;
    };
    if let Some(track) = built
        .tracks
        .iter()
        .find(|track| track.camera.is_none() && track.node == node && !track.keys.is_empty())
    {
        return single(
            pfx_build::matrix(&pfx_build::sample(&track.keys, fps, f64::from(time)))
                .to_cols_array_2d(),
        );
    }
    built
        .parts
        .iter()
        .find(|part| part.node.name == node)
        .map_or(IDENTITY, |part| {
            if part.node.keep_transform {
                single(part.node.transform.to_cols_array_2d())
            } else {
                IDENTITY
            }
        })
}
