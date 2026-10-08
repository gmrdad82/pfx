use super::{
    Board, BoardInfo, Call, DETAILS_MAX, Display, Entry, Error, Event, Fake, Hub, Range, Sort,
    SteamApi, Upload, Uploaded, User,
};

const A: User = User(76_561_197_960_265_731);
const B: User = User(76_561_197_960_265_732);
const C: User = User(76_561_197_960_265_733);

fn board(steam: &mut Fake, call: Call) -> Result<BoardInfo, Error> {
    let events = steam.run_callbacks();
    assert_eq!(events.len(), 1);
    match events.into_iter().next() {
        Some(Event::Board { call: got, result }) if got == call => result,
        other => panic!("expected the board for {call:?}, got {other:?}"),
    }
}

fn create(steam: &mut Fake, name: &str, sort: Sort) -> Board {
    let call = steam
        .find_or_create_board(name, sort, Display::Numeric)
        .unwrap();
    board(steam, call).unwrap().board
}

fn upload(steam: &mut Fake, board: Board, score: i32, how: Upload) -> Uploaded {
    upload_details(steam, board, score, &[], how)
}

fn upload_details(
    steam: &mut Fake,
    board: Board,
    score: i32,
    details: &[i32],
    how: Upload,
) -> Uploaded {
    let call = steam.upload_score(board, score, details, how).unwrap();
    match steam.run_callbacks().as_slice() {
        [
            Event::Uploaded {
                call: got,
                board: on,
                result: Ok(uploaded),
            },
        ] if *got == call && *on == board => *uploaded,
        other => panic!("expected an upload, got {other:?}"),
    }
}

fn download(steam: &mut Fake, board: Board, range: Range, details: usize) -> Vec<Entry> {
    let call = steam.download_scores(board, range, details).unwrap();
    match steam.run_callbacks().into_iter().next() {
        Some(Event::Scores {
            call: got,
            board: on,
            result: Ok(entries),
        }) if got == call && on == board => entries,
        other => panic!("expected scores, got {other:?}"),
    }
}

fn order(entries: &[Entry]) -> Vec<(User, u32, i32)> {
    entries
        .iter()
        .map(|entry| (entry.user, entry.rank, entry.score))
        .collect()
}

#[test]
fn a_board_is_found_or_created_with_its_sort_and_display() {
    let hub = Hub::new();
    let mut a = hub.client(A);
    let mut b = hub.client(B);

    let call = a.find_board("speed").unwrap();
    assert_eq!(board(&mut a, call), Err(Error::Unknown));
    let call = b.find_board("missing").unwrap();
    assert_eq!(board(&mut b, call), Err(Error::Unknown));

    let call = a
        .find_or_create_board("speed", Sort::Ascending, Display::Milliseconds)
        .unwrap();
    let made = board(&mut a, call).unwrap();
    assert_eq!(made.name, "speed");
    assert_eq!(made.sort, Sort::Ascending);
    assert_eq!(made.display, Display::Milliseconds);
    assert_eq!(made.entries, 0);

    let call = b
        .find_or_create_board("speed", Sort::Descending, Display::Numeric)
        .unwrap();
    assert_eq!(board(&mut b, call).unwrap(), made);
    let call = b.find_board("speed").unwrap();
    assert_eq!(board(&mut b, call).unwrap(), made);

    assert_eq!(
        a.find_or_create_board("", Sort::Ascending, Display::Numeric)
            .unwrap_err(),
        Error::Rejected
    );
    assert_eq!(
        a.find_board(&"n".repeat(crate::NAME_MAX + 1)).unwrap_err(),
        Error::Rejected
    );
}

#[test]
fn calls_answer_on_the_next_run_callbacks_with_their_own_ids() {
    let mut steam = Fake::new();
    let first = steam
        .find_or_create_board("one", Sort::Descending, Display::Numeric)
        .unwrap();
    let second = steam.find_board("two").unwrap();
    assert_ne!(first, second);
    let events = steam.run_callbacks();
    assert_eq!(events.len(), 2);
    assert!(matches!(&events[0], Event::Board { call, result: Ok(_) } if *call == first));
    assert_eq!(
        events[1],
        Event::Board {
            call: second,
            result: Err(Error::Unknown)
        }
    );
    assert!(steam.run_callbacks().is_empty());
}

#[test]
fn uploads_rank_in_board_order_and_downloads_follow_it() {
    let hub = Hub::new();
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    let mut c = hub.client(C);
    let high = create(&mut a, "high", Sort::Descending);

    let first = upload(&mut a, high, 100, Upload::KeepBest);
    assert_eq!(
        first,
        Uploaded {
            score: 100,
            changed: true,
            rank: 1,
            previous_rank: 0
        }
    );
    upload(&mut b, high, 300, Upload::KeepBest);
    upload(&mut c, high, 200, Upload::KeepBest);

    let global = download(&mut a, high, Range::Global { first: 1, last: 10 }, 0);
    assert_eq!(order(&global), vec![(B, 1, 300), (C, 2, 200), (A, 3, 100)]);
    let top = download(&mut a, high, Range::Global { first: 2, last: 2 }, 0);
    assert_eq!(order(&top), vec![(C, 2, 200)]);
    let past = download(&mut a, high, Range::Global { first: 5, last: 9 }, 0);
    assert!(past.is_empty());

    let low = create(&mut a, "low", Sort::Ascending);
    upload(&mut a, low, 100, Upload::KeepBest);
    upload(&mut b, low, 300, Upload::KeepBest);
    upload(&mut c, low, 200, Upload::KeepBest);
    let global = download(&mut b, low, Range::Global { first: 1, last: 3 }, 0);
    assert_eq!(order(&global), vec![(A, 1, 100), (C, 2, 200), (B, 3, 300)]);
}

#[test]
fn keep_best_holds_the_better_score_and_force_replaces_it() {
    let hub = Hub::new();
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    let high = create(&mut a, "high", Sort::Descending);
    upload(&mut a, high, 100, Upload::KeepBest);
    upload(&mut b, high, 300, Upload::KeepBest);

    let worse = upload(&mut a, high, 50, Upload::KeepBest);
    assert_eq!(
        worse,
        Uploaded {
            score: 50,
            changed: false,
            rank: 2,
            previous_rank: 2
        }
    );
    let same = upload(&mut a, high, 100, Upload::KeepBest);
    assert!(!same.changed);
    let better = upload(&mut a, high, 400, Upload::KeepBest);
    assert_eq!(
        better,
        Uploaded {
            score: 400,
            changed: true,
            rank: 1,
            previous_rank: 2
        }
    );
    let forced = upload(&mut a, high, 10, Upload::Force);
    assert_eq!(
        forced,
        Uploaded {
            score: 10,
            changed: true,
            rank: 2,
            previous_rank: 1
        }
    );
    let global = download(&mut a, high, Range::Global { first: 1, last: 5 }, 0);
    assert_eq!(order(&global), vec![(B, 1, 300), (A, 2, 10)]);

    let low = create(&mut a, "low", Sort::Ascending);
    upload(&mut a, low, 100, Upload::KeepBest);
    assert!(!upload(&mut a, low, 150, Upload::KeepBest).changed);
    assert!(upload(&mut a, low, 90, Upload::KeepBest).changed);
}

#[test]
fn a_tie_ranks_the_earlier_score_first() {
    let hub = Hub::new();
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    let high = create(&mut a, "high", Sort::Descending);
    upload(&mut b, high, 100, Upload::KeepBest);
    upload(&mut a, high, 100, Upload::KeepBest);
    let global = download(&mut a, high, Range::Global { first: 1, last: 2 }, 0);
    assert_eq!(order(&global), vec![(B, 1, 100), (A, 2, 100)]);
}

#[test]
fn around_the_user_and_friends_ranges() {
    let hub = Hub::new();
    let users: Vec<User> = (0..6).map(|index| User(1_000 + index)).collect();
    let mut clients: Vec<Fake> = users.iter().map(|user| hub.client(*user)).collect();
    let high = create(&mut clients[0], "high", Sort::Descending);
    for (index, client) in clients.iter_mut().enumerate() {
        upload(client, high, 10 * (index as i32 + 1), Upload::KeepBest);
    }

    let around = download(
        &mut clients[2],
        high,
        Range::AroundUser {
            before: 1,
            after: 2,
        },
        0,
    );
    assert_eq!(
        order(&around),
        vec![
            (users[3], 3, 40),
            (users[2], 4, 30),
            (users[1], 5, 20),
            (users[0], 6, 10)
        ]
    );
    let edge = download(
        &mut clients[5],
        high,
        Range::AroundUser {
            before: 3,
            after: 1,
        },
        0,
    );
    assert_eq!(order(&edge), vec![(users[5], 1, 60), (users[4], 2, 50)]);

    hub.befriend(users[0], users[4]);
    hub.befriend(users[0], users[1]);
    hub.befriend(users[2], users[3]);
    let friends = download(&mut clients[0], high, Range::Friends, 0);
    assert_eq!(
        order(&friends),
        vec![(users[4], 2, 50), (users[1], 5, 20), (users[0], 6, 10)]
    );

    let mut stranger = hub.client(User(9_999));
    let none = download(
        &mut stranger,
        high,
        Range::AroundUser {
            before: 2,
            after: 2,
        },
        0,
    );
    assert!(none.is_empty());
    let alone = download(&mut stranger, high, Range::Friends, 0);
    assert!(alone.is_empty());
}

#[test]
fn details_ride_with_the_score_and_are_capped() {
    let hub = Hub::new();
    let mut a = hub.client(A);
    let high = create(&mut a, "high", Sort::Descending);
    upload_details(&mut a, high, 5, &[7, 8, 9], Upload::KeepBest);
    let two = download(&mut a, high, Range::Global { first: 1, last: 1 }, 2);
    assert_eq!(two[0].details, vec![7, 8]);
    let all = download(&mut a, high, Range::Global { first: 1, last: 1 }, 100);
    assert_eq!(all[0].details, vec![7, 8, 9]);
    let none = download(&mut a, high, Range::Global { first: 1, last: 1 }, 0);
    assert!(none[0].details.is_empty());

    let most = vec![1; DETAILS_MAX];
    upload_details(&mut a, high, 6, &most, Upload::KeepBest);
    assert_eq!(
        a.upload_score(high, 7, &vec![1; DETAILS_MAX + 1], Upload::KeepBest)
            .unwrap_err(),
        Error::Rejected
    );
}

#[test]
fn bad_boards_and_ranges_are_refused() {
    let mut steam = Fake::new();
    assert_eq!(
        steam
            .download_scores(Board(1), Range::Global { first: 0, last: 3 }, 0)
            .unwrap_err(),
        Error::Rejected
    );
    assert_eq!(
        steam
            .download_scores(Board(1), Range::Global { first: 4, last: 3 }, 0)
            .unwrap_err(),
        Error::Rejected
    );
    assert_eq!(
        steam
            .download_scores(
                Board(1),
                Range::Global {
                    first: 1,
                    last: u32::MAX
                },
                0
            )
            .unwrap_err(),
        Error::Rejected
    );
    assert_eq!(
        steam
            .download_scores(
                Board(1),
                Range::AroundUser {
                    before: u32::MAX,
                    after: 0
                },
                0
            )
            .unwrap_err(),
        Error::Rejected
    );
    let call = steam
        .upload_score(Board(42), 1, &[], Upload::KeepBest)
        .unwrap();
    assert_eq!(
        steam.run_callbacks(),
        vec![Event::Uploaded {
            call,
            board: Board(42),
            result: Err(Error::Unknown)
        }]
    );
    steam.shutdown();
    assert_eq!(steam.find_board("x").unwrap_err(), Error::Closed);
}
