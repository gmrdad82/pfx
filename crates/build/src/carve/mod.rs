pub mod decals;
pub mod grooves;
pub mod hinged;
mod imprint;
pub mod regions;
pub mod split;

pub use imprint::{Frame, View};

use crate::Piece;
use pfx_mesh::Mesh;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Carve {
    pub regions: Vec<regions::Region>,
    pub decals: Vec<decals::Decal>,
    pub grooves: Vec<grooves::Groove>,
    pub hinged: Vec<hinged::Hinged>,
}

#[derive(Clone, Debug, Default)]
pub struct Carved {
    pub pieces: Vec<Piece>,
    pub pivots: BTreeMap<String, [f64; 3]>,
    pub opened: Map<String, Value>,
}

impl Carve {
    pub fn parse(params: &BTreeMap<String, Value>, at: &str) -> Result<Carve, String> {
        let path = |key: &str| {
            if at.is_empty() {
                key.to_string()
            } else {
                format!("{at}.{key}")
            }
        };
        let regions = regions::parse(params.get("regions"), &path("regions"))?;
        let decals = decals::parse(params.get("decals"), &path("decals"))?;
        let grooves = grooves::parse(params.get("grooves"), &path("grooves"))?;
        let hinged = hinged::parse(params.get("hinged"), &path("hinged"), &grooves)?;
        Ok(Carve {
            regions,
            decals,
            grooves,
            hinged,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
            && self.decals.is_empty()
            && self.grooves.is_empty()
            && self.hinged.is_empty()
    }

    pub fn apply(&self, surface: &mut Mesh, name: &str, frame: Frame) -> Result<Carved, String> {
        let mut carved = Carved::default();
        for piece in regions::apply(surface, &self.regions, frame) {
            carved.pieces.push(Piece {
                name: format!("{name}.{}", piece.name),
                mesh: piece.mesh,
            });
        }
        for piece in decals::apply(surface, &self.decals, frame) {
            carved.pieces.push(Piece {
                name: format!("{name}.{}", piece.name),
                mesh: piece.mesh,
            });
        }
        grooves::apply(surface, &self.grooves, frame);
        for ((piece, pivot), panel) in hinged::split(surface, &self.hinged, &self.grooves, frame)?
            .into_iter()
            .zip(&self.hinged)
        {
            let node = format!("{name}.{}", piece.name);
            carved.pivots.insert(node.clone(), pivot.to_array());
            carved.opened.insert(
                panel.name.clone(),
                json!({ "pivot": pivot.to_array(), "axis": panel.axis().to_array() }),
            );
            carved.pieces.push(Piece {
                name: node,
                mesh: piece.mesh,
            });
        }
        Ok(carved)
    }
}
