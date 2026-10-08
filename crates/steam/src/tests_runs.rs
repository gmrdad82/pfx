use super::{
    Board, DETAILS_MAX, Daily, Display, Entry, Error, Event, Fake, Hub, RUN_MARK, Range, Repeat,
    Sort, SteamApi, Ugc, Upload, Uploaded, User, run_details, split_run,
};

const A: User = User(76_561_197_960_265_731);
const B: User = User(76_561_197_960_265_732);

fn one(steam: &mut Fake) -> Event {
    let mut events = steam.run_callbacks();
    assert_eq!(events.len(), 1, "{events:?}");
    events.remove(0)
}

fn board(steam: &mut Fake, daily: &Daily) -> Board {
    daily
        .open(steam, super::Date::new(2026, 10, 5).unwrap())
        .unwrap();
    match one(steam) {
        Event::Board {
            result: Ok(info), ..
        } => info.board,
        other => panic!("{other:?}"),
    }
}

fn uploaded(steam: &mut Fake) -> Uploaded {
    match one(steam) {
        Event::Uploaded {
            result: Ok(uploaded),
            ..
        } => uploaded,
        other => panic!("{other:?}"),
    }
}

fn scores(steam: &mut Fake, board: Board, range: Range, details: usize) -> Vec<Entry> {
    steam.download_scores(board, range, details).unwrap();
    match one(steam) {
        Event::Scores {
            result: Ok(entries),
            ..
        } => entries,
        other => panic!("{other:?}"),
    }
}

fn shared(steam: &mut Fake, log: &[u8]) -> Ugc {
    steam.write("run.log", log).unwrap();
    let call = steam.share_file("run.log").unwrap();
    match one(steam) {
        Event::Shared {
            call: got,
            result: Ok(ugc),
        } if got == call => ugc,
        other => panic!("{other:?}"),
    }
}

fn daily() -> Daily {
    Daily::new(
        "example-game",
        "daily",
        Repeat::FirstOnly,
        Sort::Descending,
        Display::Numeric,
    )
}

#[test]
fn a_zero_at_the_start_marks_the_attempt_and_a_better_score_replaces_it() {
    let hub = Hub::new();
    let mut a = hub.client(A);
    let daily = daily();
    let board = board(&mut a, &daily);
    let me = a.me().unwrap();

    daily.check(&mut a, board).unwrap();
    let before = match one(&mut a) {
        Event::Scores {
            result: Ok(entries),
            ..
        } => entries,
        other => panic!("{other:?}"),
    };
    assert!(!Daily::used(&before, me));

    assert_eq!(daily.marker(), 0);
    daily.start(&mut a, board).unwrap();
    let marked = uploaded(&mut a);
    assert!(marked.changed);
    assert_eq!(marked.score, 0);
    daily.check(&mut a, board).unwrap();
    let after = match one(&mut a) {
        Event::Scores {
            result: Ok(entries),
            ..
        } => entries,
        other => panic!("{other:?}"),
    };
    assert!(Daily::used(&after, me));
    assert_eq!(after[0].score, 0);

    let log = b"week log".to_vec();
    let run = shared(&mut a, &log);
    a.upload_run(board, 420, &[7], Upload::KeepBest, run)
        .unwrap();
    let better = uploaded(&mut a);
    assert!(better.changed);
    a.attach_ugc(board, run).unwrap();
    assert_eq!(hub.attached(board, A), Some(run));

    let top = scores(&mut a, board, Range::Global { first: 1, last: 5 }, 4);
    assert_eq!(top.len(), 1);
    assert_eq!(top[0].score, 420);
    assert_eq!(top[0].details, vec![7]);
    assert_eq!(top[0].run, Some(run));

    let ascending = Daily {
        sort: Sort::Ascending,
        ..daily
    };
    assert_eq!(ascending.marker(), i32::MAX);
}

#[test]
fn viewers_download_each_entrys_run_and_hide_what_does_not_reproduce() {
    let hub = Hub::new();
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    let daily = daily();
    let board = board(&mut a, &daily);
    board_on(&mut b, &daily);

    let honest = b"moves that score 300".to_vec();
    let run = shared(&mut a, &honest);
    a.upload_run(board, 300, &[], Upload::KeepBest, run)
        .unwrap();
    assert!(uploaded(&mut a).changed);
    a.attach_ugc(board, run).unwrap();

    let forged = b"moves that score 10".to_vec();
    let run = shared(&mut b, &forged);
    b.upload_run(board, 9_999, &[], Upload::KeepBest, run)
        .unwrap();
    assert!(uploaded(&mut b).changed);
    b.attach_ugc(board, run).unwrap();

    let mut viewer = hub.client(User(1));
    let entries = scores(&mut viewer, board, Range::Global { first: 1, last: 10 }, 0);
    let replay = |log: &[u8]| -> i32 {
        std::str::from_utf8(log)
            .ok()
            .and_then(|text| text.split_whitespace().nth(3))
            .and_then(|score| score.parse().ok())
            .unwrap_or(-1)
    };
    let mut shown = Vec::new();
    for entry in &entries {
        let ugc = entry.run.unwrap();
        let call = viewer.download_ugc(ugc).unwrap();
        let log = match one(&mut viewer) {
            Event::UgcDownloaded {
                call: got,
                ugc: of,
                result: Ok(bytes),
            } if got == call && of == ugc => bytes,
            other => panic!("{other:?}"),
        };
        if replay(&log) == entry.score {
            shown.push(entry.user);
        }
    }
    assert_eq!(entries.len(), 2);
    assert_eq!(shown, vec![A]);
}

fn board_on(steam: &mut Fake, daily: &Daily) -> Board {
    board(steam, daily)
}

#[test]
fn run_details_round_trip_and_bad_calls_are_refused() {
    let run = Ugc(0xfedc_ba98_7654_3210);
    let details = run_details(run, &[1, -2]);
    assert_eq!(details[0], RUN_MARK);
    assert_eq!(split_run(&details, 8), (Some(run), vec![1, -2]));
    assert_eq!(split_run(&details, 1), (Some(run), vec![1]));
    assert_eq!(split_run(&[5, 6], 8), (None, vec![5, 6]));
    assert_eq!(split_run(&[RUN_MARK, 1], 8), (None, vec![RUN_MARK, 1]));

    let hub = Hub::new();
    let mut a = hub.client(A);
    let board = board(&mut a, &daily());
    assert_eq!(
        a.upload_run(board, 1, &vec![0; DETAILS_MAX - 2], Upload::KeepBest, run)
            .unwrap_err(),
        Error::Rejected
    );
    assert_eq!(a.attach_ugc(board, run).unwrap_err(), Error::Missing);
    let mine = shared(&mut a, b"x");
    assert_eq!(a.attach_ugc(board, mine).unwrap_err(), Error::Rejected);
    assert_eq!(a.attach_ugc(Board(77), mine).unwrap_err(), Error::Unknown);

    let call = a.share_file("missing.log").unwrap();
    assert_eq!(
        one(&mut a),
        Event::Shared {
            call,
            result: Err(Error::Missing)
        }
    );
    let call = a.download_ugc(Ugc(5)).unwrap();
    assert_eq!(
        one(&mut a),
        Event::UgcDownloaded {
            call,
            ugc: Ugc(5),
            result: Err(Error::Missing)
        }
    );
    a.set_account_cloud(false);
    assert!(matches!(a.share_file("run.log"), Err(Error::CloudOff(_))));
}
