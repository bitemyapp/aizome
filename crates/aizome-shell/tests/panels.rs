//! The systray's panels, headless: each opens under its widget with its
//! controls, and draws as the boards' panels do.

mod common;

use aizome_kit::prelude::*;
use aizome_shell::clock::Local;
use aizome_shell::menubar::{BarMsg, Slot, TrayWidget};
use aizome_shell::shell::Msg;
use aizome_shell::systray::panels::*;
use common::*;

fn fixture() -> aizome_shell::systray::glue::Fixture {
    let mut f = aizome_shell::systray::glue::Fixture::new();
    let net = |ssid: &str, bars: u8, secure: bool, connected: bool| NetRow { ssid: ssid.into(), bars, secure, connected, connecting: false, known: connected };
    f.insert(
        TrayWidget::Network,
        Panel::Network(NetPanel {
            wifi: Some(true),
            networks: vec![net("kestrel", 4, true, true), net("Ramen Bar Guest", 3, false, false), net("NETGEAR-5G", 2, true, false), net("printer-setup", 1, true, false)],
            vpns: vec![("u1".into(), "Office WireGuard".into(), false)],
            airplane: false,
            asking: None,
            error: None,
        }),
    );
    f.insert(
        TrayWidget::Sound,
        Panel::Sound(SoundPanel {
            output: Some((0.62, false)),
            outputs: vec![("speakers".into(), "Speakers".into(), true), ("hp".into(), "WH-1000XM4".into(), false)],
            input: Some((0.4, false)),
            media: Some(Media { title: "Clair de lune".into(), artist: "Isao Tomita".into(), playing: true }),
        }),
    );
    f.insert(
        TrayWidget::Bluetooth,
        Panel::Bluetooth(BtPanel {
            powered: true,
            devices: vec![
                BtDevice { path: "/a".into(), name: "WH-1000XM4".into(), connected: true, battery: Some(80) },
                BtDevice { path: "/b".into(), name: "MX Master 3".into(), connected: false, battery: None },
            ],
        }),
    );
    f.insert(TrayWidget::Display, Panel::Display(DisplayPanel { backlights: vec![("intel_backlight".into(), 0.7)], theme: 0, size: 1 }));
    f.insert(
        TrayWidget::Battery,
        Panel::Battery(BatteryPanel {
            percent: 84.0,
            state: "On battery".into(),
            time: Some("5:12 left".into()),
            modes: vec!["power-saver".into(), "balanced".into(), "performance".into()],
            mode: Some(1),
        }),
    );
    f.insert(
        TrayWidget::Clock,
        Panel::Clock(ClockPanel {
            now: Local { year: 2026, month: 10, day: 17, hour: 21, minute: 42, second: 0, weekday: 6 },
            dnd: false,
            zones: vec![("Tokyo".into(), "11:42".into()), ("Lisbon".into(), "03:42".into())],
        }),
    );
    f
}

fn open(h: &mut aizome_kit::testing::Harness<aizome_shell::shell::Shell>, w: TrayWidget) -> WindowKey {
    let bar = window_titled(h, "Menu bar");
    h.send(Msg::Bar(bar, BarMsg::Press(Slot::Tray(w), Rect::new(1300.0, 0.0, 1315.0, 24.0))));
    window_titled(h, w.name())
}

#[test]
fn a_panel_opens_under_its_widget_and_toggles() {
    let mut h = scene(false);
    h.program_mut().panels_fixture = Some(fixture());
    let p = open(&mut h, TrayWidget::Network);
    assert!(h.texts(p).iter().any(|t| t == "kestrel"));
    assert!(h.texts(p).iter().any(|t| t == "Network Settings…"));
    let spec = &h.platform.windows.values().last().unwrap().spec;
    assert!(matches!(spec.kind, aizome_kit::platform::OpenKind::Popup { grab: true, .. }));
    // Pressing the widget again closes it.
    open_again(&mut h, TrayWidget::Network);
    assert!(h.program().panel.is_none());
}

fn open_again(h: &mut aizome_kit::testing::Harness<aizome_shell::shell::Shell>, w: TrayWidget) {
    let bar = window_titled(h, "Menu bar");
    h.send(Msg::Bar(bar, BarMsg::Press(Slot::Tray(w), Rect::new(1300.0, 0.0, 1315.0, 24.0))));
}

#[test]
fn a_secured_network_asks_its_password_inline() {
    let mut h = scene(false);
    h.program_mut().panels_fixture = Some(fixture());
    let p = open(&mut h, TrayWidget::Network);
    h.send(Msg::Panel(PanelMsg::Network("NETGEAR-5G".into())));
    assert_eq!(h.program().panel_ui.asking.as_ref().map(|a| a.0.as_str()), Some("NETGEAR-5G"));
    h.type_text(p, "hunter22");
    assert_eq!(h.program().panel_ui.asking.as_ref().map(|a| a.1.as_str()), Some("hunter22"));
    assert!(!h.texts(p).iter().any(|t| t.contains("hunter22")), "the password doesn't show");
}

#[test]
fn pictures_of_the_panels() {
    for day in [false, true] {
        let mut h = scene(day);
        h.program_mut().panels_fixture = Some(fixture());
        for w in [TrayWidget::Network, TrayWidget::Sound, TrayWidget::Bluetooth, TrayWidget::Display, TrayWidget::Battery, TrayWidget::Clock] {
            let p = open(&mut h, w);
            let name = format!("panel-{}-{}.png", w.name(), if day { "day" } else { "night" });
            h.save_png(p, shots().join(&name), 2.0);
            if w == TrayWidget::Network || w == TrayWidget::Sound {
                aizome_kit::testing::assert_golden(&mut h, p, golden(&name), 2.0);
            }
            h.send(Msg::Panel(PanelMsg::Close));
        }
    }
}
