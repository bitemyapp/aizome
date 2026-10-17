//! What a menu tells whoever showed it, besides the item chosen: that it
//! closed (however it closed).

use aizome_kit::prelude::*;
use aizome_kit::sidestep_ui::{Modifiers, NamedKey};
use aizome_kit::testing::Harness;
use aizome_kit::{Anchors, KeyboardInteractivity, Layer, LayerOptions};

#[derive(Default)]
struct Bar {
    bar: Option<WindowKey>,
    heard: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum Msg {
    Open(u32),
    CloseAll,
    Closed(u32),
    Choose(&'static str),
}

fn menu() -> Menu<Msg> {
    Menu::new("File")
        .item(MenuItem::new("New", Msg::Choose("new")))
        .item(MenuItem::new("Open…", Msg::Choose("open")))
        .submenu(Menu::new("Open Recent").item(MenuItem::new("notes.md", Msg::Choose("notes"))))
}

impl Program for Bar {
    type Msg = Msg;
    fn info(&self) -> AppInfo {
        AppInfo::new("org.aizome.Test", "Test", "Test")
    }
    fn launched(&mut self, cx: &mut Cx<Msg>) {
        let options = LayerOptions::new(Layer::Top, "aizome-menubar")
            .anchors(Anchors::TOP | Anchors::LEFT | Anchors::RIGHT)
            .exclusive_zone(24)
            .keyboard(KeyboardInteractivity::OnDemand);
        self.bar = Some(cx.open_window(WindowSpec::layer(options, 600.0, 24.0)));
    }
    fn update(&mut self, msg: Msg, cx: &mut Cx<Msg>) {
        let bar = self.bar.expect("the bar");
        match msg {
            Msg::Open(n) => {
                let x = 40.0 * n as f64;
                cx.show_menu(bar, Rect::new(x, 0.0, x + 40.0, 24.0), menu(), MenuStyle::below().on_closed(Msg::Closed(n)));
            }
            Msg::CloseAll => cx.close_menus(bar),
            Msg::Closed(n) => self.heard.push(format!("closed {n}")),
            Msg::Choose(c) => self.heard.push(c.to_owned()),
        }
    }
    fn view(&self, _w: WindowKey) -> Element<Msg> {
        aizome_kit::row![text("File"), spacer()].padding([0.0, 8.0]).into()
    }
}

fn heard(h: &mut Harness<Bar>) -> Vec<String> {
    std::mem::take(&mut h.program_mut().heard)
}

#[test]
fn a_menu_says_when_it_closes_however_it_closes() {
    let mut h = Harness::new(Bar::default());
    let bar = h.window();

    // Escape.
    h.send(Msg::Open(1));
    assert_eq!(h.popups(bar).len(), 1);
    assert!(heard(&mut h).is_empty(), "open, nothing said yet");
    h.named(bar, NamedKey::Escape, Modifiers::NONE);
    assert!(h.popups(bar).is_empty());
    assert_eq!(heard(&mut h), ["closed 1"]);

    // An item chosen: the item's message, and the menu closing.
    h.send(Msg::Open(1));
    h.named(bar, NamedKey::ArrowDown, Modifiers::NONE);
    h.named(bar, NamedKey::Enter, Modifiers::NONE);
    let said = heard(&mut h);
    assert!(said.contains(&"new".to_owned()) && said.contains(&"closed 1".to_owned()), "{said:?}");

    // Another menu in its place: the first's closing only.
    h.send(Msg::Open(1));
    h.send(Msg::Open(2));
    assert_eq!(h.popups(bar).len(), 1);
    assert_eq!(heard(&mut h), ["closed 1"]);

    // A submenu opening and closing isn't the menu closing.
    for _ in 0..3 {
        h.named(bar, NamedKey::ArrowDown, Modifiers::NONE);
    }
    h.named(bar, NamedKey::ArrowRight, Modifiers::NONE);
    assert_eq!(h.popups(bar).len(), 2);
    h.named(bar, NamedKey::ArrowLeft, Modifiers::NONE);
    assert_eq!(h.popups(bar).len(), 1);
    assert!(heard(&mut h).is_empty());

    // Closed by the program.
    h.send(Msg::CloseAll);
    assert_eq!(heard(&mut h), ["closed 2"]);

    // A press outside it, in the window it's over.
    h.send(Msg::Open(3));
    h.click(bar, Point::new(500.0, 12.0));
    assert!(h.popups(bar).is_empty());
    assert_eq!(heard(&mut h), ["closed 3"]);
}
