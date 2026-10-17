//! Menus, headless: the Toolchest and the menu at the pointer as kit
//! popups of the bar, torn-off palettes, and their pictures.

mod common;

use std::time::Duration;

use aizome_ipc::Event;
use aizome_kit::prelude::*;
use aizome_shell::ipc::IpcEvent;
use aizome_shell::menubar::{BarMsg, Slot};
use aizome_shell::shell::Msg;
use aizome_shell::tearoff::TearMsg;
use common::*;

fn pointer_menu(h: &mut aizome_kit::testing::Harness<aizome_shell::shell::Shell>, x: f64, y: f64) {
    h.send(Msg::Ipc(IpcEvent::Event(Event::Binding { action: "app-menu".into(), pointer: Some((x, y)), output: Some("eDP-1".into()) })));
    // Services wait for the selected text, at most 150 ms.
    h.advance(Duration::from_millis(200));
}

#[test]
fn the_menu_at_the_pointer_gathers_the_apps_menus() {
    let mut h = scene(false);
    let bar = window_titled(&h, "Menu bar");
    pointer_menu(&mut h, 600.0, 300.0);
    let popups = h.popups(bar);
    assert_eq!(popups.len(), 1, "a menu opened");
    let (_menu, at) = popups[0];
    assert_eq!(at.origin(), Point::new(600.0, 300.0));
    // (The kit's menus draw their rows themselves: check what was shown.)
    let (name, menus) = h.program().focused_menus();
    let p = aizome_shell::menus::build::pointer_menu(&name, &menus, vec![], aizome_shell::actions::Action::None, aizome_shell::actions::Action::None);
    let labels: Vec<&str> = p.items.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(labels, ["Info", "File", "Edit", "Search", "Format", "Window", "Help", "Services", "Hide", "Quit"]);
}

#[test]
fn the_toolchest_opens_under_its_button() {
    let mut h = scene(false);
    let bar = window_titled(&h, "Menu bar");
    h.send(Msg::Bar(bar, BarMsg::Press(Slot::Toolchest, Rect::new(0.0, 0.0, 92.0, 24.0))));
    let (_menu, at) = h.popups(bar)[0];
    assert_eq!(at.origin(), Point::new(0.0, 23.0));
}

#[test]
fn torn_off_menus_stay_with_their_app() {
    let mut h = scene(false);
    h.send(Msg::TearOff { app: "org.aizome.Sumi".into(), path: vec!["Format".into()], output: "eDP-1".into(), at: Point::new(104.0, 412.0) });
    let _p = window_titled(&h, "Palette (torn off)");
    let (title, menu) = h.program().tear_menu(&h.program().tearoffs[0]).expect("its menu");
    assert_eq!(title, "Format");
    assert!(menu.items.iter().any(|e| e.label == "Bold"));
    // Another app focused: it hides; pinned, it stays.
    h.send(Msg::Ipc(IpcEvent::Event(Event::WindowFocused { id: Some(3) })));
    assert!(h.windows().iter().all(|k| h.title(*k) != "Palette (torn off)"));
    h.send(Msg::Tear(TearMsg::Pin(0, true)));
    assert!(h.windows().iter().any(|k| h.title(*k) == "Palette (torn off)"));
    h.send(Msg::Tear(TearMsg::Close(0)));
    assert!(h.program().tearoffs.is_empty());
}

#[test]
fn pictures_of_menus() {
    for day in [false, true] {
        let mut h = scene(day);
        let bar = window_titled(&h, "Menu bar");
        let suffix = if day { "day" } else { "night" };
        // The menu at the pointer, its Edit submenu open.
        pointer_menu(&mut h, 600.0, 160.0);
        let (menu, _) = h.popups(bar)[0];
        let img = h.render_popup(menu, 2.0);
        let (w, hh) = img.pixel_size();
        aizome_kit::testing::write_png(&shots().join(format!("menu-pointer-{suffix}.png")), w, hh, &img.to_rgba8().unwrap()).unwrap();
        h.send(Msg::Run(aizome_shell::actions::Action::None));
        // The Toolchest.
        h.send(Msg::Bar(bar, BarMsg::Press(Slot::Toolchest, Rect::new(0.0, 0.0, 92.0, 24.0))));
        let (menu, _) = *h.popups(bar).last().unwrap();
        let img = h.render_popup(menu, 2.0);
        let (w, hh) = img.pixel_size();
        aizome_kit::testing::write_png(&shots().join(format!("menu-toolchest-{suffix}.png")), w, hh, &img.to_rgba8().unwrap()).unwrap();
        // The bar's Edit menu.
        h.send(Msg::Bar(bar, BarMsg::Press(Slot::Menu(2), Rect::new(190.0, 0.0, 230.0, 24.0))));
        let (menu, _) = *h.popups(bar).last().unwrap();
        let img = h.render_popup(menu, 2.0);
        let (w, hh) = img.pixel_size();
        aizome_kit::testing::write_png(&shots().join(format!("menu-edit-{suffix}.png")), w, hh, &img.to_rgba8().unwrap()).unwrap();
        // A torn-off Format palette.
        h.send(Msg::TearOff { app: "org.aizome.Sumi".into(), path: vec!["Format".into()], output: "eDP-1".into(), at: Point::new(104.0, 412.0) });
        let p = window_titled(&h, "Palette (torn off)");
        let name = format!("palette-format-{suffix}.png");
        h.save_png(p, shots().join(&name), 2.0);
        aizome_kit::testing::assert_golden(&mut h, p, golden(&name), 2.0);
    }
}
