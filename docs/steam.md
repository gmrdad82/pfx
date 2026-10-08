# Steam

A product that ships on Steam depends on `pfx-steam` and links the Steamworks SDK. A product that does not leaves the crate out, and never links the SDK. Nothing else in the workspace depends on it.

The crate is built on `steamworks` (steamworks-rs) pinned at `=0.13.1`. That release carries Steamworks SDK 1.64. `STEAMWORKS_VERSION` is `"0.13.1"` and `SDK_VERSION` is `"1.64"`.

The default feature `live` is the real backend, `Steam`. `Fake` is always compiled and is what tests use. A game codes against the `SteamApi` trait, so the same calls run on either.

## Starting and stopping

The app id comes from the game's own config. The engine has no app id of its own. `0` is rejected with `Error::AppId` before Steam is called.

```rust
let mut steam = pfx_steam::Steam::init(app_id)?;
```

`Error::NotRunning` means the Steam client is not running. Any other init failure is `Error::Init` and carries Steam's message.

Once per frame:

```rust
use pfx_steam::{Event, Overlay};

for event in steam.run_callbacks() {
    match event {
        Event::Overlay(Overlay::Opened) => pause(),
        Event::Overlay(Overlay::Closed) => resume(),
        other => game.on_steam(other),
    }
}
```

`run_callbacks` returns every `Event` since the last call: the overlay, the answers to async calls (leaderboards and lobbies), lobby membership and data changes, and peer-to-peer session events. Each async call returns a `Call` id at once, or an `Error` when its arguments are refused before Steam is asked; its answer arrives later as an event carrying the same `Call` and a `Result`. `Error::Offline` means Steam's servers did not answer (an I/O failure, a timeout or no connection).

steamworks 0.13.1 panics (`unreachable!()`) when it decodes a lobby chat update whose member state it does not know. `Steam` catches that panic around the callback pass, frees the callback Steam still holds (`SteamAPI_ManualDispatch_FreeLastCallback`), carries on with the rest of the queue and returns one `Event::Unrecognised` for each callback it skipped, so a game ignores it and never sees a panic. The one panic message still reaches stderr through the panic hook. `Fake` never sends it.

`shutdown` drops the client, which calls `SteamAPI_Shutdown`. Later calls return `Error::Closed`, and `run_callbacks` returns an empty list. Steamworks allows one client per process.

## Achievements

`unlock` grants one, `achieved` reads it back, and `store` sends the pending changes. `sync` takes the full set the game derives from its save:

```rust
steam.sync(&achieved)?;
```

It checks every name first, unlocks any Steam does not already have, and stores only when it granted at least one. A second sync of the same set does not store again. It never clears. An achievement Steam has and the save lacks stays granted, so a missed one can be granted again and a local save cannot take one away.

`clear` exists only in a test or debug build.

Names are 1 to 127 bytes (`NAME_MAX`) and must not contain a nul. Anything else is `Error::Rejected`, and a bad name in a `sync` grants nothing.

## Stats

Integer and float stats are separate lifetime counters. `stat_i32` / `set_stat_i32` and `stat_f32` / `set_stat_f32` read and write them. `store` commits stats and achievements together.

A non-finite float is `Error::Rejected`. On `Fake`, an unset stat reads as `0` or `0.0` until the first set fixes its type; the other type then returns `Error::Rejected`. On `Steam`, a missing stat and a stat of the wrong type both come back as `Error::Unknown`, because the SDK reports both as a failed call.

## Cloud

`write`, `read`, `exists`, `delete` and `quota` are for one save file. The file must be at most 1 MiB (`MAX_SAVE`, 1_048_576 bytes). A larger buffer is `Error::TooBig` and is not stored. `Error::Quota` means the account does not have room; an overwrite gets the old file's bytes back before the new size is checked. `Error::Missing` means the file is not in Cloud. `exists` returns `Ok(false)` for that case.

When Cloud is off, every one of these calls returns `Error::CloudOff`, which says whether it is the account, the app, or both.

```rust
steam.write("save", &bytes)?;
let bytes = steam.read("save")?;
let quota = steam.quota()?;
```

`quota.total` and `quota.available` are bytes. Steam's file names are case-insensitive. `Fake` compares the name as the game wrote it.

## Rich presence

`set_presence` sets one key. `clear_presence` removes one key. An empty key or value is `Error::Rejected`; clearing is the way to unset a key. The SDK limits, including the terminating nul, are a key of 63 bytes (`PRESENCE_KEY_MAX`), a value of 255 bytes (`PRESENCE_VALUE_MAX`), and 30 keys (`PRESENCE_COUNT_MAX`). A new key past 30 is `Error::Rejected`. Replacing a key that is already set is allowed.

## Leaderboards

A board is found by name, or found and created with its sort and display type:

```rust
use pfx_steam::{Display, Range, Sort, Upload};

let call = steam.find_or_create_board("high-scores", Sort::Descending, Display::Numeric)?;
// later, from run_callbacks:
// Event::Board { call, result: Ok(info) } → info.board, info.name, info.sort, info.display, info.entries
```

`find_board` only finds; a missing board answers `Err(Error::Unknown)`. `find_or_create_board` returns an existing board as it is, with the sort and display it was made with. Names follow the achievement rule: 1 to 127 bytes, no nul.

`upload_score(board, score, details, upload)` sends one `i32` score and up to 64 `i32` details (`DETAILS_MAX`). `Upload::KeepBest` keeps the player's better score, by the board's sort; `Upload::Force` replaces it. The answer is `Event::Uploaded` with `Uploaded { score, changed, rank, previous_rank }`; a rank of 0 means none.

`download_scores(board, range, details)` asks for a range and keeps up to `details` details per entry:

- `Range::Global { first, last }`: ranks `first..=last`, starting at 1;
- `Range::AroundUser { before, after }`: the player's row with `before` rows above and `after` below; empty when the player has no score;
- `Range::Friends`: the player and their Steam friends.

The answer is `Event::Scores` with entries in rank order, each with `user`, `rank`, `score`, `details` and `run` (below). A board handle comes from a `Board` event of this session; `Steam` answers `Error::Unknown` for any other.

## Checked leaderboards

A board can be checked with no server: each entry carries its run's input log, viewers re-run it, and the game hides entries that do not reproduce.

1. When the run ends, the game writes the log to Cloud (`write`) and shares it: `share_file(name)` answers `Event::Shared { call, result: Ok(ugc) }` with a UGC handle (`ISteamRemoteStorage::FileShare`).
2. `upload_run(board, score, details, Upload::KeepBest, ugc)` uploads the score with the handle in the entry's own details: three of the 64 detail slots (`RUN_FIELDS`) hold a marker (`RUN_MARK`) and the handle's two halves, ahead of the game's details, so a run takes at most 61 details of its own.
3. When the answer says `changed`, the game calls `attach_ugc(board, ugc)` (`AttachLeaderboardUGC`), so Steam's own tools show the log too. On a score that did not beat the old one it must not attach, or the entry would point at a log that does not match it.
4. A viewer downloads a range as usual. Each `Entry` has `run: Option<Ugc>` read back from the details, and `details` holds only the game's own. `download_ugc(ugc)` answers `Event::UgcDownloaded { call, ugc, result: Ok(bytes) }`; the game re-runs the log on the day's seed and hides the entry when its score differs.

The handle rides in the details because steamworks 0.13.1 drops an entry's attached UGC handle when it downloads entries; the details round-trip whole on every backend. On `Steam`, the download is `ISteamRemoteStorage::UGCDownload`, polled each `run_callbacks` through `GetUGCDetails` and read with `UGCRead`; a log over 16 MiB is `Error::TooBig`, and one that has not arrived after 36,000 calls is `Error::Offline`. `Fake` keeps shared files on the `Hub`, and `hub.attached(board, user)` shows what was attached.

## The daily seeded run

Every player gets the same run on the same UTC day, with no server of ours. `Daily` holds the game's salt, its board prefix, its repeat rule and the board's sort and display:

```rust
use pfx_steam::{Daily, Display, Repeat, Sort};

let daily = Daily::new("example-game", "daily", Repeat::FirstOnly, Sort::Descending, Display::Numeric);
let today = daily.today(&steam, system_unix_seconds);
let rng = pfx_core::sim::Rng::new(daily.seed(today.date));
let call = daily.open(&mut steam, today.date)?;
```

- **The date.** `today` takes Steam's server time (`ISteamUtils::GetServerRealTime`) when Steam has one, and the system clock the game passes in otherwise. `Today::clock` says which (`Clock::Server` or `Clock::System`). The crate never reads a clock; the game reads the system time and passes it. A run keeps the date it started on, so a run that crosses midnight still belongs to its first day.
- **The seed.** `daily_seed(salt, date)` is FNV-1a 64 (offset `0xcbf29ce484222325`, prime `0x100000001b3`) over the salt's UTF-8 bytes, one `0x00` byte, and the date as `YYYY-MM-DD`, then the splitmix64 finalizer (add `0x9e3779b97f4a7c15`; `z ^= z >> 30; z *= 0xbf58476d1ce4e5b9; z ^= z >> 27; z *= 0x94d049bb133111eb; z ^= z >> 31`), the same mix as `pfx_core::sim::mix64`. For the salt `example-game` and 2026-10-05 the seed is `0xe6904dba18fb3efa`. A game picks its own salt, so two games never share a run.
- **The board.** One board per day, named `{prefix}-YYYY-MM-DD` (`daily-2026-10-05`). `open` finds or creates it with the game's sort and display; the first player of the day creates it.
- **Repeat runs.** `Repeat::Best` uploads every finished run with `Upload::KeepBest`. `Repeat::FirstOnly` uploads only the first run of the day: `submit(steam, board, date, last_counted, score, details)` skips the upload when `last_counted` is today. The game keeps `last_counted`, the date of its last counted run, in its save, which Cloud carries between machines; a lost save allows another attempt, and `KeepBest` still keeps the first score unless the new one beats it.

**The attempt marker.** Uploads stay keep-best, so a game marks the attempt at its start: `daily.start(steam, board)` uploads `daily.marker()` (0 on a descending board, `i32::MAX` on an ascending one), which any real score beats. The run's real score later replaces it through `KeepBest`. `daily.check(steam, board)` downloads the player's own row, and `Daily::used(&entries, me)` says whether today's attempt is already spent, so a lost save or a second machine cannot hand out a second first attempt.

`Date::from_unix` turns UTC seconds into a date; `Date::new` checks a calendar date.

## Lobbies

Lobbies are Steam's matchmaking rooms. They find the other player for a 1v1; the game's traffic then goes over messages.

```rust
use pfx_steam::{Compare, LobbyFilter, LobbyKind};

let call = steam.create_lobby(LobbyKind::Public, 2)?;
// Event::LobbyCreated { call, result: Ok(lobby) }
steam.set_lobby_data(lobby, "mode", "duel")?;

let filter = LobbyFilter::default()
    .string("mode", "duel", Compare::Equal)
    .number("rank", 1500, Compare::GreaterOrEqual)
    .near("rank", 1480)
    .open_slots(1)
    .count(10);
let call = steam.find_lobbies(&filter)?;
// Event::Lobbies { call, result: Ok(lobbies) }
let call = steam.join_lobby(lobbies[0])?;
// Event::Joined { call, result: Ok(lobby) }
```

- `create_lobby` takes the kind (`Private`, `FriendsOnly`, `Public`, `Invisible`) and 1 to 250 members (`LOBBY_MEMBERS_MAX`). The creator is in the lobby and owns it.
- `find_lobbies` lists `Public` and `Invisible` lobbies. A filter compares the lobby's value with the given one: strings, numbers (the lobby's value parsed as `i32`), `near` (sorts by distance to the value, filters nothing), `open_slots`, `distance` and `count` (50 when unset).
- `join_lobby` answers `Event::Joined`; a full or missing lobby is `Err(Error::Rejected)`, because Steam does not say which. The other members get `Event::Member { lobby, user, change: Change::Entered }`.
- `leave_lobby` leaves; the others get `Change::Left`. When the owner leaves, Steam picks a new owner. A member whose client shuts down shows as `Change::Disconnected`.
- `lobby_members` and `lobby_owner` answer for a lobby the player is in: the members list is empty and the owner is `Error::Rejected` otherwise.
- `set_lobby_data` is for the owner only, anyone else is `Error::Rejected`. Keys are 1 to 255 bytes (`LOBBY_KEY_MAX`) and values at most 8192 (`LOBBY_VALUE_MAX`), neither with a nul; an empty value deletes the key. Members get `Event::LobbyData { lobby }`. `lobby_data` reads a key, `None` when it is unset.

**Codes.** A 1v1 between friends is often set up by a short code. `lobby_code(seed)` makes a 6-letter code from a seed the game supplies, from letters that cannot be misread (no 0, 1, I or O). `set_lobby_code(steam, lobby, code)` stores it in the lobby's data under `code` (`LOBBY_CODE_KEY`); the owner calls it. `find_lobby_by_code(steam, typed)` lists the lobby with that code worldwide, with spaces and dashes dropped and letters upper-cased, so `abc-def` finds `ABCDEF`. A lobby that should be found only by its code is made `Invisible`; the code search still finds it.

## Invites

A friend joins a 1v1 through Steam itself:

- `invite_dialog(lobby)` opens the overlay's invite dialog for a lobby the player is in (`ISteamFriends::ActivateGameOverlayInviteDialog`); outside the lobby it is `Error::Rejected`.
- When a friend accepts an invite, or picks "Join game" on the player's profile, while the game runs, Steam sends `GameLobbyJoinRequested`, and `run_callbacks` returns `Event::JoinRequested { lobby, friend: Some(friend) }`. The game calls `join_lobby(lobby)`.
- When the invite starts the game, Steam launches it with `+connect_lobby <id>` on the command line. The game passes its arguments once after `init`: `steam.launched(&std::env::args().collect::<Vec<_>>())` returns the lobby and queues `Event::JoinRequested { lobby, friend: None }` for the next `run_callbacks`, so both paths reach the same handler. `connect_lobby(args)` is the parser alone.

On `Fake`, `hub.invite(from, to, lobby)` plays the friend accepting, and `invite_dialogs()` lists the dialogs a client opened.

## Messages

Peer-to-peer messages go through Steam's relayed `ISteamNetworkingMessages`, addressed by the peer's Steam id (`User`). Steam relays them, so neither side opens a port and nobody runs a server.

```rust
use pfx_steam::Delivery;

steam.send_message(peer, 1, &bytes, Delivery::Reliable)?;
for message in steam.receive_messages(1, 64)? {
    handle(message.peer, &message.bytes);
}
```

- Channels are the game's numbers; each channel has its own queue. A message is at most 512 KiB (`MESSAGE_MAX`); a larger one, or one sent to yourself, is `Error::Rejected`.
- `Delivery::Reliable` arrives once and in order. `Delivery::Unreliable` may be dropped.
- The first message to a peer asks for a session: the peer gets `Event::SessionRequest { peer }` and calls `accept_session(peer)` to take it; until then the messages wait. A peer that answers with a message of its own opens the session as well. Both sides then get `Event::Connected { peer }`. `accept_session` without a pending request is `Error::Missing`.
- `Event::SessionFailed { peer, reason }` reports a session that broke, with Steam's `ESteamNetConnectionEnd` code when there is one. The next send starts a new session; `Steam` sends with Steam's auto-restart flag, so a broken session restarts by itself.

## Blobs

`send_blob(steam, peer, channel, id, bytes)` sends one large payload, such as a week's input log of a few hundred KB, as 32 KiB chunks (`BLOB_CHUNK`) on the reliable channel; it returns the chunk count. A blob is at most 16 MiB (`BLOB_MAX`); a larger one is `Error::TooBig`. Each chunk carries the blob's id, its index, the chunk count, the length and a checksum of the whole blob (the same FNV-1a 64 and splitmix64 mix as `checksum`).

`Blobs::new(channel)` puts them back together: `receive(steam)` returns `Blob::Complete { peer, id, bytes }` when every chunk is in and the checksum holds, and `Blob::Corrupt { peer, id }` when it does not or when a chunk contradicts the others. Chunks may arrive in any order, a repeated chunk is ignored, blobs from two peers never mix, and a peer holds at most four blobs open (`BLOB_OPEN_MAX`). `accept(peer, bytes)` takes one chunk from another transport.

## Lockstep

`Lockstep` is optional. A game whose 1v1 is two separate runs on one seed (a daily run compared once a day or week) never needs it; it needs lobbies, codes, invites, messages, `Health` and blobs. `Lockstep` runs a 1v1 in lockstep over one reliable channel. Each side sends its input for every fixed step; the sim advances a step only when both inputs for it are in. The input is the game's own bytes, such as one step of its input recording. Every `every` steps both sides exchange a checksum of their sim state, and any difference is a desync. The sim is bit-exact on both machines by `pfx_core::sim`'s promise, so a desync is a bug or a cheat, never float noise.

```rust
use pfx_steam::{Lockstep, checksum};

let mut lock = Lockstep::new(me, peer, 5, 60)?;
// each frame:
lock.send_input(&mut steam, &input_bytes)?;
lock.receive(&mut steam)?;
while let Some(turn) = lock.turn() {
    sim.step(&turn.inputs[0], &turn.inputs[1]);
    if turn.check {
        lock.send_checksum(&mut steam, turn.step, checksum(&sim.state_bytes()))?;
    }
}
if let Some(desync) = lock.desync() {
    // desync.step, desync.mine, desync.theirs
}
```

- `turn.inputs` is in the same order on both machines: the lower Steam id's input first. `side()` says which slot is this player's.
- `send_input` sends the input for the next step this player has not sent yet and returns that step. A player can run up to `LOCKSTEP_WINDOW` (256) steps ahead of the sim; one more is `Error::Rejected`. Sending a few steps ahead is the game's input delay.
- `turn.check` is set on steps `every - 1`, `2 * every - 1`, and so on. `send_checksum` takes a check step already run. A desync stops `turn()`, and the game decides what follows.
- `receive` reads the whole channel and ignores anything not from the peer, so the channel belongs to the lockstep. `accept` takes one packet from another transport. Packets out of the window, repeated, malformed or for a non-check step are ignored.
- `checksum(bytes)` is the same FNV-1a 64 and splitmix64 mix as the daily seed.

## Session health

`Health` watches one peer for a 1v1 on a channel of its own. Time comes from the game in milliseconds, so the crate never reads a clock and tests drive it with their own.

```rust
use pfx_steam::{Event, Health};

let mut health = Health::new(peer, 6, 1_000, 10_000, now_ms)?
    .in_lobby(lobby)
    .reconnect_window(120_000);
// each frame:
for event in health.tick(&mut steam, now_ms)? {
    handle(event);
}
for event in steam.run_callbacks() {
    if let Some(health_event) = health.observe(&event, now_ms) {
        handle(health_event);
    }
}
// when the player quits:
health.leave(&mut steam)?;
```

- **Heartbeat.** `tick` sends a one-byte beat on the reliable channel every `every_ms`, while the link is up and while it is lost, and reads the channel. Any message from the peer there counts as heard; `heard(now_ms)` counts other traffic.
- **A lost connection.** `Event::ConnectionLost { peer, lost }` opens the reconnect window. `lost` is `Lost::Silent` when nothing was heard for `timeout_ms` (which must be longer than `every_ms`), `Lost::SessionFailed(reason)` from the peer's `SessionFailed`, or `Lost::Disconnected` when the peer drops from the watched lobby.
- **The window.** `reconnect_window(ms)` sets it; it is two minutes by default (`RECONNECT_MS`). When the peer is heard again, or enters the watched lobby again, inside the window, `Event::Reconnected { peer }` closes it and the match goes on. When the window runs out, `Event::Forfeit { peer, lost }` ends the match. Only that is a forfeit.
- **A quit.** `Event::PeerQuit { peer, quit }` ends the match at once: `Quit::Goodbye` when the peer called `leave` (a goodbye on the same channel), and `Quit::Left`, `Quit::Kicked` or `Quit::Banned` when the peer leaves or is removed from the watched lobby. A quit never opens a window and is never a forfeit.
- `lost()` says whether the window is open and why; `ended()` says how the match ended (`Ending::Quit` or `Ending::Forfeit`). After an ending, `tick` and `observe` return nothing.

The same flow runs on the fake `Hub`: beats are fake messages, and `fail_session`, a client's `shutdown` and a new client for the same player joining again reach the peer as they would on Steam.

## Steam Input

With the `input` feature, the crate depends on `pfx-input` and drives controllers through Steam Input instead of reading them raw. The dependency runs from steam to input; input never depends on steam.

```rust
use pfx_steam::SteamPads;

let mut pads = SteamPads::new("ship", &["fire", "jump"], &["move", "aim"]);
// each frame:
let step = input.update(&mut pads.feed(&mut steam));
```

`SteamPads` holds the action set and the digital and analog action names from the game's Steam Input manifest; `feed` lends it the Steam client for one call and is input's `Backend`.

- **Pads.** Steam Input starts on the first poll (`ISteamInput::Init`, with the crate calling `RunFrame` each poll). A new controller is `PadConnected` with `PadInfo { family: Family::from_steam_input_type(type), motors: Motors::Two, resolves_actions: true }`; a controller that goes away is `PadDisconnected`. Each poll activates the action set on every pad.
- **Actions.** Steam resolves the bindings, so the pad sends actions rather than buttons. A digital action is `PadAction { value: [1.0, 0.0] }` while pressed and `[0.0, 0.0]` released; an analog action is `[x, y]` as Steam reports it. An action is sent only when its value changes; an inactive action, or a non-finite value, reads as zero.
- **Origins.** `PadOrigins` carries each action's bound origins, mapped onto input's controls and sent again whenever the player rebinds in Steam's configurator. Face buttons map by position (`South`, `East`, `West`, `North`), so a Switch B and a PlayStation cross are both `South`. Bumpers, triggers, stick clicks, the d-pad, menu and view, the PlayStation touchpad click, back grips and paddles map onto their `Button`; a stick is `Control::Stick` and the whole d-pad `Control::DPad`. Gyros, trackpad surfaces and stick touches have no control and are left out. `origin_control` is that table; the live tests check every number in it against the SDK's names.
- **Steam's glyphs.** Beside the input crate's own glyphs, a game can show Steam's art for a bound origin: `pads.steam_glyphs(&mut steam, pad, action, SteamGlyph::png(GlyphSize::Medium))` returns an `OriginGlyph { origin, control, path }` for each origin Steam has bound to the action, with the path of Steam's image file (`ISteamInput::GetGlyphPNGForActionOrigin`, or `GetGlyphSVGForActionOrigin` with `SteamGlyph::svg()`). `GlyphStyle` (knockout, light, dark) and the `neutral_abxy` and `solid_abxy` flags pick Steam's variants. The game reads the file at that path for its bytes. Valve licenses those glyphs for use through this API; the input crate's glyphs stay the default, and the game chooses. A glyph is offered only for origins Steam reported for this session's pads; any other origin answers `None`.
- **Rumble.** `set_motors` calls `ISteamInput::TriggerVibration(low, high)` on the pad's handle. Steam keeps the motors at that level until the next command, and input's rumble player sends the stop.

**Beside gilrs.** Under Steam, always on the Deck and under Proton, gilrs sees Steam's virtual pad (`28DE:11FF`) for every controller Steam Input owns, and on a Linux desktop it may also read the raw controller, so a DualSense would arrive twice and show the wrong glyphs. The game builds its gilrs backend with input's Steam precedence, and runs it beside the Steam pads as one backend:

```rust
use pfx_input::SteamPrecedence;
use pfx_input::pads::Gilrs;

let mut gilrs = Gilrs::new()?.with_steam(SteamPrecedence::from_env());
let mut pads = SteamPads::new("ship", &["fire", "jump"], &["move"]);
// each frame:
let step = input.update(&mut pads.beside(&mut steam, &mut gilrs));
```

- `beside` calls the other backend's `steam_input(true)` (input's `Backend` hook) once Steam Input is up, and `steam_input(false)` when it shuts down. Without Steam Input it never calls it, so gilrs keeps every pad.
- While told, `Gilrs` hides Steam's virtual pad and the physical devices Steam lists in `SDL_GAMECONTROLLER_IGNORE_DEVICES` (read by `SteamPrecedence::from_env()`), so there is no double input and no wrong family; any other pad passes through. `docs/input.md` has the full rule, including `xinput(true)` for Windows and Proton.
- Steam pads take ids from `STEAM_PAD_BASE` (`0x8000_0000`) up, clear of gilrs's, and their family comes only from Steam Input's input type. `is_up()` says whether Steam Input answered the last poll.
- Prompts for a Steam pad come from its `PadOrigins`, so after a rebind in Steam's configurator the prompt shows the button actually bound.
- `set_motors` sends a Steam pad's rumble to `TriggerVibration` and every other pad's to gilrs.

`Fake` stands in for Steam Input in tests: `plug_pad(handle, type)`, `unplug_pad`, `press`, `tilt`, `bind` (origins as SDK numbers), `action_set` and `vibrations`. Its glyph paths are made up (`steam/glyphs/<origin>-<flags>-<size>.png`) and point at no file.

## Trading cards

Trading cards need no code. They are set up on Steamworks' partner site (the app's Community Items: cards, badges, emoticons and profile backgrounds), and Steam drops cards from playtime on its own. Nothing in this crate touches them.

## The redistributable

`steamworks-sys` links `libsteam_api.so` on Linux and `steam_api64.dll` on Windows. The copy it links is the one shipped inside that crate, under `lib/steam/redistributable_bin/linux64` and `lib/steam/redistributable_bin/win64`, which is SDK 1.64. It is not a library from the system. The build copies that library into the directory it passes to the linker. `REDISTRIBUTABLE_LINUX` and `REDISTRIBUTABLE_WINDOWS` are those file names.

The crate does not set an rpath. At run time the two loaders differ:

- On Windows the loader looks in the directory of the executable first, so `steam_api64.dll` ships beside the exe. `steam_api64.lib` is only for the link.
- On Linux the loader does not look beside the executable. It uses `DT_RUNPATH` / `DT_RPATH`, then `LD_LIBRARY_PATH`, then the cache. Ship `libsteam_api.so` beside the binary and link the game with `-Wl,-rpath,$ORIGIN`.

`cargo test` and `cargo run` set `LD_LIBRARY_PATH` from the native link-search path, so the engine's tests find the staged library. A shipped binary does not get that, and needs the rpath above.

## Tests

Every behaviour above, including `sync`, is tested on `Fake`. Those tests do not need a Steam client or a network.

`Fake::new()` is one player. For two or more, a `Hub` is the fake Steam they share: `hub.client(User(…))` makes a `Fake` for that player, and they see each other's boards, lobbies and messages. The hub also sets what Steam would: `set_server_time`, `befriend`, `set_loss(seed, per_mille)` (unreliable messages dropped from a seeded draw, so a test repeats exactly), and `fail_session(a, b, reason)`. An answer to a call arrives on the next `run_callbacks`. On `Fake`, `AroundUser` stops at the ends of the board; ties rank the earlier score first. One test, `spacewar_client`, is ignored. It inits the public Spacewar app id 480 when a client is running, and it is never part of the gate. It does not change achievements or stats.

## Later

Rich-presence joins with a connect string (`GameRichPresenceJoinRequested`) are not wrapped. The depot build and its upload are a separate step from this runtime. macOS (`libsteam_api.dylib`) is not covered here.

## Depot build

`pfx steam stage` writes SteamPipe's ContentBuilder scripts and stages each platform's depot. It does not upload, it does not run steamcmd, and it does not read credentials. The upload belongs to the release process, which keeps the builder account's credentials outside the repository.

```
pfx steam stage --manifest <game>/steam.toml --out <caller tmp dir>
```

The manifest lives in the game's repository. The app id is the game's, read from that file. pfx has no app id of its own. `--out` is a directory inside the caller's `tmp/` and must be empty. A failed stage clears what it wrote there.

The command prints one line, the steamcmd invocation the release runs, and writes nothing else to stdout:

```
steamcmd +login "$STEAM_BUILDER" +run_app_build "<out>/app_build_<appid>.vdf" +quit
```

`$STEAM_BUILDER` is the account name. The release step substitutes it and supplies the password. The password is not in the line, the manifest, or the staged tree.

### steam.toml

Paths in `path` are relative to the directory that holds the manifest, or absolute. Every one of them must stay inside the caller's git tree. A missing file, a path that leaves that tree, a symlink, a depot with no files, a duplicate depot id, and `setlive = "default"` without `setlive_default = true` are refused.

An executable built with pfx-game's `tools` feature is refused too: every staged file that starts as an ELF, PE or Mach-O binary is scanned for pfx-game's tools marker (`docs/game.md`, "The tools feature and a Steam build"), and one that carries it stops the stage before anything is written, with an error that names the file and says to build the Steam binary with `cargo build --release --no-default-features`. A release that builds without `tools` passes.

`setlive` is the branch SteamPipe sets live after a successful upload. Omitted, it is empty, and Steam does not move a branch. `default` is the public branch. It is written only when the manifest also sets `setlive_default = true`. `preview` defaults to false. `true` writes Steam's preview build (`"preview" "1"`), which does not upload.

Depots are `windows`, `linux`, and `macos`. Each has an id and a list of file mappings. `path` is a file or a directory the caller's build produced. `depot` is the path inside the install. For a file it is that file's install path, and a trailing slash means the source file's name inside that folder. For a directory it is the install folder, and the directory's children are staged under it. A depot path uses forward slashes and cannot leave the depot.

```toml
app_id = 1000
description = "Example Game 1.0.0"
setlive = ""
preview = false

[depots.windows]
id = 1001

[[depots.windows.files]]
path = "build/windows/example-game.exe"
depot = "example-game.exe"

[[depots.windows.files]]
path = "build/windows/assets"
depot = "assets"

[depots.linux]
id = 1002

[[depots.linux.files]]
path = "build/linux/example-game"
depot = "example-game"
```

### What gets written

```
<out>/app_build_<appid>.vdf
<out>/depot_build_<depotid>.vdf
<out>/content/<depotid>/...
<out>/output/
<out>/steam-manifest.json
```

The scripts are Valve's ContentBuilder shape: an `appbuild` script whose `depots` block names one `depot_build_<depotid>.vdf` per depot, and a `DepotBuildConfig` per depot with `ContentRoot` `content/<depotid>` and one `FileMapping` per manifest entry. `buildoutput` is `output`, next to the scripts, for steamcmd's logs. `contentroot` is `content`. `local` is empty.

The staged files are copies of the mapped sources. `<out>/steam-manifest.json` lists every staged file with its path inside the depot, its size in bytes, and its sha256, so the release step can check the tree it is about to upload. The same steamcmd line is recorded there as `steamcmd`.

The runtime crate, with everything above, is separate from this command. The upload itself stays with the release process.
