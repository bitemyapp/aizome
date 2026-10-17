//! The switcher, the Desks overview and the marking menu, driven by
//! synthetic input through the compositor's own paths.

mod common;

use aizome_ipc::{Request, Synthetic};
use common::*;

fn key(k: &str, pressed: bool) -> Synthetic {
    Synthetic::Key { key: k.into(), pressed }
}

#[test]
fn the_switcher_picks_the_previous_app_on_release() {
    let c = Compositor::start(Options::default());
    let _a = c.client(Spec::new("org.example.alpha", 0x2222cc).title("Alpha one"));
    let wa = c.wait_window("org.example.alpha");
    let _b = c.client(Spec::new("org.example.beta", 0x22cc22).title("Beta one"));
    c.wait_windows("b focused", |ws| ws.iter().any(|w| w.app_id == "org.example.beta" && w.focused));
    c.ok(Request::Synthesize { events: vec![key("super", true), key("tab", true), key("tab", false)] });
    c.settle();
    let img = c.screenshot();
    keep(&img, "switcher");
    // The panel is up in the middle: raised, not the desktop.
    let mid = img.px(640, 400);
    assert!(!near(mid, rgb(0x16161e)), "the switcher shows");
    c.ok(Request::Synthesize { events: vec![key("super", false)] });
    c.wait_windows("alpha focused", |ws| ws.iter().any(|w| w.id == wa.id && w.focused));
}

#[test]
fn the_switcher_goes_down_to_windows_and_escape_cancels() {
    let c = Compositor::start(Options::default());
    let _a1 = c.client(Spec::new("org.example.alpha", 0x2222cc).title("Alpha one"));
    let a1 = c.wait_window("org.example.alpha");
    let _a2 = c.client(Spec::new("org.example.alpha", 0x3333dd).title("Alpha two"));
    c.wait_windows("two alphas", |ws| ws.len() == 2);
    let _b = c.client(Spec::new("org.example.beta", 0x22cc22));
    c.wait_windows("b focused", |ws| ws.iter().any(|w| w.app_id == "org.example.beta" && w.focused));
    // ❖Tab to Alpha, ` into its windows (the second, A1), release.
    c.ok(Request::Synthesize { events: vec![key("super", true), key("tab", true), key("tab", false), key("grave", true), key("grave", false)] });
    c.settle();
    keep(&c.screenshot(), "switcher-windows");
    c.ok(Request::Synthesize { events: vec![key("super", false)] });
    c.wait_windows("alpha one focused", |ws| ws.iter().any(|w| w.id == a1.id && w.focused));
    // Esc cancels.
    let before = c.windows().into_iter().find(|w| w.focused).unwrap().id;
    c.ok(Request::Synthesize { events: vec![key("super", true), key("tab", true), key("tab", false), key("escape", true), key("escape", false), key("super", false)] });
    c.settle();
    assert_eq!(c.windows().into_iter().find(|w| w.focused).unwrap().id, before);
}

#[test]
fn the_overview_shows_desks_and_moves_windows() {
    let c = Compositor::start(Options::default());
    let _a = c.client(Spec::new("test.a", 0xcc2222).title("Red"));
    let wa = c.wait_window("test.a");
    c.ok(Request::Overview { state: Some(true) });
    c.settle();
    let img = c.screenshot();
    keep(&img, "overview");
    // The backdrop replaces the desktop; the window's spot shows cards.
    assert!(!near(img.px(wa.geometry.x + 50, wa.geometry.y + 50), rgb(0xcc2222)), "the window itself is hidden");
    // Typing filters; 2 goes to Desk 2.
    c.ok(Request::Synthesize { events: vec![Synthetic::Chord { keys: "2".into() }] });
    c.settle();
    let aizome_ipc::Response::Desks { current, .. } = c.request(Request::Desks) else { panic!() };
    assert_eq!(current, 2);
    // Back, and the overview closes with Esc.
    c.ok(Request::Overview { state: Some(true) });
    c.ok(Request::Synthesize { events: vec![Synthetic::Chord { keys: "escape".into() }] });
    c.settle();
    let img = c.screenshot();
    assert!(near(img.px(5, 5), rgb(0x16161e)), "the desktop again");
}

#[test]
fn a_marking_menu_flick_fills_without_drawing() {
    let c = Compositor::start(Options::default());
    let a = c.client(Spec::new("test.a", 0xcc2222));
    let w = c.wait_window("test.a");
    let tab = (w.geometry.x as f64 + 60.0, w.geometry.y as f64 - 12.0);
    c.ok(Request::Synthesize {
        events: vec![
            Synthetic::PointerMotion { x: tab.0, y: tab.1 },
            Synthetic::PointerButton { button: "right".into(), pressed: true },
            Synthetic::PointerMotion { x: tab.0, y: tab.1 - 20.0 },
            Synthetic::PointerMotion { x: tab.0, y: tab.1 - 60.0 },
            Synthetic::PointerButton { button: "right".into(), pressed: false },
        ],
    });
    a.wait_seen("filled by a flick up", |s| s.maximized);
}

#[test]
fn a_marking_menu_shows_after_a_pause_and_picks() {
    let c = Compositor::start(Options::default());
    let a = c.client(Spec::new("test.a", 0xcc2222));
    let w = c.wait_window("test.a");
    let tab = (w.geometry.x as f64 + 60.0, w.geometry.y as f64 - 12.0);
    c.ok(Request::Synthesize {
        events: vec![Synthetic::PointerMotion { x: tab.0, y: tab.1 }, Synthetic::PointerButton { button: "right".into(), pressed: true }],
    });
    std::thread::sleep(std::time::Duration::from_millis(400));
    // Toward "park" (down).
    c.ok(Request::Synthesize { events: vec![Synthetic::PointerMotion { x: tab.0, y: tab.1 + 90.0 }] });
    c.settle();
    let img = c.screenshot();
    keep(&img, "marking-menu");
    // The highlighted chip is ai (selection) indigo.
    let row: Vec<[u8; 3]> = (0..60).map(|d| img.px(tab.0 as i32 - 30 + d, tab.1 as i32 + 96)).collect();
    assert!(row.iter().any(|p| near(*p, rgb(0x3d59a1))), "park is highlighted");
    c.ok(Request::Synthesize { events: vec![Synthetic::PointerButton { button: "right".into(), pressed: false }] });
    c.wait_windows("parked", |ws| ws[0].minimised);
    let _ = a;
}
