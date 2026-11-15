//! Settings › Appearance › Companion, the pet's card (PET-DESIGN §17,
//! B607): a tap on the big sprite has it answer with a line in a bubble,
//! as on Home — the same lines, the same rules — and screen readers hear
//! it.

mod common;

use common::{post, settings_fixture::state, start_observing};
use moshpit_ui::{
    screen::{Params, Screen},
    settings::Message,
};
use serde_json::json;

/// The pet's card, with the settings shown and the companion `talk`ing or
/// not.
fn card(talk: bool) -> (common::App, std::sync::Arc<moshpit_ui::channel::Channel>) {
    let (mut app, channel, _) = start_observing(
        Screen::Settings,
        Params {
            page: Some("companion".into()),
            ..Params::default()
        },
        "settings",
    );
    post(
        &mut app,
        &channel,
        &Message::Settings(Box::new(state())).to_json(),
    );
    post(&mut app, &channel, &pet(talk));
    post(&mut app, &channel, &Message::Clock(14).to_json());
    app.wait_for("id:settings-pet-sprite").unwrap();
    (app, channel)
}

/// A hatched pixel pet, told to talk or not (Home's fixture).
fn pet(talk: bool) -> String {
    json!({
        "op": "pet", "style": "pixel", "shown": true, "may_hatch": false, "talk": talk, "seq": 0,
        "pet": {
            "seed": "07070707070707070707070707070707", "stage": 1, "hatch_hour": 14,
            "flags": 0, "crowned": false, "name": "Tun", "species": "tun",
            "hatched_at": 1_800_000_000, "tasks": 12,
            "traits": {"fur": "natural", "shiny": false, "pattern": "plain", "eyes": "dot", "size": "regular"},
            "history": [], "found": []
        }
    })
    .to_string()
}

fn says(app: &mut common::App) -> bool {
    app.screen()
        .layout_nodes()
        .into_iter()
        .any(|n| n.test_id.as_deref() == Some("home-pet-says"))
}

/// A tap on the sprite answers in a bubble, as a tap on Home does (a tap
/// always answers, §17.1); with 「让它说话」 off, it only pets it.
#[test]
fn a_tap_on_the_card_s_pet_answers_in_a_bubble_unless_it_is_told_not_to_talk() {
    for talk in [true, false] {
        let (mut app, channel) = card(talk);
        assert!(!says(&mut app), "nothing said before the tap");
        app.press("id:settings-pet-sprite").unwrap();
        assert_eq!(
            says(&mut app),
            talk,
            "talk {talk}: {:?}",
            app.screen().texts()
        );
        channel.close();
    }
}

/// The local hour reaches the card as Home's does (`{"op": "clock"}`), so
/// the small hours' lines can be said here too.
#[test]
fn the_clock_message_round_trips() {
    let message = Message::Clock(3);
    assert_eq!(Message::parse(&message.to_json()), Ok(message));
    assert!(Message::parse(r#"{"op":"clock","hour":3,"minute":1}"#).is_err());
}
