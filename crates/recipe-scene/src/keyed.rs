use pfx_build::{Built, Rate};
use pfx_glb::DMat4;
use pfx_live::frame::Matrix;
use std::collections::BTreeMap;

struct Bound {
    track: usize,
    undo: Option<DMat4>,
    slot: usize,
}

pub struct Keyed {
    fps: Rate,
    bound: Vec<Bound>,
    names: Vec<String>,
    models: Vec<Matrix>,
}

impl Keyed {
    pub fn of(built: &Built, fps: Rate) -> Keyed {
        let mut parts: BTreeMap<&str, Vec<usize>> = built
            .tracks
            .iter()
            .filter(|track| track.camera.is_none() && !track.keys.is_empty())
            .map(|track| (track.node.as_str(), Vec::new()))
            .collect();
        let mut bound = Vec::new();
        let mut names = Vec::new();
        if parts.is_empty() {
            return Keyed {
                fps,
                bound,
                models: Vec::new(),
                names,
            };
        }
        for (at, part) in built.parts.iter().enumerate() {
            if let Some(found) = parts.get_mut(part.node.name.as_str()) {
                found.push(at);
            }
        }
        let mut slots: BTreeMap<&str, usize> = BTreeMap::new();
        for (at, track) in built.tracks.iter().enumerate() {
            if track.camera.is_some() {
                continue;
            }
            if track.keys.is_empty() {
                continue;
            }
            let Some(found) = parts.get(track.node.as_str()) else {
                continue;
            };
            for &index in found {
                let node = &built.parts[index].node;
                let slot = *slots.entry(node.name.as_str()).or_insert_with(|| {
                    names.push(node.name.clone());
                    names.len() - 1
                });
                bound.push(Bound {
                    track: at,
                    undo: node.keep_transform.then(|| node.transform.inverse()),
                    slot,
                });
            }
        }
        Keyed {
            fps,
            bound,
            models: vec![[[0.0; 4]; 4]; names.len()],
            names,
        }
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn sample(&mut self, built: &Built, time: f64) -> &[Matrix] {
        let mut track = usize::MAX;
        let mut world = DMat4::IDENTITY;
        for bound in &self.bound {
            if bound.track != track {
                track = bound.track;
                world = pfx_build::matrix(&pfx_build::sample(
                    &built.tracks[track].keys,
                    self.fps,
                    time,
                ));
            }
            let model = match bound.undo {
                Some(undo) => world * undo,
                None => world,
            };
            self.models[bound.slot] = model
                .to_cols_array_2d()
                .map(|column| column.map(|v| v as f32));
        }
        &self.models
    }

    pub fn posed(&mut self, built: &Built, time: f64) -> BTreeMap<String, Matrix> {
        let models = self.sample(built, time).to_vec();
        self.names.iter().cloned().zip(models).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_build::{Registry, build_with};
    use std::path::Path;

    const SCENE: &str = r#"recipe = 1
name = "t"

[timeline]
fps = 12
frames = 24

[[object]]
name = "car"
kind = "box"
size = [2.0, 1.0, 4.0]
at = [5.0, 5.0, 5.0]
keys = { at = [[0.5, [0.0, 0.0, 0.0]], [1.5, [0.0, 0.0, 10.0]]], rotate = [[0.0, [0.0, 0.0, 0.0]], [1.0, [0.0, 90.0, 0.0]]] }

[[object]]
name = "wing"
kind = "box"
size = [1.0, 0.1, 1.0]
parent = "car"
at = [0.0, 1.0, 0.0]

[[object]]
name = "kerb"
kind = "box"
size = [1.0, 1.0, 1.0]
at = [0.0, 0.0, 5.0]
keys = { at = [[0.0, [0.0, 0.0, 5.0]], [2.0, [3.0, 1.0, 5.0]]], rotate = [[0.0, [10.0, 0.0, 0.0]], [2.0, [10.0, 0.0, 200.0]]] }

[[object]]
name = "stone"
kind = "box"
size = [0.2, 0.1, 0.2]
hidden = true

[[object]]
name = "ground"
kind = "plane"
size = [6.0, 4.0]

[[scatter]]
name = "stones"
object = "stone"
on = "ground"
spacing = 0.6
"#;

    fn scanned(built: &Built, fps: Rate, time: f64) -> BTreeMap<String, Matrix> {
        let mut posed = BTreeMap::new();
        for track in built.tracks.iter().filter(|track| track.camera.is_none()) {
            if track.keys.is_empty() {
                continue;
            }
            let world = pfx_build::matrix(&pfx_build::sample(&track.keys, fps, time));
            for part in built
                .parts
                .iter()
                .filter(|part| part.node.name == track.node)
            {
                let model = if part.node.keep_transform {
                    world * part.node.transform.inverse()
                } else {
                    world
                };
                posed.insert(
                    part.node.name.clone(),
                    model
                        .to_cols_array_2d()
                        .map(|column| column.map(|v| v as f32)),
                );
            }
        }
        posed
    }

    #[test]
    fn one_pass_over_the_keys_writes_the_same_bits_as_a_scan_per_track() {
        let recipe = pfx_recipe::parse(SCENE).unwrap();
        let built = build_with(&recipe, Path::new("."), &Registry::new()).unwrap();
        let fps = Rate::whole(12);
        let mut keyed = Keyed::of(&built, fps);
        assert_eq!(keyed.names(), ["car", "wing", "kerb"]);
        for frame in [-3, 0, 1, 7, 12, 18, 24, 40] {
            let time = f64::from(frame) / 12.0 + 0.013;
            let want = scanned(&built, fps, time);
            let got = keyed.posed(&built, time);
            assert_eq!(got.len(), want.len());
            for (name, model) in &want {
                let bits = |m: &Matrix| m.map(|column| column.map(f32::to_bits));
                assert_eq!(bits(&got[name]), bits(model), "{name} at {time}");
            }
        }
    }
}
