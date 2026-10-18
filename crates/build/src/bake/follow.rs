use crate::{Bake, Track};
use glam::{DMat4, DQuat, DVec3};
use pfx_glb::Node;
use pfx_mesh::math::Real;
use pfx_motion::vehicle::{self, Chassis, Lean};
use pfx_motion::{Route, Timeline, Trs, noise1, trs_from_matrix};
use pfx_recipe::{Follow, Owner, Recipe};
use std::f64::consts::{PI, TAU};

const STEER_MAX_DEG: f64 = 35.0;
const SLIP_SMOOTH_S: f64 = 0.2;
const CRAWL_M_S: f64 = 1.0;
const STILL_M_S: f64 = 0.5;
const WHEELBASE_M: f64 = 2.7;
const TRACK_M: f64 = 1.6;
const SPIN_STEP: f64 = TAU / 3.0;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Drive {
    pub distance: Vec<f64>,
    pub pose: Vec<DMat4>,
    pub curvature: Vec<f64>,
    pub counter: Vec<f64>,
    pub lean: Option<Vec<Lean>>,
}

pub(crate) fn speed_at(keys: &[[f64; 2]], t: f64) -> f64 {
    let Some(first) = keys.first() else {
        return 0.0;
    };
    if t <= first[0] {
        return first[1];
    }
    for pair in keys.windows(2) {
        let [a, b] = [pair[0], pair[1]];
        if t <= b[0] {
            let span = b[0] - a[0];
            if span <= 0.0 {
                return b[1];
            }
            return a[1] + (b[1] - a[1]) * (t - a[0]) / span;
        }
    }
    keys[keys.len() - 1][1]
}

pub(crate) fn distances(follow: &Follow, route: &Route, timeline: &Timeline) -> Vec<f64> {
    let dt = 1.0 / f64::from(timeline.fps.max(1));
    let length = route.length();
    let mut out = Vec::with_capacity(timeline.frames as usize + 1);
    let mut travelled = follow.start;
    let mut before = speed_at(&follow.speed, 0.0);
    out.push(travelled);
    for n in 1..=timeline.frames {
        let now = speed_at(&follow.speed, timeline.time(n));
        travelled += 0.5 * (before + now) * dt;
        before = now;
        out.push(travelled);
    }
    if !route.closed() {
        for d in &mut out {
            *d = d.clamp(0.0, length);
        }
    }
    out
}

fn name_seed(seed: u64, name: &str) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for byte in name.bytes() {
        h ^= u64::from(byte);
        h = h.wrapping_mul(0x0100_0000_01B3);
    }
    seed ^ h
}

pub(crate) fn chassis(follow: &Follow, (wheelbase, track): (f64, f64)) -> Option<Chassis> {
    let d = follow.dynamics?;
    Some(Chassis {
        mass: d.mass,
        wheelbase: d.wheelbase.unwrap_or(wheelbase),
        track: d.track.unwrap_or(track),
        cg_height: d.cg_height,
        stiffness: d.stiffness,
        damping: d.damping,
        grip: d.grip,
    })
}

pub(crate) fn drive(
    follow: &Follow,
    route: &Route,
    timeline: &Timeline,
    size: (f64, f64),
) -> Drive {
    let distance = distances(follow, route, timeline);
    let dt = 1.0 / f64::from(timeline.fps.max(1));
    let seed = name_seed(timeline.seed, &follow.object);
    let offset = |t: f64| match follow.drift {
        Some(d) if d.amount != 0.0 && d.period > 0.0 => {
            follow.lane + d.amount * noise1(seed, t / d.period)
        }
        _ => follow.lane,
    };
    let reference = follow
        .drift
        .filter(|d| d.amount > 0.0 && d.period > 0.0)
        .map(|d| d.amount / d.period);
    let slip = follow.drift.map_or(0.0, |d| d.slip_deg.to_radians());
    let ease = 1.0 - (-dt / SLIP_SMOOTH_S).exp_of();
    let mut yaw = None::<f64>;
    let mut basis = Vec::with_capacity(distance.len());
    let mut drifted = Vec::with_capacity(distance.len());
    for (n, &d) in distance.iter().enumerate() {
        let t = timeline.time(n as u32);
        let lateral = offset(t);
        let target = match reference {
            Some(reference) => {
                let side = (offset(t + dt) - offset(t - dt)) / (2.0 * dt);
                let forward = speed_at(&follow.speed, t).max(CRAWL_M_S);
                side.atan2_of(forward) + slip * (side / reference).tanh_of()
            }
            None => 0.0,
        };
        let smooth = match yaw {
            Some(y) => y + (target - y) * ease,
            None => target,
        };
        yaw = Some(smooth);
        let frame = route.at(d);
        let z = frame.tangent.normalize_or(DVec3::Z);
        let x = frame.up.cross(z).normalize_or(DVec3::X);
        let y = z.cross(x);
        let position = frame.position + frame.right * lateral + y * follow.ride;
        basis.push(DMat4::from_cols(
            x.extend(0.0),
            y.extend(0.0),
            z.extend(0.0),
            position.extend(1.0),
        ));
        drifted.push(smooth);
    }
    let curvature: Vec<f64> = distance.iter().map(|d| route.curvature(*d)).collect();
    let Some(chassis) = chassis(follow, size) else {
        let pose = basis
            .iter()
            .zip(&drifted)
            .map(|(basis, smooth)| *basis * DMat4::from_rotation_y(-smooth))
            .collect();
        return Drive {
            counter: vec![0.0; distance.len()],
            distance,
            pose,
            curvature,
            lean: None,
        };
    };
    let forces = vehicle::forces(&basis, dt);
    let lean = vehicle::ride(chassis, &forces, dt);
    let (mut sliding, mut tracking) = (None::<f64>, None::<f64>);
    let mut counter = Vec::with_capacity(distance.len());
    let mut pose = Vec::with_capacity(distance.len());
    for n in 0..distance.len() {
        let eased = |at: Option<f64>, goal: f64| at.map_or(goal, |at| at + (goal - at) * ease);
        let slide = eased(sliding, chassis.slip(forces[n]));
        let track = eased(tracking, chassis.tracking(-curvature[n]));
        sliding = Some(slide);
        tracking = Some(track);
        counter.push(slide);
        pose.push(
            basis[n] * DMat4::from_rotation_y(-drifted[n]) * DMat4::from_rotation_y(slide + track),
        );
    }
    Drive {
        distance,
        pose,
        curvature,
        counter,
        lean: Some(lean),
    }
}

pub(crate) fn keyed(poses: &[DMat4], step: f64) -> Drive {
    let count = poses.len();
    let forward: Vec<DVec3> = poses
        .iter()
        .map(|m| m.z_axis.truncate().normalize_or(DVec3::Z))
        .collect();
    let mut distance = Vec::with_capacity(count);
    let mut yaws: Vec<f64> = Vec::with_capacity(count);
    for n in 0..count {
        let d = match n.checked_sub(1) {
            Some(before) => {
                let travel = poses[n].w_axis.truncate() - poses[before].w_axis.truncate();
                distance[before] + travel.dot((forward[before] + forward[n]).normalize_or_zero())
            }
            None => 0.0,
        };
        distance.push(d);
        let yaw = forward[n].x.atan2_of(forward[n].z);
        yaws.push(match yaws.last() {
            Some(last) => last + (yaw - last + PI).rem_euclid(TAU) - PI,
            None => yaw,
        });
    }
    let rate = |values: &[f64], n: usize| {
        let (from, to) = (n.saturating_sub(1), (n + 1).min(count - 1));
        if to > from {
            (values[to] - values[from]) / ((to - from) as f64 * step)
        } else {
            0.0
        }
    };
    let curvature = (0..count)
        .map(|n| {
            let speed = rate(&distance, n);
            if speed.abs() > STILL_M_S {
                -rate(&yaws, n) / speed
            } else {
                0.0
            }
        })
        .collect();
    Drive {
        distance,
        pose: poses.to_vec(),
        curvature,
        counter: vec![0.0; count],
        lean: None,
    }
}

#[cfg(test)]
pub(crate) fn steer(route: &Route, distance: f64, wheelbase: f64) -> f64 {
    turn(route.curvature(distance), 0.0, wheelbase)
}

fn turn(curvature: f64, counter: f64, wheelbase: f64) -> f64 {
    let max = STEER_MAX_DEG.to_radians();
    (-(wheelbase * curvature).atan_of() - counter).clamp(-max, max)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Wheel {
    pub centre: DVec3,
    pub radius: f64,
    pub axle: DVec3,
}

pub(crate) fn spin(distance: f64, radius: f64) -> f64 {
    if radius > 0.0 { distance / radius } else { 0.0 }
}

fn vec3(value: Option<&serde_json::Value>) -> Option<DVec3> {
    let list = value?.as_array()?;
    if list.len() != 3 {
        return None;
    }
    Some(DVec3::new(
        list[0].as_f64()?,
        list[1].as_f64()?,
        list[2].as_f64()?,
    ))
}

fn wheel_of(node: &Node, corner: &str, facts: Option<&serde_json::Value>) -> Wheel {
    let fact = facts
        .and_then(|f| f.get("wheels"))
        .and_then(|w| w.get(corner));
    let bounds = pfx_mesh::bounds(&node.mesh);
    let centre = vec3(fact.and_then(|f| f.get("centre")))
        .or_else(|| bounds.map(|[lo, hi]| (DVec3::from_array(lo) + DVec3::from_array(hi)) * 0.5))
        .unwrap_or(DVec3::ZERO);
    let radius = fact
        .and_then(|f| f.get("radius"))
        .and_then(serde_json::Value::as_f64)
        .or_else(|| bounds.map(|[lo, hi]| (hi[1] - lo[1]) * 0.5))
        .unwrap_or(0.0);
    let axle = vec3(fact.and_then(|f| f.get("axle")))
        .and_then(|a| a.try_normalize())
        .unwrap_or(DVec3::X);
    let axle = if axle.x < 0.0 { -axle } else { axle };
    Wheel {
        centre,
        radius,
        axle,
    }
}

fn frame(list: &[DMat4], n: usize) -> DMat4 {
    list[n.min(list.len().saturating_sub(1))]
}

#[derive(Clone)]
struct Carry {
    front: bool,
    wheel: Wheel,
    hub: Vec<DMat4>,
    offset: Vec<DMat4>,
}

pub(crate) struct Sub<'a> {
    name: String,
    rel: Vec<DMat4>,
    wheel: Option<(bool, Wheel)>,
    carry: Option<Carry>,
    facts: Option<&'a serde_json::Value>,
}

impl Sub<'_> {
    fn owns(&self, node: &str) -> bool {
        node.strip_prefix(self.name.as_str())
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    }

    fn spins(&self, node: &str) -> bool {
        if self
            .facts
            .and_then(|f| f.get("spin"))
            .and_then(serde_json::Value::as_bool)
            == Some(false)
        {
            return false;
        }
        node.strip_prefix(self.name.as_str())
            .and_then(|rest| rest.strip_prefix('.'))
            .and_then(|piece| self.facts?.get(piece)?.get("spin")?.as_bool())
            .unwrap_or(true)
    }
}

pub(crate) fn descends(recipe: &Recipe, from: usize, to: usize) -> bool {
    let mut at = from;
    for _ in 0..recipe.objects.len() {
        let Some(parent) = &recipe.objects[at].parent else {
            return false;
        };
        let Some(up) = recipe.objects.iter().rposition(|o| o.name == *parent) else {
            return false;
        };
        if up == to {
            return true;
        }
        at = up;
    }
    false
}

fn independent(recipe: &Recipe, owners: &[String], from: usize, root: usize) -> bool {
    let mut at = from;
    for _ in 0..recipe.objects.len() {
        if at == root {
            return false;
        }
        if owners.contains(&recipe.objects[at].name) {
            return true;
        }
        let Some(parent) = &recipe.objects[at].parent else {
            return false;
        };
        let Some(up) = recipe.objects.iter().rposition(|o| o.name == *parent) else {
            return false;
        };
        at = up;
    }
    false
}

fn keyed_below(recipe: &Recipe, from: usize, root: usize) -> bool {
    let mut at = from;
    for _ in 0..recipe.objects.len() {
        if at == root {
            return false;
        }
        if recipe
            .keyed
            .contains_key(&format!("object.{}", recipe.objects[at].name))
        {
            return true;
        }
        let Some(parent) = &recipe.objects[at].parent else {
            return false;
        };
        let Some(up) = recipe.objects.iter().rposition(|o| o.name == *parent) else {
            return false;
        };
        at = up;
    }
    false
}

pub(crate) fn subs<'a>(b: &Bake<'a>, object: &str, named: bool) -> Result<Vec<Sub<'a>>, String> {
    let prefix = format!("{object}.");
    let wheel_prefix = format!("{object}.wheel_");
    let Some(root) = b.recipe.objects.iter().position(|o| o.name == object) else {
        return Ok(Vec::new());
    };
    let owners: Vec<String> = b
        .recipe
        .constrained()
        .into_iter()
        .filter_map(|c| match c.owner {
            Owner::Object(name) => Some(name),
            _ => None,
        })
        .collect();
    let world = |at: usize| crate::world(b.recipe, at).map_err(|e| e.to_string());
    let times: Vec<f64> = (0..=b.timeline.frames)
        .map(|n| b.timeline.time(n))
        .collect();
    let mut found: Vec<(usize, Sub<'a>)> = Vec::new();
    for (at, sub) in b.recipe.objects.iter().enumerate() {
        if at == root {
            continue;
        }
        let below = descends(b.recipe, at, root);
        let by_name = named && sub.name.starts_with(&prefix);
        if !by_name && (!below || independent(b.recipe, &owners, at, root)) {
            continue;
        }
        let back = if below {
            world(root)?.inverse()
        } else {
            DMat4::IDENTITY
        };
        let rel = if keyed_below(b.recipe, at, root) {
            times
                .iter()
                .map(|t| back * super::keys::world_at(b.recipe, at, *t))
                .collect()
        } else if below {
            vec![back * world(at)?]
        } else {
            vec![world(at)?]
        };
        let facts = b.facts.get(&sub.name);
        let wheel = sub
            .name
            .strip_prefix(&wheel_prefix)
            .filter(|corner| !corner.contains('.'))
            .map(|corner| {
                let fact = facts.and_then(|f| f.get("wheel"));
                let radius = fact
                    .and_then(|f| f.get("radius"))
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or_else(|| sub_radius(b.nodes, &sub.name));
                let axle = vec3(fact.and_then(|f| f.get("axle")))
                    .and_then(|a| a.try_normalize())
                    .unwrap_or(DVec3::X);
                let axle = if axle.x < 0.0 { -axle } else { axle };
                (
                    corner.starts_with('f'),
                    Wheel {
                        centre: DVec3::ZERO,
                        radius,
                        axle,
                    },
                )
            });
        let carry = wheel.map(|(front, wheel)| Carry {
            front,
            wheel,
            hub: rel.clone(),
            offset: vec![DMat4::IDENTITY],
        });
        found.push((
            at,
            Sub {
                name: sub.name.clone(),
                rel,
                wheel,
                carry,
                facts,
            },
        ));
    }
    let hubs: Vec<(usize, String, Carry)> = found
        .iter()
        .filter(|(_, s)| s.wheel.is_some())
        .filter_map(|(at, s)| Some((*at, s.name.clone(), s.carry.clone()?)))
        .collect();
    for (at, sub) in &mut found {
        if sub.carry.is_some() {
            continue;
        }
        let hub = hubs
            .iter()
            .filter(|(hub_at, name, _)| {
                hub_at != at
                    && (descends(b.recipe, *at, *hub_at)
                        || sub
                            .name
                            .strip_prefix(name.as_str())
                            .is_some_and(|rest| rest.starts_with('.')))
            })
            .max_by_key(|(_, name, _)| name.len());
        if let Some((_, _, carry)) = hub {
            let frames = carry.hub.len().max(sub.rel.len());
            sub.carry = Some(Carry {
                offset: (0..frames)
                    .map(|n| frame(&carry.hub, n).inverse() * frame(&sub.rel, n))
                    .collect(),
                ..carry.clone()
            });
        }
    }
    let mut out: Vec<Sub<'a>> = found.into_iter().map(|(_, sub)| sub).collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.name.len()));
    Ok(out)
}

fn sub_radius(nodes: &[Node], name: &str) -> f64 {
    let dotted = format!("{name}.");
    nodes
        .iter()
        .filter(|n| n.name == name || n.name.starts_with(&dotted))
        .filter_map(|n| pfx_mesh::bounds(&n.mesh))
        .map(|[lo, hi]| (hi[1] - lo[1]) * 0.5)
        .fold(0.0, f64::max)
}

fn wheelbase(wheels: &[(bool, Wheel)], facts: Option<&serde_json::Value>) -> f64 {
    let mean = |front: bool| {
        let z: Vec<f64> = wheels
            .iter()
            .filter(|(f, _)| *f == front)
            .map(|(_, w)| w.centre.z)
            .collect();
        (!z.is_empty()).then(|| z.iter().sum::<f64>() / z.len() as f64)
    };
    match (mean(true), mean(false)) {
        (Some(front), Some(rear)) if front != rear => (front - rear).abs(),
        _ => facts
            .and_then(|f| f.get("wheelbase"))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(WHEELBASE_M),
    }
}

fn owned<'n>(object: &str, nodes: &'n [Node], subs: &[Sub]) -> Vec<&'n Node> {
    let prefix = format!("{object}.");
    nodes
        .iter()
        .filter(|n| {
            n.name == object || n.name.starts_with(&prefix) || subs.iter().any(|s| s.owns(&n.name))
        })
        .collect()
}

fn axles(
    object: &str,
    owned: &[&Node],
    subs: &[Sub],
    facts: Option<&serde_json::Value>,
) -> (Vec<(bool, Wheel)>, f64) {
    let wheel_prefix = format!("{object}.wheel_");
    let wheels: Vec<(bool, Wheel)> = owned
        .iter()
        .filter(|n| !subs.iter().any(|s| s.owns(&n.name)))
        .filter_map(|n| {
            let corner = n.name.strip_prefix(&wheel_prefix)?;
            Some((corner.starts_with('f'), wheel_of(n, corner, facts)))
        })
        .collect();
    let mut axles = wheels.clone();
    axles.extend(subs.iter().filter_map(|s| {
        let (front, wheel) = s.wheel?;
        Some((
            front,
            Wheel {
                centre: frame(&s.rel, 0).transform_point3(wheel.centre),
                ..wheel
            },
        ))
    }));
    let base = wheelbase(&axles, facts);
    (wheels, base)
}

pub(crate) fn size_of(b: &Bake, object: &str) -> Result<(f64, f64), String> {
    let subs = subs(b, object, true)?;
    let owned = owned(object, b.nodes, &subs);
    let facts = b.facts.get(object);
    let (wheels, base) = axles(object, &owned, &subs, facts);
    let across: Vec<f64> = wheels.iter().map(|(_, w)| w.centre.x.abs()).collect();
    let track = facts
        .and_then(|f| f.get("track"))
        .and_then(serde_json::Value::as_f64)
        .or_else(|| {
            (!across.is_empty()).then(|| 2.0 * across.iter().sum::<f64>() / across.len() as f64)
        })
        .filter(|t| *t > 0.0)
        .unwrap_or(TRACK_M);
    Ok((base, track))
}

fn blend(a: DMat4, b: DMat4, s: f64) -> DMat4 {
    let (sa, ra, ta) = a.to_scale_rotation_translation();
    let (sb, rb, tb) = b.to_scale_rotation_translation();
    DMat4::from_scale_rotation_translation(
        sa.lerp(sb, s),
        ra.normalize().slerp(rb.normalize(), s).normalize(),
        ta.lerp(tb, s),
    )
}

fn spun(
    outer: &[DMat4],
    angle: &[f64],
    inner: &[DMat4],
    axle: DVec3,
    step: f64,
) -> (Vec<Trs>, Vec<(f64, Trs)>) {
    let at = |a: DMat4, angle: f64, b: DMat4| {
        trs_from_matrix(a * DMat4::from_quat(DQuat::from_axis_angle(axle, angle)) * b)
    };
    let keys = (0..outer.len())
        .map(|n| at(outer[n], angle[n], frame(inner, n)))
        .collect();
    let mut between = Vec::new();
    for n in 1..outer.len() {
        let swept = angle[n] - angle[n - 1];
        let steps = (swept.abs() / SPIN_STEP).ceil();
        if steps < 2.0 {
            continue;
        }
        for k in 1..steps as usize {
            let s = k as f64 / steps;
            between.push((
                (n as f64 - 1.0 + s) * step,
                at(
                    blend(outer[n - 1], outer[n], s),
                    angle[n - 1] + swept * s,
                    blend(frame(inner, n - 1), frame(inner, n), s),
                ),
            ));
        }
    }
    (keys, between)
}

pub(crate) struct Motion<'a> {
    pub poses: &'a [DMat4],
    pub scales: Option<&'a [DVec3]>,
    pub scale: DVec3,
    pub drive: Option<&'a Drive>,
    pub step: f64,
}

pub(crate) fn nodes(
    object: &str,
    motion: Motion,
    (nodes, subs): (&[Node], &[Sub]),
    facts: Option<&serde_json::Value>,
) -> Vec<Track> {
    let Motion {
        poses,
        scales,
        scale,
        drive,
        step,
    } = motion;
    let wheel_prefix = format!("{object}.wheel_");
    let owned = owned(object, nodes, subs);
    let sub_of = |node: &Node| subs.iter().find(|s| s.owns(&node.name));
    let (wheels, base) = axles(object, &owned, subs, facts);
    let constant = DMat4::from_scale(scale);
    let scaled = |n: usize| match scales {
        Some(scales) => DMat4::from_scale(scales[n]),
        None => constant,
    };
    let lean = drive.and_then(|d| d.lean.as_ref());
    let sprung = |n: usize| match lean {
        Some(lean) => poses[n] * lean[n].matrix() * scaled(n),
        None => poses[n] * scaled(n),
    };
    let steer = |n: usize| match drive {
        Some(d) => turn(d.curvature[n], d.counter[n], base),
        None => 0.0,
    };
    let count = poses.len();
    let mut tracks = Vec::with_capacity(owned.len());
    let mut wheel_at = 0;
    for node in owned {
        let sub = sub_of(node);
        let carried = sub.and_then(|s| Some((s, s.carry.as_ref()?)));
        let (keys, between) = match (sub, carried, drive) {
            (_, Some((sub, carry)), Some(drive)) => {
                let spins = sub.spins(&node.name);
                let outer: Vec<DMat4> = (0..count)
                    .map(|n| {
                        let hub = frame(&carry.hub, n);
                        let pivot = hub.transform_point3(carry.wheel.centre);
                        let turn = if carry.front { steer(n) } else { 0.0 };
                        poses[n]
                            * scaled(n)
                            * DMat4::from_translation(pivot)
                            * DMat4::from_rotation_y(turn)
                            * DMat4::from_translation(-pivot)
                            * hub
                            * DMat4::from_translation(carry.wheel.centre)
                    })
                    .collect();
                let angle: Vec<f64> = drive
                    .distance
                    .iter()
                    .map(|d| {
                        if spins {
                            spin(*d, carry.wheel.radius)
                        } else {
                            0.0
                        }
                    })
                    .collect();
                let back = DMat4::from_translation(-carry.wheel.centre);
                let inner: Vec<DMat4> = carry.offset.iter().map(|o| back * *o).collect();
                spun(&outer, &angle, &inner, carry.wheel.axle, step)
            }
            (Some(sub), _, _) => (
                (0..count)
                    .map(|n| trs_from_matrix(sprung(n) * frame(&sub.rel, n)))
                    .collect(),
                Vec::new(),
            ),
            (None, _, Some(drive)) if node.name.starts_with(&wheel_prefix) => {
                let (front, wheel) = wheels[wheel_at];
                wheel_at += 1;
                let outer: Vec<DMat4> = (0..count)
                    .map(|n| {
                        let turn = if front { steer(n) } else { 0.0 };
                        poses[n]
                            * scaled(n)
                            * DMat4::from_translation(wheel.centre)
                            * DMat4::from_rotation_y(turn)
                    })
                    .collect();
                let angle: Vec<f64> = drive
                    .distance
                    .iter()
                    .map(|d| spin(*d, wheel.radius))
                    .collect();
                spun(
                    &outer,
                    &angle,
                    &[DMat4::from_translation(-wheel.centre)],
                    wheel.axle,
                    step,
                )
            }
            _ => (
                (0..count).map(|n| trs_from_matrix(sprung(n))).collect(),
                Vec::new(),
            ),
        };
        tracks.push(Track {
            node: node.name.clone(),
            keys,
            camera: None,
            focus: None,
            fov: None,
            between,
        });
    }
    tracks
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_recipe::Drift;

    fn timeline(frames: u32) -> Timeline {
        Timeline {
            fps: 60,
            frames,
            size: [1080, 1920],
            seed: 7,
        }
    }

    fn follow(speed: Vec<[f64; 2]>, drift: Option<Drift>) -> Follow {
        Follow {
            object: "hero".to_string(),
            route: "main".to_string(),
            lane: 1.5,
            start: 0.0,
            speed,
            drift,
            dynamics: None,
            ride: 0.0,
            root_motion: pfx_recipe::RootMotion::Ignore,
        }
    }

    fn straight() -> Route {
        Route::new(&[[0.0, 0.0, 0.0], [0.0, 0.0, 500.0]], false).unwrap()
    }

    #[test]
    fn constant_speed_travels_v_times_t() {
        let tl = timeline(120);
        let d = distances(&follow(vec![[0.0, 12.0]], None), &straight(), &tl);
        for (n, d) in d.iter().enumerate() {
            assert!((d - 12.0 * tl.time(n as u32)).abs() < 1e-9);
        }
        let ramp = distances(
            &follow(vec![[0.0, 0.0], [2.0, 20.0]], None),
            &straight(),
            &tl,
        );
        assert!((ramp[120] - 20.0).abs() < 1e-9);
        let held = distances(
            &follow(vec![[0.0, 100.0]], None),
            &straight(),
            &timeline(600),
        );
        assert_eq!(held[600], 500.0);
    }

    #[test]
    fn wheel_spin_is_distance_over_radius() {
        assert!((spin(10.0, 0.35) - 10.0 / 0.35).abs() < 1e-12);
        assert_eq!(spin(10.0, 0.0), 0.0);
    }

    #[test]
    fn a_straight_route_does_not_steer() {
        let route = straight();
        for d in [0.0, 10.0, 250.0, 499.0] {
            assert_eq!(steer(&route, d, 2.7), 0.0);
        }
    }

    #[test]
    fn a_drive_is_deterministic() {
        let drift = Some(Drift {
            amount: 0.6,
            period: 1.5,
            slip_deg: 8.0,
        });
        let f = follow(vec![[0.0, 10.0], [3.0, 30.0]], drift);
        let tl = timeline(600);
        let a = drive(&f, &straight(), &tl, (2.7, 1.6));
        let b = drive(&f, &straight(), &tl, (2.7, 1.6));
        assert_eq!(a, b);
        assert_ne!(a.pose[0], a.pose[300]);
    }
}
