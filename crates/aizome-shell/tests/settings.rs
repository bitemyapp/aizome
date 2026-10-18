//! What Settings (Shitsurae) says, honoured live: the desktop's icons, the
//! Shelf, notifications blocked, silent or previewed on the lock screen,
//! the lock screen's clock and locking with the monitors off, the power
//! key, the palette's scopes, the bell's flash and Night Light.

mod common;

use std::time::Duration;

use aizome_config::PowerKey;
use aizome_ipc::Event;
use aizome_kit::prelude::*;
use aizome_shell::actions::{Action, SessionVerb};
use aizome_shell::ipc::IpcEvent;
use aizome_shell::lock::Phase;
use aizome_shell::notifications::server::NoteEvent;
use aizome_shell::notifications::store::{Incoming, NoteSound};
use aizome_shell::shell::{Msg, Shell};
use aizome_shell::systray::panels::{Panel, PanelMsg};
use common::*;

type H = aizome_kit::testing::Harness<Shell>;

/// Change the settings as Shitsurae would, and have the shell read them.
fn settings(h: &mut H, f: impl FnOnce(&mut aizome_config::Settings)) {
    f(&mut h.program_mut().settings);
    h.send(Msg::SettingsChanged);
}

fn note(h: &mut H, id: u32, app: &str, summary: &str, sound: Option<NoteSound>) {
    let n = Incoming { app_name: app.into(), summary: summary.into(), body: "Half past twelve".into(), sound, ..Default::default() };
    h.send(Msg::Note(NoteEvent::Notify(id, Box::new(n))));
}

fn binding(h: &mut H, action: &str) {
    h.send(Msg::Ipc(IpcEvent::Event(Event::Binding { action: action.into(), pointer: None, output: None })));
}

#[test]
fn the_desktop_shows_the_icons_it_is_told_to() {
    let mut h = scene(false);
    h.program_mut().volumes_fixture = vec![aizome_sys::disks::Volume { block: "/b/nvme0n1p3".into(), name: "nvme0".into(), ..Default::default() }];
    h.program_mut().user = ("callen".into(), "Chris Allen".into());
    let keys = |h: &H| h.program().desktop_icons().into_iter().map(|i| i.key).collect::<Vec<_>>();
    assert_eq!(keys(&h), ["home", "vol:/b/nvme0n1p3", "trash"]);
    settings(&mut h, |s| {
        let mut d = s.desktop_settings();
        d.show_trash = false;
        d.show_disks = false;
        s.set_shell_section("desktop", &d);
    });
    assert_eq!(keys(&h), ["home"]);
}

#[test]
fn the_shelf_can_go() {
    let mut h = scene(false);
    with_mail_and_shelf(&mut h);
    let with = h.program().deskbar_model();
    assert!(!with.hide_shelf);
    settings(&mut h, |s| {
        let mut d = s.deskbar_settings();
        d.show_shelf = false;
        s.set_shell_section("deskbar", &d);
    });
    let without = h.program().deskbar_model();
    assert!(without.hide_shelf);
    assert!(without.height() < with.height());
    let db = window_titled(&h, "Deskbar");
    assert!(!h.texts(db).iter().any(|t| t == "wallpaper-draft.png"), "no Shelf rows");
}

#[test]
fn blocked_apps_are_dropped_and_silent_ones_make_no_sound() {
    let mut h = scene(false);
    settings(&mut h, |s| {
        let mut n = s.notification_settings();
        n.blocked_apps = vec!["spam.desktop".into()];
        n.silent_apps = vec!["Mail".into()];
        s.set_shell_section("notifications", &n);
    });
    let ding = || Some(NoteSound::Named("message-new-instant".into()));
    note(&mut h, 90, "Spam", "Win a prize", ding());
    assert!(h.program().notes.get(90).is_none(), "dropped");
    assert_eq!(h.program().banner, None);
    assert_eq!(h.program().last_sound, None);
    note(&mut h, 91, "Mail", "Lunch?", ding());
    assert!(h.program().notes.get(91).is_some(), "shown");
    assert_eq!(h.program().last_sound, None, "but silent");
    note(&mut h, 92, "Chat", "Hi", ding());
    assert_eq!(h.program().last_sound.as_deref(), Some("Named(\"message-new-instant\")"));
}

fn lock(h: &mut H) -> WindowKey {
    h.send(Msg::Run(Action::Session(SessionVerb::Lock)));
    h.confirm_lock();
    let lock = h.windows().into_iter().find(|k| h.title(*k).is_empty()).expect("a lock surface");
    h.resize(lock, Size::new(W, H));
    lock
}

#[test]
fn the_lock_screen_shows_what_came_and_its_clock_as_told() {
    let mut h = scene(false);
    let lock_k = lock(&mut h);
    assert!(h.texts(lock_k).iter().any(|t| t == "21:42"));
    note(&mut h, 93, "Mail", "Lunch?", None);
    note(&mut h, 94, "Mail", "Re: Lunch?", None);
    // Without previews: whose, and how many.
    assert!(h.texts(lock_k).iter().any(|t| t == "Mail 2 notifications"), "{:?}", h.texts(lock_k));
    assert!(!h.texts(lock_k).iter().any(|t| t.contains("Lunch")));
    settings(&mut h, |s| {
        let mut n = s.notification_settings();
        n.previews_when_locked = true;
        s.set_shell_section("notifications", &n);
        let mut l = s.lock_settings();
        l.show_clock = false;
        s.set_shell_section("lock", &l);
    });
    let texts = h.texts(lock_k);
    assert!(texts.iter().any(|t| t == "Mail Re: Lunch?") && texts.iter().any(|t| t == "Mail Lunch?"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "21:42"), "no clock");
    h.save_png(lock_k, shots().join("lock-notes-night.png"), 1.0);
}

#[test]
fn the_monitors_going_off_locks_unless_told_not_to() {
    let idle = |h: &mut H| h.send(Msg::Ipc(IpcEvent::Event(Event::Idle { kind: "monitors-off".into() })));
    let mut h = scene(false);
    settings(&mut h, |s| {
        let mut l = s.lock_settings();
        l.after_monitors_off = false;
        s.set_shell_section("lock", &l);
    });
    idle(&mut h);
    assert_eq!(h.program().lock.phase, Phase::Unlocked);
    settings(&mut h, |s| {
        let mut l = s.lock_settings();
        l.after_monitors_off = true;
        s.set_shell_section("lock", &l);
    });
    idle(&mut h);
    assert_ne!(h.program().lock.phase, Phase::Unlocked);
}

#[test]
fn the_power_key_does_what_it_is_told() {
    let set = |h: &mut H, k: Option<PowerKey>| settings(h, |s| s.set_shell_section("power", &aizome_config::PowerSettings { power_key: k }));
    // Unset, with logind handling the key: the shell leaves it alone.
    let mut h = scene(false);
    h.program_mut().logind_power_key = Some("poweroff".into());
    binding(&mut h, "power-key");
    assert!(h.program().confirm.is_none() && h.program().session_done.is_none());
    // Unset, logind leaving it to the session: ask.
    h.program_mut().logind_power_key = Some("ignore".into());
    binding(&mut h, "power-key");
    let c = window_titled(&h, "Confirm");
    for want in ["Sleep", "Restart", "Cancel", "Shut Down"] {
        assert!(h.texts(c).iter().any(|t| t == want), "{want} in {:?}", h.texts(c));
    }
    h.save_png(c, shots().join("confirm-power-night.png"), 2.0);
    h.send(Msg::Confirm(aizome_shell::confirm::ConfirmMsg::Do(SessionVerb::Sleep)));
    assert_eq!(h.program().session_done, Some(SessionVerb::Sleep));
    assert!(h.program().confirm.is_none());
    // Set: lock, whatever logind says.
    set(&mut h, Some(PowerKey::Lock));
    h.program_mut().logind_power_key = Some("poweroff".into());
    binding(&mut h, "power-key");
    assert_ne!(h.program().lock.phase, Phase::Unlocked);
    let mut h = scene(false);
    set(&mut h, Some(PowerKey::Nothing));
    binding(&mut h, "power-key");
    assert!(h.program().confirm.is_none() && h.program().session_done.is_none());
}

#[test]
fn the_palette_finds_only_its_scopes() {
    let mut h = scene(false);
    settings(&mut h, |s| {
        let mut p = s.palette_settings();
        p.scopes = vec!["commands".into(), "apps".into()];
        s.set_shell_section("palette", &p);
    });
    use aizome_search::Scope;
    assert_eq!(h.program().palette_scopes(), [Scope::Command, Scope::App]);
    let items = h.program().palette_items();
    assert!(items.iter().all(|i| matches!(i.scope, Scope::Command | Scope::App)));
    assert!(items.iter().any(|i| i.scope == Scope::Command));
    // A scope it doesn't find opens All.
    h.send(Msg::Run(Action::Palette { scope: Some(Scope::Window), query: String::new() }));
    let p = window_titled(&h, "Palette");
    assert_eq!(h.program().palette.as_ref().map(|p| p.scope), Some(None));
    let texts = h.texts(p);
    assert!(texts.iter().any(|t| t == "Apps") && !texts.iter().any(|t| t == "Windows"), "the chips: {texts:?}");
}

#[test]
fn the_bell_flashes_the_edge_or_sounds() {
    let mut h = scene(false);
    h.send(Msg::Run(Action::None));
    let bell = |h: &mut H| h.send(Msg::Ipc(IpcEvent::Event(Event::Binding { action: "bell".into(), pointer: None, output: None })));
    bell(&mut h);
    assert_eq!(h.program().last_sound.as_deref(), Some("bell"));
    assert!(h.program().flash.is_none());
    settings(&mut h, |s| s.set_shell_section("sound", &aizome_config::SoundSettings { visual_alert: true }));
    bell(&mut h);
    let f = window_titled(&h, "Flash");
    h.resize(f, Size::new(W, H));
    h.save_png(f, shots().join("flash-night.png"), 1.0);
    let (w, hh, px) = h.render_rgba(f, 1.0);
    let at = |x: u32, y: u32| px[((y * w + x) * 4 + 3) as usize];
    assert_eq!(at(w / 2, hh / 2), 0, "see-through inside");
    assert_eq!(at(2, hh / 2), 255, "the edge");
    h.advance(Duration::from_millis(200));
    assert!(h.windows().iter().all(|k| h.title(*k) != "Flash"), "gone after a moment");
}

#[test]
fn night_light_shows_where_the_compositor_can() {
    let mut h = scene(false);
    let display = |h: &H| match h.program().panel_data(aizome_shell::menubar::TrayWidget::Display) {
        Panel::Display(d) => d.night_light,
        _ => unreachable!(),
    };
    assert_eq!(display(&h), None, "nothing said yet: no switch");
    let state = aizome_ipc::NightLightState { enabled: false, outputs: vec!["eDP-1".into()], ..Default::default() };
    h.send(Msg::Ipc(IpcEvent::Event(Event::NightLightChanged { state })));
    assert_eq!(display(&h), Some(false));
    h.send(Msg::Panel(PanelMsg::NightLight(true)));
    assert_eq!(display(&h), Some(true));
    assert!(h.program().settings.night_light.enabled, "kept in the settings");
}
