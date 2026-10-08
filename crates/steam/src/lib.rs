mod blob;
mod board;
mod daily;
mod fake;
mod hash;
mod health;
mod hub;
#[cfg(feature = "live")]
mod live;
mod lobby;
mod lockstep;
mod net;
#[cfg(any(feature = "input", test))]
mod steam_input;

pub use blob::{BLOB_CHUNK, BLOB_MAX, BLOB_OPEN_MAX, Blob, Blobs, send_blob};
pub use board::{
    Board, BoardInfo, DETAILS_MAX, Display, Entry, RUN_FIELDS, RUN_MARK, Range, Sort, Ugc, Upload,
    Uploaded, run_details, split_run,
};
pub use daily::{Clock, Daily, Date, Repeat, Today, daily_seed};
pub use fake::{FAKE_USER, Fake};
pub use health::{Ending, Health, Lost, Quit, RECONNECT_MS, connect_lobby};
pub use hub::Hub;
#[cfg(feature = "live")]
pub use live::Steam;
pub use lobby::{
    Change, Compare, Distance, LOBBY_CODE_KEY, LOBBY_CODE_LEN, LOBBY_KEY_MAX, LOBBY_MEMBERS_MAX,
    LOBBY_RESULTS, LOBBY_VALUE_MAX, Lobby, LobbyFilter, LobbyKind, find_lobby_by_code, lobby_code,
    normal_code, set_lobby_code,
};
pub use lockstep::{Desync, LOCKSTEP_WINDOW, Lockstep, Turn, checksum};
pub use net::{Delivery, MESSAGE_MAX, Message};
#[cfg(any(feature = "input", test))]
pub use steam_input::{
    ActionKind, Analog, Digital, GlyphFormat, GlyphSize, GlyphStyle, OriginGlyph, STEAM_PAD_BASE,
    SteamBeside, SteamFeed, SteamGlyph, SteamPads, origin_control, origin_controls,
};

pub const STEAMWORKS_VERSION: &str = "0.13.1";
pub const SDK_VERSION: &str = "1.64";
pub const REDISTRIBUTABLE_LINUX: &str = "libsteam_api.so";
pub const REDISTRIBUTABLE_WINDOWS: &str = "steam_api64.dll";
pub const MAX_SAVE: usize = 1024 * 1024;
pub const NAME_MAX: usize = 127;
pub const PRESENCE_KEY_MAX: usize = 63;
pub const PRESENCE_VALUE_MAX: usize = 255;
pub const PRESENCE_COUNT_MAX: usize = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    Opened,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct User(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Call(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Overlay(Overlay),
    Board {
        call: Call,
        result: Result<BoardInfo, Error>,
    },
    Uploaded {
        call: Call,
        board: Board,
        result: Result<Uploaded, Error>,
    },
    Scores {
        call: Call,
        board: Board,
        result: Result<Vec<Entry>, Error>,
    },
    LobbyCreated {
        call: Call,
        result: Result<Lobby, Error>,
    },
    Lobbies {
        call: Call,
        result: Result<Vec<Lobby>, Error>,
    },
    Joined {
        call: Call,
        result: Result<Lobby, Error>,
    },
    Member {
        lobby: Lobby,
        user: User,
        change: Change,
    },
    LobbyData {
        lobby: Lobby,
    },
    SessionRequest {
        peer: User,
    },
    Connected {
        peer: User,
    },
    SessionFailed {
        peer: User,
        reason: Option<i32>,
    },
    JoinRequested {
        lobby: Lobby,
        friend: Option<User>,
    },
    ConnectionLost {
        peer: User,
        lost: Lost,
    },
    Reconnected {
        peer: User,
    },
    Forfeit {
        peer: User,
        lost: Lost,
    },
    PeerQuit {
        peer: User,
        quit: Quit,
    },
    Shared {
        call: Call,
        result: Result<Ugc, Error>,
    },
    UgcDownloaded {
        call: Call,
        ugc: Ugc,
        result: Result<Vec<u8>, Error>,
    },
    Unrecognised,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quota {
    pub total: u64,
    pub available: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudOff {
    Account,
    App,
    AccountAndApp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    NotRunning,
    Init(String),
    AppId,
    Closed,
    CloudOff(CloudOff),
    TooBig,
    Quota,
    Missing,
    Rejected,
    Unknown,
    Offline,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotRunning => write!(f, "Steam is not running"),
            Error::Init(message) => write!(f, "Steam failed to start: {message}"),
            Error::AppId => write!(f, "the app id comes from the game's config"),
            Error::Closed => write!(f, "Steam is shut down"),
            Error::CloudOff(CloudOff::Account) => {
                write!(f, "Steam Cloud is off for this account")
            }
            Error::CloudOff(CloudOff::App) => write!(f, "Steam Cloud is off for this app"),
            Error::CloudOff(CloudOff::AccountAndApp) => {
                write!(f, "Steam Cloud is off for this account and this app")
            }
            Error::TooBig => write!(f, "a save is larger than 1 MiB"),
            Error::Quota => write!(f, "Steam Cloud does not have room for that save"),
            Error::Missing => write!(f, "file is not in Steam Cloud"),
            Error::Rejected => write!(f, "Steam rejected the call"),
            Error::Unknown => write!(f, "Steam does not know that name"),
            Error::Offline => write!(f, "Steam's servers did not answer"),
        }
    }
}

impl std::error::Error for Error {}

pub trait SteamApi {
    fn run_callbacks(&mut self) -> Vec<Event>;
    fn shutdown(&mut self);

    fn unlock(&mut self, name: &str) -> Result<(), Error>;
    fn achieved(&self, name: &str) -> Result<bool, Error>;
    fn store(&mut self) -> Result<(), Error>;

    fn sync(&mut self, achieved: &[&str]) -> Result<(), Error> {
        for name in achieved {
            api_name(name)?;
        }
        let mut grant = false;
        for name in achieved {
            if !self.achieved(name)? {
                self.unlock(name)?;
                grant = true;
            }
        }
        if grant {
            self.store()?;
        }
        Ok(())
    }

    #[cfg(any(test, debug_assertions))]
    fn clear(&mut self, name: &str) -> Result<(), Error>;

    fn stat_i32(&self, name: &str) -> Result<i32, Error>;
    fn set_stat_i32(&mut self, name: &str, value: i32) -> Result<(), Error>;
    fn stat_f32(&self, name: &str) -> Result<f32, Error>;
    fn set_stat_f32(&mut self, name: &str, value: f32) -> Result<(), Error>;

    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<(), Error>;
    fn read(&self, name: &str) -> Result<Vec<u8>, Error>;
    fn exists(&self, name: &str) -> Result<bool, Error>;
    fn delete(&mut self, name: &str) -> Result<(), Error>;
    fn quota(&self) -> Result<Quota, Error>;

    fn set_presence(&mut self, key: &str, value: &str) -> Result<(), Error>;
    fn clear_presence(&mut self, key: &str) -> Result<(), Error>;

    fn me(&self) -> Result<User, Error>;
    fn server_time(&self) -> Result<Option<u64>, Error>;

    fn find_board(&mut self, name: &str) -> Result<Call, Error>;
    fn find_or_create_board(
        &mut self,
        name: &str,
        sort: Sort,
        display: Display,
    ) -> Result<Call, Error>;
    fn upload_score(
        &mut self,
        board: Board,
        score: i32,
        details: &[i32],
        upload: Upload,
    ) -> Result<Call, Error>;
    fn download_scores(
        &mut self,
        board: Board,
        range: Range,
        details: usize,
    ) -> Result<Call, Error>;

    fn upload_run(
        &mut self,
        board: Board,
        score: i32,
        details: &[i32],
        upload: Upload,
        run: Ugc,
    ) -> Result<Call, Error> {
        if details.len() + RUN_FIELDS > DETAILS_MAX {
            return Err(Error::Rejected);
        }
        self.upload_score(board, score, &run_details(run, details), upload)
    }

    fn share_file(&mut self, name: &str) -> Result<Call, Error>;
    fn attach_ugc(&mut self, board: Board, ugc: Ugc) -> Result<(), Error>;
    fn download_ugc(&mut self, ugc: Ugc) -> Result<Call, Error>;

    fn create_lobby(&mut self, kind: LobbyKind, max_members: u32) -> Result<Call, Error>;
    fn find_lobbies(&mut self, filter: &LobbyFilter) -> Result<Call, Error>;
    fn join_lobby(&mut self, lobby: Lobby) -> Result<Call, Error>;
    fn leave_lobby(&mut self, lobby: Lobby) -> Result<(), Error>;
    fn lobby_members(&self, lobby: Lobby) -> Result<Vec<User>, Error>;
    fn lobby_owner(&self, lobby: Lobby) -> Result<User, Error>;
    fn set_lobby_data(&mut self, lobby: Lobby, key: &str, value: &str) -> Result<(), Error>;
    fn lobby_data(&self, lobby: Lobby, key: &str) -> Result<Option<String>, Error>;
    fn invite_dialog(&mut self, lobby: Lobby) -> Result<(), Error>;
    fn launched(&mut self, args: &[String]) -> Option<Lobby>;

    fn send_message(
        &mut self,
        peer: User,
        channel: u32,
        bytes: &[u8],
        delivery: Delivery,
    ) -> Result<(), Error>;
    fn receive_messages(&mut self, channel: u32, max: usize) -> Result<Vec<Message>, Error>;
    fn accept_session(&mut self, peer: User) -> Result<(), Error>;

    #[cfg(any(feature = "input", test))]
    fn input_frame(&mut self) -> Result<Vec<u64>, Error>;
    #[cfg(any(feature = "input", test))]
    fn input_type(&mut self, pad: u64) -> Result<i32, Error>;
    #[cfg(any(feature = "input", test))]
    fn activate_action_set(&mut self, pad: u64, set: &str) -> Result<(), Error>;
    #[cfg(any(feature = "input", test))]
    fn digital_action(&mut self, pad: u64, action: &str) -> Result<Digital, Error>;
    #[cfg(any(feature = "input", test))]
    fn analog_action(&mut self, pad: u64, action: &str) -> Result<Analog, Error>;
    #[cfg(any(feature = "input", test))]
    fn digital_origins(&mut self, pad: u64, set: &str, action: &str) -> Result<Vec<i32>, Error>;
    #[cfg(any(feature = "input", test))]
    fn analog_origins(&mut self, pad: u64, set: &str, action: &str) -> Result<Vec<i32>, Error>;
    #[cfg(any(feature = "input", test))]
    fn trigger_vibration(&mut self, pad: u64, low: u16, high: u16) -> Result<(), Error>;
    #[cfg(any(feature = "input", test))]
    fn steam_glyph(&mut self, origin: i32, glyph: SteamGlyph) -> Result<Option<String>, Error>;
}

pub(crate) fn api_name(name: &str) -> Result<(), Error> {
    if name.is_empty() || name.len() > NAME_MAX || name.as_bytes().contains(&0) {
        Err(Error::Rejected)
    } else {
        Ok(())
    }
}

pub(crate) fn file_name(name: &str) -> Result<(), Error> {
    if name.is_empty() || name.as_bytes().contains(&0) {
        Err(Error::Rejected)
    } else {
        Ok(())
    }
}

pub(crate) fn save_len(len: usize) -> Result<(), Error> {
    if len > MAX_SAVE {
        Err(Error::TooBig)
    } else {
        Ok(())
    }
}

pub(crate) fn fits(available: u64, old: u64, new_len: usize) -> Result<(), Error> {
    let new = u64::try_from(new_len).unwrap_or(u64::MAX);
    if new > available.saturating_add(old) {
        Err(Error::Quota)
    } else {
        Ok(())
    }
}

pub(crate) fn cloud_error(account: bool, app: bool) -> Result<(), Error> {
    match (account, app) {
        (true, true) => Ok(()),
        (false, false) => Err(Error::CloudOff(CloudOff::AccountAndApp)),
        (false, true) => Err(Error::CloudOff(CloudOff::Account)),
        (true, false) => Err(Error::CloudOff(CloudOff::App)),
    }
}

pub(crate) fn presence_key(key: &str) -> Result<(), Error> {
    if key.is_empty() || key.len() > PRESENCE_KEY_MAX || key.as_bytes().contains(&0) {
        Err(Error::Rejected)
    } else {
        Ok(())
    }
}

pub(crate) fn presence_value(value: &str) -> Result<(), Error> {
    if value.is_empty() || value.len() > PRESENCE_VALUE_MAX || value.as_bytes().contains(&0) {
        Err(Error::Rejected)
    } else {
        Ok(())
    }
}

pub(crate) fn details_len(details: &[i32]) -> Result<(), Error> {
    if details.len() > DETAILS_MAX {
        Err(Error::Rejected)
    } else {
        Ok(())
    }
}

pub(crate) fn lobby_key(key: &str) -> Result<(), Error> {
    if key.is_empty() || key.len() > LOBBY_KEY_MAX || key.as_bytes().contains(&0) {
        Err(Error::Rejected)
    } else {
        Ok(())
    }
}

pub(crate) fn lobby_value(value: &str) -> Result<(), Error> {
    if value.len() > LOBBY_VALUE_MAX || value.as_bytes().contains(&0) {
        Err(Error::Rejected)
    } else {
        Ok(())
    }
}

pub(crate) fn lobby_size(max_members: u32) -> Result<(), Error> {
    if max_members == 0 || max_members > LOBBY_MEMBERS_MAX {
        Err(Error::Rejected)
    } else {
        Ok(())
    }
}

pub(crate) fn message_len(len: usize) -> Result<(), Error> {
    if len > MESSAGE_MAX {
        Err(Error::Rejected)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_blob;
#[cfg(test)]
mod tests_board;
#[cfg(test)]
mod tests_daily;
#[cfg(test)]
mod tests_health;
#[cfg(test)]
mod tests_invite;
#[cfg(test)]
mod tests_lobby;
#[cfg(test)]
mod tests_lockstep;
#[cfg(test)]
mod tests_net;
#[cfg(test)]
mod tests_runs;
#[cfg(test)]
mod tests_steam_input;
