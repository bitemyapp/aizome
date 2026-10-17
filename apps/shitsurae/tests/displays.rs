//! The Displays pane over the sample compositor: the arrangement, dragging
//! a display, and the settings of the selected one.

mod common;

use aizome_kit::layout::Point;
use aizome_kit::theme::{Appearance, ThemeChoice};

#[test]
fn shows_the_arrangement_and_the_selected_display() {
    let (mut h, _) = common::open("displays-main", "displays");
    let w = h.window();
    h.resize(w, aizome_kit::layout::Size::new(940.0, 1180.0));
    settle(&mut h, |h| has(h, "Use as"));
    let texts = h.texts(w);
    for want in ["Use as", "RESOLUTION", "Refresh rate", "Rotation", "Larger Text", "More Space"] {
        assert!(texts.iter().any(|t| t == want), "{want} missing from {texts:?}");
    }
    h.save_png(w, common::shots_dir().join("displays-night.png"), 2.0);
    h.set_appearance(Appearance { theme: ThemeChoice::Day, ..Appearance::default() });
    h.save_png(w, common::shots_dir().join("displays-day.png"), 2.0);
}

#[test]
fn dragging_a_display_puts_it_beside_the_other() {
    let (mut h, _) = common::open("displays-drag", "displays");
    let w = h.window();
    settle(&mut h, |h| has(h, "Use as"));
    let area = h.find_id(w, "entry:displays.arrangement").expect("the arrangement");
    let value = |h: &aizome_kit::testing::Harness<shitsurae::Shitsurae>| {
        h.semantics(w).into_iter().find(|(s, _)| s.label.as_deref() == Some("Display arrangement")).and_then(|(s, _)| s.value).unwrap_or_default()
    };
    let before = value(&h);
    assert!(before.contains("Built-in Display at 0, 180"), "{before}");
    // The laptop sits left of the 4K display: drag it to the far right.
    let from = Point::new(area.x0 + area.width() * 0.33, area.y0 + area.height() * 0.6);
    let to = Point::new(area.x1 - 20.0, area.y0 + area.height() * 0.5);
    h.drag(w, from, to, 8);
    settle(&mut h, |h| value(h).contains("Built-in Display at 1920"));
    let after = value(&h);
    assert!(after.contains("Built-in Display at 1920"), "{after}");
    h.save_png(w, common::shots_dir().join("displays-dragged.png"), 1.0);
}

/// Let background work (sample tasks, the sample compositor) come back:
/// flush the window until `until` holds, up to two seconds.
fn settle(h: &mut aizome_kit::testing::Harness<shitsurae::Shitsurae>, until: impl Fn(&aizome_kit::testing::Harness<shitsurae::Shitsurae>) -> bool) {
    for _ in 0..400 {
        if until(h) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.send(shitsurae::AppMsg::Nothing);
    }
}

fn has(h: &aizome_kit::testing::Harness<shitsurae::Shitsurae>, s: &str) -> bool {
    let w = h.window();
    h.texts(w).iter().any(|t| t == s)
}
