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
