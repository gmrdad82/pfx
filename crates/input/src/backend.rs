use std::collections::VecDeque;

use crate::event::InputEvent;
use crate::rumble::MotorCommand;

pub trait Backend {
    fn poll(&mut self, events: &mut Vec<InputEvent>);

    fn set_motors(&mut self, command: &MotorCommand) -> bool;

    fn steam_input(&mut self, _active: bool) {}
}

#[derive(Clone, Debug, Default)]
pub struct Script {
    steps: VecDeque<Vec<InputEvent>>,
    pub motors: Vec<MotorCommand>,
}

impl Script {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, events: Vec<InputEvent>) {
        self.steps.push_back(events);
    }

    pub fn remaining(&self) -> usize {
        self.steps.len()
    }
}

impl Backend for Script {
    fn poll(&mut self, events: &mut Vec<InputEvent>) {
        if let Some(step) = self.steps.pop_front() {
            events.extend(step);
        }
    }

    fn set_motors(&mut self, command: &MotorCommand) -> bool {
        self.motors.push(*command);
        true
    }
}
