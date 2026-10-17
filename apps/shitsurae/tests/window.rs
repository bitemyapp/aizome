//! The window: the sidebar, navigation, search, `--pane`.

mod common;

use aizome_kit::theme::{Appearance, ThemeChoice};

#[test]
fn opens_on_a_pane_and_shows_it() {
    let (mut h, _) = common::open("opens", "mouse");
    let w = h.window();
    assert_eq!(h.program().pane_id(), "mouse");
    assert_eq!(h.title(w), "Mouse");
    let texts = h.texts(w);
    assert!(texts.iter().any(|t| t == "Pointer speed"), "{texts:?}");
    h.save_png(w, common::shots_dir().join("mouse-night.png"), 1.0);
    h.set_appearance(Appearance { theme: ThemeChoice::Day, ..Appearance::default() });
    h.save_png(w, common::shots_dir().join("mouse-day.png"), 1.0);
}

#[test]
fn appearance_shows_previews() {
    let (mut h, _) = common::open("appearance", "appearance");
    let w = h.window();
    h.save_png(w, common::shots_dir().join("appearance-night.png"), 1.0);
}

/// The panes this test holds to their entries (the hardware panes have
/// tests of their own).
const MINE: [&str; 15] = [
    "general", "appearance", "accessibility", "desktop", "menubar", "battery", "performance", "search", "notifications", "lock", "users",
    "keyboard", "mouse", "trackpad", "general",
];

#[test]
fn every_option_listed_is_on_its_pane() {
    let panes = shitsurae::panes::all();
    let items = shitsurae::listing::items(&panes);
    let mut ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    ids.sort();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "ids are unique");
    let mut missing = Vec::new();
    for item in items.iter().filter(|i| i.kind == "option" && MINE.contains(&i.pane.as_str())) {
        let (mut h, _) = common::open("entries", &item.id);
        let w = h.window();
        let name = shitsurae::pane::entry_name(&item.pane, item.entry.as_deref().unwrap_or(""));
        if h.find_id(w, &name).is_none() {
            missing.push(item.id.clone());
        }
    }
    assert!(missing.is_empty(), "options not on their panes: {missing:?}");
}

#[test]
fn search_finds_options_and_opens_them() {
    let (mut h, _) = common::open("search", "general");
    let w = h.window();
    h.send(shitsurae::AppMsg::Search("caps lock".into()));
    let hit = h.program().hits.iter().find(|hit| h.program().panes[hit.pane].id() == "keyboard").expect("Keyboard found");
    assert_eq!(hit.entries[0].id, "caps-lock");
    assert_eq!(hit.entries[0].ranges, vec![0..4, 5..9], "both words highlighted");
    // The pane showing highlights its options that match: none here.
    h.send(shitsurae::AppMsg::Search("night".into()));
    assert!(h.program().hits.iter().any(|hit| h.program().panes[hit.pane].id() == "appearance"));
    h.send(shitsurae::AppMsg::Search("caps lock".into()));
    h.save_png(w, common::shots_dir().join("search-caps.png"), 1.0);
    h.send(shitsurae::AppMsg::SearchSubmit);
    assert_eq!(h.program().pane_id(), "keyboard");
    assert_eq!(h.program().revealed.as_ref().map(|(p, e)| (*p, e.as_str())), Some(("keyboard", "caps-lock")));
    assert_eq!(h.focused_name(w).as_deref(), Some("entry:keyboard.caps-lock"));
    h.save_png(w, common::shots_dir().join("search-revealed.png"), 1.0);
}

#[test]
fn aliases_and_sub_pages_open() {
    let (h, _) = common::open("alias", "datetime");
    assert_eq!(h.program().pane_id(), "general");
    let texts = h.texts(h.window());
    assert!(texts.iter().any(|t| t == "Set the time automatically"), "{texts:?}");
    let (h, _) = common::open("alias2", "keyboard.shortcut-lock");
    assert!(h.texts(h.window()).iter().any(|t| t == "Lock the screen"));
}
