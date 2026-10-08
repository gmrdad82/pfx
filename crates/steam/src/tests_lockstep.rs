use super::{
    Delivery, Desync, Error, Event, Fake, Hub, LOCKSTEP_WINDOW, Lockstep, SteamApi, Turn, User,
    checksum,
};

const A: User = User(76_561_197_960_265_731);
const B: User = User(76_561_197_960_265_732);
const CHANNEL: u32 = 7;

struct Side {
    steam: Fake,
    lock: Lockstep,
    state: u64,
}

fn pair(every: u64) -> (Side, Side) {
    let hub = Hub::new();
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    a.send_message(B, 0, b"hi", Delivery::Reliable).unwrap();
    b.accept_session(A).unwrap();
    a.run_callbacks();
    b.run_callbacks();
    b.receive_messages(0, 8).unwrap();
    (
        Side {
            steam: a,
            lock: Lockstep::new(A, B, CHANNEL, every).unwrap(),
            state: 1,
        },
        Side {
            steam: b,
            lock: Lockstep::new(B, A, CHANNEL, every).unwrap(),
            state: 1,
        },
    )
}

fn input(side: &mut Side, bytes: &[u8]) -> u64 {
    side.lock.send_input(&mut side.steam, bytes).unwrap()
}

fn pump(side: &mut Side) -> Vec<Turn> {
    side.lock.receive(&mut side.steam).unwrap();
    let mut turns = Vec::new();
    while let Some(turn) = side.lock.turn() {
        for input in &turn.inputs {
            for byte in input {
                side.state = side
                    .state
                    .wrapping_mul(0x0000_0100_0000_01b3)
                    .wrapping_add(u64::from(*byte) + 1);
            }
        }
        if turn.check {
            let sum = checksum(&side.state.to_le_bytes());
            side.lock
                .send_checksum(&mut side.steam, turn.step, sum)
                .unwrap();
        }
        turns.push(turn);
    }
    turns
}

#[test]
fn the_sim_advances_only_when_both_inputs_are_in() {
    let (mut a, mut b) = pair(4);
    assert_eq!(a.lock.side(), 0);
    assert_eq!(b.lock.side(), 1);

    assert_eq!(input(&mut a, b"a0"), 0);
    assert!(pump(&mut a).is_empty());
    assert!(pump(&mut b).is_empty());
    assert_eq!(a.lock.step(), 0);
    assert_eq!(b.lock.step(), 0);
    assert_eq!(a.lock.ahead(), 1);

    assert_eq!(input(&mut b, b"b0"), 0);
    let on_b = pump(&mut b);
    assert_eq!(
        on_b,
        vec![Turn {
            step: 0,
            inputs: [b"a0".to_vec(), b"b0".to_vec()],
            check: false
        }]
    );
    assert!(pump(&mut b).is_empty());
    let on_a = pump(&mut a);
    assert_eq!(on_a, on_b);
    assert_eq!((a.lock.step(), b.lock.step()), (1, 1));

    input(&mut a, b"a1");
    input(&mut a, b"a2");
    input(&mut b, b"b1");
    let a_turns = pump(&mut a);
    assert_eq!(a_turns.len(), 1);
    assert_eq!(a_turns[0].inputs, [b"a1".to_vec(), b"b1".to_vec()]);
    let b_turns = pump(&mut b);
    assert_eq!(b_turns, a_turns);
    input(&mut b, b"");
    assert_eq!(pump(&mut b)[0].inputs, [b"a2".to_vec(), Vec::new()]);
    assert_eq!(pump(&mut a)[0].inputs, [b"a2".to_vec(), Vec::new()]);
    assert_eq!(a.state, b.state);
}

#[test]
fn matching_checksums_keep_both_sides_running() {
    let (mut a, mut b) = pair(4);
    for step in 0u8..40 {
        input(&mut a, &[step, 1]);
        input(&mut b, &[step, 2, 3]);
        pump(&mut a);
        pump(&mut b);
    }
    pump(&mut a);
    pump(&mut b);
    assert_eq!((a.lock.step(), b.lock.step()), (40, 40));
    assert_eq!(a.state, b.state);
    assert_eq!(a.lock.desync(), None);
    assert_eq!(b.lock.desync(), None);
}

#[test]
fn a_forced_checksum_mismatch_is_a_desync_on_both_sides() {
    let (mut a, mut b) = pair(4);
    for step in 0u8..4 {
        input(&mut a, &[step]);
        input(&mut b, &[step]);
    }
    a.lock.receive(&mut a.steam).unwrap();
    b.lock.receive(&mut b.steam).unwrap();
    let mut a_turns = Vec::new();
    while let Some(turn) = a.lock.turn() {
        a_turns.push(turn);
    }
    while b.lock.turn().is_some() {}
    assert_eq!(a_turns.len(), 4);
    assert!(a_turns[3].check);
    assert!(a_turns[..3].iter().all(|turn| !turn.check));

    a.lock.send_checksum(&mut a.steam, 3, 0xaaaa).unwrap();
    assert_eq!(a.lock.desync(), None);
    b.lock.send_checksum(&mut b.steam, 3, 0xbbbb).unwrap();
    assert_eq!(b.lock.desync(), None);
    a.lock.receive(&mut a.steam).unwrap();
    b.lock.receive(&mut b.steam).unwrap();
    assert_eq!(
        a.lock.desync(),
        Some(Desync {
            step: 3,
            mine: 0xaaaa,
            theirs: 0xbbbb
        })
    );
    assert_eq!(
        b.lock.desync(),
        Some(Desync {
            step: 3,
            mine: 0xbbbb,
            theirs: 0xaaaa
        })
    );

    input(&mut a, b"x");
    input(&mut b, b"y");
    assert!(pump(&mut a).is_empty());
    assert!(pump(&mut b).is_empty());
    assert_eq!(a.lock.step(), 4);
}

#[test]
fn a_diverged_sim_is_caught_at_the_next_check() {
    let (mut a, mut b) = pair(3);
    for step in 0u8..2 {
        input(&mut a, &[step]);
        input(&mut b, &[step]);
        pump(&mut a);
        pump(&mut b);
    }
    b.state ^= 1;
    input(&mut a, &[9]);
    input(&mut b, &[9]);
    pump(&mut a);
    pump(&mut b);
    pump(&mut a);
    let found = a.lock.desync().unwrap();
    assert_eq!(found.step, 2);
    assert_ne!(found.mine, found.theirs);
    assert_eq!(b.lock.desync().map(|desync| desync.step), Some(2));
}

#[test]
fn the_window_bounds_how_far_a_side_runs_ahead() {
    let (mut a, _b) = pair(4);
    for _ in 0..LOCKSTEP_WINDOW {
        input(&mut a, b"i");
    }
    assert_eq!(a.lock.ahead(), LOCKSTEP_WINDOW);
    assert_eq!(
        a.lock
            .send_input(&mut a.steam, b"one too many")
            .unwrap_err(),
        Error::Rejected
    );
}

#[test]
fn packets_that_break_the_protocol_are_ignored() {
    let mut lock = Lockstep::new(A, B, CHANNEL, 4).unwrap();
    let packet = |tag: u8, step: u64, payload: &[u8]| {
        let mut bytes = vec![tag];
        bytes.extend_from_slice(&step.to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    };
    assert!(!lock.accept(b""));
    assert!(!lock.accept(&[1, 0, 0]));
    assert!(!lock.accept(&packet(9, 0, b"x")));
    assert!(lock.accept(&packet(1, 0, b"first")));
    assert!(!lock.accept(&packet(1, 0, b"second")));
    assert!(!lock.accept(&packet(1, 2 * LOCKSTEP_WINDOW, b"far")));
    assert!(lock.accept(&packet(1, 2 * LOCKSTEP_WINDOW - 1, b"edge")));
    assert!(!lock.accept(&packet(2, 2, &7u64.to_le_bytes())));
    assert!(!lock.accept(&packet(2, 3, b"short")));
    assert!(lock.accept(&packet(2, 3, &7u64.to_le_bytes())));
    assert!(!lock.accept(&packet(2, 3, &8u64.to_le_bytes())));
    assert_eq!(lock.turn(), None);

    assert_eq!(Lockstep::new(A, A, 0, 4).unwrap_err(), Error::Rejected);
    assert_eq!(Lockstep::new(A, B, 0, 0).unwrap_err(), Error::Rejected);
    assert_eq!((lock.me(), lock.peer()), (A, B));
    assert!(lock.is_check(3) && lock.is_check(7) && !lock.is_check(4));
}

#[test]
fn checksums_are_only_for_check_steps_already_run() {
    let (mut a, mut b) = pair(2);
    assert_eq!(
        a.lock.send_checksum(&mut a.steam, 1, 5).unwrap_err(),
        Error::Rejected
    );
    input(&mut a, b"0");
    input(&mut b, b"0");
    input(&mut a, b"1");
    input(&mut b, b"1");
    a.lock.receive(&mut a.steam).unwrap();
    assert!(a.lock.turn().is_some());
    assert!(a.lock.turn().is_some());
    assert_eq!(
        a.lock.send_checksum(&mut a.steam, 0, 5).unwrap_err(),
        Error::Rejected
    );
    a.lock.send_checksum(&mut a.steam, 1, 5).unwrap();
    let stray = b.steam.run_callbacks();
    assert!(
        stray
            .iter()
            .all(|event| !matches!(event, Event::SessionFailed { .. }))
    );
}
