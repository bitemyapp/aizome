//! Keys: the command key (❖) translated to Ctrl (Ctrl+Shift in
//! terminals) for apps that don't use Aizome's kit, the system's chords
//! kept for the compositor, and shell bindings sent to subscribers.

mod common;

use std::time::Duration;

use aizome_ipc::{Event, Request, Synthetic};
use common::*;

const SHIFT: u32 = 1;
const CTRL: u32 = 4;
const LOGO: u32 = 64;
const KEY_C: u32 = 46;

fn chord(c: &Compositor, keys: &str) {
    c.ok(Request::Synthesize { events: vec![Synthetic::Chord { keys: keys.into() }] });
}

#[test]
fn super_becomes_ctrl_for_other_apps() {
    let c = Compositor::start(Options::default());
    let a = c.client(Spec::new("org.example.editor", 0x2222cc));
    c.wait_window("org.example.editor");
    a.wait_seen("keyboard focus", |s| s.keyboard_focus);
    chord(&c, "super+c");
    let s = a.wait_seen("❖C as Ctrl+C", |s| s.keys.iter().any(|(k, _)| *k == KEY_C));
    let (_, mods) = s.keys.iter().find(|(k, _)| *k == KEY_C).unwrap();
    assert_eq!(*mods & (CTRL | LOGO | SHIFT), CTRL, "the app saw Ctrl, not Super: {mods:#x}");
    // After the chord, nothing stays held.
    a.wait_seen("modifiers released", |s| s.mods & (CTRL | LOGO) == 0);
}

#[test]
fn terminals_get_ctrl_shift() {
    let c = Compositor::start(Options::default());
    let a = c.client(Spec::new("foot", 0x2222cc));
    c.wait_window("foot");
    a.wait_seen("keyboard focus", |s| s.keyboard_focus);
    chord(&c, "super+c");
    let s = a.wait_seen("❖C as Ctrl+Shift+C", |s| s.keys.iter().any(|(k, _)| *k == KEY_C));
    let (_, mods) = s.keys.iter().find(|(k, _)| *k == KEY_C).unwrap();
    assert_eq!(*mods & (CTRL | LOGO | SHIFT), CTRL | SHIFT, "{mods:#x}");
}

#[test]
fn kit_apps_keep_super() {
    let c = Compositor::start(Options::default());
    let a = c.client(Spec::new("org.aizome.Soroban", 0x2222cc));
    c.wait_window("org.aizome.Soroban");
    a.wait_seen("keyboard focus", |s| s.keyboard_focus);
    chord(&c, "super+c");
    let s = a.wait_seen("❖C as is", |s| s.keys.iter().any(|(k, _)| *k == KEY_C));
    let (_, mods) = s.keys.iter().find(|(k, _)| *k == KEY_C).unwrap();
    assert_eq!(*mods & (CTRL | LOGO), LOGO, "{mods:#x}");
}

#[test]
fn the_setting_turns_translation_off() {
    let c = Compositor::start(Options { settings: "[appearance]\neffects = \"austere\"\n[keys]\ncommand_key_for_all_apps = false\n", ..Default::default() });
    let a = c.client(Spec::new("org.example.editor", 0x2222cc));
    c.wait_window("org.example.editor");
    a.wait_seen("keyboard focus", |s| s.keyboard_focus);
    chord(&c, "super+c");
    let s = a.wait_seen("❖C as is", |s| s.keys.iter().any(|(k, _)| *k == KEY_C));
    let (_, mods) = s.keys.iter().find(|(k, _)| *k == KEY_C).unwrap();
    assert_eq!(*mods & (CTRL | LOGO), LOGO);
}

#[test]
fn system_chords_stay_with_the_system_and_shell_ones_go_to_subscribers() {
    let c = Compositor::start(Options::default());
    let a = c.client(Spec::new("org.example.editor", 0x2222cc));
    let w = c.wait_window("org.example.editor");
    a.wait_seen("keyboard focus", |s| s.keyboard_focus);
    let mut events = aizome_ipc::Client::connect_to(&c.ipc).unwrap().events().unwrap();
    events.stream().set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    // ❖Space: the shell's palette.
    chord(&c, "super+space");
    let got = events.by_ref().find_map(|e| match e {
        Ok(Event::Binding { action, .. }) => Some(action),
        _ => None,
    });
    assert_eq!(got.as_deref(), Some("palette"));
    // ⌥❖↑ fills: the compositor's, and the app sees no key.
    chord(&c, "alt+super+up");
    c.wait_windows("filled", |ws| ws[0].fill);
    // ⌃❖2 goes to Desk 2.
    chord(&c, "ctrl+super+2");
    let aizome_ipc::Response::Desks { current, .. } = c.request(Request::Desks) else { panic!() };
    assert_eq!(current, 2);
    let s = a.seen();
    assert!(!s.keys.iter().any(|(k, _)| *k == 57 || *k == 103 || *k == 3), "no system chord reached the app: {:?}", s.keys);
    let _ = w;
}
