use serde::{Deserialize, Serialize};

use crate::backend::Backend;
use crate::device::{MouseButton, PadId};
use crate::event::InputEvent;
use crate::rumble::MotorCommand;
use crate::state::{ActionState, Active};

pub const RECORDING_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Start {
    pub events: Vec<InputEvent>,
    pub states: Vec<ActionState>,
    pub active: Active,
    pub last_pad: Option<PadId>,
    pub anchor: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outside: Vec<MouseButton>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    pub version: u32,
    pub step_us: u32,
    pub start: Start,
    pub steps: Vec<Vec<InputEvent>>,
}

impl Recording {
    pub fn new(step_us: u32, start: Start) -> Self {
        Self {
            version: RECORDING_VERSION,
            step_us,
            start,
            steps: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn player(&self) -> Player<'_> {
        Player {
            recording: self,
            at: 0,
            motors: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Player<'a> {
    recording: &'a Recording,
    at: usize,
    pub motors: Vec<MotorCommand>,
}

impl Player<'_> {
    pub fn at(&self) -> usize {
        self.at
    }

    pub fn finished(&self) -> bool {
        self.at >= self.recording.steps.len()
    }
}

impl Backend for Player<'_> {
    fn poll(&mut self, events: &mut Vec<InputEvent>) {
        if let Some(step) = self.recording.steps.get(self.at) {
            events.extend(step.iter().cloned());
        }
        self.at += 1;
    }

    fn set_motors(&mut self, command: &MotorCommand) -> bool {
        self.motors.push(*command);
        false
    }
}
