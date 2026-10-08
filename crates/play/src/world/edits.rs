use pfx_load::scene::{BodyShape, Projection, Target, Value};
use pfx_materials::Material;

use super::{ObjectId, World, body_volume, collider};
use crate::live::Edit;
use crate::math::{self, number};

fn numbers<const N: usize>(value: &Value) -> Option<[f32; N]> {
    let Value::Array(items) = value else {
        return None;
    };
    if items.len() != N {
        return None;
    }
    let mut out = [0.0; N];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = number(item)?;
    }
    Some(out)
}

fn toggle(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        _ => None,
    }
}

fn finite(value: f32) -> Option<f32> {
    value.is_finite().then_some(value)
}

fn at_least(value: &Value, low: f32) -> Option<f32> {
    number(value).and_then(finite).filter(|value| *value >= low)
}

fn above(value: &Value, low: f32) -> Option<f32> {
    number(value).and_then(finite).filter(|value| *value > low)
}

fn vector(value: &Value) -> Option<[f32; 3]> {
    numbers::<3>(value).filter(|parts| parts.iter().all(|part| part.is_finite()))
}

fn toml_of(value: &Value) -> toml::Value {
    match value {
        Value::Bool(value) => toml::Value::Boolean(*value),
        Value::Int(value) => toml::Value::Integer(*value),
        Value::Float(value) => toml::Value::Float(math::shortest(*value)),
        Value::Text(text) => toml::Value::String(text.clone()),
        Value::Array(items) => toml::Value::Array(items.iter().map(toml_of).collect()),
        Value::Table(entries) => toml::Value::Table(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), toml_of(value)))
                .collect(),
        ),
    }
}

pub(crate) fn set_key(
    material: Material,
    path: &[String],
    value: &Value,
) -> Result<Material, String> {
    let text = material.to_toml()?;
    let mut table: toml::Table = toml::from_str(&text).map_err(|error| error.to_string())?;
    let Some((last, parents)) = path.split_last() else {
        return Err("no key".to_string());
    };
    let mut at = &mut table;
    for key in parents {
        at = at
            .entry(key.clone())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| format!("{key} is not a table"))?;
    }
    at.insert(last.clone(), toml_of(value));
    let text = toml::to_string(&table).map_err(|error| error.to_string())?;
    Material::from_toml(&text)
}

fn sized(shape: BodyShape, key: &str, value: &Value) -> Option<BodyShape> {
    Some(match (shape, key) {
        (BodyShape::Box { .. }, "half") => BodyShape::Box {
            half: vector(value).filter(|half| half.iter().all(|part| *part > 0.0))?,
        },
        (BodyShape::Sphere { .. }, "radius") => BodyShape::Sphere {
            radius: above(value, 0.0)?,
        },
        (BodyShape::Capsule { half_height, .. }, "radius") => BodyShape::Capsule {
            half_height,
            radius: above(value, 0.0)?,
        },
        (BodyShape::Capsule { radius, .. }, "half_height") => BodyShape::Capsule {
            half_height: above(value, 0.0)?,
            radius,
        },
        (BodyShape::Cylinder { half_height, .. }, "radius") => BodyShape::Cylinder {
            half_height,
            radius: above(value, 0.0)?,
        },
        (BodyShape::Cylinder { radius, .. }, "half_height") => BodyShape::Cylinder {
            half_height: above(value, 0.0)?,
            radius,
        },
        _ => return None,
    })
}

impl World {
    pub fn apply(&mut self, edit: &Edit) -> Result<Edit, String> {
        match edit {
            Edit::Tunable { name, value } => {
                let value = self.set_tunable(name, *value)?;
                Ok(Edit::tunable(name, value))
            }
            Edit::Scene {
                target,
                path,
                value,
            } => {
                let refuse = |why: &str| Err(format!("{}: {why}", edit.key()));
                let key: Vec<&str> = path.iter().map(String::as_str).collect();
                let kept = match (target, key.as_slice()) {
                    (Target::Object(name), [key, rest @ ..]) => {
                        let Some(id) = self.object(name) else {
                            return refuse("no such object in the running world");
                        };
                        let kept = match (*key, rest) {
                            ("body", [key]) => self.edit_body(id, edit, key, value)?,
                            ("character", [key]) => {
                                self.edit_character(id, key, value)
                                    .map_err(|why| format!("{}: {why}", edit.key()))?;
                                edit.clone()
                            }
                            (_, []) => {
                                self.edit_object(id, key, value)
                                    .map_err(|why| format!("{}: {why}", edit.key()))?;
                                edit.clone()
                            }
                            _ => return refuse("not a live key"),
                        };
                        self.refresh_physics();
                        kept
                    }
                    (Target::Material(name), [_, ..]) => {
                        let Some(material) = self.material(name) else {
                            return refuse("no such material");
                        };
                        let changed = set_key(material, path, value)
                            .map_err(|why| format!("{}: {why}", edit.key()))?;
                        self.materials.insert(name.clone(), changed);
                        edit.clone()
                    }
                    (Target::Light(name), [key]) => {
                        let Some(mut light) = self.lights.get(name).copied() else {
                            return refuse("no such light");
                        };
                        match *key {
                            "position" => {
                                light.position = vector(value).ok_or("needs 3 numbers")?
                            }
                            "color" => light.color = vector(value).ok_or("needs 3 numbers")?,
                            "intensity" => {
                                light.intensity =
                                    at_least(value, 0.0).ok_or("needs a number of 0 or more")?
                            }
                            "radius" => {
                                light.radius =
                                    at_least(value, 0.0).ok_or("needs a number of 0 or more")?
                            }
                            "range" => {
                                light.range =
                                    at_least(value, 0.0).ok_or("needs a number of 0 or more")?
                            }
                            "shadow" => {
                                light.shadow = toggle(value).ok_or("needs true or false")?
                            }
                            _ => return refuse("not a live key"),
                        }
                        self.set_light(name, light);
                        edit.clone()
                    }
                    (Target::Camera, [key]) => {
                        self.edit_camera(key, value)
                            .map_err(|why| format!("{}: {why}", edit.key()))?;
                        edit.clone()
                    }
                    (Target::Finish, ["exposure"]) => {
                        let exposure = above(value, 0.0)
                            .ok_or_else(|| format!("{}: needs a number above 0", edit.key()))?;
                        self.set_exposure(exposure);
                        edit.clone()
                    }
                    (Target::Sound(name), [key]) => {
                        self.edit_sound(name, key, value)
                            .map_err(|why| format!("{}: {why}", edit.key()))?;
                        edit.clone()
                    }
                    _ => return refuse("not a live key"),
                };
                Ok(kept)
            }
        }
    }

    fn edit_object(&mut self, id: ObjectId, key: &str, value: &Value) -> Result<(), &'static str> {
        match key {
            "hidden" => {
                let hidden = toggle(value).ok_or("needs true or false")?;
                if !self.set_hidden(id, hidden) {
                    return Err("is hidden in the scene; a game shows it by declaring it visible");
                }
            }
            "material" => {
                let Value::Text(name) = value else {
                    return Err("needs a material name");
                };
                if self.material(name).is_none() {
                    return Err("names no material in the library");
                }
                let mut look = self.look(id).clone();
                look.material = Some(name.clone());
                self.set_look(id, look);
            }
            "at" | "rotate" | "scale" => {
                let parts = match (key, value) {
                    ("scale", value) if number(value).is_some() => {
                        number(value).map(|scale| [scale; 3])
                    }
                    (_, value) => vector(value),
                }
                .ok_or("needs 3 numbers")?;
                let object = &self.scene.objects[id.0];
                let rest = [object.at, object.rotate, object.scale];
                let pose = self.poses.entry(id.0).or_insert(rest);
                let slot = match key {
                    "at" => 0,
                    "rotate" => 1,
                    _ => 2,
                };
                pose[slot] = parts;
                let [at, rotate, scale] = *pose;
                self.objects[id.0].scale = scale;
                self.set_local(id, pfx_load::scene::model(at, rotate, scale));
                if let Some(physical) = self.bodies.get(&id.0) {
                    let world = self.model(id);
                    let body = physical.id;
                    self.physics.set_pose(body, math::pose(world));
                    self.physics.wake(body);
                }
            }
            _ => return Err("not a live key"),
        }
        Ok(())
    }

    fn edit_body(
        &mut self,
        id: ObjectId,
        edit: &Edit,
        key: &str,
        value: &Value,
    ) -> Result<Edit, String> {
        let refuse = |why: &str| format!("{}: {why}", edit.key());
        let Some(physical) = self.bodies.get(&id.0).copied() else {
            return Err(refuse("the object has no body"));
        };
        let mut desc = physical.desc;
        let mut kept = edit.clone();
        let mut rebuild = false;
        match key {
            "mass" => {
                let mass = above(value, 0.0).ok_or_else(|| refuse("needs a number above 0"))?;
                desc.density = mass / body_volume(desc.shape);
                if let Edit::Scene { target, .. } = edit {
                    kept = Edit::scene(target.clone(), &["body", "density"], desc.density);
                }
                rebuild = true;
            }
            "density" => {
                desc.density =
                    at_least(value, 0.0).ok_or_else(|| refuse("needs a number of 0 or more"))?;
                rebuild = true;
            }
            "friction" => {
                desc.friction =
                    at_least(value, 0.0).ok_or_else(|| refuse("needs a number of 0 or more"))?;
                rebuild = true;
            }
            "restitution" => {
                desc.restitution =
                    at_least(value, 0.0).ok_or_else(|| refuse("needs a number of 0 or more"))?;
                rebuild = true;
            }
            "offset" => {
                desc.offset = vector(value).ok_or_else(|| refuse("needs 3 numbers"))?;
                rebuild = true;
            }
            "half" | "radius" | "half_height" => {
                desc.shape = sized(desc.shape, key, value)
                    .ok_or_else(|| refuse("needs a size above 0 that the body's shape has"))?;
                rebuild = true;
            }
            "damping" => {
                let damping = numbers::<2>(value)
                    .filter(|parts| parts.iter().all(|part| part.is_finite() && *part >= 0.0))
                    .ok_or_else(|| refuse("needs 2 numbers of 0 or more"))?;
                desc.damping = damping;
                self.physics
                    .set_damping(physical.id, damping[0], damping[1]);
            }
            "gravity_scale" => {
                desc.gravity_scale = number(value)
                    .and_then(finite)
                    .ok_or_else(|| refuse("needs a number"))?;
                self.physics
                    .set_gravity_scale(physical.id, desc.gravity_scale);
            }
            "velocity" | "spin" => {
                let parts = vector(value).ok_or_else(|| refuse("needs 3 numbers"))?;
                let state = self
                    .physics
                    .body(physical.id)
                    .ok_or_else(|| refuse("the body is gone"))?;
                let (linear, angular) = if key == "velocity" {
                    desc.velocity = parts;
                    (parts, state.angular_velocity)
                } else {
                    desc.spin = parts;
                    (state.linear_velocity, parts.map(f32::to_radians))
                };
                self.physics.set_velocity(physical.id, linear, angular);
            }
            _ => return Err(refuse("not a live key")),
        }
        if rebuild {
            self.physics.remove_collider(physical.collider);
            self.owners.remove(&physical.collider);
            let made = match self
                .physics
                .add_collider(physical.id, collider(&desc).with_layers(physical.groups))
            {
                Some(made) => made,
                None => {
                    let back = self
                        .physics
                        .add_collider(
                            physical.id,
                            collider(&physical.desc).with_layers(physical.groups),
                        )
                        .ok_or_else(|| refuse("the body lost its shape"))?;
                    self.owners.insert(back, id.0);
                    if let Some(slot) = self.bodies.get_mut(&id.0) {
                        slot.collider = back;
                    }
                    return Err(refuse("the shape is not usable"));
                }
            };
            self.owners.insert(made, id.0);
            if let Some(slot) = self.bodies.get_mut(&id.0) {
                slot.collider = made;
            }
        }
        if let Some(slot) = self.bodies.get_mut(&id.0) {
            slot.desc = desc;
        }
        self.physics.wake(physical.id);
        Ok(kept)
    }

    fn edit_camera(&mut self, key: &str, value: &Value) -> Result<(), &'static str> {
        let camera = &mut self.camera;
        match key {
            "at" => camera.at = vector(value).ok_or("needs 3 numbers")?,
            "look_at" => camera.look_at = vector(value).ok_or("needs 3 numbers")?,
            "up" => camera.up = vector(value).ok_or("needs 3 numbers")?,
            "near" => camera.near = above(value, 0.0).ok_or("needs a number above 0")?,
            "far" => camera.far = above(value, 0.0).ok_or("needs a number above 0")?,
            "fov" => match &mut camera.projection {
                Projection::Perspective { fov } => {
                    *fov = above(value, 0.0)
                        .filter(|fov| *fov < 180.0)
                        .ok_or("needs degrees between 0 and 180")?
                }
                Projection::Orthographic { .. } => return Err("the camera is orthographic"),
            },
            "height" => match &mut camera.projection {
                Projection::Orthographic { height } => {
                    *height = above(value, 0.0).ok_or("needs a number above 0")?
                }
                Projection::Perspective { .. } => return Err("the camera is a perspective one"),
            },
            "fstop" | "focus" => {
                let Some(depth) = &mut camera.depth_of_field else {
                    return Err("the camera has no depth of field");
                };
                let amount = above(value, 0.0).ok_or("needs a number above 0")?;
                if key == "fstop" {
                    depth.fstop = amount;
                } else {
                    depth.distance = amount;
                }
            }
            _ => return Err("not a live key"),
        }
        Ok(())
    }

    fn edit_sound(&mut self, name: &str, key: &str, value: &Value) -> Result<(), &'static str> {
        let Some(level) = self.levels.get_mut(name) else {
            return Err("no such sound");
        };
        let voice = self.sounds.voices.get(name).copied();
        match key {
            "volume" => {
                level.volume = at_least(value, 0.0).ok_or("needs a number of 0 or more")?;
                if let Some(voice) = voice {
                    self.sounds.mixer.set_volume(voice, level.volume);
                }
            }
            "pan" => {
                level.pan = number(value)
                    .filter(|pan| (-1.0..=1.0).contains(pan))
                    .ok_or("needs a number from -1 to 1")?;
                if let Some(voice) = voice {
                    self.sounds.mixer.set_pan(voice, level.pan);
                }
            }
            "loop" => level.looping = toggle(value).ok_or("needs true or false")?,
            _ => return Err("not a live key"),
        }
        Ok(())
    }
}
