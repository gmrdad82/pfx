use pfx_load::scene::Value;
use pfx_physics::rigid::{Character, CharacterDesc};

use super::{ObjectId, World};
use crate::PlayError;
use crate::characters::{self, Setting};

impl World {
    pub fn character(&mut self, id: ObjectId) -> Result<&mut Character, PlayError> {
        let place = id.index();
        if !self.characters.contains_key(&place) {
            let name = self.name(id).to_string();
            let desc = *self.roster.get(&name).ok_or_else(|| {
                PlayError::Body(format!("object {name} has no [object.character]"))
            })?;
            if self.bodies.contains_key(&place) {
                return Err(PlayError::Body(format!(
                    "object {name} has a body; a character brings its own capsule"
                )));
            }
            let feet = self.at(id);
            let character = Character::spawn(&mut self.physics, desc, feet)
                .map_err(|why| PlayError::Body(format!("character {name}: {why}")))?;
            self.owners.insert(character.collider(), place);
            self.characters.insert(place, character);
        }
        let objects = &self.objects;
        self.characters.get_mut(&place).ok_or_else(|| {
            PlayError::Body(format!("character {} did not spawn", objects[place].name))
        })
    }

    pub fn set_character_desc(
        &mut self,
        id: ObjectId,
        desc: CharacterDesc,
    ) -> Result<(), PlayError> {
        let name = self.name(id).to_string();
        let place = id.index();
        let refuse = |why: String| PlayError::Body(format!("character {name}: {why}"));
        match self.characters.get_mut(&place) {
            Some(character) => {
                let old = character.collider();
                character
                    .set_desc(&mut self.physics, desc)
                    .map_err(refuse)?;
                let new = character.collider();
                if new != old {
                    self.owners.remove(&old);
                    self.owners.insert(new, place);
                }
            }
            None => desc.check().map_err(refuse)?,
        }
        self.roster.insert(name, desc);
        Ok(())
    }

    pub fn character_of(&self, id: ObjectId) -> Option<&Character> {
        self.characters.get(&id.index())
    }

    pub fn characters(&self) -> impl Iterator<Item = (ObjectId, &Character)> {
        self.characters
            .iter()
            .map(|(place, character)| (ObjectId::from_index(*place), character))
    }

    pub fn character_desc(&self, id: ObjectId) -> Option<&CharacterDesc> {
        self.roster.get(self.name(id))
    }

    pub fn remove_character(&mut self, id: ObjectId) -> bool {
        match self.characters.remove(&id.index()) {
            Some(character) => {
                self.owners.remove(&character.collider());
                character.remove(&mut self.physics)
            }
            None => false,
        }
    }

    pub(crate) fn step_characters(&mut self) {
        for character in self.characters.values_mut() {
            character.step(&mut self.physics);
        }
    }

    pub(crate) fn place_characters(&mut self) {
        let placed: Vec<(usize, [f32; 3])> = self
            .characters
            .iter()
            .map(|(place, character)| (*place, character.feet()))
            .collect();
        for (place, feet) in placed {
            self.move_to(ObjectId::from_index(place), feet);
        }
    }

    pub(crate) fn edit_character(
        &mut self,
        id: ObjectId,
        key: &str,
        value: &Value,
    ) -> Result<(), String> {
        let name = self.name(id).to_string();
        let mut desc = *self
            .roster
            .get(&name)
            .ok_or_else(|| format!("object {name} has no [object.character]"))?;
        let setting = Setting::of_value(value).ok_or("not a value")?;
        characters::set(&mut desc, key, &setting)?;
        self.set_character_desc(id, desc)
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}
