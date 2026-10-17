//! Golden images of Shitsurae's main views, Night and Day, through the
//! offscreen renderer (`AIZOME_UPDATE_GOLDENS=1` writes them anew). They
//! were looked at when written.

mod common;

use std::path::PathBuf;

use aizome_kit::testing::{Harness, assert_golden};
use shitsurae::{AppMsg, Shitsurae, Start};

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/goldens").join(format!("{name}.png"))
}

fn open(target: &str, day: bool) -> Harness<Shitsurae> {
    let dir = common::temp_dir(target);
    let path = dir.join("settings.toml");
    if day {
        std::fs::write(&path, "[appearance]\ntheme = \"day\"\n").unwrap();
    }
    let start = if target.is_empty() { Start::default() } else { Start::parse(target) };
    Harness::new(Shitsurae::sample(path, start))
}

fn check(target: &str, name: &str) {
    for day in [false, true] {
        let mut h = open(target, day);
        let w = h.window();
        let suffix = if day { "day" } else { "night" };
        assert_golden(&mut h, w, golden(&format!("{name}-{suffix}")), 1.0);
    }
}

#[test]
fn general() {
    check("general", "general");
}

#[test]
fn about() {
    check("general.about", "about");
}

#[test]
fn appearance() {
    check("appearance", "appearance");
}

#[test]
fn keyboard_shortcuts() {
    check("keyboard.shortcuts", "keyboard-shortcuts");
}

#[test]
fn users() {
    check("users", "users");
}

#[test]
fn battery() {
    check("battery", "battery");
}

#[test]
fn search_results() {
    for day in [false, true] {
        let mut h = open("keyboard", day);
        let w = h.window();
        h.send(AppMsg::Search("lock".into()));
        let suffix = if day { "day" } else { "night" };
        assert_golden(&mut h, w, golden(&format!("search-{suffix}")), 1.0);
    }
}
