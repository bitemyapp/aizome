//! The palette, headless: what it finds for a query, its keys, and its
//! pictures.

mod common;

use aizome_kit::sidestep_ui::{Modifiers, NamedKey};
use aizome_search::Scope;
use aizome_shell::actions::Action;
use aizome_shell::palette::PaletteEvent;
use aizome_shell::palette::view::PaletteMsg;
use aizome_shell::shell::Msg;
use common::*;

fn open(h: &mut aizome_kit::testing::Harness<aizome_shell::shell::Shell>, query: &str) -> aizome_kit::WindowKey {
    h.send(Msg::Run(Action::Palette { scope: None, query: String::new() }));
    let p = window_titled(h, "Palette");
    if !query.is_empty() {
        h.type_text(p, query);
    }
    p
}

#[test]
fn typing_finds_the_focused_apps_commands_first() {
    let mut h = scene(false);
    let p = open(&mut h, "bold");
    let s = h.program().palette.as_ref().expect("open");
    assert_eq!(s.query, "bold");
    assert_eq!(s.sections[0].scope, Scope::Command);
    assert_eq!(s.sections[0].rows[0].item.title, "Bold");
    assert_eq!(s.sections[0].rows[0].item.trail, "Format");
    assert!(h.texts(p).iter().any(|t| t == "Bold"));
}

#[test]
fn windows_and_arithmetic() {
    let mut h = scene(false);
    open(&mut h, "cargo");
    let s = h.program().palette.as_ref().unwrap();
    assert!(s.sections.iter().any(|s| s.scope == Scope::Window && s.rows[0].item.title == "cargo test"));
    h.send(Msg::Palette(PaletteEvent::Ui(PaletteMsg::Query("2^10/4".into()))));
    let s = h.program().palette.as_ref().unwrap();
    assert_eq!(s.sections[0].rows[0].item.title, "= 256");
}

#[test]
fn keys_move_scope_run_and_close() {
    let mut h = scene(false);
    let p = open(&mut h, "");
    h.named(p, NamedKey::Tab, Modifiers::NONE);
    assert_eq!(h.program().palette.as_ref().unwrap().scope, Some(Scope::Command));
    h.named(p, NamedKey::Tab, Modifiers::SHIFT);
    assert_eq!(h.program().palette.as_ref().unwrap().scope, None);
    h.type_text(p, "lock");
    h.named(p, NamedKey::ArrowDown, Modifiers::NONE);
    h.named(p, NamedKey::ArrowUp, Modifiers::NONE);
    assert_eq!(h.program().palette.as_ref().unwrap().selected, 0);
    h.named(p, NamedKey::Escape, Modifiers::NONE);
    assert!(h.program().palette.is_none());
    assert!(h.windows().iter().all(|k| h.title(*k) != "Palette"));
}

#[test]
fn running_a_command_asks_the_app() {
    let mut h = scene(false);
    let p = open(&mut h, "italic");
    h.named(p, NamedKey::Enter, Modifiers::NONE);
    assert!(h.program().palette.is_none(), "running closes it");
    // The pick is remembered for the coach and ranking.
    assert!(h.program().frecency.score("menu:org.aizome.Sumi:Format/Italic") > 0.9);
}

#[test]
fn pictures_of_the_palette() {
    for day in [false, true] {
        let mut h = scene(day);
        let p = open(&mut h, "f");
        {
            // The coach has something to say about Find… after picks.
            let s = h.program_mut();
            for _ in 0..14 {
                s.frecency.visit("menu:org.aizome.Sumi:Format/Bold");
            }
        }
        h.send(Msg::Palette(PaletteEvent::Ui(PaletteMsg::Query("bo".into()))));
        let name = if day { "palette-day.png" } else { "palette-night.png" };
        h.save_png(p, shots().join(name), 2.0);
        aizome_kit::testing::assert_golden(&mut h, p, golden(name), 2.0);
    }
}
