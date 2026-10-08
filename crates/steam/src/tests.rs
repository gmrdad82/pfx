use super::{
    CloudOff, Error, Event, Fake, MAX_SAVE, NAME_MAX, Overlay, PRESENCE_COUNT_MAX,
    PRESENCE_KEY_MAX, PRESENCE_VALUE_MAX, Quota, REDISTRIBUTABLE_LINUX, REDISTRIBUTABLE_WINDOWS,
    SDK_VERSION, STEAMWORKS_VERSION, SteamApi,
};

fn resync(steam: &mut impl SteamApi, achieved: &[&str]) -> Result<(), Error> {
    steam.sync(achieved)
}

#[test]
fn messages_name_the_failure() {
    assert_eq!(Error::NotRunning.to_string(), "Steam is not running");
    assert_eq!(
        Error::CloudOff(CloudOff::Account).to_string(),
        "Steam Cloud is off for this account"
    );
    assert_eq!(
        Error::CloudOff(CloudOff::App).to_string(),
        "Steam Cloud is off for this app"
    );
    assert_eq!(
        Error::CloudOff(CloudOff::AccountAndApp).to_string(),
        "Steam Cloud is off for this account and this app"
    );
    assert_eq!(Error::TooBig.to_string(), "a save is larger than 1 MiB");
    assert_eq!(Error::Offline.to_string(), "Steam's servers did not answer");
    assert_eq!(
        Error::AppId.to_string(),
        "the app id comes from the game's config"
    );
    assert_eq!(STEAMWORKS_VERSION, "0.13.1");
    assert_eq!(SDK_VERSION, "1.64");
    assert_eq!(REDISTRIBUTABLE_LINUX, "libsteam_api.so");
    assert_eq!(REDISTRIBUTABLE_WINDOWS, "steam_api64.dll");
}

#[test]
fn sync_grants_what_steam_lacks_and_leaves_the_rest() {
    let mut steam = Fake::new();
    steam.unlock("kept").unwrap();
    steam.unlock("extra").unwrap();
    steam.store().unwrap();
    assert_eq!(steam.stores(), 1);

    resync(&mut steam, &["kept", "missed"]).unwrap();
    assert!(steam.achieved("kept").unwrap());
    assert!(steam.achieved("missed").unwrap());
    assert!(steam.achieved("extra").unwrap());
    assert_eq!(steam.stores(), 2);

    resync(&mut steam, &["kept", "missed"]).unwrap();
    assert_eq!(steam.stores(), 2);

    resync(&mut steam, &[]).unwrap();
    assert!(steam.achieved("extra").unwrap());
    assert_eq!(steam.stores(), 2);

    resync(&mut steam, &["once", "once"]).unwrap();
    assert!(steam.achieved("once").unwrap());
    assert_eq!(steam.stores(), 3);
}

#[test]
fn sync_rejects_a_bad_name_before_granting() {
    let mut steam = Fake::new();
    steam.unlock("kept").unwrap();
    let error = steam.sync(&["kept", ""]).unwrap_err();
    assert_eq!(error, Error::Rejected);
    assert!(!steam.achieved("other").unwrap());
    assert_eq!(steam.stores(), 0);
}

#[test]
fn clear_is_only_for_a_test_and_sync_grants_it_again() {
    let mut steam = Fake::new();
    steam.sync(&["won"]).unwrap();
    steam.clear("won").unwrap();
    assert!(!steam.achieved("won").unwrap());
    steam.sync(&["won"]).unwrap();
    assert!(steam.achieved("won").unwrap());
    assert_eq!(steam.unlock("").unwrap_err(), Error::Rejected);
    let long = "n".repeat(NAME_MAX + 1);
    assert_eq!(steam.unlock(&long).unwrap_err(), Error::Rejected);
}

#[test]
fn stats_keep_an_integer_apart_from_a_float() {
    let mut steam = Fake::new();
    assert_eq!(steam.stat_i32("runs").unwrap(), 0);
    assert_eq!(steam.stat_f32("runs").unwrap(), 0.0);
    steam.set_stat_i32("runs", 4).unwrap();
    steam.set_stat_f32("hours", 1.5).unwrap();
    assert_eq!(steam.stat_i32("runs").unwrap(), 4);
    assert_eq!(steam.stat_f32("hours").unwrap(), 1.5);
    assert_eq!(steam.stat_f32("runs").unwrap_err(), Error::Rejected);
    assert_eq!(steam.stat_i32("hours").unwrap_err(), Error::Rejected);
    assert_eq!(
        steam.set_stat_f32("runs", 1.0).unwrap_err(),
        Error::Rejected
    );
    assert_eq!(
        steam.set_stat_f32("hours", f32::NAN).unwrap_err(),
        Error::Rejected
    );
    assert_eq!(
        steam.set_stat_f32("hours", f32::INFINITY).unwrap_err(),
        Error::Rejected
    );
    assert_eq!(steam.stat_f32("hours").unwrap(), 1.5);
    steam.store().unwrap();
    assert_eq!(steam.stores(), 1);
}

#[test]
fn cloud_round_trip_quota_and_the_one_mebibyte_cap() {
    let mut steam = Fake::with_quota(8);
    assert_eq!(
        steam.quota().unwrap(),
        Quota {
            total: 8,
            available: 8
        }
    );
    steam.write("save", b"12345").unwrap();
    assert!(steam.exists("save").unwrap());
    assert_eq!(steam.read("save").unwrap(), b"12345");
    assert_eq!(steam.quota().unwrap().available, 3);

    steam.write("save", b"12").unwrap();
    assert_eq!(steam.read("save").unwrap(), b"12");
    assert_eq!(steam.quota().unwrap().available, 6);

    assert_eq!(steam.write("save", b"123456789").unwrap_err(), Error::Quota);
    assert_eq!(steam.read("save").unwrap(), b"12");

    steam.write("other", b"1234").unwrap();
    assert_eq!(steam.quota().unwrap().available, 2);
    assert_eq!(steam.write("third", b"123").unwrap_err(), Error::Quota);

    steam.delete("save").unwrap();
    assert!(!steam.exists("save").unwrap());
    assert_eq!(steam.read("save").unwrap_err(), Error::Missing);
    assert_eq!(steam.delete("save").unwrap_err(), Error::Missing);
    assert_eq!(steam.quota().unwrap().available, 4);

    assert_eq!(
        steam.write("save", &vec![0; MAX_SAVE + 1]).unwrap_err(),
        Error::TooBig
    );
    let mut room = Fake::new();
    room.write("save", &vec![0; MAX_SAVE]).unwrap();
    assert_eq!(room.quota().unwrap().available, 0);
    assert_eq!(room.write("more", b"x").unwrap_err(), Error::Quota);
}

#[test]
fn cloud_off_is_a_distinct_error_for_the_account_and_the_app() {
    let mut steam = Fake::new();
    steam.write("save", b"ok").unwrap();
    steam.set_account_cloud(false);
    assert_eq!(
        steam.write("save", b"no").unwrap_err(),
        Error::CloudOff(CloudOff::Account)
    );
    assert_eq!(
        steam.read("save").unwrap_err(),
        Error::CloudOff(CloudOff::Account)
    );
    assert_eq!(
        steam.exists("save").unwrap_err(),
        Error::CloudOff(CloudOff::Account)
    );
    assert_eq!(
        steam.delete("save").unwrap_err(),
        Error::CloudOff(CloudOff::Account)
    );
    assert_eq!(
        steam.quota().unwrap_err(),
        Error::CloudOff(CloudOff::Account)
    );
    steam.set_account_cloud(true);
    steam.set_app_cloud(false);
    assert_eq!(
        steam.read("save").unwrap_err(),
        Error::CloudOff(CloudOff::App)
    );
    steam.set_account_cloud(false);
    assert_eq!(
        steam.quota().unwrap_err(),
        Error::CloudOff(CloudOff::AccountAndApp)
    );
    steam.set_account_cloud(true);
    steam.set_app_cloud(true);
    assert_eq!(steam.read("save").unwrap(), b"ok");
    assert_eq!(steam.write("", b"x").unwrap_err(), Error::Rejected);
}

#[test]
fn overlay_opens_and_closes_and_shutdown_drops_what_follows() {
    let mut steam = Fake::new();
    assert!(steam.run_callbacks().is_empty());
    steam.push_overlay(true);
    steam.push_overlay(false);
    assert_eq!(
        steam.run_callbacks(),
        vec![
            Event::Overlay(Overlay::Opened),
            Event::Overlay(Overlay::Closed)
        ]
    );
    assert!(steam.run_callbacks().is_empty());

    steam.push_overlay(true);
    steam.shutdown();
    assert!(steam.run_callbacks().is_empty());
    assert_eq!(steam.unlock("won").unwrap_err(), Error::Closed);
    assert_eq!(steam.store().unwrap_err(), Error::Closed);
    assert_eq!(steam.write("save", b"x").unwrap_err(), Error::Closed);
    steam.shutdown();
}

#[test]
fn presence_sets_and_clears_one_key() {
    let mut steam = Fake::new();
    steam.set_presence("steam_display", "#Playing").unwrap();
    steam.set_presence("connect", "1").unwrap();
    assert_eq!(steam.presence("steam_display"), Some("#Playing"));
    steam.set_presence("steam_display", "#Menu").unwrap();
    assert_eq!(steam.presence("steam_display"), Some("#Menu"));
    steam.clear_presence("steam_display").unwrap();
    assert_eq!(steam.presence("steam_display"), None);
    assert_eq!(steam.presence("connect"), Some("1"));
    steam.clear_presence("missing").unwrap();
    assert_eq!(steam.set_presence("", "x").unwrap_err(), Error::Rejected);
    assert_eq!(
        steam.set_presence("status", "").unwrap_err(),
        Error::Rejected
    );
    assert_eq!(steam.clear_presence("").unwrap_err(), Error::Rejected);

    let long_key = "k".repeat(PRESENCE_KEY_MAX + 1);
    let long_value = "v".repeat(PRESENCE_VALUE_MAX + 1);
    assert_eq!(
        steam.set_presence(&long_key, "x").unwrap_err(),
        Error::Rejected
    );
    assert_eq!(
        steam.set_presence("status", &long_value).unwrap_err(),
        Error::Rejected
    );
    let edge_key = "k".repeat(PRESENCE_KEY_MAX);
    let edge_value = "v".repeat(PRESENCE_VALUE_MAX);
    steam.set_presence(&edge_key, &edge_value).unwrap();

    let mut full = Fake::new();
    for index in 0..PRESENCE_COUNT_MAX {
        full.set_presence(&format!("k{index}"), "v").unwrap();
    }
    assert_eq!(
        full.set_presence("overflow", "v").unwrap_err(),
        Error::Rejected
    );
    full.set_presence("k0", "again").unwrap();
    full.clear_presence("k0").unwrap();
    full.set_presence("overflow", "v").unwrap();
}

#[cfg(feature = "live")]
#[test]
fn init_rejects_a_missing_app_id_without_calling_steam() {
    let Err(error) = super::Steam::init(0) else {
        panic!("app id 0 must be rejected");
    };
    assert_eq!(error, Error::AppId);
}

#[cfg(feature = "live")]
#[test]
#[ignore = "needs a running Steam client and Spacewar (app 480); never part of the gate"]
fn spacewar_client() {
    let app_id = 480;
    let mut steam = super::Steam::init(app_id).expect("Steam is not running");
    assert_eq!(steam.app_id(), app_id);
    let _ = steam.run_callbacks();
    if steam.set_presence("status", "pfx-steam").is_ok() {
        steam.clear_presence("status").unwrap();
    }
    let probe = "pfx-steam-probe";
    match steam.write(probe, b"ok") {
        Ok(()) => {
            let read = steam.read(probe);
            let deleted = steam.delete(probe);
            assert_eq!(read.unwrap(), b"ok");
            deleted.unwrap();
            assert!(!steam.exists(probe).unwrap());
        }
        Err(Error::CloudOff(_)) => {}
        Err(error) => panic!("{error}"),
    }
    match steam.quota() {
        Ok(quota) => assert!(quota.total > 0),
        Err(Error::CloudOff(_)) => {}
        Err(error) => panic!("{error}"),
    }
    steam.shutdown();
    assert!(steam.run_callbacks().is_empty());
}
