use super::{Delivery, Error, Event, Fake, Hub, MESSAGE_MAX, Message, SteamApi, User};

const A: User = User(76_561_197_960_265_731);
const B: User = User(76_561_197_960_265_732);
const C: User = User(76_561_197_960_265_733);

fn connect(a: &mut Fake, b: &mut Fake) {
    let (me_a, me_b) = (a.me().unwrap(), b.me().unwrap());
    a.send_message(me_b, 0, b"hello", Delivery::Reliable)
        .unwrap();
    assert_eq!(
        b.run_callbacks(),
        vec![Event::SessionRequest { peer: me_a }]
    );
    b.accept_session(me_a).unwrap();
    assert_eq!(a.run_callbacks(), vec![Event::Connected { peer: me_b }]);
    assert_eq!(b.run_callbacks(), vec![Event::Connected { peer: me_a }]);
    assert_eq!(
        b.receive_messages(0, 8).unwrap(),
        vec![Message {
            peer: me_a,
            channel: 0,
            bytes: b"hello".to_vec()
        }]
    );
}

fn numbers(messages: &[Message]) -> Vec<u32> {
    messages
        .iter()
        .map(|message| u32::from_le_bytes(message.bytes[..4].try_into().unwrap()))
        .collect()
}

#[test]
fn reliable_messages_wait_for_the_session_and_arrive_in_order() {
    let hub = Hub::new();
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    for index in 0u32..100 {
        a.send_message(B, 3, &index.to_le_bytes(), Delivery::Reliable)
            .unwrap();
    }
    assert_eq!(b.run_callbacks(), vec![Event::SessionRequest { peer: A }]);
    assert!(b.receive_messages(3, 1000).unwrap().is_empty());
    assert_eq!(b.accept_session(C).unwrap_err(), Error::Missing);
    b.accept_session(A).unwrap();
    assert_eq!(b.accept_session(A).unwrap_err(), Error::Missing);
    assert_eq!(a.run_callbacks(), vec![Event::Connected { peer: B }]);
    assert_eq!(b.run_callbacks(), vec![Event::Connected { peer: A }]);

    let first = b.receive_messages(3, 40).unwrap();
    let rest = b.receive_messages(3, 1000).unwrap();
    assert_eq!(first.len(), 40);
    assert!(
        first
            .iter()
            .all(|message| message.peer == A && message.channel == 3)
    );
    let mut all = numbers(&first);
    all.extend(numbers(&rest));
    assert_eq!(all, (0..100).collect::<Vec<u32>>());
    assert!(b.receive_messages(3, 1000).unwrap().is_empty());
    assert!(b.receive_messages(3, 0).unwrap().is_empty());

    b.send_message(A, 3, b"back", Delivery::Reliable).unwrap();
    assert!(a.run_callbacks().is_empty());
    assert_eq!(a.receive_messages(3, 8).unwrap()[0].bytes, b"back");
}

#[test]
fn channels_are_kept_apart() {
    let hub = Hub::new();
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    connect(&mut a, &mut b);
    a.send_message(B, 1, b"one", Delivery::Reliable).unwrap();
    a.send_message(B, 2, b"two", Delivery::Unreliable).unwrap();
    assert_eq!(b.receive_messages(2, 8).unwrap()[0].bytes, b"two");
    assert_eq!(b.receive_messages(1, 8).unwrap()[0].bytes, b"one");
    assert!(b.receive_messages(1, 8).unwrap().is_empty());
}

#[test]
fn unreliable_messages_may_drop_but_reliable_ones_never_do() {
    let run = |seed: u64| {
        let hub = Hub::new();
        let mut a = hub.client(A);
        let mut b = hub.client(B);
        connect(&mut a, &mut b);
        hub.set_loss(seed, 300);
        for index in 0u32..1000 {
            a.send_message(B, 1, &index.to_le_bytes(), Delivery::Unreliable)
                .unwrap();
            a.send_message(B, 2, &index.to_le_bytes(), Delivery::Reliable)
                .unwrap();
        }
        let unreliable = numbers(&b.receive_messages(1, 2000).unwrap());
        let reliable = numbers(&b.receive_messages(2, 2000).unwrap());
        assert_eq!(reliable, (0..1000).collect::<Vec<u32>>());
        assert!(unreliable.windows(2).all(|pair| pair[0] < pair[1]));
        unreliable
    };
    let once = run(7);
    assert!(once.len() > 600 && once.len() < 800, "{}", once.len());
    assert_eq!(run(7), once);
    assert_ne!(run(8), once);

    let hub = Hub::new();
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    connect(&mut a, &mut b);
    for index in 0u32..50 {
        a.send_message(B, 1, &index.to_le_bytes(), Delivery::Unreliable)
            .unwrap();
    }
    assert_eq!(b.receive_messages(1, 100).unwrap().len(), 50);
    hub.set_loss(1, 1000);
    a.send_message(B, 1, b"gone", Delivery::Unreliable).unwrap();
    assert!(b.receive_messages(1, 100).unwrap().is_empty());
}

#[test]
fn a_peer_answering_a_request_opens_the_session() {
    let hub = Hub::new();
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    a.send_message(B, 0, b"ping", Delivery::Reliable).unwrap();
    assert_eq!(b.run_callbacks(), vec![Event::SessionRequest { peer: A }]);
    b.send_message(A, 0, b"pong", Delivery::Reliable).unwrap();
    assert_eq!(a.run_callbacks(), vec![Event::Connected { peer: B }]);
    assert_eq!(b.run_callbacks(), vec![Event::Connected { peer: A }]);
    assert_eq!(b.receive_messages(0, 8).unwrap()[0].bytes, b"ping");
    assert_eq!(a.receive_messages(0, 8).unwrap()[0].bytes, b"pong");
}

#[test]
fn session_failures_reach_both_sides() {
    let hub = Hub::new();
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    connect(&mut a, &mut b);
    hub.fail_session(A, B, Some(4001));
    assert_eq!(
        a.run_callbacks(),
        vec![Event::SessionFailed {
            peer: B,
            reason: Some(4001)
        }]
    );
    assert_eq!(
        b.run_callbacks(),
        vec![Event::SessionFailed {
            peer: A,
            reason: Some(4001)
        }]
    );
    a.send_message(B, 0, b"again", Delivery::Reliable).unwrap();
    assert_eq!(b.run_callbacks(), vec![Event::SessionRequest { peer: A }]);

    a.send_message(C, 0, b"nobody", Delivery::Reliable).unwrap();
    assert_eq!(
        a.run_callbacks(),
        vec![Event::SessionFailed {
            peer: C,
            reason: None
        }]
    );

    b.accept_session(A).unwrap();
    a.run_callbacks();
    b.shutdown();
    assert_eq!(
        a.run_callbacks(),
        vec![Event::SessionFailed {
            peer: B,
            reason: None
        }]
    );
    assert_eq!(b.receive_messages(0, 8).unwrap_err(), Error::Closed);
}

#[test]
fn bad_sends_are_refused() {
    let hub = Hub::new();
    let mut a = hub.client(A);
    let _b = hub.client(B);
    assert_eq!(
        a.send_message(A, 0, b"me", Delivery::Reliable).unwrap_err(),
        Error::Rejected
    );
    assert_eq!(
        a.send_message(B, 0, &vec![0; MESSAGE_MAX + 1], Delivery::Reliable)
            .unwrap_err(),
        Error::Rejected
    );
    a.send_message(B, 0, &vec![0; MESSAGE_MAX], Delivery::Reliable)
        .unwrap();
    a.shutdown();
    assert_eq!(
        a.send_message(B, 0, b"x", Delivery::Reliable).unwrap_err(),
        Error::Closed
    );
    assert_eq!(a.me().unwrap_err(), Error::Closed);
}
