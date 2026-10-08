use super::{
    Change, Delivery, Ending, Error, Event, Fake, Health, Hub, Lobby, LobbyKind, Lost, Quit,
    RECONNECT_MS, SteamApi, User,
};

const A: User = User(76_561_197_960_265_731);
const B: User = User(76_561_197_960_265_732);
const C: User = User(76_561_197_960_265_733);
const BEATS: u32 = 9;

struct Clock(u64);

impl Clock {
    fn advance(&mut self, ms: u64) -> u64 {
        self.0 += ms;
        self.0
    }
}

fn connected(hub: &Hub) -> (Fake, Fake) {
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    a.send_message(B, 0, b"hi", Delivery::Reliable).unwrap();
    b.accept_session(A).unwrap();
    a.run_callbacks();
    b.run_callbacks();
    b.receive_messages(0, 8).unwrap();
    (a, b)
}

fn lobby_of_two(a: &mut Fake, b: &mut Fake) -> Lobby {
    let call = a.create_lobby(LobbyKind::Private, 2).unwrap();
    let lobby = match a.run_callbacks().as_slice() {
        [
            Event::LobbyCreated {
                call: got,
                result: Ok(lobby),
            },
        ] if *got == call => *lobby,
        other => panic!("expected a lobby, got {other:?}"),
    };
    b.join_lobby(lobby).unwrap();
    b.run_callbacks();
    a.run_callbacks();
    lobby
}

fn both(a: &mut Fake, b: &mut Fake, on_a: &mut Health, on_b: &mut Health, now: u64) -> Vec<Event> {
    let events = on_a.tick(a, now).unwrap();
    on_b.tick(b, now).unwrap();
    events
}

#[test]
fn beats_keep_the_link_up_at_the_rate_the_game_sets() {
    let hub = Hub::new();
    let (mut a, mut b) = connected(&hub);
    let mut clock = Clock(0);
    let mut on_a = Health::new(B, BEATS, 250, 1_000, clock.0).unwrap();
    let mut on_b = Health::new(A, BEATS, 250, 1_000, clock.0).unwrap();
    let mut seen = Vec::new();
    for _ in 0..100 {
        let now = clock.advance(50);
        seen.extend(both(&mut a, &mut b, &mut on_a, &mut on_b, now));
    }
    assert!(seen.is_empty());
    assert_eq!((on_a.lost(), on_a.ended()), (None, None));

    let hub = Hub::new();
    let (mut a, mut b) = connected(&hub);
    let mut on_a = Health::new(B, BEATS, 250, 1_000, 0).unwrap();
    for now in (0..1_000).step_by(10) {
        on_a.tick(&mut a, now).unwrap();
    }
    let beats = b.receive_messages(BEATS, 100).unwrap();
    assert_eq!(beats.len(), 4);
    assert!(beats.iter().all(|beat| beat.peer == A && beat.bytes == [1]));
}

#[test]
fn silence_loses_the_connection_and_the_peer_returning_inside_the_window_reconnects() {
    let hub = Hub::new();
    let (mut a, mut b) = connected(&hub);
    let mut clock = Clock(0);
    let mut on_a = Health::new(B, BEATS, 100, 500, 0).unwrap();
    let mut on_b = Health::new(A, BEATS, 100, 500, 0).unwrap();
    both(&mut a, &mut b, &mut on_a, &mut on_b, 0);

    let mut seen = Vec::new();
    for _ in 0..20 {
        seen.extend(on_a.tick(&mut a, clock.advance(100)).unwrap());
    }
    assert_eq!(
        seen,
        vec![Event::ConnectionLost {
            peer: B,
            lost: Lost::Silent
        }]
    );
    assert_eq!(on_a.lost(), Some(Lost::Silent));

    let back = clock.advance(60_000);
    on_b.tick(&mut b, back).unwrap();
    assert_eq!(
        on_a.tick(&mut a, clock.advance(10)).unwrap(),
        vec![Event::Reconnected { peer: B }]
    );
    assert_eq!(on_a.lost(), None);
    assert_eq!(on_a.ended(), None);
}

#[test]
fn the_default_window_is_two_minutes_and_only_its_end_is_a_forfeit() {
    assert_eq!(RECONNECT_MS, 120_000);
    let hub = Hub::new();
    let (mut a, _b) = connected(&hub);
    let mut clock = Clock(0);
    let mut on_a = Health::new(B, BEATS, 1_000, 5_000, 0).unwrap();
    let mut seen = Vec::new();
    let mut lost_at = None;
    let mut forfeit_at = None;
    for _ in 0..200 {
        let now = clock.advance(1_000);
        for event in on_a.tick(&mut a, now).unwrap() {
            match event {
                Event::ConnectionLost { .. } => lost_at = Some(now),
                Event::Forfeit { .. } => forfeit_at = Some(now),
                _ => {}
            }
            seen.push(event);
        }
    }
    assert_eq!(
        seen,
        vec![
            Event::ConnectionLost {
                peer: B,
                lost: Lost::Silent
            },
            Event::Forfeit {
                peer: B,
                lost: Lost::Silent
            },
        ]
    );
    assert_eq!(lost_at, Some(5_000));
    assert_eq!(forfeit_at, Some(5_000 + RECONNECT_MS));
    assert_eq!(on_a.ended(), Some(Ending::Forfeit(Lost::Silent)));
    assert!(on_a.tick(&mut a, clock.advance(1_000)).unwrap().is_empty());
}

#[test]
fn a_session_failure_waits_out_the_window_the_game_set() {
    let hub = Hub::new();
    let (mut a, mut b) = connected(&hub);
    let mut clock = Clock(0);
    let mut on_a = Health::new(B, BEATS, 100, 10_000, 0)
        .unwrap()
        .reconnect_window(30_000);
    let mut on_b = Health::new(A, BEATS, 100, 10_000, 0).unwrap();
    both(&mut a, &mut b, &mut on_a, &mut on_b, 0);
    assert!(on_a.tick(&mut a, 50).unwrap().is_empty());

    hub.fail_session(A, B, Some(4001));
    let failed = a.run_callbacks();
    let now = clock.advance(100);
    let lost: Vec<Event> = failed
        .iter()
        .filter_map(|event| on_a.observe(event, now))
        .collect();
    assert_eq!(
        lost,
        vec![Event::ConnectionLost {
            peer: B,
            lost: Lost::SessionFailed(Some(4001))
        }]
    );
    assert_eq!(on_a.observe(&failed[0], now), None);
    assert_eq!(
        on_a.tick(&mut a, clock.advance(29_000)).unwrap(),
        Vec::<Event>::new()
    );
    assert_eq!(
        on_a.tick(&mut a, clock.advance(1_000)).unwrap(),
        vec![Event::Forfeit {
            peer: B,
            lost: Lost::SessionFailed(Some(4001))
        }]
    );
}

#[test]
fn a_session_failure_heals_when_both_sides_beat_again() {
    let hub = Hub::new();
    let (mut a, mut b) = connected(&hub);
    let mut clock = Clock(0);
    let mut on_a = Health::new(B, BEATS, 100, 10_000, 0).unwrap();
    let mut on_b = Health::new(A, BEATS, 100, 10_000, 0).unwrap();
    both(&mut a, &mut b, &mut on_a, &mut on_b, 0);
    hub.fail_session(A, B, None);
    let now = clock.advance(100);
    for event in a.run_callbacks() {
        on_a.observe(&event, now);
    }
    for event in b.run_callbacks() {
        on_b.observe(&event, now);
    }
    assert_eq!(on_a.lost(), Some(Lost::SessionFailed(None)));

    let mut seen = Vec::new();
    for _ in 0..5 {
        let now = clock.advance(100);
        seen.extend(on_a.tick(&mut a, now).unwrap());
        on_b.tick(&mut b, now).unwrap();
        a.run_callbacks();
        b.run_callbacks();
    }
    assert_eq!(seen, vec![Event::Reconnected { peer: B }]);
    assert_eq!(on_a.lost(), None);
}

#[test]
fn a_peer_that_drops_from_the_lobby_may_come_back() {
    let hub = Hub::new();
    let (mut a, mut b) = connected(&hub);
    let lobby = lobby_of_two(&mut a, &mut b);
    let mut clock = Clock(0);
    let mut on_a = Health::new(B, BEATS, 100, 500, 0).unwrap().in_lobby(lobby);

    b.shutdown();
    let now = clock.advance(100);
    let events: Vec<Event> = a
        .run_callbacks()
        .iter()
        .filter_map(|event| on_a.observe(event, now))
        .collect();
    assert_eq!(
        events,
        vec![Event::ConnectionLost {
            peer: B,
            lost: Lost::Disconnected
        }]
    );

    let mut b = hub.client(B);
    b.join_lobby(lobby).unwrap();
    let now = clock.advance(90_000);
    let back: Vec<Event> = a
        .run_callbacks()
        .iter()
        .filter_map(|event| on_a.observe(event, now))
        .collect();
    assert_eq!(back, vec![Event::Reconnected { peer: B }]);
    assert!(on_a.tick(&mut a, clock.advance(100)).unwrap().is_empty());
}

#[test]
fn a_goodbye_or_leaving_the_lobby_is_a_quit_not_a_forfeit() {
    let hub = Hub::new();
    let (mut a, mut b) = connected(&hub);
    let mut on_a = Health::new(B, BEATS, 100, 500, 0).unwrap();
    let mut on_b = Health::new(A, BEATS, 100, 500, 0).unwrap();
    on_a.tick(&mut a, 0).unwrap();
    on_b.tick(&mut b, 0).unwrap();
    on_a.leave(&mut a).unwrap();
    assert_eq!(on_a.ended(), Some(Ending::Quit(Quit::Goodbye)));
    on_a.leave(&mut a).unwrap();
    assert!(on_a.tick(&mut a, 100).unwrap().is_empty());
    assert_eq!(
        on_b.tick(&mut b, 100).unwrap(),
        vec![Event::PeerQuit {
            peer: A,
            quit: Quit::Goodbye
        }]
    );
    assert!(on_b.tick(&mut b, 1_000_000).unwrap().is_empty());

    for (change, quit) in [
        (Change::Left, Quit::Left),
        (Change::Kicked, Quit::Kicked),
        (Change::Banned, Quit::Banned),
    ] {
        let hub = Hub::new();
        let (mut a, mut b) = connected(&hub);
        let lobby = lobby_of_two(&mut a, &mut b);
        let mut on_a = Health::new(B, BEATS, 100, 500, 0).unwrap().in_lobby(lobby);
        let event = Event::Member {
            lobby,
            user: B,
            change,
        };
        assert_eq!(
            on_a.observe(&event, 10),
            Some(Event::PeerQuit { peer: B, quit })
        );
        assert_eq!(on_a.ended(), Some(Ending::Quit(quit)));
        assert!(on_a.tick(&mut a, 1_000_000).unwrap().is_empty());
    }

    let hub = Hub::new();
    let (mut a, mut b) = connected(&hub);
    let lobby = lobby_of_two(&mut a, &mut b);
    let mut on_a = Health::new(B, BEATS, 100, 500, 0).unwrap().in_lobby(lobby);
    b.leave_lobby(lobby).unwrap();
    let events: Vec<Event> = a
        .run_callbacks()
        .iter()
        .filter_map(|event| on_a.observe(event, 10))
        .collect();
    assert_eq!(
        events,
        vec![Event::PeerQuit {
            peer: B,
            quit: Quit::Left
        }]
    );
}

#[test]
fn other_lobbies_and_strangers_are_ignored_and_bad_rates_refused() {
    let hub = Hub::new();
    let (mut a, mut b) = connected(&hub);
    let lobby = lobby_of_two(&mut a, &mut b);
    let mut on_a = Health::new(B, BEATS, 100, 500, 0).unwrap().in_lobby(lobby);
    let elsewhere = Event::Member {
        lobby: Lobby(lobby.0 + 1),
        user: B,
        change: Change::Left,
    };
    let stranger = Event::Member {
        lobby,
        user: C,
        change: Change::Disconnected,
    };
    assert_eq!(on_a.observe(&elsewhere, 10), None);
    assert_eq!(on_a.observe(&stranger, 10), None);
    assert_eq!(on_a.heard(400), None);
    assert!(on_a.tick(&mut a, 800).unwrap().is_empty());
    assert_eq!(
        on_a.tick(&mut a, 900).unwrap(),
        vec![Event::ConnectionLost {
            peer: B,
            lost: Lost::Silent
        }]
    );

    assert_eq!(Health::new(B, 0, 0, 500, 0).unwrap_err(), Error::Rejected);
    assert_eq!(Health::new(B, 0, 500, 500, 0).unwrap_err(), Error::Rejected);
    let mut health = Health::new(B, 0, 100, 500, 0).unwrap();
    assert_eq!(health.peer(), B);
    let mut steam = Fake::new();
    steam.shutdown();
    assert_eq!(health.tick(&mut steam, 0).unwrap_err(), Error::Closed);
}
