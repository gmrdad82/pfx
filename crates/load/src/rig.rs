use pfx_core::anim::{Channel, Clip, Joint, MAX_JOINTS, Rig, Skeleton, Trs};

use crate::mesh::Mesh;

#[derive(Clone, Debug, PartialEq)]
pub struct SkinRig {
    pub rig: Rig,
    pub remap: Vec<u16>,
}

impl SkinRig {
    pub fn joints(&self, raw: &[[u16; 4]], weights: &[[f32; 4]]) -> Result<Vec<[u16; 4]>, String> {
        raw.iter()
            .zip(weights)
            .map(|(joints, weights)| {
                let mut out = [0u16; 4];
                for slot in 0..4 {
                    if weights[slot] == 0.0 {
                        continue;
                    }
                    out[slot] = *self.remap.get(joints[slot] as usize).ok_or_else(|| {
                        format!(
                            "a vertex names joint {} of a skin of {}",
                            joints[slot],
                            self.remap.len()
                        )
                    })?;
                }
                Ok(out)
            })
            .collect()
    }
}

impl Mesh {
    pub fn rig(&self, skin: u32) -> Result<SkinRig, String> {
        let found = self
            .skins
            .get(skin as usize)
            .ok_or_else(|| format!("mesh: no skin {skin}"))?;
        if found.joints.is_empty() {
            return Err(format!("mesh: skin {skin} has no joints"));
        }
        if found.joints.len() > MAX_JOINTS {
            return Err(format!(
                "mesh: skin {skin} has {} joints, over {MAX_JOINTS}",
                found.joints.len()
            ));
        }
        let depth = |node: u32| {
            let mut depth = 0usize;
            let mut at = self.nodes[node as usize].parent;
            while let Some(parent) = at {
                depth += 1;
                at = self.nodes[parent as usize].parent;
                if depth > self.nodes.len() {
                    break;
                }
            }
            depth
        };
        let mut order: Vec<usize> = (0..found.joints.len()).collect();
        order.sort_by_key(|&slot| (depth(found.joints[slot]), slot));
        let mut remap = vec![0u16; found.joints.len()];
        for (new, &old) in order.iter().enumerate() {
            remap[old] = new as u16;
        }
        let index_of = |node: u32| found.joints.iter().position(|&joint| joint == node);
        let mut root_parent: Option<Option<u32>> = None;
        let mut joints = Vec::with_capacity(order.len());
        let mut binds = Vec::with_capacity(order.len());
        for &old in &order {
            let node_index = found.joints[old];
            let node = &self.nodes[node_index as usize];
            let mut parent = None;
            let mut at = node.parent;
            while let Some(up) = at {
                if let Some(slot) = index_of(up) {
                    parent = Some(remap[slot] as usize);
                    break;
                }
                at = self.nodes[up as usize].parent;
            }
            if parent.is_none() {
                match root_parent {
                    None => root_parent = Some(node.parent),
                    Some(other) if other != node.parent => {
                        return Err(format!(
                            "mesh: skin {skin} has root joints under different nodes"
                        ));
                    }
                    Some(_) => {}
                }
            }
            joints.push(Joint {
                name: if node.name.is_empty() {
                    format!("joint{node_index}")
                } else {
                    node.name.clone()
                },
                parent,
                rest: node.trs,
            });
            binds.push(found.inverse_binds[old]);
        }
        let root = root_parent
            .flatten()
            .and_then(|parent| self.world(parent))
            .unwrap_or(pfx_core::anim::math::IDENTITY);
        let skeleton = Skeleton::rooted(joints, binds, root)
            .map_err(|error| format!("mesh: skin {skin}: {error}"))?;
        let clips = self.clips(|node| index_of(node).map(|slot| remap[slot] as usize))?;
        let rig = Rig::new(skeleton, clips).map_err(|error| format!("mesh: {error}"))?;
        Ok(SkinRig { rig, remap })
    }
}

impl Mesh {
    fn clips(&self, joint_of: impl Fn(u32) -> Option<usize>) -> Result<Vec<Clip>, String> {
        let mut clips = Vec::new();
        for animation in &self.animations {
            let channels: Vec<Channel> = animation
                .channels
                .iter()
                .filter_map(|channel| {
                    Some(Channel {
                        joint: joint_of(channel.node)?,
                        property: channel.property,
                        interpolation: channel.interpolation,
                        times: channel.times.clone(),
                        values: channel.values.clone(),
                    })
                })
                .collect();
            if channels.is_empty() {
                continue;
            }
            clips.push(
                Clip::new(animation.name.clone(), channels)
                    .map_err(|error| format!("mesh: {error}"))?,
            );
        }
        Ok(clips)
    }
}

impl Mesh {
    pub fn node_rig(&self, walked: &[(u32, bool)]) -> Result<Option<Rig>, String> {
        if self.animations.is_empty() || walked.is_empty() {
            return Ok(None);
        }
        if walked.len() > MAX_JOINTS {
            return Err(format!(
                "mesh: {} animated nodes, over {MAX_JOINTS}",
                walked.len()
            ));
        }
        let joint_of = |node: u32| {
            walked
                .iter()
                .position(|(walked, own)| *walked == node && *own)
        };
        let joints: Vec<Joint> = walked
            .iter()
            .enumerate()
            .map(|(index, &(node, own))| {
                let found = &self.nodes[node as usize];
                let parent = if own {
                    found
                        .parent
                        .and_then(|parent| walked[..index].iter().position(|(n, _)| *n == parent))
                } else {
                    None
                };
                Joint {
                    name: if found.name.is_empty() {
                        format!("node{node}")
                    } else {
                        found.name.clone()
                    },
                    parent,
                    rest: if own { found.trs } else { Trs::IDENTITY },
                }
            })
            .collect();
        let binds = vec![pfx_core::anim::math::IDENTITY; joints.len()];
        let skeleton = Skeleton::new(joints, binds).map_err(|error| format!("mesh: {error}"))?;
        let clips = self.clips(joint_of)?;
        if clips.is_empty() {
            return Ok(None);
        }
        Rig::new(skeleton, clips)
            .map(Some)
            .map_err(|error| format!("mesh: {error}"))
    }
}
