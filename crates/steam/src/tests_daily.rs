use super::{
    Clock, Daily, Date, Display, Error, Event, Hub, Repeat, Sort, SteamApi, Today, Upload, User,
    checksum, daily_seed,
};

const A: User = User(76_561_197_960_265_731);
const B: User = User(76_561_197_960_265_732);
const OCT_5_2026: u64 = 1_791_158_400;

fn date(year: u64, month: u8, day: u8) -> Date {
    Date::new(year, month, day).unwrap()
}

#[test]
fn dates_come_from_utc_seconds() {
    assert_eq!(Date::from_unix(0), date(1970, 1, 1));
    assert_eq!(Date::from_unix(OCT_5_2026), date(2026, 10, 5));
    assert_eq!(Date::from_unix(OCT_5_2026 + 86_399), date(2026, 10, 5));
    assert_eq!(Date::from_unix(OCT_5_2026 + 86_400), date(2026, 10, 6));
    assert_eq!(Date::from_unix(OCT_5_2026 - 1), date(2026, 10, 4));
    assert_eq!(Date::from_unix(951_782_400), date(2000, 2, 29));
    assert_eq!(Date::from_unix(1_835_395_200), date(2028, 2, 29));
    assert_eq!(Date::from_unix(4_107_542_400), date(2100, 3, 1));
    assert_eq!(Date::from_unix(4_107_542_399), date(2100, 2, 28));
    assert_eq!(date(2026, 10, 5).to_string(), "2026-10-05");
    assert_eq!(date(1970, 1, 1).to_string(), "1970-01-01");
    let parts = date(2028, 2, 29);
    assert_eq!((parts.year(), parts.month(), parts.day()), (2028, 2, 29));

    assert!(Date::new(2026, 2, 29).is_none());
    assert!(Date::new(2100, 2, 29).is_none());
    assert!(Date::new(2000, 2, 29).is_some());
    assert!(Date::new(2026, 13, 1).is_none());
    assert!(Date::new(2026, 4, 31).is_none());
    assert!(Date::new(2026, 1, 0).is_none());
}

#[test]
fn the_daily_seed_is_golden() {
    assert_eq!(
        daily_seed("example-game", date(2026, 10, 5)),
        0xe690_4dba_18fb_3efa
    );
    assert_eq!(
        daily_seed("example-game", date(2026, 10, 6)),
        0x5c3d_63d1_7a6f_e5c9
    );
    assert_eq!(
        daily_seed("other", date(2026, 10, 5)),
        0x0424_167d_bc94_6e99
    );
    assert_eq!(daily_seed("", date(1970, 1, 1)), 0xa0a4_f53f_4573_281b);
    assert_eq!(
        daily_seed("example-game", date(2028, 2, 29)),
        0x82d4_ef26_e011_60f0
    );
    assert_eq!(checksum(b""), 0xc381_7c01_6ba4_ff30);
    assert_eq!(checksum(b"abc"), 0x29e3_2c04_ec3f_9c30);
}

#[test]
fn the_date_prefers_steam_server_time_over_the_system_clock() {
    assert_eq!(
        Today::pick(Some(OCT_5_2026), 0),
        Today {
            date: date(2026, 10, 5),
            clock: Clock::Server
        }
    );
    assert_eq!(
        Today::pick(None, OCT_5_2026),
        Today {
            date: date(2026, 10, 5),
            clock: Clock::System
        }
    );

    let hub = Hub::new();
    let steam = hub.client(A);
    let daily = Daily::new(
        "example-game",
        "daily",
        Repeat::Best,
        Sort::Descending,
        Display::Numeric,
    );
    assert_eq!(daily.today(&steam, OCT_5_2026).clock, Clock::System);
    hub.set_server_time(Some(OCT_5_2026 + 86_400));
    assert_eq!(
        daily.today(&steam, OCT_5_2026),
        Today {
            date: date(2026, 10, 6),
            clock: Clock::Server
        }
    );
}

#[test]
fn two_clients_with_different_clocks_share_one_run_and_one_board() {
    let hub = Hub::new();
    hub.set_server_time(Some(OCT_5_2026 + 3_600));
    let mut a = hub.client(A);
    let mut b = hub.client(B);
    let daily = Daily::new(
        "example-game",
        "daily",
        Repeat::Best,
        Sort::Descending,
        Display::Numeric,
    );

    let today_a = daily.today(&a, OCT_5_2026 - 7_200);
    let today_b = daily.today(&b, OCT_5_2026 + 90_000);
    assert_eq!(today_a, today_b);
    assert_eq!(today_a.date, date(2026, 10, 5));
    assert_eq!(daily.seed(today_a.date), daily.seed(today_b.date));
    assert_eq!(daily.seed(today_a.date), 0xe690_4dba_18fb_3efa);
    assert_eq!(daily.board_name(today_a.date), "daily-2026-10-05");

    let call_a = daily.open(&mut a, today_a.date).unwrap();
    let call_b = daily.open(&mut b, today_b.date).unwrap();
    let board_of = |events: Vec<Event>, call| match events.as_slice() {
        [
            Event::Board {
                call: got,
                result: Ok(info),
            },
        ] if *got == call => info.clone(),
        other => panic!("expected the daily board, got {other:?}"),
    };
    let on_a = board_of(a.run_callbacks(), call_a);
    let on_b = board_of(b.run_callbacks(), call_b);
    assert_eq!(on_a, on_b);
    assert_eq!(on_a.name, "daily-2026-10-05");

    let tomorrow = daily.board_name(date(2026, 10, 6));
    assert_ne!(tomorrow, on_a.name);
}

#[test]
fn repeat_runs_follow_the_game_rule() {
    let today = date(2026, 10, 5);
    let yesterday = date(2026, 10, 4);
    let first = Daily::new(
        "example-game",
        "daily",
        Repeat::FirstOnly,
        Sort::Descending,
        Display::Numeric,
    );
    assert_eq!(first.upload(today, None), Some(Upload::KeepBest));
    assert_eq!(first.upload(today, Some(yesterday)), Some(Upload::KeepBest));
    assert_eq!(first.upload(today, Some(today)), None);
    let best = Daily {
        repeat: Repeat::Best,
        ..first.clone()
    };
    assert_eq!(best.upload(today, Some(today)), Some(Upload::KeepBest));

    let hub = Hub::new();
    let mut steam = hub.client(A);
    let call = first.open(&mut steam, today).unwrap();
    let board = match steam.run_callbacks().as_slice() {
        [
            Event::Board {
                call: got,
                result: Ok(info),
            },
        ] if *got == call => info.board,
        other => panic!("expected the daily board, got {other:?}"),
    };
    let counted = first
        .submit(&mut steam, board, today, None, 500, &[3])
        .unwrap();
    assert!(counted.is_some());
    let skipped = first
        .submit(&mut steam, board, today, Some(today), 900, &[])
        .unwrap();
    assert_eq!(skipped, None);
    let events = steam.run_callbacks();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::Uploaded { result: Ok(uploaded), .. } if uploaded.score == 500
    ));
    best.submit(&mut steam, board, today, Some(today), 900, &[])
        .unwrap();
    assert!(matches!(
        steam.run_callbacks().as_slice(),
        [Event::Uploaded { result: Ok(uploaded), .. }] if uploaded.changed && uploaded.score == 900
    ));

    steam.shutdown();
    assert_eq!(first.open(&mut steam, today).unwrap_err(), Error::Closed);
    assert_eq!(first.today(&steam, OCT_5_2026).clock, Clock::System);
}
