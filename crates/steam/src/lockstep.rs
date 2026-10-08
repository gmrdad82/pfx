use std::collections::BTreeMap;

use crate::hash::fnv1a;
use crate::{Delivery, Error, SteamApi, User};

pub const LOCKSTEP_WINDOW: u64 = 256;
const INPUT: u8 = 1;
const SUM: u8 = 2;
const HEADER: usize = 9;
const BATCH: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub step: u64,
    pub inputs: [Vec<u8>; 2],
    pub check: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Desync {
    pub step: u64,
    pub mine: u64,
    pub theirs: u64,
}

#[derive(Clone, Debug)]
pub struct Lockstep {
    me: User,
    peer: User,
    channel: u32,
    every: u64,
    next: u64,
    sent: u64,
    local: BTreeMap<u64, Vec<u8>>,
    remote: BTreeMap<u64, Vec<u8>>,
    mine: BTreeMap<u64, u64>,
    theirs: BTreeMap<u64, u64>,
    desync: Option<Desync>,
}

pub fn checksum(bytes: &[u8]) -> u64 {
    pfx_core::sim::mix64(fnv1a(bytes))
}

fn packet(tag: u8, step: u64, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER + payload.len());
    bytes.push(tag);
    bytes.extend_from_slice(&step.to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

impl Lockstep {
    pub fn new(me: User, peer: User, channel: u32, every: u64) -> Result<Self, Error> {
        if me == peer || every == 0 {
            return Err(Error::Rejected);
        }
        Ok(Self {
            me,
            peer,
            channel,
            every,
            next: 0,
            sent: 0,
            local: BTreeMap::new(),
            remote: BTreeMap::new(),
            mine: BTreeMap::new(),
            theirs: BTreeMap::new(),
            desync: None,
        })
    }

    pub fn me(&self) -> User {
        self.me
    }

    pub fn peer(&self) -> User {
        self.peer
    }

    pub fn side(&self) -> usize {
        usize::from(self.me > self.peer)
    }

    pub fn step(&self) -> u64 {
        self.next
    }

    pub fn ahead(&self) -> u64 {
        self.sent - self.next
    }

    pub fn desync(&self) -> Option<Desync> {
        self.desync
    }

    pub fn is_check(&self, step: u64) -> bool {
        step % self.every == self.every - 1
    }

    pub fn send_input(&mut self, steam: &mut impl SteamApi, input: &[u8]) -> Result<u64, Error> {
        if self.ahead() >= LOCKSTEP_WINDOW {
            return Err(Error::Rejected);
        }
        let step = self.sent;
        steam.send_message(
            self.peer,
            self.channel,
            &packet(INPUT, step, input),
            Delivery::Reliable,
        )?;
        self.local.insert(step, input.to_vec());
        self.sent += 1;
        Ok(step)
    }

    pub fn send_checksum(
        &mut self,
        steam: &mut impl SteamApi,
        step: u64,
        sum: u64,
    ) -> Result<(), Error> {
        if !self.is_check(step) || step >= self.next {
            return Err(Error::Rejected);
        }
        steam.send_message(
            self.peer,
            self.channel,
            &packet(SUM, step, &sum.to_le_bytes()),
            Delivery::Reliable,
        )?;
        self.mine.insert(step, sum);
        self.compare(step);
        Ok(())
    }

    pub fn receive(&mut self, steam: &mut impl SteamApi) -> Result<usize, Error> {
        let mut taken = 0;
        loop {
            let batch = steam.receive_messages(self.channel, BATCH)?;
            if batch.is_empty() {
                return Ok(taken);
            }
            for message in batch {
                if message.peer == self.peer && self.accept(&message.bytes) {
                    taken += 1;
                }
            }
        }
    }

    pub fn accept(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() < HEADER {
            return false;
        }
        let mut word = [0u8; 8];
        word.copy_from_slice(&bytes[1..HEADER]);
        let step = u64::from_le_bytes(word);
        let payload = &bytes[HEADER..];
        if step >= self.next.saturating_add(2 * LOCKSTEP_WINDOW) {
            return false;
        }
        match bytes[0] {
            INPUT if step >= self.next && !self.remote.contains_key(&step) => {
                self.remote.insert(step, payload.to_vec());
                true
            }
            SUM if payload.len() == 8
                && self.is_check(step)
                && !self.theirs.contains_key(&step) =>
            {
                word.copy_from_slice(payload);
                self.theirs.insert(step, u64::from_le_bytes(word));
                self.compare(step);
                true
            }
            _ => false,
        }
    }

    pub fn turn(&mut self) -> Option<Turn> {
        if self.desync.is_some() {
            return None;
        }
        let step = self.next;
        if !self.local.contains_key(&step) || !self.remote.contains_key(&step) {
            return None;
        }
        let local = self.local.remove(&step).unwrap_or_default();
        let remote = self.remote.remove(&step).unwrap_or_default();
        self.next += 1;
        let floor = self.next.saturating_sub(2 * LOCKSTEP_WINDOW);
        self.mine = self.mine.split_off(&floor);
        self.theirs = self.theirs.split_off(&floor);
        let inputs = if self.side() == 0 {
            [local, remote]
        } else {
            [remote, local]
        };
        Some(Turn {
            step,
            inputs,
            check: self.is_check(step),
        })
    }

    fn compare(&mut self, step: u64) {
        let (Some(mine), Some(theirs)) = (self.mine.get(&step), self.theirs.get(&step)) else {
            return;
        };
        if mine != theirs && self.desync.is_none() {
            self.desync = Some(Desync {
                step,
                mine: *mine,
                theirs: *theirs,
            });
        }
        self.mine.remove(&step);
        self.theirs.remove(&step);
    }
}
