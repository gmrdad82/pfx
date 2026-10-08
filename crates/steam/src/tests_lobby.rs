use super::{
    Call, Change, Compare, Distance, Error, Event, Fake, Hub, LOBBY_CODE_LEN, LOBBY_KEY_MAX,
    LOBBY_MEMBERS_MAX, Lobby, LobbyFilter, LobbyKind, SteamApi, User, find_lobby_by_code,
    lobby_code, normal_code, set_lobby_code,
};

const A: User = User(76_561_197_960_265_731);
const B: User = User(76_561_197_960_265_732);
const C: User = User(76_561_197_960_265_733);

fn created(steam: &mut Fake, kind: LobbyKind, max: u32) -> Lobby {
    let call = steam.create_lobby(kind, max).unwrap();
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

fn joined(steam: &mut Fake, lobby: Lobby) -> Result<Lobby, Error> {
    let call = steam.join_lobby(lobby).unwrap();
    match steam.run_callbacks().as_slice() {
        [Event::Joined { call: got, result }] if *got == call => result.clone(),
        other => panic!("expected a join, got {other:?}"),
    }
}

fn found(steam: &mut Fake, filter: &LobbyFilter) -> Vec<Lobby> {
    let call: Call = steam.find_lobbies(filter).unwrap();
    match steam.run_callbacks().as_slice() {
        [
            Event::Lobbies {
                call: got,
                result: Ok(lobbies),
            },
        ] if *got == call => lobbies.clone(),
        other => panic!("expected lobbies, got {other:?}"),
    }
}

#[test]
fn a_lobby_is_created_found_joined_and_left() {
    let hub = Hub::new();
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    let mut c = hub.client(C);

    let lobby = created(&mut a, LobbyKind::Public, 2);
    assert_eq!(a.lobby_members(lobby).unwrap(), vec![A]);
    assert_eq!(a.lobby_owner(lobby).unwrap(), A);
    a.set_lobby_data(lobby, "mode", "duel").unwrap();
    assert_eq!(a.run_callbacks(), vec![Event::LobbyData { lobby }]);

    let duel = LobbyFilter::default().string("mode", "duel", Compare::Equal);
    assert_eq!(found(&mut b, &duel), vec![lobby]);
    assert_eq!(
        b.lobby_data(lobby, "mode").unwrap().as_deref(),
        Some("duel")
    );
    assert_eq!(b.lobby_data(lobby, "rank").unwrap(), None);

    assert_eq!(joined(&mut b, lobby), Ok(lobby));
    assert_eq!(
        a.run_callbacks(),
        vec![Event::Member {
            lobby,
            user: B,
            change: Change::Entered
        }]
    );
    assert_eq!(a.lobby_members(lobby).unwrap(), vec![A, B]);
    assert_eq!(b.lobby_members(lobby).unwrap(), vec![A, B]);
    assert_eq!(b.lobby_owner(lobby).unwrap(), A);
    assert_eq!(joined(&mut b, lobby), Ok(lobby));
    assert!(a.run_callbacks().is_empty());

    assert_eq!(joined(&mut c, lobby), Err(Error::Rejected));
    assert!(c.lobby_members(lobby).unwrap().is_empty());
    assert_eq!(c.lobby_owner(lobby).unwrap_err(), Error::Rejected);
    assert_eq!(
        found(&mut c, &duel.clone().open_slots(1)),
        Vec::<Lobby>::new()
    );

    b.leave_lobby(lobby).unwrap();
    assert_eq!(
        a.run_callbacks(),
        vec![Event::Member {
            lobby,
            user: B,
            change: Change::Left
        }]
    );
    assert_eq!(a.lobby_members(lobby).unwrap(), vec![A]);
    assert!(b.lobby_members(lobby).unwrap().is_empty());
    b.leave_lobby(lobby).unwrap();

    a.leave_lobby(lobby).unwrap();
    assert!(found(&mut c, &LobbyFilter::default()).is_empty());
    assert_eq!(joined(&mut c, lobby), Err(Error::Rejected));
}

#[test]
fn the_owner_leaving_hands_the_lobby_on_and_shutdown_disconnects() {
    let hub = Hub::new();
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    let mut c = hub.client(C);
    let lobby = created(&mut a, LobbyKind::FriendsOnly, 3);
    joined(&mut b, lobby).unwrap();
    joined(&mut c, lobby).unwrap();
    a.run_callbacks();
    b.run_callbacks();

    assert_eq!(
        b.set_lobby_data(lobby, "mode", "x").unwrap_err(),
        Error::Rejected
    );
    a.leave_lobby(lobby).unwrap();
    assert_eq!(b.lobby_owner(lobby).unwrap(), B);
    assert_eq!(
        c.run_callbacks(),
        vec![Event::Member {
            lobby,
            user: A,
            change: Change::Left
        }]
    );
    b.run_callbacks();
    b.set_lobby_data(lobby, "mode", "x").unwrap();
    assert_eq!(c.run_callbacks(), vec![Event::LobbyData { lobby }]);
    b.set_lobby_data(lobby, "mode", "").unwrap();
    assert_eq!(c.lobby_data(lobby, "mode").unwrap(), None);
    c.run_callbacks();

    b.shutdown();
    assert_eq!(
        c.run_callbacks(),
        vec![Event::Member {
            lobby,
            user: B,
            change: Change::Disconnected
        }]
    );
    assert_eq!(c.lobby_owner(lobby).unwrap(), C);
    assert_eq!(b.lobby_members(lobby).unwrap_err(), Error::Closed);
}

#[test]
fn filters_match_sort_and_cap_the_list() {
    let hub = Hub::new();
    let mut host = hub.client(A);
    let mut seeker = hub.client(B);
    let mut lobbies = Vec::new();
    for (rank, mode) in [
        (1200, "duel"),
        (1500, "duel"),
        (1800, "duel"),
        (1490, "coop"),
    ] {
        let lobby = created(&mut host, LobbyKind::Public, 2);
        host.set_lobby_data(lobby, "rank", &rank.to_string())
            .unwrap();
        host.set_lobby_data(lobby, "mode", mode).unwrap();
        host.run_callbacks();
        lobbies.push(lobby);
    }
    let hidden = created(&mut host, LobbyKind::Private, 2);
    host.set_lobby_data(hidden, "mode", "duel").unwrap();
    host.run_callbacks();
    let friends = created(&mut host, LobbyKind::FriendsOnly, 2);
    host.set_lobby_data(friends, "mode", "duel").unwrap();
    host.run_callbacks();
    let invisible = created(&mut host, LobbyKind::Invisible, 2);
    host.set_lobby_data(invisible, "mode", "solo").unwrap();
    host.run_callbacks();

    let all = found(&mut seeker, &LobbyFilter::default());
    assert_eq!(
        all,
        vec![lobbies[0], lobbies[1], lobbies[2], lobbies[3], invisible]
    );

    let duel = LobbyFilter::default().string("mode", "duel", Compare::Equal);
    assert_eq!(found(&mut seeker, &duel), lobbies[..3].to_vec());
    let not_duel = LobbyFilter::default().string("mode", "duel", Compare::NotEqual);
    assert_eq!(found(&mut seeker, &not_duel), vec![lobbies[3], invisible]);

    let strong = duel.clone().number("rank", 1500, Compare::GreaterOrEqual);
    assert_eq!(found(&mut seeker, &strong), vec![lobbies[1], lobbies[2]]);
    let weak = LobbyFilter::default().number("rank", 1500, Compare::Less);
    assert_eq!(found(&mut seeker, &weak), vec![lobbies[0], lobbies[3]]);

    let near = LobbyFilter::default()
        .near("rank", 1480)
        .distance(Distance::Worldwide);
    assert_eq!(
        found(&mut seeker, &near),
        vec![lobbies[3], lobbies[1], lobbies[0], lobbies[2], invisible]
    );
    assert_eq!(
        found(&mut seeker, &near.clone().count(2)),
        vec![lobbies[3], lobbies[1]]
    );

    joined(&mut seeker, lobbies[3]).unwrap();
    let open = LobbyFilter::default().near("rank", 1480).open_slots(1);
    assert_eq!(
        found(&mut seeker, &open),
        vec![lobbies[1], lobbies[0], lobbies[2], invisible]
    );

    let long = "k".repeat(LOBBY_KEY_MAX + 1);
    assert_eq!(
        seeker
            .find_lobbies(&LobbyFilter::default().number(&long, 1, Compare::Equal))
            .unwrap_err(),
        Error::Rejected
    );
}

#[test]
fn bad_lobby_calls_are_refused() {
    let mut steam = Fake::new();
    assert_eq!(
        steam.create_lobby(LobbyKind::Public, 0).unwrap_err(),
        Error::Rejected
    );
    assert_eq!(
        steam
            .create_lobby(LobbyKind::Public, LOBBY_MEMBERS_MAX + 1)
            .unwrap_err(),
        Error::Rejected
    );
    let lobby = created(&mut steam, LobbyKind::Public, LOBBY_MEMBERS_MAX);
    assert_eq!(
        steam.set_lobby_data(lobby, "", "x").unwrap_err(),
        Error::Rejected
    );
    assert_eq!(
        steam.set_lobby_data(lobby, "k", "nul\0").unwrap_err(),
        Error::Rejected
    );
    assert_eq!(
        steam.set_lobby_data(Lobby(77), "k", "v").unwrap_err(),
        Error::Rejected
    );
    steam.shutdown();
    assert_eq!(
        steam.create_lobby(LobbyKind::Public, 2).unwrap_err(),
        Error::Closed
    );
    assert!(steam.run_callbacks().is_empty());
}

#[test]
fn a_lobby_is_found_by_its_code() {
    let code = lobby_code(7);
    assert_eq!(code.len(), LOBBY_CODE_LEN);
    assert_eq!(code, lobby_code(7));
    assert_ne!(code, lobby_code(8));
    assert!(code.chars().all(|letter| !"01IO".contains(letter)));
    assert_eq!(normal_code(" ab-c d "), "ABCD");

    let hub = Hub::new();
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    let other = created(&mut a, LobbyKind::Public, 2);
    set_lobby_code(&mut a, other, &lobby_code(1)).unwrap();
    a.run_callbacks();
    let lobby = created(&mut a, LobbyKind::Invisible, 2);
    set_lobby_code(&mut a, lobby, &code).unwrap();
    a.run_callbacks();

    let typed = format!(" {}-{} ", code[..3].to_lowercase(), &code[3..]);
    let call = find_lobby_by_code(&mut b, &typed).unwrap();
    assert_eq!(
        b.run_callbacks(),
        vec![Event::Lobbies {
            call,
            result: Ok(vec![lobby])
        }]
    );
    let call = find_lobby_by_code(&mut b, "ZZZZZZ").unwrap();
    assert_eq!(
        b.run_callbacks(),
        vec![Event::Lobbies {
            call,
            result: Ok(Vec::new())
        }]
    );
    assert_eq!(
        find_lobby_by_code(&mut b, " - ").unwrap_err(),
        Error::Rejected
    );
    assert_eq!(
        set_lobby_code(&mut b, lobby, &code).unwrap_err(),
        Error::Rejected
    );
}
