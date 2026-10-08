pub mod blur;
pub mod shards;
#[cfg(test)]
mod tests;

use super::{Draw, Fill, IDENTITY, Light, Matrix, Shadow, Shape, Srgba, Stroke, multiply, place};
use pfx_core::clock::{Clock, HitStop, Tick};
use pfx_core::ease::Ease;
pub use pfx_core::fx::{
    Comfort, Curve, FLASHES_PER_SECOND, FlashGate, Response, Shake, ShakeOffset, ShakeTuning,
};
use pfx_core::sim::Rng;
use pfx_post::tape::Tape;
pub use shards::{Shards, Shatter};

pub const FLASH_ELEVATION: f32 = f32::MAX;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Responses {
    pub shake: Response,
    pub burst: Response,
    pub ring: Response,
    pub flash: Response,
    pub blur: Response,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade {
    pub from: Srgba,
    pub to: Srgba,
    pub ease: Ease,
}

impl Fade {
    pub fn at(&self, t: f32) -> Srgba {
        let k = self.ease.at(t.clamp(0.0, 1.0));
        Srgba(std::array::from_fn(|i| {
            self.from.0[i] + (self.to.0[i] - self.from.0[i]) * k
        }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Burst {
    pub shape: Shape,
    pub fill: Fill,
    pub stroke: Option<Stroke>,
    pub count: u32,
    pub speed: [f32; 2],
    pub direction: f32,
    pub spread: f32,
    pub gravity: [f32; 2],
    pub drag: f32,
    pub spin: [f32; 2],
    pub life: [f32; 2],
    pub size: [f32; 2],
    pub scale: Curve,
    pub alpha: Curve,
    pub colour: Option<Fade>,
    pub elevation: f32,
    pub shadow: Shadow,
    pub shutter: f32,
    pub additive: bool,
}

impl Burst {
    pub fn new(shape: Shape, fill: Fill) -> Self {
        Self {
            shape,
            fill,
            stroke: None,
            count: 16,
            speed: [100.0, 300.0],
            direction: 0.0,
            spread: std::f32::consts::TAU,
            gravity: [0.0, 0.0],
            drag: 0.0,
            spin: [0.0, 0.0],
            life: [0.5, 1.0],
            size: [1.0, 1.0],
            scale: Curve::ONE,
            alpha: Curve::new(1.0, 0.0, Ease::Linear),
            colour: None,
            elevation: 0.0,
            shadow: Shadow::None,
            shutter: 0.0,
            additive: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BurstId(u16);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Particle {
    recipe: u16,
    position: [f32; 2],
    velocity: [f32; 2],
    angle: f32,
    spin: f32,
    size: f32,
    age: f32,
    life: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub centre: [f32; 2],
    pub radius: [f32; 2],
    pub width: [f32; 2],
    pub colour: Srgba,
    pub alpha: Curve,
    pub life: f32,
    pub ease: Ease,
    pub elevation: f32,
    pub additive: bool,
}

impl Ring {
    pub fn new(centre: [f32; 2], radius: [f32; 2], colour: Srgba, life: f32) -> Self {
        Self {
            centre,
            radius,
            width: [12.0, 2.0],
            colour,
            alpha: Curve::new(1.0, 0.0, Ease::QuadIn),
            life,
            ease: Ease::QuartOut,
            elevation: 0.0,
            additive: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct LiveRing {
    ring: Ring,
    age: f32,
    life: f32,
    gain: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flash {
    pub colour: Srgba,
    pub alpha: f32,
    pub attack: f32,
    pub release: f32,
    pub ease: Ease,
    pub additive: bool,
}

impl Flash {
    pub fn new(colour: Srgba, alpha: f32, attack: f32, release: f32) -> Self {
        Self {
            colour,
            alpha,
            attack,
            release,
            ease: Ease::QuadOut,
            additive: false,
        }
    }

    pub fn alpha_at(&self, t: f32) -> f32 {
        if t.is_nan() || t < 0.0 {
            return 0.0;
        }
        if t < self.attack {
            return self.alpha * self.ease.at(t / self.attack);
        }
        let t = t - self.attack.max(0.0);
        if t >= self.release {
            return 0.0;
        }
        self.alpha * (1.0 - self.ease.at(t / self.release))
    }

    pub fn length(&self) -> f32 {
        self.attack.max(0.0) + self.release.max(0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct LiveFlash {
    flash: Flash,
    age: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectsDesc {
    pub layout: [f32; 2],
    pub seed: u64,
    pub particles: usize,
    pub rings: usize,
    pub shake: ShakeTuning,
    pub responses: Responses,
    pub comfort: Comfort,
}

impl EffectsDesc {
    pub fn new(layout: [f32; 2], seed: u64) -> Self {
        Self {
            layout,
            seed,
            particles: 8192,
            rings: 64,
            shake: ShakeTuning::default(),
            responses: Responses::default(),
            comfort: Comfort::FULL,
        }
    }
}

pub struct Effects {
    pub comfort: Comfort,
    pub intensity: f32,
    pub responses: Responses,
    layout: [f32; 2],
    shake: Shake,
    rng: Rng,
    recipes: Vec<Burst>,
    particles: Vec<Particle>,
    particle_cap: usize,
    rings: Vec<LiveRing>,
    ring_cap: usize,
    flash: Option<LiveFlash>,
    gate: FlashGate,
    real: f64,
    dropped: u64,
    out: Vec<Draw>,
}

impl Effects {
    pub fn new(desc: EffectsDesc) -> Self {
        Self {
            comfort: desc.comfort,
            intensity: 0.0,
            responses: desc.responses,
            layout: desc.layout,
            shake: Shake::new(desc.shake),
            rng: Rng::new(desc.seed),
            recipes: Vec::new(),
            particles: Vec::with_capacity(desc.particles),
            particle_cap: desc.particles,
            rings: Vec::with_capacity(desc.rings),
            ring_cap: desc.rings,
            flash: None,
            gate: FlashGate::new(),
            real: 0.0,
            dropped: 0,
            out: Vec::new(),
        }
    }

    pub fn recipe(&mut self, burst: Burst) -> Result<BurstId, String> {
        let index = u16::try_from(self.recipes.len()).map_err(|_| "too many burst recipes")?;
        self.recipes.push(burst);
        Ok(BurstId(index))
    }

    pub fn recipe_mut(&mut self, id: BurstId) -> Option<&mut Burst> {
        self.recipes.get_mut(usize::from(id.0))
    }

    pub fn live(&self) -> usize {
        self.particles.len()
    }

    pub fn cap(&self) -> usize {
        self.particle_cap
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn real_time(&self) -> f64 {
        self.real
    }

    pub fn shake(&mut self, trauma: f32) {
        self.shake.add(trauma);
    }

    pub fn trauma(&self) -> f32 {
        self.shake.trauma()
    }

    pub fn burst(&mut self, id: BurstId, at: [f32; 2]) -> u32 {
        let direction = self
            .recipes
            .get(usize::from(id.0))
            .map_or(0.0, |recipe| recipe.direction);
        self.burst_toward(id, at, direction)
    }

    pub fn burst_toward(&mut self, id: BurstId, at: [f32; 2], direction: f32) -> u32 {
        let Some(recipe) = self.recipes.get(usize::from(id.0)) else {
            return 0;
        };
        let response = self.responses.burst;
        let count = (recipe.count as f32 * response.gain(self.intensity)).round() as u32;
        let rate = response.rate(self.intensity);
        let mut spawned = 0;
        for _ in 0..count {
            if self.particles.len() >= self.particle_cap {
                self.dropped += u64::from(count - spawned);
                break;
            }
            let rng = &mut self.rng;
            let angle = direction + (rng.next_f32() - 0.5) * recipe.spread;
            let speed = between(rng, recipe.speed) * rate;
            let life = between(rng, recipe.life).max(1e-3);
            self.particles.push(Particle {
                recipe: id.0,
                position: at,
                velocity: [angle.cos() * speed, angle.sin() * speed],
                angle: rng.next_f32() * std::f32::consts::TAU,
                spin: between(rng, recipe.spin) * if rng.next_bool() { 1.0 } else { -1.0 },
                size: between(rng, recipe.size),
                age: 0.0,
                life,
            });
            spawned += 1;
        }
        spawned
    }

    pub fn ring(&mut self, ring: Ring) -> bool {
        if self.rings.len() >= self.ring_cap || ring.life.is_nan() || ring.life <= 0.0 {
            return false;
        }
        let response = self.responses.ring;
        let rate = response.rate(self.intensity).max(1e-3);
        self.rings.push(LiveRing {
            ring,
            age: 0.0,
            life: ring.life / rate,
            gain: response.gain(self.intensity),
        });
        true
    }

    pub fn flash(&mut self, flash: Flash) -> bool {
        let response = self.responses.flash;
        let rate = response.rate(self.intensity).max(1e-3);
        let alpha = self
            .comfort
            .flash(flash.alpha * response.gain(self.intensity))
            .clamp(0.0, 1.0);
        if alpha.is_nan() || alpha <= 0.0 || !self.gate.admit(self.real) {
            return false;
        }
        self.flash = Some(LiveFlash {
            flash: Flash {
                alpha,
                attack: flash.attack.max(0.0) / rate,
                release: flash.release.max(0.0) / rate,
                ..flash
            },
            age: 0.0,
        });
        true
    }

    pub fn hit_stop(&self, clock: &mut Clock, stop: HitStop) {
        clock.hit_stop(self.comfort.hit_stop(stop));
    }

    pub fn tape(&self, tape: Tape) -> Tape {
        Tape {
            strength: self.comfort.glitch(tape.strength),
            ..tape
        }
    }

    pub fn motion(&self, velocity: [f32; 2], shutter: f32) -> [f32; 2] {
        let gain = self.responses.blur.gain(self.intensity) * shutter.max(0.0);
        velocity.map(|value| value * gain)
    }

    pub fn step(&mut self, tick: &Tick) {
        self.real += f64::from(tick.real.max(0.0));
        let dt = tick.dt.max(0.0);
        let paced = tick.paced.max(0.0);
        self.shake
            .step(paced, self.responses.shake.rate(self.intensity));
        if let Some(live) = self.flash.as_mut() {
            live.age += paced;
            if live.age >= live.flash.length() {
                self.flash = None;
            }
        }
        if dt <= 0.0 {
            return;
        }
        let mut index = 0;
        while index < self.particles.len() {
            let particle = &mut self.particles[index];
            particle.age += dt;
            if particle.age >= particle.life {
                self.particles.swap_remove(index);
                continue;
            }
            let recipe = &self.recipes[usize::from(particle.recipe)];
            let keep = (-recipe.drag.max(0.0) * dt).exp();
            for axis in 0..2 {
                particle.velocity[axis] =
                    (particle.velocity[axis] + recipe.gravity[axis] * dt) * keep;
                particle.position[axis] += particle.velocity[axis] * dt;
            }
            particle.angle += particle.spin * dt;
            index += 1;
        }
        let mut index = 0;
        while index < self.rings.len() {
            let ring = &mut self.rings[index];
            ring.age += dt;
            if ring.age >= ring.life {
                self.rings.swap_remove(index);
            } else {
                index += 1;
            }
        }
    }

    pub fn offset(&self) -> ShakeOffset {
        let gain = self
            .comfort
            .shake(self.responses.shake.gain(self.intensity));
        self.shake.offset(gain)
    }

    pub fn camera(&self) -> Matrix {
        camera(self.offset(), self.layout)
    }

    pub fn compose(&mut self, draws: &[Draw], light: Light) -> (&[Draw], Light) {
        self.out.clear();
        let offset = self.offset();
        let camera = (!offset.is_zero()).then(|| camera(offset, self.layout));
        let motion = self.comfort.amount();
        let carry = |draw: Draw| shaken(draw, camera, motion);
        self.out.extend(draws.iter().copied().map(carry));
        let blur_gain = self.responses.blur.gain(self.intensity);
        for particle in &self.particles {
            let recipe = &self.recipes[usize::from(particle.recipe)];
            let t = particle.age / particle.life;
            let size = particle.size * recipe.scale.at(t);
            let alpha = recipe.alpha.at(t).clamp(0.0, 1.0);
            if !(size > 0.0 && alpha > 0.0) {
                continue;
            }
            let transform = multiply(
                place(particle.position, particle.angle),
                super::scaling(size, size),
            );
            let fill = match recipe.colour {
                Some(fade) => Fill::Solid(fade.at(t)),
                None => recipe.fill,
            };
            let mut draw = Draw::new(recipe.shape)
                .transform(transform)
                .elevation(recipe.elevation)
                .shadow(recipe.shadow)
                .opacity(alpha)
                .unpicked();
            draw.fill = fill;
            draw.stroke = recipe.stroke;
            draw.fx.additive = recipe.additive;
            if recipe.shutter > 0.0 {
                let gain = recipe.shutter * blur_gain;
                draw.fx.blur = particle.velocity.map(|value| value * gain);
            }
            self.out.push(carry(draw));
        }
        for live in &self.rings {
            let ring = live.ring;
            let t = live.age / live.life;
            let k = ring.ease.at(t);
            let radius = ring.radius[0] + (ring.radius[1] - ring.radius[0]) * k;
            let width = ring.width[0] + (ring.width[1] - ring.width[0]) * k;
            let alpha = (ring.alpha.at(t) * live.gain).clamp(0.0, 1.0);
            if !(radius > 0.0 && width > 0.0 && alpha > 0.0) {
                continue;
            }
            let mut draw = Draw::new(Shape::Ring { radius, width })
                .at(ring.centre)
                .fill(ring.colour)
                .opacity(alpha)
                .elevation(ring.elevation)
                .shadow(Shadow::None)
                .unpicked();
            draw.fx.additive = ring.additive;
            self.out.push(carry(draw));
        }
        if let Some(live) = self.flash {
            let alpha = live.flash.alpha_at(live.age).clamp(0.0, 1.0);
            if alpha > 0.0 {
                let [w, h] = self.layout;
                let mut draw = Draw::rect([w * 0.5, h * 0.5], [w, h], 0.0)
                    .fill(live.flash.colour)
                    .opacity(alpha)
                    .elevation(FLASH_ELEVATION)
                    .shadow(Shadow::None)
                    .unpicked();
                draw.fx.additive = live.flash.additive;
                self.out.push(draw);
            }
        }
        let light = if offset.roll != 0.0 {
            let (s, c) = offset.roll.sin_cos();
            let [x, y] = light.shadow;
            Light {
                shadow: [c * x - s * y, s * x + c * y],
                ..light
            }
        } else {
            light
        };
        (&self.out, light)
    }

    pub fn composed(&self) -> &[Draw] {
        &self.out
    }

    pub fn out_capacity(&self) -> usize {
        self.out.capacity()
    }

    pub fn pool_capacity(&self) -> (usize, usize) {
        (self.particles.capacity(), self.rings.capacity())
    }
}

fn shaken(mut draw: Draw, camera: Option<Matrix>, motion: f32) -> Draw {
    if let Some(camera) = camera {
        draw.transform = multiply(camera, draw.transform);
    }
    if draw.fx.blur != [0.0; 2] && motion < 1.0 {
        draw.fx.blur = draw.fx.blur.map(|value| value * motion);
    }
    draw
}

pub fn camera(offset: ShakeOffset, layout: [f32; 2]) -> Matrix {
    if offset.is_zero() {
        return IDENTITY;
    }
    let centre = [layout[0] * 0.5, layout[1] * 0.5];
    multiply(
        place(
            [
                centre[0] + offset.translation[0],
                centre[1] + offset.translation[1],
            ],
            offset.roll,
        ),
        super::translation(-centre[0], -centre[1]),
    )
}

pub(crate) fn between(rng: &mut Rng, range: [f32; 2]) -> f32 {
    range[0] + (range[1] - range[0]) * rng.next_f32()
}
