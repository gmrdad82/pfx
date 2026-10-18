use crate::carve::{Carve, Carved, Frame, split};
use crate::pieces::{self, Found};
use crate::{Error, Generated, Piece};
use pfx_mesh::Morph;
use pfx_recipe::{Kind, MeshEdit, Object};
use serde_json::Value;
use std::collections::BTreeSet;

pub(crate) struct Edited {
    pub generated: Generated,
    pub morphs: Vec<Option<Morph>>,
    pub paints: Vec<Option<String>>,
    pub from: Vec<String>,
    pub kept: Vec<Option<usize>>,
}

fn fail(object: &Object, text: String) -> Error {
    match text.split_once(": ") {
        Some((at, text)) => Error::new(format!("object[{}].{at}", object.name), text),
        None => Error::new(format!("object[{}]", object.name), text),
    }
}

pub(crate) fn edit_of(object: &Object) -> Option<&MeshEdit> {
    match &object.kind {
        Kind::Mesh { edit, .. } | Kind::Library { edit, .. } => {
            Some(edit).filter(|e| !e.is_empty())
        }
        _ => None,
    }
}

struct Out {
    from: String,
    piece: Piece,
    morph: Option<Morph>,
    paint: Option<String>,
    kept: Option<usize>,
    island: Option<usize>,
    part: Option<String>,
}

fn selector(object: &Object, out: &Out) -> Option<String> {
    let node = out
        .from
        .strip_prefix(&format!("{}.", object.name))
        .unwrap_or("");
    let mut steps = vec![node.to_string()];
    steps.extend(out.island.map(|k| format!("#{k}")));
    steps.extend(out.part.clone());
    (steps.len() > 1 || !node.is_empty()).then(|| steps.join("/"))
}

pub(crate) fn apply(
    object: &Object,
    mut generated: Generated,
    morphs: Vec<Option<Morph>>,
    skinned: &[bool],
    found: &Found,
    hidden: &mut BTreeSet<String>,
) -> Result<Edited, Error> {
    let names = |g: &Generated| g.pieces.iter().map(|p| p.name.clone()).collect();
    let selects: Vec<_> = object
        .pieces
        .iter()
        .map(|rule| pieces::select(object, rule))
        .collect();
    let cuts = selects.iter().any(|s| s.cut.is_some());
    let edit = edit_of(object);
    if edit.is_none() && !cuts {
        return Ok(Edited {
            from: names(&generated),
            kept: (0..generated.pieces.len()).map(Some).collect(),
            paints: vec![None; generated.pieces.len()],
            generated,
            morphs,
        });
    }
    let empty = MeshEdit::default();
    let edit = edit.unwrap_or(&empty);
    let carve = Carve::parse(&edit.carve, "").map_err(|t| fail(object, t))?;
    let key = pfx_recipe::CARVE
        .into_iter()
        .find(|k| edit.carve.contains_key(*k))
        .unwrap_or("regions");
    if !carve.is_empty() && morphs.iter().any(Option::is_some) {
        return Err(fail(
            object,
            format!(
                "{key}: the mesh has morph targets, and carving would leave them behind; `split` and `pieces` work on it"
            ),
        ));
    }
    if !carve.is_empty() && skinned.contains(&true) {
        return Err(fail(
            object,
            format!(
                "{key}: the mesh is skinned, and carving would leave its skin behind; `pieces` works on it"
            ),
        ));
    }
    let mut out = Vec::new();
    for (at, (piece, morph)) in std::mem::take(&mut generated.pieces)
        .into_iter()
        .zip(morphs.into_iter().chain(std::iter::repeat(None)))
        .enumerate()
    {
        let parts = if edit.split {
            split::islands(&piece.mesh)
        } else {
            Vec::new()
        };
        if parts.len() < 2 {
            out.push(Out {
                from: piece.name.clone(),
                piece,
                morph,
                paint: None,
                kept: Some(at),
                island: None,
                part: None,
            });
            continue;
        }
        if skinned.get(at).copied().unwrap_or(false) {
            return Err(fail(
                object,
                format!(
                    "split: the node `{}` is skinned, and splitting would leave its skin behind; `pieces` works on it",
                    piece.name
                ),
            ));
        }
        for (k, part) in parts.into_iter().enumerate() {
            let morph = morph.clone().map(|mut m| {
                m.remap(&part.sources);
                m
            });
            out.push(Out {
                from: piece.name.clone(),
                piece: Piece {
                    name: format!("{}.{}", piece.name, k + 1),
                    mesh: part.mesh,
                },
                morph,
                paint: None,
                kept: None,
                island: Some(k + 1),
                part: None,
            });
        }
    }
    if !carve.is_empty() {
        let mut opened = serde_json::Map::new();
        let mut carved_out = Vec::new();
        for mut one in out {
            let Carved {
                pieces,
                pivots,
                opened: panels,
            } = carve
                .apply(&mut one.piece.mesh, &one.piece.name, Frame::WORLD)
                .map_err(|t| fail(object, t))?;
            generated.pivots.extend(pivots);
            opened.extend(panels);
            let base = one.piece.name.clone();
            let (from, island) = (one.from.clone(), one.island);
            if !one.piece.mesh.indices.is_empty() {
                carved_out.push(one);
            }
            for p in pieces.into_iter().filter(|p| !p.mesh.indices.is_empty()) {
                let part = p.name.strip_prefix(&format!("{base}.")).map(str::to_string);
                let paint = carve
                    .regions
                    .iter()
                    .find(|r| part.as_deref() == Some(r.name.as_str()))
                    .and_then(|r| r.material.clone());
                carved_out.push(Out {
                    from: from.clone(),
                    piece: p,
                    morph: None,
                    paint,
                    kept: None,
                    island,
                    part,
                });
            }
        }
        out = carved_out;
        if !opened.is_empty() {
            generated
                .facts
                .insert("hinged".into(), Value::Object(opened));
        }
    }
    let source = |i: usize, from: &str| -> Option<u8> {
        let cut = selects[i].cut.as_ref()?;
        if cut.node.is_empty() {
            return (from == object.name).then_some(1);
        }
        found
            .sources
            .get(i)?
            .iter()
            .find(|(name, _)| name == from)
            .map(|(_, rank)| *rank)
    };
    let mut reached = vec![false; object.pieces.len()];
    let library = matches!(object.kind, Kind::Library { .. });
    let mut edited = Edited {
        generated,
        morphs: Vec::new(),
        paints: Vec::new(),
        from: Vec::new(),
        kept: Vec::new(),
    };
    let listed: Vec<String> = out.iter().filter_map(|o| selector(object, o)).collect();
    for one in out {
        let depth = usize::from(one.island.is_some()) + usize::from(one.part.is_some());
        let mut paint: Option<((usize, u8), String)> = one.paint.clone().map(|p| ((depth, 0), p));
        let mut hide: Option<((usize, u8), bool)> = None;
        for (i, rule) in object.pieces.iter().enumerate() {
            let Some(cut) = selects[i].cut.as_ref() else {
                continue;
            };
            let Some(kind) = source(i, &one.from) else {
                continue;
            };
            if cut.island.is_some() && cut.island != one.island
                || cut.part.is_some() && cut.part != one.part.as_deref()
            {
                continue;
            }
            reached[i] = true;
            let rank = (if cut.part.is_some() { depth } else { 1 }, kind);
            if let Some(name) = &rule.material
                && paint.as_ref().is_none_or(|(best, _)| rank > *best)
            {
                paint = Some((rank, name.clone()));
            }
            if let Some(value) = rule.hidden
                && hide.is_none_or(|(best, _)| rank > best)
            {
                hide = Some((rank, value));
            }
        }
        let name = one.piece.name.clone();
        match hide.map(|(_, value)| value) {
            Some(true) if !library => continue,
            Some(true) => {
                hidden.insert(name);
            }
            Some(false) => {
                hidden.remove(&name);
            }
            None if name != one.from && hidden.contains(&one.from) => {
                hidden.insert(name);
            }
            None => {}
        }
        edited.paints.push(paint.map(|(_, p)| p));
        edited.from.push(one.from);
        edited.kept.push(one.kept);
        edited.morphs.push(one.morph);
        edited.generated.pieces.push(one.piece);
    }
    let missing = object.pieces.iter().enumerate().find(|&(i, _)| {
        selects[i].cut.is_some() && !reached[i] && !found.used.get(i).copied().unwrap_or(false)
    });
    if let Some((_, rule)) = missing {
        let select = &rule.select;
        return Err(Error::new(
            format!("object[{}].pieces.{select}", object.name),
            format!(
                "the object builds no piece `{select}`, expected one of: {}",
                listed.join(", ")
            ),
        ));
    }
    Ok(edited)
}
