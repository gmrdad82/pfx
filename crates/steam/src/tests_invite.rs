use super::{Error, Event, Fake, Hub, Lobby, LobbyKind, SteamApi, User, connect_lobby};

const A: User = User(76_561_197_960_265_731);
const B: User = User(76_561_197_960_265_732);

fn args(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_string).collect()
}

fn lobby(steam: &mut Fake) -> Lobby {
    let call = steam.create_lobby(LobbyKind::FriendsOnly, 2).unwrap();
    match steam.run_callbacks().as_slice() {
        [
            Event::LobbyCreated {
                call: got,
                result: Ok(lobby),
            },
        ] if *got == call => *lobby,
        other => panic!("expected a lobby, got {other:?}"),
    }
}

#[test]
fn a_friend_is_invited_through_the_overlay_and_joins() {
    let hub = Hub::new();
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    let room = lobby(&mut a);
    a.invite_dialog(room).unwrap();
    assert_eq!(a.invite_dialogs(), &[room]);
    assert_eq!(b.invite_dialog(room).unwrap_err(), Error::Rejected);

    hub.invite(A, B, room);
    let events = b.run_callbacks();
    assert_eq!(
        events,
        vec![Event::JoinRequested {
            lobby: room,
            friend: Some(A)
        }]
    );
    let Event::JoinRequested { lobby: asked, .. } = events[0] else {
        unreachable!()
    };
    let call = b.join_lobby(asked).unwrap();
    assert_eq!(
        b.run_callbacks(),
        vec![Event::Joined {
            call,
            result: Ok(room)
        }]
    );
    assert_eq!(a.lobby_members(room).unwrap(), vec![A, B]);
}

#[test]
fn a_launch_with_connect_lobby_is_a_join_request() {
    assert_eq!(
        connect_lobby(&args("game +connect_lobby 109775241234567890")),
        Some(Lobby(109_775_241_234_567_890))
    );
    assert_eq!(
        connect_lobby(&args("game -windowed +connect_lobby 42 +other 1")),
        Some(Lobby(42))
    );
    assert_eq!(connect_lobby(&args("game +connect_lobby")), None);
    assert_eq!(connect_lobby(&args("game +connect_lobby nope")), None);
    assert_eq!(connect_lobby(&args("game +connect_lobby 0")), None);
    assert_eq!(connect_lobby(&args("game")), None);

    let mut steam = Fake::new();
    assert_eq!(steam.launched(&args("game")), None);
    assert!(steam.run_callbacks().is_empty());
    assert_eq!(
        steam.launched(&args("game +connect_lobby 42")),
        Some(Lobby(42))
    );
    assert_eq!(
        steam.run_callbacks(),
        vec![Event::JoinRequested {
            lobby: Lobby(42),
            friend: None
        }]
    );
    steam.shutdown();
    assert_eq!(steam.launched(&args("game +connect_lobby 42")), None);
    assert_eq!(steam.invite_dialog(Lobby(1)).unwrap_err(), Error::Closed);
}
