use crate::{Change, Delivery, Error, Event, Lobby, SteamApi, User};

pub const RECONNECT_MS: u64 = 120_000;
const BEAT: u8 = 1;
const BYE: u8 = 2;
const BATCH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lost {
    Silent,
    SessionFailed(Option<i32>),
    Disconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quit {
    Goodbye,
    Left,
    Kicked,
    Banned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    Quit(Quit),
    Forfeit(Lost),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Link {
    Up,
    Lost { since: u64, lost: Lost },
    Ended(Ending),
}

#[derive(Clone, Debug)]
pub struct Health {
    peer: User,
    lobby: Option<Lobby>,
    channel: u32,
    every_ms: u64,
    timeout_ms: u64,
    window_ms: u64,
    sent: Option<u64>,
    heard: u64,
    link: Link,
}

impl Health {
    pub fn new(
        peer: User,
        channel: u32,
        every_ms: u64,
        timeout_ms: u64,
        now_ms: u64,
    ) -> Result<Self, Error> {
        if every_ms == 0 || timeout_ms <= every_ms {
            return Err(Error::Rejected);
        }
        Ok(Self {
            peer,
            lobby: None,
            channel,
            every_ms,
            timeout_ms,
            window_ms: RECONNECT_MS,
            sent: None,
            heard: now_ms,
            link: Link::Up,
        })
    }

    pub fn in_lobby(mut self, lobby: Lobby) -> Self {
        self.lobby = Some(lobby);
        self
    }

    pub fn reconnect_window(mut self, window_ms: u64) -> Self {
        self.window_ms = window_ms;
        self
    }

    pub fn peer(&self) -> User {
        self.peer
    }

    pub fn lost(&self) -> Option<Lost> {
        match self.link {
            Link::Lost { lost, .. } => Some(lost),
            _ => None,
        }
    }

    pub fn ended(&self) -> Option<Ending> {
        match self.link {
            Link::Ended(ending) => Some(ending),
            _ => None,
        }
    }

    pub fn heard(&mut self, now_ms: u64) -> Option<Event> {
        self.heard = self.heard.max(now_ms);
        match self.link {
            Link::Lost { .. } => {
                self.link = Link::Up;
                Some(Event::Reconnected { peer: self.peer })
            }
            _ => None,
        }
    }

    pub fn tick(&mut self, steam: &mut impl SteamApi, now_ms: u64) -> Result<Vec<Event>, Error> {
        let mut events = Vec::new();
        if self.ended().is_some() {
            return Ok(events);
        }
        let peer = self.peer;
        loop {
            let batch = steam.receive_messages(self.channel, BATCH)?;
            if batch.is_empty() {
                break;
            }
            for message in batch.into_iter().filter(|message| message.peer == peer) {
                if message.bytes == [BYE] {
                    events.push(self.quit(Quit::Goodbye));
                    return Ok(events);
                }
                events.extend(self.heard(now_ms));
            }
        }
        if self
            .sent
            .is_none_or(|sent| now_ms.saturating_sub(sent) >= self.every_ms)
        {
            steam.send_message(self.peer, self.channel, &[BEAT], Delivery::Reliable)?;
            self.sent = Some(now_ms);
        }
        match self.link {
            Link::Up if now_ms.saturating_sub(self.heard) >= self.timeout_ms => {
                events.extend(self.lose(Lost::Silent, now_ms));
            }
            Link::Lost { since, lost } if now_ms.saturating_sub(since) >= self.window_ms => {
                self.link = Link::Ended(Ending::Forfeit(lost));
                events.push(Event::Forfeit { peer, lost });
            }
            _ => {}
        }
        Ok(events)
    }

    pub fn observe(&mut self, event: &Event, now_ms: u64) -> Option<Event> {
        if self.ended().is_some() {
            return None;
        }
        match event {
            Event::SessionFailed { peer, reason } if *peer == self.peer => {
                self.lose(Lost::SessionFailed(*reason), now_ms)
            }
            Event::Member {
                lobby,
                user,
                change,
            } if *user == self.peer && Some(*lobby) == self.lobby => match change {
                Change::Entered => self.heard(now_ms),
                Change::Disconnected => self.lose(Lost::Disconnected, now_ms),
                Change::Left => Some(self.quit(Quit::Left)),
                Change::Kicked => Some(self.quit(Quit::Kicked)),
                Change::Banned => Some(self.quit(Quit::Banned)),
            },
            _ => None,
        }
    }

    pub fn leave(&mut self, steam: &mut impl SteamApi) -> Result<(), Error> {
        if self.ended().is_some() {
            return Ok(());
        }
        self.link = Link::Ended(Ending::Quit(Quit::Goodbye));
        steam.send_message(self.peer, self.channel, &[BYE], Delivery::Reliable)
    }

    fn lose(&mut self, lost: Lost, now_ms: u64) -> Option<Event> {
        if self.link != Link::Up {
            return None;
        }
        self.link = Link::Lost {
            since: now_ms,
            lost,
        };
        Some(Event::ConnectionLost {
            peer: self.peer,
            lost,
        })
    }

    fn quit(&mut self, quit: Quit) -> Event {
        self.link = Link::Ended(Ending::Quit(quit));
        Event::PeerQuit {
            peer: self.peer,
            quit,
        }
    }
}

pub fn connect_lobby(args: &[String]) -> Option<Lobby> {
    args.windows(2)
        .find(|pair| pair[0] == "+connect_lobby")
        .and_then(|pair| pair[1].parse::<u64>().ok())
        .filter(|id| *id != 0)
        .map(Lobby)
}
