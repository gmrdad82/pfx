use std::collections::BTreeMap;

use pfx_core::sim::particles::{Emitter, IDENTITY, Matrix, Recipe, STEP, Sprite};

use super::{Attach, Particles, Scene};

pub const CATCH_UP: f32 = 120.0;
pub const MAX_SPAN: f32 = 0.1;

#[derive(Clone, Debug)]
struct Link {
    object: Option<usize>,
    model: Matrix,
    attach: Option<Attach>,
}

impl Link {
    fn world(&self, objects: Option<&[Matrix]>, palettes: &Palettes) -> Matrix {
        let Some(attach) = &self.attach else {
            return IDENTITY;
        };
        let object = self
            .object
            .and_then(|index| objects.and_then(|objects| objects.get(index)))
            .copied()
            .unwrap_or(self.model);
        let palette = self
            .object
            .and_then(|index| palettes.get(&index))
            .map(Vec::as_slice);
        attach.world(object, palette)
    }
}

pub type Palettes = BTreeMap<usize, Vec<Matrix>>;

#[derive(Clone, Debug, Default)]
pub struct Sprays {
    particles: Vec<Particles>,
    links: Vec<Link>,
    emitters: Vec<Emitter>,
    steps: u64,
    sprites: Vec<Sprite>,
    last: Option<f32>,
    span: f32,
}

fn target(time: f32) -> u64 {
    (time.max(0.0) / STEP + 1e-3).floor() as u64
}

fn lerp(a: &Matrix, b: &Matrix, k: f32) -> Matrix {
    std::array::from_fn(|c| std::array::from_fn(|r| a[c][r] + (b[c][r] - a[c][r]) * k))
}

impl Sprays {
    pub fn of(scene: &Scene) -> Self {
        let particles: Vec<Particles> = scene.particles.values().cloned().collect();
        let links = particles
            .iter()
            .map(|particles| {
                let object = particles
                    .attach
                    .as_ref()
                    .and_then(|attach| attach.index(scene));
                Link {
                    object,
                    model: object.map_or(IDENTITY, |index| scene.objects[index].model),
                    attach: particles.attach.clone(),
                }
            })
            .collect();
        let emitters = particles
            .iter()
            .map(|particles| Emitter::new(particles.recipe.clone(), particles.at))
            .collect();
        Self {
            particles,
            links,
            emitters,
            steps: 0,
            sprites: Vec::new(),
            last: None,
            span: 0.0,
        }
    }

    pub fn of_emitters(emitters: Vec<Emitter>) -> Self {
        Self {
            emitters,
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.emitters.is_empty()
    }

    pub fn same(&self, scene: &Scene) -> bool {
        self.particles.len() == scene.particles.len()
            && self.particles.iter().eq(scene.particles.values())
            && self.links.iter().all(|link| {
                link.object.is_none_or(|index| {
                    scene
                        .objects
                        .get(index)
                        .is_some_and(|object| object.model == link.model)
                })
            })
    }

    pub fn live(&self) -> usize {
        self.emitters.iter().map(Emitter::live).sum()
    }

    pub fn dropped(&self) -> u64 {
        self.emitters.iter().map(Emitter::dropped).sum()
    }

    pub fn reset(&mut self) {
        for emitter in &mut self.emitters {
            emitter.reset();
        }
        self.steps = 0;
        self.last = None;
        self.span = 0.0;
    }

    pub fn parents(&self, objects: Option<&[Matrix]>, palettes: &Palettes, out: &mut Vec<Matrix>) {
        out.clear();
        out.extend(self.links.iter().map(|link| link.world(objects, palettes)));
    }

    pub fn seek(&mut self, time: f32, mut pose: impl FnMut(f32, &mut Vec<Matrix>)) {
        if self.emitters.is_empty() || !time.is_finite() {
            return;
        }
        let span = self
            .last
            .map(|last| time - last)
            .filter(|span| (0.0..=MAX_SPAN).contains(span))
            .unwrap_or(0.0);
        let goal = target(time);
        let window = (CATCH_UP / STEP) as u64;
        if goal < self.steps || goal - self.steps > window {
            self.reset();
            self.steps = goal.saturating_sub(window);
            if self.steps > 0 {
                for emitter in &mut self.emitters {
                    emitter.skip_prewarm();
                }
            }
        }
        self.last = Some(time);
        self.span = span;
        let count = self.emitters.len();
        let mut from = Vec::with_capacity(count);
        if self.steps == goal {
            if self.steps == 0 {
                pose(0.0, &mut from);
                from.resize(count, IDENTITY);
                for (emitter, at) in self.emitters.iter_mut().zip(&from) {
                    emitter.warm(*at);
                }
            }
            return;
        }
        let mut to = Vec::with_capacity(count);
        pose(self.steps as f32 * STEP, &mut from);
        from.resize(count, IDENTITY);
        while self.steps < goal {
            to.clear();
            pose((self.steps + 1) as f32 * STEP, &mut to);
            to.resize(count, IDENTITY);
            for (emitter, (from, to)) in self.emitters.iter_mut().zip(from.iter().zip(&to)) {
                emitter.step(STEP, *from, *to);
            }
            std::mem::swap(&mut from, &mut to);
            self.steps += 1;
        }
    }

    pub fn seek_posed(&mut self, time: f32, mut objects: impl FnMut(f32) -> Option<Vec<Matrix>>) {
        self.seek_rigged(time, |t| objects(t).map(|posed| (posed, Palettes::new())));
    }

    pub fn seek_rigged(
        &mut self,
        time: f32,
        mut posing: impl FnMut(f32) -> Option<(Vec<Matrix>, Palettes)>,
    ) {
        let links = std::mem::take(&mut self.links);
        let empty = Palettes::new();
        self.seek(time, |t, out| {
            let posed = posing(t);
            let (objects, palettes) = match &posed {
                Some((objects, palettes)) => (Some(objects.as_slice()), palettes),
                None => (None, &empty),
            };
            out.extend(links.iter().map(|link| link.world(objects, palettes)));
        });
        self.links = links;
    }

    pub fn seek_between(&mut self, span: [f32; 2], before: &[Matrix], after: &[Matrix]) {
        let [t0, t1] = span;
        let width = t1 - t0;
        self.seek(t1, |t, out| {
            let k = if width > 0.0 {
                ((t - t0) / width).clamp(0.0, 1.0)
            } else {
                1.0
            };
            out.extend(before.iter().zip(after).map(|(a, b)| lerp(a, b, k)));
        });
    }

    pub fn span(&self) -> f32 {
        self.span
    }

    pub fn emitters(&self) -> &[Emitter] {
        &self.emitters
    }

    pub fn each(&mut self, mut visit: impl FnMut(&Recipe, &[Sprite])) {
        for emitter in &self.emitters {
            self.sprites.clear();
            emitter.sprites(&mut self.sprites);
            visit(emitter.recipe(), &self.sprites);
        }
    }
}
