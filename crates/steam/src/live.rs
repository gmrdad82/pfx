use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, PoisonError};

use steamworks::networking_messages::SessionRequest;
use steamworks::networking_types::{NetworkingConnectionState, NetworkingIdentity, SendFlags};
use steamworks::{
    AppId, CallbackHandle, CallbackResult, ChatMemberStateChange, Client, ComparisonFilter,
    DistanceFilter, FileType, Leaderboard, LeaderboardDataRequest, LeaderboardDisplayType,
    LeaderboardEntry, LeaderboardScoreUploaded, LeaderboardSortMethod, LobbyId, LobbyKey,
    LobbyListFilter, LobbyType, NearFilter, NumberFilter, PublishedFileId, PublishedFileVisibility,
    SResult, SteamAPIInitError, SteamError, SteamId, StringFilter, StringFilterKind, UpdateStatus,
    UpdateWatchHandle, UploadScoreMethod,
};

use crate::board::split_run;
use crate::{
    Board, BoardInfo, Call, Change, Compare, Created, DETAILS_MAX, Delivery, Display, Distance,
    Entry, Error, Event, Installed, Item, ItemState, ItemUpdate, Lobby, LobbyFilter, LobbyKind,
    Message, Overlay, PRESENCE_COUNT_MAX, Progress, Quota, RUN_FIELDS, Range, Sort, Stage,
    SteamApi, Ugc, Upload, Uploaded, User, Visibility, api_name, cloud_error, details_len,
    file_name, fits, lobby_key, lobby_size, lobby_value, message_len, presence_key, presence_value,
    save_len,
};

enum Done {
    Board(Call, SResult<Option<Leaderboard>>),
    Uploaded(Call, Board, SResult<Option<LeaderboardScoreUploaded>>),
    Scores(Call, Board, usize, SResult<Vec<LeaderboardEntry>>),
    Shared(Call, SResult<u64>),
    Created(Call, SResult<LobbyId>),
    Lobbies(Call, SResult<Vec<LobbyId>>),
    Joined(Call, Result<LobbyId, ()>),
    Request(SessionRequest),
    Installed(Item),
    ItemCreated(Call, SResult<(PublishedFileId, bool)>),
    ItemSubmitted(Call, SResult<(PublishedFileId, bool)>),
}

struct ItemInstalled {
    app: u32,
    item: u64,
}

unsafe impl steamworks::Callback for ItemInstalled {
    const ID: i32 = steamworks::sys::ItemInstalled_t_k_iCallback as i32;

    unsafe fn from_raw(raw: *mut std::ffi::c_void) -> Self {
        let installed = unsafe {
            raw.cast::<steamworks::sys::ItemInstalled_t>()
                .read_unaligned()
        };
        Self {
            app: installed.m_unAppID,
            item: installed.m_nPublishedFileId,
        }
    }
}

type Completed = Arc<Mutex<Vec<Done>>>;

pub struct Steam {
    client: Option<Client>,
    app_id: u32,
    presence: BTreeSet<String>,
    done: Completed,
    calls: u64,
    boards: BTreeMap<u64, Leaderboard>,
    requests: BTreeMap<u64, SessionRequest>,
    peers: BTreeMap<u64, bool>,
    pending: Vec<Event>,
    downloads: Vec<(Call, Ugc, u32)>,
    items: BTreeMap<u64, Option<Progress>>,
    submits: BTreeMap<u64, UpdateWatchHandle>,
    installed: Option<CallbackHandle>,
    #[cfg(any(feature = "input", test))]
    handles: Handles,
}

#[cfg(any(feature = "input", test))]
#[derive(Default)]
struct Handles {
    up: bool,
    sets: BTreeMap<String, u64>,
    digital: BTreeMap<String, u64>,
    analog: BTreeMap<String, u64>,
    origins: BTreeMap<i32, steamworks::sys::EInputActionOrigin>,
}

impl Steam {
    pub fn init(app_id: u32) -> Result<Self, Error> {
        if app_id == 0 {
            return Err(Error::AppId);
        }
        match Client::init_app(app_id) {
            Ok(client) => {
                let done: Completed = Arc::default();
                let requests = done.clone();
                client
                    .networking_messages()
                    .session_request_callback(move |request| {
                        post(&requests, Done::Request(request));
                    });
                let items = done.clone();
                let installed = client.register_callback(move |installed: ItemInstalled| {
                    if installed.app == app_id {
                        post(&items, Done::Installed(Item(installed.item)));
                    }
                });
                Ok(Self {
                    client: Some(client),
                    app_id,
                    presence: BTreeSet::new(),
                    done,
                    calls: 0,
                    boards: BTreeMap::new(),
                    requests: BTreeMap::new(),
                    peers: BTreeMap::new(),
                    pending: Vec::new(),
                    downloads: Vec::new(),
                    items: BTreeMap::new(),
                    submits: BTreeMap::new(),
                    installed: Some(installed),
                    #[cfg(any(feature = "input", test))]
                    handles: Handles::default(),
                })
            }
            Err(SteamAPIInitError::NoSteamClient(_)) => Err(Error::NotRunning),
            Err(SteamAPIInitError::FailedGeneric(message))
            | Err(SteamAPIInitError::VersionMismatch(message)) => {
                Err(Error::Init(init_message(message)))
            }
        }
    }

    pub fn app_id(&self) -> u32 {
        self.app_id
    }

    fn client(&self) -> Result<&Client, Error> {
        self.client.as_ref().ok_or(Error::Closed)
    }

    fn call(&mut self) -> Result<(Call, Completed), Error> {
        self.client()?;
        self.calls += 1;
        Ok((Call(self.calls), self.done.clone()))
    }

    fn board_info(&mut self, client: &Client, board: Leaderboard) -> BoardInfo {
        let stats = client.user_stats();
        let info = BoardInfo {
            board: Board(board.raw()),
            name: stats.get_leaderboard_name(&board),
            sort: match stats.get_leaderboard_sort_method(&board) {
                Some(LeaderboardSortMethod::Ascending) => Sort::Ascending,
                _ => Sort::Descending,
            },
            display: match stats.get_leaderboard_display_type(&board) {
                Some(LeaderboardDisplayType::TimeSeconds) => Display::Seconds,
                Some(LeaderboardDisplayType::TimeMilliSeconds) => Display::Milliseconds,
                _ => Display::Numeric,
            },
            entries: u32::try_from(stats.get_leaderboard_entry_count(&board)).unwrap_or(0),
        };
        self.boards.insert(board.raw(), board);
        info
    }

    #[cfg(any(feature = "input", test))]
    fn input(&mut self) -> Result<steamworks::Input, Error> {
        let input = self.client()?.input();
        if !self.handles.up {
            if !input.init(true) {
                return Err(Error::Rejected);
            }
            self.handles.up = true;
        }
        Ok(input)
    }

    #[cfg(any(feature = "input", test))]
    fn set_handle(&mut self, set: &str) -> Result<u64, Error> {
        api_name(set)?;
        let input = self.input()?;
        handle(&mut self.handles.sets, set, |name| {
            input.get_action_set_handle(name)
        })
    }

    #[cfg(any(feature = "input", test))]
    fn digital_handle(&mut self, action: &str) -> Result<u64, Error> {
        api_name(action)?;
        let input = self.input()?;
        handle(&mut self.handles.digital, action, |name| {
            input.get_digital_action_handle(name)
        })
    }

    #[cfg(any(feature = "input", test))]
    fn analog_handle(&mut self, action: &str) -> Result<u64, Error> {
        api_name(action)?;
        let input = self.input()?;
        handle(&mut self.handles.analog, action, |name| {
            input.get_analog_action_handle(name)
        })
    }

    #[cfg(any(feature = "input", test))]
    fn remember(&mut self, origin: steamworks::sys::EInputActionOrigin) -> i32 {
        let number = origin as i32;
        self.handles.origins.insert(number, origin);
        number
    }

    fn leaderboard(&self, board: Board) -> Result<&Leaderboard, Error> {
        self.boards.get(&board.0).ok_or(Error::Unknown)
    }

    fn finish(&mut self, client: &Client, done: Done) -> Option<Event> {
        Some(match done {
            Done::Board(call, result) => Event::Board {
                call,
                result: match result {
                    Ok(Some(board)) => Ok(self.board_info(client, board)),
                    Ok(None) => Err(Error::Unknown),
                    Err(error) => Err(steam_error(error)),
                },
            },
            Done::Uploaded(call, board, result) => Event::Uploaded {
                call,
                board,
                result: match result {
                    Ok(Some(uploaded)) => Ok(Uploaded {
                        score: uploaded.score,
                        changed: uploaded.was_changed,
                        rank: rank(uploaded.global_rank_new),
                        previous_rank: rank(uploaded.global_rank_previous),
                    }),
                    Ok(None) => Err(Error::Rejected),
                    Err(error) => Err(steam_error(error)),
                },
            },
            Done::Scores(call, board, keep, result) => Event::Scores {
                call,
                board,
                result: result
                    .map(|entries| {
                        entries
                            .into_iter()
                            .map(|entry| {
                                let (run, details) = split_run(&entry.details, keep);
                                Entry {
                                    user: User(entry.user.raw()),
                                    rank: rank(entry.global_rank),
                                    score: entry.score,
                                    details,
                                    run,
                                }
                            })
                            .collect()
                    })
                    .map_err(steam_error),
            },
            Done::Shared(call, result) => Event::Shared {
                call,
                result: result.map(Ugc).map_err(steam_error),
            },
            Done::Created(call, result) => Event::LobbyCreated {
                call,
                result: result.map(|lobby| Lobby(lobby.raw())).map_err(steam_error),
            },
            Done::Lobbies(call, result) => Event::Lobbies {
                call,
                result: result
                    .map(|lobbies| lobbies.into_iter().map(|id| Lobby(id.raw())).collect())
                    .map_err(steam_error),
            },
            Done::Joined(call, result) => Event::Joined {
                call,
                result: result
                    .map(|lobby| Lobby(lobby.raw()))
                    .map_err(|()| Error::Rejected),
            },
            Done::Request(request) => {
                let peer = request.remote().steam_id()?.raw();
                self.requests.insert(peer, request);
                Event::SessionRequest { peer: User(peer) }
            }
            Done::Installed(item) => {
                self.items.remove(&item.0);
                Event::ItemInstalled { item }
            }
            Done::ItemCreated(call, result) => Event::ItemCreated {
                call,
                result: created(result),
            },
            Done::ItemSubmitted(call, result) => {
                self.submits.remove(&call.0);
                Event::ItemSubmitted {
                    call,
                    result: created(result),
                }
            }
        })
    }

    fn poll_items(&mut self, client: &Client, events: &mut Vec<Event>) {
        let ugc = client.ugc();
        for (item, reported) in self.items.iter_mut() {
            let Some((done, total)) = ugc.item_download_info(PublishedFileId(*item)) else {
                continue;
            };
            let progress = Some(Progress { done, total });
            if total > 0 && progress != *reported {
                *reported = progress;
                events.push(Event::ItemProgress {
                    item: Item(*item),
                    progress: Progress { done, total },
                });
            }
        }
    }

    fn poll_downloads(&mut self, events: &mut Vec<Event>) {
        if self.downloads.is_empty() {
            return;
        }
        let remote = unsafe { steamworks::sys::SteamAPI_SteamRemoteStorage_v016() };
        let mut waiting = Vec::new();
        for (call, ugc, frames) in std::mem::take(&mut self.downloads) {
            let result = if remote.is_null() {
                Some(Err(Error::Rejected))
            } else {
                read_ugc(remote, ugc)
            };
            match result {
                Some(result) => events.push(Event::UgcDownloaded { call, ugc, result }),
                None if frames >= UGC_FRAMES => events.push(Event::UgcDownloaded {
                    call,
                    ugc,
                    result: Err(Error::Offline),
                }),
                None => waiting.push((call, ugc, frames + 1)),
            }
        }
        self.downloads = waiting;
    }

    fn poll_peers(&mut self, client: &Client, events: &mut Vec<Event>) {
        let messages = client.networking_messages();
        for (peer, connected) in self.peers.iter_mut() {
            let identity = NetworkingIdentity::new_steam_id(SteamId::from_raw(*peer));
            let (state, _, _) = messages.get_session_connection_info(&identity);
            let now = state == NetworkingConnectionState::Connected;
            if now && !*connected {
                events.push(Event::Connected { peer: User(*peer) });
            }
            *connected = now;
        }
    }
}

#[cfg(any(feature = "input", test))]
fn handle(
    cache: &mut BTreeMap<String, u64>,
    name: &str,
    find: impl FnOnce(&str) -> u64,
) -> Result<u64, Error> {
    if let Some(found) = cache.get(name) {
        return Ok(*found);
    }
    match find(name) {
        0 => Err(Error::Unknown),
        found => {
            cache.insert(name.to_string(), found);
            Ok(found)
        }
    }
}

#[cfg(any(feature = "input", test))]
fn input_type(kind: steamworks::InputType) -> i32 {
    use pfx_input::family as steam;
    use steamworks::InputType;
    match kind {
        InputType::Unknown => steam::STEAM_INPUT_UNKNOWN,
        InputType::SteamController => steam::STEAM_INPUT_STEAM_CONTROLLER,
        InputType::XBox360Controller => steam::STEAM_INPUT_XBOX_360,
        InputType::XBoxOneController => steam::STEAM_INPUT_XBOX_ONE,
        InputType::GenericGamepad => steam::STEAM_INPUT_GENERIC,
        InputType::PS4Controller => steam::STEAM_INPUT_PS4,
        InputType::AppleMFiController => steam::STEAM_INPUT_APPLE_MFI,
        InputType::AndroidController => steam::STEAM_INPUT_ANDROID,
        InputType::SwitchJoyConPair => steam::STEAM_INPUT_SWITCH_JOYCON_PAIR,
        InputType::SwitchJoyConSingle => steam::STEAM_INPUT_SWITCH_JOYCON_SINGLE,
        InputType::SwitchProController => steam::STEAM_INPUT_SWITCH_PRO,
        InputType::MobileTouch => steam::STEAM_INPUT_MOBILE_TOUCH,
        InputType::PS3Controller => steam::STEAM_INPUT_PS3,
        InputType::PS5Controller => steam::STEAM_INPUT_PS5,
        InputType::SteamDeckController => steam::STEAM_INPUT_STEAM_DECK,
    }
}

const UGC_FRAMES: u32 = 36_000;

fn read_ugc(
    remote: *mut steamworks::sys::ISteamRemoteStorage,
    ugc: Ugc,
) -> Option<Result<Vec<u8>, Error>> {
    let mut app = 0;
    let mut name = std::ptr::null_mut();
    let mut size = 0i32;
    let mut owner: steamworks::sys::CSteamID = unsafe { std::mem::zeroed() };
    let ready = unsafe {
        steamworks::sys::SteamAPI_ISteamRemoteStorage_GetUGCDetails(
            remote, ugc.0, &mut app, &mut name, &mut size, &mut owner,
        )
    };
    if !ready {
        return None;
    }
    let Ok(len) = usize::try_from(size) else {
        return Some(Err(Error::Rejected));
    };
    if len > crate::BLOB_MAX {
        return Some(Err(Error::TooBig));
    }
    let mut bytes = vec![0u8; len];
    let read = unsafe {
        steamworks::sys::SteamAPI_ISteamRemoteStorage_UGCRead(
            remote,
            ugc.0,
            bytes.as_mut_ptr().cast(),
            size,
            0,
            steamworks::sys::EUGCReadAction::k_EUGCRead_ContinueReadingUntilFinished,
        )
    };
    match usize::try_from(read) {
        Ok(read) if read == len => Some(Ok(bytes)),
        _ => Some(Err(Error::Rejected)),
    }
}

fn post(completed: &Completed, done: Done) {
    completed
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(done);
}

fn created(result: SResult<(PublishedFileId, bool)>) -> Result<Created, Error> {
    result
        .map(|(item, agreement)| Created {
            item: Item(item.0),
            agreement,
        })
        .map_err(steam_error)
}

fn visibility(visibility: Visibility) -> PublishedFileVisibility {
    match visibility {
        Visibility::Public => PublishedFileVisibility::Public,
        Visibility::Friends => PublishedFileVisibility::FriendsOnly,
        Visibility::Private => PublishedFileVisibility::Private,
        Visibility::Unlisted => PublishedFileVisibility::Unlisted,
    }
}

fn steam_error(error: SteamError) -> Error {
    match error {
        SteamError::IOFailure | SteamError::Timeout | SteamError::NoConnection => Error::Offline,
        _ => Error::Rejected,
    }
}

fn rank(rank: i32) -> u32 {
    u32::try_from(rank).unwrap_or(0)
}

fn compare(compare: Compare) -> ComparisonFilter {
    match compare {
        Compare::Equal => ComparisonFilter::Equal,
        Compare::NotEqual => ComparisonFilter::NotEqual,
        Compare::Less => ComparisonFilter::LessThan,
        Compare::LessOrEqual => ComparisonFilter::LessThanEqualTo,
        Compare::Greater => ComparisonFilter::GreaterThan,
        Compare::GreaterOrEqual => ComparisonFilter::GreaterThanEqualTo,
    }
}

fn string_compare(compare: Compare) -> StringFilterKind {
    match compare {
        Compare::Equal => StringFilterKind::Equal,
        Compare::NotEqual => StringFilterKind::NotEqual,
        Compare::Less => StringFilterKind::LessThan,
        Compare::LessOrEqual => StringFilterKind::EqualToOrLessThan,
        Compare::Greater => StringFilterKind::GreaterThan,
        Compare::GreaterOrEqual => StringFilterKind::EqualToOrGreaterThan,
    }
}

fn key(key: &str) -> Result<LobbyKey<'_>, Error> {
    LobbyKey::try_new(key).map_err(|_| Error::Rejected)
}

fn change(change: ChatMemberStateChange) -> Change {
    match change {
        ChatMemberStateChange::Entered => Change::Entered,
        ChatMemberStateChange::Left => Change::Left,
        ChatMemberStateChange::Disconnected => Change::Disconnected,
        ChatMemberStateChange::Kicked => Change::Kicked,
        ChatMemberStateChange::Banned => Change::Banned,
    }
}

const UNDECODED_MAX: usize = 64;

fn drain(mut run: impl FnMut(), mut free: impl FnMut()) -> usize {
    let mut skipped = 0;
    while skipped < UNDECODED_MAX && catch_unwind(AssertUnwindSafe(&mut run)).is_err() {
        free();
        skipped += 1;
    }
    skipped
}

fn free_last_callback() {
    unsafe {
        steamworks::sys::SteamAPI_ManualDispatch_FreeLastCallback(
            steamworks::sys::SteamAPI_GetHSteamPipe(),
        );
    }
}

impl SteamApi for Steam {
    fn run_callbacks(&mut self) -> Vec<Event> {
        let Some(client) = self.client.clone() else {
            return Vec::new();
        };
        let app_id = self.app_id;
        let mut events = std::mem::take(&mut self.pending);
        let mut failed = Vec::new();
        let skipped = drain(
            || {
                client.process_callbacks(|callback| match callback {
                    CallbackResult::GameOverlayActivated(overlay) => {
                        events.push(Event::Overlay(if overlay.active {
                            Overlay::Opened
                        } else {
                            Overlay::Closed
                        }));
                    }
                    CallbackResult::LobbyChatUpdate(update) => events.push(Event::Member {
                        lobby: Lobby(update.lobby.raw()),
                        user: User(update.user_changed.raw()),
                        change: change(update.member_state_change),
                    }),
                    CallbackResult::LobbyDataUpdate(update)
                        if update.member.raw() == update.lobby.raw() =>
                    {
                        events.push(Event::LobbyData {
                            lobby: Lobby(update.lobby.raw()),
                        });
                    }
                    CallbackResult::GameLobbyJoinRequested(request) => {
                        events.push(Event::JoinRequested {
                            lobby: Lobby(request.lobby_steam_id.raw()),
                            friend: Some(User(request.friend_steam_id.raw())),
                        });
                    }
                    CallbackResult::DownloadItemResult(result) if result.app_id.0 == app_id => {
                        events.push(Event::ItemDownloaded {
                            item: Item(result.published_file_id.0),
                            result: result.error.map_or(Ok(()), |error| Err(steam_error(error))),
                        });
                    }
                    CallbackResult::NetworkingMessagesSessionFailed(failure) => {
                        if let Some(peer) = failure
                            .info
                            .identity_remote()
                            .and_then(|identity| identity.steam_id())
                        {
                            failed.push(Event::SessionFailed {
                                peer: User(peer.raw()),
                                reason: failure.info.end_reason().map(i32::from),
                            });
                        }
                    }
                    _ => {}
                })
            },
            free_last_callback,
        );
        events.extend(std::iter::repeat_n(Event::Unrecognised, skipped));
        let finished =
            std::mem::take(&mut *self.done.lock().unwrap_or_else(PoisonError::into_inner));
        for done in finished {
            if let Some(event) = self.finish(&client, done) {
                events.push(event);
            }
        }
        for event in &failed {
            if let Event::SessionFailed { peer, .. } = event {
                self.peers.insert(peer.0, false);
            }
        }
        events.extend(failed);
        for event in &events {
            if let Event::ItemDownloaded { item, .. } = event {
                self.items.remove(&item.0);
            }
        }
        self.poll_peers(&client, &mut events);
        self.poll_downloads(&mut events);
        self.poll_items(&client, &mut events);
        events
    }

    fn shutdown(&mut self) {
        #[cfg(any(feature = "input", test))]
        if self.handles.up
            && let Some(client) = self.client.as_ref()
        {
            client.input().shutdown();
        }
        #[cfg(any(feature = "input", test))]
        {
            self.handles = Handles::default();
        }
        self.requests.clear();
        self.pending.clear();
        self.downloads.clear();
        self.items.clear();
        self.submits.clear();
        self.installed.take();
        self.boards.clear();
        self.peers.clear();
        self.done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.client.take();
        self.presence.clear();
    }

    fn unlock(&mut self, name: &str) -> Result<(), Error> {
        let client = self.client()?;
        api_name(name)?;
        client
            .user_stats()
            .achievement(name)
            .set()
            .map_err(|_| Error::Unknown)
    }

    fn achieved(&self, name: &str) -> Result<bool, Error> {
        let client = self.client()?;
        api_name(name)?;
        client
            .user_stats()
            .achievement(name)
            .get()
            .map_err(|_| Error::Unknown)
    }

    fn store(&mut self) -> Result<(), Error> {
        let client = self.client()?;
        client
            .user_stats()
            .store_stats()
            .map_err(|_| Error::Rejected)
    }

    #[cfg(any(test, debug_assertions))]
    fn clear(&mut self, name: &str) -> Result<(), Error> {
        let client = self.client()?;
        api_name(name)?;
        client
            .user_stats()
            .achievement(name)
            .clear()
            .map_err(|_| Error::Unknown)
    }

    fn stat_i32(&self, name: &str) -> Result<i32, Error> {
        let client = self.client()?;
        api_name(name)?;
        client
            .user_stats()
            .get_stat_i32(name)
            .map_err(|_| Error::Unknown)
    }

    fn set_stat_i32(&mut self, name: &str, value: i32) -> Result<(), Error> {
        let client = self.client()?;
        api_name(name)?;
        client
            .user_stats()
            .set_stat_i32(name, value)
            .map_err(|_| Error::Unknown)
    }

    fn stat_f32(&self, name: &str) -> Result<f32, Error> {
        let client = self.client()?;
        api_name(name)?;
        client
            .user_stats()
            .get_stat_f32(name)
            .map_err(|_| Error::Unknown)
    }

    fn set_stat_f32(&mut self, name: &str, value: f32) -> Result<(), Error> {
        let client = self.client()?;
        api_name(name)?;
        if !value.is_finite() {
            return Err(Error::Rejected);
        }
        client
            .user_stats()
            .set_stat_f32(name, value)
            .map_err(|_| Error::Unknown)
    }

    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<(), Error> {
        let client = self.client()?;
        let storage = client.remote_storage();
        cloud_error(
            storage.is_cloud_enabled_for_account(),
            storage.is_cloud_enabled_for_app(),
        )?;
        file_name(name)?;
        save_len(bytes.len())?;
        let old = file_size(&storage, name);
        let quota = read_quota()?;
        fits(quota.available, old, bytes.len())?;
        let mut writer = storage.file(name).write();
        let wrote = writer.write_all(bytes);
        drop(writer);
        wrote.map_err(|_| Error::Rejected)
    }

    fn read(&self, name: &str) -> Result<Vec<u8>, Error> {
        let client = self.client()?;
        file_name(name)?;
        let storage = client.remote_storage();
        cloud_error(
            storage.is_cloud_enabled_for_account(),
            storage.is_cloud_enabled_for_app(),
        )?;
        if !storage.file(name).exists() {
            return Err(Error::Missing);
        }
        let mut reader = storage.file(name).read();
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Rejected)?;
        Ok(bytes)
    }

    fn exists(&self, name: &str) -> Result<bool, Error> {
        let client = self.client()?;
        file_name(name)?;
        let storage = client.remote_storage();
        cloud_error(
            storage.is_cloud_enabled_for_account(),
            storage.is_cloud_enabled_for_app(),
        )?;
        Ok(storage.file(name).exists())
    }

    fn delete(&mut self, name: &str) -> Result<(), Error> {
        let client = self.client()?;
        file_name(name)?;
        let storage = client.remote_storage();
        cloud_error(
            storage.is_cloud_enabled_for_account(),
            storage.is_cloud_enabled_for_app(),
        )?;
        let file = storage.file(name);
        if !file.exists() {
            return Err(Error::Missing);
        }
        if file.delete() {
            Ok(())
        } else {
            Err(Error::Rejected)
        }
    }

    fn quota(&self) -> Result<Quota, Error> {
        let client = self.client()?;
        let storage = client.remote_storage();
        cloud_error(
            storage.is_cloud_enabled_for_account(),
            storage.is_cloud_enabled_for_app(),
        )?;
        read_quota()
    }

    fn set_presence(&mut self, key: &str, value: &str) -> Result<(), Error> {
        presence_key(key)?;
        presence_value(value)?;
        let known = self.presence.contains(key);
        if !known && self.presence.len() >= PRESENCE_COUNT_MAX {
            return Err(Error::Rejected);
        }
        let accepted = self.client()?.friends().set_rich_presence(key, Some(value));
        if !accepted {
            return Err(Error::Rejected);
        }
        self.presence.insert(key.to_string());
        Ok(())
    }

    fn clear_presence(&mut self, key: &str) -> Result<(), Error> {
        presence_key(key)?;
        let accepted = self.client()?.friends().set_rich_presence(key, None);
        if !accepted {
            return Err(Error::Rejected);
        }
        self.presence.remove(key);
        Ok(())
    }

    fn me(&self) -> Result<User, Error> {
        Ok(User(self.client()?.user().steam_id().raw()))
    }

    fn server_time(&self) -> Result<Option<u64>, Error> {
        let seconds = self.client()?.utils().get_server_real_time();
        Ok((seconds != 0).then_some(u64::from(seconds)))
    }

    fn find_board(&mut self, name: &str) -> Result<Call, Error> {
        api_name(name)?;
        let (call, done) = self.call()?;
        self.client()?
            .user_stats()
            .find_leaderboard(name, move |result| post(&done, Done::Board(call, result)));
        Ok(call)
    }

    fn find_or_create_board(
        &mut self,
        name: &str,
        sort: Sort,
        display: Display,
    ) -> Result<Call, Error> {
        api_name(name)?;
        let (call, done) = self.call()?;
        let sort = match sort {
            Sort::Ascending => LeaderboardSortMethod::Ascending,
            Sort::Descending => LeaderboardSortMethod::Descending,
        };
        let display = match display {
            Display::Numeric => LeaderboardDisplayType::Numeric,
            Display::Seconds => LeaderboardDisplayType::TimeSeconds,
            Display::Milliseconds => LeaderboardDisplayType::TimeMilliSeconds,
        };
        self.client()?.user_stats().find_or_create_leaderboard(
            name,
            sort,
            display,
            move |result| post(&done, Done::Board(call, result)),
        );
        Ok(call)
    }

    fn upload_score(
        &mut self,
        board: Board,
        score: i32,
        details: &[i32],
        upload: Upload,
    ) -> Result<Call, Error> {
        details_len(details)?;
        self.leaderboard(board)?;
        let (call, done) = self.call()?;
        let method = match upload {
            Upload::KeepBest => UploadScoreMethod::KeepBest,
            Upload::Force => UploadScoreMethod::ForceUpdate,
        };
        self.client()?.user_stats().upload_leaderboard_score(
            self.leaderboard(board)?,
            method,
            score,
            details,
            move |result| post(&done, Done::Uploaded(call, board, result)),
        );
        Ok(call)
    }

    fn download_scores(
        &mut self,
        board: Board,
        range: Range,
        details: usize,
    ) -> Result<Call, Error> {
        let details = details.min(DETAILS_MAX);
        if !range.valid() {
            return Err(Error::Rejected);
        }
        self.leaderboard(board)?;
        let (call, done) = self.call()?;
        let (request, first, last) = match range {
            Range::Global { first, last } => (
                LeaderboardDataRequest::Global,
                first as usize,
                last as usize,
            ),
            Range::AroundUser { before, after } => (
                LeaderboardDataRequest::GlobalAroundUser,
                (before as usize).wrapping_neg(),
                after as usize,
            ),
            Range::Friends => (LeaderboardDataRequest::Friends, 0, 0),
        };
        self.client()?.user_stats().download_leaderboard_entries(
            self.leaderboard(board)?,
            request,
            first,
            last,
            (details + RUN_FIELDS).min(DETAILS_MAX),
            move |result| post(&done, Done::Scores(call, board, details, result)),
        );
        Ok(call)
    }

    fn share_file(&mut self, name: &str) -> Result<Call, Error> {
        file_name(name)?;
        let storage = self.client()?.remote_storage();
        cloud_error(
            storage.is_cloud_enabled_for_account(),
            storage.is_cloud_enabled_for_app(),
        )?;
        if !storage.file(name).exists() {
            return Err(Error::Missing);
        }
        let (call, done) = self.call()?;
        storage
            .file(name)
            .share(move |result| post(&done, Done::Shared(call, result)));
        Ok(call)
    }

    fn attach_ugc(&mut self, board: Board, ugc: Ugc) -> Result<(), Error> {
        self.client()?;
        let board = self.leaderboard(board)?.raw();
        let stats = unsafe { steamworks::sys::SteamAPI_SteamUserStats_v013() };
        if stats.is_null() {
            return Err(Error::Rejected);
        }
        unsafe {
            steamworks::sys::SteamAPI_ISteamUserStats_AttachLeaderboardUGC(stats, board, ugc.0)
        };
        Ok(())
    }

    fn subscribed_items(&self) -> Result<Vec<Item>, Error> {
        let mut items: Vec<Item> = self
            .client()?
            .ugc()
            .subscribed_items(false)
            .into_iter()
            .map(|item| Item(item.0))
            .collect();
        items.sort();
        Ok(items)
    }

    fn item_state(&self, item: Item) -> Result<ItemState, Error> {
        let state = self.client()?.ugc().item_state(PublishedFileId(item.0));
        Ok(ItemState {
            subscribed: state.contains(steamworks::ItemState::SUBSCRIBED),
            installed: state.contains(steamworks::ItemState::INSTALLED),
            needs_update: state.contains(steamworks::ItemState::NEEDS_UPDATE),
            downloading: state.contains(steamworks::ItemState::DOWNLOADING),
            pending: state.contains(steamworks::ItemState::DOWNLOAD_PENDING),
        })
    }

    fn item_progress(&self, item: Item) -> Result<Option<Progress>, Error> {
        Ok(self
            .client()?
            .ugc()
            .item_download_info(PublishedFileId(item.0))
            .map(|(done, total)| Progress { done, total }))
    }

    fn item_installed(&self, item: Item) -> Result<Option<Installed>, Error> {
        Ok(self
            .client()?
            .ugc()
            .item_install_info(PublishedFileId(item.0))
            .filter(|info| !info.folder.is_empty())
            .map(|info| Installed {
                folder: info.folder.into(),
                bytes: info.size_on_disk,
                updated: info.timestamp,
            }))
    }

    fn download_item(&mut self, item: Item, now: bool) -> Result<(), Error> {
        if !self
            .client()?
            .ugc()
            .download_item(PublishedFileId(item.0), now)
        {
            return Err(Error::Rejected);
        }
        self.items.entry(item.0).or_insert(None);
        Ok(())
    }

    fn create_item(&mut self) -> Result<Call, Error> {
        let app = AppId(self.app_id);
        let (call, done) = self.call()?;
        self.client()?
            .ugc()
            .create_item(app, FileType::Community, move |result| {
                post(&done, Done::ItemCreated(call, result));
            });
        Ok(call)
    }

    fn submit_item(&mut self, item: Item, update: &ItemUpdate) -> Result<Call, Error> {
        update.check()?;
        let app = AppId(self.app_id);
        let (call, done) = self.call()?;
        let ugc = self.client()?.ugc();
        let watch = catch_unwind(AssertUnwindSafe(|| {
            let mut handle = ugc.start_item_update(app, PublishedFileId(item.0));
            if let Some(title) = &update.title {
                handle = handle.title(title);
            }
            if let Some(description) = &update.description {
                handle = handle.description(description);
            }
            if let Some(content) = &update.content {
                handle = handle.content_path(content);
            }
            if let Some(preview) = &update.preview {
                handle = handle.preview_path(preview);
            }
            if let Some(shown) = update.visibility {
                handle = handle.visibility(visibility(shown));
            }
            if let Some(tags) = &update.tags {
                handle = handle.tags(tags.clone(), false);
            }
            handle.submit(update.note.as_deref(), move |result| {
                post(&done, Done::ItemSubmitted(call, result));
            })
        }))
        .map_err(|_| Error::Rejected)?;
        self.submits.insert(call.0, watch);
        Ok(call)
    }

    fn submit_progress(&self, call: Call) -> Result<Option<(Stage, Progress)>, Error> {
        self.client()?;
        let Some(watch) = self.submits.get(&call.0) else {
            return Ok(None);
        };
        let (status, done, total) = watch.progress();
        let stage = match status {
            UpdateStatus::Invalid => return Ok(None),
            UpdateStatus::PreparingConfig => Stage::Config,
            UpdateStatus::PreparingContent => Stage::Content,
            UpdateStatus::UploadingContent => Stage::Uploading,
            UpdateStatus::UploadingPreviewFile => Stage::Preview,
            UpdateStatus::CommittingChanges => Stage::Committing,
        };
        Ok(Some((stage, Progress { done, total })))
    }

    fn download_ugc(&mut self, ugc: Ugc) -> Result<Call, Error> {
        let (call, _) = self.call()?;
        let remote = unsafe { steamworks::sys::SteamAPI_SteamRemoteStorage_v016() };
        if remote.is_null() {
            return Err(Error::Rejected);
        }
        unsafe { steamworks::sys::SteamAPI_ISteamRemoteStorage_UGCDownload(remote, ugc.0, 0) };
        self.downloads.push((call, ugc, 0));
        Ok(call)
    }

    fn create_lobby(&mut self, kind: LobbyKind, max_members: u32) -> Result<Call, Error> {
        lobby_size(max_members)?;
        let (call, done) = self.call()?;
        let kind = match kind {
            LobbyKind::Private => LobbyType::Private,
            LobbyKind::FriendsOnly => LobbyType::FriendsOnly,
            LobbyKind::Public => LobbyType::Public,
            LobbyKind::Invisible => LobbyType::Invisible,
        };
        self.client()?
            .matchmaking()
            .create_lobby(kind, max_members, move |result| {
                post(&done, Done::Created(call, result))
            });
        Ok(call)
    }

    fn find_lobbies(&mut self, filter: &LobbyFilter) -> Result<Call, Error> {
        filter.check()?;
        let strings = filter
            .strings
            .iter()
            .map(|(name, value, how)| Ok(StringFilter(key(name)?, value, string_compare(*how))))
            .collect::<Result<Vec<_>, Error>>()?;
        let numbers = filter
            .numbers
            .iter()
            .map(|(name, value, how)| Ok(NumberFilter(key(name)?, *value, compare(*how))))
            .collect::<Result<Vec<_>, Error>>()?;
        let near = filter
            .near
            .iter()
            .map(|(name, value)| Ok(NearFilter(key(name)?, *value)))
            .collect::<Result<Vec<_>, Error>>()?;
        let (call, done) = self.call()?;
        let list = LobbyListFilter {
            string: (!strings.is_empty()).then_some(strings),
            number: (!numbers.is_empty()).then_some(numbers),
            near_value: (!near.is_empty()).then_some(near),
            open_slots: filter.open_slots,
            distance: filter.distance.map(|distance| match distance {
                Distance::Close => DistanceFilter::Close,
                Distance::Default => DistanceFilter::Default,
                Distance::Far => DistanceFilter::Far,
                Distance::Worldwide => DistanceFilter::Worldwide,
            }),
            count: filter.count.map(u64::from),
        };
        let matchmaking = self.client()?.matchmaking();
        matchmaking
            .set_lobby_list_filter(list)
            .request_lobby_list(move |result| post(&done, Done::Lobbies(call, result)));
        Ok(call)
    }

    fn join_lobby(&mut self, lobby: Lobby) -> Result<Call, Error> {
        let (call, done) = self.call()?;
        self.client()?
            .matchmaking()
            .join_lobby(LobbyId::from_raw(lobby.0), move |result| {
                post(&done, Done::Joined(call, result))
            });
        Ok(call)
    }

    fn leave_lobby(&mut self, lobby: Lobby) -> Result<(), Error> {
        self.client()?
            .matchmaking()
            .leave_lobby(LobbyId::from_raw(lobby.0));
        Ok(())
    }

    fn lobby_members(&self, lobby: Lobby) -> Result<Vec<User>, Error> {
        Ok(self
            .client()?
            .matchmaking()
            .lobby_members(LobbyId::from_raw(lobby.0))
            .into_iter()
            .map(|member| User(member.raw()))
            .collect())
    }

    fn lobby_owner(&self, lobby: Lobby) -> Result<User, Error> {
        let owner = self
            .client()?
            .matchmaking()
            .lobby_owner(LobbyId::from_raw(lobby.0))
            .raw();
        if owner == 0 {
            Err(Error::Rejected)
        } else {
            Ok(User(owner))
        }
    }

    fn set_lobby_data(&mut self, lobby: Lobby, key: &str, value: &str) -> Result<(), Error> {
        lobby_key(key)?;
        lobby_value(value)?;
        let matchmaking = self.client()?.matchmaking();
        let lobby = LobbyId::from_raw(lobby.0);
        let accepted = if value.is_empty() {
            matchmaking.delete_lobby_data(lobby, key)
        } else {
            matchmaking.set_lobby_data(lobby, key, value)
        };
        if accepted {
            Ok(())
        } else {
            Err(Error::Rejected)
        }
    }

    fn lobby_data(&self, lobby: Lobby, key: &str) -> Result<Option<String>, Error> {
        lobby_key(key)?;
        Ok(self
            .client()?
            .matchmaking()
            .lobby_data(LobbyId::from_raw(lobby.0), key)
            .filter(|value| !value.is_empty()))
    }

    fn invite_dialog(&mut self, lobby: Lobby) -> Result<(), Error> {
        self.client()?
            .friends()
            .activate_invite_dialog(LobbyId::from_raw(lobby.0));
        Ok(())
    }

    fn web_page(&mut self, url: &str) -> Result<bool, Error> {
        if !crate::web_link(url) {
            return Err(Error::Rejected);
        }
        let client = self.client()?;
        if !client.utils().is_overlay_enabled() {
            return Ok(false);
        }
        client.friends().activate_game_overlay_to_web_page(url);
        Ok(true)
    }

    fn launched(&mut self, args: &[String]) -> Option<Lobby> {
        let lobby = crate::connect_lobby(args)?;
        self.client().ok()?;
        self.pending.push(Event::JoinRequested {
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
        message_len(bytes.len())?;
        if peer == self.me()? {
            return Err(Error::Rejected);
        }
        let flags = match delivery {
            Delivery::Reliable => SendFlags::RELIABLE,
            Delivery::Unreliable => SendFlags::UNRELIABLE,
        } | SendFlags::AUTO_RESTART_BROKEN_SESSION;
        let identity = NetworkingIdentity::new_steam_id(SteamId::from_raw(peer.0));
        self.client()?
            .networking_messages()
            .send_message_to_user(identity, flags, bytes, channel)
            .map_err(steam_error)?;
        self.peers.entry(peer.0).or_insert(false);
        Ok(())
    }

    fn receive_messages(&mut self, channel: u32, max: usize) -> Result<Vec<Message>, Error> {
        if max == 0 {
            return Ok(Vec::new());
        }
        Ok(self
            .client()?
            .networking_messages()
            .receive_messages_on_channel(channel, max)
            .into_iter()
            .filter_map(|message| {
                let peer = message.identity_peer().steam_id()?;
                Some(Message {
                    peer: User(peer.raw()),
                    channel,
                    bytes: message.data().to_vec(),
                })
            })
            .collect())
    }

    fn accept_session(&mut self, peer: User) -> Result<(), Error> {
        self.client()?;
        let request = self.requests.remove(&peer.0).ok_or(Error::Missing)?;
        if request.accept() {
            self.peers.entry(peer.0).or_insert(false);
            Ok(())
        } else {
            Err(Error::Rejected)
        }
    }

    #[cfg(any(feature = "input", test))]
    fn input_frame(&mut self) -> Result<Vec<u64>, Error> {
        let input = self.input()?;
        input.run_frame();
        Ok(input
            .get_connected_controllers()
            .into_iter()
            .filter(|handle| *handle != 0)
            .collect())
    }

    #[cfg(any(feature = "input", test))]
    fn input_type(&mut self, pad: u64) -> Result<i32, Error> {
        Ok(input_type(self.input()?.get_input_type_for_handle(pad)))
    }

    #[cfg(any(feature = "input", test))]
    fn activate_action_set(&mut self, pad: u64, set: &str) -> Result<(), Error> {
        let set = self.set_handle(set)?;
        self.input()?.activate_action_set_handle(pad, set);
        Ok(())
    }

    #[cfg(any(feature = "input", test))]
    fn digital_action(&mut self, pad: u64, action: &str) -> Result<crate::Digital, Error> {
        let action = self.digital_handle(action)?;
        let data = self.input()?.get_digital_action_data(pad, action);
        Ok(crate::Digital {
            state: data.bState,
            active: data.bActive,
        })
    }

    #[cfg(any(feature = "input", test))]
    fn analog_action(&mut self, pad: u64, action: &str) -> Result<crate::Analog, Error> {
        let action = self.analog_handle(action)?;
        let data = self.input()?.get_analog_action_data(pad, action);
        Ok(crate::Analog {
            x: data.x,
            y: data.y,
            active: data.bActive,
        })
    }

    #[cfg(any(feature = "input", test))]
    fn digital_origins(&mut self, pad: u64, set: &str, action: &str) -> Result<Vec<i32>, Error> {
        let set = self.set_handle(set)?;
        let action = self.digital_handle(action)?;
        Ok(self
            .input()?
            .get_digital_action_origins(pad, set, action)
            .into_iter()
            .map(|origin| self.remember(origin))
            .collect())
    }

    #[cfg(any(feature = "input", test))]
    fn analog_origins(&mut self, pad: u64, set: &str, action: &str) -> Result<Vec<i32>, Error> {
        let set = self.set_handle(set)?;
        let action = self.analog_handle(action)?;
        Ok(self
            .input()?
            .get_analog_action_origins(pad, set, action)
            .into_iter()
            .map(|origin| self.remember(origin))
            .collect())
    }

    #[cfg(any(feature = "input", test))]
    fn trigger_vibration(&mut self, pad: u64, low: u16, high: u16) -> Result<(), Error> {
        self.input()?;
        let input = unsafe { steamworks::sys::SteamAPI_SteamInput_v006() };
        if input.is_null() {
            return Err(Error::Rejected);
        }
        unsafe { steamworks::sys::SteamAPI_ISteamInput_TriggerVibration(input, pad, low, high) };
        Ok(())
    }

    #[cfg(any(feature = "input", test))]
    fn steam_glyph(
        &mut self,
        origin: i32,
        glyph: crate::SteamGlyph,
    ) -> Result<Option<String>, Error> {
        use steamworks::sys::ESteamInputGlyphSize;
        self.input()?;
        let Some(known) = self.handles.origins.get(&origin).copied() else {
            return Ok(None);
        };
        let input = unsafe { steamworks::sys::SteamAPI_SteamInput_v006() };
        if input.is_null() {
            return Err(Error::Rejected);
        }
        let path = match glyph.format {
            crate::GlyphFormat::Png(size) => {
                let size = match size {
                    crate::GlyphSize::Small => ESteamInputGlyphSize::k_ESteamInputGlyphSize_Small,
                    crate::GlyphSize::Medium => ESteamInputGlyphSize::k_ESteamInputGlyphSize_Medium,
                    crate::GlyphSize::Large => ESteamInputGlyphSize::k_ESteamInputGlyphSize_Large,
                };
                unsafe {
                    steamworks::sys::SteamAPI_ISteamInput_GetGlyphPNGForActionOrigin(
                        input,
                        known,
                        size,
                        glyph.flags(),
                    )
                }
            }
            crate::GlyphFormat::Svg => unsafe {
                steamworks::sys::SteamAPI_ISteamInput_GetGlyphSVGForActionOrigin(
                    input,
                    known,
                    glyph.flags(),
                )
            },
        };
        if path.is_null() {
            return Ok(None);
        }
        let path = unsafe { std::ffi::CStr::from_ptr(path) }
            .to_string_lossy()
            .into_owned();
        Ok((!path.is_empty()).then_some(path))
    }
}

fn init_message(message: String) -> String {
    if message.is_empty() {
        "Steam failed to start".to_string()
    } else {
        message
    }
}

fn file_size(storage: &steamworks::RemoteStorage, name: &str) -> u64 {
    storage
        .files()
        .into_iter()
        .find(|file| file.name == name)
        .map(|file| file.size)
        .unwrap_or(0)
}

fn read_quota() -> Result<Quota, Error> {
    let remote = unsafe { steamworks::sys::SteamAPI_SteamRemoteStorage_v016() };
    if remote.is_null() {
        return Err(Error::Rejected);
    }
    let mut total = 0u64;
    let mut available = 0u64;
    let ok = unsafe {
        steamworks::sys::SteamAPI_ISteamRemoteStorage_GetQuota(remote, &mut total, &mut available)
    };
    if ok {
        Ok(Quota { total, available })
    } else {
        Err(Error::Rejected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_member_state_steam_names_converts() {
        for (state, expected) in [
            (ChatMemberStateChange::Entered, Change::Entered),
            (ChatMemberStateChange::Left, Change::Left),
            (ChatMemberStateChange::Disconnected, Change::Disconnected),
            (ChatMemberStateChange::Kicked, Change::Kicked),
            (ChatMemberStateChange::Banned, Change::Banned),
        ] {
            assert_eq!(change(state), expected);
        }
    }

    #[test]
    fn a_callback_the_crate_cannot_decode_is_skipped_and_freed_not_fatal() {
        let mut runs = 0;
        let mut freed = 0;
        let skipped = drain(
            || {
                runs += 1;
                if runs <= 2 {
                    let state = 0x40_u32;
                    match state {
                        0x1 => {}
                        _ => unreachable!(),
                    }
                }
            },
            || freed += 1,
        );
        assert_eq!((skipped, runs, freed), (2, 3, 2));
    }

    #[test]
    fn a_pass_with_nothing_to_skip_runs_once_and_frees_nothing() {
        let mut runs = 0;
        let mut freed = 0;
        assert_eq!(drain(|| runs += 1, || freed += 1), 0);
        assert_eq!((runs, freed), (1, 0));
    }

    #[test]
    fn a_callback_that_never_decodes_stops_at_the_cap() {
        let mut freed = 0;
        let skipped = drain(|| panic!("undecodable"), || freed += 1);
        assert_eq!(skipped, UNDECODED_MAX);
        assert_eq!(freed, UNDECODED_MAX);
    }
}
