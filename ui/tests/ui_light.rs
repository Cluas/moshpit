//! Which appearance each screen asks for (R18, UI-DESIGN §10.6): the
//! system's on the screens that follow it (`Screen::follows_system`), dark
//! on every other; what a device test forces (`Params::light`) over both;
//! the terminal's by its colour scheme's luminance.

use std::sync::{Arc, Mutex};

use moshpit_ui::{
    channel::Channel,
    screen::{AppearanceChoice, Params, Screen, terminal_appearance},
};
use nsh_core::{MountItem, color::Rgba, platform::Appearance};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

/// The appearance `screen` asks the platform for with `light` as the
/// params have it: the one it sets, or the system's when it sets none (the
/// default a root starts on).
fn asked(screen: Screen, light: Option<bool>) -> Appearance {
    let params = Params {
        light,
        ..Params::default()
    };
    let channel = Channel::open();
    let mut config = EngineConfig::new(screen.root(&channel, &params));
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    let mut app = Headless::start(config).expect("the engine starts");
    let seen = Arc::new(Mutex::new(None));
    let observed = seen.clone();
    app.observe(move |_, batch| {
        for item in &batch.items {
            if let MountItem::Appearance { appearance } = item {
                *observed.lock().unwrap() = Some(*appearance);
            }
        }
    });
    let _ = app.settle();
    let asked = seen.lock().unwrap().unwrap_or(Appearance::System);
    channel.close();
    asked
}

/// The screens that follow the system ask for nothing of their own; every
/// other asks for dark. The terminal is listed (its chrome is clean, R12)
/// but asks for its colour scheme's appearance, never the system's.
#[test]
fn a_screen_follows_the_system_only_once_listed() {
    for screen in Screen::ALL {
        if screen == Screen::Terminal {
            continue;
        }
        let want = if screen.follows_system() {
            Appearance::System
        } else {
            Appearance::Dark
        };
        assert_eq!(
            screen.appearance(&Params::default()),
            want,
            "{}",
            screen.name()
        );
    }
    for screen in [
        Screen::Home,
        Screen::UiGallery,
        Screen::ConnectionForm,
        Screen::HostSetup,
        Screen::Settings,
    ] {
        assert!(screen.follows_system(), "{}", screen.name());
        assert_eq!(asked(screen, None), Appearance::System, "{}", screen.name());
    }
    // Settings was the last screen dark only: none is now.
    assert!(
        Screen::ALL.into_iter().all(Screen::follows_system),
        "every screen follows the system"
    );
    assert!(
        Screen::Terminal.follows_system(),
        "the terminal's chrome is clean"
    );
    for (scheme, background, want) in [
        ("none told", None, Appearance::Dark),
        ("GitHub Dark", Some("#0D1117"), Appearance::Dark),
        ("Solarized Light", Some("#FDF6E3"), Appearance::Light),
    ] {
        let params = Params {
            terminal: background.map(str::to_owned),
            ..Params::default()
        };
        assert_eq!(Screen::Terminal.appearance(&params), want, "{scheme}");
    }
}

/// The appearance chosen in Settings (`Params::appearance`, the host's
/// stored `moshpit.settings.appearance`): the system's, or light, or dark,
/// on a screen that follows the system; the terminal takes its colour
/// scheme's whatever is chosen (dark when none is told).
#[test]
fn a_screen_that_follows_the_system_takes_the_appearance_chosen() {
    let chosen = |choice: Option<AppearanceChoice>| Params {
        appearance: choice,
        ..Params::default()
    };
    for (choice, want) in [
        (None, Appearance::System),
        (Some(AppearanceChoice::System), Appearance::System),
        (Some(AppearanceChoice::Light), Appearance::Light),
        (Some(AppearanceChoice::Dark), Appearance::Dark),
    ] {
        for screen in [
            Screen::Home,
            Screen::Shortcuts,
            Screen::AppIcon,
            Screen::HostSetup,
            Screen::Settings,
        ] {
            assert_eq!(
                screen.appearance(&chosen(choice)),
                want,
                "{} {choice:?}",
                screen.name()
            );
        }
        assert_eq!(
            Screen::Terminal.appearance(&chosen(choice)),
            Appearance::Dark,
            "terminal {choice:?}"
        );
    }
    // What the root asks the platform for, end to end.
    let params = Params {
        appearance: Some(AppearanceChoice::Light),
        ..Params::default()
    };
    let channel = Channel::open();
    let mut config = EngineConfig::new(Screen::Home.root(&channel, &params));
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    let mut app = Headless::start(config).expect("the engine starts");
    let seen = Arc::new(Mutex::new(None));
    let observed = seen.clone();
    app.observe(move |_, batch| {
        for item in &batch.items {
            if let MountItem::Appearance { appearance } = item {
                *observed.lock().unwrap() = Some(*appearance);
            }
        }
    });
    let _ = app.settle();
    assert_eq!(*seen.lock().unwrap(), Some(Appearance::Light));
    channel.close();
    // A device test's forced appearance wins over the choice.
    let forced = Params {
        light: Some(false),
        appearance: Some(AppearanceChoice::Light),
        ..Params::default()
    };
    assert_eq!(Screen::Home.appearance(&forced), Appearance::Dark);
    // The host's JSON.
    let parse = |json: &str| Params::parse(Some(json)).unwrap().appearance;
    assert_eq!(
        parse(r#"{"appearance": "system"}"#),
        Some(AppearanceChoice::System)
    );
    assert_eq!(
        parse(r#"{"appearance": "light"}"#),
        Some(AppearanceChoice::Light)
    );
    assert_eq!(
        parse(r#"{"appearance": "dark"}"#),
        Some(AppearanceChoice::Dark)
    );
    assert_eq!(parse("{}"), None);
    assert!(Params::parse(Some(r#"{"appearance": "sepia"}"#)).is_err());
}

/// A device test's pictures: light where the screen draws it, dark when it
/// asks for dark, whatever the screen's own rule.
#[test]
fn a_device_test_forces_either_appearance() {
    for screen in [
        Screen::UiGallery,
        Screen::Home,
        Screen::Theme,
        Screen::Feedback,
        Screen::Settings,
    ] {
        assert!(screen.draws_light(), "{}", screen.name());
        assert_eq!(
            asked(screen, Some(true)),
            Appearance::Light,
            "{}",
            screen.name()
        );
        assert_eq!(
            asked(screen, Some(false)),
            Appearance::Dark,
            "{}",
            screen.name()
        );
    }
    // The terminal's pictures are its colour scheme's.
    assert!(!Screen::Terminal.draws_light());
    assert_eq!(asked(Screen::Terminal, Some(true)), Appearance::Dark);
    // The host's JSON: `"light": false` is a forced dark, no key the
    // screen's own.
    let params = |json: &str| Params::parse(Some(json)).unwrap();
    assert_eq!(params(r#"{"light": true}"#).light, Some(true));
    assert_eq!(params(r#"{"light": false}"#).light, Some(false));
    assert_eq!(params("{}").light, None);
}

/// Every screen that follows the system can be drawn light by a device test
/// (its pictures in both appearances), but the terminal, whose appearance
/// is its colour scheme's.
#[test]
fn a_screen_that_follows_the_system_has_pictures_in_both() {
    for screen in Screen::ALL {
        if screen.follows_system() && screen != Screen::Terminal {
            assert!(screen.draws_light(), "{} follows the system", screen.name());
        }
    }
}

/// The terminal's chrome takes its colour scheme's appearance (UI-DESIGN
/// §2.2, §6.3): light past a relative luminance of 0.179, where black and
/// white words contrast with the background alike, dark under it. The one
/// rule, the same as `Params::terminal_is_light`.
#[test]
fn a_terminal_is_as_light_as_its_scheme() {
    let rgb = |hex: u32| nsh_core::color::rgb(hex);
    assert_eq!(moshpit_ui::screen::LIGHT_BACKGROUND, 0.179);
    for (scheme, background, want) in [
        ("GitHub Dark", rgb(0x0D1117), Appearance::Dark),
        ("Solarized Dark", rgb(0x002B36), Appearance::Dark),
        ("Moshpit's near black", rgb(0x030405), Appearance::Dark),
        ("a grey just under (0.178)", rgb(0x757575), Appearance::Dark),
        ("a grey just over (0.181)", rgb(0x767676), Appearance::Light),
        ("a light grey (0.39)", rgb(0xA8A8A8), Appearance::Light),
        ("Solarized Light", rgb(0xFDF6E3), Appearance::Light),
        ("GitHub Light", rgb(0xFFFFFF), Appearance::Light),
    ] {
        assert_eq!(terminal_appearance(background), want, "{scheme}");
    }
    // Its alpha is not its lightness.
    let faint_white = Rgba {
        a: 0.1,
        ..rgb(0xFFFFFF)
    };
    assert_eq!(terminal_appearance(faint_white), Appearance::Light);
}
