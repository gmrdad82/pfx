use super::imprint::{self, Facing, Frame, View, Work, key};
use super::regions::POINTS;
use crate::Piece;
use glam::{DVec2, DVec3};
use pfx_mesh::{Mesh, Slot};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const OFFSET: f64 = 0.001;

const FIELDS: [&str; 6] = ["name", "view", "curve", "material", "offset", "inset"];

#[derive(Clone, Debug, PartialEq)]
pub struct Decal {
    pub name: String,
    pub view: View,
    pub curve: Vec<DVec2>,
    pub material: Option<String>,
    pub offset: f64,
    pub inset: f64,
}

pub fn parse(value: Option<&Value>, at: &str) -> Result<Vec<Decal>, String> {
    let decals = imprint::list(value, at)?
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            let at = format!("{at}.{i}");
            imprint::known(t, &at, &FIELDS)?;
            let name = imprint::name(t, &at)?;
            let curve = imprint::polygon(&imprint::curve(t, &at, 3)?);
            if curve.len() > POINTS {
                return Err(format!(
                    "{at}.curve: decal `{name}` has {} points, at most {POINTS}; thin the outline",
                    curve.len()
                ));
            }
            Ok(Decal {
                name,
                view: imprint::view(t, &at)?,
                curve,
                material: imprint::text(t, &at, "material")?,
                offset: imprint::number(t, &at, "offset", Some(OFFSET), 0.0)?,
                inset: imprint::number(t, &at, "inset", Some(0.0), 0.0)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    imprint::unique(decals.iter().map(|d| d.name.as_str()), at)?;
    Ok(decals)
}

pub fn apply(mesh: &mut Mesh, decals: &[Decal], frame: Frame) -> Vec<Piece> {
    if decals.is_empty() {
        return Vec::new();
    }
    let mut w = Work::from_mesh(mesh);
    let mut pieces = Vec::new();
    for d in decals {
        let facing = Facing::all(d.view, frame);
        imprint::imprint(&mut w, facing, &[(&d.curve, true)]);
        let picked = w.select(facing, &d.curve);
        let tris: Vec<([u32; 3], usize)> = picked
            .iter()
            .enumerate()
            .filter(|(_, p)| **p)
            .map(|(t, _)| (w.tris[t], w.mat[t]))
            .collect();
        let rim = if d.inset > 0.0 {
            w.cut(&picked, d.inset)
        } else {
            BTreeMap::new()
        };
        let used: BTreeSet<u32> = tris.iter().flat_map(|(t, _)| *t).collect();
        let base: BTreeMap<u32, (DVec3, DVec3)> = used
            .iter()
            .map(|&v| {
                let p = w.pos[v as usize];
                let n = w.nrm[v as usize].normalize_or_zero();
                let at = if d.inset > 0.0 {
                    rim.get(&key(p)).copied().unwrap_or(p - n * d.inset)
                } else {
                    p
                };
                (v, (at, n))
            })
            .collect();
        let uv = Box2::of(&d.curve);
        let mut out = Mesh {
            uvs: Some(Vec::new()),
            ..Mesh::default()
        };
        let mut index = BTreeMap::new();
        for (&v, &(at, n)) in &base {
            index.insert(v, out.positions.len() as u32);
            out.positions.push((at + n * d.offset).as_vec3().to_array());
            out.normals.push(n.as_vec3().to_array());
            if let Some(uvs) = out.uvs.as_mut() {
                uvs.push(uv.place(facing.project(w.pos[v as usize])));
            }
        }
        let mut groups: BTreeMap<usize, Vec<[u32; 3]>> = BTreeMap::new();
        for (t, m) in &tris {
            let m = if d.material.is_some() { 0 } else { *m };
            groups.entry(m).or_default().push(*t);
        }
        for (m, list) in groups {
            let start = out.indices.len() as u32;
            for t in list {
                out.indices.extend(t.map(|v| index[&v]));
            }
            let material = d.material.clone().or_else(|| w.materials[m].clone());
            if let Some(material) = material {
                out.slots.push(Slot {
                    material,
                    start,
                    count: out.indices.len() as u32 - start,
                });
            }
        }
        if d.inset > 0.0 {
            let floor: BTreeMap<u32, u32> = base
                .iter()
                .map(|(&v, &(at, n))| (v, w.push_vertex(&[(v, 1.0)], at, Some(n))))
                .collect();
            for (t, m) in &tris {
                w.tris.push(t.map(|v| floor[&v]));
                w.mat.push(*m);
            }
        }
        pieces.push(Piece {
            name: d.name.clone(),
            mesh: out,
        });
    }
    *mesh = w.to_mesh(&vec![true; w.tris.len()]);
    pieces
}

struct Box2 {
    lo: DVec2,
    size: DVec2,
}

impl Box2 {
    fn of(curve: &[DVec2]) -> Box2 {
        let (lo, hi) = curve
            .iter()
            .fold((DVec2::INFINITY, DVec2::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(*p), hi.max(*p))
            });
        Box2 {
            lo,
            size: (hi - lo).max(DVec2::splat(1e-12)),
        }
    }

    fn place(&self, q: DVec2) -> [f32; 2] {
        let t = (q - self.lo) / self.size;
        [t.x as f32, (1.0 - t.y) as f32]
    }
}

#[cfg(test)]
mod tests {
    use super::super::imprint::test::{area, body, open_edges, point};
    use super::*;
    use serde_json::json;

    const OUTLINE: [[f64; 2]; 5] = [
        [-1.0, -0.2],
        [0.6, -0.25],
        [0.9, 0.2],
        [0.1, 0.4],
        [-0.8, 0.3],
    ];

    fn decals(v: Value) -> Vec<Decal> {
        parse(Some(&v), "decals").unwrap()
    }

    fn shoelace(p: &[[f64; 2]]) -> f64 {
        let n = p.len();
        0.5 * (0..n)
            .map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1])
            .sum::<f64>()
            .abs()
    }

    #[test]
    fn a_decal_lies_on_the_body_at_its_offset_and_leaves_the_body_whole() {
        let original = body();
        let ds = decals(
            json!([{ "name": "badge", "view": "side", "material": "badge",
            "offset": 0.004, "curve": OUTLINE }]),
        );
        let mut mesh = original.clone();
        let pieces = apply(&mut mesh, &ds, Frame::WORLD);
        assert_eq!(pieces.len(), 1);
        let badge = &pieces[0].mesh;
        assert_eq!(pieces[0].name, "badge");
        assert!(badge.slots.iter().all(|s| s.material == "badge"));
        for i in 0..badge.positions.len() as u32 {
            assert!((point(badge, i).x.abs() - 1.004).abs() < 2e-4);
        }
        let flat = 2.0 * shoelace(&OUTLINE);
        assert!(
            (area(badge) - flat).abs() < 2e-3 * flat,
            "{} {flat}",
            area(badge)
        );
        assert!((area(&mesh) - area(&original)).abs() < 1e-5);
        assert!(open_edges(&[&mesh]).is_empty());
        let uvs = badge.uvs.as_ref().unwrap();
        assert!(
            uvs.iter()
                .flatten()
                .all(|c| (-1e-5..=1.0 + 1e-5).contains(c))
        );
        let mut again = original.clone();
        assert_eq!(apply(&mut again, &ds, Frame::WORLD)[0].mesh, *badge);
    }

    #[test]
    fn an_inset_decal_sits_in_a_closed_recess() {
        let original = body();
        let ds = decals(json!([{ "name": "lamp", "view": "side", "inset": 0.03,
            "offset": 0.01, "curve": OUTLINE }]));
        let mut mesh = original.clone();
        let pieces = apply(&mut mesh, &ds, Frame::WORLD);
        assert!(open_edges(&[&mesh]).is_empty());
        let lamp = &pieces[0].mesh;
        for i in 0..lamp.positions.len() as u32 {
            assert!(
                (point(lamp, i).x.abs() - 0.98).abs() < 2e-4,
                "{}",
                point(lamp, i).x
            );
        }
        let floor = (0..mesh.positions.len() as u32)
            .filter(|&i| (point(&mesh, i).x.abs() - 0.97).abs() < 2e-4)
            .count();
        assert!(floor >= lamp.positions.len());
        assert!(area(&mesh) > area(&original));
    }

    #[test]
    fn a_decal_follows_a_curved_body() {
        let ball = pfx_mesh::sphere(1.0, 48, 24);
        let circle: Vec<[f64; 2]> = (0..40)
            .map(|i| {
                let a = i as f64 / 40.0 * std::f64::consts::TAU;
                [0.4 * a.cos(), 0.4 * a.sin()]
            })
            .collect();
        let ds = decals(json!([{ "name": "lens", "view": "front", "inset": 0.02,
            "offset": 0.005, "curve": circle }]));
        let mut mesh = ball.clone();
        let pieces = apply(&mut mesh, &ds, Frame::WORLD);
        let lens = &pieces[0].mesh;
        let sag = 1.0 - (std::f64::consts::PI / 48.0).cos();
        let (mut near, mut far) = (f64::INFINITY, f64::NEG_INFINITY);
        for i in 0..lens.positions.len() as u32 {
            let p = point(lens, i);
            assert!(p.z > 0.0);
            let r = p.length();
            assert!(r > 0.985 - 2.0 * sag && r < 0.985 + 1e-5, "{r}");
            near = near.min(p.z);
            far = far.max(p.z);
        }
        assert!(far - near > 0.07, "{near} {far}");
        assert_eq!(open_edges(&[&mesh]), open_edges(&[&ball]));
    }
}
