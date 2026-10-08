use std::collections::HashMap;

use super::*;
use crate::window::{
    DisplayMode, Fallback, FullscreenPlan, LetterboxRounding, Monitor, MonitorChoice, Monitors,
    PlatformCaps, Point, Rect, RenderScale, Size, VideoMode, VideoModeChoice, WindowOps,
    letterbox_with, plan_display_mode, viewport_with,
};
use crate::{FrameTimings, PassTiming};

const LCD: Device = Device::SteamDeck {
    model: DeckModel::Lcd,
};
const OLED: Device = Device::SteamDeck {
    model: DeckModel::Oled,
};
const WAYLAND: PlatformCaps = PlatformCaps { exclusive: false };
const X11: PlatformCaps = PlatformCaps { exclusive: true };

fn size(width: u32, height: u32) -> Size {
    Size { width, height }
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-2
}

fn rect_near(actual: Rect, x: f32, y: f32, width: f32, height: f32) -> bool {
    near(actual.x, x)
        && near(actual.y, y)
        && near(actual.width, width)
        && near(actual.height, height)
}

#[derive(Default)]
struct Source {
    vars: HashMap<&'static str, &'static str>,
    product: Option<&'static str>,
}

impl DeviceSource for Source {
    fn var(&self, name: &str) -> Option<String> {
        self.vars.get(name).map(|value| (*value).to_owned())
    }

    fn product_name(&self) -> Option<String> {
        self.product.map(str::to_owned)
    }
}

fn source(vars: &[(&'static str, &'static str)], product: Option<&'static str>) -> Source {
    Source {
        vars: vars.iter().copied().collect(),
        product,
    }
}

#[test]
fn the_device_comes_from_steam_then_the_product_name_then_gamescope() {
    let steam = detect(&source(&[("SteamDeck", "1")], None));
    assert_eq!(steam.device, LCD);
    assert_eq!(steam.evidence, Evidence::SteamDeckVar);
    let steam_oled = detect(&source(&[("SteamDeck", "1")], Some("Galileo\n")));
    assert_eq!(steam_oled.device, OLED);
    assert_eq!(steam_oled.evidence, Evidence::SteamDeckVar);

    let jupiter = detect(&source(&[], Some("Jupiter\n")));
    assert_eq!(jupiter.device, LCD);
    assert_eq!(jupiter.evidence, Evidence::ProductName("Jupiter".into()));
    let galileo = detect(&source(&[("SteamDeck", "0")], Some("Galileo")));
    assert_eq!(galileo.device, OLED);
    assert_eq!(galileo.evidence, Evidence::ProductName("Galileo".into()));

    let session = detect(&source(
        &[("XDG_CURRENT_DESKTOP", "gamescope")],
        Some("B650"),
    ));
    assert_eq!(session.device, LCD);
    assert_eq!(session.evidence, Evidence::Gamescope);
    let nested = detect(&source(
        &[("GAMESCOPE_WAYLAND_DISPLAY", "gamescope-0")],
        None,
    ));
    assert_eq!(nested.evidence, Evidence::Gamescope);
    assert!(
        detect(&source(&[("GAMESCOPE_WAYLAND_DISPLAY", "")], None))
            .device
            .eq(&Device::Desktop)
    );

    let desktop = detect(&source(
        &[("XDG_CURRENT_DESKTOP", "Hyprland")],
        Some("B650"),
    ));
    assert_eq!(desktop.device, Device::Desktop);
    assert_eq!(desktop.evidence, Evidence::Nothing);
    assert_eq!(detect(&Source::default()).device, Device::Desktop);

    let opted_out = detect(&source(
        &[
            ("PFX_DEVICE", "desktop"),
            ("XDG_CURRENT_DESKTOP", "gamescope"),
            ("SteamDeck", "1"),
        ],
        Some("Jupiter"),
    ));
    assert_eq!(opted_out.device, Device::Desktop);
    assert_eq!(opted_out.evidence, Evidence::Override);
    let forced = detect(&source(&[("PFX_DEVICE", " deck-oled ")], Some("B650")));
    assert_eq!(forced.device, OLED);
    assert_eq!(forced.evidence, Evidence::Override);
    let lcd = detect(&source(&[("PFX_DEVICE", "deck-lcd")], None));
    assert_eq!(lcd.device, LCD);
    assert_eq!(lcd.evidence, Evidence::Override);
    let unknown = detect(&source(&[("PFX_DEVICE", "switch")], Some("Galileo")));
    assert_eq!(unknown.device, OLED);
    assert_eq!(unknown.evidence, Evidence::ProductName("Galileo".into()));
    for device in [LCD, OLED, Device::Desktop] {
        assert_eq!(Device::from_label(device.label()), Some(device));
    }
    assert_eq!(Device::from_label("deck"), None);

    assert_eq!(LCD.label(), "steam-deck-lcd");
    assert_eq!(OLED.label(), "steam-deck-oled");
    assert_eq!(Device::Desktop.label(), "desktop");
    assert_eq!(deck_model(" Jupiter "), Some(DeckModel::Lcd));
    assert_eq!(deck_model("jupiter"), None);
}

#[test]
fn aspects_parse_and_reduce() {
    assert_eq!(Aspect::parse("16:10"), Some(Aspect::DECK));
    assert_eq!(Aspect::parse(" 21 : 9 "), Aspect::new(21, 9));
    assert_eq!(Aspect::parse("16:0"), None);
    assert_eq!(Aspect::parse("wide"), None);
    assert_eq!(Aspect::of(size(1280, 800)), Aspect::new(8, 5));
    assert_eq!(Aspect::of(size(3840, 2160)), Some(Aspect::WIDE));
    assert_eq!(Aspect::of(size(0, 800)), None);
    assert_eq!(Aspect::WIDE.to_string(), "16:9");
}

fn policy(deck: &[&str], desktop: &[&str]) -> ScreenPolicy {
    ScreenPolicy {
        deck: deck
            .iter()
            .map(|text| Aspect::parse(text).unwrap())
            .collect(),
        desktop: desktop
            .iter()
            .map(|text| Aspect::parse(text).unwrap())
            .collect(),
        ..ScreenPolicy::default()
    }
}

const DECK_LCD: Size = Size {
    width: 1280,
    height: 800,
};
const SIXTEEN_TEN: Size = Size {
    width: 2560,
    height: 1600,
};
const FULL_HD: Size = Size {
    width: 1920,
    height: 1080,
};
const UHD: Size = Size {
    width: 3840,
    height: 2160,
};
const ULTRAWIDE: Size = Size {
    width: 2560,
    height: 1080,
};
const ULTRAWIDE_QHD: Size = Size {
    width: 3440,
    height: 1440,
};
const SUPERWIDE: Size = Size {
    width: 5120,
    height: 1440,
};

fn one_letterbox(report: &ScreenReport) {
    let shared = letterbox_with(report.layout, report.window, LetterboxRounding::Nearest).unwrap();
    assert_eq!(report.letterbox, shared);
    let view = report.viewport().unwrap();
    assert_eq!(view.letterbox, shared);
    assert_eq!(
        view,
        viewport_with(
            report.window,
            RenderScale::Native,
            report.layout,
            LetterboxRounding::Nearest
        )
        .unwrap()
    );
}

#[test]
fn a_sixteen_by_nine_game_fits_every_window() {
    let policy = policy(&["16:10"], &["16:9"]);
    let cases = [
        (DECK_LCD, Bars::Letterbox, [0.0, 40.0, 1280.0, 720.0]),
        (SIXTEEN_TEN, Bars::Letterbox, [0.0, 80.0, 2560.0, 1440.0]),
        (FULL_HD, Bars::None, [0.0, 0.0, 1920.0, 1080.0]),
        (UHD, Bars::None, [0.0, 0.0, 3840.0, 2160.0]),
        (ULTRAWIDE, Bars::Pillarbox, [320.0, 0.0, 1920.0, 1080.0]),
        (ULTRAWIDE_QHD, Bars::Pillarbox, [440.0, 0.0, 2560.0, 1440.0]),
        (SUPERWIDE, Bars::Pillarbox, [1280.0, 0.0, 2560.0, 1440.0]),
    ];
    for (window, bars, [x, y, width, height]) in cases {
        let report = policy.fit(Device::Desktop, window).unwrap();
        assert_eq!(report.bars, bars, "{window:?}");
        assert_eq!(report.native, bars == Bars::None, "{window:?}");
        assert_eq!(report.aspect, Aspect::WIDE, "{window:?}");
        assert_eq!(report.layout, size(1920, 1080), "{window:?}");
        assert!(
            rect_near(report.content, x, y, width, height),
            "{window:?} {:?}",
            report.content
        );
        assert_eq!(report.safe, report.content, "{window:?}");
        assert_eq!(report.ui_scale, 1.0);
        one_letterbox(&report);
    }
}

#[test]
fn a_deck_game_runs_native_on_the_deck_and_boxes_elsewhere() {
    let policy = policy(&["16:10"], &["16:9"]);
    let deck = policy.fit(LCD, DECK_LCD).unwrap();
    assert!(deck.native);
    assert_eq!(deck.bars, Bars::None);
    assert_eq!(deck.aspect, Aspect::DECK);
    assert_eq!(deck.layout, size(1728, 1080));
    assert!(rect_near(deck.content, 0.0, 0.0, 1280.0, 800.0));
    assert!(near(deck.pixels_per_unit(), 800.0 / 1080.0));
    assert_eq!(deck.ui_scale, DECK_UI_SCALE);
    assert!(rect_near(deck.safe, 24.0, 24.0, 1232.0, 752.0));
    assert!(rect_near(deck.safe_layout, 32.4, 32.4, 1663.2, 1015.2));
    one_letterbox(&deck);

    let cases = [
        (SIXTEEN_TEN, Bars::None, [0.0, 0.0, 2560.0, 1600.0]),
        (FULL_HD, Bars::Pillarbox, [96.0, 0.0, 1728.0, 1080.0]),
        (UHD, Bars::Pillarbox, [192.0, 0.0, 3456.0, 2160.0]),
        (ULTRAWIDE, Bars::Pillarbox, [416.0, 0.0, 1728.0, 1080.0]),
        (SUPERWIDE, Bars::Pillarbox, [1408.0, 0.0, 2304.0, 1440.0]),
    ];
    for (window, bars, [x, y, width, height]) in cases {
        let report = policy.fit(OLED, window).unwrap();
        assert_eq!(report.bars, bars, "{window:?}");
        assert_eq!(report.aspect, Aspect::DECK);
        assert!(
            rect_near(report.content, x, y, width, height),
            "{window:?} {:?}",
            report.content
        );
        one_letterbox(&report);
    }
}

#[test]
fn ultrawide_runs_native_when_the_game_declares_it() {
    let policy = policy(&["16:10"], &["16:9", "21:9"]);
    let wide = policy.fit(Device::Desktop, ULTRAWIDE).unwrap();
    assert!(wide.native);
    assert_eq!(wide.aspect, Aspect::new(21, 9).unwrap());
    assert_eq!(wide.layout, size(2560, 1080));
    assert!(rect_near(wide.content, 0.0, 0.0, 2560.0, 1080.0));
    let qhd = policy.fit(Device::Desktop, ULTRAWIDE_QHD).unwrap();
    assert!(qhd.native);
    assert_eq!(qhd.layout, size(2580, 1080));
    assert!(rect_near(qhd.content, 0.0, 0.0, 3440.0, 1440.0));
    let superwide = policy.fit(Device::Desktop, SUPERWIDE).unwrap();
    assert_eq!(superwide.bars, Bars::Pillarbox);
    assert_eq!(superwide.aspect, Aspect::new(21, 9).unwrap());
    assert_eq!(superwide.layout, size(2520, 1080));
    assert!(rect_near(superwide.content, 880.0, 0.0, 3360.0, 1440.0));
    let deck_on_desktop = policy.fit(Device::Desktop, DECK_LCD).unwrap();
    assert_eq!(deck_on_desktop.bars, Bars::Letterbox);
    assert_eq!(deck_on_desktop.aspect, Aspect::WIDE);
    for window in [DECK_LCD, FULL_HD, ULTRAWIDE, ULTRAWIDE_QHD, SUPERWIDE] {
        one_letterbox(&policy.fit(Device::Desktop, window).unwrap());
    }

    let strict = ScreenPolicy {
        tolerance: 0.01,
        ..policy.clone()
    };
    let boxed = strict.fit(Device::Desktop, ULTRAWIDE_QHD).unwrap();
    assert!(!boxed.native);
    assert_eq!(boxed.bars, Bars::Pillarbox);
    assert_eq!(boxed.layout, size(2520, 1080));
}

#[test]
fn an_empty_list_takes_any_window_natively() {
    let any = policy(&[], &[]);
    for window in [DECK_LCD, FULL_HD, ULTRAWIDE, SUPERWIDE, size(1000, 1000)] {
        let report = any.fit(Device::Desktop, window).unwrap();
        assert!(report.native);
        assert_eq!(report.aspect, Aspect::of(window).unwrap());
        assert!(rect_near(
            report.content,
            0.0,
            0.0,
            window.width as f32,
            window.height as f32
        ));
    }
    assert!(any.fit(Device::Desktop, size(0, 10)).is_none());
}

#[test]
fn the_pointer_maps_through_the_fit_and_bars_report_outside() {
    let policy = policy(&["16:10"], &["16:9"]);
    let wide = policy.fit(Device::Desktop, ULTRAWIDE).unwrap();
    let view = wide.viewport().unwrap();
    assert!(!view.pointer_inside(100.0, 500.0));
    assert!(wide.in_bars(100.0, 500.0));
    assert!(!view.pointer_inside(2300.0, 500.0));
    assert!(view.pointer_inside(320.0, 0.0));
    assert_eq!(view.pointer_to_layout(320.0, 0.0), [0.0, 0.0]);
    assert_eq!(view.pointer_to_layout(1280.0, 540.0), [960.0, 540.0]);
    let left = view.pointer_to_layout(100.0, 500.0);
    assert!(left[0] < 0.0);

    let letter = policy.fit(Device::Desktop, DECK_LCD).unwrap();
    let view = letter.viewport().unwrap();
    assert!(!view.pointer_inside(640.0, 20.0));
    assert!(!view.pointer_inside(640.0, 780.0));
    assert!(view.pointer_inside(640.0, 40.0));
    let [x, y] = view.pointer_to_layout(640.0, 400.0);
    assert!(near(x, 960.0) && near(y, 540.0));

    let deck = policy.fit(LCD, DECK_LCD).unwrap();
    let view = deck.viewport().unwrap();
    assert!(view.pointer_inside(0.0, 0.0));
    assert!(view.pointer_inside(1279.0, 799.0));
    let [x, y] = view.pointer_to_layout(1280.0, 800.0);
    assert!(near(x, 1728.0) && near(y, 1080.0));
}

#[test]
fn the_policy_reads_from_a_screen_table() {
    let text = r##"
title = "a game's own key"

[screen]
deck = ["16:10"]
desktop = ["16:9", "21:9"]
tolerance = 0.02
bars = "#10203040"
layout_height = 1200

[screen.safe]
deck = 0.05
desktop = { left = 0.01, right = 0.02, top = 0.0, bottom = 0.03 }

[screen.ui_scale]
deck = 1.5
"##;
    let policy = ScreenPolicy::from_toml(text).unwrap();
    assert_eq!(policy.deck, vec![Aspect::DECK]);
    assert_eq!(
        policy.desktop,
        vec![Aspect::WIDE, Aspect::new(21, 9).unwrap()]
    );
    assert_eq!(policy.tolerance, 0.02);
    assert_eq!(
        policy.bars,
        [16.0 / 255.0, 32.0 / 255.0, 48.0 / 255.0, 64.0 / 255.0]
    );
    assert_eq!(policy.layout_height, 1200);
    assert_eq!(policy.safe.deck, Inset::all(0.05));
    assert_eq!(
        policy.safe.desktop,
        Inset {
            left: 0.01,
            right: 0.02,
            top: 0.0,
            bottom: 0.03,
        }
    );
    assert_eq!(policy.ui_scale.deck, 1.5);
    assert_eq!(policy.ui_scale.desktop, 1.0);
    let deck = policy.fit(LCD, DECK_LCD).unwrap();
    assert_eq!(deck.layout, size(1920, 1200));

    assert_eq!(
        ScreenPolicy::from_toml("title = \"none\"").unwrap(),
        ScreenPolicy::default()
    );
    assert_eq!(
        ScreenPolicy::from_toml("[screen]").unwrap(),
        ScreenPolicy::default()
    );

    let refused = |body: &str| {
        ScreenPolicy::from_toml(&format!("[screen]\n{body}"))
            .unwrap_err()
            .key
    };
    assert_eq!(refused("deck = [\"16:0\"]"), "deck");
    assert_eq!(refused("desktop = [\"wide\"]"), "desktop");
    assert_eq!(refused("desktop = [\"1:9\"]"), "desktop");
    assert_eq!(refused("tolerance = 0.5"), "tolerance");
    assert_eq!(refused("bars = \"black\""), "bars");
    assert_eq!(refused("bars = \"Backdrop\""), "bars");
    assert_eq!(refused("layout_height = 10"), "layout_height");
    assert_eq!(refused("safe = { deck = 0.4 }"), "safe.deck");
    assert_eq!(refused("ui_scale = { desktop = 9.0 }"), "ui_scale.desktop");
    assert_eq!(refused("aspect = \"16:9\""), "screen");
    let error = ScreenPolicy::from_toml("[screen]\ntolerance = 0.5").unwrap_err();
    assert_eq!(
        error.to_string(),
        "[screen] tolerance: must be from 0 to 0.25"
    );
    assert_eq!(
        ScreenPolicy::from_toml("screen = 3").unwrap_err().key,
        "screen"
    );
}

fn desk() -> Monitors {
    let mode = |width, height, refresh, depth| VideoMode {
        size: size(width, height),
        bit_depth: depth,
        refresh_millihertz: refresh,
    };
    Monitors {
        list: vec![
            Monitor {
                name: Some("main".into()),
                position: Point { x: 0, y: 0 },
                size: UHD,
                refresh_millihertz: Some(143_998),
                video_modes: vec![
                    mode(3840, 2160, 60_000, 32),
                    mode(3840, 2160, 144_000, 24),
                    mode(3840, 2160, 144_000, 32),
                    mode(3840, 2160, 240_000, 32),
                    mode(1920, 1080, 144_000, 32),
                ],
            },
            Monitor {
                name: Some("side".into()),
                position: Point { x: 3840, y: 0 },
                size: FULL_HD,
                refresh_millihertz: None,
                video_modes: vec![mode(1920, 1080, 60_000, 32), mode(1920, 1080, 75_000, 32)],
            },
        ],
        primary: Some(0),
        current: Some(0),
    }
}

#[derive(Debug, PartialEq)]
enum Call {
    Fullscreen(FullscreenPlan),
    Decorations(bool),
    Resizable(bool),
    Size(Size),
    Position(Point),
}

#[derive(Default)]
struct StandIn {
    calls: Vec<Call>,
}

impl WindowOps for StandIn {
    fn set_fullscreen(&mut self, fullscreen: &FullscreenPlan) {
        self.calls.push(Call::Fullscreen(*fullscreen));
    }

    fn set_decorations(&mut self, decorations: bool) {
        self.calls.push(Call::Decorations(decorations));
    }

    fn request_inner_size(&mut self, size: Size) {
        self.calls.push(Call::Size(size));
    }

    fn set_outer_position(&mut self, position: Point) {
        self.calls.push(Call::Position(position));
    }

    fn set_resizable(&mut self, resizable: bool) {
        self.calls.push(Call::Resizable(resizable));
    }
}

#[test]
fn native_video_mode_takes_the_monitor_size_at_its_refresh() {
    let monitors = desk();
    let plan = plan_display_mode(
        &DisplayMode::ExclusiveFullscreen {
            monitor: MonitorChoice::Current,
            video_mode: VideoModeChoice::Native,
        },
        X11,
        &monitors,
    )
    .unwrap();
    assert_eq!(
        plan.fullscreen,
        FullscreenPlan::Exclusive {
            monitor: 0,
            mode: VideoMode {
                size: UHD,
                bit_depth: 32,
                refresh_millihertz: 144_000,
            },
        }
    );
    let side = plan_display_mode(
        &DisplayMode::ExclusiveFullscreen {
            monitor: MonitorChoice::Name("side".into()),
            video_mode: VideoModeChoice::Native,
        },
        X11,
        &monitors,
    )
    .unwrap();
    assert!(matches!(
        side.fullscreen,
        FullscreenPlan::Exclusive { monitor: 1, mode } if mode.refresh_millihertz == 75_000
    ));
}

#[test]
fn modes_switch_at_run_time_on_a_stand_in_window() {
    let monitors = desk();
    let mut display = Display::new(Device::Desktop, DisplaySettings::default());
    assert_eq!(display.mode(), WindowMode::Borderless);
    let mut window = StandIn::default();
    let plan = display.apply(WAYLAND, &monitors, &mut window).unwrap();
    assert_eq!(plan.fullscreen, FullscreenPlan::Borderless { monitor: 0 });
    assert_eq!(
        window.calls,
        vec![
            Call::Decorations(false),
            Call::Fullscreen(FullscreenPlan::Borderless { monitor: 0 }),
        ]
    );

    let mut window = StandIn::default();
    let (change, plan) = display
        .switch(WindowMode::Windowed, WAYLAND, &monitors, &mut window)
        .unwrap();
    assert_eq!(
        change,
        Change::Switched {
            from: WindowMode::Borderless,
            to: WindowMode::Windowed,
        }
    );
    assert!(plan.unwrap().resizable);
    assert_eq!(
        window.calls,
        vec![
            Call::Fullscreen(FullscreenPlan::Windowed),
            Call::Decorations(true),
            Call::Resizable(true),
            Call::Size(DEFAULT_WINDOWED),
        ]
    );
    display.resized(size(1280, 720));
    assert_eq!(display.settings().size, size(1280, 720));

    let mut window = StandIn::default();
    let (same, plan) = display
        .switch(WindowMode::Windowed, WAYLAND, &monitors, &mut window)
        .unwrap();
    assert_eq!(same, Change::Same);
    assert!(plan.is_none() && window.calls.is_empty());

    let mut window = StandIn::default();
    let (_, plan) = display
        .switch(WindowMode::Fullscreen, WAYLAND, &monitors, &mut window)
        .unwrap();
    let plan = plan.unwrap();
    assert_eq!(plan.fullscreen, FullscreenPlan::Borderless { monitor: 0 });
    assert_eq!(plan.fallback, Some(Fallback::ExclusiveUnsupported));
    display.resized(size(800, 600));
    assert_eq!(display.settings().size, size(1280, 720));

    display.moved_to(Some("side".into()));
    let mut window = StandIn::default();
    let plan = display.apply(X11, &monitors, &mut window).unwrap();
    assert!(matches!(
        plan.fullscreen,
        FullscreenPlan::Exclusive { monitor: 1, mode } if mode.size == FULL_HD
    ));

    let stored = toml::to_string(display.settings()).unwrap();
    let back: DisplaySettings = toml::from_str(&stored).unwrap();
    assert_eq!(&back, display.settings());
    assert_eq!(back.mode, WindowMode::Fullscreen);
    assert_eq!(back.monitor.as_deref(), Some("side"));
    let restarted = Display::new(Device::Desktop, back);
    assert_eq!(restarted.display_mode(), display.display_mode());
    let sparse: DisplaySettings = toml::from_str("mode = \"windowed\"").unwrap();
    assert_eq!(sparse.size, DEFAULT_WINDOWED);
    assert!(toml::from_str::<DisplaySettings>("mode = \"exclusive\"").is_err());

    let mut empty = Display::new(Device::Desktop, DisplaySettings::default());
    let none = Monitors {
        list: Vec::new(),
        primary: None,
        current: None,
    };
    assert!(
        empty
            .switch(WindowMode::Fullscreen, X11, &none, &mut StandIn::default())
            .is_err()
    );
    assert_eq!(empty.settings().mode, WindowMode::Borderless);
}

#[test]
fn the_deck_stays_fullscreen_and_says_so() {
    let monitors = Monitors {
        list: vec![Monitor {
            name: Some("deck".into()),
            position: Point { x: 0, y: 0 },
            size: DECK_LCD,
            refresh_millihertz: Some(60_000),
            video_modes: Vec::new(),
        }],
        primary: Some(0),
        current: Some(0),
    };
    let stored = DisplaySettings {
        mode: WindowMode::Windowed,
        size: size(800, 600),
        monitor: Some("elsewhere".into()),
        position: Some(Point { x: 10, y: 20 }),
    };
    let mut display = Display::new(LCD, stored.clone());
    assert_eq!(display.mode(), WindowMode::Fullscreen);
    assert_eq!(
        display.display_mode(),
        DisplayMode::BorderlessFullscreen {
            monitor: MonitorChoice::Current,
        }
    );
    let mut window = StandIn::default();
    let plan = display.apply(WAYLAND, &monitors, &mut window).unwrap();
    assert_eq!(plan.fullscreen, FullscreenPlan::Borderless { monitor: 0 });
    assert_eq!(plan.fallback, None);
    for mode in [
        WindowMode::Windowed,
        WindowMode::Borderless,
        WindowMode::Fullscreen,
    ] {
        let mut window = StandIn::default();
        let (change, plan) = display
            .switch(mode, WAYLAND, &monitors, &mut window)
            .unwrap();
        assert_eq!(change, Change::FixedOnDeck);
        assert!(plan.is_none() && window.calls.is_empty());
    }
    display.resized(size(640, 400));
    display.moved_to(None);
    display.placed(None, Point { x: 0, y: 0 });
    assert_eq!(display.settings(), &stored);
}

#[test]
fn the_windowed_position_comes_back_on_its_monitor_when_it_still_exists() {
    let monitors = desk();
    let mut display = Display::new(Device::Desktop, DisplaySettings::default());
    display.placed(Some("side".into()), Point { x: 100, y: 50 });
    assert_eq!(
        display.settings().position,
        None,
        "kept only while windowed"
    );
    display.set_mode(WindowMode::Windowed);
    display.resized(size(1280, 720));
    display.placed(Some("side".into()), Point { x: 100, y: 50 });
    assert_eq!(display.settings().monitor.as_deref(), Some("side"));
    assert_eq!(display.settings().position, Some(Point { x: 100, y: 50 }));

    let stored = toml::to_string(display.settings()).unwrap();
    assert!(stored.contains("[position]"), "{stored}");
    let restarted = Display::new(Device::Desktop, toml::from_str(&stored).unwrap());
    let mut window = StandIn::default();
    let plan = restarted.apply(X11, &monitors, &mut window).unwrap();
    let at = Point { x: 3940, y: 50 };
    assert_eq!(plan.outer_position, Some(at));
    assert_eq!(
        window.calls,
        vec![
            Call::Fullscreen(FullscreenPlan::Windowed),
            Call::Decorations(true),
            Call::Resizable(true),
            Call::Position(at),
            Call::Size(size(1280, 720)),
        ]
    );
    assert_eq!(
        restarted.display_mode_on(&monitors),
        DisplayMode::Windowed {
            size: size(1280, 720),
            position: Some(at),
        }
    );
    assert_eq!(
        restarted.display_mode(),
        DisplayMode::Windowed {
            size: size(1280, 720),
            position: None,
        }
    );

    let mut far = restarted.clone();
    far.placed(Some("side".into()), Point { x: 1800, y: -40 });
    assert_eq!(
        far.windowed_at(&monitors),
        Some(Point {
            x: 3840 + 640,
            y: 0
        }),
        "clamped so the window stays on its monitor"
    );

    let mut gone = monitors.clone();
    gone.list.truncate(1);
    let mut window = StandIn::default();
    let plan = restarted.apply(X11, &gone, &mut window).unwrap();
    assert_eq!(
        plan.outer_position, None,
        "a missing monitor leaves it to the desktop"
    );
    assert!(
        !window
            .calls
            .iter()
            .any(|call| matches!(call, Call::Position(_)))
    );

    let mut unnamed = Display::new(Device::Desktop, DisplaySettings::default());
    unnamed.set_mode(WindowMode::Windowed);
    unnamed.placed(None, Point { x: 7, y: 9 });
    assert_eq!(unnamed.windowed_at(&monitors), Some(Point { x: 7, y: 9 }));

    let old: DisplaySettings = toml::from_str("mode = \"windowed\"\nmonitor = \"side\"").unwrap();
    assert_eq!(old.position, None);
    assert_eq!(
        Display::new(Device::Desktop, old).windowed_at(&monitors),
        None
    );
}

#[test]
fn a_width_anchored_layout_keeps_its_width_and_one_letterbox() {
    let policy = ScreenPolicy::from_toml(
        "[screen]\ndeck = [\"16:10\"]\ndesktop = [\"16:9\", \"16:10\"]\nlayout_width = 1920",
    )
    .unwrap();
    assert_eq!(policy.layout_width, Some(1920));
    let cases = [
        (Device::Desktop, SIXTEEN_TEN, true, size(1920, 1200)),
        (Device::Desktop, FULL_HD, true, size(1920, 1080)),
        (Device::Desktop, UHD, true, size(1920, 1080)),
        (LCD, DECK_LCD, true, size(1920, 1200)),
        (LCD, FULL_HD, false, size(1920, 1200)),
        (Device::Desktop, ULTRAWIDE, false, size(1920, 1080)),
        (Device::Desktop, SUPERWIDE, false, size(1920, 1080)),
    ];
    for (device, window, native, layout) in cases {
        let report = policy.fit(device, window).unwrap();
        assert_eq!(report.native, native, "{window:?}");
        assert_eq!(report.layout, layout, "{window:?}");
        one_letterbox(&report);
    }
    let deck = policy.fit(LCD, DECK_LCD).unwrap();
    assert!(rect_near(deck.content, 0.0, 0.0, 1280.0, 800.0));
    assert!(near(deck.pixels_per_unit(), 1280.0 / 1920.0));
    let boxed = policy.fit(LCD, FULL_HD).unwrap();
    assert!(
        rect_near(boxed.content, 96.0, 0.0, 1728.0, 1080.0),
        "{:?}",
        boxed.content
    );
    assert_eq!(boxed.layout_units(), [1920.0, 1200.0]);
    let near_native = policy.fit(Device::Desktop, size(1920, 1090)).unwrap();
    assert!(near_native.native);
    assert_eq!(near_native.layout, size(1920, 1090));

    let refused = |body: &str| {
        ScreenPolicy::from_toml(&format!("[screen]\n{body}"))
            .unwrap_err()
            .key
    };
    assert_eq!(refused("layout_width = 100"), "layout_width");
    assert_eq!(refused("layout_width = 20000"), "layout_width");
    assert_eq!(
        refused("layout_width = 1920\nlayout_height = 1080"),
        "layout_width"
    );
    let height = ScreenPolicy::default();
    assert_eq!(height.layout_width, None);
    for window in [
        DECK_LCD,
        SIXTEEN_TEN,
        FULL_HD,
        UHD,
        ULTRAWIDE,
        ULTRAWIDE_QHD,
        SUPERWIDE,
    ] {
        let report = height.fit(Device::Desktop, window).unwrap();
        assert_eq!(report.layout, size(1920, 1080), "{window:?}");
    }
    assert_eq!(height.layout_size(2560.0 / 1080.0), Some(size(2560, 1080)));
    assert_eq!(height.layout_size(0.0), None);
}

fn noise(state: &mut u64) -> f64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    ((*state >> 33) as f64 / f64::from(1u32 << 31)) * 2.0 - 1.0
}

fn timings(frame: u64, gpu_ms: f64) -> FrameTimings {
    FrameTimings {
        frame,
        passes: vec![
            PassTiming {
                label: "opaque".into(),
                milliseconds: gpu_ms * 0.7,
            },
            PassTiming {
                label: "post".into(),
                milliseconds: gpu_ms * 0.3,
            },
        ],
    }
}

fn load_trace(budget: Budget, frames: u64) -> (Vec<f32>, Vec<u64>, DynamicResolution) {
    let mut resolution = DynamicResolution::new(budget).unwrap();
    let mut used = Vec::new();
    let mut seed = 7u64;
    let mut changed_at = Vec::new();
    for frame in 0..frames {
        used.push(resolution.begin(frame));
        if frame >= 2 {
            let measured = frame - 2;
            let scale = used[measured as usize];
            let gpu = load(measured, scale) * (1.0 + 0.03 * noise(&mut seed));
            if resolution
                .observe_timings(&timings(measured, gpu))
                .is_some()
            {
                changed_at.push(frame);
            }
        }
    }
    (used, changed_at, resolution)
}

fn load(frame: u64, scale: f32) -> f64 {
    let area = f64::from(scale) * f64::from(scale);
    if (300..450).contains(&frame) {
        3.0 + 18.0 * area
    } else {
        2.0 + 12.0 * area
    }
}

#[test]
fn dynamic_resolution_converges_under_the_budget_and_follows_the_load() {
    let budget = Budget::new(10.0);
    assert_eq!(DynamicResolution::new(budget).unwrap().scale(), 1.0);
    let (used, changed_at, resolution) = load_trace(budget, 700);
    for (frame, scale) in [
        (6, 0.8),
        (299, 0.8),
        (306, 0.6),
        (449, 0.6),
        (480, 0.75),
        (699, 0.75),
    ] {
        assert_eq!(used[frame], scale, "frame {frame}: {changed_at:?}");
    }
    assert!(
        changed_at
            .iter()
            .all(|frame| *frame < 6 || (300..306).contains(frame) || (450..480).contains(frame)),
        "{changed_at:?}"
    );
    assert!(resolution.changes() <= 6, "{changed_at:?}");
    let settled: Vec<f64> = (10..300)
        .chain(310..450)
        .chain(480..700)
        .map(|frame| load(frame, used[frame as usize]))
        .collect();
    assert!(settled.iter().all(|gpu| *gpu <= budget.gpu_ms));
    assert!(resolution.cost_ms_at_full().unwrap() > 12.0);
}

#[test]
fn dynamic_resolution_holds_a_scale_inside_its_band_under_noise() {
    let budget = Budget::new(10.0);
    let mut total = 0;
    for step in 0..40u32 {
        let base = 1.0 + f64::from(step) * 0.1;
        let slope = 8.0 + f64::from(step % 7) * 2.0;
        let mut resolution = DynamicResolution::new(budget).unwrap();
        let mut used = Vec::new();
        let mut seed = 11u64 + u64::from(step);
        let mut late = 0;
        for frame in 0..600u64 {
            used.push(resolution.begin(frame));
            if frame >= 2 {
                let scale = used[frame as usize - 2];
                let area = f64::from(scale) * f64::from(scale);
                let gpu = (base + slope * area) * (1.0 + 0.03 * noise(&mut seed));
                if resolution.observe(frame - 2, gpu).is_some() && frame >= 120 {
                    late += 1;
                }
            }
        }
        let held = used[120..]
            .iter()
            .all(|scale| base + slope * f64::from(*scale) * f64::from(*scale) <= budget.gpu_ms);
        assert!(held, "base {base} slope {slope}");
        total += late;
    }
    assert!(total <= 20, "{total} changes after settling");
}

#[test]
fn dynamic_resolution_ignores_unknown_frames_and_refuses_bad_budgets() {
    let mut resolution = DynamicResolution::new(Budget::new(8.0)).unwrap();
    assert_eq!(resolution.observe(3, 20.0), None);
    resolution.begin(0);
    assert_eq!(resolution.observe(0, f64::NAN), None);
    assert_eq!(resolution.observe(0, 0.0), None);
    let dropped = resolution.observe(0, 32.0).unwrap();
    assert_eq!(dropped, 0.5);
    assert_eq!(resolution.observe(0, 32.0), None);
    let mut raised = Vec::new();
    for frame in 1..=200 {
        resolution.begin(frame);
        if let Some(scale) = resolution.observe(frame, 0.5) {
            raised.push((frame, scale));
        }
    }
    assert_eq!(raised.last().map(|(_, scale)| *scale), Some(1.0));
    assert!(raised[0].0 >= u64::from(Budget::new(8.0).settle_frames));
    assert!(
        raised.windows(2).all(|pair| pair[1].0 - pair[0].0
            >= u64::from(Budget::new(8.0).settle_frames)
            && pair[1].1 > pair[0].1),
        "{raised:?}"
    );
    assert_eq!(resolution.changes(), 1 + raised.len() as u32);
    for bad in [
        Budget {
            gpu_ms: 0.0,
            ..Budget::new(1.0)
        },
        Budget {
            min_scale: 0.0,
            ..Budget::new(1.0)
        },
        Budget {
            min_scale: 0.9,
            max_scale: 0.8,
            ..Budget::new(1.0)
        },
        Budget {
            step: 0.0,
            ..Budget::new(1.0)
        },
        Budget {
            headroom: 1.5,
            ..Budget::new(1.0)
        },
    ] {
        assert!(DynamicResolution::new(bad).is_err());
    }
}

#[test]
fn the_render_size_scales_the_content_rect() {
    let policy = policy(&["16:10"], &["16:9"]);
    let wide = policy.fit(Device::Desktop, ULTRAWIDE_QHD).unwrap();
    assert_eq!(wide.render_size(1.0), Some(size(2560, 1440)));
    assert_eq!(wide.render_size(0.75), Some(size(1920, 1080)));
    let deck = policy.fit(LCD, DECK_LCD).unwrap();
    assert_eq!(deck.render_size(0.5), Some(size(640, 400)));
    assert_eq!(deck.render_size(0.0), None);
    assert!(near(deck.units_for_pixels(9.0), 12.15));
}

#[test]
fn bars_can_name_a_backdrop_with_the_colour_kept_as_its_fallback() {
    let policy = ScreenPolicy::from_toml("[screen]\nbars = \"backdrop\"").unwrap();
    assert!(policy.backdrop);
    assert_eq!(policy.bars, ScreenPolicy::default().bars);
    assert!(!ScreenPolicy::default().backdrop);
    assert!(
        !ScreenPolicy::from_toml("[screen]\nbars = \"#102030\"")
            .unwrap()
            .backdrop
    );
    let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
    let report = policy.fit(Device::Desktop, SUPERWIDE).unwrap();
    assert!(report.backdrop);
    assert_eq!(report.bars, Bars::Pillarbox);
    let [width, height] = report.window_units();
    assert!(
        close(width, 3840.0) && close(height, 1080.0),
        "{width} {height}"
    );
    let content = report.content_units();
    assert!(
        close(content.x, 960.0) && close(content.y, 0.0),
        "{content:?}"
    );
    assert_eq!([content.width, content.height], [1920.0, 1080.0]);
    let native = policy.fit(Device::Desktop, FULL_HD).unwrap();
    assert_eq!(native.window_units(), [1920.0, 1080.0]);
    assert_eq!(native.content_units().x, 0.0);
    let letterboxed = policy.fit(Device::Desktop, SIXTEEN_TEN).unwrap();
    let [width, height] = letterboxed.window_units();
    assert!(
        close(width, 1920.0) && close(height, 1200.0),
        "{width} {height}"
    );
    assert!(close(letterboxed.content_units().y, 60.0));
}
