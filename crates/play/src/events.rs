use crate::world::{Contact, ObjectId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    Enter,
    Stay,
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TriggerEvent {
    pub phase: Phase,
    pub trigger: ObjectId,
    pub other: ObjectId,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WorldEvent {
    Contact(Contact),
    Trigger(TriggerEvent),
}
