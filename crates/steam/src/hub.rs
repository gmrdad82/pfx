use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::board::split_run;
use crate::hash::draw;
use crate::{
    Board, BoardInfo, Change, Delivery, Display, Entry, Error, Event, Fake, Lobby, LobbyFilter,
    LobbyKind, Message, Range, Sort, Ugc, Upload, Uploaded, User,
};

#[derive(Clone, Debug, Default)]
pub struct Hub {
    world: Arc<Mutex<World>>,
}

#[derive(Debug, Default)]
pub(crate) struct World {
    online: BTreeSet<User>,
    events: BTreeMap<User, VecDeque<Event>>,
    server_time: Option<u64>,
    friends: BTreeSet<(User, User)>,
    boards: Vec<BoardState>,
    lobbies: BTreeMap<Lobby, LobbyState>,
    lobby_count: u64,
    sessions: BTreeMap<(User, User), Session>,
    mailbox: BTreeMap<(User, u32), VecDeque<Message>>,
    loss: Option<Loss>,
    tick: u64,
    ugc: BTreeMap<Ugc, Vec<u8>>,
    ugc_count: u64,
}

#[derive(Debug)]
struct BoardState {
    name: String,
    sort: Sort,
    display: Display,
    scores: BTreeMap<User, Scored>,
}

#[derive(Debug)]
struct Scored {
    score: i32,
    details: Vec<i32>,
    tick: u64,
    attached: Option<Ugc>,
}

#[derive(Debug)]
struct LobbyState {
    kind: LobbyKind,
    max: u32,
    owner: User,
    members: Vec<User>,
    data: BTreeMap<String, String>,
}

#[derive(Debug)]
enum Session {
    Requested {
        from: User,
        queued: Vec<(u32, Vec<u8>)>,
    },
    Open,
}

#[derive(Debug)]
struct Loss {
    state: u64,
    per_mille: u32,
}

fn pair(a: User, b: User) -> (User, User) {
    if a < b { (a, b) } else { (b, a) }
}

impl Hub {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn client(&self, user: User) -> Fake {
        Fake::on(self, user)
    }

    pub fn set_server_time(&self, seconds: Option<u64>) {
        self.world().server_time = seconds;
    }

    pub fn befriend(&self, a: User, b: User) {
        if a != b {
            self.world().friends.insert(pair(a, b));
        }
    }

    pub fn set_loss(&self, seed: u64, per_mille: u32) {
        self.world().loss = (per_mille > 0).then_some(Loss {
            state: seed,
            per_mille: per_mille.min(1000),
        });
    }

    pub fn invite(&self, from: User, to: User, lobby: Lobby) {
        self.world().push(
            to,
            Event::JoinRequested {
                lobby,
                friend: Some(from),
            },
        );
    }

    pub fn fail_session(&self, a: User, b: User, reason: Option<i32>) {
        let mut world = self.world();
        world.sessions.remove(&pair(a, b));
        world.push(a, Event::SessionFailed { peer: b, reason });
        world.push(b, Event::SessionFailed { peer: a, reason });
    }

    pub fn attached(&self, board: Board, user: User) -> Option<Ugc> {
        let world = self.world();
        let state = world.board(board).ok()?;
        state.scores.get(&user).and_then(|scored| scored.attached)
    }

    pub(crate) fn world(&self) -> MutexGuard<'_, World> {
        self.world.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl World {
    pub(crate) fn arrive(&mut self, user: User) {
        self.online.insert(user);
    }

    pub(crate) fn push(&mut self, user: User, event: Event) {
        if self.online.contains(&user) {
            self.events.entry(user).or_default().push_back(event);
        }
    }

    pub(crate) fn take(&mut self, user: User) -> Vec<Event> {
        self.events
            .get_mut(&user)
            .map(|queue| queue.drain(..).collect())
            .unwrap_or_default()
    }

    pub(crate) fn server_time(&self) -> Option<u64> {
        self.server_time
    }

    pub(crate) fn depart(&mut self, user: User) {
        let joined: Vec<Lobby> = self
            .lobbies
            .iter()
            .filter(|(_, lobby)| lobby.members.contains(&user))
            .map(|(id, _)| *id)
            .collect();
        for lobby in joined {
            self.leave(user, lobby, Change::Disconnected);
        }
        let peers: Vec<(User, User)> = self
            .sessions
            .keys()
            .filter(|(a, b)| *a == user || *b == user)
            .copied()
            .collect();
        for key in peers {
            self.sessions.remove(&key);
            let peer = if key.0 == user { key.1 } else { key.0 };
            self.push(
                peer,
                Event::SessionFailed {
                    peer: user,
                    reason: None,
                },
            );
        }
        self.online.remove(&user);
        self.events.remove(&user);
        self.mailbox.retain(|(owner, _), _| *owner != user);
    }

    fn info(&self, index: usize) -> BoardInfo {
        let board = &self.boards[index];
        BoardInfo {
            board: Board(index as u64 + 1),
            name: board.name.clone(),
            sort: board.sort,
            display: board.display,
            entries: board.scores.len() as u32,
        }
    }

    pub(crate) fn find_board(&self, name: &str) -> Result<BoardInfo, Error> {
        self.boards
            .iter()
            .position(|board| board.name == name)
            .map(|index| self.info(index))
            .ok_or(Error::Unknown)
    }

    pub(crate) fn create_board(&mut self, name: &str, sort: Sort, display: Display) -> BoardInfo {
        if let Ok(info) = self.find_board(name) {
            return info;
        }
        self.boards.push(BoardState {
            name: name.to_string(),
            sort,
            display,
            scores: BTreeMap::new(),
        });
        self.info(self.boards.len() - 1)
    }

    fn board(&self, board: Board) -> Result<&BoardState, Error> {
        usize::try_from(board.0)
            .ok()
            .and_then(|id| id.checked_sub(1))
            .and_then(|index| self.boards.get(index))
            .ok_or(Error::Unknown)
    }

    fn ranking(board: &BoardState) -> Vec<(User, &Scored)> {
        let mut rows: Vec<(User, &Scored)> = board
            .scores
            .iter()
            .map(|(user, scored)| (*user, scored))
            .collect();
        rows.sort_by(|(a_user, a), (b_user, b)| {
            let by_score = match board.sort {
                Sort::Ascending => a.score.cmp(&b.score),
                Sort::Descending => b.score.cmp(&a.score),
            };
            by_score.then(a.tick.cmp(&b.tick)).then(a_user.cmp(b_user))
        });
        rows
    }

    fn rank(board: &BoardState, user: User) -> u32 {
        Self::ranking(board)
            .iter()
            .position(|(who, _)| *who == user)
            .map(|index| index as u32 + 1)
            .unwrap_or(0)
    }

    pub(crate) fn upload(
        &mut self,
        user: User,
        board: Board,
        score: i32,
        details: &[i32],
        upload: Upload,
    ) -> Result<Uploaded, Error> {
        self.board(board)?;
        self.tick += 1;
        let tick = self.tick;
        let index = board.0 as usize - 1;
        let state = &mut self.boards[index];
        let previous_rank = Self::rank(state, user);
        let changed = match (upload, state.scores.get(&user)) {
            (Upload::Force, _) | (Upload::KeepBest, None) => true,
            (Upload::KeepBest, Some(old)) => state.sort.better(score, old.score),
        };
        if changed {
            state.scores.insert(
                user,
                Scored {
                    score,
                    details: details.to_vec(),
                    tick,
                    attached: None,
                },
            );
        }
        Ok(Uploaded {
            score,
            changed,
            rank: Self::rank(state, user),
            previous_rank,
        })
    }

    pub(crate) fn download(
        &self,
        user: User,
        board: Board,
        range: Range,
        details: usize,
    ) -> Result<Vec<Entry>, Error> {
        let state = self.board(board)?;
        let rows = Self::ranking(state);
        let picked: Vec<usize> = match range {
            Range::Global { first, last } => {
                let first = first as usize - 1;
                let last = (last as usize).min(rows.len());
                (first..last).collect()
            }
            Range::AroundUser { before, after } => {
                match rows.iter().position(|(who, _)| *who == user) {
                    Some(at) => {
                        let first = at.saturating_sub(before as usize);
                        let last = at.saturating_add(after as usize).min(rows.len() - 1);
                        (first..=last).collect()
                    }
                    None => Vec::new(),
                }
            }
            Range::Friends => (0..rows.len())
                .filter(|index| {
                    let who = rows[*index].0;
                    who == user || self.friends.contains(&pair(who, user))
                })
                .collect(),
        };
        Ok(picked
            .into_iter()
            .map(|index| {
                let (who, scored) = rows[index];
                let (run, details) = split_run(&scored.details, details);
                Entry {
                    user: who,
                    rank: index as u32 + 1,
                    score: scored.score,
                    details,
                    run,
                }
            })
            .collect())
    }

    pub(crate) fn share(&mut self, bytes: Vec<u8>) -> Ugc {
        self.ugc_count += 1;
        let ugc = Ugc(0x0100_0000_0000_0000 + self.ugc_count);
        self.ugc.insert(ugc, bytes);
        ugc
    }

    pub(crate) fn ugc(&self, ugc: Ugc) -> Result<Vec<u8>, Error> {
        self.ugc.get(&ugc).cloned().ok_or(Error::Missing)
    }

    pub(crate) fn attach(&mut self, user: User, board: Board, ugc: Ugc) -> Result<(), Error> {
        self.board(board)?;
        if !self.ugc.contains_key(&ugc) {
            return Err(Error::Missing);
        }
        let state = &mut self.boards[board.0 as usize - 1];
        let scored = state.scores.get_mut(&user).ok_or(Error::Rejected)?;
        scored.attached = Some(ugc);
        Ok(())
    }

    pub(crate) fn create_lobby(&mut self, user: User, kind: LobbyKind, max: u32) -> Lobby {
        self.lobby_count += 1;
        let lobby = Lobby(self.lobby_count);
        self.lobbies.insert(
            lobby,
            LobbyState {
                kind,
                max,
                owner: user,
                members: vec![user],
                data: BTreeMap::new(),
            },
        );
        lobby
    }

    pub(crate) fn find_lobbies(&self, filter: &LobbyFilter) -> Vec<Lobby> {
        let mut found: Vec<(Vec<u64>, Lobby)> = self
            .lobbies
            .iter()
            .filter(|(_, state)| matches!(state.kind, LobbyKind::Public | LobbyKind::Invisible))
            .filter(|(_, state)| {
                let open = state.max.saturating_sub(state.members.len() as u32);
                filter.matches(&state.data, open)
            })
            .map(|(lobby, state)| (filter.nearness(&state.data), *lobby))
            .collect();
        found.sort();
        found
            .into_iter()
            .map(|(_, lobby)| lobby)
            .take(filter.limit())
            .collect()
    }

    pub(crate) fn join(&mut self, user: User, lobby: Lobby) -> Result<Lobby, Error> {
        let state = self.lobbies.get_mut(&lobby).ok_or(Error::Rejected)?;
        if state.members.contains(&user) {
            return Ok(lobby);
        }
        if state.members.len() as u32 >= state.max {
            return Err(Error::Rejected);
        }
        let others = state.members.clone();
        state.members.push(user);
        for member in others {
            self.push(
                member,
                Event::Member {
                    lobby,
                    user,
                    change: Change::Entered,
                },
            );
        }
        Ok(lobby)
    }

    pub(crate) fn leave(&mut self, user: User, lobby: Lobby, change: Change) {
        let Some(state) = self.lobbies.get_mut(&lobby) else {
            return;
        };
        let Some(at) = state.members.iter().position(|member| *member == user) else {
            return;
        };
        state.members.remove(at);
        if state.members.is_empty() {
            self.lobbies.remove(&lobby);
            return;
        }
        if state.owner == user {
            state.owner = state.members[0];
        }
        let others = state.members.clone();
        for member in others {
            self.push(
                member,
                Event::Member {
                    lobby,
                    user,
                    change,
                },
            );
        }
    }

    fn member_of(&self, user: User, lobby: Lobby) -> Option<&LobbyState> {
        self.lobbies
            .get(&lobby)
            .filter(|state| state.members.contains(&user))
    }

    pub(crate) fn members(&self, user: User, lobby: Lobby) -> Vec<User> {
        self.member_of(user, lobby)
            .map(|state| state.members.clone())
            .unwrap_or_default()
    }

    pub(crate) fn owner(&self, user: User, lobby: Lobby) -> Result<User, Error> {
        self.member_of(user, lobby)
            .map(|state| state.owner)
            .ok_or(Error::Rejected)
    }

    pub(crate) fn set_lobby_data(
        &mut self,
        user: User,
        lobby: Lobby,
        key: &str,
        value: &str,
    ) -> Result<(), Error> {
        let state = self
            .lobbies
            .get_mut(&lobby)
            .filter(|state| state.owner == user)
            .ok_or(Error::Rejected)?;
        if value.is_empty() {
            state.data.remove(key);
        } else {
            state.data.insert(key.to_string(), value.to_string());
        }
        let members = state.members.clone();
        for member in members {
            self.push(member, Event::LobbyData { lobby });
        }
        Ok(())
    }

    pub(crate) fn lobby_data(&self, lobby: Lobby, key: &str) -> Option<String> {
        self.lobbies
            .get(&lobby)
            .and_then(|state| state.data.get(key).cloned())
    }

    fn dropped(&mut self) -> bool {
        let Some(loss) = self.loss.as_mut() else {
            return false;
        };
        draw(&mut loss.state) % 1000 < u64::from(loss.per_mille)
    }

    fn deliver(&mut self, from: User, to: User, channel: u32, bytes: Vec<u8>) {
        self.mailbox
            .entry((to, channel))
            .or_default()
            .push_back(Message {
                peer: from,
                channel,
                bytes,
            });
    }

    fn open(&mut self, from: User, to: User, queued: Vec<(u32, Vec<u8>)>) {
        self.sessions.insert(pair(from, to), Session::Open);
        for (channel, bytes) in queued {
            self.deliver(from, to, channel, bytes);
        }
        self.push(from, Event::Connected { peer: to });
        self.push(to, Event::Connected { peer: from });
    }

    pub(crate) fn send(
        &mut self,
        user: User,
        peer: User,
        channel: u32,
        bytes: &[u8],
        delivery: Delivery,
    ) {
        if !self.online.contains(&peer) {
            self.push(user, Event::SessionFailed { peer, reason: None });
            return;
        }
        if delivery == Delivery::Unreliable && self.dropped() {
            return;
        }
        let key = pair(user, peer);
        match self.sessions.remove(&key) {
            None => {
                self.sessions.insert(
                    key,
                    Session::Requested {
                        from: user,
                        queued: vec![(channel, bytes.to_vec())],
                    },
                );
                self.push(peer, Event::SessionRequest { peer: user });
            }
            Some(Session::Requested { from, mut queued }) if from == user => {
                queued.push((channel, bytes.to_vec()));
                self.sessions
                    .insert(key, Session::Requested { from, queued });
            }
            Some(Session::Requested { queued, .. }) => {
                self.open(peer, user, queued);
                self.deliver(user, peer, channel, bytes.to_vec());
            }
            Some(Session::Open) => {
                self.sessions.insert(key, Session::Open);
                self.deliver(user, peer, channel, bytes.to_vec());
            }
        }
    }

    pub(crate) fn accept(&mut self, user: User, peer: User) -> Result<(), Error> {
        let key = pair(user, peer);
        match self.sessions.remove(&key) {
            Some(Session::Requested { from, queued }) if from == peer => {
                self.open(peer, user, queued);
                Ok(())
            }
            Some(session) => {
                self.sessions.insert(key, session);
                Err(Error::Missing)
            }
            None => Err(Error::Missing),
        }
    }

    pub(crate) fn receive(&mut self, user: User, channel: u32, max: usize) -> Vec<Message> {
        let Some(queue) = self.mailbox.get_mut(&(user, channel)) else {
            return Vec::new();
        };
        let count = max.min(queue.len());
        queue.drain(..count).collect()
    }
}
