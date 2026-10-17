//! Hako's window, headless: browsing in columns, lists and icons, and
//! pictures of each.

mod common;

use aizome_kit::theme::ThemeChoice;
use common::*;
use hako::ui::Msg;
use hako::ui::browser::Loc;
use hako::ui::fileview::Layout;

#[test]
fn shows_home_in_columns() {
    let s = Sandbox::new("columns");
    populate(&s);
    let mut h = harness(&s, ThemeChoice::Night);
    let w = h.window();
    h.focus_window(w, true);
    let b = h.program().browser(w).unwrap();
    assert_eq!(b.columns.len(), 1);
    assert_eq!(b.columns[0].loc, Loc::Folder(s.home()));
    let names: Vec<String> = b.columns[0].rows.iter().map(|e| e.display.clone()).collect();
    assert_eq!(names, ["Desktop", "Documents", "Downloads", "Music", "Pictures", "Videos", "work"], "folders, sorted, hidden files hidden");
    h.send(Msg::GoPath(w, s.home().join("work/sidestep/README.md")));
    settle(&mut h);
    h.save_png(w, out("columns-night.png"), 2.0);
    h.send(Msg::SetLayout(w, Layout::List));
    h.send(Msg::GoPath(w, s.home().join("Downloads")));
    settle(&mut h);
    h.save_png(w, out("list-night.png"), 2.0);
    h.send(Msg::SetLayout(w, Layout::Icons));
    h.send(Msg::GoPath(w, s.home().join("Pictures")));
    settle(&mut h);
    h.save_png(w, out("icons-night.png"), 2.0);
}

#[test]
fn day() {
    let s = Sandbox::new("day");
    populate(&s);
    let mut h = harness(&s, ThemeChoice::Day);
    let w = h.window();
    h.focus_window(w, true);
    h.send(Msg::GoPath(w, s.home().join("Pictures/wallpaper-draft.png")));
    settle(&mut h);
    h.save_png(w, out("columns-day.png"), 2.0);
}
