//! Pictures of each page of the sample system, written where
//! `YUKIMI_PICTURES` says (for looking at while working on the look).
use std::time::{Duration, Instant};

use aizome_kit::testing::Harness;
use aizome_kit::theme::{Appearance, ThemeChoice};
use aizome_yukimi::app::{Changes, Msg, Page, Yukimi};

/// Let background work answer: wait until `done` says so (or a while).
fn settle(h: &mut Harness<Yukimi>, done: impl Fn(&Yukimi) -> bool) {
    let start = Instant::now();
    while !done(h.program()) && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(5));
        h.send(Msg::NoteGone(0));
    }
}

#[test]
fn pictures() {
    let Some(dir) = std::env::var_os("YUKIMI_PICTURES") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (theme, name) in [(ThemeChoice::Night, "night"), (ThemeChoice::Day, "day")] {
        let appearance = Appearance { theme, reduced_motion: true, ..Appearance::default() };
        let mut h = Harness::with(Yukimi::sample(), appearance, false);
        let w = h.window();
        for page in Page::ALL {
            h.send(Msg::Show(page));
            if page == Page::History {
                h.send(Msg::ToggleGeneration(28));
                settle(&mut h, |a| matches!(a.changes.get(&28), Some(Changes::Ready { .. })));
            }
            if page == Page::Discover
                && let Ok(query) = std::env::var("YUKIMI_QUERY")
            {
                h.send(Msg::Search(query));
                settle(&mut h, |a| a.found.is_some());
            }
            h.save_png(w, dir.join(format!("{}-{name}.png", page.id())), 1.0);
        }
    }
}
