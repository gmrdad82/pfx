use super::imprint::{Key, key};
use glam::Vec3;
use pfx_mesh::{Mesh, Slot};
use std::collections::BTreeMap;

pub struct Island {
    pub mesh: Mesh,
    pub sources: Vec<u32>,
}

fn find(parent: &mut [u32], mut i: u32) -> u32 {
    while parent[i as usize] != i {
        let up = parent[parent[i as usize] as usize];
        parent[i as usize] = up;
        i = up;
    }
    i
}

pub fn islands(mesh: &Mesh) -> Vec<Island> {
    let mut welded: BTreeMap<Key, u32> = BTreeMap::new();
    let spot: Vec<u32> = mesh
        .positions
        .iter()
        .map(|&p| {
            let next = welded.len() as u32;
            *welded.entry(key(Vec3::from(p).as_dvec3())).or_insert(next)
        })
        .collect();
    let mut parent: Vec<u32> = (0..welded.len() as u32).collect();
    for t in mesh.indices.chunks_exact(3) {
        for &v in &t[1..] {
            let a = find(&mut parent, spot[t[0] as usize]);
            let b = find(&mut parent, spot[v as usize]);
            parent[a.max(b) as usize] = a.min(b);
        }
    }
    let count = mesh.indices.len() / 3;
    let mut group: BTreeMap<u32, usize> = BTreeMap::new();
    let mut of = Vec::with_capacity(count);
    for t in mesh.indices.chunks_exact(3) {
        let root = find(&mut parent, spot[t[0] as usize]);
        let next = group.len();
        of.push(*group.entry(root).or_insert(next));
    }
    let mut slot_of = vec![None; count];
    for (s, slot) in mesh.slots.iter().enumerate() {
        let end = ((slot.start + slot.count) as usize / 3).min(count);
        for at in slot_of.iter_mut().take(end).skip(slot.start as usize / 3) {
            *at = Some(s);
        }
    }
    let mut out: Vec<Island> = (0..group.len())
        .map(|_| Island {
            mesh: Mesh {
                uvs: mesh.uvs.as_ref().map(|_| Vec::new()),
                uvs1: mesh.uvs1.as_ref().map(|_| Vec::new()),
                ..Mesh::default()
            },
            sources: Vec::new(),
        })
        .collect();
    let mut maps: Vec<BTreeMap<u32, u32>> = vec![BTreeMap::new(); out.len()];
    let order = (0..mesh.slots.len()).map(Some).chain([None]);
    for s in order {
        let start: Vec<u32> = out.iter().map(|i| i.mesh.indices.len() as u32).collect();
        for t in (0..count).filter(|&t| slot_of[t] == s) {
            let island = &mut out[of[t]];
            let map = &mut maps[of[t]];
            for &v in &mesh.indices[t * 3..t * 3 + 3] {
                let at = *map.entry(v).or_insert_with(|| {
                    let i = v as usize;
                    island.mesh.positions.push(mesh.positions[i]);
                    island
                        .mesh
                        .normals
                        .push(mesh.normals.get(i).copied().unwrap_or([0.0; 3]));
                    if let (Some(o), Some(u)) = (island.mesh.uvs.as_mut(), mesh.uvs.as_ref()) {
                        o.push(u[i]);
                    }
                    if let (Some(o), Some(u)) = (island.mesh.uvs1.as_mut(), mesh.uvs1.as_ref()) {
                        o.push(u[i]);
                    }
                    island.sources.push(v);
                    island.sources.len() as u32 - 1
                });
                island.mesh.indices.push(at);
            }
        }
        if let Some(s) = s {
            for (island, start) in out.iter_mut().zip(start) {
                let count = island.mesh.indices.len() as u32 - start;
                if count > 0 {
                    island.mesh.slots.push(Slot {
                        material: mesh.slots[s].material.clone(),
                        start,
                        count,
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(at: [f32; 3], material: &str) -> Mesh {
        let mut m = pfx_mesh::grid([1.0, 1.0], [2, 2], None);
        let [x, y, z] = at;
        for p in &mut m.positions {
            *p = [p[0] + x, p[1] + y, p[2] + z];
        }
        m.set_material(material);
        m
    }

    #[test]
    fn touching_faces_stay_one_piece_and_apart_ones_split_in_order() {
        let mut whole = cube([0.0, 0.0, 0.0], "a");
        whole.append(&cube([5.0, 0.0, 0.0], "b"));
        whole.append(&cube([0.0, 0.0, 9.0], "a"));
        let parts = islands(&whole);
        assert_eq!(parts.len(), 3);
        assert_eq!(
            parts.iter().map(|p| p.mesh.indices.len()).sum::<usize>(),
            whole.indices.len()
        );
        assert_eq!(parts[1].mesh.slots[0].material, "b");
        assert!(parts[1].mesh.positions.iter().all(|p| p[0] > 4.0));
        assert!(parts[2].mesh.positions.iter().all(|p| p[2] > 8.0));
        for part in &parts {
            for (at, &from) in part.sources.iter().enumerate() {
                assert_eq!(part.mesh.positions[at], whole.positions[from as usize]);
            }
        }
    }

    #[test]
    fn a_seam_of_split_vertices_does_not_split_a_piece() {
        let mut m = cube([0.0, 0.0, 0.0], "a");
        let n = m.positions.len() as u32;
        let copy: Vec<[f32; 3]> = m.positions.clone();
        m.positions.extend(copy);
        m.normals.extend(m.normals.clone());
        if let Some(uv) = m.uvs.as_mut() {
            uv.extend(uv.clone());
        }
        let half = m.indices.len() / 2;
        for i in &mut m.indices[half..] {
            *i += n;
        }
        assert_eq!(islands(&m).len(), 1);
    }
}
