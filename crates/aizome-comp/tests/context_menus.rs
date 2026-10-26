//! Context menus over an input method. ibus grabs the keyboard while a
//! text field has it, as Telegram's message field nearly always does: a
//! right-click there opens the field's menu all the same, the menu keeps
//! the keys until it closes (once the input method has let go of the
//! keyboard too), and the input method has them back after.

mod common;

use aizome_ipc::{Request, Response, Synthetic, WindowInfo};
use common::text::*;
use common::*;

const KEY_ESC: u32 = 1;
const KEY_F: u32 = 33;

fn tap(c: &Compositor, key: &str) {
    c.ok(Request::Synthesize {
        events: vec![
            Synthetic::Key { key: key.into(), pressed: true },
            Synthetic::Key { key: key.into(), pressed: false },
        ],
    });
}

fn right_button(c: &Compositor, pressed: bool) {
    c.ok(Request::Synthesize { events: vec![Synthetic::PointerButton { button: "right".into(), pressed }] });
}

fn centre(w: &WindowInfo) -> (f64, f64) {
    ((w.geometry.x + w.geometry.w / 2) as f64, (w.geometry.y + w.geometry.h / 2) as f64)
}

fn keyboard_grabbed(c: &Compositor) -> bool {
    match c.request(Request::InputState) {
        Response::InputState { state } => state.keyboard_grab,
        r => panic!("{r:?}"),
    }
}

/// An input method, and the app with its field focused: the input method
/// holds the keyboard.
fn setup(field_in_menu: bool) -> (Compositor, InputMethod, TextApp) {
    let c = Compositor::start(Options::default());
    let ime = InputMethod::spawn(&c);
    let app = TextApp::spawn(&c, field_in_menu);
    c.wait_window(APP_ID);
    app.wait("the field has the keyboard", |s| s.focus == Some("window"));
    ime.wait("the input method has the keyboard", |s| s.active && s.grabbed);
    (c, ime, app)
}

/// Right-press on the field, which opens its menu, then let go.
fn right_click(c: &Compositor, app: &TextApp) {
    let (x, y) = centre(&c.window(APP_ID).unwrap());
    c.ok(Request::Synthesize { events: vec![Synthetic::PointerMotion { x, y }] });
    right_button(c, true);
    let s = app.wait("the menu shown", |s| s.menu_shown || s.menu_dismissed);
    assert!(!s.menu_dismissed, "the compositor closed the menu as it opened");
    right_button(c, false);
    app.wait("the menu has the keyboard", |s| s.focus == Some("menu") || s.menu_dismissed);
}

#[test]
fn a_right_click_in_a_text_field_opens_its_menu_over_the_input_method() {
    let (c, ime, app) = setup(false);
    right_click(&c, &app);
    // The field lost the keyboard, so the input method lets go of it, and
    // with its grab the menu's would go.
    ime.wait("the input method let go", |s| !s.active && !s.grabbed);
    c.settle();
    assert!(!app.seen().menu_dismissed, "the menu closed:\n{}", c.log_text());
    assert_eq!(app.seen().focus, Some("menu"));
    assert!(keyboard_grabbed(&c), "the menu holds the keyboard no longer");
    tap(&c, "escape");
    app.wait("Escape in the menu", |s| s.keys_to("menu") == [KEY_ESC]);
    assert!(ime.seen().keys.is_empty(), "the input method got keys: {:?}", ime.seen().keys);
    // Closed, the keys go back to the field, through the input method.
    app.send(FieldCmd::CloseMenu);
    app.wait("the field has the keyboard again", |s| s.focus == Some("window"));
    ime.wait("the input method has the keyboard again", |s| s.active && s.grabbed);
    tap(&c, "f");
    ime.wait("f to the input method", |s| s.keys == [KEY_F]);
    assert!(!app.seen().menu_dismissed);
}

#[test]
fn a_menu_keeps_the_keys_when_the_window_is_focused_again() {
    let (c, ime, app) = setup(false);
    right_click(&c, &app);
    ime.wait("the input method let go", |s| !s.grabbed);
    // Focusing its window (the Deskbar, the switcher) looks again at where
    // the keys go: still to its menu.
    let id = c.window(APP_ID).unwrap().id;
    c.ok(Request::Focus { id });
    c.settle();
    assert!(!app.seen().menu_dismissed, "the menu closed");
    assert_eq!(app.seen().focus, Some("menu"));
    tap(&c, "escape");
    app.wait("Escape in the menu", |s| s.keys_to("menu") == [KEY_ESC]);
    assert!(app.seen().keys_to("window").is_empty());
}

#[test]
fn a_field_in_a_menu_types_through_the_input_method() {
    let (c, ime, app) = setup(true);
    right_click(&c, &app);
    // The input method moves from the window's field to the menu's, and
    // holds the keyboard for it.
    ime.wait("the input method in the menu's field", |s| s.activations == 2 && s.active && s.grabbed);
    // Looking again at where the keys go leaves them in the menu, though
    // the keyboard's grab is the input method's.
    let id = c.window(APP_ID).unwrap().id;
    c.ok(Request::Focus { id });
    c.settle();
    assert!(!app.seen().menu_dismissed, "the menu closed");
    assert_eq!(app.seen().focus, Some("menu"));
    tap(&c, "f");
    ime.wait("f to the input method", |s| s.keys == [KEY_F]);
    // Closed, the window's field has the keyboard and the input method.
    app.send(FieldCmd::CloseMenu);
    app.wait("the field has the keyboard again", |s| s.focus == Some("window"));
    ime.wait("the input method back in the window's field", |s| s.activations == 3 && s.grabbed);
}
