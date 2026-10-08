use super::super::{Draw, Fill, Shadow, Shape, Stroke, place};
use super::between;
use pfx_core::sim::Rng;
use pfx_physics::rigid::{
    BodyDesc, BodyId, ColliderDesc, Combine, Friction, Layers, Locks, Pose, World,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shatter {
    pub fill: Fill,
    pub stroke: Option<Stroke>,
    pub count: u32,
    pub half: [[f32; 2]; 2],
    pub radius: f32,
    pub depth: f32,
    pub speed: [f32; 2],
    pub direction: f32,
    pub spread: f32,
    pub spin: [f32; 2],
    pub life: [f32; 2],
    pub fade: f32,
    pub density: f32,
    pub restitution: f32,
    pub friction: f32,
    pub layers: Layers,
    pub elevation: f32,
    pub shadow: Shadow,
}

impl Shatter {
    pub fn new(fill: Fill, half: [[f32; 2]; 2]) -> Self {
        Self {
            fill,
            stroke: None,
            count: 8,
            half,
            radius: 2.0,
            depth: 4.0,
            speed: [200.0, 500.0],
            direction: -std::f32::consts::FRAC_PI_2,
            spread: std::f32::consts::PI,
            spin: [0.0, 6.0],
            life: [1.5, 2.5],
            fade: 0.4,
            density: 1.0,
            restitution: 0.3,
            friction: 0.6,
            layers: Layers::ALL,
            elevation: 1.0,
            shadow: Shadow::Cast,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Shard {
    body: BodyId,
    half: [f32; 2],
    radius: f32,
    age: f32,
    life: f32,
    fade: f32,
    fill: Fill,
    stroke: Option<Stroke>,
    elevation: f32,
    shadow: Shadow,
}

pub struct Shards {
    items: Vec<Shard>,
    cap: usize,
    rng: Rng,
}

pub fn angle_of(rotation: [f32; 4]) -> f32 {
    2.0 * rotation[2].atan2(rotation[3])
}

impl Shards {
    pub fn new(cap: usize, seed: u64) -> Self {
        Self {
            items: Vec::with_capacity(cap),
            cap,
            rng: Rng::new(seed),
        }
    }

    pub fn live(&self) -> usize {
        self.items.len()
    }

    pub fn bodies(&self) -> impl Iterator<Item = BodyId> + '_ {
        self.items.iter().map(|shard| shard.body)
    }

    pub fn spawn(&mut self, world: &mut World, shatter: &Shatter, at: [f32; 2]) -> u32 {
        let mut spawned = 0;
        for _ in 0..shatter.count {
            if self.items.len() >= self.cap {
                break;
            }
            let rng = &mut self.rng;
            let half = [
                between(rng, [shatter.half[0][0], shatter.half[1][0]]),
                between(rng, [shatter.half[0][1], shatter.half[1][1]]),
            ];
            let angle = shatter.direction + (rng.next_f32() - 0.5) * shatter.spread;
            let speed = between(rng, shatter.speed);
            let spin = between(rng, shatter.spin) * if rng.next_bool() { 1.0 } else { -1.0 };
            let life = between(rng, shatter.life).max(1e-3);
            let turn = rng.next_f32() * std::f32::consts::TAU;
            let pose = Pose::at([at[0], at[1], 0.0]).turned([0.0, 0.0, 1.0], turn);
            let body = world.add_body(
                BodyDesc::dynamic(pose)
                    .with_velocity([angle.cos() * speed, angle.sin() * speed, 0.0])
                    .with_spin([0.0, 0.0, spin])
                    .with_locks(Locks::PLANE_XY),
            );
            let radius = shatter.radius.clamp(0.0, half[0].min(half[1]));
            let collider =
                ColliderDesc::round_box([half[0], half[1], shatter.depth.max(0.5)], radius)
                    .with_density(shatter.density)
                    .with_friction(Friction::uniform(shatter.friction))
                    .with_restitution(shatter.restitution, Combine::Max)
                    .with_layers(shatter.layers);
            if world.add_collider(body, collider).is_none() {
                world.remove_body(body);
                continue;
            }
            self.items.push(Shard {
                body,
                half,
                radius,
                age: 0.0,
                life,
                fade: shatter.fade.max(0.0),
                fill: shatter.fill,
                stroke: shatter.stroke,
                elevation: shatter.elevation,
                shadow: shatter.shadow,
            });
            spawned += 1;
        }
        spawned
    }

    pub fn step(&mut self, world: &mut World, dt: f32) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        let mut index = 0;
        while index < self.items.len() {
            let shard = &mut self.items[index];
            shard.age += dt;
            if shard.age >= shard.life {
                world.remove_body(shard.body);
                self.items.swap_remove(index);
            } else {
                index += 1;
            }
        }
    }

    pub fn clear(&mut self, world: &mut World) {
        for shard in self.items.drain(..) {
            world.remove_body(shard.body);
        }
    }

    pub fn draws(&self, world: &World, out: &mut Vec<Draw>) {
        for shard in &self.items {
            let Some(pose) = world.pose(shard.body) else {
                continue;
            };
            let left = shard.life - shard.age;
            let alpha = if shard.fade > 0.0 {
                (left / shard.fade).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let mut draw = Draw::new(Shape::Rect {
                half: shard.half,
                radii: [shard.radius; 4],
            })
            .transform(place(
                [pose.position[0], pose.position[1]],
                angle_of(pose.rotation),
            ))
            .elevation(shard.elevation)
            .shadow(shard.shadow)
            .opacity(alpha)
            .unpicked();
            draw.fill = shard.fill;
            draw.stroke = shard.stroke;
            out.push(draw);
        }
    }
}
