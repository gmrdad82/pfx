use super::{
    BLOB_CHUNK, BLOB_MAX, BLOB_OPEN_MAX, Blob, Blobs, Delivery, Error, Fake, Hub, SteamApi, User,
    send_blob,
};

const A: User = User(76_561_197_960_265_731);
const B: User = User(76_561_197_960_265_732);
const CHANNEL: u32 = 4;

fn connected(hub: &Hub) -> (Fake, Fake) {
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    a.send_message(B, 0, b"hi", Delivery::Reliable).unwrap();
    b.accept_session(A).unwrap();
    a.run_callbacks();
    b.run_callbacks();
    (a, b)
}

fn log(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index * 31 % 251) as u8).collect()
}

fn packets(id: u32, bytes: &[u8]) -> Vec<Vec<u8>> {
    let hub = Hub::new();
    let (mut a, mut b) = connected(&hub);
    send_blob(&mut a, B, CHANNEL, id, bytes).unwrap();
    b.receive_messages(CHANNEL, 10_000)
        .unwrap()
        .into_iter()
        .map(|message| message.bytes)
        .collect()
}

#[test]
fn a_week_of_input_arrives_in_chunks_and_is_put_back_together() {
    let hub = Hub::new();
    let (mut a, mut b) = connected(&hub);
    let week = log(300_000);
    let chunks = send_blob(&mut a, B, CHANNEL, 41, &week).unwrap();
    assert_eq!(chunks as usize, 300_000usize.div_ceil(BLOB_CHUNK));
    let mut blobs = Blobs::new(CHANNEL);
    assert_eq!(
        blobs.receive(&mut b).unwrap(),
        vec![Blob::Complete {
            peer: A,
            id: 41,
            bytes: week
        }]
    );
    assert_eq!(blobs.pending(), 0);

    send_blob(&mut a, B, CHANNEL, 42, &[]).unwrap();
    send_blob(&mut a, B, CHANNEL, 43, &log(BLOB_CHUNK)).unwrap();
    assert_eq!(
        blobs.receive(&mut b).unwrap(),
        vec![
            Blob::Complete {
                peer: A,
                id: 42,
                bytes: Vec::new()
            },
            Blob::Complete {
                peer: A,
                id: 43,
                bytes: log(BLOB_CHUNK)
            },
        ]
    );
    assert_eq!(
        send_blob(&mut a, B, CHANNEL, 44, &vec![0; BLOB_MAX + 1]).unwrap_err(),
        Error::TooBig
    );
}

#[test]
fn chunks_reassemble_in_any_order_and_repeats_are_ignored() {
    let week = log(5 * BLOB_CHUNK + 7);
    let mut parts = packets(9, &week);
    assert_eq!(parts.len(), 6);
    parts.reverse();
    parts.insert(2, parts[0].clone());
    let mut blobs = Blobs::new(CHANNEL);
    let mut done = Vec::new();
    for part in &parts {
        done.extend(blobs.accept(A, part));
    }
    assert_eq!(
        done,
        vec![Blob::Complete {
            peer: A,
            id: 9,
            bytes: week
        }]
    );
}

#[test]
fn two_senders_with_one_id_do_not_mix() {
    let first = log(2 * BLOB_CHUNK);
    let second: Vec<u8> = first.iter().map(|byte| byte ^ 0xff).collect();
    let mut blobs = Blobs::new(CHANNEL);
    let (from_a, from_b) = (packets(1, &first), packets(1, &second));
    assert_eq!(blobs.accept(A, &from_a[0]), None);
    assert_eq!(blobs.accept(B, &from_b[0]), None);
    assert_eq!(blobs.pending(), 2);
    assert_eq!(
        blobs.accept(B, &from_b[1]),
        Some(Blob::Complete {
            peer: B,
            id: 1,
            bytes: second
        })
    );
    assert_eq!(
        blobs.accept(A, &from_a[1]),
        Some(Blob::Complete {
            peer: A,
            id: 1,
            bytes: first
        })
    );
}

#[test]
fn a_bad_checksum_or_a_misshapen_chunk_is_corrupt() {
    let week = log(2 * BLOB_CHUNK + 1);
    let parts = packets(5, &week);
    let mut blobs = Blobs::new(CHANNEL);
    let mut flipped = parts[1].clone();
    let last = flipped.len() - 1;
    flipped[last] ^= 1;
    assert_eq!(blobs.accept(A, &parts[0]), None);
    assert_eq!(blobs.accept(A, &flipped), None);
    assert_eq!(
        blobs.accept(A, &parts[2]),
        Some(Blob::Corrupt { peer: A, id: 5 })
    );
    assert_eq!(blobs.pending(), 0);

    assert_eq!(blobs.accept(A, &parts[0]), None);
    let mut short = parts[1].clone();
    short.pop();
    assert_eq!(
        blobs.accept(A, &short),
        Some(Blob::Corrupt { peer: A, id: 5 })
    );
    let other = packets(5, &log(2 * BLOB_CHUNK + 2));
    assert_eq!(blobs.accept(A, &parts[0]), None);
    assert_eq!(
        blobs.accept(A, &other[1]),
        Some(Blob::Corrupt { peer: A, id: 5 })
    );
    assert_eq!(blobs.accept(A, b"junk"), None);
    assert_eq!(blobs.accept(A, &[0u8; 40]), None);
}

#[test]
fn a_peer_holds_only_a_few_blobs_open() {
    let mut blobs = Blobs::new(CHANNEL);
    for id in 0..BLOB_OPEN_MAX as u32 + 2 {
        let parts = packets(id, &log(2 * BLOB_CHUNK));
        blobs.accept(A, &parts[0]);
    }
    assert_eq!(blobs.pending(), BLOB_OPEN_MAX);
    let parts = packets(0, &log(2 * BLOB_CHUNK));
    assert!(matches!(
        blobs.accept(A, &parts[1]),
        Some(Blob::Complete { id: 0, .. })
    ));
    let parts = packets(99, &log(2 * BLOB_CHUNK));
    assert_eq!(blobs.accept(B, &parts[0]), None);
    assert_eq!(blobs.pending(), BLOB_OPEN_MAX);
}
