//! The session's dialogs and the notifications' cards, headless.

mod common;

use std::time::Duration;

use aizome_kit::sidestep_ui::{Modifiers, NamedKey};
use aizome_shell::actions::{Action, SessionVerb};
use aizome_shell::confirm::ConfirmMsg;
use aizome_shell::notifications::cards::CardMsg;
use aizome_shell::notifications::server::NoteEvent;
use aizome_shell::notifications::store::{Incoming, Urgency};
use aizome_shell::shell::Msg;
use common::*;

#[test]
fn shutting_down_counts_down_a_minute() {
    let mut h = scene(false);
    h.send(Msg::Run(Action::Session(SessionVerb::ShutDown)));
    let c = window_titled(&h, "Confirm");
    assert!(h.texts(c).iter().any(|t| t.contains("in 60 seconds")));
    h.advance(Duration::from_secs(30));
    assert!(h.texts(c).iter().any(|t| t.contains("in 30 seconds")), "{:?}", h.texts(c));
    assert_eq!(h.program().session_done, None);
    h.advance(Duration::from_secs(31));
    assert_eq!(h.program().session_done, Some(SessionVerb::ShutDown));
    assert!(h.windows().iter().all(|k| h.title(*k) != "Confirm"));
}

#[test]
fn cancelling_asks_nothing_more() {
    let mut h = scene(false);
    h.send(Msg::Run(Action::Session(SessionVerb::LogOut)));
    h.send(Msg::Confirm(ConfirmMsg::Cancel));
    h.advance(Duration::from_secs(90));
    assert_eq!(h.program().session_done, None);
    h.send(Msg::Run(Action::Session(SessionVerb::Restart)));
    let c = window_titled(&h, "Confirm");
    h.named(c, NamedKey::Enter, Modifiers::NONE);
    assert_eq!(h.program().session_done, Some(SessionVerb::Restart), "Return does it now");
}

#[test]
fn desks_rename_in_place() {
    let mut h = scene(false);
    h.send(Msg::Run(Action::RenameDesk { index: 2 }));
    let bar = window_titled(&h, "Menu bar");
    h.named(bar, NamedKey::Backspace, Modifiers::SUPER);
    h.type_text(bar, "Ship");
    assert_eq!(h.program().renaming.as_ref().map(|r| r.1.as_str()), Some("Ship"));
    h.named(bar, NamedKey::Enter, Modifiers::NONE);
    assert!(h.program().renaming.is_none());
}

#[test]
fn critical_notifications_stay_as_cards() {
    let mut h = scene(false);
    let n = Incoming {
        app_name: "Power".into(),
        summary: "Battery low".into(),
        body: "Plug in soon: 6% left".into(),
        urgency: Urgency::Critical,
        actions: vec![("settings".into(), "Battery Settings".into())],
        ..Default::default()
    };
    h.send(Msg::Note(NoteEvent::Notify(40, Box::new(n))));
    let c = window_titled(&h, "Notifications");
    assert!(h.texts(c).iter().any(|t| t.contains("Battery low")));
    h.advance(Duration::from_secs(10));
    assert!(h.windows().iter().any(|k| h.title(*k) == "Notifications"), "it stays");
    h.save_png(c, shots().join("cards-night.png"), 2.0);
    h.send(Msg::Card(CardMsg::Dismiss(40)));
    assert!(h.windows().iter().all(|k| h.title(*k) != "Notifications"));
}

#[test]
fn screenshots_say_where_they_went() {
    let mut h = scene(false);
    h.send(Msg::Ipc(aizome_shell::ipc::IpcEvent::Event(aizome_ipc::Event::ScreenshotTaken { path: "/home/u/Pictures/Screenshots/a.png".into() })));
    let id = h.program().banner.expect("shown in the bar");
    let n = h.program().notes.get(id).unwrap().clone();
    assert_eq!(n.summary, "Screenshot saved");
    assert!(n.buttons().any(|(_, l)| l == "Show in Hako"));
}
