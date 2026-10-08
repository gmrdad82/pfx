use std::collections::{BTreeMap, VecDeque};

use crate::{
    Board, Call, Delivery, Display, Error, Event, Hub, Lobby, LobbyFilter, LobbyKind, MAX_SAVE,
    Message, Overlay, PRESENCE_COUNT_MAX, Quota, Range, Sort, SteamApi, Upload, User, api_name,
    cloud_error, details_len, file_name, fits, lobby_key, lobby_size, lobby_value, message_len,
    presence_key, presence_value, save_len,
};

#[cfg(any(feature = "input", test))]
#[derive(Clone, Debug, Default)]
struct FakePad {
    kind: i32,
    set: Option<String>,
    digital: BTreeMap<String, bool>,
    analog: BTreeMap<String, [f32; 2]>,
    origins: BTreeMap<String, Vec<i32>>,
}

#[cfg(any(feature = "input", test))]
const ORIGIN_COUNT: i32 = 495;

pub const FAKE_USER: User = User(76_561_197_960_265_729);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stat {
    Int(i32),
    Float(f32),
}

#[derive(Debug)]
pub struct Fake {
    up: bool,
    achieved: BTreeMap<String, bool>,
    stats: BTreeMap<String, Stat>,
    files: BTreeMap<String, Vec<u8>>,
    presence: BTreeMap<String, String>,
    overlays: VecDeque<Overlay>,
    account_cloud: bool,
    app_cloud: bool,
    total: u64,
    stores: u32,
    hub: Hub,
    user: User,
    calls: u64,
    invites: Vec<Lobby>,
    #[cfg(any(feature = "input", test))]
    pads: BTreeMap<u64, FakePad>,
    #[cfg(any(feature = "input", test))]
    vibrations: Vec<(u64, u16, u16)>,
}

impl Fake {
    pub fn new() -> Self {
        Self::with_quota(u64::try_from(MAX_SAVE).unwrap_or(u64::MAX))
    }

    pub fn with_quota(total: u64) -> Self {
        Self::on_with_quota(&Hub::new(), FAKE_USER, total)
    }

    pub fn on(hub: &Hub, user: User) -> Self {
        Self::on_with_quota(hub, user, u64::try_from(MAX_SAVE).unwrap_or(u64::MAX))
    }

    fn on_with_quota(hub: &Hub, user: User, total: u64) -> Self {
        hub.world().arrive(user);
        Self {
            up: true,
            achieved: BTreeMap::new(),
            stats: BTreeMap::new(),
            files: BTreeMap::new(),
            presence: BTreeMap::new(),
            overlays: VecDeque::new(),
            account_cloud: true,
            app_cloud: true,
            total,
            stores: 0,
            hub: hub.clone(),
            user,
            calls: 0,
            invites: Vec::new(),
            #[cfg(any(feature = "input", test))]
            pads: BTreeMap::new(),
            #[cfg(any(feature = "input", test))]
            vibrations: Vec::new(),
        }
    }

    pub fn hub(&self) -> &Hub {
        &self.hub
    }

    #[cfg(any(feature = "input", test))]
    pub fn plug_pad(&mut self, handle: u64, kind: i32) {
        self.pads.insert(
            handle,
            FakePad {
                kind,
                ..FakePad::default()
            },
        );
    }

    #[cfg(any(feature = "input", test))]
    pub fn unplug_pad(&mut self, handle: u64) {
        self.pads.remove(&handle);
    }

    #[cfg(any(feature = "input", test))]
    pub fn press(&mut self, handle: u64, action: &str, down: bool) {
        if let Some(pad) = self.pads.get_mut(&handle) {
            pad.digital.insert(action.to_string(), down);
        }
    }

    #[cfg(any(feature = "input", test))]
    pub fn tilt(&mut self, handle: u64, action: &str, x: f32, y: f32) {
        if let Some(pad) = self.pads.get_mut(&handle) {
            pad.analog.insert(action.to_string(), [x, y]);
        }
    }

    #[cfg(any(feature = "input", test))]
    pub fn bind(&mut self, handle: u64, action: &str, origins: &[i32]) {
        if let Some(pad) = self.pads.get_mut(&handle) {
            pad.origins.insert(action.to_string(), origins.to_vec());
        }
    }

    #[cfg(any(feature = "input", test))]
    pub fn action_set(&self, handle: u64) -> Option<&str> {
        self.pads.get(&handle).and_then(|pad| pad.set.as_deref())
    }

    #[cfg(any(feature = "input", test))]
    pub fn vibrations(&self) -> &[(u64, u16, u16)] {
        &self.vibrations
    }

    #[cfg(any(feature = "input", test))]
    fn fake_pad(&self, handle: u64) -> Result<&FakePad, Error> {
        self.open()?;
        self.pads.get(&handle).ok_or(Error::Unknown)
    }

    pub fn invite_dialogs(&self) -> &[Lobby] {
        &self.invites
    }

    fn call(&mut self) -> Call {
        self.calls += 1;
        Call(self.calls)
    }

    fn post(&self, event: Event) {
        self.hub.world().push(self.user, event);
    }

    pub fn set_account_cloud(&mut self, enabled: bool) {
        self.account_cloud = enabled;
    }

    pub fn set_app_cloud(&mut self, enabled: bool) {
        self.app_cloud = enabled;
    }

    pub fn push_overlay(&mut self, open: bool) {
        if self.up {
            self.overlays.push_back(if open {
                Overlay::Opened
            } else {
                Overlay::Closed
            });
        }
    }

    pub fn presence(&self, key: &str) -> Option<&str> {
        self.presence.get(key).map(String::as_str)
    }

    pub fn stores(&self) -> u32 {
        self.stores
    }

    fn open(&self) -> Result<(), Error> {
        if self.up { Ok(()) } else { Err(Error::Closed) }
    }

    fn cloud(&self) -> Result<(), Error> {
        self.open()?;
        cloud_error(self.account_cloud, self.app_cloud)
    }

    fn used(&self) -> u64 {
        self.files.values().map(|bytes| bytes.len() as u64).sum()
    }

    fn available(&self) -> u64 {
        self.total.saturating_sub(self.used())
    }
}

impl Default for Fake {
    fn default() -> Self {
        Self::new()
    }
}

impl SteamApi for Fake {
    fn run_callbacks(&mut self) -> Vec<Event> {
        if !self.up {
            return Vec::new();
        }
        let mut events: Vec<Event> = self.overlays.drain(..).map(Event::Overlay).collect();
        events.extend(self.hub.world().take(self.user));
        events
    }

    fn shutdown(&mut self) {
        if self.up {
            self.hub.world().depart(self.user);
        }
        self.up = false;
        self.overlays.clear();
    }

    fn unlock(&mut self, name: &str) -> Result<(), Error> {
        self.open()?;
        api_name(name)?;
        self.achieved.insert(name.to_string(), true);
        Ok(())
    }

    fn achieved(&self, name: &str) -> Result<bool, Error> {
        self.open()?;
        api_name(name)?;
        Ok(self.achieved.get(name).copied().unwrap_or(false))
    }

    fn store(&mut self) -> Result<(), Error> {
        self.open()?;
        self.stores = self.stores.saturating_add(1);
        Ok(())
    }

    #[cfg(any(test, debug_assertions))]
    fn clear(&mut self, name: &str) -> Result<(), Error> {
        self.open()?;
        api_name(name)?;
        self.achieved.remove(name);
        Ok(())
    }

    fn stat_i32(&self, name: &str) -> Result<i32, Error> {
        self.open()?;
        api_name(name)?;
        match self.stats.get(name) {
            Some(Stat::Int(value)) => Ok(*value),
            Some(Stat::Float(_)) => Err(Error::Rejected),
            None => Ok(0),
        }
    }

    fn set_stat_i32(&mut self, name: &str, value: i32) -> Result<(), Error> {
        self.open()?;
        api_name(name)?;
        if matches!(self.stats.get(name), Some(Stat::Float(_))) {
            return Err(Error::Rejected);
        }
        self.stats.insert(name.to_string(), Stat::Int(value));
        Ok(())
    }

    fn stat_f32(&self, name: &str) -> Result<f32, Error> {
        self.open()?;
        api_name(name)?;
        match self.stats.get(name) {
            Some(Stat::Float(value)) => Ok(*value),
            Some(Stat::Int(_)) => Err(Error::Rejected),
            None => Ok(0.0),
        }
    }

    fn set_stat_f32(&mut self, name: &str, value: f32) -> Result<(), Error> {
        self.open()?;
        api_name(name)?;
        if !value.is_finite() || matches!(self.stats.get(name), Some(Stat::Int(_))) {
            return Err(Error::Rejected);
        }
        self.stats.insert(name.to_string(), Stat::Float(value));
        Ok(())
    }

    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<(), Error> {
        self.cloud()?;
        file_name(name)?;
        save_len(bytes.len())?;
        let old = self
            .files
            .get(name)
            .map(|stored| stored.len() as u64)
            .unwrap_or(0);
        fits(self.available(), old, bytes.len())?;
        self.files.insert(name.to_string(), bytes.to_vec());
        Ok(())
    }

    fn read(&self, name: &str) -> Result<Vec<u8>, Error> {
        self.cloud()?;
        file_name(name)?;
        self.files.get(name).cloned().ok_or(Error::Missing)
    }

    fn exists(&self, name: &str) -> Result<bool, Error> {
        self.cloud()?;
        file_name(name)?;
        Ok(self.files.contains_key(name))
    }

    fn delete(&mut self, name: &str) -> Result<(), Error> {
        self.cloud()?;
        file_name(name)?;
        if self.files.remove(name).is_none() {
            Err(Error::Missing)
        } else {
            Ok(())
        }
    }

    fn quota(&self) -> Result<Quota, Error> {
        self.cloud()?;
        Ok(Quota {
            total: self.total,
            available: self.available(),
        })
    }

    fn set_presence(&mut self, key: &str, value: &str) -> Result<(), Error> {
        self.open()?;
        presence_key(key)?;
        presence_value(value)?;
        if !self.presence.contains_key(key) && self.presence.len() >= PRESENCE_COUNT_MAX {
            return Err(Error::Rejected);
        }
        self.presence.insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn clear_presence(&mut self, key: &str) -> Result<(), Error> {
        self.open()?;
        presence_key(key)?;
        self.presence.remove(key);
        Ok(())
    }

    fn me(&self) -> Result<User, Error> {
        self.open()?;
        Ok(self.user)
    }

    fn server_time(&self) -> Result<Option<u64>, Error> {
        self.open()?;
        Ok(self.hub.world().server_time())
    }

    fn find_board(&mut self, name: &str) -> Result<Call, Error> {
        self.open()?;
        api_name(name)?;
        let call = self.call();
        let result = self.hub.world().find_board(name);
        self.post(Event::Board { call, result });
        Ok(call)
    }

    fn find_or_create_board(
        &mut self,
        name: &str,
        sort: Sort,
        display: Display,
    ) -> Result<Call, Error> {
        self.open()?;
        api_name(name)?;
        let call = self.call();
        let info = self.hub.world().create_board(name, sort, display);
        self.post(Event::Board {
            call,
            result: Ok(info),
        });
        Ok(call)
    }

    fn upload_score(
        &mut self,
        board: Board,
        score: i32,
        details: &[i32],
        upload: Upload,
    ) -> Result<Call, Error> {
        self.open()?;
        details_len(details)?;
        let call = self.call();
        let result = self
            .hub
            .world()
            .upload(self.user, board, score, details, upload);
        self.post(Event::Uploaded {
            call,
            board,
            result,
        });
        Ok(call)
    }

    fn download_scores(
        &mut self,
        board: Board,
        range: Range,
        details: usize,
    ) -> Result<Call, Error> {
        self.open()?;
        if !range.valid() {
            return Err(Error::Rejected);
        }
        let call = self.call();
        let result =
            self.hub
                .world()
                .download(self.user, board, range, details.min(crate::DETAILS_MAX));
        self.post(Event::Scores {
            call,
            board,
            result,
        });
        Ok(call)
    }

    fn share_file(&mut self, name: &str) -> Result<Call, Error> {
        self.cloud()?;
        file_name(name)?;
        let call = self.call();
        let result = match self.files.get(name) {
            Some(bytes) => Ok(self.hub.world().share(bytes.clone())),
            None => Err(Error::Missing),
        };
        self.post(Event::Shared { call, result });
        Ok(call)
    }

    fn attach_ugc(&mut self, board: Board, ugc: crate::Ugc) -> Result<(), Error> {
        self.open()?;
        self.hub.world().attach(self.user, board, ugc)
    }

    fn download_ugc(&mut self, ugc: crate::Ugc) -> Result<Call, Error> {
        self.open()?;
        let call = self.call();
        let result = self.hub.world().ugc(ugc);
        self.post(Event::UgcDownloaded { call, ugc, result });
        Ok(call)
    }

    fn create_lobby(&mut self, kind: LobbyKind, max_members: u32) -> Result<Call, Error> {
        self.open()?;
        lobby_size(max_members)?;
        let call = self.call();
        let lobby = self.hub.world().create_lobby(self.user, kind, max_members);
        self.post(Event::LobbyCreated {
            call,
            result: Ok(lobby),
        });
        Ok(call)
    }

    fn find_lobbies(&mut self, filter: &LobbyFilter) -> Result<Call, Error> {
        self.open()?;
        filter.check()?;
        let call = self.call();
        let found = self.hub.world().find_lobbies(filter);
        self.post(Event::Lobbies {
            call,
            result: Ok(found),
        });
        Ok(call)
    }

    fn join_lobby(&mut self, lobby: Lobby) -> Result<Call, Error> {
        self.open()?;
        let call = self.call();
        let result = self.hub.world().join(self.user, lobby);
        self.post(Event::Joined { call, result });
        Ok(call)
    }

    fn leave_lobby(&mut self, lobby: Lobby) -> Result<(), Error> {
        self.open()?;
        self.hub
            .world()
            .leave(self.user, lobby, crate::Change::Left);
        Ok(())
    }

    fn lobby_members(&self, lobby: Lobby) -> Result<Vec<User>, Error> {
        self.open()?;
        Ok(self.hub.world().members(self.user, lobby))
    }

    fn lobby_owner(&self, lobby: Lobby) -> Result<User, Error> {
        self.open()?;
        self.hub.world().owner(self.user, lobby)
    }

    fn set_lobby_data(&mut self, lobby: Lobby, key: &str, value: &str) -> Result<(), Error> {
        self.open()?;
        lobby_key(key)?;
        lobby_value(value)?;
        self.hub
            .world()
            .set_lobby_data(self.user, lobby, key, value)
    }

    fn lobby_data(&self, lobby: Lobby, key: &str) -> Result<Option<String>, Error> {
        self.open()?;
        lobby_key(key)?;
        Ok(self.hub.world().lobby_data(lobby, key))
    }

    fn invite_dialog(&mut self, lobby: Lobby) -> Result<(), Error> {
        self.open()?;
        if self.hub.world().members(self.user, lobby).is_empty() {
            return Err(Error::Rejected);
        }
        self.invites.push(lobby);
        Ok(())
    }

    fn launched(&mut self, args: &[String]) -> Option<Lobby> {
        let lobby = crate::connect_lobby(args)?;
        self.open().ok()?;
        self.post(Event::JoinRequested {
            lobby,
            friend: None,
        });
        Some(lobby)
    }

    fn send_message(
        &mut self,
        peer: User,
        channel: u32,
        bytes: &[u8],
        delivery: Delivery,
    ) -> Result<(), Error> {
        self.open()?;
        message_len(bytes.len())?;
        if peer == self.user {
            return Err(Error::Rejected);
        }
        self.hub
            .world()
            .send(self.user, peer, channel, bytes, delivery);
        Ok(())
    }

    fn receive_messages(&mut self, channel: u32, max: usize) -> Result<Vec<Message>, Error> {
        self.open()?;
        Ok(self.hub.world().receive(self.user, channel, max))
    }

    fn accept_session(&mut self, peer: User) -> Result<(), Error> {
        self.open()?;
        self.hub.world().accept(self.user, peer)
    }

    #[cfg(any(feature = "input", test))]
    fn input_frame(&mut self) -> Result<Vec<u64>, Error> {
        self.open()?;
        Ok(self.pads.keys().copied().collect())
    }

    #[cfg(any(feature = "input", test))]
    fn input_type(&mut self, pad: u64) -> Result<i32, Error> {
        Ok(self.fake_pad(pad)?.kind)
    }

    #[cfg(any(feature = "input", test))]
    fn activate_action_set(&mut self, pad: u64, set: &str) -> Result<(), Error> {
        self.fake_pad(pad)?;
        api_name(set)?;
        if let Some(state) = self.pads.get_mut(&pad) {
            state.set = Some(set.to_string());
        }
        Ok(())
    }

    #[cfg(any(feature = "input", test))]
    fn digital_action(&mut self, pad: u64, action: &str) -> Result<crate::Digital, Error> {
        let state = self.fake_pad(pad)?;
        let active = state.set.is_some();
        Ok(crate::Digital {
            state: active && state.digital.get(action).copied().unwrap_or(false),
            active,
        })
    }

    #[cfg(any(feature = "input", test))]
    fn analog_action(&mut self, pad: u64, action: &str) -> Result<crate::Analog, Error> {
        let state = self.fake_pad(pad)?;
        let active = state.set.is_some();
        let [x, y] = state.analog.get(action).copied().unwrap_or([0.0, 0.0]);
        Ok(crate::Analog { x, y, active })
    }

    #[cfg(any(feature = "input", test))]
    fn digital_origins(&mut self, pad: u64, _set: &str, action: &str) -> Result<Vec<i32>, Error> {
        Ok(self
            .fake_pad(pad)?
            .origins
            .get(action)
            .cloned()
            .unwrap_or_default())
    }

    #[cfg(any(feature = "input", test))]
    fn analog_origins(&mut self, pad: u64, set: &str, action: &str) -> Result<Vec<i32>, Error> {
        self.digital_origins(pad, set, action)
    }

    #[cfg(any(feature = "input", test))]
    fn trigger_vibration(&mut self, pad: u64, low: u16, high: u16) -> Result<(), Error> {
        self.fake_pad(pad)?;
        self.vibrations.push((pad, low, high));
        Ok(())
    }

    #[cfg(any(feature = "input", test))]
    fn steam_glyph(
        &mut self,
        origin: i32,
        glyph: crate::SteamGlyph,
    ) -> Result<Option<String>, Error> {
        self.open()?;
        if !(1..ORIGIN_COUNT).contains(&origin) {
            return Ok(None);
        }
        let style = glyph.flags();
        Ok(Some(match glyph.format {
            crate::GlyphFormat::Png(size) => {
                format!("steam/glyphs/{origin}-{style}-{size:?}.png").to_lowercase()
            }
            crate::GlyphFormat::Svg => format!("steam/glyphs/{origin}-{style}.svg"),
        }))
    }
}
